//! 分段加载大型文档，并在后台建立可见内容索引。
use super::*;

impl DocPane {
    pub(super) fn visible_block_range(&self, area: Rect, scroll: f32) -> std::ops::Range<usize> {
        let lines = &self.live.layout.lines;
        let first = lines.partition_point(|line| line.y + line.height < scroll - 13.0);
        let end = lines.partition_point(|line| line.y <= scroll + area.height() + 13.0);
        match (
            lines.get(first),
            end.checked_sub(1).and_then(|i| lines.get(i)),
        ) {
            (Some(first), Some(last)) if first.block <= last.block => first.block..last.block + 1,
            _ => 0..0,
        }
    }

    pub(super) fn rebuild_background_index(&mut self) {
        self.background_headers = self
            .live
            .layout
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                matches!(
                    line.decoration,
                    document::Decoration::Container { .. }
                        | document::Decoration::CodeHeader { .. }
                )
                .then_some(index)
            })
            .collect();
    }

    pub fn set_progressive(&mut self, enabled: bool) {
        self.progressive = enabled;
    }

    pub fn is_loading(&self) -> bool {
        self.key.is_some() && self.loading.is_some()
    }

    pub fn take_ready_loading_caret(&mut self, cursor: usize) -> bool {
        if self.loading_caret == Some(cursor) && self.live.locate(cursor).is_some() {
            self.loading_caret = None;
            true
        } else {
            false
        }
    }

    /// 将 UI 线程上的工作分成时间片，并在块边界让出执行权。Win32
    /// 会等输入处理和绘制完成后再安排下一个时间片；单个
    /// 大块可能超过目标时长，但不会因此丢失内容。
    pub fn advance_loading(&mut self, area: Rect) {
        if self.key.is_none() {
            return;
        }
        let Some(progress) = self.loading.as_mut() else {
            return;
        };
        let first = self.layout_blocks.start + progress.next_block;
        let sizes = &self.image_sizes;
        let base = self.base_dir.as_deref();
        let sizer = |src: &str| {
            sizes
                .get(&document::resolve_image_src(src, base))
                .copied()
                .flatten()
        };
        let mut chunk = document::layout_editor_chunk(
            &self.live.parsed.blocks[self.layout_blocks.clone()],
            self.live.source(),
            document::content_width(area),
            &sizer,
            &mut self.layout_cache,
            progress,
            std::time::Duration::from_millis(6),
        );
        let end = self.layout_blocks.start + progress.next_block;
        for line in &mut chunk.lines {
            line.block += self.layout_blocks.start;
        }
        chunk.table_widths = chunk
            .table_widths
            .into_iter()
            .map(|(i, widths)| (i + self.layout_blocks.start, widths))
            .collect();
        let raw_height = chunk.height;
        self.decorate_layout(area, &mut chunk, first..end);
        let progress = self.loading.as_mut().unwrap();
        progress.shift_y(chunk.height - raw_height);
        let complete = progress.is_complete(self.layout_blocks.len());
        // 像完整布局一样，恢复时仍使用正文坐标中的光标位置。
        // 如果先把标题高度加到累计的数百万像素上，
        // 后续各行的 f32 舍入结果就会发生变化。
        if !self.code_document.as_os_str().is_empty() {
            let offset = crate::ui::file_title::body_offset();
            for line in &mut chunk.lines {
                line.y += offset;
            }
            for heading in &mut chunk.headings {
                heading.y += offset;
            }
            chunk.height += offset;
        }
        let base_index = self.live.layout.lines.len();
        self.background_headers
            .extend(chunk.lines.iter().enumerate().filter_map(|(index, line)| {
                matches!(
                    line.decoration,
                    document::Decoration::Container { .. }
                        | document::Decoration::CodeHeader { .. }
                )
                .then_some(base_index + index)
            }));
        self.live.layout.lines.extend(chunk.lines);
        self.live.layout.headings.extend(chunk.headings);
        self.live.layout.table_widths.extend(chunk.table_widths);
        self.live.layout.height = chunk.height;
        if complete {
            self.loading = None;
            self.complete = true;
        }
    }
}
