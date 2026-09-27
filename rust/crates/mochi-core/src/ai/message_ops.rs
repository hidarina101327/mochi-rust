//! 消息操作使用已保存的 ID 和对话中的原始顺序，不依赖视图索引。
use super::{locator, session::AiConversation};
use anyhow::{ensure, Context, Result};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub fn actionable(m: &super::session::AiStoredMessage) -> bool {
    locator::visible(m)
        && m.role() == "assistant"
        && m.id().is_some_and(|s| !s.is_empty())
        && [
            "pendingEdit",
            "pendingScheduleDiff",
            "pendingPluginPermission",
            "pendingPluginChange",
            "pendingFileOperation",
            "pendingShellCommand",
        ]
        .iter()
        .all(|key| {
            m.get(key)
                .is_none_or(|v| v.is_null() || v == &serde_json::Value::Bool(false))
        })
}
pub fn round_range(c: &AiConversation, id: &str) -> Option<Range<usize>> {
    let target = c
        .messages
        .iter()
        .position(|m| m.id() == Some(id) && actionable(m))?;
    let start = (0..=target)
        .rev()
        .find(|&i| c.messages[i].get("role").and_then(|v| v.as_str()) == Some("user"))
        .unwrap_or(target);
    let end = (target + 1..c.messages.len())
        .find(|&i| c.messages[i].get("role").and_then(|v| v.as_str()) == Some("user"))
        .unwrap_or(c.messages.len());
    Some(start..end)
}
pub fn copy_markdown(source: &str) -> String {
    static BLOCK: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)\\\[\s*(.*?)\s*\\\]").unwrap());
    static INLINE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)\\\(\s*(.*?)\s*\\\)").unwrap());
    let convert = |s: &str| {
        let block = BLOCK.replace_all(s, |c: &regex::Captures<'_>| {
            format!("\n$$\n{}\n$$\n", c[1].trim())
        });
        INLINE
            .replace_all(&block, |c: &regex::Captures<'_>| {
                format!("${}$", c[1].trim())
            })
            .into_owned()
    };
    let mut protected = Vec::new();
    let mut start = None;
    for (event, range) in pulldown_cmark::Parser::new(source).into_offset_iter() {
        use pulldown_cmark::{Event, Tag, TagEnd};
        match event {
            Event::Start(Tag::CodeBlock(_)) => start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(a) = start.take() {
                    protected.push(a..range.end)
                }
            }
            Event::Code(_) => protected.push(range),
            _ => {}
        }
    }
    let mut result = String::new();
    let mut offset = 0;
    for range in protected {
        if range.start < offset {
            continue;
        }
        result.push_str(&convert(&source[offset..range.start]));
        result.push_str(&source[range.clone()]);
        offset = range.end;
    }
    result.push_str(&convert(&source[offset..]));
    result
}
pub fn snippet(source: &str) -> String {
    static CLEAN: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
        [
            r"📎\s*\*\*[^*]*\*\*",
            r"_[^_\n]*_",
            r"!\[[^\]]*\]\([^)]*\)",
            r"(?i)\[image\s*\d*\]",
            r"[#>*`~_$]",
            r"\s+",
        ]
        .iter()
        .map(|s| regex::Regex::new(s).unwrap())
        .collect()
    });
    let mut value = source.to_owned();
    for re in CLEAN.iter() {
        value = re.replace_all(&value, " ").into_owned();
    }
    let value = value.trim();
    if value.encode_utf16().count() <= 50 {
        return value.to_owned();
    }
    let mut used = 0;
    let mut out = String::new();
    for c in value.chars() {
        if used + c.len_utf16() > 50 {
            break;
        }
        used += c.len_utf16();
        out.push(c);
    }
    out.push('…');
    out
}
struct Change {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
}
pub struct Deletion {
    pub root: PathBuf,
    pub next: AiConversation,
    expected_active: String,
    changes: Vec<Change>,
    absent_mount: Option<PathBuf>,
}
impl Deletion {
    pub fn prepare(root: &Path, active: &AiConversation, id: &str) -> Result<Self> {
        ensure!(locator::safe_session_id(&active.id), "会话标识无效");
        let range = round_range(active, id).context("回复已不存在或暂不可操作")?;
        let removed = active.messages[range.clone()]
            .iter()
            .filter_map(|m| m.id().map(str::to_owned))
            .collect::<Vec<_>>();
        let mut next = active.clone();
        next.messages.drain(range);
        next.updated_at = crate::jstime::now_millis();
        let root = root.canonicalize()?;
        let base = root.join(".mochi/ai-sessions").canonicalize()?;
        ensure!(
            crate::paths::path_is_within(&root, &base),
            "会话目录不在工作区内"
        );
        let read = |relative: &str| -> Result<(PathBuf, Vec<u8>, serde_json::Value)> {
            let path = base.join(relative).canonicalize()?;
            ensure!(
                crate::paths::path_is_within(&base, &path),
                "会话文件越出工作区"
            );
            let before = std::fs::read(&path)?;
            let value = serde_json::from_slice(&before)?;
            Ok((path, before, value))
        };
        let (path, before, mut value) = read(&format!("sessions/{}.json", active.id))?;
        ensure!(
            value["id"].as_str() == Some(&active.id),
            "磁盘会话标识不一致"
        );
        ensure!(
            serde_json::from_value::<AiConversation>(value.clone())? == *active,
            "会话在磁盘上已变化或尚未保存，请刷新后再删除"
        );
        value["messages"] = serde_json::to_value(&next.messages)?;
        value["title"] = serde_json::json!(next.title);
        value["updatedAt"] = serde_json::json!(next.updated_at);
        let mut changes = vec![Change {
            path,
            before,
            after: crate::json2::serialize(&value)?.into_bytes(),
        }];
        let (path, before, mut value) = read("index.json")?;
        let meta = value
            .get_mut("sessions")
            .and_then(|v| v.as_array_mut())
            .and_then(|entries| {
                entries
                    .iter_mut()
                    .find(|m| m["id"].as_str() == Some(&active.id))
            })
            .context("会话索引已变化")?;
        meta["updatedAt"] = serde_json::json!(next.updated_at);
        meta["messageCount"] = serde_json::json!(next.messages.len());
        changes.push(Change {
            path,
            before,
            after: crate::json2::serialize(&value)?.into_bytes(),
        });
        let mount_path = base.join("document-mounts.json");
        let has_mounts = mount_path.try_exists()?;
        if has_mounts {
            let (path, before, value) = read("document-mounts.json")?;
            let mut index = super::document_mounts::AiDocumentMountIndex::from_value(&value);
            index.mounts.retain(|m| {
                m.session_id() != active.id
                    || m.scope() != super::document_mounts::MountScope::Message
                    || (!m
                        .message_id()
                        .is_some_and(|id| removed.iter().any(|s| s == id))
                        && !m
                            .user_message_id()
                            .is_some_and(|id| removed.iter().any(|s| s == id)))
            });
            changes.push(Change {
                path,
                before,
                after: crate::json2::serialize(&index.normalize())?.into_bytes(),
            });
        }
        Ok(Self {
            root,
            next,
            expected_active: crate::json2::serialize(active)?,
            changes,
            absent_mount: (!has_mounts).then_some(mount_path),
        })
    }
    pub fn commit(&self, root: &Path, active: &AiConversation) -> Result<()> {
        ensure!(
            root.canonicalize()? == self.root
                && crate::json2::serialize(active)? == self.expected_active,
            "会话已变化，请重新确认"
        );
        if let Some(path) = &self.absent_mount {
            ensure!(!path.try_exists()?, "挂载索引已变化，请重新确认");
        }
        for change in &self.changes {
            ensure!(
                std::fs::read(&change.path)? == change.before,
                "磁盘数据已变化，请重新确认"
            );
            let backup = PathBuf::from(format!("{}.backup", change.path.display()));
            if backup.try_exists()? {
                ensure!(
                    backup.canonicalize()?.starts_with(&self.root),
                    "备份路径不在工作区内"
                );
            }
        }
        let files = crate::files::FileService::new();
        for (i, change) in self.changes.iter().enumerate() {
            let result = (|| -> Result<()> {
                ensure!(
                    std::fs::read(&change.path)? == change.before,
                    "磁盘数据已变化"
                );
                files.write_bytes_safe(&change.path, &change.after)
            })();
            if let Err(error) = result {
                let mut rollback = Vec::new();
                for prior in self.changes[..i].iter().rev() {
                    if std::fs::read(&prior.path).ok().as_deref() == Some(prior.after.as_slice()) {
                        if let Err(e) = files.write_bytes_safe(&prior.path, &prior.before) {
                            rollback.push(e.to_string());
                        }
                    } else {
                        rollback.push(format!("{} 已被外部修改，未覆盖", prior.path.display()));
                    }
                }
                ensure!(
                    rollback.is_empty(),
                    "删除失败：{error}；恢复需检查：{}",
                    rollback.join("；")
                );
                return Err(error);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::session::{AiSessionIndex, AiSessionMeta, AiSessionService, AiStoredMessage};
    use super::*;
    fn conversation() -> AiConversation {
        let messages = [
            ("system", "sys"),
            ("user", "q"),
            ("assistant", "call"),
            ("tool", "tool"),
            ("assistant", "a"),
            ("user", "q2"),
            ("assistant", "a2"),
        ]
        .into_iter()
        .map(|(role, id)| {
            let mut m = AiStoredMessage::new(role, id);
            m.set("id", serde_json::json!(id));
            m.set("custom", serde_json::json!({"keep":true}));
            if id == "call" {
                m.set("tool_calls", serde_json::json!([{"id":"call-1"}]));
            }
            m
        })
        .collect();
        AiConversation {
            id: "s".into(),
            title: "会话".into(),
            messages,
            ..Default::default()
        }
    }
    fn fixture() -> (PathBuf, AiConversation, AiSessionService) {
        let root =
            std::env::temp_dir().join(format!("mochi-round-{}", crate::paths::random_base36(12)));
        let c = conversation();
        let svc = AiSessionService::new(&root);
        svc.save_session(&c).unwrap();
        svc.save_index(&AiSessionIndex {
            sessions: vec![AiSessionMeta {
                id: "s".into(),
                ..Default::default()
            }],
            ..Default::default()
        })
        .unwrap();
        (root, c, svc)
    }
    #[test]
    fn round_boundaries_keep_other_rounds_and_tool_pairs_together() {
        let c = conversation();
        assert_eq!(round_range(&c, "a"), Some(1..5));
        assert!(round_range(&c, "call").is_none());
        assert!(round_range(&c, "missing").is_none());
        let mut no_user = c.clone();
        no_user.messages.drain(1..4);
        assert_eq!(round_range(&no_user, "a"), Some(1..2));
    }
    #[test]
    fn copy_normalizes_math_but_never_code_or_literal_placeholder_tokens() {
        let source="公式 \\( x^2 \\)\n\\[ a+b \\]\n\n```text\n\\(code\\)\n```\n`\\[inline\\]` @@MOCHI_CODE_0@@";
        let copied = copy_markdown(source);
        assert!(copied.contains("$x^2$"));
        assert!(copied.contains("\n$$\na+b\n$$\n"));
        assert!(copied.contains("\\(code\\)"));
        assert!(copied.contains("`\\[inline\\]` @@MOCHI_CODE_0@@"));
        assert_eq!(copy_markdown(&copied), copied);
        assert_eq!(
            snippet("📎 **文件** _私有路径_ ![x](img.png) [image 1] # 真正的问题"),
            "真正的问题"
        );
    }
    #[test]
    fn delete_updates_files_and_keeps_unknown_fields_and_session_mounts() {
        let (root, c, svc) = fixture();
        let file = svc.base_path().join("sessions/s.json");
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        raw["future"] = serde_json::json!("keep");
        std::fs::write(&file, crate::json2::serialize(&raw).unwrap()).unwrap();
        let doc = root.join("note.md");
        std::fs::write(&doc, "原文").unwrap();
        use super::super::document_mounts::{AiDocumentMountService, CreateMountInput, MountScope};
        let mounts = AiDocumentMountService::new(&root);
        for scope in [MountScope::Message, MountScope::Session] {
            mounts
                .add_mounts(vec![CreateMountInput {
                    session_id: "s".into(),
                    scope,
                    message_id: (scope == MountScope::Message).then(|| "a".into()),
                    user_message_id: (scope == MountScope::Message).then(|| "q".into()),
                    document_path: doc.to_string_lossy().into_owned(),
                    session_title: "会话".into(),
                    snippet: "问答".into(),
                    created_at: None,
                }])
                .unwrap();
        }
        assert_eq!(mounts.load().mounts.len(), 2);
        let plan = Deletion::prepare(&root, &c, "a").unwrap();
        plan.commit(&root, &c).unwrap();
        let result = svc.load_session("s").unwrap();
        assert_eq!(
            result
                .messages
                .iter()
                .filter_map(|m| m.id())
                .collect::<Vec<_>>(),
            vec!["sys", "q2", "a2"]
        );
        assert!(result.messages[1].get("custom").is_some());
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(value["future"], "keep");
        assert_eq!(svc.load_index().sessions[0].message_count, 3);
        assert_eq!(mounts.load().mounts.len(), 1);
        assert_eq!(mounts.load().mounts[0].scope(), MountScope::Session);
        assert_eq!(std::fs::read_to_string(doc).unwrap(), "原文");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn stale_disk_or_memory_confirmation_is_rejected_without_writes() {
        let (root, c, svc) = fixture();
        let file = svc.base_path().join("sessions/s.json");
        let original = std::fs::read(&file).unwrap();
        let plan = Deletion::prepare(&root, &c, "a").unwrap();
        let mut changed = c.clone();
        changed.messages[0].set_content("change");
        assert!(plan.commit(&root, &changed).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), original);
        let index = svc.base_path().join("index.json");
        let mut external = std::fs::read(&index).unwrap();
        external.push(b' ');
        std::fs::write(&index, &external).unwrap();
        assert!(plan.commit(&root, &c).is_err());
        assert_eq!(std::fs::read(file).unwrap(), original);
        assert_eq!(std::fs::read(index).unwrap(), external);
        std::fs::remove_dir_all(root).unwrap();
    }
}
