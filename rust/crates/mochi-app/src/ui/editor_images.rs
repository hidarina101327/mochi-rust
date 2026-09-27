//! 笔记图片与 Electron 版使用相同的 ./assets/ 路径约定。
use anyhow::{ensure, Context};
use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub fn store(note: &Path, bytes: &[u8]) -> anyhow::Result<String> {
    ensure!(
        super::imginfo::dimensions_from_bytes(bytes).is_some(),
        "图片无法读取，请选择 PNG、JPEG、GIF、BMP 或 WebP 图片"
    );
    let extension = if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "jpg"
    } else if bytes.starts_with(b"GIF") {
        "gif"
    } else if bytes.starts_with(b"BM") {
        "bmp"
    } else {
        "webp"
    };
    let directory = note.parent().context("文档目录不可用")?.join("assets");
    std::fs::create_dir_all(&directory)?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    let name = format!(
        "{time}-{}-{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        extension
    );
    let path = directory.join(&name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(error.into());
    }
    Ok(format!("./assets/{name}"))
}

pub fn html(source: &str) -> Option<(String, String, Option<String>)> {
    let source = source.trim();
    if !source.starts_with("<img ") || !source.ends_with('>') {
        return None;
    }
    let src = super::containers::attribute(source, "src")?;
    let alt = super::containers::attribute(source, "alt").unwrap_or_default();
    let width = super::containers::attribute(source, "width")
        .or_else(|| {
            super::containers::attribute(source, "style")?
                .split(';')
                .find_map(|p| p.trim().strip_prefix("width:").map(|w| w.trim().to_owned()))
        })
        .filter(|w| parse_width(w, 100.0).is_some());
    Some((alt, src, width))
}

pub fn parse_width(value: &str, available: f32) -> Option<f32> {
    let value = value.trim();
    let (number, scale) = if let Some(percent) = value.strip_suffix('%') {
        (percent, available / 100.0)
    } else {
        (value.strip_suffix("px").unwrap_or(value), 1.0)
    };
    number
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite() && *n > 0.0)
        .map(|n| (n * scale).min(available).max(1.0))
}

pub fn resize(buffer: &mut super::editor::TextBuffer, start: usize, width: &str) -> bool {
    if parse_width(width, 1000.0).is_none() {
        return false;
    }
    let parsed = super::document::parse_ranged(buffer.text());
    let Some(block) = parsed.blocks.iter().find(|b| b.start == start) else {
        return false;
    };
    let super::document::Block::Image { alt, src, .. } = &block.block else {
        return false;
    };
    let raw = &buffer.text()[block.start..block.end];
    let image = if raw.trim_start().starts_with("<img ") {
        static WIDTH: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let pattern = WIDTH.get_or_init(|| {
            regex::Regex::new(r#"\s+width\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)"#).unwrap()
        });
        let stripped = pattern.replace_all(raw, "");
        let at = stripped.rfind('>').unwrap();
        let at = if stripped[..at].ends_with('/') {
            at - 1
        } else {
            at
        };
        format!(
            "{} width=\"{}\"{}",
            &stripped[..at],
            super::containers::escape(width),
            &stripped[at..]
        )
    } else {
        format!(
            "<img src=\"{}\" alt=\"{}\" width=\"{}\">",
            super::containers::escape(src),
            super::containers::escape(alt),
            super::containers::escape(width)
        )
    };
    buffer.replace_range_select(block.start..block.end, &image, 0..0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_image_writes_never_overwrite_each_other_or_the_note() {
        let root = std::env::temp_dir().join(format!(
            "mochi-editor-image-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(10)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let note = root.join("墨池-食用说明.mc");
        std::fs::write(&note, "原文必须保留").unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1u32.to_be_bytes());
        png.extend_from_slice(&1u32.to_be_bytes());
        let first = store(&note, &png).unwrap();
        let second = store(&note, &png).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&note).unwrap(), "原文必须保留");
        assert_eq!(
            std::fs::read(root.join(first.trim_start_matches("./"))).unwrap(),
            png
        );
        assert_eq!(
            std::fs::read(root.join(second.trim_start_matches("./"))).unwrap(),
            png
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn electron_image_width_and_escaping_roundtrip_without_rewriting_other_content() {
        let source = "前文\r\n![示意图](./assets/test.png)\r\n后文";
        let mut buffer = super::super::editor::TextBuffer::new(source);
        let start = source.find("![").unwrap();
        assert!(resize(&mut buffer, start, "50%"));
        assert_eq!(
            html(buffer.text().lines().nth(1).unwrap()),
            Some((
                "示意图".into(),
                "./assets/test.png".into(),
                Some("50%".into())
            ))
        );
        assert!(buffer.text().starts_with("前文\r\n"));
        assert!(buffer.text().ends_with("\r\n后文"));
        assert_eq!(parse_width("50%", 600.0), Some(300.0));
        assert_eq!(parse_width("320px", 600.0), Some(320.0));
        buffer.undo();
        assert_eq!(buffer.text(), source);
    }
}
