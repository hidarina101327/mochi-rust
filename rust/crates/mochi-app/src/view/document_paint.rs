//! 绘制文档正文、光标、覆盖层和滚动区域。
use super::*;

impl DocPane {
    /// 分片模式的大纲来自全文索引，不依赖尚未挂载片段的排版几何。
    pub fn headings(&self) -> &[document::Heading] {
        self.chunk_plan
            .as_ref()
            .map_or(&self.live.layout.headings, |p| &p.headings)
    }

    pub fn paint(&self, list: &mut DrawList, area: Rect, scroll: f32, p: &Palette) {
        if let Some(name) = self.code_document.file_stem() {
            crate::ui::file_title::paint(
                list,
                area,
                scroll,
                &name.to_string_lossy(),
                self.title_hidden,
                p,
            );
        }
        document::paint_in_indexed(
            list,
            area,
            &self.live.layout,
            scroll,
            self.base_dir.as_deref(),
            p,
            &self.background_headers,
        );
        if let Some(progress) = self.loading.as_ref().filter(|_| self.is_loading()) {
            let status = Rect::new(
                area.left + 12.0,
                area.bottom - 30.0,
                area.right - 12.0,
                area.bottom,
            );
            list.rect(status, p.area_main_default);
            let percent = progress.next_block * 100 / self.layout_blocks.len().max(1);
            list.text(
                status,
                if self.is_chunked() {
                    format!("正在加载当前分片… {percent}%")
                } else {
                    format!("正在排版后续内容… {percent}%")
                },
                TextStyle::Caption,
                p.muted,
            );
        }
    }

    /// 光标与选区叠层。只读场景（没有缓冲区）不调。
    #[allow(clippy::too_many_arguments)]
    pub fn paint_overlay(
        &self,
        list: &mut DrawList,
        area: Rect,
        buffer: &TextBuffer,
        scroll: f32,
        active: bool,
        caret: bool,
        p: &Palette,
    ) {
        live::paint_overlay(list, area, &self.live, buffer, scroll, active, caret, p);
    }

    pub fn max_scroll(&self, area: Rect) -> f32 {
        (self.live.layout.height + self.tail_height - area.height()).max(0.0)
    }

    pub fn caret_rect(&self, area: Rect, buffer: &TextBuffer, scroll: f32) -> Option<Rect> {
        live::caret_in_area(area, &self.live, buffer, scroll)
    }

    /// 包含 `offset` 的可视行的内容起点。续行占位行
    /// 会使用此值而不是文档的全局缩进，这样嵌套
    /// 列表和引用块才能保留实际缩进。
    pub fn line_left(&self, area: Rect, offset: usize) -> Option<f32> {
        let (line_index, _) = self.live.locate(offset)?;
        let line = self.live.layout.lines.get(line_index)?;
        Some(area.left + crate::ui::editor_preferences::current().padding_left + line.x)
    }
}
