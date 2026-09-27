//! 绘制应用主界面，并协调焦点、输入活动和光标刷新。
use super::*;

impl App {
    pub fn paint(&mut self, hwnd: HWND) -> Result<()> {
        if !hwnd.is_invalid()
            && unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } == hwnd
        {
            self.acknowledge_visible_documents();
        }
        self.poll_object_picker();
        self.desktop_agent_requests();
        self.console_poll_jobs();
        self.console_agent_requests(hwnd);
        self.window_motion.sync(
            [
                self.settings_overlay.is_some(),
                self.object_picker.is_some(),
                self.link_create.is_some(),
                self.mapped_folder.is_some(),
                self.dialog.is_some(),
                self.export_form.is_some(),
                self.notification_open(),
                self.template_picker.is_some() || self.global_import.is_some(),
            ],
            !hwnd.is_invalid()
                && !self.window_motion.keyboard
                && platform::client_area_animations_enabled(),
        );
        if self.sync_document_format_changes() {
            self.sync_state();
        }
        if self.state.view == WorkspaceView::QuickNote && !self.ensure_quick_note_tab() {
            self.state.view = WorkspaceView::Editor;
        }
        if self.object_picker.is_none()
            && self.global_import.is_none()
            && self.dialog.is_none()
            && self.title_editing.is_none()
            && self.commands.review.is_none()
        {
            if let Some(path) = self
                .ai
                .host
                .as_ref()
                .and_then(|h| h.take_knowledge_request())
            {
                let index = self.shell.workspace().and_then(|w| {
                    w.libraries.iter().position(|l| {
                        l.path
                            .replace('\\', "/")
                            .eq_ignore_ascii_case(&path.to_string_lossy().replace('\\', "/"))
                    })
                });
                if let Some(index) = index {
                    self.shell.select_library(index);
                    self.state.view = WorkspaceView::Editor;
                    self.sync_state();
                }
            }
        }
        if self.object_picker.is_none()
            && self.global_import.is_none()
            && self.dialog.is_none()
            && self.title_editing.is_none()
            && self.commands.review.is_none()
        {
            if let Some(tab) = self
                .ai
                .host
                .as_ref()
                .and_then(|h| h.take_settings_request())
            {
                self.open_settings(&tab);
            }
        }
        // 首帧也必须先有真实 DIP 视口；离屏验收已有目标，不会创建窗口。
        self.renderer.ensure_target(hwnd)?;
        let chrome = self.build_chrome();
        let palette = theme::configured_palette(self.state.dark);

        self.list.clear();
        chrome.paint(&self.state, &mut self.list);
        self.paint_navigation(&chrome, &palette);
        self.paint_tab_bar(&chrome, &palette);
        self.paint_caption_buttons(&palette, unsafe { IsZoomed(hwnd).as_bool() });
        self.paint_main(&chrome, &palette);
        self.paint_editor_comments(&palette);
        self.paint_editor_controls(&palette);
        if self.content() == MainContent::Document && self.dialog.is_none() && self.menu.is_none() {
            if let Some((x, y)) = self.block_pointer {
                if let Some((rect, _, empty)) =
                    self.doc
                        .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
                {
                    self.list.rounded_rect(rect, 4.0, palette.surface);
                    self.list.icon_centered(
                        rect,
                        if empty {
                            Icon::PLUS
                        } else {
                            Icon::MORE_HORIZONTAL
                        },
                        16.0,
                        palette.muted,
                    );
                }
            }
        }
        self.paint_sidebar(&chrome, &palette);
        self.paint_right_sidebar(&chrome, &palette);
        self.paint_scrollbars(&palette);
        self.paint_status(&chrome, &palette);
        self.paint_notification_chrome(&chrome, &palette);
        let split_button = tab_bar::split_button_rect(chrome.tree.rect(chrome.tab_bar));
        if !chrome.tree.is_hidden(chrome.tab_bar)
            && self
                .block_pointer
                .is_some_and(|(x, y)| split_button.contains(x, y))
            && self.menu.is_none()
            && self.dialog.is_none()
        {
            let tip = Rect::from_size(
                (split_button.right - 156.0).max(0.0),
                split_button.bottom + 6.0,
                156.0,
                30.0,
            );
            self.list.rounded_rect(tip, 5.0, palette.surface);
            let label = if self
                .shell
                .active()
                .is_some_and(|tab| tab.buffer().is_some())
            {
                "向右拆分编辑器"
            } else {
                "仅支持文本编辑标签"
            };
            self.list.text_aligned(
                tip,
                label,
                TextStyle::Caption,
                palette.foreground,
                Align::Center,
            );
        }
        self.paint_base_modal(&palette);
        if self.automation.panel.is_some() {
            self.list.clear_carets();
        }
        self.paint_automations(&palette);
        if self.settings_overlay.is_some() {
            self.list.clear_carets();
        }
        let start = self.list.cmds().len();
        self.paint_settings_overlay(&palette);
        self.list.fade_since(
            start,
            self.renderer.viewport(),
            self.window_motion.opacity(0),
        );
        self.refresh_wiki_suggestion();
        if let Some(s) = self.wiki_suggestion.as_mut() {
            s.menu.constrain_to_viewport(self.renderer.viewport());
            s.menu.paint(&mut self.list, &palette);
        }
        if self.ai.float.is_some() && matches!(self.focus, Focus::AiInput | Focus::AiMessageQuery) {
            self.list.clear_carets();
        }
        self.paint_ai_float(&palette);
        // 覆盖层最后画：菜单在一切之上，对话框在菜单之上
        if let Some(menu) = &mut self.menu {
            // 窗口缩放后已打开的菜单要继续可用。锚定菜单在打开时就完成了布局，
            // 所以绘制前把矩形（含所有打开的子菜单）夹紧到当前视口，帧绘制
            // 是最后的安全时机。
            menu.constrain_to_viewport(self.renderer.viewport());
            if self.slash_trigger.is_none() || menu.search.is_some() {
                self.list.clear_carets();
            }
            menu.paint(&mut self.list, &palette);
        }
        if let Some(picker) = &self.table_picker {
            self.list.clear_carets();
            picker.paint(&mut self.list, &palette);
        }
        if let Some(src) = &self.image_preview {
            self.list.clear_carets();
            let viewport = self.renderer.viewport();
            self.list.rect_alpha(viewport, 0x000000, 0.8);
            self.list.image(
                Rect::new(
                    viewport.left + 48.0,
                    viewport.top + 48.0,
                    viewport.right - 48.0,
                    viewport.bottom - 48.0,
                ),
                src,
                "图片预览",
            );
            self.list.icon_centered(
                Rect::from_size(viewport.right - 46.0, 12.0, 32.0, 32.0),
                Icon::X,
                24.0,
                0xffffff,
            );
        }
        let viewport = self.renderer.viewport();
        if let Some(s) = self.search.as_mut() {
            let layout = search::layout(s, viewport);
            s.scroll = s.scroll.min(layout.max_scroll());
            search::paint(&mut self.list, s, &layout, viewport, &palette);
            self.search_layout = layout;
        }
        if let Some(c) = self.command.as_mut() {
            // 行矩形带着滚动偏移：先夹住滚动，再按夹住后的值排一次
            let max = command::layout(viewport, c).max_scroll();
            c.scroll = c.scroll.min(max);
            let layout = command::layout(viewport, c);
            command::paint(&mut self.list, viewport, &layout, c, &palette);
            self.command_layout = layout;
        }
        if let Some(dialog) = self.link_create.as_mut() {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            dialog.paint(&mut self.list, viewport, &palette);
            self.list
                .fade_since(start, viewport, self.window_motion.opacity(2));
        }
        if let Some(dialog) = self.mapped_folder.as_mut() {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            dialog.paint(&mut self.list, viewport, &palette);
            self.list
                .fade_since(start, viewport, self.window_motion.opacity(3));
        }
        if let Some(dialog) = self.dialog.as_mut() {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            dialog.paint(&mut self.list, viewport, &palette);
            self.list
                .fade_since(start, viewport, self.window_motion.opacity(4));
        }
        if let Some(form) = &self.export_form {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            form.paint(&mut self.list, viewport, &palette);
            self.list
                .fade_since(start, viewport, self.window_motion.opacity(5));
        }
        if let Some(picker) = &self.template_picker {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            picker.paint(&mut self.list, viewport, &palette);
            self.list
                .fade_since(start, viewport, self.window_motion.opacity(7));
        }
        // 裁剪栈必须成对出入，否则裁剪区域会泄漏到后续绘制。
        // debug 档直接崩在这里，比让它悄悄画错好。
        debug_assert!(self.list.finish().is_ok(), "裁剪栈没配平");

        if crate::ui::settings_values::boolean("background.enabled", false) {
            if let Some((src, size)) = &self.background {
                crate::ui::background::apply(&mut self.list, src, *size, viewport, &palette);
            }
        }
        self.paint_resource_detail(&palette);
        if let Some(review) = self.commands.review.as_mut() {
            self.list.clear_carets();
            crate::ui::command_review::paint(&mut self.list, viewport, review, &palette);
        }
        // 全局提示始终位于最上层；复制可能发生在编辑器、AI、弹窗或设置页。
        let notice_area = Rect::new(
            viewport.left,
            chrome
                .tree
                .rect(chrome.title_bar)
                .bottom
                .max(chrome.tree.rect(chrome.tab_bar).bottom)
                .max(self.toolbar_area.bottom)
                .max(self.split.toolbar.bottom)
                .max(chrome.tree.rect(chrome.right_sidebar_toolbar).bottom),
            viewport.right,
            viewport.bottom,
        );
        self.status_bar
            .toast
            .paint(&mut self.list, notice_area, Self::now_ms(), &palette);
        if self.notification_open() {
            self.list.clear_carets();
        }
        let start = self.list.cmds().len();
        self.paint_notifications(&palette);

        self.list
            .fade_since(start, viewport, self.window_motion.opacity(6));
        if let Some(dialog) = self.object_picker.as_mut() {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            crate::ui::object_picker::paint(
                &mut dialog.state,
                &mut self.list,
                self.renderer.viewport(),
                &palette,
            );
            self.list.fade_since(
                start,
                self.renderer.viewport(),
                self.window_motion.opacity(1),
            );
        }
        if let Some(pending) = &self.global_import {
            self.list.clear_carets();
            let start = self.list.cmds().len();
            pending.view.paint(&mut self.list, viewport, &palette);
            self.list
                .fade_since(start, viewport, self.window_motion.opacity(7));
        }
        let hovered = self
            .window_motion
            .pointer_pos
            .and_then(|(x, y)| self.window_hover_rect(x, y));
        self.window_motion.pointer(hovered);
        self.window_motion.paint_hover(&mut self.list, &palette);
        self.sync_remote_images();
        self.input_feedback
            .geometry(self.list.caret_rect(), crate::ui::input_feedback::now_ms());
        self.list.set_caret_visible(self.input_feedback.visible(
            crate::ui::input_feedback::now_ms(),
            crate::platform::caret_blink_period(),
        ));
        self.renderer.present(hwnd, palette.background, &self.list)
    }

    pub fn input_activity(&mut self) {
        self.input_feedback
            .activity(crate::ui::input_feedback::now_ms());
    }

    pub fn input_window_focus(&mut self, focused: bool) {
        self.input_feedback
            .focus(focused, crate::ui::input_feedback::now_ms());
        if !focused {
            self.on_ime_cancel();
        }
    }

    pub fn caret_timer_delay(&self) -> Option<u32> {
        self.input_feedback.next_tick(
            crate::ui::input_feedback::now_ms(),
            crate::platform::caret_blink_period(),
        )
    }

    /// 光标闪烁只重放已构建好的帧。绝不为了切换光标而重读文件、扫描工作区
    /// 或重排大文档。
    pub fn paint_caret_tick(&mut self, hwnd: HWND) {
        let show = self.input_feedback.visible(
            crate::ui::input_feedback::now_ms(),
            crate::platform::caret_blink_period(),
        );
        if self.list.set_caret_visible(show) {
            let p = theme::configured_palette(self.state.dark);
            let _ = self.renderer.present(hwnd, p.background, &self.list);
        }
    }
}
