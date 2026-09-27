//! AI 共享图片资源。只有 assets/... 形式的引用才会按本地文件解析。
use anyhow::{bail, ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 60 * 1024 * 1024;
pub const MAX_IMAGES: usize = 20;
fn component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !value
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
}
fn parts(reference: &str) -> Result<Vec<&str>> {
    let parts = reference.split('/').collect::<Vec<_>>();
    ensure!(
        parts.len() >= 2 && parts[0] == "assets" && parts.iter().all(|p| component(p)),
        "图片引用不是有效的 assets/ 路径"
    );
    Ok(parts)
}
pub(super) fn open_read(path: &Path, directory: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(if directory { 3 } else { 1 })
            .custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 });
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    ensure!(
        !meta.file_type().is_symlink() && meta.is_dir() == directory,
        "附件路径不能重定向或改变类型"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            meta.file_attributes() & 0x400 == 0,
            "附件路径不能是重解析点"
        );
    }
    Ok(file)
}
/// 操作期间握住目录句柄，Windows 上就不允许改名/删除。
fn directories(root: &Path, extra: &[&str], create: bool) -> Result<(PathBuf, Vec<File>)> {
    let mut path = root.canonicalize()?;
    let mut guards = vec![open_read(&path, true)?];
    for part in [".mochi", "ai-sessions"]
        .into_iter()
        .chain(extra.iter().copied())
    {
        ensure!(component(part), "非法附件目录");
        path.push(part);
        if create && !path.exists() {
            fs::create_dir(&path)?;
        }
        guards.push(open_read(&path, true)?);
    }
    Ok((path, guards))
}
fn format(bytes: &[u8]) -> Result<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(("png", "image/png"));
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Ok(("jpg", "image/jpeg"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok(("gif", "image/gif"));
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Ok(("webp", "image/webp"));
    }
    if bytes.starts_with(b"BM") {
        return Ok(("bmp", "image/bmp"));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let text = text.trim_start_matches('\u{feff}').trim_start();
        if text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg")) {
            return Ok(("svg", "image/svg+xml"));
        }
    }
    bail!("不支持的图片格式，或文件不是图片")
}
fn bytes(file: File) -> Result<Vec<u8>> {
    ensure!(
        file.metadata()?.len() <= MAX_IMAGE_BYTES as u64,
        "图片超过 20 MiB 限制"
    );
    let mut bytes = Vec::new();
    file.take((MAX_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_IMAGE_BYTES, "图片超过 20 MiB 限制");
    format(&bytes)?;
    Ok(bytes)
}
pub fn import_image(root: &Path, source: &Path) -> Result<String> {
    let data = bytes(open_read(source, false)?)?;
    save_image(root, &data)
}
pub fn save_image(root: &Path, bytes: &[u8]) -> Result<String> {
    ensure!(bytes.len() <= MAX_IMAGE_BYTES, "图片超过 20 MiB 限制");
    let (extension, _) = format(bytes)?;
    let (directory, _guards) = directories(root, &["assets"], true)?;
    let name = format!(
        "{}-{}.{}",
        crate::jstime::now_millis(),
        crate::paths::random_base36(9),
        extension
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let mut output = options.open(directory.join(&name))?;
    output.write_all(bytes)?;
    output.sync_all()?;
    Ok(format!("assets/{name}"))
}
pub fn local_path(root: &Path, reference: &str) -> Result<PathBuf> {
    let parts = parts(reference)?;
    let (directory, _guards) = directories(root, &parts[..parts.len() - 1], false)?;
    let path = directory.join(parts.last().unwrap());
    let _file = open_read(&path, false)?;
    Ok(path)
}
pub fn data_url(root: &Path, reference: &str) -> Result<String> {
    let parts = parts(reference)?;
    let (directory, _guards) = directories(root, &parts[..parts.len() - 1], false)?;
    let bytes = bytes(open_read(&directory.join(parts.last().unwrap()), false)?)?;
    let (_, mime) = format(&bytes)?;
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}
pub fn prepare_images(root: &Path, references: &[String]) -> Result<Vec<String>> {
    ensure!(references.len() <= MAX_IMAGES, "一轮最多附加 20 张图片");
    let mut total = 0;
    let mut result = Vec::new();
    for reference in references {
        let value = data_url(root, reference)?;
        total += value.len();
        ensure!(total <= MAX_REQUEST_BYTES, "本轮图片编码超过 60 MiB 限制");
        result.push(value);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn directory_guards_prevent_path_replacement_during_image_read() {
        let root = std::env::temp_dir().join(format!(
            "mochi-assets-lock-{}",
            crate::paths::random_base36(12)
        ));
        fs::create_dir_all(&root).unwrap();
        save_image(&root, b"\x89PNG\r\n\x1a\nsynthetic").unwrap();
        let (path, guards) = directories(&root, &["assets"], false).unwrap();
        let moved = path.with_file_name("assets-moved");
        assert!(fs::rename(&path, &moved).is_err());
        drop(guards);
        fs::rename(&path, &moved).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn image_bytes_round_trip_without_bloating_session_references() {
        let root =
            std::env::temp_dir().join(format!("mochi-assets-{}", crate::paths::random_base36(12)));
        fs::create_dir_all(&root).unwrap();
        let bytes = b"\x89PNG\r\n\x1a\nsynthetic";
        let rel = save_image(&root, bytes).unwrap();
        assert!(rel.starts_with("assets/"));
        assert!(rel.len() < 80);
        assert_eq!(fs::read(local_path(&root, &rel).unwrap()).unwrap(), bytes);
        assert_eq!(
            data_url(&root, &rel).unwrap(),
            format!("data:image/png;base64,{}", STANDARD.encode(bytes))
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_traversal_absolute_ads_and_non_images() {
        for path in [
            "../secret.png",
            "assets/../secret.png",
            "C:/secret.png",
            "assets/a:secret.png",
            "assets/a\\b.png",
            "assets/./x.png",
            "assets/x.png ",
        ] {
            assert!(parts(path).is_err(), "{path}");
        }
        assert!(format(b"not a picture").is_err());
        assert!(save_image(Path::new("missing"), &vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    }
}
