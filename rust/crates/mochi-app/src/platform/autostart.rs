//! Windows 登录启动项。
//!
//! 这个设置刻意在这里实现，而不是通过 shell 命令或安装器任务。
//! HKCU\Run 是按用户的，不需要提权，且跟随实际运行的
//! 可执行文件（便携版和安装版因此行为一致）。

use std::path::Path;

/// 值名在安装版与便携版之间保持稳定：升级应用时替换同一条目，
/// 而不是越攒越多。
pub const RUN_VALUE_NAME: &str = "Mochi";
pub const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const STARTUP_APPROVED_KEY_PATH: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
pub const AUTOSTART_ARGUMENT: &str = "--autostart";

/// 用户偏好与 Windows 同步的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncResult {
    /// Run 值现在指向当前可执行文件的命令行。
    Enabled,
    /// Run 值本来就是想要的命令行。
    AlreadyEnabled,
    /// Windows 的 StartupApproved 状态显示用户已停用该条目。
    /// 该状态和 Run 值都不动。
    SkippedByWindows,
    /// Run 值已被删除。
    Disabled,
    /// 本来就没有要删的 Run 值。
    AlreadyDisabled,
}

/// 同步发布版应用的 HKCU 登录条目。
///
/// 调试/测试二进制绝不能改坏开发者真实的启动注册表。
/// 若某个仅调试用的 UI 调了这个设置，调用方仍可把这个错误
/// 呈现出来；发布构建走下面的真正实现。
pub fn sync(enabled: bool) -> anyhow::Result<SyncResult> {
    if cfg!(debug_assertions) || cfg!(test) || std::env::var_os("MOCHI_VERIFY_OFFSCREEN").is_some()
    {
        anyhow::bail!("开发/测试/验收构建不会修改 Windows 登录启动项")
    }

    #[cfg(windows)]
    {
        return sync_windows(enabled);
    }

    #[cfg(not(windows))]
    {
        let _ = enabled;
        anyhow::bail!("Windows 登录启动项仅在 Windows 发布构建中可用")
    }
}

/// 为 Run 值构造带引号的命令行。保持纯函数，测试空格、引号和
/// 尾随反斜杠时才不用写用户的注册表。
pub fn command_line_for(executable: &Path) -> String {
    format!(
        "{} {AUTOSTART_ARGUMENT}",
        quote_windows_argument(executable)
    )
}

/// 解读 StartupApproved\Run 二进制值的第一个字节。
///
/// 资源管理器/任务管理器为用户停用的启动项写 `0x03`，
/// 启用的写 `0x02`。未知/过短的值保守地按停用处理，
/// 应用更新才不会把畸形或未来格式意外变成「开机自启」。
pub fn startup_approved_is_disabled(bytes: &[u8]) -> bool {
    bytes.first().copied() != Some(0x02)
}

/// 按 CommandLineToArgvW 的转义规则给一个 Windows 命令行参数加引号。
/// 路径里通常不会出现引号，但处理一下才能让这个助手
/// 在测试和非常规路径来源下也保持正确。
fn quote_windows_argument(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut output = String::with_capacity(text.len() + 2);
    output.push('"');
    let mut backslashes = 0usize;
    for ch in text.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                output.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                output.push('"');
                backslashes = 0;
            }
            _ => {
                output.extend(std::iter::repeat_n('\\', backslashes));
                output.push(ch);
                backslashes = 0;
            }
        }
    }
    // 收尾引号前紧挨着的反斜杠必须翻倍。
    output.extend(std::iter::repeat_n('\\', backslashes * 2));
    output.push('"');
    output
}

#[cfg(windows)]
fn sync_windows(enabled: bool) -> anyhow::Result<SyncResult> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, REG_SZ,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn check(operation: &str, status: WIN32_ERROR) -> anyhow::Result<()> {
        status
            .is_ok()
            .then_some(())
            .ok_or_else(|| anyhow::anyhow!("{operation} 失败（Win32 错误码 {}）", status.0))
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open_key(
        path: &str,
        access: windows::Win32::System::Registry::REG_SAM_FLAGS,
    ) -> anyhow::Result<Option<Key>> {
        let path = wide(path);
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                access,
                &mut key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            Ok(None)
        } else {
            check("打开注册表键", status)?;
            Ok(Some(Key(key)))
        }
    }

    fn user_disabled() -> anyhow::Result<bool> {
        let Some(key) = open_key(STARTUP_APPROVED_KEY_PATH, KEY_QUERY_VALUE)? else {
            return Ok(false);
        };
        let value = wide(RUN_VALUE_NAME);
        let mut value_type = REG_BINARY;
        let mut size = 0u32;
        let status = unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                PCWSTR(value.as_ptr()),
                windows::Win32::System::Registry::RRF_RT_REG_BINARY,
                Some(&mut value_type),
                None,
                Some(&mut size),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        check("读取 Windows 启动项状态", status)?;
        if size == 0 {
            // 存在但为空/畸形的标记并不能证明 Windows 允许该条目。
            // 从严处理：保持 OS 状态原样不动。
            return Ok(true);
        }
        let mut bytes = vec![0u8; size as usize];
        let status = unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                PCWSTR(value.as_ptr()),
                windows::Win32::System::Registry::RRF_RT_REG_BINARY,
                Some(&mut value_type),
                Some(bytes.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        check("读取 Windows 启动项状态", status)?;
        Ok(startup_approved_is_disabled(
            &bytes[..(size as usize).min(bytes.len())],
        ))
    }

    if enabled && user_disabled()? {
        // 不调用 RegSetValueExW，也不碰 StartupApproved。
        // 用户在 OS 层的选择优先于应用内偏好。
        return Ok(SyncResult::SkippedByWindows);
    }

    if enabled {
        let key_path = wide(RUN_KEY_PATH);
        let mut key = HKEY::default();
        let status =
            unsafe { RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(key_path.as_ptr()), &mut key) };
        check("创建 Windows 登录启动项", status)?;
        let key = Key(key);
        let value_name = wide(RUN_VALUE_NAME);
        let command = command_line_for(&std::env::current_exe()?);
        let command = wide(&command);
        let mut value_type = REG_SZ;
        let mut old_size = 0u32;
        let current = unsafe {
            RegGetValueW(
                key.0,
                PCWSTR::null(),
                PCWSTR(value_name.as_ptr()),
                windows::Win32::System::Registry::RRF_RT_REG_SZ,
                Some(&mut value_type),
                None,
                Some(&mut old_size),
            )
        };
        if current.is_ok() && old_size == (command.len() * std::mem::size_of::<u16>()) as u32 {
            let mut old = vec![0u16; old_size as usize / 2];
            let mut read_size = old_size;
            let second = unsafe {
                RegGetValueW(
                    key.0,
                    PCWSTR::null(),
                    PCWSTR(value_name.as_ptr()),
                    windows::Win32::System::Registry::RRF_RT_REG_SZ,
                    Some(&mut value_type),
                    Some(old.as_mut_ptr().cast()),
                    Some(&mut read_size),
                )
            };
            if second.is_ok() && old == command {
                return Ok(SyncResult::AlreadyEnabled);
            }
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(
                command.as_ptr().cast::<u8>(),
                command.len() * std::mem::size_of::<u16>(),
            )
        };
        let status = unsafe {
            RegSetValueExW(
                key.0,
                PCWSTR(value_name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(bytes),
            )
        };
        check("写入 Windows 登录启动项", status)?;
        Ok(SyncResult::Enabled)
    } else {
        let Some(key) = open_key(RUN_KEY_PATH, KEY_SET_VALUE)? else {
            return Ok(SyncResult::AlreadyDisabled);
        };
        let value_name = wide(RUN_VALUE_NAME);
        let status = unsafe { RegDeleteValueW(key.0, PCWSTR(value_name.as_ptr())) };
        if status == ERROR_FILE_NOT_FOUND {
            Ok(SyncResult::AlreadyDisabled)
        } else {
            check("移除 Windows 登录启动项", status)?;
            Ok(SyncResult::Disabled)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn command_line_quotes_spaces_and_keeps_autostart_argument() {
        assert_eq!(
            command_line_for(Path::new(r"C:\Program Files\Mochi\Mochi.exe")),
            r#""C:\Program Files\Mochi\Mochi.exe" --autostart"#
        );
    }

    #[test]
    fn command_line_doubles_trailing_backslashes_before_quote() {
        let command = command_line_for(Path::new(r"C:\Mochi\"));
        assert!(command.starts_with(r#""C:\Mochi\\"#));
        assert!(command.ends_with("\" --autostart"));
    }

    #[test]
    fn startup_approved_only_recognises_the_known_disabled_marker() {
        assert!(startup_approved_is_disabled(&[0x03, 0, 0, 0]));
        assert!(!startup_approved_is_disabled(&[0x02, 0, 0, 0]));
        assert!(startup_approved_is_disabled(&[]));
        assert!(startup_approved_is_disabled(&[0xff]));
    }

    #[test]
    fn tests_do_not_touch_the_real_startup_registry() {
        // `sync` 在调试/测试构建中刻意不可用。这条断言守住构建闸门，
        // 且不触发任何注册表 API。
        assert!(cfg!(debug_assertions) || cfg!(test));
        assert!(sync(true).is_err());
        let _ = PathBuf::from(RUN_KEY_PATH);
    }
}
