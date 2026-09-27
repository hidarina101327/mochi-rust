//! 桌面卡片和登录启动所用的常驻入口及单实例路由。
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, POINT, WPARAM,
        },
        System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

pub const MESSAGE: u32 = WM_APP + 82;
pub const ACTIVATE: u32 = WM_APP + 83;
const ICON_ID: u32 = 84;
#[derive(Default)]
pub struct Tray {
    hwnd: HWND,
    registered: bool,
}

fn payload(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = MESSAGE;
    data.hIcon = unsafe {
        GetModuleHandleW(None)
            .ok()
            .and_then(|m| LoadIconW(Some(m.into()), PCWSTR(101usize as *const u16)).ok())
            .or_else(|| LoadIconW(None, IDI_APPLICATION).ok())
            .unwrap_or_default()
    };
    let text: Vec<_> = "墨池 · 桌面卡片".encode_utf16().collect();
    data.szTip[..text.len()].copy_from_slice(&text);
    data
}

impl Tray {
    pub fn ensure(&mut self, hwnd: HWND) -> bool {
        if self.registered && self.hwnd == hwnd {
            return true;
        }
        self.clear();
        self.hwnd = hwnd;
        let mut data = payload(hwnd);
        self.registered = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
        if self.registered {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
        self.registered
    }
    pub fn available(&self) -> bool {
        self.registered
    }
    pub fn recover(&mut self, hwnd: HWND) -> bool {
        self.registered = false;
        self.ensure(hwnd)
    }
    fn clear(&mut self) {
        if self.registered {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &payload(self.hwnd));
            }
        }
        self.registered = false;
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        self.clear();
    }
}

pub fn event(lp: LPARAM) -> Option<u32> {
    let v = lp.0 as u32;
    (v >> 16 == ICON_ID).then_some(v & 0xffff)
}
pub fn activation(event: u32) -> bool {
    event == NIN_SELECT || event == (NIN_SELECT | 1) || event == WM_LBUTTONDBLCLK
}

pub fn menu(hwnd: HWND, paused: bool) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let _ = AppendMenuW(menu, MF_STRING, 1, w!("打开墨池"));
        let _ = AppendMenuW(menu, MF_STRING, 2, w!("管理桌面卡片…"));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            3,
            if paused {
                w!("恢复桌面卡片")
            } else {
                w!("暂时隐藏全部卡片")
            },
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 4, w!("退出墨池（同时关闭卡片）"));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            Some(0),
            hwnd,
            None,
        )
        .0 as u32;
        let _ = DestroyMenu(menu);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        selected
    }
}

pub struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// 返回 `None` 表示已有实例已接管启动；出错时仍可继续正常启动。
pub fn acquire(autostart: bool) -> windows::core::Result<Option<Instance>> {
    unsafe {
        let mutex = CreateMutexW(None, false, w!("Local\\MochiNativeDesktop.Instance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(hwnd) = FindWindowW(w!("MochiNativeWindow"), None) {
                if !autostart {
                    let _ = PostMessageW(Some(hwnd), ACTIVATE, WPARAM(0), LPARAM(0));
                }
                let _ = CloseHandle(mutex);
                return Ok(None);
            }
            // 首个实例可能还在初始化，避免同时启动第二个写入者。
            let _ = CloseHandle(mutex);
            return Ok(None);
        }
        Ok(Some(Instance(mutex)))
    }
}

pub fn isolated() -> bool {
    cfg!(test)
        || [
            "MOCHI_VERIFY_OFFSCREEN",
            "MOCHI_SETTINGS_FILE",
            "MOCHI_SETTINGS_PATH",
        ]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
}
