//! 解析文档行和内容块，并识别表格、代码块及文档头部。
use super::*;

/// 把 Markdown 源码切成带范围的块。编辑器要知道每个块对应源码的哪一段。
pub fn parse_ranged(source: &str) -> Parsed {
    #[cfg(test)]
    PARSE_CALLS.with(|calls| calls.set(calls.get() + 1));
    // 每行的起点字节。`lines()` 会吃掉 `\r`，范围按不含 `\r` 的内容算
    let mut starts = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let mut pos = 0;
    for raw in source.split_inclusive('\n') {
        starts.push(pos);
        let content = raw.strip_suffix('\n').unwrap_or(raw);
        let content = content.strip_suffix('\r').unwrap_or(content);
        lines.push(content);
        pos += raw.len();
    }
    // 以换行结尾的文档还有一个空的末行，光标能停在那里
    if source.is_empty() || source.ends_with('\n') {
        starts.push(source.len());
        lines.push("");
    }
    let body = strip_frontmatter(&lines);
    let body = &body[body
        .iter()
        .take_while(|line| crate::ui::block_markers::standalone(line))
        .count()..];
    let skipped = lines.len() - body.len();
    let body_start = starts.get(skipped).copied().unwrap_or(source.len());
    let mut ordinals = Vec::new();
    let mut blocks: Vec<RangedBlock> = parse_lines(body, &mut ordinals)
        .into_iter()
        .map(|(block, first, count)| {
            let first = first + skipped;
            let last = first + count.max(1) - 1;
            RangedBlock {
                block,
                start: starts[first],
                end: starts[last] + lines[last].len(),
            }
        })
        .collect();
    let code_metadata = blocks
        .iter()
        .filter(|b| matches!(b.block, Block::Code { .. }))
        .map(|b| crate::ui::code_blocks::metadata(source, b.start).0)
        .filter(|r| !r.is_empty())
        .collect::<Vec<_>>();
    let mut metadata_index = 0;
    blocks.retain(|block| {
        while code_metadata
            .get(metadata_index)
            .is_some_and(|range| range.end <= block.start)
        {
            metadata_index += 1;
        }
        !code_metadata
            .get(metadata_index)
            .is_some_and(|range| range.contains(&block.start))
    });
    crate::ui::containers::project(source, &mut blocks);
    if blocks.is_empty() {
        blocks.push(RangedBlock {
            block: Block::Blank,
            start: body_start,
            end: body_start,
        });
    }
    Parsed { blocks, body_start }
}

/// 从第二行开始查找文档头部的结束分隔线；若未闭合，则保留整段正文。
fn strip_frontmatter<'a>(lines: &'a [&'a str]) -> &'a [&'a str] {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return lines;
    }
    match lines[1..].iter().position(|l| l.trim_end() == "---") {
        Some(offset) => &lines[offset + 2..],
        None => lines,
    }
}

/// 逐行切块。返回 (块, 首行下标, 行数)——编辑器要靠行号反推源码范围。
pub fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let marker = line.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = line.chars().take_while(|c| *c == marker).count();
    if count < 3 {
        return None;
    }
    let info = &line[count..];
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some((marker, count, info.trim()))
}

fn fence_close(line: &str, marker: char, count: usize) -> bool {
    let line = line.trim();
    let n = line.chars().take_while(|c| *c == marker).count();
    n >= count && line[n..].trim().is_empty()
}

fn parse_lines(lines: &[&str], ordinals: &mut Vec<usize>) -> Vec<(Block, usize, usize)> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        let first = i;

        if crate::ui::block_markers::standalone(trimmed) {
            i += 1;
            continue;
        }

        // TipTap/Electron 可能把表格持久化成 HTML。这里接受这种形式，
        // 打开旧的 Electron 笔记时就不会露出原始标签。
        if trimmed
            .get(..6)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("<table"))
        {
            let mut html = raw.to_owned();
            let mut closed = raw.to_ascii_lowercase().contains("</table>");
            i += 1;
            while i < lines.len() && !closed {
                html.push('\n');
                html.push_str(lines[i]);
                closed = lines[i].to_ascii_lowercase().contains("</table>");
                i += 1;
            }
            if let Some((rows, header)) = parse_html_table(&html) {
                blocks.push((Block::Table { rows, header }, first, i - first));
                ordinals.clear();
                continue;
            }
            i = first;
        }

        // 围栏代码块必须有闭合围栏。未闭合的 Markdown 不是一个可编辑的
        // CodeBlock：若把它吞到文件末尾，用户在卡片下方插行也会继续落进代码。
        // 所见即所得的输入规则会在 Enter 时补上闭合围栏，见 editor_blocks；
        // 这里因此只负责忠实渲染已经结构完整的源码。
        if i + 1 < lines.len() {
            if let Some((marker, count, lang)) = fence_open(trimmed) {
                let lang = lang.trim().to_owned();
                let mut body = Vec::new();
                let mut end = i + 1;
                while end < lines.len() && !fence_close(lines[end], marker, count) {
                    body.push(lines[end].to_owned());
                    end += 1;
                }
                if end < lines.len() {
                    // 吃掉闭合围栏；未闭合时回落到普通段落，绝不吞后文。
                    i = end + 1;
                    blocks.push((Block::Code { lang, lines: body }, first, i - first));
                    ordinals.clear();
                    continue;
                }
            }
        }

        if trimmed.is_empty() {
            blocks.push((Block::Blank, first, 1));
            ordinals.clear();
            i += 1;
            continue;
        }

        // 独占段落的 $$ / \[ \]；未闭合围栏保持普通文本，不吞掉后文。
        let math_delimiter = if trimmed.starts_with("$$") {
            Some(("$$", "$$"))
        } else if trimmed.starts_with("\\[") {
            Some(("\\[", "\\]"))
        } else {
            None
        };
        if let Some((open, close)) = math_delimiter {
            let rest = &trimmed[open.len()..];
            if let Some(body) = rest.strip_suffix(close) {
                blocks.push((Block::Math(body.trim().to_owned()), first, 1));
                i += 1;
                continue;
            }
            if rest.trim().is_empty() {
                if let Some(end) = lines[i + 1..].iter().position(|l| l.trim() == close) {
                    let end = i + 1 + end;
                    blocks.push((
                        Block::Math(lines[i + 1..end].join("\n")),
                        first,
                        end - i + 1,
                    ));
                    i = end + 1;
                    ordinals.clear();
                    continue;
                }
            }
        }

        if !raw.starts_with("    ") && !raw.starts_with('\t') {
            if let Some(reference) = crate::ui::object_link::ObjectLink::parse(trimmed) {
                // 保留旧版 AI 卡片的预览行为，同时共用普通文本的展示样式。
                if mochi_core::ai::locator::Locator::parse(trimmed).is_none()
                    || reference.text_style
                {
                    blocks.push((Block::ObjectReference(reference), first, 1));
                    i += 1;
                    ordinals.clear();
                    continue;
                }
            }
            if let Some(locator) = mochi_core::ai::locator::Locator::parse(trimmed) {
                blocks.push((Block::AiLocator(locator), first, 1));
                i += 1;
                ordinals.clear();
                continue;
            }
        }
        // 分隔线：--- / *** / ___
        if let Some((align, level, text)) = crate::ui::styles::aligned(trimmed) {
            blocks.push((Block::Aligned { align, level, text }, first, 1));
            i += 1;
            continue;
        }
        if is_divider(trimmed) {
            blocks.push((Block::Divider, first, 1));
            i += 1;
            continue;
        }

        // 标题：# 到 ######，井号后必须有空格（`#标签` 不是标题）
        if let Some(rest) = raw.trim_start().strip_prefix('#') {
            let extra = rest.chars().take_while(|c| *c == '#').count();
            let level = (1 + extra).min(6) as u8;
            let after = &rest[extra..];
            if let Some(body) = after.strip_prefix(' ') {
                blocks.push((
                    Block::Heading {
                        level,
                        text: body.trim().to_owned(),
                    },
                    first,
                    1,
                ));
                ordinals.clear();
                i += 1;
                continue;
            }
        }

        if let Some(body) = raw
            .trim_start()
            .strip_prefix("> ")
            .or_else(|| raw.trim_start().strip_prefix('>'))
        {
            blocks.push((Block::Quote(body.trim().to_owned()), first, 1));
            i += 1;
            continue;
        }

        let depth = indent_depth(line);
        if let Some(item) = parse_list_item(raw.trim_start(), depth, ordinals) {
            blocks.push((item, first, 1));
            i += 1;
            continue;
        }

        // 表格：本行有管道，下一行是分隔行 `| --- | :-: |`
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1].trim()) {
            let mut rows = vec![split_table_row(trimmed)];
            i += 2;
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(split_table_row(lines[i].trim()));
                i += 1;
            }
            let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
            for r in &mut rows {
                r.resize(cols, String::new());
            }
            blocks.push((Block::Table { rows, header: true }, first, i - first));
            ordinals.clear();
            continue;
        }

        // 独占一行的图片
        if let Some((alt, src, width)) = crate::ui::editor_images::html(trimmed)
            .or_else(|| parse_image_line(trimmed).map(|(alt, src)| (alt, src, None)))
        {
            blocks.push((Block::Image { alt, src, width }, first, 1));
            ordinals.clear();
            i += 1;
            continue;
        }

        ordinals.clear();
        // `trimmed` 只用于识别 Markdown 结构；段落正文必须保留原始空格。
        // 否则光标停在行尾输入一个空格时，解析和映射都会把它裁掉，直到下一个
        // 非空白字符出现才突然显示出来。
        blocks.push((Block::Paragraph(raw.to_owned()), first, 1));
        i += 1;
    }
    blocks
}

/// `| --- | :---: | ---: |`（首尾管道可省）。每格是若干个 `-`（GFM 允许一个，如 `:-:`），
/// 可带冒号对齐标记。
fn is_table_separator(line: &str) -> bool {
    if !line.contains('-') || !line.contains('|') && !line.contains(':') {
        return false;
    }
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    let cells: Vec<&str> = inner.split('|').map(str::trim).collect();
    !cells.is_empty()
        && cells.iter().all(|c| {
            let core = c.trim_start_matches(':').trim_end_matches(':');
            !core.is_empty() && core.chars().all(|ch| ch == '-')
        })
}

/// 按未转义的 `|` 切单元格，去掉首尾空格与首尾空格子。`\|` 还原成 `|`。
pub(super) fn split_table_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    cells.push(cur);
    // 首尾管道产生的空格子不算单元格
    if cells.first().map(|c| c.trim().is_empty()).unwrap_or(false) {
        cells.remove(0);
    }
    if cells.len() > 1 && cells.last().map(|c| c.trim().is_empty()).unwrap_or(false) {
        cells.pop();
    }
    cells.into_iter().map(|c| c.trim().to_owned()).collect()
}

/// 整行只有一个 `![alt](src)` 时是图片块。
fn parse_image_line(trimmed: &str) -> Option<(String, String)> {
    let rest = trimmed.strip_prefix("![")?;
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let after = &rest[close + 2..];
    let end = after.rfind(')')?;
    if end + 1 != after.len() {
        return None;
    }
    let src = after[..end].trim();
    // 去掉可选的标题 `![a](x.png "title")`
    let src = src
        .split_whitespace()
        .next()
        .unwrap_or(src)
        .trim_matches(['<', '>']);
    if src.is_empty() {
        return None;
    }
    Some((alt.to_owned(), src.to_owned()))
}

fn is_divider(s: &str) -> bool {
    let c = s.chars().next().unwrap_or(' ');
    matches!(c, '-' | '*' | '_') && s.len() >= 3 && s.chars().all(|x| x == c)
}

/// 缩进层级。四个空格或一个 Tab 算一级——CommonMark 的常见约定。
fn indent_depth(line: &str) -> usize {
    let mut spaces = 0;
    for ch in line.chars() {
        match ch {
            ' ' => spaces += 1,
            '\t' => spaces += 4,
            _ => break,
        }
    }
    spaces / 2 // 两个空格一级：实际笔记里两空格缩进比四空格常见得多
}

fn parse_list_item(trimmed: &str, depth: usize, ordinals: &mut Vec<usize>) -> Option<Block> {
    // 任务列表要先于无序列表判断——`- [ ] x` 也匹配无序列表的前缀
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(bullet) {
            let (marker, text) = if let Some(t) = rest.strip_prefix("[ ] ") {
                ("☐".to_owned(), t)
            } else if let Some(t) = rest
                .strip_prefix("[x] ")
                .or_else(|| rest.strip_prefix("[X] "))
            {
                ("☑".to_owned(), t)
            } else {
                ("•".to_owned(), rest)
            };
            return Some(Block::ListItem {
                marker,
                text: text.trim().to_owned(),
                depth,
            });
        }
    }

    // 有序列表：`1. ` / `1) `
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let after = &trimmed[digits.len()..];
        if let Some(text) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            // 序号按**源码里写的**走，不自己重编号——用户手写 1. 1. 1. 是常见写法，
            // 自动改成 1. 2. 3. 就和原文对不上了
            ordinals.resize(depth + 1, 0);
            ordinals[depth] += 1;
            return Some(Block::ListItem {
                marker: format!("{digits}."),
                text: text.trim().to_owned(),
                depth,
            });
        }
    }
    None
}
