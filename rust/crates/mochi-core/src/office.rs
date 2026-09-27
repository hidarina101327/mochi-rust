//! Office 预览。Word/PowerPoint 转 PDF；表格由 calamine 原生读取。原文件仅只读打开。
use anyhow::{bail, Context, Result};
use calamine::Reader;
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub rows: std::sync::Arc<Vec<Vec<String>>>,
    pub row_count: usize,
    pub column_count: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Workbook {
    pub names: Vec<String>,
    pub sheet: Sheet,
}
pub fn load_sheet(path: &Path, selected: Option<&str>) -> Result<Workbook> {
    if std::fs::metadata(path)?.len() > 128 * 1024 * 1024 {
        bail!("表格超过 128 MB 预览限制")
    }
    let mut book = calamine::open_workbook_auto(path)?;
    let names = book.sheet_names();
    let name = selected
        .map(str::to_owned)
        .or_else(|| names.first().cloned())
        .context("工作簿没有工作表")?;
    let range = book.worksheet_range(&name)?;
    let (height, width) = range.get_size();
    let rows = range
        .rows()
        .filter(|row| row.iter().any(|v| !v.to_string().is_empty()))
        .take(5000)
        .map(|row| row.iter().take(120).map(|v| v.to_string()).collect())
        .collect();
    Ok(Workbook {
        names,
        sheet: Sheet {
            name,
            rows: std::sync::Arc::new(rows),
            row_count: height,
            column_count: width,
        },
    })
}
fn run(mut command: Command, timeout: Duration) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!("转换程序退出：{status}")
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Office 转换超时")
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}
pub fn convert_pdf(path: &Path, force: bool) -> Result<PathBuf> {
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !matches!(extension.as_str(), "doc" | "docx" | "ppt" | "pptx") {
        bail!("不是支持的 Office 文档")
    }
    let meta = std::fs::metadata(path)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().to_lowercase().hash(&mut hasher);
    meta.len().hash(&mut hasher);
    meta.modified()?.hash(&mut hasher);
    if force {
        crate::jstime::now_millis().hash(&mut hasher);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("mochi-native")
        .join("office-preview-cache");
    let output_dir = base.join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&output_dir)?;
    let output = output_dir.join(format!(
        "{}.pdf",
        path.file_stem().unwrap_or_default().to_string_lossy()
    ));
    if output.is_file() {
        return Ok(output);
    }
    let mut candidates = Vec::new();
    if let Some(p) = std::env::var_os("LIBREOFFICE_PATH") {
        candidates.push(PathBuf::from(p));
    }
    for env in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Some(p) = std::env::var_os(env) {
            candidates.push(PathBuf::from(p).join("LibreOffice/program/soffice.exe"));
        }
    }
    candidates.push(PathBuf::from("soffice"));
    candidates.push(PathBuf::from("libreoffice"));
    for converter in candidates {
        if converter.is_absolute() && !converter.is_file() {
            continue;
        }
        let mut version = Command::new(&converter);
        version.arg("--version");
        if run(version, Duration::from_secs(5)).is_err() {
            continue;
        }
        let profile = url::Url::from_directory_path(output_dir.join("profile"))
            .map_err(|_| anyhow::anyhow!("预览缓存路径无效"))?;
        let mut cmd = Command::new(converter);
        cmd.arg(format!("-env:UserInstallation={profile}"))
            .args([
                "--headless",
                "--nologo",
                "--nodefault",
                "--nofirststartwizard",
                "--nolockcheck",
                "--convert-to",
                "pdf",
                "--outdir",
            ])
            .arg(&output_dir)
            .arg(path);
        if run(cmd, Duration::from_secs(90)).is_ok() && output.is_file() {
            return Ok(output);
        }
    }
    let script = output_dir.join("office-export.ps1");
    std::fs::write(&script, include_str!("../assets/office-export.ps1"))?;
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(script)
    .arg(path)
    .arg(&output);
    if run(cmd, Duration::from_secs(90)).is_ok() && output.is_file() {
        return Ok(output);
    }
    bail!("无法生成 Office 预览。请安装 LibreOffice/soffice，或 Microsoft Word/PowerPoint。")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_office_extensions_are_rejected_before_launching() {
        assert!(convert_pdf(Path::new("a.exe"), false).is_err());
    }
    #[test]
    fn missing_workbook_is_an_explicit_error() {
        assert!(load_sheet(Path::new("missing-x.xlsx"), None).is_err());
    }
}
