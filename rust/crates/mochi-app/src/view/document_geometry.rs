//! 计算文档行、表格列和标记区域的几何位置。
use super::*;

impl DocPane {
    pub(super) fn decorate_layout(
        &self,
        area: Rect,
        layout: &mut document::Layout,
        blocks: std::ops::Range<usize>,
    ) {
        if self.live.parsed.blocks[blocks.clone()]
            .iter()
            .any(|rb| matches!(rb.block, document::Block::Code { .. }))
        {
            // 代码卡片只根据当前输出的内容块进行协调。所有
            // 源码偏移仍是绝对位置，只有块索引改为局部编号。
            let parsed = document::Parsed {
                blocks: self.live.parsed.blocks[blocks.clone()].to_vec(),
                body_start: self.live.parsed.body_start,
            };
            for line in &mut layout.lines {
                line.block -= blocks.start;
            }
            self.code_state.apply(&parsed, layout);
            for line in &mut layout.lines {
                line.block += blocks.start;
            }
        }
        if let Some(widths) = self.table_widths.get(&self.code_document) {
            for bi in blocks.clone() {
                if let Some(table) = widths.get(&self.live.parsed.blocks[bi].start) {
                    layout.table_widths.insert(bi, table.clone());
                }
            }
        }
        let width = document::content_width(area);
        if let Some(offsets) = self.table_offsets.get(&self.code_document) {
            for line in &mut layout.lines {
                if let document::Decoration::TableRow { cols, .. } = line.decoration {
                    let start = self.live.parsed.blocks[line.block].start;
                    let full = layout
                        .table_widths
                        .get(&line.block)
                        .filter(|w| w.len() == cols)
                        .map(|w| w.iter().sum())
                        .unwrap_or(document::table_column_width(cols, width) * cols as f32);
                    line.x -= offsets
                        .get(&start)
                        .copied()
                        .unwrap_or(0.0)
                        .min((full - width).max(0.0));
                }
            }
        }
        if !crate::ui::editor_preferences::current().code_wrap {
            for bi in blocks {
                if !matches!(
                    self.live.parsed.blocks[bi].block,
                    document::Block::Code { .. }
                ) {
                    continue;
                }
                let start = self.live.parsed.blocks[bi].start;
                let offset = self
                    .code_offsets
                    .get(&self.code_document)
                    .and_then(|m| m.get(&start))
                    .copied()
                    .unwrap_or(0.0);
                if offset > 0.0 {
                    let offset = offset.min(document::code_horizontal_overflow(layout, bi, width));
                    let range = layout.block_line_range(bi);
                    for line in &mut layout.lines[range] {
                        if matches!(line.decoration, document::Decoration::CodeBackground { .. }) {
                            line.x -= offset;
                        }
                    }
                }
            }
        }
    }

    pub(super) fn widths_for(&self, start: usize, cols: usize, available: f32) -> Vec<f32> {
        self.table_widths
            .get(&self.code_document)
            .and_then(|tables| tables.get(&start))
            .filter(|widths| widths.len() == cols)
            .cloned()
            .unwrap_or_else(|| vec![document::table_column_width(cols, available); cols])
    }

    pub fn table_column_handles(
        &self,
        area: Rect,
        scroll: f32,
    ) -> Vec<(usize, usize, Rect, Vec<f32>)> {
        let available = document::content_width(area);
        let origin = area.left + crate::ui::editor_preferences::current().padding_left;
        let mut handles = Vec::new();
        for bi in self.visible_block_range(area, scroll) {
            let block = &self.live.parsed.blocks[bi];
            if !matches!(block.block, document::Block::Table { .. }) {
                continue;
            }
            let rows = self
                .live
                .layout
                .lines_for_block(bi)
                .iter()
                .filter(|line| {
                    line.block == bi
                        && matches!(line.decoration, document::Decoration::TableRow { .. })
                })
                .collect::<Vec<_>>();
            let Some(first) = rows.first() else { continue };
            let Some(last) = rows.last() else { continue };
            let document::Decoration::TableRow { cols, .. } = first.decoration else {
                continue;
            };
            let widths = self.widths_for(block.start, cols, available);
            let mut x = origin + first.x;
            for col in 0..cols.saturating_sub(1) {
                x += widths[col];
                handles.push((
                    block.start,
                    col,
                    Rect::new(
                        x - 4.0,
                        area.top + first.y - scroll,
                        x + 4.0,
                        area.top + last.y + last.height - scroll,
                    ),
                    widths.clone(),
                ));
            }
        }
        handles
    }

    pub fn set_table_widths(&mut self, start: usize, widths: Vec<f32>) {
        self.table_widths
            .entry(self.code_document.clone())
            .or_default()
            .insert(start, widths);
        self.key = None;
    }

    /// 使用已渲染编辑器持有的解析结果插入可见文本。
    /// 替换选区时仍只重新解析受影响的
    /// 源码，并调用 `rich::insert_parsed`；常见的折叠光标输入路径
    /// 不再需要每输入一个字符就解析整篇文档。
    pub fn insert(&self, buffer: &mut TextBuffer, value: &str) {
        crate::ui::rich::insert_parsed(buffer, value, &self.live.parsed);
    }

    pub fn mark_rects(
        &self,
        area: Rect,
        range: std::ops::Range<usize>,
        scroll: f32,
        palette: &Palette,
    ) -> Vec<Rect> {
        let mut list = DrawList::new();
        live::paint_selection(&mut list, area, &self.live, range, scroll, palette);
        list.cmds()
            .iter()
            .filter_map(|cmd| {
                if let crate::ui::draw::DrawCmd::RectAlpha { rect, .. } = cmd {
                    Some(*rect)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn parsed(&self) -> &document::Parsed {
        &self.live.parsed
    }

    pub fn display_source(&self) -> &str {
        self.live.source()
    }

    pub fn image_rect(&self, area: Rect, scroll: f32, start: usize) -> Option<Rect> {
        let line = self.live.layout.lines.iter().find(|l| {
            l.decoration == document::Decoration::Image
                && self.live.parsed.blocks[l.block].start == start
        })?;
        let left = area.left + crate::ui::editor_preferences::current().padding_left + line.x;
        let width = line.runs.get(2)?.text.parse::<f32>().ok()?;
        Some(Rect::from_size(
            left,
            area.top + line.y - scroll,
            width,
            line.height,
        ))
    }

    pub fn image_handle(&self, area: Rect, scroll: f32, x: f32, y: f32) -> Option<(usize, Rect)> {
        let start = self.image_at(area, scroll, x, y)?;
        let rect = self.image_rect(area, scroll, start)?;
        Some((
            start,
            Rect::from_size(rect.right - 13.0, rect.bottom - 13.0, 12.0, 12.0),
        ))
    }

    /// 显示滚动轨道，让用户不用了解 Shift+滚轮也能发现溢出内容。
    pub fn table_scrollbars(&self, area: Rect, scroll: f32) -> Vec<(usize, Rect, Rect, f32)> {
        let available = document::content_width(area);
        let left = area.left + crate::ui::editor_preferences::current().padding_left;
        let lines = &self.live.layout.lines;
        let first = lines.partition_point(|line| line.y + line.height < scroll - 13.0);
        let end = lines.partition_point(|line| line.y <= scroll + area.height());
        self.live.layout.lines[first.min(end)..end]
            .iter()
            .enumerate()
            .filter_map(|(i, line)| {
                let document::Decoration::TableRow { cols, .. } = line.decoration else {
                    return None;
                };
                if self
                    .live
                    .layout
                    .lines
                    .get(first + i + 1)
                    .is_some_and(|next| next.block == line.block)
                {
                    return None;
                }
                let start = self.live.parsed.blocks[line.block].start;
                let full = self.widths_for(start, cols, available).iter().sum::<f32>();
                if full <= available {
                    return None;
                }
                let offset = self
                    .table_offsets
                    .get(&self.code_document)
                    .and_then(|m| m.get(&start))
                    .copied()
                    .unwrap_or(0.0)
                    .min(full - available);
                let track = Rect::from_size(
                    left,
                    area.top + line.y + line.height - scroll + 3.0,
                    available,
                    10.0,
                );
                let width = (available * available / full).max(24.0).min(available);
                let thumb = Rect::from_size(
                    left + offset / (full - available) * (available - width),
                    track.top,
                    width,
                    10.0,
                );
                Some((start, track, thumb, full - available))
            })
            .collect()
    }

    pub fn set_table_scroll(&mut self, start: usize, offset: f32) {
        self.table_offsets
            .entry(self.code_document.clone())
            .or_default()
            .insert(start, offset);
        self.invalidate();
    }

    pub fn selected_text(&self, buffer: &TextBuffer) -> String {
        self.live.selected_text(buffer)
    }

    pub fn math_at(
        &self,
        area: Rect,
        source: &str,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> Option<(std::ops::Range<usize>, String, bool)> {
        let local_x = x - area.left - crate::ui::editor_preferences::current().padding_left;
        let local_y = y - area.top + scroll;
        let i = self.live.line_at(local_y)?;
        let line = &self.live.layout.lines[i];
        if local_y < line.y || local_y >= line.y + line.height {
            return None;
        }
        let rb = &self.live.parsed.blocks[line.block];
        if let document::Block::Math(tex) = &rb.block {
            return Some((rb.start..rb.end, tex.clone(), true));
        }
        let mut left = line.x;
        for run in &line.runs {
            let width = crate::ui::text::run_width(run, line.style);
            if local_x >= left
                && local_x < left + width
                && run.emphasis.base() == crate::ui::text::Emphasis::Math
            {
                let offset = self.live.offset_in_line(source, i, left);
                let body = crate::ui::rich::body_range(rb, source);
                let span = crate::ui::text::parse_inline_spans(&source[body.clone()])
                    .into_iter()
                    .find(|s| {
                        s.run.emphasis.base() == crate::ui::text::Emphasis::Math
                            && body.start + s.start <= offset
                            && offset <= body.start + s.end
                    })?;
                return Some((
                    body.start + span.start..body.start + span.end,
                    span.run.text,
                    false,
                ));
            }
            left += width;
        }
        None
    }
}
