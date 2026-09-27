//! 预处理 AI Markdown 文本，识别代码语言并规范数学公式分隔符。
use super::*;

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 把 Electron 渲染器接受的两种 LaTeX 定界符写法归一化，同时让所有代码
/// span 原样保留。`message_ops::copy_markdown` 对完整围栏有同样的转换，
/// 但流式响应可能在闭合围栏到达前就结束。此时解析器不会发出配对的
/// `End(CodeBlock)` 事件，所以这里也必须把未闭合的区间保护到源文本末尾。
pub(super) fn normalize_math_delimiters(source: &str) -> String {
    static BLOCK: OnceLock<regex::Regex> = OnceLock::new();
    static INLINE: OnceLock<regex::Regex> = OnceLock::new();
    let block = BLOCK.get_or_init(|| regex::Regex::new(r"(?s)\\\[\s*(.*?)\s*\\\]").unwrap());
    let inline = INLINE.get_or_init(|| regex::Regex::new(r"(?s)\\\(\s*(.*?)\s*\\\)").unwrap());

    let mut protected = Vec::<Range<usize>>::new();
    let mut open_code = None;
    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => {
                // Markdown 不会嵌套代码块，但保留首个 start
                // 能让实现对畸形流式输入更健壮。
                if open_code.is_none() {
                    open_code = Some(range.start);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = open_code.take() {
                    protected.push(start..range.end);
                }
            }
            Event::Code(_) => protected.push(range),
            _ => {}
        }
    }
    if let Some(start) = open_code {
        protected.push(start..source.len());
    }
    protected.sort_by_key(|range| range.start);

    // 区间可能相邻（行内代码紧跟着围栏块），切片前先合并。
    // 每个区间都由 pulldown-cmark 生成在 UTF-8 边界上。
    let mut merged = Vec::<Range<usize>>::new();
    for range in protected {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }

    let convert = |value: &str| {
        let value = block
            .replace_all(value, |caps: &regex::Captures<'_>| {
                format!("\n$$\n{}\n$$\n", caps[1].trim())
            })
            .into_owned();
        let value = inline
            .replace_all(&value, |caps: &regex::Captures<'_>| {
                format!("${}$", caps[1].trim())
            })
            .into_owned();
        // pulldown-cmark 的 math 扩展会把未配对的 `\(`/`\[` 当成行内公式
        // 开定界符，并把它从输出文本里丢掉。流式渲染时，半截公式就会变成
        // 一段没有定界符、令人误解的数学 run。这里把未配对的开启符变成
        // 字面 Markdown 转义（双反斜杠）；解析器随后会输出原来的单个
        // 反斜杠，半截源码就能一直可见。
        escape_unclosed_math(&value)
    };
    let mut result = String::with_capacity(source.len());
    let mut offset = 0;
    for range in merged {
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

fn escape_unclosed_math(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let start = index;
            while index < bytes.len() && bytes[index] == b'\\' {
                index += 1;
            }
            let slash_count = index - start;
            result.push_str(&value[start..index]);
            if index < bytes.len() && matches!(bytes[index], b'(' | b'[') && slash_count % 2 == 1 {
                result.push('\\');
            }
            continue;
        }
        let character = value[index..].chars().next().expect("valid UTF-8 boundary");
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn code_language(kind: CodeBlockKind<'_>) -> String {
    match kind {
        CodeBlockKind::Indented => String::new(),
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned(),
    }
}

pub(super) fn table_payload(node: &Node) -> CopyPayload {
    let mut html = String::from("<table style=\"border-collapse:collapse\">");
    let mut rows = Vec::new();
    for row in &node.children {
        html.push_str("<tr>");
        let tag = if matches!(row.kind, Kind::Row(true)) {
            "th"
        } else {
            "td"
        };
        let mut cells = Vec::new();
        for cell in &row.children {
            html.push_str(&format!("<{tag} style=\"border:1px solid #e7e8ea;padding:6px 8px;text-align:left;vertical-align:top\">"));
            let mut plain = String::new();
            for run in inline_runs(cell) {
                // TeX 在这里无损复制。完整的 KaTeX/MathML 剪贴板对等另行处理。
                plain.push_str(&run.text);
                let escaped = escape_html(&run.text).replace('\n', "<br>");
                let flags = match run.emphasis {
                    Emphasis::Styled { flags, .. } => flags,
                    Emphasis::Bold => 1,
                    Emphasis::Italic => 2,
                    Emphasis::BoldItalic => 3,
                    Emphasis::Code => 16,
                    _ => 0,
                };
                let mut value = escaped;
                if flags & 16 != 0 {
                    value = format!("<code>{value}</code>")
                }
                if flags & 8 != 0 {
                    value = format!("<s>{value}</s>")
                }
                if flags & 2 != 0 {
                    value = format!("<em>{value}</em>")
                }
                if flags & 1 != 0 {
                    value = format!("<strong>{value}</strong>")
                }
                if flags & 4 != 0 {
                    value = format!("<u>{value}</u>")
                }
                html.push_str(&value);
            }
            cells.push(plain.split_whitespace().collect::<Vec<_>>().join(" "));
            html.push_str(&format!("</{tag}>"));
        }
        html.push_str("</tr>");
        rows.push(cells.join("\t"));
    }
    html.push_str("</table>");
    CopyPayload {
        kind: CopyKind::Table,
        text: rows.join("\n"),
        html: Some(html),
    }
}

fn flush_runs(node: &mut Node) {
    if !node.runs.is_empty() {
        let mut paragraph = Node::new(Kind::Inline);
        paragraph.runs = std::mem::take(&mut node.runs);
        node.children.push(paragraph);
    }
}

pub(super) fn parse(source: &str) -> Result<Node, ()> {
    let mut stack = vec![Node::new(Kind::Root)];
    let mut strong = 0_u32;
    let mut italic = 0_u32;
    let mut strike = 0_u32;
    let mut links = 0_u32;
    let mut image_depth = 0_u32;
    for (count, (event, range)) in Parser::new_ext(
        source,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_MATH,
    )
    .into_offset_iter()
    .enumerate()
    {
        if count > 100_000 || stack.len() > 64 {
            return Err(());
        }
        let mut push = None;
        let mut pop = false;
        let mut value = None;
        match event {
            Event::Start(tag) => match tag {
                Tag::Emphasis => italic += 1,
                Tag::Strong => strong += 1,
                Tag::Strikethrough => strike += 1,
                Tag::Link { dest_url, .. } => {
                    links += 1;
                    push = Some(Kind::Link(dest_url.into_string()));
                }
                Tag::Image { .. } => {
                    image_depth += 1;
                    value = Some(("[图片：".into(), false, false));
                }
                Tag::Paragraph => push = Some(Kind::Paragraph),
                Tag::Heading { level, .. } => push = Some(Kind::Heading(level as u8)),
                Tag::BlockQuote(_) => push = Some(Kind::Quote),
                Tag::List(_) => push = Some(Kind::List),
                Tag::Item => push = Some(Kind::Item),
                Tag::CodeBlock(kind) => {
                    push = Some(Kind::Code {
                        language: code_language(kind),
                    })
                }
                Tag::Table(_) => push = Some(Kind::Table),
                Tag::TableHead => push = Some(Kind::Row(true)),
                Tag::TableRow => push = Some(Kind::Row(false)),
                Tag::TableCell => push = Some(Kind::Cell),
                Tag::HtmlBlock => push = Some(Kind::Html),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Emphasis => italic = italic.saturating_sub(1),
                TagEnd::Strong => strong = strong.saturating_sub(1),
                TagEnd::Strikethrough => strike = strike.saturating_sub(1),
                TagEnd::Link => {
                    links = links.saturating_sub(1);
                    pop = true;
                }
                TagEnd::Image => {
                    image_depth = image_depth.saturating_sub(1);
                    value = Some(("]".into(), false, false));
                }
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell
                | TagEnd::HtmlBlock => pop = true,
                _ => {}
            },
            Event::Text(s) => value = Some((s.into_string(), false, false)),
            Event::Code(s) => value = Some((s.into_string(), true, false)),
            Event::InlineMath(s) => value = Some((s.into_string(), false, true)),
            Event::DisplayMath(s) => {
                let mut node = Node::new(Kind::Math);
                node.anchor = range.start;
                node.runs.push(Run::plain(s.into_string()));
                let parent = stack.last_mut().unwrap();
                flush_runs(parent);
                parent.children.push(node);
            }
            Event::SoftBreak | Event::HardBreak => value = Some(("\n".into(), false, false)),
            Event::TaskListMarker(done) => {
                value = Some((if done { "☑ " } else { "☐ " }.into(), false, false))
            }
            Event::Rule => {
                let parent = stack.last_mut().unwrap();
                flush_runs(parent);
                parent.children.push(Node::new(Kind::Rule));
            }
            Event::Html(s) => {
                if matches!(stack.last().unwrap().kind, Kind::Html) {
                    value = Some((s.into_string(), false, false));
                }
            }
            // 行内原始 HTML 标签绝不执行，文本事件保持可见。
            Event::InlineHtml(_) => {}
            _ => {}
        }
        if let Some(kind) = push {
            let parent = stack.last_mut().unwrap();
            flush_runs(parent);
            let mut node = Node::new(kind);
            node.anchor = range.start;
            stack.push(node);
        }
        if pop && stack.len() > 1 {
            let mut node = stack.pop().unwrap();
            if !node.children.is_empty() {
                flush_runs(&mut node);
            }
            stack.last_mut().unwrap().children.push(node);
        }
        if let Some((text, code, math)) = value {
            let literal = matches!(stack.last().unwrap().kind, Kind::Code { .. } | Kind::Html);
            let flags = if literal {
                0
            } else if code {
                16 | (u8::from(links > 0) * 32)
            } else if math {
                64 | (u8::from(links > 0) * 32)
            } else {
                u8::from(strong > 0)
                    | (u8::from(italic > 0) * 2)
                    | (u8::from(strike > 0) * 8)
                    | (u8::from(links > 0) * 36)
            };
            let emphasis = if flags == 0 {
                Emphasis::None
            } else {
                Emphasis::Styled {
                    fg: None,
                    bg: None,
                    flags,
                }
            };
            let parent = stack.last_mut().unwrap();
            if let Some(last) = parent
                .runs
                .last_mut()
                .filter(|r| r.emphasis == emphasis && !code && !math)
            {
                last.text.push_str(&text);
            } else {
                parent.runs.push(Run { text, emphasis });
            }
        }
    }
    while stack.len() > 1 {
        let node = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(node);
    }
    Ok(stack.pop().unwrap())
}
