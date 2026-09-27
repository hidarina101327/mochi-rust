//! 格式操作直接修改源码，每次操作只产生一条撤销记录。

use super::editor::TextBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
pub enum Format {
    Bold,
    Italic,
    Strike,
    Underline,
    Code,
    ClearFormat,
    /// 0 = 正文，1..=6 = 标题级别。
    Heading(u8),
    BulletList,
    OrderedList,
    TaskList,
    Table,
    Image,
    Link,
    CodeBlock,
    Quote,
    Rule,
    HighlightBlock,
    Details,
}

impl Format {
    /// 配对标记符（行内格式）。
    fn markers(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Format::Bold => ("**", "**"),
            Format::Italic => ("*", "*"),
            Format::Strike => ("~~", "~~"),
            Format::Underline => ("<u>", "</u>"),
            Format::Code => ("`", "`"),
            _ => return None,
        })
    }

    /// 行前缀（块格式）。有序列表按行号生成，单独处理。
    fn line_prefix(self) -> Option<&'static str> {
        Some(match self {
            Format::BulletList => "- ",
            Format::TaskList => "- [ ] ",
            Format::Quote => "> ",
            _ => return None,
        })
    }
}

/// 光标所在行的字节范围（不含换行）。
pub fn line_range(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let start = text[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[offset..]
        .find('\n')
        .map(|i| offset + i)
        .unwrap_or(text.len());
    (start, end)
}

/// 选区覆盖的整行范围（没有选区就是光标所在行）。
fn covered_lines(buffer: &TextBuffer) -> (usize, usize) {
    let (a, b) = buffer.selection();
    let (start, _) = line_range(buffer.text(), a);
    // 选区尾巴刚好落在行首时，那一行不算被选中
    let b = if b > a && buffer.text()[..b].ends_with('\n') {
        b - 1
    } else {
        b
    };
    let (_, end) = line_range(buffer.text(), b);
    (start, end)
}

/// 当前行的标题级别（0 = 不是标题）。工具栏的段落格式按钮显示用。
pub fn heading_level_at(text: &str, offset: usize) -> u8 {
    let (s, e) = line_range(text, offset);
    let line = text[s..e].trim_start();
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        hashes as u8
    } else {
        0
    }
}

/// 当前行是否已带某个块前缀。工具栏高亮用。
pub fn line_has(text: &str, offset: usize, format: Format) -> bool {
    let (s, e) = line_range(text, offset);
    let line = text[s..e].trim_start();
    match format {
        Format::OrderedList => ordered_prefix_len(line) > 0,
        Format::BulletList => {
            (line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ "))
                && !line.starts_with("- [")
        }
        Format::TaskList => {
            line.starts_with("- [ ] ") || line.starts_with("- [x] ") || line.starts_with("- [X] ")
        }
        Format::Quote => line.starts_with('>'),
        _ => false,
    }
}

fn ordered_prefix_len(line: &str) -> usize {
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && (line[digits..].starts_with(". ") || line[digits..].starts_with(") ")) {
        digits + 2
    } else {
        0
    }
}

/// 去掉一行已有的块前缀（标题井号、列表符号、引用符号），返回去掉了几个字节。
fn strip_block_prefix(line: &str) -> usize {
    let indent = line.len() - line.trim_start().len();
    let body = &line[indent..];
    let hashes = body.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
        return indent + hashes + 1;
    }
    for p in ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ ", "> "] {
        if body.starts_with(p) {
            return indent + p.len();
        }
    }
    let n = ordered_prefix_len(body);
    if n > 0 {
        return indent + n;
    }
    indent
}

pub fn apply(buffer: &mut TextBuffer, format: Format) {
    if let Some((open, close)) = format.markers() {
        toggle_wrap(buffer, open, close);
        return;
    }
    match format {
        Format::ClearFormat => clear_formatting(buffer),
        Format::Heading(level) => set_heading(buffer, level),
        Format::OrderedList => toggle_ordered(buffer),
        Format::BulletList | Format::TaskList | Format::Quote => {
            toggle_prefix(buffer, format.line_prefix().unwrap())
        }
        Format::Table => insert_block(
            buffer,
            "| 列 1 | 列 2 | 列 3 |\n| --- | --- | --- |\n|  |  |  |",
        ),
        Format::Rule => insert_block(buffer, "---"),
        Format::CodeBlock => wrap_code_block(buffer),
        Format::Link => insert_link(buffer, "[", "](url)"),
        Format::Image => insert_link(buffer, "![", "](url)"),
        Format::HighlightBlock => {
            super::containers::insert(buffer, super::containers::Kind::Highlight)
        }
        Format::Details => super::containers::insert(buffer, super::containers::Kind::Details),
        _ => {}
    }
}

/// 给选区加上/去掉配对标记符。没有选区时插入一对，光标落在中间。
fn toggle_wrap(buffer: &mut TextBuffer, open: &str, close: &str) {
    let (a, b) = buffer.selection();
    let text = buffer.text();
    if a == b {
        // 光标正夹在一对空标记符中间（刚插入的那对）：再按一次就是取消
        if text[..a].ends_with(open) && text[a..].starts_with(close) {
            buffer.replace_range(a - open.len()..a + close.len(), "");
            return;
        }
        let with = format!("{open}{close}");
        buffer.replace_range_select(a..a, &with, open.len()..open.len());
        return;
    }
    let selected = &text[a..b];
    // 选区自身带着标记符：去掉
    if selected.len() >= open.len() + close.len()
        && selected.starts_with(open)
        && selected.ends_with(close)
    {
        let inner = selected[open.len()..selected.len() - close.len()].to_owned();
        let len = inner.len();
        buffer.replace_range_select(a..b, &inner, 0..len);
        return;
    }
    if text[..a].ends_with(open) && text[b..].starts_with(close) {
        let inner = selected.to_owned();
        let len = inner.len();
        buffer.replace_range_select(a - open.len()..b + close.len(), &inner, 0..len);
        return;
    }
    let with = format!("{open}{selected}{close}");
    let inner = open.len()..open.len() + selected.len();
    buffer.replace_range_select(a..b, &with, inner);
}

fn clear_formatting(buffer: &mut TextBuffer) {
    let (a, b) = buffer.selection();
    if a == b {
        return;
    }
    let tags =
        regex::Regex::new(r"(?i)</?(?:span|mark|u|s|del|strong|b|em|i)(?:\s[^>]*)?>").unwrap();
    let mut s = tags.replace_all(&buffer.text()[a..b], "").into_owned();
    for m in ["**", "__", "~~", "<u>", "</u>", "`", "*", "_"] {
        s = s.replace(m, "");
    }
    let len = s.len();
    buffer.replace_range_select(a..b, &s, 0..len);
}

/// 设定当前行的标题级别；0 变回正文。
fn set_heading(buffer: &mut TextBuffer, level: u8) {
    let (s, e) = line_range(buffer.text(), buffer.cursor());
    let line = buffer.text()[s..e].to_owned();
    let strip = strip_block_prefix(&line);
    let body = &line[strip..];
    let prefix = if level == 0 {
        String::new()
    } else {
        format!("{} ", "#".repeat(level as usize))
    };
    let new_line = format!("{prefix}{body}");
    // 光标尽量停在原来的字上
    let cursor_in_body = buffer.cursor().saturating_sub(s + strip);
    buffer.replace_range(s..e, &new_line);
    let target = (s + prefix.len() + cursor_in_body).min(s + new_line.len());
    buffer.set_cursor(target, false);
}

/// 切换若干行的前缀：全带就都去掉，否则都加上（先剥掉别的块前缀，标题除外）。
fn toggle_prefix(buffer: &mut TextBuffer, prefix: &str) {
    let (s, e) = covered_lines(buffer);
    let block = buffer.text()[s..e].to_owned();
    let all_have = block.lines().all(|l| l.trim_start().starts_with(prefix));
    let rebuilt: Vec<String> = block
        .lines()
        .map(|l| {
            let indent = l.len() - l.trim_start().len();
            let (ws, body) = l.split_at(indent);
            if all_have {
                format!("{ws}{}", &body[prefix.len()..])
            } else if body.starts_with(prefix) {
                l.to_owned()
            } else {
                let strip = strip_block_prefix(body);
                format!("{ws}{prefix}{}", &body[strip..])
            }
        })
        .collect();
    let joined = rebuilt.join("\n");
    let len = joined.len();
    buffer.replace_range_select(s..e, &joined, 0..len);
    if block.lines().count() <= 1 {
        // 单行：不留选区，光标放到行尾
        buffer.set_cursor(s + len, false);
    }
}

fn toggle_ordered(buffer: &mut TextBuffer) {
    let (s, e) = covered_lines(buffer);
    let block = buffer.text()[s..e].to_owned();
    let all_have = block
        .lines()
        .all(|l| ordered_prefix_len(l.trim_start()) > 0);
    let rebuilt: Vec<String> = block
        .lines()
        .enumerate()
        .map(|(i, l)| {
            let indent = l.len() - l.trim_start().len();
            let (ws, body) = l.split_at(indent);
            if all_have {
                format!("{ws}{}", &body[ordered_prefix_len(body)..])
            } else {
                let strip = strip_block_prefix(body);
                format!("{ws}{}. {}", i + 1, &body[strip..])
            }
        })
        .collect();
    let joined = rebuilt.join("\n");
    let len = joined.len();
    buffer.replace_range_select(s..e, &joined, 0..len);
    if block.lines().count() <= 1 {
        buffer.set_cursor(s + len, false);
    }
}

/// 在当前行之后另起一段插入块模板；当前行为空则就地插入。
pub fn insert_block(buffer: &mut TextBuffer, template: &str) {
    let (s, mut e) = line_range(buffer.text(), buffer.cursor());
    if e > s && buffer.text().as_bytes()[e - 1] == b'\r' {
        e -= 1;
    }
    let newline = if buffer.text().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let line_empty = buffer.text()[s..e].trim().is_empty();
    if line_empty {
        buffer.replace_range(s..e, template);
    } else {
        let with = format!("{newline}{newline}{template}");
        buffer.replace_range(e..e, &with);
    }
}

fn wrap_code_block(buffer: &mut TextBuffer) {
    let (s, e) = covered_lines(buffer);
    let inner = buffer.text()[s..e].to_owned();
    let with = format!("```\n{inner}\n```");
    buffer.replace_range(s..e, &with);
    // 光标放到第一行代码开头（或空代码块的中间行）
    buffer.set_cursor(s + 4, false);
}

fn insert_link(buffer: &mut TextBuffer, open: &str, close: &str) {
    let (a, b) = buffer.selection();
    let label = buffer.text()[a..b].to_owned();
    let with = format!("{open}{label}{close}");
    // 选中 `url` 占位，直接敲就是地址
    let url_start = open.len() + label.len() + 2;
    buffer.replace_range_select(a..b, &with, url_start..url_start + 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str, a: usize, b: usize) -> TextBuffer {
        let mut b0 = TextBuffer::new(text);
        b0.set_cursor(a, false);
        b0.set_cursor(b, true);
        b0
    }

    #[test]
    fn bold_wraps_the_selection_and_keeps_it_selected() {
        let mut b = buf("加粗这个词", 6, 12);
        apply(&mut b, Format::Bold);
        assert_eq!(b.text(), "加粗**这个**词");
        assert_eq!(b.selected_text(), "这个");
        // 再按一次：去掉
        apply(&mut b, Format::Bold);
        assert_eq!(b.text(), "加粗这个词");
        assert_eq!(b.selected_text(), "这个");
    }

    #[test]
    fn bold_without_a_selection_inserts_a_pair_and_a_second_press_removes_it() {
        let mut b = buf("ab", 1, 1);
        apply(&mut b, Format::Bold);
        assert_eq!(b.text(), "a****b");
        assert_eq!(b.cursor(), 3);
        apply(&mut b, Format::Bold);
        assert_eq!(b.text(), "ab");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn selecting_the_marked_text_including_markers_unwraps_it() {
        let mut b = buf("x**粗**y", 1, 8);
        apply(&mut b, Format::Bold);
        assert_eq!(b.text(), "x粗y");
    }

    #[test]
    fn headings_replace_any_existing_block_prefix_on_the_line() {
        let mut b = buf("- 列表项", 5, 5);
        apply(&mut b, Format::Heading(2));
        assert_eq!(b.text(), "## 列表项");
        assert_eq!(heading_level_at(b.text(), 3), 2);
        apply(&mut b, Format::Heading(0));
        assert_eq!(b.text(), "列表项");
        assert_eq!(heading_level_at(b.text(), 0), 0);
    }

    #[test]
    fn heading_keeps_the_cursor_on_the_same_character() {
        let mut b = buf("正文行", 3, 3);
        apply(&mut b, Format::Heading(1));
        assert_eq!(b.text(), "# 正文行");
        assert_eq!(b.cursor(), 5, "光标仍在「文」之前");
    }

    #[test]
    fn list_toggles_apply_to_every_selected_line_and_undo_together() {
        let mut b = buf("一\n二\n三", 0, 11);
        apply(&mut b, Format::BulletList);
        assert_eq!(b.text(), "- 一\n- 二\n- 三");
        apply(&mut b, Format::OrderedList);
        assert_eq!(b.text(), "1. 一\n2. 二\n3. 三", "换成有序列表要先剥掉圆点");
        apply(&mut b, Format::OrderedList);
        assert_eq!(b.text(), "一\n二\n三", "全带前缀时再按一次是去掉");
        b.undo();
        assert_eq!(b.text(), "1. 一\n2. 二\n3. 三", "一次操作一步撤销");
    }

    #[test]
    fn task_list_and_quote_prefixes_are_recognised_for_toolbar_state() {
        let mut b = buf("待办", 0, 0);
        apply(&mut b, Format::TaskList);
        assert_eq!(b.text(), "- [ ] 待办");
        assert!(line_has(b.text(), 3, Format::TaskList));
        assert!(!line_has(b.text(), 3, Format::BulletList));
        let mut q = buf("引用", 0, 0);
        apply(&mut q, Format::Quote);
        assert_eq!(q.text(), "> 引用");
        assert!(line_has(q.text(), 0, Format::Quote));
    }

    #[test]
    fn block_templates_go_on_their_own_paragraph_after_a_non_empty_line() {
        let mut b = buf("段落", 3, 3);
        apply(&mut b, Format::Rule);
        assert_eq!(b.text(), "段落\n\n---");
        let mut e = buf("", 0, 0);
        apply(&mut e, Format::Table);
        assert!(e
            .text()
            .starts_with("| 列 1 | 列 2 | 列 3 |\n| --- | --- | --- |"));
    }

    #[test]
    fn code_block_wraps_the_selected_lines_in_fences() {
        let mut b = buf("let a = 1;\nlet b = 2;", 0, 21);
        apply(&mut b, Format::CodeBlock);
        assert_eq!(b.text(), "```\nlet a = 1;\nlet b = 2;\n```");
        assert_eq!(b.cursor(), 4);
    }

    #[test]
    fn link_uses_the_selection_as_label_and_selects_the_url_placeholder() {
        let mut b = buf("看这里", 0, 9);
        apply(&mut b, Format::Link);
        assert_eq!(b.text(), "[看这里](url)");
        assert_eq!(b.selected_text(), "url");
        let mut i = buf("", 0, 0);
        apply(&mut i, Format::Image);
        assert_eq!(i.text(), "![](url)");
        assert_eq!(i.selected_text(), "url");
    }

    #[test]
    fn clear_formatting_strips_inline_markers_inside_the_selection() {
        let mut b = buf("**粗**和*斜*和`码`", 0, 24);
        let end = b.text().len();
        b.set_cursor(0, false);
        b.set_cursor(end, true);
        apply(&mut b, Format::ClearFormat);
        assert_eq!(b.text(), "粗和斜和码");
    }

    #[test]
    fn clear_formatting_removes_color_but_preserves_links_and_other_html() {
        let raw = "<span style=\"color:#abc\">**中文**</span><mark>重点</mark>[链接](note.md)<br>";
        let mut b = buf(raw, 0, raw.len());
        apply(&mut b, Format::ClearFormat);
        assert_eq!(b.text(), "中文重点[链接](note.md)<br>");
        b.undo();
        assert_eq!(b.text(), raw);
    }

    #[test]
    fn line_range_handles_first_last_and_middle_lines() {
        let t = "ab\ncd\nef";
        assert_eq!(line_range(t, 0), (0, 2));
        assert_eq!(line_range(t, 4), (3, 5));
        assert_eq!(line_range(t, 8), (6, 8));
    }
}
