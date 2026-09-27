//! IExpress 只能稳定启动解包后的 `.exe`，不能可靠执行 `.cmd` 或带参数的命令。
//! 此启动器位于同一个解包目录，负责隐藏地执行 `install.ps1` 并将退出码返给 IExpress。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn main() {
    let result = (|| -> Result<(), String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let root = executable
            .parent()
            .ok_or_else(|| "安装启动器路径无效".to_owned())?;
        let script = root.join("install.ps1");
        if !script.is_file() {
            return Err("安装脚本 install.ps1 不存在".into());
        }
        let status = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|error| format!("无法启动安装脚本：{error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("安装脚本退出码：{}", status.code().unwrap_or(-1)))
        }
    })();
    if result.is_err() {
        std::process::exit(1);
    }
}
