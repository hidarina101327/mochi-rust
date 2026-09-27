//! 按标题分页且不丢失内容，与 Electron 的 largeDocument 行为一致。
//! 完整源码保留在 TextBuffer 中，只排版当前页面。
use super::{
    document::{Block, Heading, Parsed},
    draw::{Align, DrawList, TextStyle},
    layout::Rect,
    theme::Palette,
};
use std::ops::Range;

pub const SIZE_BYTES: usize = 512 * 1024;
pub const HEADING_COUNT: usize = 180;
pub const LINE_COUNT: usize = 8_000;
pub const TARGET_CHARS: usize = 48_000;
pub const MAX_HEADINGS: usize = 36;
pub const HARD_CHARS: usize = 96_000;
pub const BAR_HEIGHT: f32 = 34.0;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub blocks: Range<usize>,
    pub source: Range<usize>,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub chunks: Vec<Chunk>,
    pub headings: Vec<Heading>,
    pub heading_blocks: Vec<usize>,
}

fn heading(block: &Block) -> Option<(u8, &str)> {
    match block {
        Block::Heading { level, text }
        | Block::Aligned {
            level: level @ 1..=6,
            text,
            ..
        } => Some((*level, text)),
        _ => None,
    }
}

impl Plan {
    pub fn build(source: &str, parsed: &Parsed) -> Option<Self> {
        let heading_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(i, rb)| heading(&rb.block).map(|_| i))
            .collect();
        if source.len() < SIZE_BYTES
            && heading_blocks.len() < HEADING_COUNT
            && source.bytes().filter(|&b| b == b'\n').count() + usize::from(!source.is_empty())
                < LINE_COUNT
        {
            return None;
        }
        let mut plan = Self {
            chunks: Vec::new(),
            headings: Vec::new(),
            heading_blocks,
        };
        // 解析器已经能识别代码围栏、表格、数学公式和容器
        // 边界。不要把代码示例中的“# text”误当成标题，
        // 也不要把容器与它的结束标记拆开。
        let mut units = Vec::new();
        let mut i = 0;
        while i < parsed.blocks.len() {
            let start = i;
            i += 1;
            if let Block::Container(panel) = &parsed.blocks[start].block {
                while i < parsed.blocks.len() && parsed.blocks[i].start < panel.footer.end {
                    i += 1;
                }
            }
            units.push(start..i);
        }
        if units.is_empty() {
            return None;
        }
        let offset = |index: usize| {
            if index == 0 {
                0
            } else {
                parsed.blocks.get(index).map_or(source.len(), |b| b.start)
            }
        };
        let heading_count = |range: &Range<usize>| {
            plan.heading_blocks.partition_point(|&b| b < range.end)
                - plan.heading_blocks.partition_point(|&b| b < range.start)
        };
        let mut sections: Vec<Range<usize>> = Vec::new();
        for (index, unit) in units.iter().enumerate() {
            if index == 0 || heading(&parsed.blocks[unit.start].block).is_some() {
                sections.push(index..index + 1);
            } else {
                sections.last_mut().unwrap().end = index + 1;
            }
        }
        let mut pieces: Vec<(Range<usize>, usize, usize)> = Vec::new();
        for section in sections {
            let blocks = units[section.start].start..units[section.end - 1].end;
            let chars = source[offset(blocks.start)..offset(blocks.end)]
                .encode_utf16()
                .count();
            if chars <= HARD_CHARS {
                let count = heading_count(&blocks);
                pieces.push((blocks, chars, count));
            } else {
                // 即使没有标题，过长的段落也可以按完整的
                // Markdown 块分页；单个超大的块仍保持完整。
                for unit in &units[section] {
                    let chars = source[offset(unit.start)..offset(unit.end)]
                        .encode_utf16()
                        .count();
                    pieces.push((unit.clone(), chars, heading_count(unit)));
                }
            }
        }
        let mut bucket: Option<Range<usize>> = None;
        let (mut chars, mut count) = (0, 0);
        for (piece, n, h) in pieces {
            if bucket.is_some() && (chars + n > TARGET_CHARS || count + h > MAX_HEADINGS) {
                plan.chunks.push(Chunk {
                    blocks: bucket.take().unwrap(),
                    source: 0..0,
                    title: String::new(),
                });
                chars = 0;
                count = 0;
            }
            if let Some(bucket) = &mut bucket {
                bucket.end = piece.end;
            } else {
                bucket = Some(piece);
            }
            chars += n;
            count += h;
        }
        if let Some(blocks) = bucket {
            plan.chunks.push(Chunk {
                blocks,
                source: 0..0,
                title: String::new(),
            });
        }
        plan.rebase(source, parsed);
        Some(plan)
    }

    /// 行内编辑会保留块索引。调整页面的字节范围并刷新
    /// 标题标签时，不要重新分配页面或丢失当前所在章节。
    pub fn rebase(&mut self, source: &str, parsed: &Parsed) {
        self.headings = self
            .heading_blocks
            .iter()
            .filter_map(|&i| {
                let (level, text) = heading(&parsed.blocks.get(i)?.block)?;
                Some(Heading {
                    level,
                    text: text.into(),
                    y: 0.0,
                })
            })
            .collect();
        for chunk in &mut self.chunks {
            chunk.source = if chunk.blocks.start == 0 {
                0
            } else {
                parsed.blocks[chunk.blocks.start].start
            }
                ..parsed
                    .blocks
                    .get(chunk.blocks.end)
                    .map_or(source.len(), |b| b.start);
            let hi = self
                .heading_blocks
                .partition_point(|&i| i < chunk.blocks.start);
            chunk.title = if self
                .heading_blocks
                .get(hi)
                .is_some_and(|&i| i < chunk.blocks.end)
            {
                self.headings[hi].text.clone()
            } else if chunk.blocks.start == 0 && !self.headings.is_empty() {
                "前言".into()
            } else {
                "正文".into()
            };
        }
    }

    pub fn chunk_at(&self, offset: usize) -> usize {
        self.chunks
            .partition_point(|chunk| chunk.source.start <= offset)
            .saturating_sub(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Previous,
    Next,
    Toggle,
}

pub fn buttons(area: Rect) -> [(Rect, Action); 3] {
    let width = ((area.width() - 24.0).max(0.0) / 3.0).min(62.0);
    let right = area.right - 6.0;
    [Action::Toggle, Action::Previous, Action::Next].map(|action| {
        let index = match action {
            Action::Toggle => 3,
            Action::Previous => 2,
            Action::Next => 1,
        };
        (
            Rect::from_size(
                right - index as f32 * (width + 4.0),
                area.top + 4.0,
                width,
                BAR_HEIGHT - 8.0,
            ),
            action,
        )
    })
}

pub fn paint(
    list: &mut DrawList,
    area: Rect,
    plan: &Plan,
    current: usize,
    paged: bool,
    p: &Palette,
) {
    list.rect(area, p.surface);
    let controls = buttons(area);
    let label = if paged {
        format!(
            "分片加载 · 第 {}/{} 段 · {}",
            current + 1,
            plan.chunks.len(),
            plan.chunks[current].title
        )
    } else {
        "完整文档 · 可切回分片加载".into()
    };
    list.text(
        Rect::new(
            area.left + 10.0,
            area.top,
            controls[0].0.left - 6.0,
            area.bottom,
        ),
        super::text::ellipsize(
            &label,
            TextStyle::Caption,
            (controls[0].0.left - area.left - 16.0).max(0.0),
        ),
        TextStyle::Caption,
        p.muted,
    );
    for (rect, action) in controls {
        let enabled = match action {
            Action::Toggle => true,
            Action::Previous => paged && current > 0,
            Action::Next => paged && current + 1 < plan.chunks.len(),
        };
        let label = match action {
            Action::Toggle => {
                if paged {
                    "全文"
                } else {
                    "分片"
                }
            }
            Action::Previous => {
                if rect.width() < 50.0 {
                    "‹"
                } else {
                    "上一段"
                }
            }
            Action::Next => {
                if rect.width() < 50.0 {
                    "›"
                } else {
                    "下一段"
                }
            }
        };
        if enabled {
            list.rounded_rect(rect, 4.0, p.surface_muted);
        }
        list.text_aligned(
            rect,
            label,
            TextStyle::Caption,
            if enabled { p.foreground } else { p.muted },
            Align::Center,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vocab() -> String {
        (0..220)
            .map(|i| format!("### 单词 {i}😀\r\n\r\n- **释义**：内容\r\n\r\n"))
            .collect()
    }
    #[test]
    fn chunks_default_to_electron_thresholds_and_join_byte_exactly() {
        let source = format!("\r\n前言\r\n{}", vocab());
        let parsed = super::super::document::parse_ranged(&source);
        let plan = Plan::build(&source, &parsed).unwrap();
        assert!(plan.chunks.len() > 1);
        assert_eq!(
            plan.chunks
                .iter()
                .map(|c| &source[c.source.clone()])
                .collect::<String>(),
            source
        );
        for (hi, &bi) in plan.heading_blocks.iter().enumerate() {
            let chunk = &plan.chunks[plan.chunk_at(parsed.blocks[bi].start)];
            assert!(chunk.blocks.contains(&bi));
            assert!(hi < plan.headings.len());
        }
        assert!(Plan::build(
            "# small\n",
            &super::super::document::parse_ranged("# small\n")
        )
        .is_none());
    }
    #[test]
    fn fake_code_headings_and_containers_are_not_split() {
        let source = format!(
            "```md\n{}\n```\n\n:::mochi-highlight color=\"blue\"\n{}\n:::\n\n{}",
            vocab(),
            vocab(),
            vocab()
        );
        let parsed = super::super::document::parse_ranged(&source);
        let plan = Plan::build(&source, &parsed).unwrap();
        assert_eq!(plan.headings.len(), 440);
        for chunk in &plan.chunks {
            for block in &parsed.blocks {
                if let Block::Container(panel) = &block.block {
                    assert!(
                        !(block.start < chunk.source.start
                            && chunk.source.start < panel.footer.end)
                    );
                }
            }
        }
        assert_eq!(
            plan.chunks
                .iter()
                .map(|c| &source[c.source.clone()])
                .collect::<String>(),
            source
        );
    }
    #[test]
    fn heading_free_text_pages_at_blocks_without_cutting_utf8_or_tables() {
        let source = "段落😀内容\n\n".repeat(40_000);
        let parsed = super::super::document::parse_ranged(&source);
        let plan = Plan::build(&source, &parsed).unwrap();
        assert!(plan.chunks.len() > 2);
        assert_eq!(
            plan.chunks
                .iter()
                .map(|c| &source[c.source.clone()])
                .collect::<String>(),
            source
        );
    }
}
