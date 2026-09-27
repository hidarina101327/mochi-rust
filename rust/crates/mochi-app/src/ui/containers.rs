//! Electron 版持久化的 highlight/details 块，映射到普通的富文本子块上。
//! 分隔符保留在源码里，读取时从不重新序列化。
use super::{
    document::{Block, RangedBlock},
    editor::TextBuffer,
};
use std::ops::Range;

pub const HEADER_HEIGHT: f32 = 34.0;
pub const INDENT: f32 = 16.0;
pub const COLORS: &[(&str, &str, u32, u32)] = &[
    ("gray", "灰色", 0xf3f4f6, 0xd1d5db),
    ("yellow", "黄色", 0xfff7d6, 0xf4d35e),
    ("green", "绿色", 0xeaf7ed, 0x95d5a6),
    ("blue", "蓝色", 0xeaf3ff, 0x93c5fd),
    ("red", "红色", 0xfff0f0, 0xfca5a5),
    ("purple", "紫色", 0xf4efff, 0xc4b5fd),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Highlight,
    Details,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Title,
    Color(String),
    Unwrap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub kind: Kind,
    pub title: String,
    pub color: String,
    pub open: bool,
    pub header: Range<usize>,
    pub body: Range<usize>,
    pub footer: Range<usize>,
}

pub fn attribute(source: &str, name: &str) -> Option<String> {
    static ATTRS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let regex = ATTRS.get_or_init(|| {
        regex::Regex::new(r#"([\w-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap()
    });
    regex
        .captures_iter(source)
        .find(|c| &c[1] == name)
        .map(|c| {
            let value = c.get(2).or(c.get(3)).or(c.get(4)).unwrap().as_str();
            unescape(value)
        })
}

pub fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

impl Container {
    pub fn colors(&self) -> (u32, u32) {
        COLORS
            .iter()
            .find(|(name, ..)| *name == self.color)
            .map(|(_, _, bg, border)| (*bg, *border))
            .or_else(|| super::styles::color(&self.color).map(|c| (c, c)))
            .unwrap_or((COLORS[0].2, COLORS[0].3))
    }
    fn opening(&self, newline: &str) -> String {
        match self.kind {
            Kind::Highlight => format!(
                ":::mochi-highlight color=\"{}\" title=\"{}\"{newline}",
                escape(&self.color),
                escape(&self.title)
            ),
            Kind::Details => format!(
                "<details{}>{newline}<summary>{}</summary>{newline}",
                if self.open { " open" } else { "" },
                escape(&self.title)
            ),
        }
    }
}

pub fn scan(source: &str) -> Vec<Container> {
    let mut out = Vec::new();
    let mut stack: Vec<(Container, bool)> = Vec::new();
    let mut offset = 0;
    let mut fence: Option<(char, usize)> = None;
    for raw in source.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let line = raw.trim();
        if let Some((marker, count)) = fence {
            let n = line.chars().take_while(|c| *c == marker).count();
            if n >= count && line[n..].trim().is_empty() {
                fence = None;
            }
            continue;
        }
        if let Some((marker, count, _)) = super::document::fence_open(line) {
            fence = Some((marker, count));
            continue;
        }
        let kind = if line.starts_with(":::mochi-highlight")
            && line[18..].chars().next().is_none_or(char::is_whitespace)
        {
            Some(Kind::Highlight)
        } else if line == "<details>" || (line.starts_with("<details ") && line.ends_with('>')) {
            Some(Kind::Details)
        } else {
            None
        };
        if let Some(kind) = kind {
            stack.push((
                Container {
                    kind,
                    title: attribute(line, "title").unwrap_or_else(|| {
                        if kind == Kind::Details {
                            "Details"
                        } else {
                            "Tips"
                        }
                        .into()
                    }),
                    color: attribute(line, "color").unwrap_or_else(|| "gray".into()),
                    open: kind == Kind::Highlight
                        || line
                            .trim_end_matches('>')
                            .split_whitespace()
                            .skip(1)
                            .any(|v| v == "open" || v.starts_with("open=")),
                    header: start..offset,
                    body: offset..offset,
                    footer: offset..offset,
                },
                false,
            ));
            continue;
        }
        let Some((panel, wrapped)) = stack.last_mut() else {
            continue;
        };
        if panel.kind == Kind::Details && panel.body.start == start {
            if let Some(summary) = line
                .strip_prefix("<summary>")
                .and_then(|s| s.strip_suffix("</summary>"))
            {
                panel.title = unescape(summary);
                panel.header.end = offset;
                panel.body.start = offset;
                continue;
            }
            if line.starts_with("<div ")
                && attribute(line, "data-type").as_deref() == Some("details-content")
            {
                panel.header.end = offset;
                panel.body.start = offset;
                *wrapped = true;
                continue;
            }
        }
        let closes = (panel.kind == Kind::Highlight && line == ":::")
            || (panel.kind == Kind::Details && line == "</details>");
        if closes {
            let (mut panel, wrapped) = stack.pop().unwrap();
            let body_end = if wrapped {
                let body = &source[panel.body.start..start];
                body.trim_end()
                    .strip_suffix("</div>")
                    .map(|s| panel.body.start + s.len())
                    .unwrap_or(start)
            } else {
                start
            };
            panel.body.end = body_end;
            panel.footer = body_end..(start + raw.trim_end_matches(['\r', '\n']).len());
            out.push(panel);
        }
    }
    out.sort_by_key(|p| p.header.start);
    out
}

pub fn project(source: &str, blocks: &mut Vec<RangedBlock>) {
    let body_start = blocks.first().map(|b| b.start).unwrap_or(source.len());
    let panels = scan(source)
        .into_iter()
        .filter(|p| p.header.start >= body_start)
        .collect::<Vec<_>>();
    if panels.is_empty() {
        return;
    }
    blocks.retain(|b| {
        !panels.iter().any(|p| {
            (p.header.start <= b.start && b.start < p.header.end)
                || (p.footer.start <= b.start && b.end <= p.footer.end)
        })
    });
    for panel in panels {
        let end = source[..panel.header.end]
            .trim_end_matches(['\r', '\n'])
            .len();
        blocks.push(RangedBlock {
            start: panel.header.start,
            end,
            block: Block::Container(panel.clone()),
        });
        // 空容器在结束符前也保留一个可编辑的空段落。
        if panel.body.is_empty() {
            blocks.push(RangedBlock {
                start: panel.body.start,
                end: panel.body.start,
                block: Block::Blank,
            });
        }
    }
    blocks.sort_by_key(|b| b.start);
}

pub fn insert(buffer: &mut TextBuffer, kind: Kind) {
    let newline = if buffer.text().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let (a, b) = buffer.selection();
    let (start, _) = super::format::line_range(buffer.text(), a);
    let (_, mut end) = super::format::line_range(buffer.text(), b);
    if buffer.text().as_bytes().get(end.wrapping_sub(1)) == Some(&b'\r') {
        end -= 1;
    }
    let panel = Container {
        kind,
        title: if kind == Kind::Details {
            "Details"
        } else {
            "Tips"
        }
        .into(),
        color: "gray".into(),
        open: true,
        header: 0..0,
        body: 0..0,
        footer: 0..0,
    };
    let opening = panel.opening(newline);
    let closing = if kind == Kind::Details {
        "</details>"
    } else {
        ":::"
    };
    let body = buffer.text()[start..end].to_owned();
    let content = format!("{opening}{body}{newline}{closing}");
    let cursor = opening.len() + a - start;
    buffer.replace_range_select(start..end, &content, cursor..cursor);
}

pub fn update(buffer: &mut TextBuffer, panel: &Container) {
    let newline = if buffer.text().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut opening = panel.opening(newline);
    // 保留旧版 Electron 笔记生成的可选 HTML body 包装层。
    if buffer.text()[panel.header.clone()].contains("details-content") {
        opening.push_str(&format!("<div data-type=\"details-content\">{newline}"));
    }
    buffer.replace_range_select(panel.header.clone(), &opening, 0..0);
}

pub fn unwrap(buffer: &mut TextBuffer, panel: &Container) {
    let body = buffer.text()[panel.body.clone()]
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    buffer.replace_range_select(panel.header.start..panel.footer.end, &body, 0..0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn electron_highlights_and_nested_details_keep_exact_child_ranges() {
        let source = ":::mochi-highlight color=\"blue\" title=\"A &amp; B\"\r\n**正文**\r\n<details open>\r\n<summary>标题</summary>\r\n内容\r\n</details>\r\n:::";
        let parsed = super::super::document::parse_ranged(source);
        let panels = scan(source);
        assert_eq!(panels.len(), 2);
        assert_eq!(panels[0].title, "A & B");
        assert!(panels[1].open);
        for value in ["**正文**", "内容"] {
            let block = parsed
                .blocks
                .iter()
                .find(|b| &source[b.start..b.end] == value)
                .unwrap();
            assert!(matches!(block.block, Block::Paragraph(_)));
        }
        assert!(!parsed.blocks.iter().any(
            |b| matches!(&b.block, Block::Paragraph(s) if s.contains("summary") || s == ":::")
        ));
    }
    #[test]
    fn fences_and_unclosed_containers_remain_literal() {
        for source in [
            "```text\n:::mochi-highlight\nx\n:::\n```",
            ":::mochi-highlight title=\"unfinished\"\nx",
            "<details>\n<summary>x</summary>",
        ] {
            assert!(scan(source).is_empty());
        }
    }
    #[test]
    fn insert_edit_unwrap_and_undo_preserve_unicode_and_crlf() {
        for kind in [Kind::Highlight, Kind::Details] {
            let raw = "前文\r\n**正文😀**\r\n后文";
            let mut buffer = TextBuffer::new(raw);
            buffer.set_cursor(raw.find("正文").unwrap(), false);
            insert(&mut buffer, kind);
            let inserted = buffer.text().to_owned();
            let mut panel = scan(buffer.text()).remove(0);
            panel.title = "新的 <标题> & \"名称\"".into();
            update(&mut buffer, &panel);
            assert_eq!(scan(buffer.text())[0].title, panel.title);
            assert!(buffer.text().contains("**正文😀**\r\n"));
            buffer.undo();
            assert_eq!(buffer.text(), inserted);
            let panel = scan(buffer.text()).remove(0);
            unwrap(&mut buffer, &panel);
            assert_eq!(buffer.text(), raw);
            buffer.undo();
            assert_eq!(buffer.text(), inserted);
            buffer.undo();
            assert_eq!(buffer.text(), raw);
        }
    }
}
