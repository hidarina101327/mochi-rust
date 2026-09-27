//! 处理鼠标移动、悬停和内容区域的指针状态。
use super::*;

impl App {
    pub fn on_pointer_leave(&mut self) -> bool {
        let import_changed = self
            .global_import
            .as_mut()
            .is_some_and(|p| p.view.hover.take().is_some());
        self.window_motion.pointer_pos = None;
        let window_changed = self.window_motion.pointer(None);
        let provider_changed = self
            .prefs
            .providers
            .interaction
            .pointer(None, Self::now_ms());
        let notification_changed = self.notifications.title_hover.take().is_some()
            | self.notifications.view.hover.take().is_some()
            | self.notifications.view.scrollbar.hover.take().is_some();
        let mut scrollbar_changed = self.scrollbars.interaction.hover.take().is_some();
        if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
            scrollbar_changed |= state.pointer_leave();
        }
        let changed = self.block_pointer.take().is_some();
        let resource_changed = self.on_resource_hover(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let ai_changed = self.ai.panel.hover_message.take().is_some()
            | self.ai.panel.hover_content.take().is_some()
            | self.ai.panel.hover_hit.take().is_some();
        self.ai.panel.hover_content_hit = false;
        changed
            || resource_changed
            || ai_changed
            || scrollbar_changed
            || notification_changed
            || provider_changed
            || window_changed
            || import_changed
    }

    /// 拖动中。返回是否需要重画。
    pub fn on_mouse_move(&mut self, x: f32, y: f32) -> bool {
        if let Some(pending) = self.global_import.as_mut() {
            let hit = pending.view.hit(self.renderer.viewport(), x, y);
            let changed = pending.view.hover != Some(hit);
            pending.view.hover = Some(hit);
            return changed;
        }
        if self.desktop_manager_captures_pointer(x, y) {
            let changed = self.notifications.title_hover.take().is_some();
            return self.desktop_manager_pointer(x, y) || changed;
        }
        self.window_motion.pointer_pos = Some((x, y));
        if self.ai_float_drag_to(x, y) {
            return true;
        }
        let window_changed = self.window_motion.pointer(self.window_hover_rect(x, y));
        if self.menu_input_active() {
            return self.menu.as_mut().unwrap().pointer_move(x, y) || window_changed;
        }
        if let Some(suggestion) = self.wiki_suggestion.as_mut() {
            if suggestion.menu.is_scrollbar_dragging() {
                return suggestion.menu.pointer_move(x, y) || window_changed;
            }
        }
        let provider_hit = if self.dialog.is_none()
            && self.menu.is_none()
            && !self.notification_open()
            && self
                .settings_tab()
                .is_some_and(|(tab, section)| Self::is_provider_settings(&tab, &section))
        {
            self.prefs.provider_layout.hit(x, y)
        } else {
            None
        };
        let provider_changed = self
            .prefs
            .providers
            .interaction
            .pointer(provider_hit, Self::now_ms());
        if self.nav.library_drag.is_some() {
            self.update_navigation_library_drag(x, y);
            return true;
        }
        if self.workflows_active()
            && (self.workflows.view.area.contains(x, y) || self.workflows.view.drag.is_some())
        {
            return self.workflows.view.motion(x, y);
        }
        if self.notification_open() {
            return self.notification_pointer(x, y) || window_changed;
        }
        let chrome = self.build_chrome();
        let hover = chrome.hit(x, y).filter(|h| {
            matches!(
                h,
                NodeKey::TitleBarBack
                    | NodeKey::TitleBarThemeToggle
                    | NodeKey::TitleBarAiToggle
                    | NodeKey::TitleBarNotifications
                    | NodeKey::TitleBarDesktop
                    | NodeKey::TitleBarMarketplace
                    | NodeKey::TitleBarTemplates
                    | NodeKey::TitleBarAutomations
            )
        });
        let title_changed = self.notifications.title_hover != hover;
        self.notifications.title_hover = hover;
        let scrollbar_changed = self.scrollbar_pointer(x, y);
        if self.scrollbars.interaction.dragging() {
            return true;
        }
        self.on_content_mouse_move(x, y)
            || scrollbar_changed
            || title_changed
            || provider_changed
            || window_changed
    }

    pub(super) fn on_content_mouse_move(&mut self, x: f32, y: f32) -> bool {
        if let Some(dialog) = self.object_picker.as_mut() {
            let layout = crate::ui::object_picker::layout(&dialog.state, self.renderer.viewport());
            if self
                .drag
                .is_some_and(|d| d.target == DragTarget::ObjectPickerQuery)
            {
                dialog.state.query.click(x - layout.query.left - 12.0, true);
                return true;
            }
            return dialog.state.hover(&layout, x, y);
        }
        if self
            .drag
            .is_some_and(|d| d.target == DragTarget::AiTextSelect)
        {
            return self.ai_update_text_selection(x, y);
        }
        if self
            .drag
            .is_some_and(|d| d.target == DragTarget::SidebarTree)
        {
            return self.update_sidebar_tree_drag(x, y);
        }
        // 即使底下有多维表格详情，模态字段的指针捕获仍然优先。
        if self.settings_overlay.is_some() {
            return self.on_mouse_move_inner(x, y);
        }
        let session_hover = if self.state.view == WorkspaceView::MochiAi
            && self.dialog.is_none()
            && self.menu.is_none()
        {
            self.ai
                .workspace_side
                .rows
                .iter()
                .find(|(r, _)| self.ai.workspace_side.list.contains(x, y) && r.contains(x, y))
                .map(|(_, m)| m.id.clone())
        } else {
            None
        };
        let session_changed = self.ai.workspace.hover != session_hover;
        self.ai.workspace.hover = session_hover;
        let resource_changed = self.on_resource_hover(x, y);
        let base_changed = self.on_base_pointer(x, y);
        if self.base_detail_open()
            && self.dialog.is_none()
            && self.menu.is_none()
            && self.search.is_none()
            && self.command.is_none()
        {
            return base_changed;
        }
        if matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.dragging()) {
            return base_changed;
        }
        let previous = self.block_pointer.and_then(|(x, y)| {
            self.doc
                .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
        });
        let old_image = self.block_pointer.and_then(|(x, y)| {
            self.doc
                .image_at(self.editor_area, self.shell.active_scroll(), x, y)
        });
        let split_chrome = self.build_chrome();
        let split_button = tab_bar::split_button_rect(split_chrome.tree.rect(split_chrome.tab_bar));
        let split_hover_changed = self
            .block_pointer
            .is_some_and(|(px, py)| split_button.contains(px, py))
            != split_button.contains(x, y);
        let navigation_area = split_chrome.tree.rect(split_chrome.tab_bar);
        let navigation_hover_changed = self
            .block_pointer
            .and_then(|(px, py)| tab_bar::navigation_hit(navigation_area, px, py))
            != tab_bar::navigation_hit(navigation_area, x, y);
        self.block_pointer = Some((x, y));
        let current = self
            .doc
            .block_handle(self.editor_area, self.shell.active_scroll(), x, y);
        let block_changed = previous != current
            || old_image
                != self
                    .doc
                    .image_at(self.editor_area, self.shell.active_scroll(), x, y);
        let horizontal_changed = self.ai_scroll_pointer(x, y);
        if self.ai.panel.horizontal.dragging() {
            return horizontal_changed;
        }
        let hover = if self.dialog.is_none()
            && self.export_form.is_none()
            && self.menu.is_none()
            && ((self.state.ai_panel_open && self.state.right_panel == RightPanel::Assistant)
                || self.state.view == WorkspaceView::MochiAi
                || self.ai.float.is_some())
        {
            self.ai.layout.message_at(self.ai.panel.scroll, x, y)
        } else {
            None
        };
        let hit_hover = if self.ai_scroll_enabled() {
            self.ai.layout.hit(x, y)
        } else {
            None
        };
        let hit_changed = self.ai.panel.hover_hit != hit_hover;
        self.ai.panel.hover_hit = hit_hover;
        let changed = self.ai.panel.hover_message != hover;
        self.ai.panel.hover_message = hover;
        let content =
            hover.and_then(|_| self.ai.layout.content_at(self.ai.panel.scroll, x, y, true));
        let content_hit = content.is_some()
            && self.ai.layout.content_at(self.ai.panel.scroll, x, y, false) == content;
        let content_changed = self.ai.panel.hover_content != content
            || self.ai.panel.hover_content_hit != content_hit;
        self.ai.panel.hover_content = content;
        self.ai.panel.hover_content_hit = content_hit;
        self.on_mouse_move_inner(x, y)
            || changed
            || content_changed
            || horizontal_changed
            || block_changed
            || base_changed
            || resource_changed
            || session_changed
            || hit_changed
            || split_hover_changed
            || navigation_hover_changed
    }

    pub(super) fn on_mouse_move_inner(&mut self, x: f32, y: f32) -> bool {
        if self.automation.panel.is_some() && self.dialog.is_none() {
            return self.automation_hover(x, y);
        }
        if let Some(picker) = self.template_picker.as_mut() {
            let hit = picker.hit(self.renderer.viewport(), x, y);
            let hover = (!matches!(
                hit,
                crate::ui::template_picker::Hit::Inside | crate::ui::template_picker::Hit::Outside
            ))
            .then_some(hit);
            let changed = picker.hover != hover;
            picker.hover = hover;
            return changed;
        }
        if let Some(s) = self.wiki_suggestion.as_mut() {
            if s.menu.rect.contains(x, y) {
                return s.menu.pointer_move(x, y);
            }
        }
        if let Some(picker) = self.table_picker.as_mut() {
            return picker.hover(x, y);
        }
        if let Some(form) = self.export_form.as_mut() {
            let viewport = self.renderer.viewport();
            let hit = if form.rect(viewport).contains(x, y) {
                form.hit(viewport, x, y)
            } else {
                None
            };
            let changed = form.hover != hit;
            form.hover = hit;
            return changed;
        }
        if self.commands.review.is_some() {
            return false;
        }
        let viewport = self.renderer.viewport();
        // 覆盖层的悬停：对话框按钮、菜单项
        if let Some(d) = self.link_create.as_mut() {
            return d.set_hover(viewport, x, y);
        }
        if let Some(d) = self.mapped_folder.as_mut() {
            return d.set_hover(viewport, x, y);
        }
        if let Some(d) = self.dialog.as_mut() {
            if d.is_math_editor()
                && self
                    .drag
                    .is_some_and(|drag| drag.target == DragTarget::DialogFieldSelect)
            {
                let rect = d.field_rect(viewport).unwrap();
                d.field.as_mut().unwrap().multiline_select(rect, x, y, true);
                return true;
            }
            return d.set_hover(viewport, x, y);
        }
        if let Some(s) = self.search.as_mut() {
            // TSX：onMouseEnter 即选中该行
            if let SearchHit::Row(i) = self.search_layout.hit(x, y) {
                if s.selected != i {
                    s.selected = i;
                    return true;
                }
            }
            return false;
        }
        if let Some(c) = self.command.as_mut() {
            // TSX：onMouseEnter 即选中该行
            if let command::Hit::Row(i) = self.command_layout.hit(x, y) {
                if c.selected != i {
                    c.selected = i;
                    return true;
                }
            }
            return false;
        }
        if let Some(m) = self.menu.as_mut() {
            return m.pointer_move(x, y);
        }
        // 窗口按钮的悬停高亮
        let hover = platform::caption_buttons(viewport)
            .iter()
            .position(|b| b.contains(x, y));
        let mut changed = hover != self.caption_hover;
        if matches!(self.state.view, WorkspaceView::Schedule) {
            changed |= self.agenda_hover(x, y);
        }
        if self.state.view == WorkspaceView::Inbox {
            let hover = self
                .views
                .inbox_layout
                .hit(x, y)
                .filter(|h| *h != views::inbox::Hit::Blank);
            changed |= hover != self.views.inbox.hover;
            self.views.inbox.hover = hover;
        }
        if self.state.view == WorkspaceView::Marketplace {
            let hover = self.marketplace.layout.hit(x, y);
            changed |= hover != self.marketplace.view.hover;
            self.marketplace.view.hover = hover;
        }
        if self.state.view == WorkspaceView::Templates {
            let hover = self
                .views
                .templates_layout
                .hit(x, y)
                .filter(|h| *h != crate::ui::templates::Hit::Blank);
            changed |= hover != self.views.templates.hover;
            self.views.templates.hover = hover;
        }
        if self.content() == MainContent::Home {
            changed |= self.home.set_hover(self.editor_area, x, y);
        }
        let tab_chrome = self.build_chrome();
        let tab_area = tab_bar::tabs_content_area(tab_chrome.tree.rect(tab_chrome.tab_bar));
        let hovered = tab_bar::hit_scrolled_with_fixed_width(
            tab_area,
            &self.tab_projection(),
            self.state.compact_tab_bar,
            self.tab_fixed_width(),
            self.tab_scroll,
            x,
            y,
        )
        .map(|h| match h {
            tab_bar::Hit::Select(i) | tab_bar::Hit::Close(i) => i,
        });
        changed |= tab_bar::set_hover(hovered);
        self.caption_hover = hover;
        let hover = if self.content() == MainContent::Document {
            self.links_layout.hover_at(x, y)
        } else {
            None
        };
        if hover != self.links.hover {
            self.links.hover = hover;
            changed = true;
        }
        // 文件树行的悬停（hover:bg-surface-muted + 子项数）
        let row_hover = match self.side.layout.hit(x, y) {
            Some(SidebarHit::Row(i)) | Some(SidebarHit::RowChevron(i)) => Some(i),
            _ => None,
        };
        if row_hover != self.side.hover {
            self.side.hover = row_hover;
            changed = true;
        }
        // 工具栏按钮悬停
        let bar_hover = if self.toolbar_area.contains(x, y) {
            self.toolbar_layout().hit(x, y)
        } else {
            None
        };
        if bar_hover != self.toolbar_hover {
            self.toolbar_hover = bar_hover;
            changed = true;
        }
        // Agent 配置页按钮悬停
        if self.agent_config_section().is_some() {
            let nav_hover = self.agent.nav_layout.hit(x, y);
            if nav_hover != self.agent.nav_hover {
                self.agent.nav_hover = nav_hover;
                changed = true;
            }
            let hover = self
                .agent
                .layout
                .hit(x, y)
                .filter(|h| *h != agent_config::Hit::Blank);
            if hover != self.agent.hover {
                self.agent.hover = hover;
                changed = true;
            }
        }
        // 查看器按钮悬停
        if self.viewer_tab().is_some() {
            let hover = self
                .viewer_layout
                .hit(x, y)
                .filter(|h| !matches!(h, viewer::Hit::ImageCanvas | viewer::Hit::Body));
            if hover != self.viewer_hover {
                self.viewer_hover = hover;
                changed = true;
            }
        }

        let Some(drag) = self.drag else {
            return changed;
        };
        let clamps = theme::tokens().clamps;
        let window_width = viewport.width();
        match drag.target {
            DragTarget::SidebarTree => return self.update_sidebar_tree_drag(x, y),
            DragTarget::ScheduleBlock => return self.schedule_drag_to(x, y) || changed,
            DragTarget::AiTextSelect => return self.ai_update_text_selection(x, y),
            DragTarget::ObjectPickerQuery | DragTarget::DialogFieldSelect => return false,
            DragTarget::EditorImage => return self.resize_editor_image(x),
            DragTarget::EditorTable => {
                if let Some((start, track, thumb_width, max)) = self.table_drag {
                    self.doc.set_table_scroll(
                        start,
                        ((x - track.left - drag.grab_offset)
                            / (track.width() - thumb_width).max(1.0)
                            * max)
                            .clamp(0.0, max),
                    );
                }
                return true;
            }
            DragTarget::Split => {
                self.split.ratio = ((x - drag.grab_offset - self.split.full.left)
                    / self.split.full.width().max(1.0))
                .clamp(0.2, 0.8);
            }
            DragTarget::Navigation => {
                self.state.navigation_width = clamps.navigation(x - drag.grab_offset);
            }
            DragTarget::SettingsNav => {
                if let Some(bar) = self.prefs.nav_layout.scrollbar {
                    let travel = (bar.track.height() - bar.thumb.height()).max(1.0);
                    let ratio = ((y - bar.track.top - drag.grab_offset) / travel).clamp(0.0, 1.0);
                    self.prefs.nav_scroll = bar.max * ratio;
                }
                return true;
            }
            DragTarget::Sidebar => {
                let nav_right = self.nav_layout.scroll_area.right.max(0.0);
                self.state.sidebar_width =
                    (x - drag.grab_offset - nav_right).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
            }
            DragTarget::AiPanel => {
                // 右侧栏靠右，宽度是「窗口右缘减去面板左缘」
                let left = x - drag.grab_offset;
                self.state.ai_panel_width = clamps.ai_panel(window_width - left, window_width);
            }
            DragTarget::ViewerPan => return self.on_viewer_pan(x, y) || changed,
            DragTarget::CanvasCard => return self.canvas_drag_to(x, y) || changed,
            DragTarget::CanvasPan => return self.canvas_pan_to(x, y) || changed,
            DragTarget::PdfAnnotation => return self.pdf_drag_to(x, y) || changed,
            DragTarget::EditorSelect => {
                // 拖选：锚点不动，光标跟着指针
                let (area, scroll) = (self.editor_area, self.shell.active_scroll());
                let content = self.content();
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    let before = buffer.cursor();
                    match content {
                        MainContent::Document => self.doc.click(area, buffer, scroll, x, y, true),
                        MainContent::Source => self.source.click(area, buffer, scroll, x, y, true),
                        _ => {}
                    }
                    if buffer.cursor() != before {
                        // 光标可能跨块，活动块要跟着换
                        if content == MainContent::Document {
                            self.after_doc_edit(false);
                        }
                        return true;
                    }
                }
                return changed;
            }
            DragTarget::ProviderFieldSelect => {
                let Some(form) = self.prefs.providers.form.as_mut() else {
                    return changed;
                };
                let Some(rect) = self.prefs.provider_layout.field(form.focus) else {
                    return changed;
                };
                if form.focus == 3 {
                    form.fields[form.focus].click_masked(x - rect.left - 12.0, true);
                } else {
                    form.fields[form.focus].click(x - rect.left - 12.0, true);
                }
                return true;
            }
            DragTarget::SettingsFieldSelect => {
                let Some(edit) = self.prefs.editing.as_mut() else {
                    return changed;
                };
                let Some(rect) = self.prefs.content_layout.control_rect(edit.index) else {
                    return changed;
                };
                edit.field.click(x - rect.left - 12.0, true);
                return true;
            }
            DragTarget::EditorTableColumn => return self.resize_editor_table_column(x),
        }
        // 编辑器宽度变了，正文要重排
        self.invalidate_main();
        true
    }

    pub fn is_dragging(&self) -> bool {
        if self.menu.as_ref().is_some_and(Menu::is_scrollbar_dragging)
            || self
                .wiki_suggestion
                .as_ref()
                .is_some_and(|s| s.menu.is_scrollbar_dragging())
        {
            return true;
        }
        if self.desktop.panel.as_ref().is_some_and(|p| p.dragging()) {
            return true;
        }
        if self.nav.library_drag.is_some() {
            return true;
        }
        if self.workflows.view.drag.is_some() {
            return true;
        }
        if self.notifications.view.scrollbar.dragging() {
            return true;
        }
        self.drag.is_some()
            || self.scrollbars.interaction.dragging()
            || self.ai.panel.horizontal.dragging()
            || matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.dragging())
    }

    /// 鼠标抬起时再采样一次坐标。Windows 在拖动很快、或刚越过窗口边界后松开时，
    /// 不保证会在 `WM_LBUTTONUP` 前再发一条 `WM_MOUSEMOVE`；如果只依赖 move 事件，
    /// 文件夹会被误判成普通单击而折叠。
    pub fn end_drag_at(&mut self, x: f32, y: f32) {
        if let Some(menu) = self.menu.as_mut() {
            if menu.is_scrollbar_dragging() {
                menu.pointer_move(x, y);
                menu.end_scrollbar_drag();
                return;
            }
        }
        if let Some(suggestion) = self.wiki_suggestion.as_mut() {
            if suggestion.menu.is_scrollbar_dragging() {
                suggestion.menu.pointer_move(x, y);
                suggestion.menu.end_scrollbar_drag();
                return;
            }
        }
        if self.desktop_manager_captures_pointer(x, y) {
            self.desktop_manager_pointer(x, y);
            if let Some(p) = self.desktop.panel.as_mut() {
                p.pointer_up();
            }
            return;
        }
        if self.nav.library_drag.is_some() {
            self.finish_navigation_library_drag(x, y);
            return;
        }
        if self.workflows.view.drag.is_some() {
            self.workflows.view.motion(x, y);
            self.workflows.view.release(x, y);
            return;
        }
        if self.notification_open() {
            self.notification_pointer(x, y);
            self.notifications.view.scrollbar.end();
            return;
        }
        self.scrollbar_pointer(x, y);
        if self
            .drag
            .is_some_and(|d| matches!(d.target, DragTarget::CanvasCard | DragTarget::CanvasPan))
        {
            self.canvas_drag_to(x, y);
        }
        if matches!(self.viewer_tab(),Some((_,viewer::Content::Base(state)))if state.dragging()) {
            self.on_base_pointer(x, y);
        }
        if self
            .drag
            .is_some_and(|d| d.target == DragTarget::SidebarTree)
        {
            self.update_sidebar_tree_drag(x, y);
        }
        self.end_drag();
    }

    pub fn end_drag(&mut self) {
        if let Some(menu) = self.menu.as_mut() {
            menu.end_scrollbar_drag();
        }
        if let Some(suggestion) = self.wiki_suggestion.as_mut() {
            suggestion.menu.end_scrollbar_drag();
        }
        if let Some(p) = self
            .desktop
            .panel
            .as_mut()
            .filter(|_| self.object_picker.is_none())
        {
            p.pointer_up();
        }
        if self.cancel_navigation_library_drag() {
            return;
        }
        self.workflows.view.drag = None;
        self.notifications.view.scrollbar.end();
        self.scrollbars.interaction.end();
        if self
            .drag
            .is_some_and(|d| d.target == DragTarget::SidebarTree)
        {
            self.finish_sidebar_tree_drag();
            self.drag = None;
            return;
        }
        if self
            .drag
            .is_some_and(|d| d.target == DragTarget::AiTextSelect)
        {
            self.ai_end_text_selection();
        }
        if self
            .drag
            .is_some_and(|d| d.target == DragTarget::ScheduleBlock)
        {
            self.drag = None;
            self.finish_schedule_drag();
            return;
        }
        let geometry = match self.drag.map(|drag| drag.target) {
            Some(DragTarget::Navigation) => Some(("navigation.width", self.state.navigation_width)),
            Some(DragTarget::Sidebar) => Some(("sidebar.width", self.state.sidebar_width)),
            Some(DragTarget::AiPanel) => {
                Some(("editorLayout.aiPanelWidth", self.state.ai_panel_width))
            }
            Some(DragTarget::Split) => Some(("editorLayout.splitRatio", self.split.ratio)),
            _ => None,
        };
        if let Some((key, value)) = geometry {
            self.persist_chrome_setting(key, SettingValue::Number(f64::from(value)));
        }
        if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
            if state.end_drag() {
                self.schedule_autosave();
            }
        }
        if self.image_drag.take().is_some() {
            self.after_doc_edit(true);
        }
        self.table_drag = None;
        self.table_column_drag = None;
        self.ai.panel.horizontal.end_drag();
        if self.drag.map(|d| d.target) == Some(DragTarget::PdfAnnotation) {
            let completed = match self.viewer_content_mut() {
                Some(viewer::Content::Pdf(s)) => s.annotations.finish(),
                _ => None,
            };
            if let (Some(a), Some((path, viewer::Content::Pdf(s)))) = (completed, self.viewer_tab())
            {
                let path = path.to_path_buf();
                let mut items = s.annotations.items.clone();
                let id = a.id.clone();
                items.push(a);
                if self.pdf_save_annotations(&path, items) {
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
                        s.annotations.selected = Some(id);
                    }
                }
            }
        }
        if self.drag.map(|d| d.target) == Some(DragTarget::ViewerPan) {
            if let Some(viewer::Content::Image(s)) = self.viewer_content_mut() {
                s.dragging = None;
            }
        }
        if self
            .drag
            .is_some_and(|d| matches!(d.target, DragTarget::CanvasCard | DragTarget::CanvasPan))
        {
            self.canvas_release();
        }
        self.drag = None;
        self.ai_float_end_drag();
    }
}
