//! 处理 AI 对话区域的滚动、横向拖动和滚轮输入。
use super::*;
impl App {
    pub(super) fn ai_scroll_enabled(&self) -> bool {
        self.dialog.is_none()
            && self.global_import.is_none()
            && self.object_picker.is_none()
            && self.settings_overlay.is_none()
            && self.export_form.is_none()
            && self.menu.is_none()
            && self.commands.review.is_none()
            && self.search.is_none()
            && self.command.is_none()
            && self.table_editing.is_none()
            && (self.state.view == WorkspaceView::MochiAi
                || self.ai.float.is_some()
                || (self.state.ai_panel_open && self.state.right_panel == RightPanel::Assistant))
    }
    pub fn on_horizontal_wheel(&mut self, x: f32, y: f32, delta: i16) -> bool {
        if self.global_import.is_some() {
            return true;
        }
        if self.desktop_manager_captures_pointer(x, y) {
            return true;
        }
        if self.notification_open() {
            return true;
        }
        if self.object_picker.is_some() || self.settings_overlay.is_some() {
            return true;
        }
        if self.state.view == WorkspaceView::Editor
            && self.dialog.is_none()
            && self.menu.is_none()
            && self.editor_area.contains(x, y)
        {
            let area = self.editor_area;
            if self.viewer_layout.body.contains(x, y)
                && matches!(self.viewer_tab(), Some((_, viewer::Content::Canvas(_))))
            {
                self.canvas_scroll(-f32::from(delta) / 120.0, true);
                return true;
            }
            if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
                state.scroll(area, -f32::from(delta) / 120.0, true);
                return true;
            }
        }
        if !self.ai_scroll_enabled() {
            return false;
        }
        if self
            .ai
            .layout
            .files_viewport
            .is_some_and(|r| r.contains(x, y))
        {
            self.ai.panel.files_scroll = (self
                .ai
                .panel
                .files_scroll
                .clamp(0.0, self.ai.layout.files_max_scroll)
                + f32::from(delta) / 120.0 * 96.0)
                .clamp(0.0, self.ai.layout.files_max_scroll);
            return true;
        }
        let used = self.ai.panel.horizontal.wheel(
            &self.ai.layout,
            self.ai.panel.scroll,
            x,
            y,
            f32::from(delta) / 120.0 * 96.0,
        );
        if used {
            self.ai.panel.hover_content = None;
            self.ai.panel.hover_content_hit = false;
        }
        used
    }
    pub(super) fn ai_scroll_click(&mut self, x: f32, y: f32) -> bool {
        if !self.ai_scroll_enabled() {
            return false;
        }
        let used = self
            .ai
            .panel
            .horizontal
            .click(&self.ai.layout, self.ai.panel.scroll, x, y);
        if used {
            self.ai.panel.hover_content = None;
            self.ai.panel.hover_content_hit = false;
        }
        used
    }
    pub(super) fn ai_scroll_pointer(&mut self, x: f32, y: f32) -> bool {
        if !self.ai_scroll_enabled() {
            return self.ai.panel.horizontal.hover.take().is_some();
        }
        if self.ai.panel.horizontal.dragging() {
            return self
                .ai
                .panel
                .horizontal
                .drag_to(&self.ai.layout, self.ai.panel.scroll, x);
        }
        self.ai
            .panel
            .horizontal
            .pointer(&self.ai.layout, self.ai.panel.scroll, x, y)
    }
    pub fn cancel_horizontal_drag(&mut self) -> bool {
        let had = self.ai.panel.horizontal.dragging();
        self.ai.panel.horizontal.end_drag();
        had
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversation_history_wheel_keeps_chat_position_and_clamps_at_edges() {
        let profile = std::env::temp_dir().join(format!(
            "mochi-history-scroll-{}.json",
            mochi_core::paths::random_base36(12)
        ));
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(profile)))).unwrap();
        app.state.ai_panel_open = true;
        app.state.right_panel = RightPanel::Assistant;
        app.ai.panel.show_conversations = true;
        app.ai.panel.scroll = 40.0;
        app.ai.panel.sessions = (0..25)
            .map(|i| mochi_core::ai::session::AiSessionMeta {
                id: format!("session-{i}"),
                ..Default::default()
            })
            .collect();
        app.ai.layout =
            assistant::layout(&app.ai.panel, Rect::from_size(780.0, 72.0, 420.0, 728.0));
        let viewport = app.ai.layout.conversations_viewport.unwrap();
        let point = (viewport.left + 10.0, viewport.top + 10.0);
        app.on_wheel(point.0, point.1, -120);
        assert_eq!(app.ai.panel.conversations_scroll, 96.0);
        assert_eq!(app.ai.panel.scroll, 40.0);
        app.on_wheel(point.0, point.1, i16::MIN);
        assert_eq!(
            app.ai.panel.conversations_scroll,
            app.ai.layout.conversations_max_scroll
        );
        app.on_wheel(point.0, point.1, i16::MAX);
        assert_eq!(app.ai.panel.conversations_scroll, 0.0);
    }
    #[test]
    fn horizontal_wheel_routes_only_visible_ai_and_does_not_move_vertical_scroll() {
        let profile = std::env::temp_dir().join(format!(
            "mochi-ai-scroll-{}.json",
            mochi_core::paths::random_base36(12)
        ));
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(profile)))).unwrap();
        app.state.view = WorkspaceView::MochiAi;
        app.ai.panel.active = Some(AiConversation {
            id: "test".into(),
            messages: vec![mochi_core::ai::session::AiStoredMessage::new(
                "assistant",
                include_str!("../../tests/fixtures/ai-scroll-verification.md"),
            )],
            ..Default::default()
        });
        app.ai.layout = assistant::layout(&app.ai.panel, Rect::from_size(0.0, 0.0, 420.0, 900.0));
        let r = app.ai.layout.messages[0].body.scroll_regions[0].clone();
        let o = app.ai.layout.body_origin(0, 0.0).unwrap();
        let point = (o.0 + r.viewport.left + 2.0, o.1 + r.viewport.top + 2.0);
        assert!(app.on_horizontal_wheel(point.0, point.1, 120));
        assert_eq!(app.ai.panel.scroll, 0.0);
        let offsets = app.ai.panel.horizontal.offsets(
            &app.ai.layout.session_id,
            &app.ai.layout.messages[0].scroll_key,
        );
        assert_eq!(r.offset(Some(&offsets)), 96.0);
        app.state.view = WorkspaceView::Home;
        assert!(!app.on_horizontal_wheel(point.0, point.1, 120));
        app.state.view = WorkspaceView::MochiAi;
        app.dialog = Some(Dialog {
            title: "Blocked".into(),
            description: String::new(),
            field: None,
            error: String::new(),
            note: None,
            buttons: Vec::new(),
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        assert!(!app.on_horizontal_wheel(point.0, point.1, 120));
        assert_eq!(
            app.ai.panel.horizontal.offsets(
                &app.ai.layout.session_id,
                &app.ai.layout.messages[0].scroll_key
            ),
            offsets
        );
    }
}
