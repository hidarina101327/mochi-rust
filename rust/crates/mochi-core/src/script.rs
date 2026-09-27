//! 经审批的多行脚本，使用应用自带的 Python 运行时。
//! 这些是普通 OS 进程，不是文件系统/网络安全沙箱。
use crate::shell::ShellRunResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

pub const PYTHON_VERSION: &str = "3.14.7";
pub const MEMORY_LIMIT_MIB: usize = 512;
pub const GUIDE: &str = include_str!("../assets/script-guide.md");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Powershell,
    Cmd,
}
impl Language {
    pub fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Powershell => "powershell",
            Self::Cmd => "cmd",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Python => "py",
            Self::Powershell => "ps1",
            Self::Cmd => "cmd",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptSpec {
    pub language: Language,
    pub code: String,
    pub summary: String,
    /// 明确写清意图，绝不等于声明「这段代码只读」。
    pub intent: Intent,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Intent {
    Inspect,
    Preview,
    Apply,
}
fn default_timeout() -> u64 {
    120_000
}

impl ScriptSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.code.trim().is_empty() || self.code.len() > 65_536 || self.code.contains('\0') {
            return Err("脚本不能为空、含 NUL 或超过 64 KiB".into());
        }
        if self.summary.trim().is_empty() || self.summary.len() > 2000 {
            return Err("summary 必须说明目标、范围和预期结果（最多 2000 字节）".into());
        }
        if !(1000..=300_000).contains(&self.timeout_ms) {
            return Err("timeoutMs 必须在 1000–300000 之间".into());
        }
        if self.input.to_string().len() > 1_048_576 {
            return Err("input 不得超过 1 MiB；大数据应按路径流式读取".into());
        }
        Ok(())
    }
    pub fn working_directory(&self, workspace: &Path) -> Result<PathBuf, String> {
        self.resolve_directory(workspace, false)
    }
    fn resolve_directory(&self, workspace: &Path, trusted: bool) -> Result<PathBuf, String> {
        let root = workspace.canonicalize().map_err(|e| e.to_string())?;
        let cwd = self.cwd.as_deref().map(Path::new).unwrap_or(&root);
        let cwd = if cwd.is_absolute() {
            cwd.to_path_buf()
        } else {
            root.join(cwd)
        };
        let cwd = cwd
            .canonicalize()
            .map_err(|e| format!("工作目录不可用：{e}"))?;
        if !cwd.is_dir() {
            return Err("脚本工作目录不是文件夹".into());
        }
        if !trusted && !cwd.starts_with(&root) {
            return Err("脚本工作目录必须位于当前工作区内；这仅限制启动目录，不是安全沙箱".into());
        }
        Ok(cwd)
    }
}

/// 不做 PATH 查找、不走 Python 启动器、注册表、工作区可执行文件或自动安装回退。
pub fn application_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("无法找到应用目录")?;
    // Cargo 的测试/链接产物在 <profile>/deps 下。
    Ok(if dir.file_name().is_some_and(|n| n == "deps") {
        dir.parent().unwrap_or(dir)
    } else {
        dir
    }
    .to_path_buf())
}
pub fn runtime_path(language: Language, app_dir: &Path) -> Result<PathBuf, String> {
    let path = match language {
        Language::Python => {
            let dir = app_dir.join("runtime/python");
            for name in [
                "python314.zip",
                "python314.dll",
                "python314._pth",
                "LICENSE.txt",
                "mochi-runtime.json",
            ] {
                if !dir.join(name).is_file() {
                    return Err("内置 Python 未完整安装；请修复墨池安装包。开发环境运行 rust/tools/prepare-python-runtime.ps1，不要调用系统 Python 或在线安装依赖".into());
                }
            }
            let manifest: Value = serde_json::from_slice(
                &fs::read(dir.join("mochi-runtime.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if manifest["version"].as_str() != Some(PYTHON_VERSION) {
                return Err("内置 Python 版本不匹配，请修复安装".into());
            }
            dir.join("python.exe")
        }
        Language::Powershell | Language::Cmd => {
            if !cfg!(windows) {
                return Err("此脚本语言仅在 Windows 上可用".into());
            }
            let system = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot 不可用")?)
                .join("System32");
            system.join(if language == Language::Cmd {
                "cmd.exe"
            } else {
                "WindowsPowerShell/v1.0/powershell.exe"
            })
        }
    };
    if !path.is_file() {
        return Err(format!("{} 运行环境不可用", language.name()));
    }
    Ok(path)
}
pub fn environment() -> Value {
    let app = application_dir();
    let runtimes: Vec<Value> = [Language::Python, Language::Powershell, Language::Cmd].into_iter().map(|language| {
        let result = app.as_ref().map_err(Clone::clone).and_then(|d| runtime_path(language,d));
        match result {
            Ok(path) => json!({"language":language,"available":true,"executable":path,"version":if language==Language::Python {Some(PYTHON_VERSION)} else {None},"bundled":language==Language::Python}),
            Err(error) => json!({"language":language,"available":false,"error":error})
        }
    }).collect();
    json!({"runtimes":runtimes,"requiresApproval":true,"sandboxed":false,"previewIsEnforcedReadOnly":false,"maxCodeBytes":65536,"maxInputBytes":1048576,"maxProcessTreeMemoryMiB":if cfg!(windows) {Some(MEMORY_LIMIT_MIB)} else {None},"guide":GUIDE})
}

/// 解析持久化的元数据，并证明用户审阅过的源码和目录与实际执行的完全一致。
/// 无效的脚本元数据绝不变成 shell 代码。
pub fn approved_spec(data: &Value, source: &str, cwd: &str) -> Result<ScriptSpec, String> {
    let spec: ScriptSpec =
        serde_json::from_value(data.get("script").cloned().ok_or("脚本参数缺失")?)
            .map_err(|e| format!("脚本参数无效：{e}"))?;
    spec.validate()?;
    if spec.code != source || spec.cwd.as_deref() != Some(cwd) {
        return Err("脚本与审批内容不一致；请重新提案".into());
    }
    Ok(spec)
}

pub fn run_approved(
    data: &Value,
    source: &str,
    cwd: &str,
    workspace: &Path,
    cancel: &AtomicBool,
) -> ShellRunResult {
    let result = (|| {
        let spec = approved_spec(data, source, cwd)?;
        let app_dir = application_dir()?;
        let executable = runtime_path(spec.language, &app_dir)?;
        if data["runtime"].as_str() != executable.to_str() {
            return Err("脚本运行环境已变化，请重新提案".into());
        }
        run(&spec, workspace, &app_dir, cancel)
    })();
    result.unwrap_or_else(ShellRunResult::failed)
}

fn run(
    spec: &ScriptSpec,
    workspace: &Path,
    app_dir: &Path,
    cancel: &AtomicBool,
) -> Result<ShellRunResult, String> {
    run_with_mode(spec, workspace, app_dir, cancel, false)
}

/// 工作流是用户明确信任的自动化，拥有普通 OS 访问权限。
/// AI 提出的脚本仍走 run_approved；信任只适用于工作流运行。
pub fn run_trusted(spec: &ScriptSpec, workspace: &Path, cancel: &AtomicBool) -> ShellRunResult {
    application_dir()
        .and_then(|app| run_with_mode(spec, workspace, &app, cancel, true))
        .unwrap_or_else(ShellRunResult::failed)
}

fn run_with_mode(
    spec: &ScriptSpec,
    workspace: &Path,
    app_dir: &Path,
    cancel: &AtomicBool,
    trusted: bool,
) -> Result<ShellRunResult, String> {
    spec.validate()?;
    let cwd = spec.resolve_directory(workspace, trusted)?;
    let runtime = runtime_path(spec.language, app_dir)?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(ShellRunResult::failed("脚本已取消"));
    }
    let scratch = Scratch::new()?;
    let code_path = scratch
        .0
        .join(format!("script.{}", spec.language.extension()));
    let source = match spec.language {
        Language::Python => spec.code.clone(),
        // 带 BOM 才能在 Windows PowerShell 5.1 下正确显示中文。
        Language::Powershell => format!("\u{feff}{}", spec.code),
        Language::Cmd => format!(
            "@echo off\r\nchcp 65001 >nul\r\n{}",
            spec.code.replace("\r\n", "\n").replace('\n', "\r\n")
        ),
    };
    scratch.write(&code_path, source.as_bytes())?;
    let input_path = scratch.0.join("input.json");
    scratch.write(&input_path, spec.input.to_string().as_bytes())?;
    let mut command = Command::new(&runtime);
    match spec.language {
        Language::Python => {
            command.args(["-I", "-X", "utf8", "-u"]).arg(&code_path);
        }
        Language::Powershell => {
            // 应用继承了 PS 7 的环境时，优先加载 PS 5.1 自带的模块。
            let mut module_paths = vec![runtime.parent().unwrap().join("Modules")];
            if let Some(existing) = std::env::var_os("PSModulePath") {
                module_paths.extend(std::env::split_paths(&existing));
            }
            command.env(
                "PSModulePath",
                std::env::join_paths(module_paths).map_err(|e| e.to_string())?,
            );
            let bootstrap = scratch.0.join("bootstrap.ps1");
            scratch.write(&bootstrap, b"$ErrorActionPreference = 'Stop'\n[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n$global:LASTEXITCODE = 0\n& $env:MOCHI_SCRIPT_SOURCE\nif ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }\n")?;
            if trusted {
                // 只对这条受信任的工作流做进程级策略，机器/用户级策略不动。
                command.args(["-ExecutionPolicy", "Bypass"]);
            }
            command
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
                .arg(bootstrap)
                .env("MOCHI_SCRIPT_SOURCE", &code_path);
        }
        Language::Cmd => {
            // 文件名固定，不拼接任何输入/参数；/D 禁用注册表 AutoRun。
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let path = code_path.to_str().ok_or("脚本路径编码无效")?;
                if path.contains(['"', '%', '\r', '\n']) {
                    return Err("临时目录包含 CMD 不支持的字符，请使用 Python".into());
                }
                command.raw_arg(format!("/D /Q /S /C \"\"{path}\"\""));
            }
            #[cfg(not(windows))]
            return Err("CMD 仅在 Windows 上可用".into());
        }
    }
    command
        .current_dir(cwd)
        .env("MOCHI_SCRIPT_INPUT", &input_path)
        .env(
            "MOCHI_WORKSPACE",
            workspace.canonicalize().map_err(|e| e.to_string())?,
        )
        .env(
            "MOCHI_SCRIPT_INTENT",
            serde_json::to_value(spec.intent).unwrap().as_str().unwrap(),
        );
    Ok(crate::shell::run_process(
        &mut command,
        Some(spec.timeout_ms),
        Some(cancel),
        Some(MEMORY_LIMIT_MIB * 1024 * 1024),
    ))
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("mochi-script-{}", crate::paths::random_base36(24)));
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .and_then(|mut f| f.write_all(bytes))
            .map_err(|e| e.to_string())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // 绝不递归进脚本创建的任意路径。
        if fs::symlink_metadata(&self.0).is_ok_and(|m| {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return false;
                }
            }
            !m.file_type().is_symlink()
        }) {
            for name in [
                "script.py",
                "script.ps1",
                "script.cmd",
                "input.json",
                "bootstrap.ps1",
            ] {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }
}

#[cfg(test)]
mod tests;
