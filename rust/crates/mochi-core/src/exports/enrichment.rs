//! 可选的「仅导出」扩展。绝不重新序列化或写入源笔记。
use crate::{
    ai::session::{AiConversation, AiStoredMessage},
    sidecars::DocumentComment,
};
use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConversationMode {
    #[default]
    None,
    Answer,
    QuestionAnswer,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub format: String,
    pub conversation: ConversationMode,
    pub comments: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: "pdf".into(),
            conversation: ConversationMode::None,
            comments: false,
        }
    }
}
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

/// locator 属于不受信任的笔记内容，绝不是文件系统路径。
pub use crate::ai::locator::{load_session, safe_session_id};
fn visible(m: &AiStoredMessage) -> bool {
    crate::ai::locator::visible(m)
}
fn locator(line: &str) -> Option<(String, String)> {
    if !line.starts_with("mochi://ai-locate?") {
        return None;
    }
    let parsed =
        crate::ai::locator::Locator::parse(line.trim_end_matches(['\r', '\n', ' ', '\t']))?;
    Some((parsed.session_id, parsed.message_id))
}
fn plain(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if "\\`*_{}[]()#!<>|".contains(c) {
            out.push('\\');
        }
        if c == '\n' || c == '\r' {
            out.push(' ')
        } else {
            out.push(c)
        }
    }
    out
}
pub fn prepare(
    source: &str,
    options: &Options,
    comments: &[DocumentComment],
    mut resolve: impl FnMut(&str) -> Option<AiConversation>,
) -> Result<String> {
    if !["pdf", "html", "markdown"].contains(&options.format.as_str()) {
        bail!("不支持的导出格式")
    }
    if source.len() > MAX_OUTPUT {
        bail!("导出正文超过大小限制")
    }
    // Markdown 解析器会保护围栏/缩进代码、原生 HTML 和 YAML 元数据。
    let mut protected = Vec::new();
    let mut start = None;
    let parser = pulldown_cmark::Parser::new_ext(
        source,
        pulldown_cmark::Options::ENABLE_YAML_STYLE_METADATA_BLOCKS,
    );
    for (event, range) in parser.into_offset_iter() {
        use pulldown_cmark::{Event, Tag, TagEnd};
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::MetadataBlock(_)) => {
                start = Some(range.start)
            }
            Event::End(TagEnd::CodeBlock | TagEnd::HtmlBlock | TagEnd::MetadataBlock(_)) => {
                if let Some(a) = start.take() {
                    protected.push(a..range.end)
                }
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(source.len());
    let mut offset = 0;
    let mut protected_index = 0;
    let mut sessions = HashMap::new();
    for raw in source.split_inclusive('\n') {
        while protected
            .get(protected_index)
            .is_some_and(|r| r.end <= offset)
        {
            protected_index += 1;
        }
        let reference = if protected
            .get(protected_index)
            .is_some_and(|r| r.contains(&offset))
        {
            None
        } else {
            locator(raw)
        };
        if let Some((sid, mid)) = reference {
            if options.conversation != ConversationMode::None && safe_session_id(&sid) {
                // 一条笔记可以引用很多会话，背后藏着庞大的历史。
                // 保留的输入同样要设上限，不只是渲染产物的大小。
                if !sessions.contains_key(&sid) && sessions.len() >= 2 {
                    sessions.clear();
                }
                let conversation = sessions.entry(sid.clone()).or_insert_with(|| resolve(&sid));
                if let Some(c) = conversation.as_ref().filter(|c| c.id == sid) {
                    let messages = c.messages.iter().filter(|m| visible(m)).collect::<Vec<_>>();
                    if let Some(at) = messages
                        .iter()
                        .position(|m| m.role() == "assistant" && m.id() == Some(mid.as_str()))
                    {
                        let answer = messages[at].content();
                        let question = messages[..at]
                            .iter()
                            .rev()
                            .find(|m| m.role() == "user")
                            .map(|m| m.content())
                            .unwrap_or("");
                        if !answer.trim().is_empty()
                            || (options.conversation == ConversationMode::QuestionAnswer
                                && !question.trim().is_empty())
                        {
                            out.push_str("\n\n### AI 对话\n\n");
                            if options.conversation == ConversationMode::QuestionAnswer
                                && !question.trim().is_empty()
                            {
                                out.push_str("#### 问题\n\n");
                                out.push_str(question);
                                out.push_str("\n\n");
                            }
                            if !answer.trim().is_empty() {
                                out.push_str("#### 答案\n\n");
                                out.push_str(answer);
                                out.push_str("\n\n");
                            }
                        }
                    }
                }
            }
            // 与 Electron 的 enrichAILocators 一致：缺席/停用的引用直接省略。
            // 保留源文件周围的原字节，不伪造卡片。
            if raw.ends_with("\r\n") {
                out.push_str("\r\n")
            } else if raw.ends_with('\n') {
                out.push('\n')
            }
        } else {
            out.push_str(raw)
        }
        offset += raw.len();
        if out.len() > MAX_OUTPUT {
            bail!("导出附加内容超过大小限制")
        }
    }
    if options.comments && options.format != "markdown" && !comments.is_empty() {
        out.push_str("\n\n## 评论\n\n");
        let mut children = HashMap::<&str, Vec<usize>>::new();
        for (i, c) in comments.iter().enumerate() {
            if let Some(parent) = c.parent_id.as_deref().filter(|s| !s.is_empty()) {
                children.entry(parent).or_default().push(i);
            }
        }
        let mut stack = comments
            .iter()
            .enumerate()
            .filter(|(_, c)| c.parent_id.as_deref().is_none_or(str::is_empty))
            .map(|(i, _)| (i, 0usize))
            .collect::<Vec<_>>();
        stack.reverse();
        let mut seen = HashSet::new();
        while let Some((i, depth)) = stack.pop() {
            if !seen.insert(i) {
                continue;
            }
            let c = &comments[i];
            let timestamp = chrono::DateTime::parse_from_rfc3339(&c.created_at)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y/%m/%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|_| c.created_at.clone());
            out.push_str(&format!(
                "### {}{}\n\n{}\n\n",
                if depth > 0 {
                    format!("回复（第 {depth} 层）· ")
                } else {
                    String::new()
                },
                plain(&c.author),
                plain(&timestamp)
            ));
            if let Some(anchor) = &c.anchor {
                let target = if !anchor.selected_text.is_empty() {
                    &anchor.selected_text
                } else {
                    &anchor.block_text
                };
                if !target.is_empty() {
                    out.push_str(&format!("> 评论对象：{}\n\n", plain(target)));
                }
            }
            out.push_str(&c.content);
            out.push_str("\n\n");
            if !c.attachments.is_empty() {
                out.push_str(&format!(
                    "附件：{}\n\n",
                    c.attachments
                        .iter()
                        .map(|a| plain(&a.name))
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            }
            if let Some(indices) = children.get(c.id.as_str()) {
                for &j in indices.iter().rev() {
                    stack.push((j, depth + 1));
                }
            }
            if out.len() > MAX_OUTPUT {
                bail!("导出评论超过大小限制")
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(role: &str, id: &str, content: &str) -> AiStoredMessage {
        let mut m = AiStoredMessage::new(role, content);
        m.set("id", serde_json::json!(id));
        m
    }
    fn conversation() -> AiConversation {
        let mut hidden = message("user", "hidden", "SECRET");
        hidden.set("hidden", serde_json::json!(true));
        let mut tool = message("assistant", "tool", "INTERNAL");
        tool.set("tool_calls", serde_json::json!([{"id":"call"}]));
        let mut malformed = message("user", "bad-role", "SECRET malformed");
        malformed.set("role", serde_json::Value::Null);
        let mut hidden_text = message("user", "bad-hidden", "SECRET hidden flag");
        hidden_text.set("hidden", serde_json::json!("true"));
        AiConversation {
            id: "session".into(),
            messages: vec![
                message("user", "q1", "旧问题"),
                message("assistant", "a1", "旧答案"),
                message("user", "q2", "本轮问题"),
                hidden,
                tool,
                malformed,
                hidden_text,
                message("assistant", "a2", "本轮答案"),
            ],
            ..Default::default()
        }
    }
    const URL: &str = "mochi://ai-locate?session=session&message=a2";
    #[test]
    fn defaults_do_not_read_sessions_and_protected_source_is_unchanged() {
        let protected =
            format!("---\nvalue: one\n{URL}\n---\n\n```text\n{URL}\n```\n\n    {URL}\n");
        assert_eq!(
            prepare(&protected, &Options::default(), &[], |_| panic!(
                "unexpected read"
            ))
            .unwrap(),
            protected
        );
        let source = format!("# 保留\r\n\r\n{URL}\r\n末尾  ");
        let out = prepare(&source, &Options::default(), &[], |_| {
            panic!("unexpected read")
        })
        .unwrap();
        assert_eq!(out, "# 保留\r\n\r\n\r\n末尾  ");
    }
    #[test]
    fn only_the_referenced_answer_and_its_visible_question_are_exported() {
        let mut options = Options {
            conversation: ConversationMode::Answer,
            ..Default::default()
        };
        let answer = prepare(URL, &options, &[], |_| Some(conversation())).unwrap();
        assert!(answer.contains("本轮答案"));
        assert!(
            !answer.contains("问题") && !answer.contains("SECRET") && !answer.contains("INTERNAL")
        );
        options.conversation = ConversationMode::QuestionAnswer;
        let both = prepare(URL, &options, &[], |_| Some(conversation())).unwrap();
        assert!(both.contains("本轮问题") && both.contains("本轮答案"));
        assert!(!both.contains("旧问题") && !both.contains("SECRET"));
        for id in ["hidden", "tool", "q2", "missing"] {
            assert!(prepare(
                &format!("mochi://ai-locate?session=session&message={id}"),
                &options,
                &[],
                |_| Some(conversation())
            )
            .unwrap()
            .is_empty());
        }
        let bad = "mochi://ai-locate?session=..%2Foutside&message=a2";
        assert!(prepare(bad, &options, &[], |_| panic!("unsafe id read"))
            .unwrap()
            .is_empty());
        assert!(prepare(URL, &options, &[], |_| Some(AiConversation {
            id: "wrong".into(),
            ..conversation()
        }))
        .unwrap()
        .is_empty());
    }
    fn comment(id: &str, parent: Option<&str>, body: &str) -> DocumentComment {
        DocumentComment {
            resolved: false,
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            target_type: "document".into(),
            author: "测试作者".into(),
            content: body.into(),
            created_at: "2026-09-05T00:00:00Z".into(),
            updated_at: None,
            anchor: None,
            attachments: vec![],
        }
    }
    #[test]
    fn comments_include_replies_and_names_not_attachment_file_contents_or_paths() {
        let mut root = comment("root", None, "评论正文");
        root.attachments.push(crate::sidecars::CommentAttachment {
            id: "file".into(),
            name: "附件.pdf".into(),
            path: "D:/private/secret.pdf".into(),
            size: None,
        });
        let comments = vec![
            root,
            comment("child", Some("root"), "一级回复"),
            comment("grandchild", Some("child"), "二级回复"),
            comment("orphan", Some("absent"), "孤立评论"),
        ];
        let mut options = Options {
            comments: true,
            ..Default::default()
        };
        let out = prepare("正文", &options, &comments, |_| None).unwrap();
        for expected in ["评论正文", "一级回复", "二级回复", "附件.pdf", "第 2 层"]
        {
            assert!(out.contains(expected), "{out}");
        }
        assert!(!out.contains("private") && !out.contains("孤立评论"));
        options.format = "markdown".into();
        assert_eq!(
            prepare("正文", &options, &comments, |_| None).unwrap(),
            "正文"
        );
        let cycle = vec![
            comment("a", None, "ROOT"),
            comment("b", Some("a"), "CHILD"),
            comment("a", Some("b"), "DUPLICATE"),
        ];
        options.format = "html".into();
        let out = prepare("", &options, &cycle, |_| None).unwrap();
        assert_eq!(out.matches("CHILD").count(), 1);
    }
    #[test]
    fn session_lookup_is_read_only_and_rejects_path_ids_and_identity_mismatch() {
        let root = std::env::temp_dir().join(format!(
            "mochi-export-lookup-{}-{}",
            std::process::id(),
            crate::jstime::now_millis()
        ));
        std::fs::create_dir_all(root.join(".mochi/ai-sessions/sessions")).unwrap();
        let path = root.join(".mochi/ai-sessions/sessions/session.json");
        let bytes = crate::json2::serialize(&conversation()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load_session(&root, "session").unwrap().id, "session");
        for id in [
            "../session",
            "..\\session",
            "D:session",
            "session.",
            "session ",
            "missing",
        ] {
            assert!(load_session(&root, id).is_none());
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        assert!(!root.join(".mochi/ai-sessions/index.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
