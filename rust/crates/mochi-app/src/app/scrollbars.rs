use super::*;
use crate::ui::overlay_scrollbar::{Axis, Bar, Interaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Document,
    OtherDocument,
    Outline,
    Files,
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    fn fixture() -> (App, PathBuf, PathBuf) {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-overlay-scroll-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("scroll.md");
        let text = (0..100)
            .map(|n| format!("## 标题 {n}\n\n正文内容，需要保持稳定的布局。\n\n"))
            .collect::<String>();
        std::fs::write(&file, text).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.settings.set("app.editorLayout.cardEnabled", "false");
        app.load_chrome_settings();
        app.open_workspace(HWND::default(), root.clone(), false)
            .unwrap();
        app.state.view = WorkspaceView::Editor;
        assert!(app.shell.open_file_with_mode(&file, true));
        app.renderer.prepare_snapshot(1400, 900, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        (app, root, file)
    }
    fn bar(app: &App, target: Target) -> Bar {
        app.scrollbars
            .bars
            .iter()
            .find(|(id, _)| *id == target)
            .unwrap()
            .1
    }

    #[test]
    fn document_hover_does_not_reflow_and_drag_does_not_edit_or_select() {
        let (mut app, root, file) = fixture();
        let original = std::fs::read_to_string(&file).unwrap();
        for source in [false, true] {
            if app.shell.active().unwrap().source_mode() != source {
                app.toggle_source_mode();
            }
            app.shell.set_active_scroll(0.0);
            app.paint(HWND::default()).unwrap();
            let area = app.editor_area;
            let thumb = bar(&app, Target::Document).thumb;
            let selection = app.shell.active_buffer_mut().unwrap().selection();
            app.on_mouse_move(area.left + 100.0, area.top + 100.0);
            assert_eq!(app.scrollbars.interaction.hover, None);
            assert!(app.on_mouse_move(thumb.right - 1.0, thumb.top + 2.0));
            app.paint(HWND::default()).unwrap();
            assert_eq!(app.editor_area, area);
            assert_eq!(app.scrollbars.interaction.hover, Some(Target::Document));
            app.on_click(thumb.right - 1.0, thumb.top + 2.0);
            assert!(app.is_dragging());
            app.on_mouse_move(area.right + 100.0, area.bottom + 100.0);
            assert!(app.shell.active_scroll() > 0.0);
            assert_eq!(
                app.shell.active_buffer_mut().unwrap().selection(),
                selection
            );
            app.end_drag_at(area.right + 100.0, area.bottom + 100.0);
            assert!(!app.is_dragging());
            app.focus = Focus::Main;
            app.editor_engaged = true;
            app.on_double_click(thumb.right - 1.0, thumb.top + 2.0);
            assert_eq!(
                app.shell.active_buffer_mut().unwrap().selection(),
                selection
            );
            app.on_pointer_leave();
            assert_eq!(app.scrollbars.interaction.hover, None);
            assert_eq!(app.shell.active_buffer_mut().unwrap().text(), original);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn split_scrollbars_keep_independent_offsets_and_focus() {
        let (mut app, root, file) = fixture();
        app.split_to_right(file);
        app.paint(HWND::default()).unwrap();
        let focused = app.split.right;
        let active = app.shell.active_scroll();
        let b = bar(&app, Target::OtherDocument);
        app.on_click(b.thumb.right - 1.0, b.thumb.top + 2.0);
        assert!(app.scrollbars.interaction.dragging());
        app.end_drag_at(b.track.right, b.track.bottom + 100.0);
        assert!(app.split.scroll > 0.0);
        assert_eq!(app.shell.active_scroll(), active);
        assert_eq!(app.split.right, focused);
        assert!(app.drag.is_none());
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn outline_drag_reaches_last_row_without_jumping_document_and_capture_cancel_stops_drag() {
        let (mut app, root, _) = fixture();
        app.outline_area = Rect::new(1100.0, 100.0, 1380.0, 750.0);
        let p = theme::configured_palette(false);
        app.paint_scrollbars(&p);
        let b = bar(&app, Target::Outline);
        let scroll = app.shell.active_scroll();
        assert!(app.begin_scrollbar_drag(b.thumb.right - 1.0, b.thumb.top + 2.0));
        app.scrollbar_pointer(b.track.right, b.track.bottom + 100.0);
        assert_eq!(
            app.outline_scroll,
            outline::max_scroll(app.outline_area, app.doc.headings().len())
        );
        assert_eq!(app.shell.active_scroll(), scroll);
        assert!(app.cancel_scrollbar_drag());
        let end = app.outline_scroll;
        app.on_mouse_move(b.track.right, b.track.top);
        assert_eq!(app.outline_scroll, end);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn all_outline_placements_have_overlay_tracks_and_short_document_has_none() {
        let (mut app, root, _) = fixture();
        for placement in ["editor-right", "left-sidebar", "ai-sidebar"] {
            app.settings.set("app.outline.placement", placement);
            app.state.ai_panel_open = false;
            app.state.outline_in_ai_sidebar = placement == "ai-sidebar";
            app.state.right_panel = RightPanel::Outline;
            app.outline_left_active = placement == "left-sidebar";
            app.paint(HWND::default()).unwrap();
            let b = bar(&app, Target::Outline);
            assert_eq!(b.hotzone.right, app.outline_area.right);
            assert_eq!(b.hotzone.top, outline::body(app.outline_area).top);
            assert_eq!(b.thumb.width(), 6.0);
        }
        let short = root.join("short.md");
        std::fs::write(&short, "一行内容").unwrap();
        assert!(app.shell.open_file_with_mode(&short, true));
        app.invalidate_main();
        app.paint(HWND::default()).unwrap();
        assert!(!app
            .scrollbars
            .bars
            .iter()
            .any(|(id, _)| matches!(id, Target::Document | Target::Outline)));
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scrolling_chunked_document_keeps_current_chunk_and_full_buffer() {
        let (mut app, root, _) = fixture();
        let source = (0..240)
            .map(|i| format!("## 标题{i}\n\n**词义** 中文😀\n\n"))
            .collect::<String>();
        let path = root.join("chunks.md");
        std::fs::write(&path, &source).unwrap();
        assert!(app.shell.open_file_with_mode(&path, true));
        app.invalidate_main();
        app.paint(HWND::default()).unwrap();
        assert!(app.doc.is_chunked());
        let index = app.doc.chunk_index();
        let count = app.doc.chunk_plan().unwrap().chunks.len();
        assert!(count > 1);
        let b = bar(&app, Target::Document);
        app.on_click(b.thumb.right - 1.0, b.thumb.top + 2.0);
        app.end_drag_at(b.track.right, b.track.bottom + 100.0);
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.doc.chunk_index(), index);
        assert_eq!(app.doc.chunk_plan().unwrap().chunks.len(), count);
        assert!(app.doc.is_chunked());
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), source);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[derive(Default)]
pub(super) struct State {
    pub interaction: Interaction<Target>,
    bars: Vec<(Target, Bar)>,
}

impl App {
    fn scrollbars_enabled(&self) -> bool {
        self.dialog.is_none()
            && self.menu.is_none()
            && self.settings_overlay.is_none()
            && self.template_picker.is_none()
            && self.global_import.is_none()
            && self.automation.panel.is_none()
            && self.object_picker.is_none()
            && self.image_preview.is_none()
            && self.export_form.is_none()
            && self.commands.review.is_none()
            && self.mapped_folder.is_none()
            && self.link_create.is_none()
            && self.search.is_none()
            && self.command.is_none()
            && self.table_picker.is_none()
            && !self.base_detail_open()
    }

    pub(super) fn paint_scrollbars(&mut self, palette: &Palette) {
        let mut bars = std::mem::take(&mut self.scrollbars.bars);
        bars.clear();
        let mut add = |id, area, max, offset| {
            if let Some(bar) = Bar::new(area, Axis::Vertical, max, offset, false) {
                bars.push((id, bar));
            }
        };
        if matches!(self.content(), MainContent::Document | MainContent::Source) {
            let max = if self.content() == MainContent::Source {
                self.source.max_scroll(self.editor_area)
            } else {
                self.doc.max_scroll(self.editor_area)
            };
            let mut track_area = self.editor_area;
            // 浮动控件仍保持原有的点击优先级，只缩短
            // 覆盖式滚动条的轨道，不改变文档视口和换行。
            for control in [
                self.outline_toggle_rect,
                if self.find.is_some() {
                    self.find_layout.card
                } else {
                    Rect::ZERO
                },
            ] {
                let edge = Rect::new(
                    track_area.right - crate::ui::overlay_scrollbar::hotzone_width(),
                    track_area.top,
                    track_area.right,
                    track_area.bottom,
                );
                if !edge.intersect(&control).is_empty() {
                    track_area.top = (control.bottom + 3.0).min(track_area.bottom);
                }
            }
            add(
                Target::Document,
                track_area,
                max,
                self.shell.active_scroll(),
            );
            if self.split.other.is_some() && !self.split.area.is_empty() {
                let max = if self.split.other_source_mode {
                    self.split.source.max_scroll(self.split.area)
                } else {
                    self.split.doc.max_scroll(self.split.area)
                };
                add(
                    Target::OtherDocument,
                    self.split.area,
                    max,
                    self.split.scroll,
                );
            }
        }
        if !self.outline_area.is_empty() && self.link_source().is_some() {
            let body = outline::body(self.outline_area);
            let max = outline::max_scroll(self.outline_area, self.doc.headings().len());
            add(
                Target::Outline,
                body,
                max as f32 * theme::ROW_HEIGHT,
                self.outline_scroll as f32 * theme::ROW_HEIGHT,
            );
        }
        if self.state.sidebar_visible
            && !self.state.view.hides_left_panel()
            && !self.side.layout.content.is_empty()
        {
            add(
                Target::Files,
                self.side.layout.content,
                self.side.layout.max_scroll(),
                self.side.scroll,
            );
        }
        self.scrollbars.bars = bars;
        if self.scrollbars_enabled() {
            if let Some((x, y)) = self.block_pointer {
                self.scrollbars
                    .interaction
                    .pointer(&self.scrollbars.bars, x, y);
            }
            self.scrollbars
                .interaction
                .paint(&mut self.list, &self.scrollbars.bars, palette);
        } else {
            self.scrollbars.interaction.hover = None;
            self.scrollbars.interaction.end();
        }
    }
    fn apply_scrollbar_offset(&mut self, target: Target, value: f32) {
        match target {
            Target::Document => self.shell.set_active_scroll(value),
            Target::OtherDocument => self.split.scroll = value,
            Target::Outline => self.outline_scroll = (value / theme::ROW_HEIGHT).round() as usize,
            Target::Files => self.side.scroll = value,
        }
    }
    pub(super) fn begin_scrollbar_drag(&mut self, x: f32, y: f32) -> bool {
        if !self.scrollbars_enabled() {
            return false;
        }
        let Some((id, value)) = self
            .scrollbars
            .interaction
            .begin(&self.scrollbars.bars, x, y)
        else {
            return false;
        };
        self.block_pointer = Some((x, y));
        self.apply_scrollbar_offset(id, value);
        true
    }
    pub(super) fn scrollbar_pointer(&mut self, x: f32, y: f32) -> bool {
        if !self.scrollbars_enabled() {
            return self.scrollbars.interaction.hover.take().is_some()
                | self.scrollbars.interaction.end();
        }
        let changed = self
            .scrollbars
            .interaction
            .pointer(&self.scrollbars.bars, x, y);
        if let Some((id, value)) = self
            .scrollbars
            .interaction
            .drag_to(&self.scrollbars.bars, x, y)
        {
            self.block_pointer = Some((x, y));
            self.apply_scrollbar_offset(id, value);
            return true;
        }
        changed
    }
    pub(super) fn scrollbar_hit(&self, x: f32, y: f32) -> bool {
        self.scrollbars_enabled()
            && (self.scrollbars.interaction.dragging()
                || Interaction::hit(&self.scrollbars.bars, x, y).is_some())
    }
    pub fn cancel_scrollbar_drag(&mut self) -> bool {
        let mut changed =
            self.notifications.view.scrollbar.end() | self.scrollbars.interaction.end();
        if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
            changed |= state.cancel_scrollbar_drag();
        }
        changed
    }
    pub(super) fn reset_scrollbars(&mut self) {
        self.scrollbars = State::default();
    }
}
