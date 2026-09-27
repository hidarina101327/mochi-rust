//! 计算文档标题、图片和表格等内容的装饰与交互位置。
use super::*;

impl DocPane {
    pub fn hide_title(&mut self, hidden: bool) {
        self.title_hidden = hidden;
    }

    pub fn style_at(&self, offset: usize) -> TextStyle {
        self.live
            .locate(offset)
            .and_then(|(i, _)| self.live.layout.lines.get(i))
            .map(|l| l.style)
            .unwrap_or(TextStyle::Document)
    }

    pub fn title_rect(&self, area: Rect, scroll: f32) -> Rect {
        if self.code_document.as_os_str().is_empty() {
            Rect::ZERO
        } else {
            crate::ui::file_title::rect(area, scroll)
        }
    }

    pub fn set_image_size(&mut self, url: &str, size: (u32, u32)) {
        if let Some(cached) = self.image_sizes.get_mut(url) {
            if *cached != Some(size) {
                *cached = Some(size);
                self.key = None;
            }
        }
    }

    pub fn table_cells(
        &self,
        area: Rect,
        source: &str,
        scroll: f32,
    ) -> Vec<crate::ui::table_edit::Cell> {
        let mut cells = Vec::new();
        let origin = area.left + crate::ui::editor_preferences::current().padding_left;
        for (bi, block) in self.live.parsed.blocks.iter().enumerate() {
            if !matches!(block.block, document::Block::Table { .. }) {
                continue;
            }
            let mut offset = block.start;
            let source_rows = source
                .get(block.start..block.end)
                .unwrap_or("")
                .split_inclusive('\n')
                .enumerate()
                .filter_map(|(i, line)| {
                    let start = offset;
                    offset += line.len();
                    (i != 1).then_some((line, start))
                })
                .collect::<Vec<_>>();
            for (ri, line) in self
                .live
                .layout
                .lines_for_block(bi)
                .iter()
                .filter(|l| {
                    l.block == bi && matches!(l.decoration, document::Decoration::TableRow { .. })
                })
                .enumerate()
            {
                let Some((raw, start)) = source_rows.get(ri) else {
                    continue;
                };
                let document::Decoration::TableRow { cols, .. } = line.decoration else {
                    continue;
                };
                let widths = self.widths_for(block.start, cols, document::content_width(area));
                let mut left = origin + line.x;
                for (col, range) in crate::ui::table_edit::ranges(raw, *start)
                    .into_iter()
                    .enumerate()
                {
                    let width = widths
                        .get(col)
                        .copied()
                        .unwrap_or(document::TABLE_CELL_MIN_WIDTH);
                    let top = area.top + line.y - scroll;
                    cells.push(crate::ui::table_edit::Cell {
                        range,
                        rect: Rect::new(left, top, left + width, top + line.height),
                    });
                    left += width;
                }
            }
        }
        cells
    }

    pub fn scroll_table(&mut self, area: Rect, scroll: f32, x: f32, y: f32, delta: f32) -> bool {
        if !area.contains(x, y) {
            return false;
        }
        let y = y - area.top + scroll;
        let Some(line) = self.live.layout.lines.iter().find(|l| {
            l.y <= y
                && y < l.y + l.height
                && matches!(l.decoration, document::Decoration::TableRow { .. })
        }) else {
            return false;
        };
        let document::Decoration::TableRow { cols, .. } = line.decoration else {
            return false;
        };
        let width = document::content_width(area);
        let start = self.live.parsed.blocks[line.block].start;
        let max = (self.widths_for(start, cols, width).iter().sum::<f32>() - width).max(0.0);
        let value = self
            .table_offsets
            .entry(self.code_document.clone())
            .or_default()
            .entry(start)
            .or_default();
        *value = (*value - delta).clamp(0.0, max);
        self.invalidate();
        true
    }

    /// Shift+滚轮在未换行的长代码块上横向浏览。
    pub fn scroll_code(&mut self, area: Rect, scroll: f32, x: f32, y: f32, delta: f32) -> bool {
        if crate::ui::editor_preferences::current().code_wrap || !area.contains(x, y) {
            return false;
        }
        let y = y - area.top + scroll;
        let Some(line) = self.live.layout.lines.iter().find(|line| {
            line.y <= y
                && y < line.y + line.height
                && matches!(line.decoration, document::Decoration::CodeBackground { .. })
        }) else {
            return false;
        };
        let max = document::code_horizontal_overflow(
            &self.live.layout,
            line.block,
            document::content_width(area),
        );
        if max <= 0.0 {
            return false;
        }
        let start = self.live.parsed.blocks[line.block].start;
        let value = self
            .code_offsets
            .entry(self.code_document.clone())
            .or_default()
            .entry(start)
            .or_default();
        *value = (*value - delta).clamp(0.0, max);
        self.invalidate();
        true
    }

    pub fn content_height(&self) -> f32 {
        self.live.layout.height
    }

    pub fn block_handle(
        &self,
        area: Rect,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> Option<(Rect, usize, bool)> {
        if !area.contains(x, y) {
            return None;
        }
        let local_y = y - area.top + scroll;
        // 分隔线只绘制为 1 像素高；菜单仍需要有足够大的点击区域，
        // 应使用两侧留白，但不能改变文档布局。
        let first_candidate = self
            .live
            .layout
            .lines
            .partition_point(|line| line.y + line.height + 7.0 <= local_y);
        let line = self.live.layout.lines[first_candidate..]
            .iter()
            .take_while(|line| line.y - 7.0 <= local_y)
            .find(|l| {
                let padding = if matches!(l.decoration, document::Decoration::Divider) {
                    7.0
                } else {
                    0.0
                };
                l.y - padding <= local_y && local_y < l.y + l.height + padding
            })?;
        let first = self.live.layout.lines_for_block(line.block).first()?;
        let left =
            area.left + crate::ui::editor_preferences::current().padding_left + first.x - 23.0;
        let top = area.top + first.y - scroll + 3.0;
        let block = &self.live.parsed.blocks[line.block];
        Some((
            Rect::from_size(left.max(area.left), top, 21.0, 24.0),
            block.start,
            matches!(block.block, document::Block::Blank),
        ))
    }

    pub fn image_at(&self, area: Rect, scroll: f32, x: f32, y: f32) -> Option<usize> {
        let origin = area.left + crate::ui::editor_preferences::current().padding_left;
        let local_y = y - area.top + scroll;
        let line = self.live.layout.lines.iter().find(|l| {
            l.decoration == document::Decoration::Image
                && l.y <= local_y
                && local_y < l.y + l.height
                && x >= origin + l.x
                && x < origin
                    + l.x
                    + l.runs
                        .get(2)
                        .and_then(|r| r.text.parse::<f32>().ok())
                        .unwrap_or(document::content_width(area))
        })?;
        Some(self.live.parsed.blocks[line.block].start)
    }

    pub fn container_at(&self, area: Rect, scroll: f32, x: f32, y: f32) -> Option<(usize, bool)> {
        let origin = area.left + crate::ui::editor_preferences::current().padding_left;
        let local_y = y - area.top + scroll;
        let line = self.live.layout.lines.iter().find(|l| {
            matches!(l.decoration, document::Decoration::Container { .. })
                && l.y <= local_y
                && local_y < l.y + l.height
                && x >= origin + l.x
                && x <= origin + document::content_width(area) - l.x
        })?;
        let toggle = matches!(
            line.decoration,
            document::Decoration::Container { details: true, .. }
        ) && x < origin + document::content_width(area) - line.x - 34.0;
        Some((self.live.parsed.blocks[line.block].start, toggle))
    }

    pub fn set_tail_height(&mut self, height: f32) {
        self.tail_height = height;
    }

    pub fn set_code_document(&mut self, path: &Path) {
        if self.code_document != path {
            self.remember_chunk();
            self.code_document = path.to_path_buf();
            self.code_state = Default::default();
            self.layout_cache.clear();
            self.mapping_cache.clear();
            self.live = Default::default();
            self.complete = false;
            self.chunk_plan = None;
            self.chunk_index = 0;
            self.layout_blocks = 0..0;
            self.background_headers.clear();
            self.invalidate();
        }
    }

    /// 后台标签页保留可复用的布局，但标签页关闭后，
    /// 应释放最后一个文档，不要在查看其他内容时继续占用。
    pub fn retain_open_documents(&mut self, paths: &[&Path]) {
        self.chunk_positions
            .retain(|path, _| paths.contains(&path.as_path()));
        self.full_documents
            .retain(|path| paths.contains(&path.as_path()));
        self.table_offsets
            .retain(|path, _| paths.contains(&path.as_path()));
        self.table_widths
            .retain(|path, _| paths.contains(&path.as_path()));
        self.code_offsets
            .retain(|path, _| paths.contains(&path.as_path()));
        if !self.code_document.as_os_str().is_empty()
            && !paths.contains(&self.code_document.as_path())
        {
            self.code_document = PathBuf::new();
            self.code_state = Default::default();
            self.live = Default::default();
            self.complete = false;
            self.chunk_plan = None;
            self.layout_blocks = 0..0;
            self.background_headers.clear();
            self.loading = None;
            self.loading_caret = None;
            self.layout_cache.clear();
            self.mapping_cache.clear();
            self.image_sizes.clear();
            self.base_dir = None;
            self.key = None;
            self.desired_x = 0.0;
        }
    }

    pub fn toggle_code(&mut self, buffer: &mut TextBuffer, start: usize) {
        let mut attrs = crate::ui::code_blocks::metadata(buffer.text(), start).1;
        attrs.collapsed = !attrs.collapsed;
        crate::ui::code_blocks::update(buffer, start, attrs);
        self.invalidate();
    }

    pub fn set_code_title(&mut self, buffer: &mut TextBuffer, start: usize, title: String) {
        let mut attrs = crate::ui::code_blocks::metadata(buffer.text(), start).1;
        attrs.title = title;
        crate::ui::code_blocks::update(buffer, start, attrs);
        self.invalidate();
    }

    pub fn code_title(&self, start: usize) -> String {
        self.code_state
            .blocks
            .get(&start)
            .map(|s| s.title.clone())
            .unwrap_or_default()
    }

    pub fn code_header_at(
        &self,
        area: Rect,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> Option<crate::ui::code_blocks::Hit> {
        let origin_x = area.left + crate::ui::editor_preferences::current().padding_left;
        let local_y = y - area.top + scroll;
        let line = self.live.layout.lines.get(self.live.line_at(local_y)?)?;
        if !matches!(line.decoration, document::Decoration::CodeHeader { .. })
            || local_y < line.y
            || local_y >= line.y + line.height
        {
            return None;
        }
        let header = Rect::new(
            origin_x,
            area.top + line.y - scroll,
            origin_x + document::card_width(area),
            area.top + line.y - scroll + line.height,
        );
        crate::ui::code_blocks::hit(header, self.live.parsed.blocks[line.block].start, x, y)
    }

    /// 让排版失效。换标签、改宽度、切模式、等长替换之后都要调。
    pub fn invalidate(&mut self) {
        self.key = None;
        self.loading = None;
        self.loading_caret = None;
    }

    /// 文档所在目录。换标签时调；图片尺寸缓存一并清掉（同名相对路径在别的目录指向别的文件）。
    pub fn set_base_dir(&mut self, dir: Option<&Path>) {
        if self.base_dir.as_deref() != dir {
            self.base_dir = dir.map(Path::to_path_buf);
            self.image_sizes.clear();
            self.key = None;
        }
    }
}
