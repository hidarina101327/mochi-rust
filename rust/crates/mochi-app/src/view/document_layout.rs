//! 管理文档布局缓存，并按需构建或更新排版结果。
use super::*;

impl DocPane {
    /// 宽度取整成 key 的一部分——拖窗口时宽度是连续变化的浮点数，
    /// 不取整会每帧都判定为「变了」。未聚焦时按 Markdown 渲染语义折叠分隔空行。
    pub fn ensure(
        &mut self,
        area: Rect,
        tab: Option<usize>,
        text: Option<&str>,
        cursor: Option<usize>,
    ) {
        let text = text.unwrap_or("");
        self.ensure_generation(area, tab, text, cursor, (false, content_hash(text)));
    }

    /// 正式编辑器路径中，缓冲区准确知道内容何时变化，
    /// 因此检查缓存时不能重新扫描整篇文档。
    pub fn ensure_buffer(
        &mut self,
        area: Rect,
        tab: Option<usize>,
        buffer: &TextBuffer,
        cursor: Option<usize>,
    ) {
        self.ensure_generation(area, tab, buffer.text(), cursor, (true, buffer.revision()));
    }

    pub(super) fn ensure_generation(
        &mut self,
        area: Rect,
        tab: Option<usize>,
        text: &str,
        cursor: Option<usize>,
        generation: (bool, u64),
    ) {
        #[cfg(debug_assertions)]
        let profile_start = std::time::Instant::now();
        let width = document::content_width(area);
        let tab = tab.unwrap_or(usize::MAX);
        let width_key = width.round() as u32;
        let live_line_source = crate::ui::editor_preferences::current().live_line_source;
        let old_height = self.live.layout.height;
        if !live_line_source
            && (self.chunk_plan.is_some() || text.len() < crate::ui::large_document::SIZE_BYTES)
            && self.key.is_some_and(|(t, w, _, old_generation, active)| {
                t == tab && w == width_key && active.is_none() && old_generation != generation
            })
            && self.live.update_inline(text, width)
        {
            if let Some(plan) = &mut self.chunk_plan {
                plan.rebase(text, &self.live.parsed);
            }
            self.code_state.reconcile_parsed(text, &self.live.parsed);
            if let Some(progress) = &mut self.loading {
                progress.shift_y(self.live.layout.height - old_height);
            }
            self.rebuild_background_index();
            self.key = Some((tab, width_key, text.len(), generation, None));
            return;
        }
        // 上一次的键仍有效，表示已解析的块范围仍对应
        // 当前缓冲区。宽度变化不会影响解析，因此调整窗口大小后
        // 也可以继续使用同一份 Parsed。内容指纹可避免
        // 内容长度相同但实际内容不同的文档误用旧几何信息。
        let reusable_parsed = self.key.is_some_and(|(t, _, len, hash, _)| {
            t == tab && len == text.len() && hash == generation
        });
        let mut parsed = None;
        // 空段落已经有准确的源码行范围，因此移动光标
        // 不会让已渲染的几何信息失效。只有用户主动启用源码行模式时，
        // 布局键才需要记录当前块。
        let active = cursor.filter(|_| live_line_source).and_then(|c| {
            if reusable_parsed {
                let active = self.live.parsed.block_at(c);
                if live_line_source
                    || active.is_some_and(|bi| {
                        matches!(self.live.parsed.blocks[bi].block, document::Block::Blank)
                    })
                {
                    active
                } else {
                    None
                }
            } else {
                let next = document::parse_ranged(text);
                let active = next.block_at(c);
                let keep = live_line_source
                    || active
                        .is_some_and(|bi| matches!(next.blocks[bi].block, document::Block::Blank));
                parsed = Some(next);
                if keep {
                    active
                } else {
                    None
                }
            }
        });
        let key = (tab, width_key, text.len(), generation, active);
        if self.key == Some(key) {
            if let Some(cursor) = cursor {
                self.reveal_chunk(area, cursor);
            }
            if let Some(cursor) = cursor {
                if self.is_loading() && self.live.locate(cursor).is_none() {
                    self.loading_caret = Some(cursor);
                }
            }
            return;
        }
        // 先把这篇文档用到的图片尺寸读进缓存（只读文件头），排版时按缓存查
        let parsed = parsed.unwrap_or_else(|| {
            if reusable_parsed {
                // ensure 替换旧几何信息时，没有调用方会读取它；
                // 直接移动已解析的值，就不必为了调整文档宽度
                // 而复制整篇长文档。
                std::mem::take(&mut self.live.parsed)
            } else {
                document::parse_ranged(text)
            }
        });
        let chunk_anchor = cursor
            .or_else(|| {
                self.chunk_plan
                    .as_ref()
                    .and_then(|plan| plan.chunks.get(self.chunk_index))
                    .map(|c| c.source.start)
            })
            .or_else(|| self.chunk_positions.get(&self.code_document).copied())
            .unwrap_or(0);
        self.chunk_plan = crate::ui::large_document::Plan::build(text, &parsed);
        self.chunk_index = self
            .chunk_plan
            .as_ref()
            .map_or(0, |p| p.chunk_at(chunk_anchor));
        self.layout_blocks = if self.is_chunked() {
            self.chunk_plan.as_ref().unwrap().chunks[self.chunk_index]
                .blocks
                .clone()
        } else {
            0..parsed.blocks.len()
        };
        for rb in &parsed.blocks[self.layout_blocks.clone()] {
            if let document::Block::Image { src, .. } = &rb.block {
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
        let sizes = &self.image_sizes;
        #[cfg(debug_assertions)]
        let parse_elapsed = profile_start.elapsed();
        let base = self.base_dir.as_deref();
        let sizer = |src: &str| -> Option<(u32, u32)> {
            sizes
                .get(&document::resolve_image_src(src, base))
                .copied()
                .flatten()
        };
        if self.is_chunked() || (self.progressive && !live_line_source && !self.complete) {
            self.code_state.reconcile_parsed(text, &parsed);
            self.live = live::LiveLayout::pending(text, parsed, &self.mapping_cache);
            self.background_headers.clear();
            self.loading = Some(document::LayoutProgress::default());
            self.key = Some(key);
            if live_line_source && self.is_chunked() {
                self.layout_current_source_line(area, cursor);
            } else {
                self.advance_loading(area);
            }
            self.loading_caret = cursor.filter(|&cursor| self.live.locate(cursor).is_none());
            return;
        }
        self.loading = None;
        self.loading_caret = None;
        self.complete = true;
        self.live = live::layout_cached(
            text,
            parsed,
            cursor,
            width,
            &sizer,
            &mut self.layout_cache,
            &mut self.mapping_cache,
        );
        #[cfg(debug_assertions)]
        let layout_elapsed = profile_start.elapsed();
        let state = &mut self.code_state;
        state.reconcile_parsed(text, &self.live.parsed);
        state.apply(&self.live.parsed, &mut self.live.layout);
        if let Some(widths) = self.table_widths.get(&self.code_document) {
            for (bi, rb) in self.live.parsed.blocks.iter().enumerate() {
                if let Some(table) = widths.get(&rb.start) {
                    self.live.layout.table_widths.insert(bi, table.clone());
                }
            }
        }
        if let Some(offsets) = self.table_offsets.get(&self.code_document) {
            for line in &mut self.live.layout.lines {
                if let document::Decoration::TableRow { cols, .. } = line.decoration {
                    let start = self.live.parsed.blocks[line.block].start;
                    let full = self
                        .live
                        .layout
                        .table_widths
                        .get(&line.block)
                        .filter(|w| w.len() == cols)
                        .map(|w| w.iter().sum())
                        .unwrap_or(document::table_column_width(cols, width) * cols as f32);
                    let max = (full - width).max(0.0);
                    line.x -= offsets.get(&start).copied().unwrap_or(0.0).min(max);
                }
            }
        }
        if !crate::ui::editor_preferences::current().code_wrap {
            let width = document::content_width(area);
            for bi in 0..self.live.parsed.blocks.len() {
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
                    .and_then(|offsets| offsets.get(&start))
                    .copied()
                    .unwrap_or(0.0);
                if offset > 0.0 {
                    let offset = offset.min(document::code_horizontal_overflow(
                        &self.live.layout,
                        bi,
                        width,
                    ));
                    let range = self.live.layout.block_line_range(bi);
                    for line in &mut self.live.layout.lines[range] {
                        if line.block == bi
                            && matches!(
                                line.decoration,
                                document::Decoration::CodeBackground { .. }
                            )
                        {
                            line.x -= offset;
                        }
                    }
                }
            }
        }
        if !self.code_document.as_os_str().is_empty() {
            let offset = crate::ui::file_title::body_offset();
            for line in &mut self.live.layout.lines {
                line.y += offset;
            }
            for heading in &mut self.live.layout.headings {
                heading.y += offset;
            }
            self.live.layout.height += offset;
        }
        self.rebuild_background_index();
        self.key = Some(key);
        #[cfg(debug_assertions)]
        if std::env::var_os("MOCHI_PROFILE_LAYOUT").is_some() {
            eprintln!(
                "layout bytes={} blocks={} lines={} parse={:?} layout={:?} post={:?}",
                text.len(),
                self.live.parsed.blocks.len(),
                self.live.layout.lines.len(),
                parse_elapsed,
                layout_elapsed - parse_elapsed,
                profile_start.elapsed() - layout_elapsed
            );
        }
    }
}
