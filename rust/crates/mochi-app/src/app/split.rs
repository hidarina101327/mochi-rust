//! 双编辑面：Shell 的缓冲区仍只有一份，切焦点只交换视图缓存和独立滚动。
use super::*;

/// 内部窗格分隔线绘制为 6 DIP，另有更宽的不可见拖动区域
/// 位于分隔线两侧。将两者分开，避免扩大点击区域时
/// 改变两个窗格的宽度或遮挡其中的控件。
const DIVIDER_VISUAL_HALF_WIDTH: f32 = 3.0;
pub(super) const DIVIDER_HIT_HALF_WIDTH: f32 = 8.0;

pub(super) fn divider_geometry(area: Rect, ratio: f32) -> (Rect, Rect) {
    let x = area.left + area.width() * normalized_ratio(ratio);
    (
        Rect::new(
            x - DIVIDER_VISUAL_HALF_WIDTH,
            area.top,
            x + DIVIDER_VISUAL_HALF_WIDTH,
            area.bottom,
        ),
        Rect::new(
            x - DIVIDER_HIT_HALF_WIDTH,
            area.top,
            x + DIVIDER_HIT_HALF_WIDTH,
            area.bottom,
        ),
    )
}

fn normalized_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() && ratio > 0.0 {
        ratio.clamp(0.2, 0.8)
    } else {
        0.5
    }
}

fn selection(buffer: &crate::ui::editor::TextBuffer) -> (usize, usize) {
    let (start, end) = buffer.selection();
    (
        if buffer.cursor() == start { end } else { start },
        buffer.cursor(),
    )
}

#[derive(Default)]
pub(super) struct State {
    pub other: Option<PathBuf>,
    pub right: bool,
    pub ratio: f32,
    pub full: Rect,
    pub divider: Rect,
    /// 内部窗格分隔线的扩展点击区域；`divider` 仍表示
    /// 绘制出来的 6 DIP 细线。`App::on_click` 和 `cursor_for` 会使用此区域。
    pub divider_hit: Rect,
    pub close: Rect,
    pub area: Rect,
    pub toolbar: Rect,
    pub header: Rect,
    pub active_header: Rect,
    pub pane_area: Rect,
    pub scroll: f32,
    pub doc: DocPane,
    pub source: SourcePane,
    pub other_source_mode: bool,
    other_selection: Option<(usize, usize)>,
    chunk_scroll_to_end: bool,
    active_path: Option<PathBuf>,
}
impl App {
    pub(super) fn finish_other_chunk_navigation(&mut self) {
        if self.split.chunk_scroll_to_end && !self.split.doc.is_loading() {
            self.split.scroll = self.split.doc.max_scroll(self.split.area);
            self.split.chunk_scroll_to_end = false;
        }
    }

    pub(super) fn page_other_document_at_edge(&mut self, step: f32) -> bool {
        if self.split.other_source_mode
            || !self.split.doc.is_chunked()
            || self.split.doc.is_loading()
            || self
                .chunk_edge_until
                .is_some_and(|t| std::time::Instant::now() < t)
        {
            return false;
        }
        let previous = step > 0.0 && self.split.scroll <= 2.0;
        let next =
            step < 0.0 && self.split.scroll >= self.split.doc.max_scroll(self.split.area) - 2.0;
        let index = if previous {
            self.split.doc.chunk_index().checked_sub(1)
        } else if next {
            self.split.doc.chunk_index().checked_add(1)
        } else {
            None
        };
        let Some(index) = index else {
            return false;
        };
        let Some(offset) = self.split.doc.chunk_offset(index, previous) else {
            return false;
        };
        if !self.split.doc.select_chunk(self.split.area, index) {
            return false;
        }
        self.split.other_selection = Some((offset, offset));
        self.split.scroll = 0.0;
        self.split.chunk_scroll_to_end = previous;
        self.finish_other_chunk_navigation();
        self.chunk_edge_until =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(400));
        true
    }

    pub(super) fn remember_split_active(&mut self) {
        if self.split.other.is_some() {
            self.split.active_path = self.active_file_path();
        }
    }

    fn clear_split_state(&mut self) {
        let ratio = normalized_ratio(self.split.ratio);
        self.split = State {
            ratio,
            ..State::default()
        };
        self.shell.set_protected_view_path(None);
    }

    /// 标签页发生变动后调用。不要在此方法中调用 sync_state。
    pub(super) fn reconcile_split(&mut self) {
        let Some(other) = self.split.other.clone() else {
            return;
        };
        let target = self.shell.text_tab_index(&other);
        let Some(target) = target else {
            self.clear_split_state();
            return;
        };
        let active_was_closed = self.split.active_path.as_ref().is_some_and(|old| {
            !self
                .shell
                .tabs()
                .iter()
                .any(|tab| tab.path() == Some(old.as_path()))
        });
        if active_was_closed {
            self.shell.select_tab(target);
            if let Some(TabKind::File { source_mode, .. }) =
                self.shell.active_mut().map(|tab| &mut tab.kind)
            {
                *source_mode = self.split.other_source_mode;
            }
            if let (Some((anchor, cursor)), Some(buffer)) =
                (self.split.other_selection, self.shell.active_buffer_mut())
            {
                buffer.set_cursor(anchor, false);
                buffer.set_cursor(cursor, true);
            }
            self.shell.set_active_scroll(self.split.scroll);
            std::mem::swap(&mut self.doc, &mut self.split.doc);
            std::mem::swap(&mut self.source, &mut self.split.source);
            self.clear_split_state();
            self.editor_engaged = false;
            self.links_layout = backlinks::Layout::default();
        } else {
            self.remember_split_active();
        }
    }

    pub(super) fn split_to_right(&mut self, path: PathBuf) {
        let was_active = self.active_file_path().as_deref() == Some(path.as_path());
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        let path = if was_active {
            self.active_file_path().unwrap_or(path)
        } else {
            path
        };
        if !self
            .shell
            .tabs()
            .iter()
            .any(|t| t.path() == Some(path.as_path()) && t.buffer().is_some())
        {
            return;
        }
        // 重复拆分时会替换右侧窗格，保留左侧窗格。
        let existed = self.split.other.is_some();
        if existed && self.split.right && !self.focus_other_editor() {
            return;
        }
        if self.shell.active().is_none_or(|tab| tab.buffer().is_none()) {
            let index = self
                .shell
                .tabs()
                .iter()
                .position(|tab| tab.path() == Some(path.as_path()))
                .unwrap();
            self.shell.select_tab(index);
            self.doc.invalidate();
            self.source.invalidate();
        }
        let ratio = if existed {
            normalized_ratio(self.split.ratio)
        } else {
            normalized_ratio(
                app_settings::descriptor("editorLayout.splitRatio")
                    .and_then(|d| match self.app_settings.read(d) {
                        SettingValue::Number(n) => Some(n as f32),
                        _ => None,
                    })
                    .unwrap_or(0.5),
            )
        };
        let target = self
            .shell
            .tabs()
            .iter()
            .find(|tab| tab.path() == Some(path.as_path()))
            .unwrap();
        self.split = State {
            other: Some(path),
            ratio,
            scroll: target.scroll,
            other_selection: target.buffer().map(selection),
            other_source_mode: target.source_mode(),
            active_path: self.active_file_path(),
            ..State::default()
        };
        self.state.view = WorkspaceView::Editor;
        self.focus_other_editor();
        self.invalidate_main();
    }
    pub(super) fn focus_other_editor(&mut self) -> bool {
        if !self.commit_title() || !self.commit_table_cell() {
            return false;
        }
        let Some(next) = self.split.other.clone() else {
            return false;
        };
        let Some(old) = self.active_file_path() else {
            return false;
        };
        let Some(index) = self.shell.text_tab_index(&next) else {
            return false;
        };
        let reveal_page_end = std::mem::take(&mut self.split.chunk_scroll_to_end);
        let scroll = self.shell.active_scroll();
        let previous_mode = self.shell.active().is_some_and(|tab| tab.source_mode());
        let previous = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .map(selection);
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.cancel_composition();
        }
        self.shell.select_tab(index);
        if let Some(TabKind::File { source_mode, .. }) =
            self.shell.active_mut().map(|tab| &mut tab.kind)
        {
            *source_mode = self.split.other_source_mode;
        }
        self.split.other_source_mode = previous_mode;
        if let (Some((anchor, cursor)), Some(buffer)) =
            (self.split.other_selection, self.shell.active_buffer_mut())
        {
            buffer.set_cursor(anchor, false);
            buffer.set_cursor(cursor, true);
        }
        self.split.other_selection = previous;
        self.split.pane_area = Rect::new(
            self.split.active_header.left,
            self.split.active_header.top,
            self.split.active_header.right,
            self.split.full.bottom,
        );
        self.shell.set_active_scroll(self.split.scroll);
        self.split.other = Some(old);
        self.split.scroll = scroll;
        self.split.right = !self.split.right;
        std::mem::swap(&mut self.doc, &mut self.split.doc);
        std::mem::swap(&mut self.source, &mut self.split.source);
        std::mem::swap(&mut self.editor_area, &mut self.split.area);
        std::mem::swap(&mut self.toolbar_area, &mut self.split.toolbar);
        std::mem::swap(&mut self.split.active_header, &mut self.split.header);
        self.links_layout = backlinks::Layout::default();
        self.outline_area = Rect::ZERO;
        self.find = None;
        self.editor_ai.invalidate();
        self.toolbar_hover = None;
        self.editor_engaged = false;
        self.focus = Focus::Main;
        self.remember_split_active();
        self.sync_state();
        if reveal_page_end {
            self.after_doc_selection_change(true);
        }
        true
    }

    pub(super) fn close_split_view(&mut self) -> bool {
        if self.split.other.is_none() {
            return true;
        }
        if !self.commit_title() || !self.commit_table_cell() {
            return false;
        }
        if self.split.right && !self.focus_other_editor() {
            return false;
        }
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.cancel_composition();
        }
        self.clear_split_state();
        self.focus = Focus::Main;
        self.editor_engaged = false;
        self.state.view = WorkspaceView::Editor;
        self.invalidate_main();
        self.sync_state();
        true
    }
    pub(super) fn paint_split(&mut self, area: Rect, p: &Palette) -> Rect {
        self.split.area = Rect::ZERO;
        self.split.toolbar = Rect::ZERO;
        self.split.divider = Rect::ZERO;
        self.split.divider_hit = Rect::ZERO;
        self.split.close = Rect::ZERO;
        self.split.header = Rect::ZERO;
        self.split.active_header = Rect::ZERO;
        self.split.pane_area = Rect::ZERO;
        self.split.full = Rect::ZERO;
        if area.width() < 64.0
            || area.height() < 40.0
            || self.state.view != WorkspaceView::Editor
            || !matches!(self.content(), MainContent::Document | MainContent::Source)
        {
            return area;
        }
        let Some(path) = self.split.other.clone() else {
            return area;
        };
        let Some(index) = self.shell.text_tab_index(&path) else {
            self.clear_split_state();
            return area;
        };
        self.split.full = area;
        let ratio = normalized_ratio(self.split.ratio);
        let (divider, divider_hit) = divider_geometry(area, ratio);
        self.split.divider = divider;
        self.split.divider_hit = divider_hit;
        self.list.rect(self.split.divider, p.border);
        let left = Rect::new(area.left, area.top, divider.left, area.bottom);
        let right = Rect::new(divider.right, area.top, area.right, area.bottom);
        let (mut active, mut other) = if self.split.right {
            (right, left)
        } else {
            (left, right)
        };
        let active_header = Rect::new(active.left, active.top, active.right, active.top + 28.0);
        self.split.active_header = active_header;
        self.split.pane_area = other;
        self.list.rect(active_header, p.surface_muted);
        self.list
            .hline(active.left, active.right, active.top, p.accent);
        self.list.text(
            Rect::new(
                active.left + 12.0,
                active.top,
                active.right - if self.split.right { 36.0 } else { 12.0 },
                active.top + 28.0,
            ),
            crate::ui::text::ellipsize(
                self.shell.active().map(|t| t.title.as_str()).unwrap_or(""),
                TextStyle::Caption,
                (active.width() - if self.split.right { 48.0 } else { 24.0 }).max(0.0),
            ),
            TextStyle::Caption,
            p.accent,
        );
        self.split.header = Rect::new(other.left, other.top, other.right, other.top + 28.0);
        self.list.rect(self.split.header, p.surface_muted);
        let tab = &self.shell.tabs()[index];
        self.list.text(
            Rect::new(
                other.left + 12.0,
                other.top,
                other.right - 36.0,
                other.top + 28.0,
            ),
            crate::ui::text::ellipsize(
                &tab.title,
                TextStyle::Caption,
                (other.width() - 48.0).max(0.0),
            ),
            TextStyle::Caption,
            p.muted,
        );
        self.split.close = Rect::new(
            right.right - 28.0,
            right.top + 2.0,
            right.right - 4.0,
            right.top + 26.0,
        );
        self.list
            .icon_centered(self.split.close, Icon::X, 14.0, p.muted);
        active.top += 28.0;
        other.top += 28.0;
        let buffer = tab.buffer().unwrap();
        if self.split.other_source_mode {
            self.split.source.ensure(other, index, buffer);
            self.split.scroll = self.split.scroll.min(self.split.source.max_scroll(other));
            self.split
                .source
                .paint(&mut self.list, other, buffer, self.split.scroll, false, p);
        } else {
            let mut state = toolbar::State::at(buffer.text(), buffer.cursor());
            state.hover = None;
            let lay = toolbar::layout_mode(
                Rect::new(
                    other.left,
                    other.top,
                    other.right,
                    other.top + toolbar::HEIGHT,
                ),
                state.heading_level,
                crate::ui::settings_values::boolean("editor.simpleDocumentMode", true),
            );
            let bar = Rect::new(other.left, other.top, other.right, other.top + lay.height);
            self.split.toolbar = bar;
            self.list.push_clip(bar);
            toolbar::paint(&mut self.list, bar, &lay, &state, p);
            self.list.pop_clip();
            other.top = bar.bottom;
            if crate::ui::settings_values::boolean("editorLayout.cardEnabled", false) {
                let margin = crate::ui::settings_values::number("editorLayout.cardMargin", 32.0)
                    .min(other.width() / 4.0)
                    .min(other.height() / 4.0);
                other = Rect::new(
                    other.left + margin,
                    other.top + margin,
                    other.right - margin,
                    other.bottom - margin,
                );
                self.list.rounded_rect_alpha(
                    other,
                    8.0,
                    if self.state.dark { 0x1e1e1e } else { 0xffffff },
                    crate::ui::settings_values::number("background.uiOpacity", 85.0) / 100.0,
                );
            }
            other = crate::ui::editor_preferences::current().aligned_area(other);
            self.split.doc.set_base_dir(path.parent());
            if self.hwnd_raw != 0 {
                self.split.doc.set_progressive(true);
            }
            self.split.doc.set_code_document(&path);
            self.split.doc.hide_title(false);
            self.split.doc.set_tail_height(0.0);
            self.split
                .doc
                .ensure_buffer(other, Some(index), buffer, None);
            if let Some(plan) = self.split.doc.chunk_plan() {
                let bar = Rect::new(
                    other.left,
                    other.top,
                    other.right,
                    (other.top + crate::ui::large_document::BAR_HEIGHT).min(other.bottom),
                );
                crate::ui::large_document::paint(
                    &mut self.list,
                    bar,
                    plan,
                    self.split.doc.chunk_index(),
                    self.split.doc.is_chunked(),
                    p,
                );
                other.top = bar.bottom;
            }
            self.split.scroll = self.split.scroll.min(self.split.doc.max_scroll(other));
            self.split
                .doc
                .paint(&mut self.list, other, self.split.scroll, p);
        }
        self.split.area = other;
        active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (App, PathBuf, Vec<PathBuf>) {
        let root = std::env::temp_dir().join(format!(
            "mochi-pane-state-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        let files = ["a.md", "b.md", "c.md"]
            .into_iter()
            .map(|name| root.join(name))
            .collect::<Vec<_>>();
        for path in &files {
            std::fs::write(path, "甲乙丙").unwrap();
            assert!(app.shell.open_file_with_mode(path, true));
        }
        app.shell.select_tab(0);
        (app, root, files)
    }

    #[test]
    fn same_document_keeps_two_selections_and_one_undo_history() {
        let (mut app, root, files) = fixture();
        app.shell.active_buffer_mut().unwrap().set_cursor(3, false);
        app.split_to_right(files[0].clone());
        assert!(app.split.right);
        app.shell.active_buffer_mut().unwrap().set_cursor(6, false);
        assert!(app.focus_other_editor());
        assert!(!app.split.right);
        assert_eq!(app.shell.active_buffer_mut().unwrap().cursor(), 3);
        let buffer = app.shell.active_buffer_mut().unwrap();
        buffer.set_cursor(0, false);
        buffer.set_cursor(3, true);
        assert!(app.focus_other_editor());
        assert_eq!(app.shell.active_buffer_mut().unwrap().cursor(), 6);
        app.shell.active_buffer_mut().unwrap().insert("新");
        assert!(app.focus_other_editor());
        let buffer = app.shell.active_buffer_mut().unwrap();
        assert_eq!(buffer.selected_text(), "甲");
        assert_eq!(buffer.text(), "甲乙新丙");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "甲乙丙");
        assert_eq!(
            app.shell
                .tabs()
                .iter()
                .filter(|tab| tab.path() == Some(files[0].as_path()))
                .count(),
            1
        );
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn same_document_keeps_source_mode_per_pane() {
        let (mut app, root, files) = fixture();
        app.split_to_right(files[0].clone());
        if let TabKind::File { source_mode, .. } = &mut app.shell.active_mut().unwrap().kind {
            *source_mode = true;
        }
        assert!(app.focus_other_editor());
        assert!(!app.shell.active().unwrap().source_mode());
        assert!(app.split.other_source_mode);
        assert!(app.focus_other_editor());
        assert!(app.shell.active().unwrap().source_mode());
        assert!(!app.split.other_source_mode);
        assert!(app.close_split_view());
        assert!(!app.shell.active().unwrap().source_mode());
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resplitting_replaces_right_and_closing_tab_returns_to_left() {
        let (mut app, root, files) = fixture();
        app.split_to_right(files[1].clone());
        app.split_to_right(files[2].clone());
        assert_eq!(app.active_file_path(), Some(files[2].clone()));
        assert_eq!(app.split.other, Some(files[0].clone()));
        assert!(app.split.right);
        assert_eq!(app.shell.tabs().len(), 3);
        app.shell.close_tab(app.shell.active_tab().unwrap());
        app.sync_state();
        assert_eq!(app.active_file_path(), Some(files[0].clone()));
        assert!(app.split.other.is_none());
        assert!(app.split.area.is_empty());
        assert!(app.split.close.is_empty());
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dashboard_replacement_updates_only_the_focused_pane() {
        let (mut app, root, files) = fixture();
        app.settings.set("app.tabs.openFileBehavior", "replace");
        app.load_chrome_settings();
        app.split_to_right(files[1].clone());
        let replacement = root.join("dashboard.md");
        std::fs::write(&replacement, "新打开的文档").unwrap();
        app.open_dashboard_document(&replacement);
        assert_eq!(app.active_file_path(), Some(replacement));
        assert_eq!(app.split.other, Some(files[0].clone()));
        assert!(app.split.right);
        assert!(!app
            .shell
            .tabs()
            .iter()
            .any(|tab| tab.path() == Some(files[1].as_path())));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn divider_clamps_persisted_invalid_values_without_nan_geometry() {
        let area = Rect::new(100.0, 20.0, 1100.0, 620.0);
        assert_eq!(
            divider_geometry(area, f32::NAN),
            divider_geometry(area, 0.5)
        );
        assert_eq!(divider_geometry(area, 0.01), divider_geometry(area, 0.2));
        assert_eq!(divider_geometry(area, 3.0), divider_geometry(area, 0.8));
    }

    #[test]
    fn internal_divider_hit_area_is_wider_without_changing_pane_edges() {
        let area = Rect::new(100.0, 20.0, 1100.0, 620.0);
        let (line, hit) = divider_geometry(area, 0.5);

        assert_eq!(line, Rect::new(597.0, 20.0, 603.0, 620.0));
        assert_eq!(hit, Rect::new(592.0, 20.0, 608.0, 620.0));
        assert!(hit.contains(592.1, 300.0));
        assert!(hit.contains(607.9, 300.0));
        assert!(!hit.contains(591.9, 300.0));
        assert!(!hit.contains(608.0, 300.0));
    }
}
