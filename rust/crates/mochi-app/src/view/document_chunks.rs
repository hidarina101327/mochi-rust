//! 按需拆分文档内容，并在分页或加载后恢复导航位置。
use super::*;

impl DocPane {
    pub fn navigation_chunk_for(&self, path: &Path) -> Option<usize> {
        (self.code_document == path && self.is_chunked())
            .then(|| self.chunk_offset(self.chunk_index(), false))
            .flatten()
    }

    pub fn restore_navigation_chunk(&mut self, path: &Path, offset: usize) {
        self.chunk_positions.insert(path.to_path_buf(), offset);
    }

    pub(super) fn adjacent_page_caret(
        &self,
        cursor: usize,
        forward: bool,
        vertical: bool,
    ) -> Option<usize> {
        if !self.is_chunked() || self.is_loading() {
            return None;
        }
        let (line, _) = self.live.locate(cursor)?;
        let navigable = |line: &document::LaidOutLine| {
            line.source.is_some() || line.visible_start != usize::MAX
        };
        let edge = if forward {
            self.live.layout.lines.iter().rposition(navigable)?
        } else {
            self.live.layout.lines.iter().position(navigable)?
        };
        if line != edge {
            return None;
        }
        let (start, end) = self.live.line_bounds(cursor);
        if !vertical && ((forward && cursor < end) || (!forward && cursor > start)) {
            return None;
        }
        let index = if forward {
            self.chunk_index.checked_add(1)?
        } else {
            self.chunk_index.checked_sub(1)?
        };
        self.chunk_offset(index, !forward)
    }
    pub fn is_chunked(&self) -> bool {
        self.chunk_plan.is_some() && !self.full_documents.contains(&self.code_document)
    }

    pub fn chunk_plan(&self) -> Option<&crate::ui::large_document::Plan> {
        self.chunk_plan.as_ref()
    }
    pub fn chunk_index(&self) -> usize {
        self.chunk_index
    }

    pub(super) fn remember_chunk(&mut self) {
        if let Some(chunk) = self
            .chunk_plan
            .as_ref()
            .and_then(|p| p.chunks.get(self.chunk_index))
        {
            self.chunk_positions
                .insert(self.code_document.clone(), chunk.source.start);
        }
    }

    pub fn chunk_offset(&self, index: usize, end: bool) -> Option<usize> {
        let chunk = self.chunk_plan.as_ref()?.chunks.get(index)?;
        Some(if end {
            self.live.source()[chunk.source.clone()]
                .char_indices()
                .next_back()
                .map_or(chunk.source.start, |(at, _)| chunk.source.start + at)
        } else {
            chunk.source.start.max(self.live.parsed.body_start)
        })
    }

    fn load_chunk_images(&mut self) {
        for block in &self.live.parsed.blocks[self.layout_blocks.clone()] {
            if let document::Block::Image { src, .. } = &block.block {
                let resolved = document::resolve_image_src(src, self.base_dir.as_deref());
                self.image_sizes.entry(resolved.clone()).or_insert_with(|| {
                    if resolved.starts_with("http") || resolved.starts_with("data:") {
                        None
                    } else {
                        crate::ui::imginfo::dimensions(Path::new(&resolved))
                    }
                });
            }
        }
    }

    pub(super) fn reveal_chunk(&mut self, area: Rect, cursor: usize) {
        if !self.is_chunked() {
            return;
        }
        let index = self.chunk_plan.as_ref().unwrap().chunk_at(cursor);
        if index != self.chunk_index {
            self.select_chunk(area, index);
        }
    }

    pub fn select_chunk(&mut self, area: Rect, index: usize) -> bool {
        if !self.is_chunked() {
            return false;
        }
        let Some(chunk) = self.chunk_plan.as_ref().and_then(|p| p.chunks.get(index)) else {
            return false;
        };
        let range = chunk.blocks.clone();
        self.live.clear_view(self.layout_blocks.clone());
        self.layout_blocks = range;
        self.chunk_index = index;
        self.complete = false;
        self.background_headers.clear();
        self.load_chunk_images();
        self.loading = Some(document::LayoutProgress::default());
        self.loading_caret = None;
        if crate::ui::editor_preferences::current().live_line_source {
            self.layout_current_source_line(area, self.chunk_offset(index, false));
        } else {
            self.advance_loading(area);
        }
        self.remember_chunk();
        true
    }

    pub fn toggle_chunking(&mut self) {
        if self.is_chunked() {
            self.full_documents.insert(self.code_document.clone());
        } else {
            self.full_documents.remove(&self.code_document);
        }
        self.complete = false;
        self.invalidate();
    }

    pub fn heading_offset(&self, index: usize) -> Option<usize> {
        let block = *self.chunk_plan.as_ref()?.heading_blocks.get(index)?;
        Some(self.live.parsed.blocks[block].start)
    }

    pub fn active_heading(&self, scroll: f32) -> Option<usize> {
        let Some(plan) = &self.chunk_plan else {
            return crate::ui::outline::active_at(&self.live.layout.headings, scroll);
        };
        let index = self
            .live
            .layout
            .lines
            .partition_point(|line| line.y <= scroll + 64.0)
            .saturating_sub(1);
        let block = self.live.layout.lines.get(index)?.block;
        plan.heading_blocks
            .partition_point(|&i| i <= block)
            .checked_sub(1)
    }

    // 源码行模式保留原有的绘制行为。同步执行的工作
    // 只处理当前内容页，不扫描整篇文档。
    pub(super) fn layout_current_source_line(&mut self, area: Rect, cursor: Option<usize>) {
        let range = self.layout_blocks.clone();
        let active = cursor
            .and_then(|c| self.live.parsed.block_at(c))
            .filter(|i| range.contains(i))
            .map(|i| i - range.start);
        let sizes = &self.image_sizes;
        let base = self.base_dir.as_deref();
        let mut layout = document::layout_editor_cached(
            &self.live.parsed.blocks[range.clone()],
            self.live.source(),
            active,
            document::content_width(area),
            &|src| {
                sizes
                    .get(&document::resolve_image_src(src, base))
                    .copied()
                    .flatten()
            },
            &mut self.layout_cache,
        );
        for line in &mut layout.lines {
            line.block += range.start;
        }
        layout.table_widths = layout
            .table_widths
            .into_iter()
            .map(|(i, w)| (i + range.start, w))
            .collect();
        self.decorate_layout(area, &mut layout, range);
        if !self.code_document.as_os_str().is_empty() {
            let offset = crate::ui::file_title::body_offset();
            for line in &mut layout.lines {
                line.y += offset;
            }
            for heading in &mut layout.headings {
                heading.y += offset;
            }
            layout.height += offset;
        }
        self.live.layout = layout;
        self.loading = None;
        self.complete = true;
        self.rebuild_background_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 800.0,
        bottom: 650.0,
    };
    fn source() -> String {
        (0..220)
            .map(|i| format!("## 标题 {i}😀\r\n\r\n正文 **粗体 {i}** 和 `代码`。\r\n\r\n"))
            .collect()
    }
    fn finish(pane: &mut DocPane) {
        let mut steps = 0;
        while pane.is_loading() {
            pane.advance_loading(AREA);
            steps += 1;
            assert!(steps < 1000);
        }
    }

    #[test]
    fn default_chunks_mount_only_current_page_and_outline_covers_the_whole_file() {
        let original = source();
        let buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        finish(&mut pane);
        assert!(pane.is_chunked());
        assert_eq!(pane.headings().len(), 220);
        assert!(pane.layout_blocks.len() < pane.parsed().blocks.len() / 3);
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .all(|line| pane.layout_blocks.contains(&line.block)));
        let total = pane.chunk_plan().unwrap().chunks.len();
        for index in [total - 1, total / 2, 0] {
            assert!(pane.select_chunk(AREA, index));
            finish(&mut pane);
            assert_eq!(pane.chunk_index(), index);
            assert!(pane
                .live
                .layout
                .lines
                .iter()
                .all(|line| pane.layout_blocks.contains(&line.block)));
            assert_eq!(pane.display_source(), original);
        }
        assert_eq!(buffer.text(), original);
        assert!(!buffer.dirty());
    }

    #[test]
    fn cross_chunk_edits_undo_tail_jump_and_selection_keep_absolute_source_offsets() {
        let original = source();
        let mut buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        let at = pane.heading_offset(180).unwrap() + "## ".len();
        buffer.set_cursor(at, false);
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(at));
        finish(&mut pane);
        assert!(pane.chunk_index() > 0);
        pane.insert(&mut buffer, "新增😀 ");
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert!(pane.headings()[180].text.starts_with("新增😀"));
        assert_eq!(&buffer.text()[..at], &original[..at]);
        assert!(buffer.undo());
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(buffer.text(), original);
        pane.handle_key(&mut buffer, 0x23, false, true);
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(
            pane.chunk_index() + 1,
            pane.chunk_plan().unwrap().chunks.len()
        );
        assert_eq!(buffer.cursor(), original.len());
        assert!(pane.live.locate(buffer.cursor()).is_some());
        buffer.set_cursor(0, false);
        buffer.set_cursor(original.len(), true);
        let selected = pane.selected_text(&buffer);
        assert!(selected.contains("标题 0😀") && selected.contains("标题 219😀"));
    }

    #[test]
    fn structural_edits_repack_safely_and_full_document_toggle_preserves_text() {
        let original = source();
        let mut buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        let at = pane.heading_offset(180).unwrap();
        buffer.set_cursor(at, false);
        buffer.insert("# 新章节\r\n\r\n");
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(pane.headings().len(), 221);
        assert!(pane.chunk_index() > 0);
        pane.toggle_chunking();
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert!(!pane.is_chunked());
        assert_eq!(pane.layout_blocks, 0..pane.parsed().blocks.len());
        pane.toggle_chunking();
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert!(pane.is_chunked());
        buffer.undo();
        assert_eq!(buffer.text(), original);
    }

    #[test]
    fn directional_keys_cross_page_boundaries_and_keep_shift_selection() {
        let original = source();
        let mut buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        finish(&mut pane);
        let end = pane.chunk_offset(0, true).unwrap();
        buffer.set_cursor(end, false);
        assert!(pane.handle_key(&mut buffer, 0x28, true, false).handled);
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(pane.chunk_index(), 1);
        assert!(buffer.has_selection());
        let start = pane.chunk_offset(1, false).unwrap();
        buffer.set_cursor(start, false);
        pane.handle_key(&mut buffer, 0x26, false, false);
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(pane.chunk_index(), 0);
        buffer.set_cursor(pane.chunk_offset(0, true).unwrap(), false);
        pane.handle_key(&mut buffer, 0x27, false, false);
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(pane.chunk_index(), 1);
        buffer.set_cursor(pane.chunk_offset(1, false).unwrap(), false);
        pane.handle_key(&mut buffer, 0x25, false, false);
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(buffer.cursor()));
        finish(&mut pane);
        assert_eq!(pane.chunk_index(), 0);
        assert_eq!(buffer.text(), original);
    }

    #[test]
    fn page_position_survives_resize_tab_switch_and_source_line_mode() {
        let original = source();
        let buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.set_code_document(Path::new("pages.md"));
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        pane.select_chunk(AREA, 3);
        finish(&mut pane);
        let narrow = Rect::from_size(0.0, 0.0, 540.0, 650.0);
        pane.ensure_buffer(narrow, Some(0), &buffer, None);
        while pane.is_loading() {
            pane.advance_loading(narrow);
        }
        assert_eq!(pane.chunk_index(), 3);
        pane.set_code_document(Path::new("small.md"));
        pane.ensure_buffer(AREA, Some(1), &TextBuffer::new("small"), None);
        assert!(!pane.is_chunked());
        pane.set_code_document(Path::new("pages.md"));
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        finish(&mut pane);
        assert_eq!(pane.chunk_index(), 3);

        let original_prefs = crate::ui::editor_preferences::current();
        let mut prefs = original_prefs.clone();
        prefs.live_line_source = true;
        crate::ui::editor_preferences::set(prefs);
        let cursor = pane.heading_offset(180).unwrap();
        pane.ensure_buffer(AREA, Some(0), &buffer, Some(cursor));
        crate::ui::editor_preferences::set(original_prefs);
        assert!(pane.is_chunked());
        assert!(!pane.is_loading());
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .all(|line| pane.layout_blocks.contains(&line.block)));
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .any(|line| line.source.is_some_and(|(a, b)| a <= cursor && cursor <= b)));
        assert_eq!(pane.display_source(), original);
    }

    #[test]
    fn later_content_pages_keep_code_tables_containers_and_local_reflow_geometry() {
        let original: String = (0..200).map(|i| format!("## Section {i}\n\n```rust\nlet 中文 = {i};\n```\n\n| A | B |\n| --- | --- |\n| 甲 | 乙 |\n\n:::mochi-highlight title=\"Panel\"\n### Inside {i}\n\n正文😀\n:::\n\n")).collect();
        let buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        let index = pane.chunk_plan().unwrap().chunks.len() / 2;
        pane.select_chunk(AREA, index);
        finish(&mut pane);
        assert!(pane.layout_blocks.start > 0);
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .any(|line| matches!(line.decoration, document::Decoration::CodeHeader { .. })));
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .any(|line| matches!(line.decoration, document::Decoration::TableRow { .. })));
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .any(|line| matches!(line.decoration, document::Decoration::Container { .. })));
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .any(|line| !line.tokens.is_empty()));
        assert!(pane
            .live
            .layout
            .lines
            .iter()
            .all(|line| pane.layout_blocks.contains(&line.block)));
        assert_eq!(pane.display_source(), original);
    }

    #[test]
    #[ignore = "set MOCHI_CHUNK_BENCHMARK_FILE; no screenshots or source writes"]
    fn real_document_chunk_loading_probe() {
        let path = std::env::var("MOCHI_CHUNK_BENCHMARK_FILE").unwrap();
        let original = std::fs::read_to_string(&path).unwrap();
        let buffer = TextBuffer::new(&original);
        let mut pane = DocPane::default();
        pane.set_code_document(Path::new(&path));
        let started = std::time::Instant::now();
        pane.ensure_buffer(AREA, Some(0), &buffer, None);
        finish(&mut pane);
        let chunks = pane.chunk_plan().unwrap().chunks.len();
        eprintln!(
            "bytes={} chunks={} headings={} first_page={:?} laid_out_blocks={}/{}",
            original.len(),
            chunks,
            pane.headings().len(),
            started.elapsed(),
            pane.layout_blocks.len(),
            pane.parsed().blocks.len()
        );
        for index in [chunks / 2, chunks - 1] {
            let started = std::time::Instant::now();
            pane.select_chunk(AREA, index);
            finish(&mut pane);
            eprintln!(
                "page={} time={:?} blocks={}",
                index + 1,
                started.elapsed(),
                pane.layout_blocks.len()
            );
            assert!(pane
                .live
                .layout
                .lines
                .iter()
                .all(|line| pane.layout_blocks.contains(&line.block)));
        }
        assert_eq!(pane.display_source(), original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
