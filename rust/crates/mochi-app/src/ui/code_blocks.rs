//! 代码卡片的属性沿用 Electron 版 mochi-code-block 注释，标题和折叠状态
//! 因此能在保存、重新打开和切换版本后保留下来。
use super::{
    document::{self, Decoration, Layout, Parsed},
    layout::Rect,
    text::Run,
};
use std::collections::HashMap;

pub const LANGUAGES: &[&str] = &[
    "plaintext",
    "javascript",
    "typescript",
    "python",
    "java",
    "go",
    "rust",
    "html",
    "css",
    "markdown",
    "json",
    "yaml",
    "sql",
    "bash",
    "shell",
    "c",
    "cpp",
    "csharp",
    "php",
    "ruby",
    "swift",
];
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct Attrs {
    pub collapsed: bool,
    pub title: String,
}
#[derive(Default)]
pub struct State {
    pub blocks: HashMap<usize, Attrs>,
}
impl State {
    /// 源码变化时根据持久化的属性重建展示状态。
    pub fn reconcile(&mut self, next: &str) {
        let parsed = document::parse_ranged(next);
        self.reconcile_parsed(next, &parsed);
    }

    /// 与上一函数相同的对账逻辑，但复用调用方已有的解析结果。编辑路径
    /// 本来就要为了光标/图片/布局等工作检查块范围，直接接收 `Parsed`
    /// 就不必在这里重新解析整篇文档。
    pub fn reconcile_parsed(&mut self, next: &str, parsed: &Parsed) {
        // 只在调用方重建布局时执行。属性本就持久化在 Markdown 里；如果
        // 仅为省掉这一小段提取就再保留一份完整源码副本，访问过的每篇
        // 笔记都得白白多占内存。`metadata` 的 API 刻意保持简单（供编辑
        // 路径使用），代价是每次调用都要从头搜索源码。这里只扫一遍源码，
        // 记住最近一个非空行的边界，这样即使文档里有很多代码卡片，整体
        // 仍是 O(源码 + 块数)。非空边界很关键：metadata() 在查找注释行
        // 之前会把所有 CR/LF（包括连续多个空行）都裁掉。
        let bytes = next.as_bytes();
        let mut cursor = 0;
        let mut line_start = 0;
        let mut last_content_start = 0;
        let mut last_content_end = 0;
        let mut blocks = HashMap::new();
        for block in parsed
            .blocks
            .iter()
            .filter(|b| matches!(b.block, document::Block::Code { .. }))
        {
            let start = block.start.min(next.len());
            while cursor < start {
                if bytes[cursor] == b'\n' {
                    line_start = cursor + 1;
                } else if bytes[cursor] != b'\r' {
                    last_content_start = line_start;
                    last_content_end = cursor + 1;
                }
                cursor += 1;
            }

            // 围栏从它所在行的行首开始。若其前面紧挨着的源码以换行结尾，
            // metadata() 会在裁掉该换行（以及其前的若干空行）之后，检查
            // 最后一个有内容的行。
            let (candidate_start, candidate_end) =
                if start > 0 && matches!(bytes[start - 1], b'\r' | b'\n') {
                    (last_content_start, last_content_end)
                } else {
                    (line_start, start)
                };
            let mut candidate_end = candidate_end;
            while candidate_end > candidate_start
                && matches!(bytes[candidate_end - 1], b'\r' | b'\n')
            {
                candidate_end -= 1;
            }
            let line = next.get(candidate_start..candidate_end).unwrap_or("");
            blocks.insert(block.start, metadata_line(line));
        }
        self.blocks = blocks;
    }
    pub fn apply(&self, parsed: &Parsed, lay: &mut Layout) {
        let block_count = parsed.blocks.len();
        let mut headers = vec![None; block_count];
        let mut block_bottoms = vec![f32::NEG_INFINITY; block_count];
        for (line_index, line) in lay.lines.iter().enumerate() {
            let Some(bottom) = block_bottoms.get_mut(line.block) else {
                continue;
            };
            *bottom = f32::max(*bottom, line.y + line.height);
            if headers[line.block].is_none()
                && matches!(line.decoration, Decoration::CodeHeader { .. })
            {
                headers[line.block] = Some(line_index);
            }
        }

        // 每个解析出的块各占一项，下面的正文过滤和前缀和都能做到与布局
        // 尺寸成线性。`Some` 与零高度移除含义不同：前者仍要求隐藏正文行。
        let mut removed = vec![None; block_count];
        for (bi, rb) in parsed.blocks.iter().enumerate() {
            let Some(attrs) = self.blocks.get(&rb.start) else {
                continue;
            };
            let Some(index) = headers[bi] else {
                continue;
            };
            let header = &mut lay.lines[index];
            header.runs.truncate(1);
            header.runs.push(Run::plain(attrs.title.clone()));
            // 无标题模式没有展开按钮，已持久化的折叠状态也不能让代码永久不可见。
            if attrs.collapsed && document::code_show_title() {
                if let Decoration::CodeHeader { body_lines, .. } = header.decoration {
                    header.decoration = Decoration::CodeHeader {
                        body_lines,
                        collapsed: true,
                    };
                }
                let start = header.y + document::code_header_height();
                let last_bottom = f32::max(start, block_bottoms[bi]);
                let amount = (last_bottom - start + document::code_pad_y() + 1.0).max(0.0);
                removed[bi] = Some((start, amount));
            }
        }

        // `removed_before[bi]` 是 `bi` 之前各块累计隐藏的高度。旧实现会为
        // 每一行重新求和；改用前缀和后 y 调整是 O(块数 + 行数)。
        let mut removed_before = vec![0.0; block_count + 1];
        for (bi, removal) in removed.iter().enumerate() {
            removed_before[bi + 1] =
                removed_before[bi] + removal.map(|(_, amount)| amount).unwrap_or(0.0);
        }
        let total_removed = removed_before[block_count];
        lay.lines.retain(|line| {
            let collapsed = removed.get(line.block).is_some_and(Option::is_some);
            !(collapsed && !matches!(line.decoration, Decoration::CodeHeader { .. }))
        });
        for line in &mut lay.lines {
            let amount = removed_before
                .get(line.block)
                .copied()
                .unwrap_or(total_removed);
            line.y -= amount;
        }

        // 布局标题与代码头都按源码顺序生成；对两条有序流各扫一遍即可，
        // 不必为每个标题遍历所有移除项。比较保持严格小于，以沿用旧行为：
        // 标题与某个代码头 y 坐标相同时不计入其隐藏量。
        let removals = removed.iter().filter_map(|r| *r).collect::<Vec<_>>();
        let mut next_removal = 0;
        let mut removed_for_headings = 0.0;
        for heading in &mut lay.headings {
            while next_removal < removals.len() && removals[next_removal].0 < heading.y {
                removed_for_headings += removals[next_removal].1;
                next_removal += 1;
            }
            heading.y -= removed_for_headings;
        }
        lay.height -= total_removed;
    }
}

fn metadata_line(line: &str) -> Attrs {
    let line = line.trim();
    if line.starts_with("<!-- mochi-code-block ") && line.ends_with("-->") {
        Attrs {
            title: super::containers::attribute(line, "title").unwrap_or_default(),
            collapsed: super::containers::attribute(line, "collapsed").as_deref() == Some("true"),
        }
    } else {
        Attrs::default()
    }
}

pub fn metadata(source: &str, start: usize) -> (std::ops::Range<usize>, Attrs) {
    let before = source
        .get(..start)
        .unwrap_or("")
        .trim_end_matches(['\r', '\n']);
    let line_start = before.rfind('\n').map(|n| n + 1).unwrap_or(0);
    let line = before[line_start..].trim();
    if line.starts_with("<!-- mochi-code-block ") && line.ends_with("-->") {
        return (line_start..start, metadata_line(line));
    }
    (start..start, Attrs::default())
}

pub fn update(buffer: &mut super::editor::TextBuffer, start: usize, attrs: Attrs) {
    let (range, _) = metadata(buffer.text(), start);
    let newline = if buffer.text().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let title = super::containers::escape(&attrs.title);
    let comment = format!(
        "<!-- mochi-code-block title=\"{title}\" collapsed=\"{}\" -->{newline}",
        attrs.collapsed
    );
    let cursor = comment.len();
    buffer.replace_range_select(range, &comment, cursor..cursor);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Collapse(usize),
    Title(usize),
    Copy(usize),
    Language(usize),
}
pub fn hit(header: Rect, start: usize, x: f32, y: f32) -> Option<Hit> {
    if !document::code_show_title() {
        return None;
    }
    if !header.contains(x, y) {
        return None;
    }
    let language_left = (header.right - 1.0 - 114.0).max(header.left + 41.0);
    Some(if x < header.left + 36.0 {
        Hit::Collapse(start)
    } else if x >= language_left {
        Hit::Language(start)
    } else if x >= language_left - 34.0 {
        Hit::Copy(start)
    } else {
        Hit::Title(start)
    })
}
pub fn language_range(source: &str, start: usize) -> Option<std::ops::Range<usize>> {
    let line = source
        .get(start..)?
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let indent = line.len() - line.trim_start().len();
    let (_, count, _) = document::fence_open(&line[indent..])?;
    Some(start + indent + count..start + line.len())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn reconcile_reference(source: &str, parsed: &Parsed) -> HashMap<usize, Attrs> {
        parsed
            .blocks
            .iter()
            .filter_map(|block| {
                matches!(block.block, document::Block::Code { .. })
                    .then(|| (block.start, metadata(source, block.start).1))
            })
            .collect()
    }

    fn apply_reference(state: &State, parsed: &Parsed, lay: &mut Layout) {
        let mut removed = Vec::new();
        for (bi, rb) in parsed.blocks.iter().enumerate() {
            let Some(attrs) = state.blocks.get(&rb.start) else {
                continue;
            };
            let Some(index) = lay.lines.iter().position(|line| {
                line.block == bi && matches!(line.decoration, Decoration::CodeHeader { .. })
            }) else {
                continue;
            };
            let header = &mut lay.lines[index];
            header.runs.truncate(1);
            header.runs.push(Run::plain(attrs.title.clone()));
            if attrs.collapsed && document::code_show_title() {
                if let Decoration::CodeHeader { body_lines, .. } = header.decoration {
                    header.decoration = Decoration::CodeHeader {
                        body_lines,
                        collapsed: true,
                    };
                }
                let start = header.y + document::code_header_height();
                let last_bottom = lay
                    .lines
                    .iter()
                    .filter(|line| line.block == bi)
                    .map(|line| line.y + line.height)
                    .fold(start, f32::max);
                let amount = (last_bottom - start + document::code_pad_y() + 1.0).max(0.0);
                removed.push((bi, start, amount));
            }
        }
        lay.lines.retain(|line| {
            !removed.iter().any(|(bi, _, _)| {
                *bi == line.block && !matches!(line.decoration, Decoration::CodeHeader { .. })
            })
        });
        for line in &mut lay.lines {
            line.y -= removed
                .iter()
                .filter(|(bi, _, _)| *bi < line.block)
                .map(|(_, _, amount)| amount)
                .sum::<f32>();
        }
        for heading in &mut lay.headings {
            heading.y -= removed
                .iter()
                .filter(|(_, start, _)| *start < heading.y)
                .map(|(_, _, amount)| amount)
                .sum::<f32>();
        }
        lay.height -= removed.iter().map(|(_, _, amount)| amount).sum::<f32>();
    }

    #[test]
    fn reconcile_parsed_matches_per_block_metadata_for_many_cards() {
        let source = concat!(
            "前文\n",
            "<!-- mochi-code-block title=\"第一\" collapsed=\"true\" -->\r\n",
            "```rust\r\n",
            "fn first() {}\r\n",
            "```\r\n",
            "\n",
            "中间段落\n",
            "  <!-- mochi-code-block title='第二' collapsed='false' -->\n",
            "```python\n",
            "print(2)\n",
            "```\n",
            "<!-- mochi-code-block title=\"第三\" collapsed=\"true\" -->\n",
            "```text\n",
            "third\n",
            "```\n",
        );
        let parsed = document::parse_ranged(source);
        let expected = reconcile_reference(source, &parsed);
        let mut state = State::default();
        state.reconcile_parsed(source, &parsed);
        assert_eq!(state.blocks, expected);
    }

    #[test]
    fn apply_matches_reference_for_many_collapsed_cards() {
        let source = concat!(
            "# 前文\n",
            "```rust\n",
            "fn first() {}\n",
            "```\n",
            "中间段落\n",
            "```python\n",
            "print(2)\n",
            "```\n",
            "## 后文\n",
            "```text\n",
            "third\n",
            "```\n",
        );
        let parsed = document::parse_ranged(source);
        let mut state = State::default();
        for (index, block) in parsed.blocks.iter().enumerate() {
            if matches!(block.block, document::Block::Code { .. }) {
                state.blocks.insert(
                    block.start,
                    Attrs {
                        collapsed: index != 2,
                        title: format!("卡片 {index}"),
                    },
                );
            }
        }
        let mut reference = document::layout_live(&parsed.blocks, source, None, 600.0, &|_| None);
        let mut optimized = reference.clone();
        apply_reference(&state, &parsed, &mut reference);
        state.apply(&parsed, &mut optimized);
        assert_eq!(optimized.lines, reference.lines);
        assert_eq!(optimized.headings, reference.headings);
        assert_eq!(optimized.height, reference.height);
        assert_eq!(optimized.table_widths, reference.table_widths);
    }

    #[test]
    fn collapsed_card_removes_body_and_moves_following_heading() {
        let src = "```rust\nfn main() {}\n```\n# 后文";
        let parsed = document::parse_ranged(src);
        let mut lay = document::layout_live(&parsed.blocks, src, None, 600.0, &|_| None);
        let old = lay.height;
        let mut s = State::default();
        s.blocks.insert(
            0,
            Attrs {
                collapsed: true,
                title: "示例".into(),
            },
        );
        s.apply(&parsed, &mut lay);
        assert!(lay.height < old);
        assert!(!lay
            .lines
            .iter()
            .any(|l| matches!(l.decoration, Decoration::CodeBackground { .. })));
        assert!(lay.headings[0].y < old);
    }
    #[test]
    fn title_state_moves_when_text_is_inserted_before_code() {
        let mut s = State::default();
        let source =
            "前文\n<!-- mochi-code-block title=\"代码\" collapsed=\"true\" -->\n```rs\nx\n```";
        s.reconcile(source);
        let next = source.replace("前文", "新的前文");
        s.reconcile(&next);
        assert_eq!(s.blocks[&next.find("```rs").unwrap()].title, "代码");
        assert!(s.blocks[&next.find("```rs").unwrap()].collapsed);
    }
    #[test]
    fn attributes_roundtrip_without_touching_code_and_undo_restores_original() {
        let original = "前文\r\n```rust\r\nprintln!(\"中文😀\");\r\n```";
        let mut buffer = super::super::editor::TextBuffer::new(original);
        let attrs = Attrs {
            title: "名称 & \"说明\"".into(),
            collapsed: true,
        };
        update(
            &mut buffer,
            original.find("```rust").unwrap(),
            attrs.clone(),
        );
        let start = buffer.text().find("```rust").unwrap();
        assert_eq!(metadata(buffer.text(), start).1, attrs);
        assert!(buffer
            .text()
            .ends_with("```rust\r\nprintln!(\"中文😀\");\r\n```"));
        let parsed = document::parse_ranged(buffer.text());
        assert!(!parsed.blocks.iter().any(
            |b| matches!(&b.block, document::Block::Paragraph(s) if s.contains("mochi-code-block"))
        ));
        buffer.undo();
        assert_eq!(buffer.text(), original);
    }

    #[test]
    fn metadata_survives_block_id_conversion_before_the_code_card_comment() {
        let source = "<!-- mochi-code-block title=\"演示\" collapsed=\"true\" -->\r\n```rust\r\n中文😀\r\n```";
        let converted = mochi_blocks::model::Document::import("editor", source)
            .unwrap()
            .render_with_ids();
        let code_start = converted.find("```rust").unwrap();
        let (_, attrs) = metadata(&converted, code_start);
        assert_eq!(
            attrs,
            Attrs {
                title: "演示".into(),
                collapsed: true,
            }
        );
        assert!(
            converted.find("<!-- mochi:block").unwrap()
                < converted.find("<!-- mochi-code-block").unwrap()
        );
        let reopened = mochi_blocks::model::Document::import("editor", &converted).unwrap();
        let reopened_start = converted.find("```rust").unwrap();
        let (_, reopened_attrs) = metadata(reopened.source(), reopened_start);
        assert_eq!(reopened_attrs, attrs);
    }
    #[test]
    fn language_replacement_preserves_indentation_crlf_and_body() {
        let src = "  ```rust\r\n代码\r\n  ```";
        let range = language_range(src, 0).unwrap();
        let mut next = src.to_owned();
        next.replace_range(range, "python");
        assert_eq!(next, "  ```python\r\n代码\r\n  ```");
    }
}
