//! 通过桌面图标宿主找到桌面，并让卡片始终紧贴其上方显示。
//! 前台事件只会发到 UI 线程；Shell 重启时由 1 秒的备用检查处理。
//!
//! 打开文件对话框时，系统会临时再建一个带 `SHELLDLL_DefView` 的 `WorkerW`。
//! `EnumWindows` 自上而下，先碰到的是这个窗口而不是桌面。若把它当成桌面宿主，
//! 卡片会被插到对话框旁边，一下子盖到桌面上。
use super::{state, MESSAGE};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        UI::{
            Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
            WindowsAndMessaging::*,
        },
    },
};

static OWNER: AtomicIsize = AtomicIsize::new(0);
static QUEUED: AtomicBool = AtomicBool::new(false);
#[derive(Default)]
pub(super) struct Layer {
    hook: Option<HWINEVENTHOOK>,
    host: HWND,
    anchor: HWND,
}

unsafe extern "system" fn foreground(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let owner = OWNER.load(Ordering::Relaxed);
    if owner != 0 && !QUEUED.swap(true, Ordering::Relaxed) {
        unsafe {
            if PostMessageW(Some(HWND(owner as *mut _)), MESSAGE, WPARAM(0), LPARAM(0)).is_err() {
                QUEUED.store(false, Ordering::Relaxed);
            }
        }
    }
}

struct HostFacts {
    hwnd: HWND,
    shell: bool,
    defview: bool,
    /// `SysListView32` / `FolderView`：只有真正的桌面图标列表叫这个名字。
    icons: bool,
    owned: bool,
    area: i64,
}

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn window_area(hwnd: HWND) -> i64 {
    let mut r = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut r).is_err() } {
        return 0;
    }
    let w = (r.right - r.left).max(0) as i64;
    let h = (r.bottom - r.top).max(0) as i64;
    w.saturating_mul(h)
}

fn inspect_window(hwnd: HWND) -> HostFacts {
    let class = window_class(hwnd);
    let shell = class == "Progman" || class == "WorkerW";
    let owned = unsafe { !GetWindow(hwnd, GW_OWNER).unwrap_or_default().is_invalid() };
    let (defview, icons) = if shell {
        match unsafe { FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None) } {
            Ok(view) => (true, unsafe {
                FindWindowExW(Some(view), None, w!("SysListView32"), w!("FolderView")).is_ok()
            }),
            Err(_) => (false, false),
        }
    } else {
        (false, false)
    };
    let area = if shell { window_area(hwnd) } else { 0 };
    HostFacts {
        hwnd,
        shell,
        defview,
        icons,
        owned,
        area,
    }
}

unsafe extern "system" fn collect_hosts(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
    unsafe {
        let rows = &mut *(lp.0 as *mut Vec<HostFacts>);
        let class = window_class(hwnd);
        if class == "Progman" || class == "WorkerW" {
            rows.push(inspect_window(hwnd));
        }
    }
    true.into()
}

/// 在候选里选出桌面宿主。文件对话框的临时 `WorkerW` 更小，也通常没有
/// `FolderView`；两者都有时取面积更大的那个（桌面盖住整块虚拟屏幕）。
fn choose_desktop_host(windows: &[HostFacts]) -> Option<usize> {
    fn prefer(
        windows: &[HostFacts],
        best: Option<usize>,
        index: usize,
        icons: bool,
    ) -> Option<usize> {
        let candidate = &windows[index];
        if !candidate.shell || !candidate.defview || candidate.owned || candidate.icons != icons {
            return best;
        }
        match best {
            Some(prev) if candidate.area < windows[prev].area => best,
            _ => Some(index),
        }
    }
    let mut exact = None;
    let mut fallback = None;
    for index in 0..windows.len() {
        exact = prefer(windows, exact, index, true);
        fallback = prefer(windows, fallback, index, false);
    }
    exact.or(fallback)
}

/// 返回宿主，以及它是不是带桌面图标列表的那个。
fn desktop_host() -> (HWND, bool) {
    let mut rows = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_hosts),
            LPARAM(&mut rows as *mut Vec<HostFacts> as isize),
        );
    }
    match choose_desktop_host(&rows) {
        Some(index) => (rows[index].hwnd, rows[index].icons),
        None => (HWND::default(), false),
    }
}

fn common_dialog_foreground() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    !hwnd.is_invalid() && window_class(hwnd) == "#32770"
}

fn foreign_foreground(cards: &[HWND], host: HWND) -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    !hwnd.is_invalid() && hwnd != host && !cards.contains(&hwnd)
}

fn top_anchor(host: HWND) -> HWND {
    if unsafe { GetWindowLongPtrW(host, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 } {
        HWND_TOPMOST
    } else {
        HWND_TOP
    }
}

impl Layer {
    pub fn stop(&mut self) {
        OWNER.store(0, Ordering::Relaxed);
        QUEUED.store(false, Ordering::Relaxed);
        if let Some(h) = self.hook.take() {
            unsafe {
                let _ = UnhookWinEvent(h);
            }
        }
        self.host = HWND::default();
        self.anchor = HWND::default();
    }
    pub fn arrange(&mut self, owner: HWND, windows: &[HWND]) {
        QUEUED.store(false, Ordering::Relaxed);
        if windows.is_empty() {
            self.stop();
            return;
        }
        // 弹出菜单激活前台时，会通过所属窗口的消息循环重新进入此代码。
        // 调整该窗口的顺序可能会关闭弹出菜单。
        // 将整批调整延后执行，避免其他卡片同时改变相对顺序。
        if windows
            .iter()
            .any(|&hwnd| unsafe { state(hwnd).is_some_and(|s| s.context_active) })
        {
            return;
        }
        if self.hook.is_none() {
            OWNER.store(owner.0 as isize, Ordering::Relaxed);
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(foreground),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            if !hook.0.is_null() {
                self.hook = Some(hook);
            }
        }
        // 公共文件对话框（#32770）打开的瞬间会改 Z 序，并冒出临时 WorkerW。
        // 这段时间不要重排；对话框关掉、前台回到原来的窗口后再排一次。
        if !self.host.is_invalid() && common_dialog_foreground() {
            return;
        }
        // Explorer 重启后窗口句柄可能会被重复使用。应重新查找，不要缓存未经验证的父窗口。
        let (host, icons) = desktop_host();
        if host.is_invalid() {
            for &hwnd in windows {
                position(hwnd, HWND_BOTTOM);
            }
            self.host = host;
            self.anchor = HWND::default();
            return;
        }
        let mut anchor = unsafe { GetWindow(host, GW_HWNDPREV).unwrap_or_default() };
        if anchor.is_invalid() {
            // 宿主自己已经在最顶上。正常桌面在最底下；文件对话框有时会把
            // WorkerW 抬上来。这时再放到 HWND_TOP，卡片就会盖住桌面。
            // 前台就是这块桌面（或没有其他窗口）时，仍让卡片留在桌面上。
            if !icons || foreign_foreground(windows, host) {
                return;
            }
            anchor = top_anchor(host);
        } else {
            let mut guard = 0;
            while windows.contains(&anchor) && guard < 32 {
                anchor = unsafe { GetWindow(anchor, GW_HWNDPREV).unwrap_or_default() };
                guard += 1;
            }
            if anchor.is_invalid() {
                // 桌面上面只剩卡片自己。不能因为点了卡片就把它们塞到桌面后面。
                anchor = top_anchor(host);
            }
        }
        let stable = self.host == host && self.anchor == anchor && {
            // 也要校验卡片当前的实际顺序，因为 Win+D 和焦点切换都可能改变顺序而不更换句柄。
            let mut previous = anchor;
            let mut ok = true;
            for &hwnd in windows {
                if unsafe { GetWindow(hwnd, GW_HWNDPREV).unwrap_or_default() } != previous {
                    ok = false;
                    break;
                }
                previous = hwnd;
            }
            ok
        };
        if stable {
            return;
        }
        self.host = host;
        self.anchor = anchor;
        let mut after = anchor;
        for &hwnd in windows {
            position(hwnd, after);
            after = hwnd;
        }
    }
}

fn position(hwnd: HWND, after: HWND) {
    unsafe {
        if let Some(s) = state(hwnd) {
            s.allow_z = true;
        }
        // 切换层级前先取消置顶；旧的“显示桌面”位置不能让卡片一直浮在应用上方。
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        let _ = SetWindowPos(
            hwnd,
            Some(after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        if let Some(s) = state(hwnd) {
            s.allow_z = false;
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(shell: bool, defview: bool, icons: bool, owned: bool, area: i64) -> HostFacts {
        HostFacts {
            hwnd: HWND::default(),
            shell,
            defview,
            icons,
            owned,
            area,
        }
    }

    #[test]
    fn file_dialog_worker_is_not_chosen_over_the_desktop() {
        // EnumWindows 自上而下：先是对话框临时造的小 WorkerW，对话框本体即使有
        // DefView 也不是 Progman/WorkerW，最下面才是带 FolderView 的桌面。
        let rows = vec![
            fact(true, true, false, false, 800 * 600),
            fact(false, true, true, false, 1200 * 800),
            fact(true, true, true, false, 1920 * 1080),
        ];
        assert_eq!(choose_desktop_host(&rows), Some(2));
    }

    #[test]
    fn a_smaller_folder_view_does_not_beat_the_desktop() {
        let rows = vec![
            fact(true, true, true, false, 900 * 700),
            fact(true, true, true, false, 1920 * 1080),
        ];
        assert_eq!(choose_desktop_host(&rows), Some(1));
    }

    #[test]
    fn a_lower_but_smaller_window_does_not_replace_the_desktop() {
        let rows = vec![
            fact(true, true, true, false, 1920 * 1080),
            fact(true, true, true, false, 800 * 600),
        ];
        assert_eq!(choose_desktop_host(&rows), Some(0));
    }

    #[test]
    fn owned_shell_window_is_ignored() {
        let rows = vec![
            fact(true, true, true, true, 1920 * 1080),
            fact(true, true, false, false, 1920 * 1080),
        ];
        assert_eq!(choose_desktop_host(&rows), Some(1));
    }

    #[test]
    fn equal_area_keeps_the_lower_window() {
        let rows = vec![
            fact(true, true, true, false, 100),
            fact(true, true, true, false, 100),
        ];
        assert_eq!(choose_desktop_host(&rows), Some(1));
    }

    #[test]
    fn without_folder_view_the_larger_shell_window_is_the_desktop() {
        let rows = vec![
            fact(true, true, false, false, 800 * 600),
            fact(true, true, false, false, 1920 * 1080),
        ];
        assert_eq!(choose_desktop_host(&rows), Some(1));
    }

    #[test]
    fn nothing_matches_without_a_shell_defview() {
        let rows = vec![
            fact(false, true, true, false, 1920 * 1080),
            fact(true, false, false, false, 1920 * 1080),
            fact(true, true, true, true, 1920 * 1080),
        ];
        assert_eq!(choose_desktop_host(&rows), None);
    }

    #[test]
    fn live_desktop_host_is_a_shell_window() {
        let (host, _) = desktop_host();
        if host.is_invalid() {
            return;
        }
        let class = window_class(host);
        assert!(
            class == "Progman" || class == "WorkerW",
            "桌面宿主应是 Progman 或 WorkerW，实际是 {class}"
        );
        let facts = inspect_window(host);
        assert!(facts.defview);
        assert!(!facts.owned);
    }
}
