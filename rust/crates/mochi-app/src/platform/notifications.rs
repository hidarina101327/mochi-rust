//! Shell 通知区域 API 同时适用于便携版和已安装的 Win32 程序。
//! <https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw>
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};

const ICON_ID: u32 = 42;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;

#[derive(Default)]
pub struct Notifier {
    hwnd: HWND,
    registered: bool,
}

impl Notifier {
    pub fn show(&mut self, hwnd: HWND, title: &str, message: &str) -> anyhow::Result<()> {
        if hwnd.is_invalid() {
            anyhow::bail!("通知窗口尚未就绪");
        }
        if self.hwnd != hwnd {
            self.clear();
            self.hwnd = hwnd;
        }
        if !self.registered {
            self.register()?;
        }
        let data = payload(hwnd, title, message);
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            // Explorer 可能已经重启并清除了图标；此时重新注册一次。
            self.clear();
            self.hwnd = hwnd;
            self.register()?;
            if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
                self.clear();
                anyhow::bail!("Windows 未接受系统通知");
            }
        }
        Ok(())
    }

    fn register(&mut self) -> anyhow::Result<()> {
        let mut data = identity(self.hwnd);
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = super::WM_APP_NOTIFICATION_CLICK;
        data.hIcon = unsafe {
            let instance = GetModuleHandleW(None)?;
            LoadIconW(Some(instance.into()), PCWSTR(101usize as *const u16))
                .or_else(|_| LoadIconW(None, IDI_APPLICATION))?
        };
        wide_text(&mut data.szTip, "墨池 · 通知中心");
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            anyhow::bail!("Windows 通知区域暂不可用");
        }
        self.registered = true;
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        if !unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) }.as_bool() {
            self.clear();
            anyhow::bail!("无法初始化 Windows 通知回调");
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        if self.registered {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &identity(self.hwnd));
            }
        }
        self.registered = false;
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        self.clear();
    }
}

pub fn is_activation(lparam: LPARAM) -> bool {
    let value = lparam.0 as u32;
    // VERSION_4 将图标 ID 放在 HIWORD，将事件放在 LOWORD。
    value >> 16 == ICON_ID
        && matches!(
            value & 0xffff,
            NIN_BALLOONUSERCLICK | NIN_SELECT | NIN_KEYSELECT
        )
}

fn identity(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

fn payload(hwnd: HWND, title: &str, message: &str) -> NOTIFYICONDATAW {
    let mut data = identity(hwnd);
    // 不要在其他应用的消息后排队处理过期通知；完整历史记录仍保存在 Mochi 中。
    data.uFlags = NIF_INFO | NIF_REALTIME;
    data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
    wide_text(&mut data.szInfoTitle, title);
    wide_text(&mut data.szInfo, message);
    data
}

fn wide_text<const N: usize>(buffer: &mut [u16; N], text: &str) {
    buffer.fill(0);
    let mut offset = 0;
    for c in text.chars() {
        let c = if c.is_control() { ' ' } else { c };
        if offset + c.len_utf16() >= N {
            break;
        }
        let mut encoded = [0; 2];
        let units = c.encode_utf16(&mut encoded);
        buffer[offset..offset + units.len()].copy_from_slice(units);
        offset += units.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_payload_bounds_unicode_without_broken_surrogates_or_nuls() {
        let data = payload(
            HWND::default(),
            &"墨池🦀".repeat(100),
            &format!("中文\0\n{}", "🦀".repeat(200)),
        );
        for buffer in [data.szInfoTitle.as_slice(), data.szInfo.as_slice()] {
            let end = buffer.iter().position(|c| *c == 0).unwrap();
            assert!(end < buffer.len());
            let text = String::from_utf16(&buffer[..end]).unwrap();
            assert!(!text.contains(['\0', '\n']));
            assert!(!text.is_empty());
        }
        assert_eq!(data.uFlags, NIF_INFO | NIF_REALTIME);
        assert!(data.dwInfoFlags.contains(NIIF_RESPECT_QUIET_TIME));
    }

    #[test]
    fn notifications_callback_only_activates_our_icon_on_click_or_keyboard() {
        for event in [NIN_BALLOONUSERCLICK, NIN_SELECT, NIN_KEYSELECT] {
            assert!(is_activation(LPARAM(((ICON_ID << 16) | event) as isize)));
        }
        assert!(!is_activation(LPARAM(
            ((ICON_ID << 16) | NIN_BALLOONTIMEOUT) as isize
        )));
        assert!(!is_activation(LPARAM(
            ((43 << 16) | NIN_BALLOONUSERCLICK) as isize
        )));
    }

    #[test]
    #[ignore = "Sends one Windows notification; run explicitly on an interactive desktop"]
    fn notifications_windows_api_smoke() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use windows::core::w;
        use windows::Win32::Foundation::{LRESULT, WPARAM};
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::WindowsAndMessaging::*;
        static SHOWN: AtomicBool = AtomicBool::new(false);
        extern "system" fn callback(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if msg == super::super::WM_APP_NOTIFICATION_CLICK {
                if lparam.0 as u32 & 0xffff == NIN_BALLOONSHOW {
                    SHOWN.store(true, Ordering::Relaxed);
                }
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        SHOWN.store(false, Ordering::Relaxed);
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let instance = unsafe { GetModuleHandleW(None) }.unwrap();
        let class = w!("MochiNotificationSmokeTest");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(callback),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        assert_ne!(unsafe { RegisterClassW(&window_class) }, 0);
        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let window = TestWindow(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("MochiNotificationTest"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .unwrap()
        });
        let mut notifier = Notifier::default();
        println!("Windows notification state: {:?}", unsafe {
            SHQueryUserNotificationState()
        });
        notifier
            .show(
                window.0,
                "墨池 · 通知功能测试",
                "Windows 系统通知已接通。此消息仅用于功能验证。",
            )
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < deadline && !SHOWN.load(Ordering::Relaxed) {
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, Some(window.0), 0, 0, PM_REMOVE) }.as_bool() {
                unsafe {
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        println!(
            "Windows API accepted notification; NIN_BALLOONSHOW={}",
            SHOWN.load(Ordering::Relaxed)
        );
        notifier.clear();
        assert!(!notifier.registered);
    }
}
