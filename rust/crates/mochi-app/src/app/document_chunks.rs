//! 内容分页继续使用现有的完整源码缓冲区，以及原有的撤销和保存流程。
use super::*;
use crate::ui::large_document::{self, Action};

impl App {
    fn document_chunk_bar(&self) -> Option<Rect> {
        self.doc.chunk_plan()?;
        Some(Rect::new(
            self.editor_area.left,
            self.editor_area.top - large_document::BAR_HEIGHT,
            self.editor_area.right,
            self.editor_area.top,
        ))
    }

    pub(super) fn click_document_chunk_bar(&mut self, x: f32, y: f32) -> bool {
        let Some(area) = self.document_chunk_bar().filter(|r| r.contains(x, y)) else {
            return false;
        };
        if let Some((_, action)) = large_document::buttons(area)
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
        {
            self.change_document_chunk(action, false);
        }
        true
    }

    pub(super) fn change_document_chunk(&mut self, action: Action, align_end: bool) -> bool {
        if !self.commit_title() || !self.commit_table_cell() {
            return false;
        }
        if self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .is_some_and(|b| b.composition().is_some())
        {
            self.state.status_text = "请先完成输入法输入，再切换分段".into();
            return false;
        }
        if action == Action::Toggle {
            if self.doc.chunk_plan().is_none() {
                return false;
            }
            self.doc.toggle_chunking();
            self.after_doc_selection_change(true);
            return true;
        }
        if !self.doc.is_chunked() {
            return false;
        }
        let index = match action {
            Action::Previous => self.doc.chunk_index().checked_sub(1),
            Action::Next => self.doc.chunk_index().checked_add(1),
            Action::Toggle => None,
        };
        let Some(index) = index else {
            return false;
        };
        let Some(offset) = self.doc.chunk_offset(index, align_end) else {
            return false;
        };
        if !self.doc.select_chunk(self.editor_area, index) {
            return false;
        }
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(offset, false);
        }
        self.shell.set_active_scroll(0.0);
        if align_end {
            self.after_doc_selection_change(true);
        }
        self.chunk_edge_until =
            Some(std::time::Instant::now() + std::time::Duration::from_millis(400));
        true
    }

    pub(super) fn page_document_at_edge(&mut self, step: f32) -> bool {
        if !self.doc.is_chunked()
            || self.doc.is_loading()
            || self
                .chunk_edge_until
                .is_some_and(|until| std::time::Instant::now() < until)
        {
            return false;
        }
        let scroll = self.shell.active_scroll();
        let max = self.doc.max_scroll(self.editor_area);
        if step < 0.0 && scroll >= max - 2.0 {
            self.change_document_chunk(Action::Next, false)
        } else if step > 0.0 && scroll <= 2.0 {
            self.change_document_chunk(Action::Previous, true)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn with_app(test: impl FnOnce(&mut App, &Path, &str)) {
        let source: String = (0..220)
            .map(|i| format!("## 标题 {i}\r\n\r\n词义{i} **粗体😀**\r\n\r\n"))
            .collect();
        let root = std::env::temp_dir().join(format!(
            "mochi-content-pages-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let path = root.join("pages.md");
            std::fs::write(&path, &source).unwrap();
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            assert!(app.shell.open_file(&path));
            app.state.view = WorkspaceView::Editor;
            app.sync_state();
            app.focus = Focus::Main;
            app.editor_engaged = true;
            app.editor_area = Rect::new(0.0, 84.0, 800.0, 700.0);
            app.doc.set_code_document(&path);
            app.after_doc_selection_change(true);
            finish(&mut app);
            test(&mut app, &path, &source);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    fn finish(app: &mut App) {
        for _ in 0..1000 {
            if !app.doc.is_loading() {
                return;
            }
            app.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
        }
        panic!("current page did not finish");
    }
    #[test]
    fn content_page_controls_edges_and_outline_navigation_use_the_full_buffer() {
        with_app(|app, _, source| {
            assert!(app.doc.is_chunked());
            let bar = app.document_chunk_bar().unwrap();
            let next = large_document::buttons(bar)[2].0;
            assert!(app.click_document_chunk_bar(next.left + 2.0, next.top + 2.0));
            finish(app);
            assert_eq!(app.doc.chunk_index(), 1);
            assert!(
                !app.page_document_at_edge(60.0),
                "cooldown prevents bouncing back"
            );
            app.chunk_edge_until = None;
            app.shell.set_active_scroll(0.0);
            assert!(app.page_document_at_edge(60.0));
            finish(app);
            assert_eq!(app.doc.chunk_index(), 0);
            app.outline_area = Rect::new(850.0, 0.0, 1100.0, 700.0);
            app.outline_scroll = outline::max_scroll(app.outline_area, app.doc.headings().len());
            // 在实际滚动到底时，点击当前可见区域的最后一行。
            let y = outline::body(app.outline_area).top
                + (219 - app.outline_scroll) as f32 * theme::ROW_HEIGHT
                + theme::ROW_HEIGHT / 2.0;
            app.on_outline_click(900.0, y);
            finish(app);
            assert_eq!(
                app.doc.chunk_index() + 1,
                app.doc.chunk_plan().unwrap().chunks.len()
            );
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().cursor(),
                app.doc.heading_offset(219).unwrap()
            );
            let caret = app
                .doc
                .caret_rect(
                    app.editor_area,
                    app.shell.active().unwrap().buffer().unwrap(),
                    app.shell.active_scroll(),
                )
                .unwrap();
            assert!(
                caret.top >= app.editor_area.top && caret.bottom <= app.editor_area.bottom + 1.0
            );
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), source);
        });
    }

    #[test]
    fn find_and_split_panes_navigate_unmounted_content_without_mutating_it() {
        with_app(|app, path, source| {
            let mut find = findbar::State::new(false);
            find.query.set_text("词义219");
            app.find = Some(find);
            app.focus = Focus::FindQuery;
            app.find_query_changed();
            finish(app);
            assert_eq!(app.find.as_ref().unwrap().matches.len(), 1);
            assert_eq!(
                app.shell
                    .active()
                    .unwrap()
                    .buffer()
                    .unwrap()
                    .selected_text(),
                "词义219"
            );
            assert_eq!(
                app.doc.chunk_index() + 1,
                app.doc.chunk_plan().unwrap().chunks.len()
            );
            assert!(app.change_document_chunk(Action::Previous, false));
            finish(app);
            let other_index = app.doc.chunk_index();
            app.split_to_right(path.to_path_buf());
            app.doc.set_code_document(path);
            app.editor_area = Rect::new(820.0, 84.0, 1600.0, 700.0);
            app.after_doc_selection_change(true);
            finish(app);
            app.split.area = Rect::new(0.0, 84.0, 800.0, 700.0);
            assert_eq!(app.split.doc.chunk_index(), other_index);
            app.split.scroll = app.split.doc.max_scroll(app.split.area);
            app.chunk_edge_until = None;
            let cursor = app.shell.active().unwrap().buffer().unwrap().cursor();
            assert!(app.page_other_document_at_edge(-60.0));
            while app.split.doc.is_loading() {
                app.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
            }
            assert_eq!(app.split.doc.chunk_index(), other_index + 1);
            assert_eq!(app.doc.chunk_index(), other_index);
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().cursor(),
                cursor
            );
            assert!(app.focus_other_editor());
            assert_eq!(app.doc.chunk_index(), other_index + 1);
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), source);
        });
    }

    #[test]
    fn content_page_edit_save_undo_and_ime_do_not_drop_unmounted_content() {
        with_app(|app, path, source| {
            assert!(app.change_document_chunk(Action::Next, false));
            finish(app);
            let offset = app.shell.active().unwrap().buffer().unwrap().cursor();
            app.shell
                .active_buffer_mut()
                .unwrap()
                .insert("# 新标题\r\n\r\n");
            app.after_doc_edit(true);
            finish(app);
            let mut expected = source.to_owned();
            expected.insert_str(offset, "# 新标题\r\n\r\n");
            assert!(app.shell.save_active());
            assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
            app.shell.active_buffer_mut().unwrap().undo();
            app.after_doc_selection_change(true);
            finish(app);
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), source);
            assert!(app.shell.save_active());
            assert_eq!(std::fs::read_to_string(path).unwrap(), source);
            app.shell
                .active_buffer_mut()
                .unwrap()
                .set_composition("组合输入", 0);
            let index = app.doc.chunk_index();
            assert!(!app.change_document_chunk(Action::Next, false));
            assert_eq!(app.doc.chunk_index(), index);
            assert!(app
                .shell
                .active()
                .unwrap()
                .buffer()
                .unwrap()
                .composition()
                .is_some());
            app.shell.active_buffer_mut().unwrap().cancel_composition();
            assert!(app.change_document_chunk(Action::Toggle, false));
            finish(app);
            assert!(!app.doc.is_chunked());
            assert!(app.change_document_chunk(Action::Toggle, false));
            finish(app);
            assert!(app.doc.is_chunked());
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), source);
        });
    }

    #[test]
    fn comment_highlights_appear_when_their_content_page_is_loaded() {
        with_app(|app, _, source| {
            let start = source.find("词义181").unwrap();
            app.panels.comments.comments = vec![sidecars::DocumentComment {
                resolved: false,
                id: "page-comment".into(),
                parent_id: None,
                target_type: "text".into(),
                author: "test".into(),
                content: "批注".into(),
                created_at: String::new(),
                updated_at: None,
                attachments: Vec::new(),
                anchor: crate::ui::comment_anchors::create(
                    source,
                    start..start + "词义181".len(),
                    "text",
                ),
            }];
            let palette = theme::configured_palette(false);
            app.paint_editor_comments(&palette);
            assert!(app.comment_bubbles.is_empty());
            app.shell
                .active_buffer_mut()
                .unwrap()
                .set_cursor(start, false);
            app.after_doc_selection_change(true);
            finish(app);
            app.paint_editor_comments(&palette);
            assert!(app
                .comment_bubbles
                .iter()
                .any(|(_, id)| id == "page-comment"));
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), source);
        });
    }
}
