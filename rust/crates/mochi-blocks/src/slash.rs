//! 纯函数的斜杠命令匹配。菜单、撤销和 AI 请求都归宿主管。
//! 只有当这一行 Markdown 除命令外为空时才识别；路径、行内斜杠、
//! 代码、数学公式和文档头部仍按普通文档文本处理。
use std::ops::Range;

use anyhow::{bail, Result};
use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Ai,
    Rewrite,
    Summarize,
    Todo,
    Heading,
}

impl Command {
    pub const ALL: [Self; 5] = [
        Self::Ai,
        Self::Rewrite,
        Self::Summarize,
        Self::Todo,
        Self::Heading,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Rewrite => "rewrite",
            Self::Summarize => "summarize",
            Self::Todo => "todo",
            Self::Heading => "heading",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ai => "让 AI 处理当前块",
            Self::Rewrite => "改写当前块（审批后应用）",
            Self::Summarize => "总结当前块（审批后应用）",
            Self::Todo => "任务列表",
            Self::Heading => "二级标题",
        }
    }

    pub fn is_ai(self) -> bool {
        matches!(self, Self::Ai | Self::Rewrite | Self::Summarize)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub range: Range<usize>,
    pub query: String,
    /// 提交编辑前再比对一次，过期的弹窗才不会删掉
    /// 刚敲进去的文字、或外部刷新过的文档。
    pub original: String,
}

impl Trigger {
    pub fn commands(&self) -> Vec<Command> {
        Command::ALL
            .into_iter()
            .filter(|command| command.name().starts_with(&self.query))
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub range: Range<usize>,
    pub replacement: String,
    pub cursor_after: usize,
    /// 宿主把它路由成提案动作，绝不是直接写操作。
    pub ai_intent: Option<Command>,
}

pub fn detect(source: &str, cursor: usize) -> Option<Trigger> {
    if !source.is_char_boundary(cursor) {
        return None;
    }
    let line_start = source[..cursor].rfind('\n').map_or(0, |at| at + 1);
    let line_end = source[cursor..]
        .find('\n')
        .map_or(source.len(), |at| cursor + at);
    if !source[cursor..line_end].trim().is_empty() {
        return None;
    }
    let prefix = &source[line_start..cursor];
    let indent = prefix.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let token = &prefix[indent..];
    let query = token.strip_prefix('/')?;
    if query.len() > 32 || !query.bytes().all(|byte| byte.is_ascii_lowercase()) {
        return None;
    }
    let start = line_start + indent;
    if inside_frontmatter(source, start) {
        return None;
    }
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        if range.start <= start
            && start < range.end
            && matches!(
                event,
                Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock)
                    | Event::Code(_)
                    | Event::InlineMath(_)
                    | Event::DisplayMath(_)
            )
        {
            return None;
        }
    }
    Some(Trigger {
        range: start..cursor,
        query: query.into(),
        original: token.into(),
    })
}

fn inside_frontmatter(source: &str, at: usize) -> bool {
    let (source, at) = source
        .strip_prefix('\u{feff}')
        .map_or((source, at), |rest| {
            (rest, at.saturating_sub('\u{feff}'.len_utf8()))
        });
    let Some(first_end) = source.find('\n') else {
        return false;
    };
    if source[..first_end].trim_end_matches('\r') != "---" {
        return false;
    }
    let mut end = first_end + 1;
    for line in source[end..].split_inclusive('\n') {
        end += line.len();
        if matches!(line.trim_end_matches(['\r', '\n']), "---" | "...") {
            return at < end;
        }
    }
    true
}

pub fn complete(source: &str, trigger: &Trigger, command: Command) -> Result<Completion> {
    if source.get(trigger.range.clone()) != Some(trigger.original.as_str())
        || detect(source, trigger.range.end).as_ref() != Some(trigger)
        || !trigger.commands().contains(&command)
    {
        bail!("命令位置已变化，请重新输入 /");
    }
    let replacement = match command {
        Command::Todo => "- [ ] ",
        Command::Heading => "## ",
        _ => "",
    }
    .to_owned();
    Ok(Completion {
        cursor_after: trigger.range.start + replacement.len(),
        range: trigger.range.clone(),
        replacement,
        ai_intent: command.is_ai().then_some(command),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_commands_without_changing_surrounding_utf8_or_crlf() {
        let source = "前文🙂\r\n\r\n  /to\r\n";
        let cursor = source.find("/to").unwrap() + 3;
        let trigger = detect(source, cursor).unwrap();
        assert_eq!(trigger.commands(), vec![Command::Todo]);
        let completion = complete(source, &trigger, Command::Todo).unwrap();
        let mut edited = source.to_owned();
        edited.replace_range(completion.range, &completion.replacement);
        assert_eq!(edited, "前文🙂\r\n\r\n  - [ ] \r\n");
        assert!(completion.ai_intent.is_none());
    }

    #[test]
    fn ordinary_slashes_and_code_are_not_commands() {
        for source in [
            "文字 /todo",
            "/usr/local",
            "/readme.md",
            "https://host/ai",
            "    /todo",
            "\t/todo",
            "> /todo",
            "- /todo",
            "```rust\n/todo",
            "~~~\n/todo\n~~~",
            "$$\n/todo\n$$",
            "---\n/todo\n---",
            "<!--\n/todo\n-->",
        ] {
            let cursor = source.find("/todo").map_or(source.len(), |at| at + 5);
            assert!(
                detect(source, cursor).is_none(),
                "unexpected trigger: {source:?}"
            );
        }
        assert!(detect("🙂", 1).is_none());
        assert!(detect("", 4).is_none());
    }

    #[test]
    fn stale_and_unknown_completions_do_not_edit_source() {
        let trigger = detect("/todo", 5).unwrap();
        assert!(complete("/todoX", &trigger, Command::Todo).is_err());
        assert!(complete("/ai", &trigger, Command::Todo).is_err());
        assert!(complete("/todo", &trigger, Command::Heading).is_err());
        let ai = detect("/rewrite", 8).unwrap();
        let completion = complete("/rewrite", &ai, Command::Rewrite).unwrap();
        assert_eq!(completion.ai_intent, Some(Command::Rewrite));
        assert!(completion.replacement.is_empty());
    }
}
