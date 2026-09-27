//! 未知 HTML 标签保留子内容；脚本和可执行 URL 丢弃。

use scraper::{ElementRef, Html, Node};

const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_TABLE_ROWS: usize = 1_024;
const MAX_TABLE_COLUMNS: usize = 256;

/// 把 HTML 剪贴板片段转换成 Markdown/MC 源码。
///
/// 返回 `None` 表示输入超出了资源上限，或无法安全转换。
/// 空但合法的片段返回 `Some(String::new())`；剪贴板里只有一个
/// script/style 节点时正需要这个行为——调用方绝不能因此回退成
/// 把该节点的源码原样插进文档。
pub fn from_html(fragment: &str) -> Option<String> {
    if fragment.len() > MAX_INPUT_BYTES {
        return None;
    }

    let document = Html::parse_fragment(fragment);
    let root = document.root_element();
    let mut converter = Converter::default();
    let children: Vec<_> = root.children().collect();
    let blocks = converter.flow(&children, InlineStyle::default(), 0).ok()?;
    // 原生解析器本来就把每行源码当段落。
    // Markdown 式的双分隔线会画出可见的空行块。
    // 空条目代表显式的空 HTML 段落，必须保留。
    let output = blocks.join("\n");

    if output.len() > MAX_OUTPUT_BYTES {
        return None;
    }
    Some(output)
}

/// 转义纯文本剪贴板内容，插入 Markdown 缓冲时才不会意外
/// 生成标题、列表、链接、强调或围栏代码块。
pub fn plain_text_source(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    escape_text(&normalized, false, true)
}

#[derive(Debug, Clone, Default)]
struct Converter {
    nodes: usize,
}

#[derive(Debug, Clone, Default)]
struct InlineStyle {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    code: bool,
    color: Option<String>,
    background: Option<String>,
}

type ConvertResult<T> = Result<T, ()>;

impl Converter {
    fn visit(&mut self, _node: ego_tree::NodeRef<'_, Node>, depth: usize) -> ConvertResult<()> {
        if depth > MAX_DEPTH {
            return Err(());
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_NODES {
            return Err(());
        }
        Ok(())
    }

    fn flow(
        &mut self,
        nodes: &[ego_tree::NodeRef<'_, Node>],
        style: InlineStyle,
        depth: usize,
    ) -> ConvertResult<Vec<String>> {
        if depth > MAX_DEPTH {
            return Err(());
        }
        let mut blocks = Vec::new();
        let mut inline = String::new();

        for &node in nodes {
            self.visit(node, depth)?;
            if is_block_node(node) {
                push_inline_block(&mut blocks, &mut inline);
                blocks.extend(self.block(node, style.clone(), depth)?);
            } else {
                let rendered = self.inline(node, style.clone(), depth)?;
                append_limited(&mut inline, &rendered)?;
            }
        }
        push_inline_block(&mut blocks, &mut inline);
        Ok(blocks)
    }

    fn block(
        &mut self,
        node: ego_tree::NodeRef<'_, Node>,
        style: InlineStyle,
        depth: usize,
    ) -> ConvertResult<Vec<String>> {
        let Some(element) = ElementRef::wrap(node) else {
            return Ok(Vec::new());
        };
        let name = element.value().name().to_ascii_lowercase();

        if is_ignored_element(&name) {
            return Ok(Vec::new());
        }
        // 块级元素可以像行内 span 一样携带继承的呈现样式（Word 常把
        // font-weight/color 放在段落或 div 上）。下钻之前先应用它们，
        // 块内文本和嵌套子元素才能继承这份样式。
        let style = style_for_element(&element, &style);
        if name == "br" {
            return Ok(vec![String::new()]);
        }
        if heading_level(&name).is_some() {
            let body = self.inline_children(&element, style, depth + 1)?;
            let body = trim_inline_block(&body);
            let level = heading_level(&name).unwrap_or(1);
            if let Some(alignment) = text_alignment(element.attr("style").unwrap_or_default()) {
                return Ok(vec![format!(
                    "<{name} style=\"text-align: {alignment}\">{body}</{name}>"
                )]);
            }
            return Ok(vec![format!("{} {}", "#".repeat(level), body)]);
        }
        if name == "pre" {
            return Ok(vec![self.code_block(&element, depth + 1)?]);
        }
        if name == "blockquote" {
            let inner_nodes: Vec<_> = element.children().collect();
            let inner = self.flow(&inner_nodes, style, depth + 1)?;
            let text = inner.join("\n");
            if text.is_empty() {
                return Ok(vec![">".to_owned()]);
            }
            return Ok(vec![text
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n")]);
        }
        if name == "ul" || name == "ol" {
            return Ok(vec![self.list(
                &element,
                name == "ol",
                style,
                depth + 1,
                0,
            )?]);
        }
        if name == "table" {
            let table = self.table(&element, style, depth + 1)?;
            return Ok(if table.is_empty() {
                Vec::new()
            } else {
                vec![table]
            });
        }
        if name == "hr" {
            return Ok(vec!["---".to_owned()]);
        }
        if name == "img" {
            let image = self.image(&element)?;
            return Ok(image.into_iter().collect());
        }

        // 段落类元素只在内容全是行内时才算块；容器（div/section/article/body…）
        // 可以装下多个块。
        let children: Vec<_> = element.children().collect();
        let has_block_child = children.iter().copied().any(is_block_node);
        // 只含行内子元素的段落若带 `text-align`，输出为对齐的 HTML 行；
        // 容器继续下钻，让每个子块保留自己的边界。
        if has_block_child || (is_flow_container(&name) && name != "p" && name != "div") {
            return self.flow(&children, style, depth + 1);
        }

        let body = self.inline_children(&element, style, depth + 1)?;
        let body = trim_inline_block(&body);
        if body.is_empty() {
            // 空段落（常见 `<p><br></p>` 或 `<div><br></div>`）是有意的空行，
            // 跟标签之间的空白不是一回事。
            return Ok(vec![String::new()]);
        }
        if matches!(name.as_str(), "p") {
            if let Some(alignment) = text_alignment(element.attr("style").unwrap_or_default()) {
                return Ok(vec![format!(
                    "<p style=\"text-align: {alignment}\">{body}</p>"
                )]);
            }
        }
        Ok(vec![body])
    }

    fn inline_children(
        &mut self,
        element: &ElementRef<'_>,
        style: InlineStyle,
        depth: usize,
    ) -> ConvertResult<String> {
        let mut output = String::new();
        for child in element.children() {
            self.visit(child, depth)?;
            let rendered = self.inline(child, style.clone(), depth)?;
            append_limited(&mut output, &rendered)?;
        }
        Ok(output)
    }

    fn inline(
        &mut self,
        node: ego_tree::NodeRef<'_, Node>,
        style: InlineStyle,
        depth: usize,
    ) -> ConvertResult<String> {
        self.visit(node, depth)?;
        match node.value() {
            Node::Text(text) => {
                let text = normalize_inline_whitespace(text);
                if text.is_empty() {
                    return Ok(String::new());
                }
                Ok(decorate(escape_text(&text, false, true), &style))
            }
            Node::Comment(_) | Node::Doctype(_) | Node::ProcessingInstruction(_) => {
                Ok(String::new())
            }
            Node::Document | Node::Fragment => Ok(String::new()),
            Node::Element(_) => {
                let Some(element) = ElementRef::wrap(node) else {
                    return Ok(String::new());
                };
                let name = element.value().name().to_ascii_lowercase();
                if is_ignored_element(&name) {
                    return Ok(String::new());
                }
                if name == "br" {
                    return Ok("\n".to_owned());
                }
                if name == "wbr" {
                    return Ok(String::new());
                }
                if name == "img" {
                    return Ok(self.image(&element)?.unwrap_or_default());
                }
                if name == "a" {
                    let anchor_style = style_for_element(&element, &style);
                    let body = self.inline_children(&element, anchor_style, depth + 1)?;
                    let label = self.plain_children(&element, depth + 1)?;
                    let label = trim_inline_block(&label);
                    if body.trim().is_empty() {
                        return Ok(String::new());
                    }
                    // 原生图片语法没法给图片挂链接。保留图片（和周围文字），
                    // 而不是把整个锚点内容丢掉。
                    if contains_image_syntax(&body) {
                        return Ok(body);
                    }
                    if label.is_empty() {
                        return Ok(body);
                    }
                    let Some(href) = element.attr("href").and_then(safe_link_url) else {
                        return Ok(body);
                    };
                    let linked = linkify_inline_body(&body, &markdown_url(&href));
                    return Ok(linked);
                }
                if name == "input" || name == "textarea" || name == "button" {
                    return Ok(String::new());
                }

                let next = style_for_element(&element, &style);
                let body = self.inline_children(&element, next.clone(), depth + 1)?;
                // 格式错误的剪贴板片段里也可能出现 `<p>` 等块级标签。
                // 保留它们的换行，但不再额外造一个段落。
                Ok(body)
            }
        }
    }

    fn plain_children(&mut self, element: &ElementRef<'_>, depth: usize) -> ConvertResult<String> {
        let mut output = String::new();
        for child in element.children() {
            self.visit(child, depth)?;
            match child.value() {
                Node::Text(text) => {
                    append_limited(&mut output, &normalize_inline_whitespace(text))?
                }
                Node::Element(_) => {
                    let Some(child_element) = ElementRef::wrap(child) else {
                        continue;
                    };
                    let name = child_element.value().name().to_ascii_lowercase();
                    if is_ignored_element(&name) || name == "input" {
                        continue;
                    }
                    if name == "br" {
                        append_limited(&mut output, " ")?;
                    } else {
                        let text = self.plain_children(&child_element, depth + 1)?;
                        append_limited(&mut output, &text)?;
                    }
                }
                _ => {}
            }
        }
        Ok(output)
    }

    fn code_block(&mut self, element: &ElementRef<'_>, depth: usize) -> ConvertResult<String> {
        let mut body = String::new();
        for child in element.children() {
            self.visit(child, depth)?;
            append_limited(&mut body, &raw_pre_text(child, depth, self)?)?;
        }
        let body = body.replace("\r\n", "\n").replace('\r', "\n");
        let language = code_language(element);
        let marker = fence_marker(&body);
        let fence = marker.0.to_string().repeat(marker.1);
        let mut output = format!("{fence}{language}\n{body}");
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&fence);
        Ok(output)
    }

    fn list(
        &mut self,
        element: &ElementRef<'_>,
        ordered: bool,
        style: InlineStyle,
        depth: usize,
        level: usize,
    ) -> ConvertResult<String> {
        let mut lines = Vec::new();
        let mut ordinal = element
            .attr("start")
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(1);

        for child in element.children() {
            self.visit(child, depth)?;
            let Some(item) = ElementRef::wrap(child) else {
                continue;
            };
            if !item.value().name().eq_ignore_ascii_case("li") {
                continue;
            }
            let (body_nodes, nested_lists): (Vec<_>, Vec<_>) =
                item.children().partition(|node| !is_list_node(*node));
            let body_blocks = self.flow(&body_nodes, style.clone(), depth + 1)?;
            let mut body = body_blocks
                .into_iter()
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            body = trim_inline_block(&body);
            let checked = task_checked(&item);
            let marker = if checked == Some(true) {
                "- [x] ".to_owned()
            } else if checked == Some(false) {
                "- [ ] ".to_owned()
            } else if ordered {
                let value = item
                    .attr("value")
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .unwrap_or(ordinal);
                ordinal = value.saturating_add(1);
                format!("{value}. ")
            } else {
                "- ".to_owned()
            };
            let indent = "  ".repeat(level);
            if body.is_empty() {
                lines.push(format!("{indent}{marker}"));
            } else {
                let mut body_lines = body.lines();
                if let Some(first) = body_lines.next() {
                    lines.push(format!("{indent}{marker}{first}"));
                }
                let continuation = "  ".repeat(level + 1);
                lines.extend(body_lines.map(|line| format!("{continuation}{line}")));
            }
            for nested in nested_lists {
                let Some(nested_element) = ElementRef::wrap(nested) else {
                    continue;
                };
                let nested_name = nested_element.value().name().to_ascii_lowercase();
                let nested_text = self.list(
                    &nested_element,
                    nested_name == "ol",
                    style.clone(),
                    depth + 1,
                    level + 1,
                )?;
                if !nested_text.is_empty() {
                    lines.extend(nested_text.lines().map(str::to_owned));
                }
            }
        }
        Ok(lines.join("\n"))
    }

    fn table(
        &mut self,
        element: &ElementRef<'_>,
        style: InlineStyle,
        depth: usize,
    ) -> ConvertResult<String> {
        let mut rows = Vec::new();
        collect_rows(*element, &mut rows, depth, self)?;
        if rows.len() > MAX_TABLE_ROWS {
            return Err(());
        }
        let mut data = Vec::new();
        let mut header = false;
        // `occupancy[col]` 记录之前某行的 rowspan 覆盖的后续行数。
        // 每个被覆盖的位置都物化成空单元格，原生矩形表格解析器才能
        // 保持列对齐，不会把后面的单元格左移。
        let mut occupancy: Vec<usize> = Vec::new();
        for row in rows {
            let mut cells = Vec::new();
            let mut column = 0usize;
            for cell in row.children() {
                let Some(cell) = ElementRef::wrap(cell) else {
                    continue;
                };
                let name = cell.value().name().to_ascii_lowercase();
                if name != "td" && name != "th" {
                    continue;
                }

                while occupancy.get(column).copied().unwrap_or(0) > 0 {
                    cells.push(String::new());
                    occupancy[column] = occupancy[column].saturating_sub(1);
                    column = column.saturating_add(1);
                    if cells.len() > MAX_TABLE_COLUMNS {
                        return Err(());
                    }
                }

                let colspan = table_span(cell.attr("colspan"), MAX_TABLE_COLUMNS);
                let rowspan = table_span(cell.attr("rowspan"), MAX_TABLE_ROWS);
                // 坏表格可能要求一个与早前行 rowspan 冲突的跨度。
                // 先跳过被占用的列、保留占位，再放当前单元格。
                loop {
                    let end = column.saturating_add(colspan);
                    if end > MAX_TABLE_COLUMNS {
                        return Err(());
                    }
                    let occupied =
                        (column..end).find(|index| occupancy.get(*index).copied().unwrap_or(0) > 0);
                    let Some(occupied) = occupied else {
                        break;
                    };
                    while column <= occupied {
                        if occupancy.get(column).copied().unwrap_or(0) > 0 {
                            occupancy[column] = occupancy[column].saturating_sub(1);
                        }
                        cells.push(String::new());
                        column = column.saturating_add(1);
                        if cells.len() > MAX_TABLE_COLUMNS {
                            return Err(());
                        }
                    }
                }

                header |= name == "th";
                let cell_style = style_for_element(&cell, &style);
                let value = self.inline_children(&cell, cell_style, depth + 1)?;
                let value = value
                    .replace('\n', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                cells.push(value);
                for _ in 1..colspan {
                    cells.push(String::new());
                }
                if cells.len() > MAX_TABLE_COLUMNS {
                    return Err(());
                }
                let end = column + colspan;
                if occupancy.len() < end {
                    occupancy.resize(end, 0);
                }
                let following_rows = rowspan.saturating_sub(1);
                for occupied in &mut occupancy[column..end] {
                    *occupied = (*occupied).max(following_rows);
                }
                column = end;
            }
            while occupancy.get(column).copied().unwrap_or(0) > 0 {
                cells.push(String::new());
                occupancy[column] = occupancy[column].saturating_sub(1);
                column = column.saturating_add(1);
                if cells.len() > MAX_TABLE_COLUMNS {
                    return Err(());
                }
            }
            if !cells.is_empty() {
                data.push(cells);
            }
        }
        if data.is_empty() {
            return Ok(String::new());
        }
        let columns = data
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .min(MAX_TABLE_COLUMNS);
        if columns == 0 {
            return Ok(String::new());
        }
        for row in &mut data {
            row.resize(columns, String::new());
        }
        // 原生解析器靠 GFM 分隔行识别表格。没有 <th> 的 HTML 表格
        // 就把首行当视觉表头，这样所有表格都能由同一个原生块渲染。
        let mut output = Vec::new();
        output.push(pipe_row(&data[0]));
        output.push(pipe_row(&vec!["---".to_owned(); columns]));
        output.extend(data.iter().skip(1).map(|row| pipe_row(row)));
        let _ = header; // 标题语义由分隔线表示。
        Ok(output.join("\n"))
    }

    fn image(&mut self, element: &ElementRef<'_>) -> ConvertResult<Option<String>> {
        let Some(src) = element
            .attr("data-mochi-src")
            .or_else(|| element.attr("src"))
            .and_then(safe_image_url)
        else {
            return Ok(None);
        };
        let alt = element.attr("alt").unwrap_or_default();
        let width = element
            .attr("width")
            .and_then(normalize_width)
            .or_else(|| style_width(element.attr("style").unwrap_or_default()));
        let src = markdown_url(&src);
        let alt = escape_image_alt(alt);
        if let Some(width) = width {
            Ok(Some(format!(
                "<img src=\"{}\" alt=\"{}\" width=\"{}\">",
                html_attribute(&src),
                html_attribute(&alt),
                html_attribute(&width)
            )))
        } else {
            Ok(Some(format!("![{alt}]({src})")))
        }
    }
}

fn push_inline_block(blocks: &mut Vec<String>, inline: &mut String) {
    let text = trim_inline_block(inline);
    if !text.is_empty() {
        blocks.push(text);
    }
    inline.clear();
}

fn append_limited(target: &mut String, value: &str) -> ConvertResult<()> {
    if target.len().saturating_add(value.len()) > MAX_OUTPUT_BYTES {
        return Err(());
    }
    target.push_str(value);
    Ok(())
}

fn trim_inline_block(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_owned()
}

fn normalize_inline_whitespace(value: &str) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for ch in value.replace('\u{a0}', " ").chars() {
        if ch == '\n' || ch == '\r' || ch == '\t' || ch == ' ' {
            pending_space = true;
            continue;
        }
        if pending_space {
            output.push(' ');
        }
        pending_space = false;
        output.push(ch);
    }
    // 两个行内元素之间只有空白的文本节点是真正的分隔符
    //（`<span>甲</span> <span>乙</span>`）。这里保留一个空格；
    // 若节点只是段落/容器周围的缩进，块级裁剪会把它去掉。
    if pending_space {
        output.push(' ');
    }
    output
}

fn decorate(value: String, style: &InlineStyle) -> String {
    if value.is_empty() {
        return value;
    }
    if style.code {
        return value
            .split('\n')
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("<code>{}</code>", escape_code_html(line))
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let mut output = value;
    if style.bold {
        output = wrap_lines("strong", &output);
    }
    if style.italic {
        output = wrap_lines("em", &output);
    }
    if style.strike {
        output = wrap_lines("del", &output);
    }
    if style.underline {
        output = wrap_lines("u", &output);
    }
    if style.color.is_some() || style.background.is_some() {
        let mut attributes = Vec::new();
        if let Some(color) = &style.color {
            attributes.push(format!("color: {color}"));
        }
        if let Some(color) = &style.background {
            attributes.push(format!("background-color: {color}"));
        }
        output = wrap_lines(
            &format!("span style=\"{}\"", attributes.join("; ")),
            &output,
        );
    }
    output
}

fn wrap_lines(tag: &str, value: &str) -> String {
    value
        .split('\n')
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!(
                    "<{tag}>{line}</{}>",
                    tag.split_whitespace().next().unwrap_or(tag)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_image_syntax(value: &str) -> bool {
    value.contains("![") || value.to_ascii_lowercase().contains("<img ")
}

/// 在链接文字周围保留生成的行内样式标签。Markdown 链接的显示文字里
/// 不能安全地塞进一个任意生成的 HTML 标签（原生的行内解析器会把
/// 标签原样画出来），所以只给每个文字叶子加链接，样式标签原样不动：
/// `<strong>a <em>b</em></strong>` 变成
/// `<strong>[a](url) <em>[b](url)</em></strong>`。
fn linkify_inline_body(body: &str, url: &str) -> String {
    let mut output = String::with_capacity(body.len().saturating_add(url.len()));
    let mut text_start = 0;
    let mut cursor = 0;
    let mut in_code = false;
    while cursor < body.len() {
        let Some(ch) = body[cursor..].chars().next() else {
            break;
        };
        if ch == '<' {
            if let Some(tag_len) = generated_html_tag_len(body, cursor) {
                append_linkified_text(&mut output, &body[text_start..cursor], url, in_code);
                let tag = &body[cursor..cursor + tag_len];
                output.push_str(tag);
                if tag == "<code>" {
                    in_code = true;
                } else if tag == "</code>" {
                    in_code = false;
                }
                cursor += tag_len;
                text_start = cursor;
                continue;
            }
        }
        cursor += ch.len_utf8();
    }
    append_linkified_text(&mut output, &body[text_start..], url, in_code);
    output
}

fn append_linkified_text(output: &mut String, text: &str, url: &str, in_code: bool) {
    if in_code {
        output.push_str(text);
        return;
    }
    for part in text.split_inclusive('\n') {
        let (line, newline) = part
            .strip_suffix('\n')
            .map_or((part, false), |line| (line, true));
        if line.trim().is_empty() {
            output.push_str(line);
        } else {
            let leading = line.len() - line.trim_start().len();
            let trailing = line.len() - line.trim_end().len();
            output.push_str(&line[..leading]);
            let end = line.len().saturating_sub(trailing);
            output.push('[');
            output.push_str(&line[leading..end]);
            output.push_str("](");
            output.push_str(url);
            output.push(')');
            output.push_str(&line[end..]);
        }
        if newline {
            output.push('\n');
        }
    }
}

fn generated_html_tag_len(value: &str, offset: usize) -> Option<usize> {
    let value = value.get(offset..)?;
    for tag in [
        "<strong>",
        "</strong>",
        "<em>",
        "</em>",
        "<del>",
        "</del>",
        "<u>",
        "</u>",
        "<code>",
        "</code>",
        "</span>",
    ] {
        if value.starts_with(tag) {
            return Some(tag.len());
        }
    }
    if value.starts_with("<span style=\"") {
        return value.find("\">").map(|end| end + 2);
    }
    None
}

fn style_for_element(element: &ElementRef<'_>, parent: &InlineStyle) -> InlineStyle {
    let name = element.value().name().to_ascii_lowercase();
    let mut style = parent.clone();
    match name.as_str() {
        "b" | "strong" => style.bold = true,
        "i" | "em" | "cite" | "var" => style.italic = true,
        "s" | "strike" | "del" => style.strike = true,
        "u" => style.underline = true,
        "code" | "kbd" | "samp" => style.code = true,
        "mark" => style.background = Some("#fff2cc".to_owned()),
        _ => {}
    }

    if let Some(style_text) = element.attr("style") {
        apply_css_style(&mut style, style_text);
    }
    if name == "font" {
        if let Some(color) = element.attr("color").and_then(parse_color) {
            style.color = Some(color);
        }
    }
    if let Some(class) = element.attr("class") {
        for token in class.split_ascii_whitespace() {
            let token = token.to_ascii_lowercase();
            if token.contains("underline") {
                style.underline = true;
            }
            if token.contains("strike") || token.contains("through") {
                style.strike = true;
            }
            if token == "bold" || token == "strong" {
                style.bold = true;
            }
            if token == "italic" || token == "emphasis" {
                style.italic = true;
            }
        }
    }
    style
}

fn apply_css_style(style: &mut InlineStyle, declarations: &str) {
    for declaration in declarations.split(';') {
        let Some((raw_name, raw_value)) = declaration.split_once(':') else {
            continue;
        };
        let name = raw_name.trim().to_ascii_lowercase();
        let value = raw_value.trim().trim_end_matches("!important").trim();
        match name.as_str() {
            "color" | "mso-foreground" => {
                if let Some(color) = parse_color(value) {
                    style.color = Some(color);
                }
            }
            "background-color" | "mso-highlight" => {
                if let Some(color) = parse_color(value) {
                    style.background = Some(color);
                }
            }
            "background" => {
                if let Some(color) = parse_color(value) {
                    style.background = Some(color);
                }
            }
            "font-weight" | "mso-bidi-font-weight" => {
                if value.eq_ignore_ascii_case("bold")
                    || value.eq_ignore_ascii_case("bolder")
                    || value
                        .parse::<u16>()
                        .ok()
                        .is_some_and(|weight| weight >= 600)
                {
                    style.bold = true;
                }
            }
            "font-style" => {
                if value.eq_ignore_ascii_case("italic") || value.eq_ignore_ascii_case("oblique") {
                    style.italic = true;
                }
            }
            "text-decoration" | "text-decoration-line" => {
                let value = value.to_ascii_lowercase();
                style.underline |= value.contains("underline");
                style.strike |= value.contains("line-through");
            }
            _ => {}
        }
    }
}

fn text_alignment(declarations: &str) -> Option<&'static str> {
    declarations.split(';').find_map(|declaration| {
        let (raw_name, raw_value) = declaration.split_once(':')?;
        if !raw_name.trim().eq_ignore_ascii_case("text-align") {
            return None;
        }
        let value = raw_value.trim().to_ascii_lowercase();
        let value = value.strip_suffix("!important").unwrap_or(&value).trim();
        match value {
            "left" => Some("left"),
            "center" => Some("center"),
            "right" => Some("right"),
            _ => None,
        }
    })
}

fn parse_color(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    if value.is_empty() || value == "transparent" || value.contains("url(") {
        return None;
    }
    if let Some(hex) = value.strip_prefix('#') {
        // 切分字节前先校验完整的词法单元。CSS 输入不可信，一个非 ASCII
        // 字符就能让 `hex[..3]` panic，哪怕它的 UTF-8 字节长度碰巧看着合法。
        if !hex.is_ascii() || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let hex = match hex.len() {
            3 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
            4 => hex[..3].chars().flat_map(|ch| [ch, ch]).collect::<String>(),
            6 | 8 => hex[..6].to_owned(),
            _ => return None,
        };
        if hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Some(format!("#{hex}"));
        }
        return None;
    }
    if let Some(body) = value
        .strip_prefix("rgb(")
        .and_then(|v| v.strip_suffix(')'))
        .or_else(|| {
            value
                .strip_prefix("rgba(")
                .and_then(|v| v.strip_suffix(')'))
        })
    {
        let components = body
            .replace([',', '/'], " ")
            .split_whitespace()
            .take(3)
            .map(parse_color_component)
            .collect::<Option<Vec<_>>>()?;
        if components.len() == 3 {
            return Some(format!(
                "#{:02x}{:02x}{:02x}",
                components[0], components[1], components[2]
            ));
        }
    }
    let named = match value.as_str() {
        "black" => 0x000000,
        "silver" => 0xc0c0c0,
        "gray" | "grey" => 0x808080,
        "white" => 0xffffff,
        "maroon" => 0x800000,
        "red" => 0xff0000,
        "purple" => 0x800080,
        "fuchsia" | "magenta" => 0xff00ff,
        "green" => 0x008000,
        "lime" => 0x00ff00,
        "olive" => 0x808000,
        "yellow" => 0xffff00,
        "navy" => 0x000080,
        "blue" => 0x0000ff,
        "teal" => 0x008080,
        "aqua" | "cyan" => 0x00ffff,
        "orange" => 0xffa500,
        "windowtext" => 0x000000,
        _ => return None,
    };
    Some(format!("#{named:06x}"))
}

fn parse_color_component(value: &str) -> Option<u8> {
    if let Some(percent) = value.strip_suffix('%') {
        let number = percent.trim().parse::<f32>().ok()?;
        if !number.is_finite() || !(0.0..=100.0).contains(&number) {
            return None;
        }
        return Some((number * 2.55).round() as u8);
    }
    let number = value.trim().parse::<f32>().ok()?;
    if !number.is_finite() || !(0.0..=255.0).contains(&number) {
        return None;
    }
    Some(number.round() as u8)
}

fn escape_text(value: &str, table: bool, protect_line_prefix: bool) -> String {
    let mut output = String::with_capacity(value.len());
    let mut line_start = true;
    let mut ordered_prefix = false;
    let chars: Vec<char> = value.chars().collect();
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch == '\n' {
            output.push('\n');
            line_start = true;
            ordered_prefix = false;
            continue;
        }
        let next = chars.get(index + 1).copied();
        if line_start && ch.is_ascii_digit() && ordered_marker_from(&chars, index) {
            ordered_prefix = true;
        }
        let must_escape = matches!(
            ch,
            '\\' | '*' | '_' | '~' | '[' | ']' | '`' | '<' | '>' | '$' | '|'
        ) || (table && ch == '|')
            || (line_start && protect_line_prefix && matches!(ch, '#' | '-' | '+' | '>'))
            || (line_start && protect_line_prefix && ordered_prefix && matches!(ch, '.' | ')'))
            || (line_start && protect_line_prefix && starts_with_chars(&chars, index, ":::"))
            || (line_start
                && protect_line_prefix
                && ch == '!'
                && next.is_some_and(|next| next == '['));
        if must_escape {
            output.push('\\');
        }
        output.push(ch);
        if line_start && ch.is_whitespace() {
            // 前导缩进本身就是剪贴板文本的一部分。行前缀状态要穿过它，
            // `  # 标题`、`  - 条目`、`  12. 条目` 的语法标点照样转义。
            continue;
        }
        if ordered_prefix && ch.is_ascii_digit() {
            line_start = true;
        } else {
            line_start = false;
            ordered_prefix = false;
        }
    }
    output
}

fn ordered_marker_from(chars: &[char], index: usize) -> bool {
    let mut end = index;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }
    let mut line_prefix = index;
    while line_prefix > 0 && chars[line_prefix - 1] != '\n' {
        if !chars[line_prefix - 1].is_whitespace() {
            return false;
        }
        line_prefix -= 1;
    }
    end > index
        && chars.get(end).is_some_and(|ch| *ch == '.' || *ch == ')')
        && (line_prefix == 0 || chars.get(line_prefix - 1) == Some(&'\n'))
}

fn starts_with_chars(chars: &[char], index: usize, marker: &str) -> bool {
    marker
        .chars()
        .enumerate()
        .all(|(offset, expected)| chars.get(index + offset) == Some(&expected))
}

fn escape_code_html(value: &str) -> String {
    value.replace("</code", "<\\/code")
}

fn escape_image_alt(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(']', "\\]")
        .replace('[', "\\[")
}

fn html_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn safe_link_url(value: &str) -> Option<String> {
    safe_url(value, false)
}

fn safe_image_url(value: &str) -> Option<String> {
    safe_url(value, true)
}

fn safe_url(value: &str, image: bool) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    let scheme = value
        .split_once(':')
        .filter(|(prefix, _)| {
            !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_alphabetic())
        })
        .map(|(prefix, _)| prefix.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http") | Some("https") => {
            let (prefix, rest) = value.split_once(':')?;
            Some(format!("{}:{rest}", prefix.to_ascii_lowercase()))
        }
        Some("mochi") => Some(value.to_owned()),
        Some("mailto") if !image => Some(value.to_owned()),
        Some("ftp") if !image => Some(value.to_owned()),
        Some(_) => None,
        None => Some(value.to_owned()),
    }
}

fn markdown_url(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            ' ' => "%20".chars().collect::<Vec<_>>(),
            '(' => "%28".chars().collect::<Vec<_>>(),
            ')' => "%29".chars().collect::<Vec<_>>(),
            '<' => "%3C".chars().collect::<Vec<_>>(),
            '>' => "%3E".chars().collect::<Vec<_>>(),
            '|' => "%7C".chars().collect::<Vec<_>>(),
            _ => vec![ch],
        })
        .collect()
}

fn normalize_width(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 16 {
        return None;
    }
    if let Some(percent) = value.strip_suffix('%') {
        let number = percent.parse::<f32>().ok()?;
        return number.is_finite().then(|| format!("{number}%"));
    }
    let number = value
        .strip_suffix("px")
        .unwrap_or(value)
        .parse::<f32>()
        .ok()?;
    if number.is_finite() && number > 0.0 {
        Some(number.round().to_string())
    } else {
        None
    }
}

fn style_width(style: &str) -> Option<String> {
    style.split(';').find_map(|declaration| {
        let (name, value) = declaration.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("width")
            .then(|| normalize_width(value.trim()))
            .flatten()
    })
}

fn code_language(element: &ElementRef<'_>) -> String {
    let code = if element.value().name().eq_ignore_ascii_case("code") {
        *element
    } else {
        element
            .child_elements()
            .find(|child| child.value().name().eq_ignore_ascii_case("code"))
            .unwrap_or(*element)
    };
    let class = code.attr("class").unwrap_or_default();
    let mut language = code
        .attr("data-language")
        .or_else(|| code.attr("data-lang"))
        .or_else(|| element.attr("data-language"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    if language.is_empty() {
        for token in class.split_ascii_whitespace() {
            if let Some(value) = token
                .strip_prefix("language-")
                .or_else(|| token.strip_prefix("lang-"))
            {
                language = value.to_owned();
                break;
            }
        }
    }
    language
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-' | '#'))
        .take(32)
        .collect()
}

fn fence_marker(body: &str) -> (char, usize) {
    let backticks = longest_run(body, '`');
    let tildes = longest_run(body, '~');
    let (marker, run) = if backticks <= tildes {
        ('`', backticks)
    } else {
        ('~', tildes)
    };
    (marker, (run + 1).max(3))
}

fn longest_run(value: &str, needle: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in value.chars() {
        if ch == needle {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn raw_pre_text(
    node: ego_tree::NodeRef<'_, Node>,
    depth: usize,
    converter: &mut Converter,
) -> ConvertResult<String> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    match node.value() {
        Node::Text(text) => Ok(text.text.to_string()),
        Node::Element(_) => {
            let Some(element) = ElementRef::wrap(node) else {
                return Ok(String::new());
            };
            let name = element.value().name().to_ascii_lowercase();
            if is_ignored_element(&name) {
                return Ok(String::new());
            }
            if name == "br" {
                return Ok("\n".to_owned());
            }
            let mut output = String::new();
            for child in element.children() {
                converter.visit(child, depth + 1)?;
                append_limited(&mut output, &raw_pre_text(child, depth + 1, converter)?)?;
            }
            Ok(output)
        }
        _ => Ok(String::new()),
    }
}

fn collect_rows<'a>(
    element: ElementRef<'a>,
    rows: &mut Vec<ElementRef<'a>>,
    depth: usize,
    converter: &mut Converter,
) -> ConvertResult<()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    for child in element.children() {
        converter.visit(child, depth)?;
        let Some(child_element) = ElementRef::wrap(child) else {
            continue;
        };
        let name = child_element.value().name().to_ascii_lowercase();
        if name == "tr" {
            rows.push(child_element);
            if rows.len() > MAX_TABLE_ROWS {
                return Err(());
            }
        } else if name != "table" {
            collect_rows(child_element, rows, depth + 1, converter)?;
        }
    }
    Ok(())
}

fn table_span(value: Option<&str>, limit: usize) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|span| *span > 0)
        .map(|span| span.min(limit))
        .unwrap_or(1)
}

fn pipe_row(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

fn heading_level(name: &str) -> Option<usize> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn is_ignored_element(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "meta" | "link" | "noscript" | "template" | "head" | "title" | "base"
    )
}

fn is_flow_container(name: &str) -> bool {
    matches!(
        name,
        "html"
            | "body"
            | "p"
            | "div"
            | "section"
            | "article"
            | "aside"
            | "header"
            | "footer"
            | "main"
            | "nav"
            | "form"
            | "fieldset"
            | "figure"
            | "details"
            | "summary"
            | "dl"
            | "dt"
            | "dd"
            | "caption"
    )
}

fn is_block_node(node: ego_tree::NodeRef<'_, Node>) -> bool {
    let Some(element) = ElementRef::wrap(node) else {
        return false;
    };
    let name = element.value().name().to_ascii_lowercase();
    heading_level(&name).is_some()
        || is_flow_container(&name)
        || matches!(
            name.as_str(),
            "pre" | "blockquote" | "ul" | "ol" | "table" | "hr" | "img"
        )
}

fn is_list_node(node: ego_tree::NodeRef<'_, Node>) -> bool {
    ElementRef::wrap(node).is_some_and(|element| {
        matches!(
            element.value().name().to_ascii_lowercase().as_str(),
            "ul" | "ol"
        )
    })
}

fn task_checked(element: &ElementRef<'_>) -> Option<bool> {
    let checked_attr = element
        .attr("data-checked")
        .or_else(|| element.attr("checked"));
    if let Some(value) = checked_attr {
        return Some(value.is_empty() || value.eq_ignore_ascii_case("true") || value == "checked");
    }
    for child in element.descendent_elements() {
        if child.value().name().eq_ignore_ascii_case("input")
            && child
                .attr("type")
                .is_some_and(|value| value.eq_ignore_ascii_case("checkbox"))
        {
            return Some(child.attr("checked").is_some());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjacent_html_paragraphs_do_not_create_blank_lines() {
        let lines = [
            "事实上，市面上的笔记软件数不胜数，无论是开源还是闭源，每款软件都有自己的设计理念和侧重点，也形成了各具特色的使用体验。",
            "在不断尝试的过程中，我逐渐萌生了一个想法：如果能够融合不同产品中的优秀设计与操作方式，或许可以构建出一个更符合自己期待的个人工作台。",
            "以及，由此衍生出的各类其他的问题：数据是否真正地可控？Agent 是否能够深度理解笔记软件本身并且操作内容？用户能否按照自己的习惯自由配置和使用？",
            "因而，基于不同笔记软件的设计理念与操作特色，也融入了自己对个人工作台的期待，借助AI，我创作了【墨池】。",
            "墨池不仅是一个记录内容的工具，也期望能成为一个更加开放、智能、自由的个人工作空间；让知识真正属于用户，也让 Agent 更自然地参与到学习与工作之中。",
        ];
        for tag in ["p", "div"] {
            let html = lines
                .iter()
                .map(|line| format!("<{tag}>{line}</{tag}>"))
                .collect::<Vec<_>>()
                .join("\r\n");
            let source = from_html(&format!("<section>{html}</section>")).unwrap();
            assert_eq!(source, lines.join("\n"));
            let parsed = crate::ui::document::parse_ranged(&source);
            assert_eq!(parsed.blocks.len(), lines.len());
            assert!(parsed
                .blocks
                .iter()
                .all(|block| matches!(block.block, crate::ui::document::Block::Paragraph(_))));
        }
    }

    #[test]
    fn html_paste_keeps_explicit_blank_lines() {
        for html in [
            "<p>第一行</p><p><br></p><p>第二行</p><p></p><p></p><p>第三行</p>",
            "<div>第一行</div><div><br></div><div>第二行</div><div></div><div></div><div>第三行</div>",
            "<p>第一行<br><br>第二行<br><br><br>第三行</p>",
        ] {
            assert_eq!(from_html(html).unwrap(), "第一行\n\n第二行\n\n\n第三行");
        }
        assert_eq!(
            from_html("<blockquote><p>甲</p><p>乙</p></blockquote>").unwrap(),
            "> 甲\n> 乙"
        );
    }

    #[test]
    fn nested_styles_and_entities_are_converted() {
        let source = from_html(
            r#"<p><strong>粗 <em>斜</em></strong> <del>删</del> <u>下</u> <mark>标记</mark> &amp; 中文😀</p>"#,
        )
        .unwrap();
        assert!(source.contains("<strong>"));
        assert!(source.contains("<em>"));
        assert!(source.contains("<del>删</del>"));
        assert!(source.contains("<u>下</u>"));
        assert!(source.contains("#fff2cc"));
        assert!(source.contains("& 中文😀"));
    }

    #[test]
    fn inline_boundary_spaces_and_block_alignment_are_preserved() {
        let source = from_html(
            r#"<p>甲 <strong>乙</strong> 丙</p><p style="text-align:center">中间</p><h2 style="text-align:right">右侧</h2>"#,
        )
        .unwrap();
        assert_eq!(visible_inline(source.lines().next().unwrap()), "甲 乙 丙");
        assert!(source.contains("<p style=\"text-align: center\">中间</p>"));
        assert!(source.contains("<h2 style=\"text-align: right\">右侧</h2>"));
    }

    #[test]
    fn links_keep_nested_styles_and_do_not_drop_wrapped_images() {
        let source = from_html(
            r#"<p><a href="HTTPS://example.test"><strong>粗 <em>斜</em></strong></a> <a href="https://image.test"><img src="https://image.test/a.png" alt="图"></a></p>"#,
        )
        .unwrap();
        assert!(source.contains("<strong>[粗](https://example.test)"));
        assert!(crate::ui::text::parse_inline(&source)
            .iter()
            .any(|run| run.text == "斜"
                && run.emphasis.base() == crate::ui::text::Emphasis::BoldItalic));
        assert_eq!(
            crate::ui::text::scan_links(&source)
                .iter()
                .filter(|link| link.target == "https://example.test")
                .count(),
            2
        );
        assert!(source.contains("![图](https://image.test/a.png)"));
    }

    #[test]
    fn external_table_is_emitted_as_native_pipe_table() {
        let source = from_html(
            r#"<table><thead><tr><th>名称</th><th>值</th></tr></thead><tbody><tr><td>A</td><td>1 &amp; 2</td></tr></tbody></table>"#,
        )
        .unwrap();
        assert_eq!(source, "| 名称 | 值 |\n| --- | --- |\n| A | 1 & 2 |");
    }

    #[test]
    fn html_wrappers_do_not_change_list_nesting() {
        let source = from_html(
            "<div><section><ul><li>一<ul><li>子项</li></ul></li><li>二</li></ul></section></div>",
        )
        .unwrap();
        assert_eq!(source, "- 一\n  - 子项\n- 二");
    }

    #[test]
    fn html_table_spans_expand_into_empty_native_cells() {
        let source = from_html(
            r#"<table><tr><th colspan="2">合并</th><th>尾</th></tr><tr><td rowspan="2">纵向</td><td>B</td><td>C</td></tr><tr><td>D</td><td>E</td></tr></table>"#,
        )
        .unwrap();
        assert_eq!(
            source,
            "| 合并 |  | 尾 |\n| --- | --- | --- |\n| 纵向 | B | C |\n|  | D | E |"
        );
    }

    #[test]
    fn code_block_preserves_indentation_and_language() {
        let source = from_html(
            r#"<pre><code class="language-rust">fn main() {
    println!("hi");
}</code></pre>"#,
        )
        .unwrap();
        assert!(source.starts_with("```rust\nfn main() {\n    println!"));
        assert!(source.ends_with("\n}\n```"));
    }

    #[test]
    fn word_styles_and_task_lists_survive() {
        let source = from_html(
            r#"<ul><li><input type="checkbox" checked> <span style="font-weight:700;color:#abc;text-decoration: underline">完成</span></li><li><span style="mso-highlight:yellow">待办</span></li></ul>"#,
        )
        .unwrap();
        assert!(source.contains("- [x]"));
        assert!(source.contains("font-weight") || source.contains("<strong>"));
        assert!(source.contains("#aabbcc"));
        assert!(source.contains("#ffff00"));
    }

    #[test]
    fn malicious_elements_and_urls_are_removed() {
        let source = from_html(
            r#"<script>alert(1)</script><p>ok <a href="javascript:alert(2)">bad</a> <a href="https://safe.example">good</a></p><img src="data:text/html,evil" alt="x">"#,
        )
        .unwrap();
        assert!(!source.contains("alert"));
        assert!(source.contains("bad"));
        assert!(source.contains("[good](https://safe.example)"));
        assert!(!source.contains("data:text"));
    }

    #[test]
    fn plain_text_escapes_fake_markdown() {
        let source =
            plain_text_source("# title\n- item\n1. ordered\n**bold** [link](javascript:x) `code`");
        assert_eq!(
            source,
            "\\# title\n\\- item\n1\\. ordered\n\\*\\*bold\\*\\* \\[link\\](javascript:x) \\`code\\`"
        );
    }

    fn visible_inline(source: &str) -> String {
        crate::ui::text::parse_inline(source)
            .into_iter()
            .map(|run| run.text)
            .collect()
    }

    #[test]
    fn plain_text_is_visible_literal_for_math_tables_and_html_like_text() {
        let source = plain_text_source("$x$\n| 表格 |\n|---|\n<b>标签</b>");
        let lines: Vec<_> = source.lines().map(visible_inline).collect();
        assert_eq!(lines, vec!["$x$", "| 表格 |", "|---|", "<b>标签</b>"]);
        let parsed = crate::ui::document::parse_ranged(&source);
        assert!(parsed
            .blocks
            .iter()
            .all(|block| !matches!(block.block, crate::ui::document::Block::Table { .. })));
        assert!(parsed
            .blocks
            .iter()
            .all(|block| !matches!(block.block, crate::ui::document::Block::Math(_))));
    }

    #[test]
    fn plain_text_keeps_leading_prefixes_and_multidigit_ordered_markers_literal() {
        let source = plain_text_source("  # 标题\n  - 项目\n  12. 双位\n  123) 三位");
        let visible: Vec<_> = source.lines().map(visible_inline).collect();
        assert_eq!(
            visible,
            vec!["  # 标题", "  - 项目", "  12. 双位", "  123) 三位"]
        );
        let parsed = crate::ui::document::parse_ranged(&source);
        assert!(parsed.blocks.iter().all(|block| {
            !matches!(
                block.block,
                crate::ui::document::Block::Heading { .. }
                    | crate::ui::document::Block::ListItem { .. }
            )
        }));
    }

    #[test]
    fn converted_function_blocks_remain_native_code_blocks() {
        let source = from_html(
            r#"<pre><code class="language-rust"># not a heading
- not a list
    keep spaces</code></pre>"#,
        )
        .unwrap();
        let parsed = crate::ui::document::parse_ranged(&source);
        assert!(matches!(
            parsed.blocks.first().map(|block| &block.block),
            Some(crate::ui::document::Block::Code { lang, lines })
                if lang == "rust"
                    && lines.as_slice()
                        == [
                            "# not a heading".to_owned(),
                            "- not a list".to_owned(),
                            "    keep spaces".to_owned(),
                        ]
                        .as_slice()
        ));
    }

    #[test]
    fn over_limit_input_is_rejected() {
        let input = "x".repeat(MAX_INPUT_BYTES + 1);
        assert!(from_html(&input).is_none());
    }

    #[test]
    fn executable_urls_are_case_insensitive() {
        assert!(safe_link_url("JaVaScRiPt:alert(1)").is_none());
        assert_eq!(
            safe_link_url("HTTPS://example.test").as_deref(),
            Some("https://example.test")
        );
        assert!(parse_color("#éé").is_none());
    }

    #[test]
    fn plain_text_container_markers_stay_literal_in_the_native_parser() {
        let source = plain_text_source(":::mochi-highlight title=\"x\"\n:::mochi-details");
        let visible: Vec<_> = source.lines().map(visible_inline).collect();
        assert_eq!(
            visible,
            vec![":::mochi-highlight title=\"x\"", ":::mochi-details"]
        );
        let parsed = crate::ui::document::parse_ranged(&source);
        assert!(parsed
            .blocks
            .iter()
            .all(|block| !matches!(block.block, crate::ui::document::Block::Container(_))));
    }
}
