//! 文件改名时一起移动伴生数据；预检冲突，失败时回滚已完成的移动。
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
pub fn with_companions(from: &Path, to: &Path) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let mut pairs = vec![(from.to_path_buf(), to.to_path_buf())];
    let mut comments = None;
    if from.is_file() {
        let old = from.to_string_lossy();
        let new = to.to_string_lossy();
        let old_comment = PathBuf::from(crate::sidecars::comment_sidecar_path(&old));
        let new_comment = PathBuf::from(crate::sidecars::comment_sidecar_path(&new));
        if old_comment.is_file() {
            let original = std::fs::read(&old_comment)?;
            let mut value: serde_json::Value =
                serde_json::from_slice(&original).context("批注文件损坏，未进行重命名")?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("批注文件不是 JSON 对象"))?;
            object.insert("sourcePath".into(), new.to_string().into());
            object.insert("updatedAt".into(), crate::jstime::now().into());
            let old_assets = crate::sidecars::comment_assets_dir(&old).replace('\\', "/");
            let new_assets = crate::sidecars::comment_assets_dir(&new).replace('\\', "/");
            if let Some(items) = object
                .get_mut("comments")
                .and_then(serde_json::Value::as_array_mut)
            {
                for item in items {
                    if let Some(attachments) = item
                        .get_mut("attachments")
                        .and_then(serde_json::Value::as_array_mut)
                    {
                        for attachment in attachments {
                            if let Some(path) =
                                attachment.get("path").and_then(serde_json::Value::as_str)
                            {
                                let normalized = path.replace('\\', "/");
                                if let Some(rest) =
                                    normalized.strip_prefix(&(old_assets.clone() + "/"))
                                {
                                    attachment["path"] = format!("{new_assets}/{rest}").into();
                                }
                            }
                        }
                    }
                }
            }
            comments = Some((
                old_comment.clone(),
                new_comment.clone(),
                original,
                format!("{}\n", crate::json2::serialize(&value)?),
            ));
            pairs.push((old_comment, new_comment));
        }
        for (old, new) in [
            (
                format!("{old}.blocks-backup"),
                format!("{new}.blocks-backup"),
            ),
            (
                format!("{old}.blocks-restore-backup"),
                format!("{new}.blocks-restore-backup"),
            ),
            (
                crate::sidecars::comment_assets_dir(&old),
                crate::sidecars::comment_assets_dir(&new),
            ),
            (
                crate::sidecars::annotation_sidecar_path(&old),
                crate::sidecars::annotation_sidecar_path(&new),
            ),
            (
                crate::sub_documents::sidecar_path(&old),
                crate::sub_documents::sidecar_path(&new),
            ),
        ] {
            let old = PathBuf::from(old);
            let new = PathBuf::from(new);
            if old.exists() && old != new {
                pairs.push((old, new));
            }
        }
    }
    for (_, target) in &pairs {
        if target.exists() {
            bail!("目标已存在，未覆盖：{}", target.display())
        }
    }
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| -> Result<()> {
        for (source, target) in &pairs {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(source, target)
                .with_context(|| format!("无法移动 {}", source.display()))?;
            moved.push((source.clone(), target.clone()));
        }
        if let Some((_, target, _, updated)) = &comments {
            crate::files::FileService::new().write_file_safe(target, updated)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for (source, target) in moved.iter().rev() {
            if let Err(e) = std::fs::rename(target, source) {
                failures.push(format!("{}：{e}", target.display()));
            }
        }
        if let Some((source, _, original, _)) = &comments {
            if source.exists() {
                if let Err(e) = crate::files::FileService::new().write_bytes_safe(source, original)
                {
                    failures.push(e.to_string());
                }
            }
        }
        if failures.is_empty() {
            return Err(error);
        }
        bail!("{error}；部分回滚失败，请检查：{}", failures.join("；"))
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_block_backup_follows_document_rename_and_format_upgrade() {
        let root = std::env::temp_dir().join(format!(
            "mochi-block-rename-{}",
            crate::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let from = root.join("old.md");
        let to = root.join("new.mc");
        std::fs::write(&from, "旧版转换后的正文\n").unwrap();
        std::fs::write(
            crate::document_blocks::conversion_backup_path(&from),
            "恢复原文\n",
        )
        .unwrap();
        with_companions(&from, &to).unwrap();
        assert!(!crate::document_blocks::conversion_backup_path(&from).exists());
        assert!(crate::document_blocks::conversion_backup_path(&to).exists());
        assert_eq!(
            crate::document_blocks::restore_file(&to, "旧版转换后的正文\n")
                .unwrap()
                .source,
            "恢复原文\n"
        );
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mochi-block-rename-"));
        std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn locked_companion_rolls_back_the_main_file_move() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = std::env::temp_dir().join(format!(
            "mochi-rename-locked-{}",
            crate::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let from = root.join("old.pdf");
        let to = root.join("new.pdf");
        std::fs::write(&from, b"original").unwrap();
        let side = PathBuf::from(crate::sidecars::annotation_sidecar_path(
            &from.to_string_lossy(),
        ));
        std::fs::create_dir_all(side.parent().unwrap()).unwrap();
        std::fs::write(&side, b"annotations").unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&side)
            .unwrap();
        assert!(with_companions(&from, &to).is_err());
        assert_eq!(std::fs::read(&from).unwrap(), b"original");
        assert!(!to.exists());
        drop(lock);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn moves_comments_assets_and_preserves_markdown_bytes() {
        let root =
            std::env::temp_dir().join(format!("mochi-rename-{}", crate::paths::random_base36(12)));
        std::fs::create_dir_all(&root).unwrap();
        let from = root.join("旧.md");
        let to = root.join("新.md");
        let body = b"# note\r\n\r\n**body**";
        std::fs::write(&from, body).unwrap();
        let assets = crate::sidecars::comment_assets_dir(&from.to_string_lossy());
        std::fs::create_dir_all(&assets).unwrap();
        let attachment = Path::new(&assets).join("file.txt");
        std::fs::write(&attachment, b"attachment").unwrap();
        let comment = crate::sidecars::comment_sidecar_path(&from.to_string_lossy());
        std::fs::write(&comment,serde_json::json!({"sourcePath":from,"custom":7,"comments":[{"attachments":[{"path":attachment}]}]}).to_string()).unwrap();
        with_companions(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), body);
        assert!(!from.exists());
        let moved =
            std::fs::read_to_string(crate::sidecars::comment_sidecar_path(&to.to_string_lossy()))
                .unwrap();
        let value: serde_json::Value = serde_json::from_str(&moved).unwrap();
        assert_eq!(value["custom"], 7);
        assert!(Path::new(
            value["comments"][0]["attachments"][0]["path"]
                .as_str()
                .unwrap()
        )
        .is_file());
        assert!(moved.ends_with('\n'));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn companion_collision_is_checked_before_moving_source() {
        let root = std::env::temp_dir().join(format!(
            "mochi-rename-conflict-{}",
            crate::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let from = root.join("old.pdf");
        let to = root.join("new.pdf");
        std::fs::write(&from, b"pdf").unwrap();
        let old = PathBuf::from(crate::sidecars::annotation_sidecar_path(
            &from.to_string_lossy(),
        ));
        let new = PathBuf::from(crate::sidecars::annotation_sidecar_path(
            &to.to_string_lossy(),
        ));
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(old, b"old").unwrap();
        std::fs::write(&new, b"existing").unwrap();
        assert!(with_companions(&from, &to).is_err());
        assert!(from.exists());
        assert!(!to.exists());
        assert_eq!(std::fs::read(new).unwrap(), b"existing");
        std::fs::remove_dir_all(root).unwrap();
    }
}
