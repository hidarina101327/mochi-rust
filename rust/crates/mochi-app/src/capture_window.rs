//! 捕获输入使用原生 EDIT 控件，外观由 D2D 绘制。
use crate::{
    gfx::Renderer,
    ui::{capture_view, draw::DrawList, layout::Rect},
};

// Win32 EDIT messages (winuser.h)，不引入整套 Common Controls。
mod hotkey;
#[cfg(debug_assertions)]
mod verification;
pub use hotkey::{mark_hotkey_notice, register_hotkey, release_hotkey};
#[cfg(debug_assertions)]
pub use verification::snapshot;
pub const OPEN_ITEM: u32 = WM_APP + 70;
#[derive(Clone)]
pub enum Action {
    File(PathBuf),
    Chat(String),
    Schedule(String),
    Page(usize),
}
pub struct Entry {
    pub title: String,
    pub detail: String,
    pub action: Action,
}

const EM_SETSEL: u32 = 0x00b1;
const EM_SETLIMITTEXT: u32 = 0x00c5;
use std::path::PathBuf;
use windows::core::{w, Result};
use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL, VK_MENU, VK_SHIFT},
        WindowsAndMessaging::*,
    },
};
struct State {
    owner: HWND,
    root: Option<PathBuf>,
    dark: bool,
    error: String,
    edit: HWND,
    old_edit: isize,
    font: HFONT,
    brush: HBRUSH,
    composing: bool,
    renderer: Renderer,
    pages: [Vec<Entry>; 5],
    page: usize,
    selected: usize,
    pending: Option<Action>,
    drafts: std::collections::HashMap<Option<PathBuf>, String>,
}
pub struct Handle(HWND);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.0)).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}
impl Handle {
    pub fn refresh_theme(&self, dark: bool) {
        unsafe {
            if let Some(s) = state(self.0) {
                s.dark = dark;
                let _ = DeleteObject(s.brush.into());
                s.brush = CreateSolidBrush(COLORREF(if dark { 0x382e28 } else { 0xfbf7f4 }));
                let _ = InvalidateRect(Some(s.edit), None, false);
            }
            let _ = InvalidateRect(Some(self.0), None, false);
        }
    }

    pub fn new(owner: HWND) -> Result<Self> {
        unsafe {
            let module = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: module.into(),
                lpszClassName: w!("MochiCaptureWindow"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            let _ = RegisterClassW(&class);
            let state = Box::new(State {
                owner,
                root: None,
                dark: false,
                error: String::new(),
                edit: HWND::default(),
                old_edit: 0,
                font: HFONT::default(),
                brush: CreateSolidBrush(COLORREF(0xfbf7f4)),
                composing: false,
                renderer: Renderer::new()?,
                pages: Default::default(),
                page: 0,
                selected: 0,
                pending: None,
                drafts: Default::default(),
            });
            let h = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("MochiCaptureWindow"),
                w!("墨池 · 快捷空间"),
                // 不用 WS_BORDER：系统非客户区是方角灰线，会破坏自绘的悬浮卡片。
                WS_POPUP,
                0,
                0,
                800,
                520,
                Some(owner),
                None,
                Some(module.into()),
                Some(Box::into_raw(state) as *const _),
            )?;
            Ok(Self(h))
        }
    }
    pub fn hide_if_visible(&self) -> bool {
        unsafe {
            if IsWindowVisible(self.0).as_bool() {
                let _ = ShowWindow(self.0, SW_HIDE);
                true
            } else {
                false
            }
        }
    }
    pub fn take_action(&self) -> Option<Action> {
        unsafe { state(self.0).and_then(|s| s.pending.take()) }
    }
    pub fn show(&self, root: Option<PathBuf>, dark: bool, pages: [Vec<Entry>; 5]) {
        unsafe {
            let Some(s) = state(self.0) else { return };
            if s.root != root {
                s.drafts.insert(s.root.clone(), edit_text(s.edit));
                let draft = s.drafts.remove(&root).unwrap_or_default();
                let _ = SetWindowTextW(s.edit, &windows::core::HSTRING::from(draft));
                s.page = 0;
            }
            s.pages = pages;
            s.selected = s.selected.min(s.pages[s.page].len().saturating_sub(1));
            s.root = root;
            s.dark = dark;
            s.error.clear();
            let _ = DeleteObject(s.brush.into());
            // EDIT 只负责文字、IME 和剪贴板；它的底色与 capture_view 画出的输入框一致。
            s.brush = CreateSolidBrush(COLORREF(if dark { 0x382e28 } else { 0xfbf7f4 }));
            let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(s.owner).max(96) as i32;
            let mut point = Default::default();
            let _ = GetCursorPos(&mut point);
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let width = (800 * dpi / 96).min(info.rcWork.right - info.rcWork.left);
            let height = (520 * dpi / 96).min(info.rcWork.bottom - info.rcWork.top);
            let left = info.rcWork.left + (info.rcWork.right - info.rcWork.left - width) / 2;
            let top = info.rcWork.top + (info.rcWork.bottom - info.rcWork.top - height) / 2;
            let _ = SetWindowPos(
                self.0,
                Some(HWND_TOPMOST),
                left,
                top,
                width,
                height,
                SWP_SHOWWINDOW,
            );
            let _ = ShowWindow(self.0, SW_SHOW);
            let _ = SetForegroundWindow(self.0);
            layout_edit(self.0, s);
            let _ = SetFocus(Some(if s.page == 0 { s.edit } else { self.0 }));
            let _ = SendMessageW(
                s.edit,
                EM_SETSEL,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(-1)),
            );
            let _ = InvalidateRect(Some(self.0), None, false);
        }
    }
}
unsafe fn state(hwnd: HWND) -> Option<&'static mut State> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    unsafe { ptr.as_mut() }
}
fn layout_edit(hwnd: HWND, s: &State) {
    let area = crate::platform::client_size_dips(hwnd);
    let l = capture_view::layout(Rect::new(0.0, 0.0, area.0, area.1));
    let scale = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) } as f32 / 96.0;
    unsafe {
        let _ = ShowWindow(s.edit, if s.page == 0 { SW_SHOW } else { SW_HIDE });
        let _ = MoveWindow(
            s.edit,
            ((l.input.left + 14.0) * scale) as i32,
            ((l.input.top + 46.0) * scale) as i32,
            ((l.input.width() - 28.0) * scale) as i32,
            ((l.input.height() - 60.0) * scale) as i32,
            true,
        );
    }
}
fn update_font(hwnd: HWND, s: &mut State) {
    unsafe {
        let dpi = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as i32;
        let mut font = LOGFONTW {
            lfHeight: -(16 * dpi / 96),
            ..Default::default()
        };
        let name = "Microsoft YaHei".encode_utf16().collect::<Vec<_>>();
        font.lfFaceName[..name.len()].copy_from_slice(&name);
        let next = CreateFontIndirectW(&font);
        if !next.is_invalid() {
            SendMessageW(
                s.edit,
                WM_SETFONT,
                Some(WPARAM(next.0 as usize)),
                Some(LPARAM(1)),
            );
            let _ = DeleteObject(s.font.into());
            s.font = next;
        }
    }
}
fn edit_text(edit: HWND) -> String {
    unsafe {
        let mut text = vec![0u16; GetWindowTextLengthW(edit).max(0) as usize + 1];
        let n = GetWindowTextW(edit, &mut text);
        String::from_utf16_lossy(&text[..n.max(0) as usize])
    }
}
fn submit(hwnd: HWND, s: &mut State) {
    if s.composing {
        return;
    }
    let Some(root) = &s.root else { return };
    unsafe {
        let value = edit_text(s.edit);
        match mochi_core::capture::CaptureService::new(root).add(&value, "quick-capture") {
            Ok(Some(_)) => {
                let _ = SetWindowTextW(s.edit, w!(""));
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = PostMessageW(
                    Some(s.owner),
                    crate::platform::WM_APP_FILES_CHANGED,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            Ok(None) => {}
            Err(e) => {
                s.error = e.to_string();
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }
}
fn switch_page(hwnd: HWND, s: &mut State, page: usize) {
    s.page = page % capture_view::PAGES.len();
    s.selected = 0;
    s.error.clear();
    layout_edit(hwnd, s);
    unsafe {
        let _ = SetFocus(Some(if s.page == 0 { s.edit } else { hwnd }));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}
fn move_selection(hwnd: HWND, s: &mut State, delta: isize) {
    s.selected = s
        .selected
        .saturating_add_signed(delta)
        .min(s.pages[s.page].len().saturating_sub(1));
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}
fn open_action(hwnd: HWND, s: &mut State, action: Action) {
    s.pending = Some(action);
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = PostMessageW(Some(s.owner), OPEN_ITEM, WPARAM(0), LPARAM(0));
    }
}
fn handle_key(hwnd: HWND, s: &mut State, key: usize, editing: bool) -> bool {
    unsafe {
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let alt = GetKeyState(VK_MENU.0 as i32) < 0;
        // 即使全局快捷键被其他应用占用，本地操作仍可使用。
        if key == 32 && ctrl && shift && !alt {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return true;
        }
        if s.composing {
            return false;
        }
        if key == 27 {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return true;
        }
        if (key == 37 || key == 39) && (alt || !editing || GetWindowTextLengthW(s.edit) == 0) {
            switch_page(hwnd, s, (s.page + if key == 37 { 4 } else { 1 }) % 5);
            return true;
        }
        if key == 9 {
            let _ = SetFocus(Some(if s.page == 0 && !editing {
                s.edit
            } else {
                hwnd
            }));
            return true;
        }
        if s.page == 0 && key == 13 && ctrl {
            submit(hwnd, s);
            return true;
        }
        if s.page == 0 && key == 13 && !editing {
            let _ = SetFocus(Some(s.edit));
            return true;
        }
        if s.page != 0 {
            if key == 38 || key == 40 {
                move_selection(hwnd, s, if key == 38 { -1 } else { 1 });
                return true;
            }
            if key == 33 || key == 34 {
                let (w, h) = crate::platform::client_size_dips(hwnd);
                let count = capture_view::capacity(Rect::new(0.0, 0.0, w, h)) as isize;
                move_selection(hwnd, s, if key == 33 { -count } else { count });
                return true;
            }
            if key == 13 {
                if let Some(entry) = s.pages[s.page].get(s.selected) {
                    let action = entry.action.clone();
                    open_action(hwnd, s, action);
                }
                return true;
            }
        }
    }
    false
}
extern "system" fn edit_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd).unwrap_or_default();
        let Some(s) = state(parent) else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        if msg == WM_IME_STARTCOMPOSITION {
            s.composing = true;
        }
        if msg == WM_IME_ENDCOMPOSITION {
            s.composing = false;
        }
        if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && handle_key(parent, s, wparam.0, true) {
            return LRESULT(0);
        }
        if msg == WM_CHAR
            && (matches!(wparam.0, 9 | 10 | 27)
                || (wparam.0 == 32
                    && GetKeyState(VK_CONTROL.0 as i32) < 0
                    && GetKeyState(VK_SHIFT.0 as i32) < 0))
        {
            return LRESULT(0);
        }
        let old: WNDPROC = std::mem::transmute(s.old_edit);
        CallWindowProcW(old, hwnd, msg, wparam, lparam)
    }
}
extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CREATE => {
                if let Some(s) = state(hwnd) {
                    let style = WS_CHILD
                        | WS_VISIBLE
                        | WINDOW_STYLE(
                            ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32,
                        );
                    if let Ok(edit) = CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        w!("EDIT"),
                        w!(""),
                        style,
                        0,
                        0,
                        100,
                        70,
                        Some(hwnd),
                        None,
                        None,
                        None,
                    ) {
                        s.edit = edit;
                        s.old_edit =
                            SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
                        update_font(hwnd, s);
                        SendMessageW(
                            edit,
                            EM_SETLIMITTEXT,
                            Some(WPARAM(mochi_core::capture::MAX_CONTENT_LENGTH)),
                            Some(LPARAM(0)),
                        );
                        layout_edit(hwnd, s);
                    }
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                if let Some(s) = state(hwnd) {
                    update_font(hwnd, s);
                    let r = &*(lparam.0 as *const windows::Win32::Foundation::RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    layout_edit(hwnd, s);
                }
                LRESULT(0)
            }
            WM_SIZE => {
                if let Some(s) = state(hwnd) {
                    layout_edit(hwnd, s);
                    let size = crate::platform::client_size_dips(hwnd);
                    let scale = windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) as f32 / 96.0;
                    s.renderer
                        .resize((size.0 * scale) as u32, (size.1 * scale) as u32);
                }
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                if let Some(s) = state(hwnd) {
                    let (w, h) = crate::platform::client_size_dips(hwnd);
                    let mut list = DrawList::new();
                    let name = s
                        .root
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|p| p.to_string_lossy());
                    capture_view::paint(
                        &mut list,
                        Rect::new(0.0, 0.0, w, h),
                        name.as_deref(),
                        &s.error,
                        s.dark,
                        s.page,
                        &s.pages[s.page],
                        s.selected,
                    );
                    let _ =
                        s.renderer
                            .present(hwnd, if s.dark { 0x20242b } else { 0xf4f7fb }, &list);
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => {
                if let Some(s) = state(hwnd) {
                    let dc = HDC(wparam.0 as *mut _);
                    SetTextColor(dc, COLORREF(if s.dark { 0xeeedec } else { 0x27221e }));
                    SetBkColor(dc, COLORREF(if s.dark { 0x382e28 } else { 0xfbf7f4 }));
                    return LRESULT(s.brush.0 as isize);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_LBUTTONUP => {
                if let Some(s) = state(hwnd) {
                    let (x, y) = crate::platform::mouse_dips(hwnd, lparam);
                    let (w, h) = crate::platform::client_size_dips(hwnd);
                    let area = Rect::new(0.0, 0.0, w, h);
                    let l = capture_view::layout(area);
                    if l.close.contains(x, y) {
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    } else if let Some(page) = l.tabs.iter().position(|r| r.contains(x, y)) {
                        switch_page(hwnd, s, page);
                    } else if l.save.contains(x, y) {
                        if s.page == 0 {
                            submit(hwnd, s);
                        } else {
                            open_action(hwnd, s, Action::Page(s.page));
                        }
                    } else if s.page != 0 {
                        let count = capture_view::capacity(area);
                        let offset = s.selected / count * count;
                        for row in 0..count {
                            if capture_view::row_rect(area, row).contains(x, y) {
                                if let Some(entry) = s.pages[s.page].get(offset + row) {
                                    let action = entry.action.clone();
                                    open_action(hwnd, s, action);
                                }
                                break;
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if let Some(s) = state(hwnd) {
                    if handle_key(hwnd, s, wparam.0, false) {
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_MOUSEWHEEL => {
                if let Some(s) = state(hwnd) {
                    if s.page != 0 {
                        move_selection(hwnd, s, if ((wparam.0 >> 16) as i16) > 0 { -1 } else { 1 });
                    }
                }
                LRESULT(0)
            }
            WM_ACTIVATE if wparam.0 & 0xffff == WA_INACTIVE as usize => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !ptr.is_null() {
                    let s = Box::from_raw(ptr);
                    let _ = DeleteObject(s.font.into());
                    let _ = DeleteObject(s.brush.into());
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_native_edit_preserves_unicode_draft_and_saves_once() {
        let root = std::env::temp_dir().join(format!(
            "mochi-capture-window-{}-{}",
            std::process::id(),
            mochi_core::jstime::now_millis()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let handle = Handle::new(HWND::default()).unwrap();
        unsafe {
            assert!(!IsWindowVisible(handle.0).as_bool());
            let s = state(handle.0).unwrap();
            s.root = Some(root.clone());
            SetWindowTextW(
                s.edit,
                &windows::core::HSTRING::from("中文捕获 😀\r\n第二行"),
            )
            .unwrap();
            s.composing = true;
            submit(handle.0, s);
            assert!(mochi_core::capture::CaptureService::new(&root)
                .list_items("inbox")
                .is_empty());
            s.composing = false;
            submit(handle.0, s);
            let items = mochi_core::capture::CaptureService::new(&root).list_items("inbox");
            assert_eq!(items.len(), 1);
            assert!(items[0].content.contains("中文捕获 😀"));
            assert!(items[0].content.contains("第二行"));
            assert_eq!(items[0].source, "quick-capture");
            assert_eq!(GetWindowTextLengthW(s.edit), 0);
            submit(handle.0, s);
            assert_eq!(
                mochi_core::capture::CaptureService::new(&root)
                    .list_items("inbox")
                    .len(),
                1
            );
            assert!(!IsWindowVisible(handle.0).as_bool());
        }
        drop(handle);
        std::fs::remove_dir_all(root).unwrap();
    }
}
