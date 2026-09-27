//! 分派鼠标滚轮输入并更新对应区域的滚动状态。
use super::*;

impl App {
    /// 滚轮。滚哪个面板取决于**指针在哪**，不是焦点在哪——
    /// 这是所有人对滚轮的预期，焦点跟随会让「看着侧栏却滚了正文」。
    pub fn on_wheel(&mut self, x: f32, y: f32, delta: i16) {
        if let Some(dialog) = self.dialog.as_mut() {
            if matches!(
                dialog.note,
                Some(crate::ui::widgets::DialogNote::Markdown { .. })
            ) {
                dialog.scroll_note(
                    self.renderer.viewport(),
                    x,
                    y,
                    f32::from(delta) / 120.0 * 72.0,
                );
                return;
            }
        }
        if let Some(pending) = self.global_import.as_mut() {
            let layout = pending.view.layout(self.renderer.viewport());
            if layout.tree.contains(x, y) {
                pending.view.scroll = (pending.view.scroll - f32::from(delta) / 120.0 * 96.0)
                    .clamp(0.0, layout.max_scroll);
                pending.view.hover = None;
            }
            return;
        }
        if self.desktop_manager_captures_pointer(x, y) {
            self.desktop_manager_wheel(x, y, delta);
            return;
        }
        if self.menu_input_active() {
            self.menu
                .as_mut()
                .unwrap()
                .scroll_at(x, y, f32::from(delta) / 120.0 * 108.0);
            return;
        }
        if self.dialog.is_none() && self.settings_overlay.is_none() {
            if let Some(suggestion) = self.wiki_suggestion.as_mut() {
                if suggestion
                    .menu
                    .scroll_at(x, y, f32::from(delta) / 120.0 * 108.0)
                {
                    return;
                }
            }
        }
        if self.navigation_library_wheel(x, y, delta) {
            return;
        }
        if self.workflows_active() && self.workflows.view.area.contains(x, y) {
            self.workflows_wheel(x, y, delta);
            return;
        }
        if self.notification_open() {
            self.notification_wheel(x, y, delta);
            return;
        }
        if self.automation.panel.is_some() {
            if self.dialog.is_none() {
                self.automation_wheel(delta);
            }
            return;
        }
        if let Some(picker) = self.template_picker.as_mut() {
            let layout = picker.layout(self.renderer.viewport());
            picker.scroll =
                (picker.scroll - f32::from(delta) / 120.0 * 72.0).clamp(0.0, layout.max_scroll);
            return;
        }
        if let Some(dialog) = self.object_picker.as_mut() {
            let layout = crate::ui::object_picker::layout(&dialog.state, self.renderer.viewport());
            dialog.state.scroll_by(&layout, f32::from(delta) / 120.0);
            return;
        }
        if self.image_preview.is_some() {
            return;
        }
        if self.ai_scroll_enabled()
            && self.ai.panel.show_conversations
            && self.ai.layout.popover.is_some_and(|r| r.contains(x, y))
        {
            let max = self.ai.layout.conversations_max_scroll;
            self.ai.panel.conversations_scroll =
                (self.ai.panel.conversations_scroll.clamp(0.0, max)
                    - f32::from(delta) / 120.0 * 96.0)
                    .clamp(0.0, max);
            self.ai.panel.hover_hit = None;
            return;
        }
        if shift_down() && self.ai_scroll_enabled() && self.ai.layout.messages_rect.contains(x, y) {
            self.on_horizontal_wheel(x, y, delta.saturating_neg());
            return;
        }
        if self.export_form.is_some() {
            return;
        }
        if let Some(review) = self.commands.review.as_mut() {
            let layout = crate::ui::command_review::layout(self.renderer.viewport());
            review
                .field
                .scroll_multiline(layout.field, f32::from(delta) / 120.0 * 60.0);
            return;
        }
        if self.table_editing.is_some() && !self.commit_table_cell() {
            return;
        }
        if self.dialog.is_none()
            && self.settings_overlay.is_none()
            && self
                .ai
                .float
                .is_some_and(|float| float.rect().contains(x, y))
        {
            if self.ai.layout.messages_rect.contains(x, y) {
                let max = self.ai.layout.max_scroll();
                let step = f32::from(delta) / 120.0 * theme::ROW_HEIGHT * 3.0;
                let next = (self.ai.panel.scroll.min(max) - step).clamp(0.0, max);
                self.ai.panel.stick_to_bottom = next >= max - 1.0;
                self.ai.panel.scroll = next;
            }
            return;
        }
        if let Some(dialog) = self.dialog.as_mut() {
            if dialog.is_math_editor() {
                let rect = dialog.field_rect(self.renderer.viewport()).unwrap();
                if rect.contains(x, y) {
                    dialog
                        .field
                        .as_mut()
                        .unwrap()
                        .scroll_multiline(rect, f32::from(delta) / 120.0 * 60.0);
                }
            }
            return;
        }
        let notches = delta as f32 / 120.0;
        if self.settings_overlay.is_some()
            || (self.settings_overlay.is_none() && self.settings_tab().is_some())
        {
            let step = notches * theme::ROW_HEIGHT * 3.0;
            if self.prefs.nav_layout.scroll_area.contains(x, y) {
                self.prefs.nav_scroll =
                    (self.prefs.nav_scroll - step).clamp(0.0, self.prefs.nav_layout.max_scroll());
                return;
            }
            if self
                .settings_tab()
                .is_some_and(|(tab, section)| tab == "ai" && section != "parameters")
            {
                self.prefs.providers.scroll = (self.prefs.providers.scroll - step)
                    .clamp(0.0, self.prefs.provider_layout.max_scroll());
            } else {
                self.prefs.scroll =
                    (self.prefs.scroll - step).clamp(0.0, self.prefs.content_layout.max_scroll());
                self.sync_settings_section();
            }
            return;
        }
        let chrome = self.build_chrome();
        let tab_area = tab_bar::tabs_content_area(chrome.tree.rect(chrome.tab_bar));
        if !chrome.tree.is_hidden(chrome.tab_bar) && tab_area.contains(x, y) {
            self.tab_scroll = (self.tab_scroll - notches * 180.0).clamp(
                0.0,
                tab_bar::max_scroll_with_fixed_width(
                    tab_area,
                    &self.tab_projection(),
                    self.state.compact_tab_bar,
                    self.tab_fixed_width(),
                ),
            );
            return;
        }
        if let Some(s) = self.search.as_mut() {
            if self.search_layout.list.contains(x, y) {
                let max = self.search_layout.max_scroll();
                s.scroll = (s.scroll - notches * 32.0 * 3.0).clamp(0.0, max);
            }
            return;
        }
        if let Some(c) = self.command.as_mut() {
            if self.command_layout.list.contains(x, y) {
                let max = self.command_layout.max_scroll();
                c.scroll = (c.scroll - notches * 44.0 * 3.0).clamp(0.0, max);
            }
            return;
        }
        if self.base_detail_open() {
            let area = self.renderer.viewport();
            if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
                state.scroll(area, notches, false);
            }
            return;
        }
        if self.nav_layout.scroll_area.contains(x, y) {
            let max = self.nav_layout.max_scroll();
            self.nav.scroll = (self.nav.scroll - notches * theme::ROW_HEIGHT * 3.0).clamp(0.0, max);
            return;
        }
        // 右侧面板的列表
        let step = notches * theme::ROW_HEIGHT * 3.0;
        if self.agent_config_section().is_some() && self.agent.nav_layout.body.contains(x, y) {
            self.agent.nav_scroll =
                (self.agent.nav_scroll - step).clamp(0.0, self.agent.nav_layout.max_scroll());
            return;
        }
        if self.agent_config_section().is_some() {
            if let Some(editor) = self.agent.source_editor.as_mut() {
                if editor.area.contains(x, y) {
                    editor.field.scroll_multiline(editor.area, step);
                    return;
                }
            }
        }
        if self.split.area.contains(x, y) {
            if self.page_other_document_at_edge(step) {
                return;
            }
            let max = if self.split.other_source_mode {
                self.split.source.max_scroll(self.split.area)
            } else {
                self.split.doc.max_scroll(self.split.area)
            };
            self.split.scroll = (self.split.scroll - step).clamp(0.0, max);
            return;
        }
        if self.outline_area.contains(x, y) {
            let max = outline::max_scroll(self.outline_area, self.doc.headings().len());
            self.outline_scroll =
                (self.outline_scroll as f32 - notches * 3.0).clamp(0.0, max as f32) as usize;
            return;
        }
        if self.state.view == WorkspaceView::MochiAi {
            if self.ai.workspace_side.list.contains(x, y) {
                self.ai.workspace.scroll = (self.ai.workspace.scroll - step)
                    .clamp(0.0, self.ai.workspace_side.max_scroll());
                return;
            }
            if self
                .ai
                .layout
                .navigation_rect
                .is_some_and(|rect| rect.contains(x, y))
            {
                self.ai.panel.navigation_scroll = (self.ai.panel.navigation_scroll - step)
                    .clamp(0.0, self.ai.layout.navigation_max_scroll);
                return;
            }
            if self.ai.layout.messages_rect.contains(x, y) {
                let max = self.ai.layout.max_scroll();
                self.ai.panel.scroll = (self.ai.panel.scroll.min(max) - step).clamp(0.0, max);
                self.ai.panel.stick_to_bottom = self.ai.panel.scroll >= max - 1.0;
                return;
            }
        }
        let scrolled = match self.state.right_panel {
            RightPanel::Annotations
                if self.state.right_sidebar_visible()
                    && self.panels.annotation_layout.list.contains(x, y) =>
            {
                self.panels.annotation_scroll = (self.panels.annotation_scroll - step)
                    .clamp(0.0, self.panels.annotation_layout.max_scroll());
                true
            }
            RightPanel::Assistant if self.ai.layout.messages_rect.contains(x, y) => {
                let max = self.ai.layout.max_scroll();
                let next = (self.ai.panel.scroll.min(max) - step).clamp(0.0, max);
                // 用户往上滚了就别再自动贴底；滚回底部又恢复
                self.ai.panel.stick_to_bottom = next >= max - 1.0;
                self.ai.panel.scroll = next;
                true
            }
            RightPanel::VersionHistory if self.panels.version_layout.list.contains(x, y) => {
                let max = self.panels.version_layout.max_scroll();
                self.panels.version.scroll = (self.panels.version.scroll - step).clamp(0.0, max);
                true
            }
            RightPanel::Comments if self.panels.comments_layout.list.contains(x, y) => {
                let max = self.panels.comments_layout.max_scroll();
                self.panels.comments.scroll = (self.panels.comments.scroll - step).clamp(0.0, max);
                true
            }
            RightPanel::DocumentMounts if self.panels.mounts_layout.list.contains(x, y) => {
                let max = self.panels.mounts_layout.max_scroll();
                self.panels.mounts.scroll = (self.panels.mounts.scroll - step).clamp(0.0, max);
                true
            }
            RightPanel::AgentInbox if self.panels.inbox_layout.list.contains(x, y) => {
                let max = self.panels.inbox_layout.max_scroll();
                self.panels.inbox.scroll = (self.panels.inbox.scroll - step).clamp(0.0, max);
                true
            }
            _ => false,
        };
        if scrolled {
            return;
        }
        if !self.editor_area.contains(x, y) {
            if self.side.layout.content.contains(x, y) {
                // 一格滚轮 = 3 行，与资源管理器手感一致
                let step = notches * (sidebar::row_height() + sidebar::row_gap()) * 3.0;
                let max = self.side.layout.max_scroll();
                self.side.scroll = (self.side.scroll - step).clamp(0.0, max);
            }
            return;
        }

        // 一格 = 3 行正文高度
        let step = notches * theme::ROW_HEIGHT * 3.0;
        let area = self.editor_area;
        match self.content() {
            MainContent::Home => self.home.scroll_by(area, step),
            MainContent::Document => {
                if shift_down()
                    && (self
                        .doc
                        .scroll_code(area, self.shell.active_scroll(), x, y, step)
                        || self
                            .doc
                            .scroll_table(area, self.shell.active_scroll(), x, y, step))
                {
                    return;
                }
                if !shift_down() && self.page_document_at_edge(step) {
                    return;
                }
                let next =
                    (self.shell.active_scroll() - step).clamp(0.0, self.doc.max_scroll(area));
                self.shell.set_active_scroll(next);
            }
            MainContent::Source => {
                let max = self.source.max_scroll(area);
                let next = (self.shell.active_scroll() - step).clamp(0.0, max);
                self.shell.set_active_scroll(next);
            }
            MainContent::Standalone(WorkspaceView::AgentConfig) => {
                let max = self.agent.layout.max_scroll();
                self.agent.scroll = (self.agent.scroll - step).clamp(0.0, max);
            }
            MainContent::Special if self.viewer_tab().is_some() => {
                self.on_viewer_wheel(x, y, notches)
            }
            MainContent::Special => {
                if self
                    .settings_tab()
                    .is_some_and(|(tab, section)| tab == "ai" && section != "parameters")
                {
                    self.prefs.providers.scroll = (self.prefs.providers.scroll - step)
                        .clamp(0.0, self.prefs.provider_layout.max_scroll());
                    return;
                }
                let max = self.prefs.content_layout.max_scroll();
                self.prefs.scroll = (self.prefs.scroll - step).clamp(0.0, max);
            }
            MainContent::Standalone(WorkspaceView::Schedule) => self.agenda_wheel(x, y, step),
            MainContent::Standalone(WorkspaceView::Inbox) => {
                let max = self.views.inbox_layout.max_scroll();
                self.views.inbox.scroll = (self.views.inbox.scroll - step).clamp(0.0, max);
            }
            MainContent::Standalone(WorkspaceView::Recent) => {
                let max = self.views.recent_layout.max_scroll();
                self.views.recent.scroll = (self.views.recent.scroll - step).clamp(0.0, max);
            }
            MainContent::Standalone(WorkspaceView::Templates) => {
                let max = self.views.templates_layout.max_scroll;
                self.views.templates.scroll = (self.views.templates.scroll - step).clamp(0.0, max);
            }
            MainContent::Standalone(WorkspaceView::Marketplace) => {
                self.marketplace.view.scroll = (self.marketplace.view.scroll - step)
                    .clamp(0.0, self.marketplace.layout.max_scroll);
            }
            // 没有可滚动内容的视图不处理滚轮。
            _ => {}
        }
    }
}
