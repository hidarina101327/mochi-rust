//! 用户显式选择的本地文件；源数据绝不改动，也不存进历史。
use super::{assets, AiFile};
use anyhow::{ensure, Context, Result};
use std::path::{Path, PathBuf};
pub const MAX_FILES: usize = 20;
pub struct Prepared {
    pub display: String,
    pub content: String,
    pub files: Vec<AiFile>,
}
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
pub fn is_image(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
    )
}
/// 附件只是本地文档的引用。模型只拿到绝对路径，真正需要内容时
/// 自己调 `file_read`。这样就不用每轮请求都把大文档整个搬一遍。
pub fn prepare(text: &str, paths: &[PathBuf], image_bytes: usize) -> Result<Prepared> {
    ensure!(paths.len() <= MAX_FILES, "一轮最多附加 20 个文件");
    let mut out = Prepared {
        display: text.into(),
        content: text.into(),
        files: Vec::new(),
    };
    let mut total = image_bytes.checked_add(text.len()).context("附件过大")?;
    ensure!(
        total <= assets::MAX_REQUEST_BYTES,
        "本轮附件超过 60 MiB 限制"
    );
    for path in paths {
        ensure!(path.is_absolute(), "附件必须为用户选择的完整路径");
        let name = path.file_name().context("无效附件路径")?.to_string_lossy();
        let source = path.to_string_lossy();
        let metadata =
            std::fs::metadata(path).with_context(|| format!("附件路径不可用：{name}"))?;
        ensure!(metadata.is_file(), "附件必须是文件：{name}");
        let suffix = format!(
            "\n\n---\n文档：{name}\n路径：{source}\n说明：这是本地文档路径；需要正文时请使用 file_read 读取，不要让用户重复上传。"
        );
        total = total.checked_add(suffix.len()).context("附件过大")?;
        ensure!(
            total <= assets::MAX_REQUEST_BYTES,
            "本轮附件超过 60 MiB 限制"
        );
        out.content.push_str(&suffix);
        out.display
            .push_str(&format!("\n\n📎 **{name}**\n_{source}_"));
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_match_electron_and_bounds_are_explicit() {
        assert!(prepare("", &[PathBuf::from("relative.txt")], 0).is_err());
        assert!(prepare("", &[], assets::MAX_REQUEST_BYTES + 1).is_err());
    }
    #[test]
    fn text_and_binary_are_ephemeral_and_never_rewrite_sources() {
        let root =
            std::env::temp_dir().join(format!("mochi-files-{}", crate::paths::random_base36(12)));
        std::fs::create_dir_all(&root).unwrap();
        let text = root.join("笔记.MD");
        let binary = root.join("sample.pdf");
        let original = b"# note\r\n\r\nexact";
        let raw = b"opaque binary fixture\0\xff";
        std::fs::write(&text, original).unwrap();
        std::fs::write(&binary, raw).unwrap();
        let prepared = prepare("question", &[text.clone(), binary.clone()], 0).unwrap();
        assert!(prepared.display.contains("📎 **笔记.MD**"));
        assert!(!prepared.display.contains("exact"));
        assert!(!prepared.content.contains("# note\r\n\r\nexact"));
        assert!(prepared.content.contains(text.to_string_lossy().as_ref()));
        assert!(prepared.content.contains(binary.to_string_lossy().as_ref()));
        assert!(prepared.files.is_empty());
        assert_eq!(std::fs::read(&text).unwrap(), original);
        assert_eq!(std::fs::read(&binary).unwrap(), raw);
        assert!(prepare("", std::slice::from_ref(&root), 0).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
