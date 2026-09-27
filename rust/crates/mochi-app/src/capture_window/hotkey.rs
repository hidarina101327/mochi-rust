//! 每个进程只注册一个全局截图快捷键，即使同时打开多个工作区窗口也是如此。
use crate::ui::shortcuts::Chord;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use windows::{
    core::Result,
    Win32::{
        Foundation::HWND,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::IsWindow},
    },
};
static REGISTERED: Mutex<Option<(isize, Chord)>> = Mutex::new(None);
static NOTIFIED: AtomicBool = AtomicBool::new(false);
pub fn mark_hotkey_notice() -> bool {
    !NOTIFIED.swap(true, Ordering::Relaxed)
}
fn flags(chord: Chord) -> HOT_KEY_MODIFIERS {
    let mut flags = MOD_NOREPEAT;
    if chord.ctrl {
        flags |= MOD_CONTROL;
    }
    if chord.alt {
        flags |= MOD_ALT;
    }
    if chord.shift {
        flags |= MOD_SHIFT;
    }
    flags
}
pub fn release_hotkey(owner: isize) {
    let mut registered = REGISTERED.lock().unwrap_or_else(|e| e.into_inner());
    if registered.as_ref().is_some_and(|(hwnd, _)| *hwnd == owner) {
        unsafe {
            let _ = UnregisterHotKey(Some(HWND(owner as *mut _)), 1);
        }
        *registered = None;
    }
}
pub fn register_hotkey(owner: isize, chord: Chord) -> Result<()> {
    if owner == 0 || std::env::var_os("MOCHI_VERIFY_OFFSCREEN").is_some() {
        return Ok(());
    }
    let mut registered = REGISTERED.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        if let Some((old_owner, old_chord)) = *registered {
            if IsWindow(Some(HWND(old_owner as *mut _))).as_bool() {
                if old_owner == owner && old_chord == chord {
                    return Ok(());
                }
                let _ = UnregisterHotKey(Some(HWND(old_owner as *mut _)), 1);
            }
        }
        match RegisterHotKey(
            Some(HWND(owner as *mut _)),
            1,
            flags(chord),
            u32::from(chord.key),
        ) {
            Ok(()) => {
                *registered = Some((owner, chord));
                Ok(())
            }
            Err(error) => {
                if let Some((old_owner, old_chord)) = *registered {
                    if RegisterHotKey(
                        Some(HWND(old_owner as *mut _)),
                        1,
                        flags(old_chord),
                        u32::from(old_chord.key),
                    )
                    .is_err()
                    {
                        *registered = None;
                    }
                }
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{core::w, Win32::UI::WindowsAndMessaging::*};
    #[test]
    fn repeated_registration_is_single_and_failed_rebind_keeps_old_key() {
        unsafe {
            let owner = CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("capture-hotkey-test"),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap();
            let chord = Chord::parse("Ctrl+Alt+Shift+F24").unwrap();
            let unavailable = Chord::parse("Ctrl+Alt+Shift+F23").unwrap();
            register_hotkey(owner.0 as isize, chord).unwrap();
            register_hotkey(owner.0 as isize, chord).unwrap();
            RegisterHotKey(None, 91, flags(unavailable), unavailable.key.into()).unwrap();
            assert!(register_hotkey(owner.0 as isize, unavailable).is_err());
            assert!(RegisterHotKey(None, 92, flags(chord), chord.key.into()).is_err());
            UnregisterHotKey(Some(owner), 1).unwrap();
            // 重复刷新后注销快捷键时，释放按键。
            RegisterHotKey(None, 92, flags(chord), chord.key.into()).unwrap();
            UnregisterHotKey(None, 91).unwrap();
            UnregisterHotKey(None, 92).unwrap();
            *REGISTERED.lock().unwrap() = None;
            DestroyWindow(owner).unwrap();
        }
    }
}
