//! 窗口过程持有鼠标捕获；这里的选区操作由侧栏和独立 AI 页共用。

use super::*;

impl App {
    fn ai_panel_visible_for_interaction(&self) -> bool {
        (self.state.ai_panel_open && self.state.right_panel == RightPanel::Assistant)
            || self.state.view == WorkspaceView::MochiAi
            || self.ai.float.is_some()
    }

    /// 开始一次渲染消息的选区。公式/表格/代码的复制按钮等控件
    /// 返回 false，让既有的点击动作保持优先。
    pub(super) fn ai_begin_text_selection(&mut self, x: f32, y: f32) -> bool {
        if !self.ai_panel_visible_for_interaction()
            || self.ai.panel.is_streaming() && self.ai.layout.messages.is_empty()
        {
            return false;
        }
        if self
            .ai
            .layout
            .content_at(self.ai.panel.scroll, x, y, false)
            .is_some()
        {
            return false;
        }
        // 普通点击行内链接时，仍要触达 `on_assistant_click` 里既有的
        // 链接动作。只有没有更高优先级语义点击目标的文字才启动选区；
        // 否则第一次按下鼠标就把点击消费掉了，链接看起来就像坏了。
        if self.ai.layout.link_at(self.ai.panel.scroll, x, y).is_some() {
            return false;
        }
        let Some((message, point)) = self.ai.layout.text_point_at(self.ai.panel.scroll, x, y)
        else {
            return false;
        };
        self.ai.panel.begin_text_selection(message, point);
        true
    }

    /// 扩展当前渲染消息的选区。指针落在正文之外时，布局会钳制到
    /// 最近的文字边缘，与原生控件在拖出消息视口时的表现一致。
    pub(super) fn ai_update_text_selection(&mut self, x: f32, y: f32) -> bool {
        if !self.ai_panel_visible_for_interaction()
            || self
                .ai
                .panel
                .text_selection
                .is_none_or(|selection| !selection.dragging)
        {
            return false;
        }
        let Some((message, point)) = self.ai.layout.text_point_near(self.ai.panel.scroll, x, y)
        else {
            return false;
        };
        self.ai
            .panel
            .update_text_selection_clamped(&self.ai.layout, message, point)
    }

    /// 结束选区，报告是否留下了非空范围。没有移动的点击直接丢弃，
    /// 用户回到普通面板操作后就不会残留一个过期的复制入口。
    pub(super) fn ai_end_text_selection(&mut self) -> bool {
        let selected = self.ai.panel.end_text_selection();
        if !selected {
            self.ai.panel.clear_text_selection();
        }
        selected
    }

    pub(super) fn ai_clear_text_selection(&mut self) {
        self.ai.panel.clear_text_selection();
    }

    /// 复制当前渲染选区。AI 消息是持久化的回复，Ctrl+X 被消费掉
    /// 但不改动会话；快捷键保持安全，编辑器也不会吞掉这个按键事件。
    pub(super) fn ai_copy_selection(&mut self, cut: bool) -> bool {
        self.ai_copy_selection_with(cut, platform::copy_to_clipboard)
    }

    /// 可注入的复制路径，应用快捷键和离屏交互测试都用它。
    /// 写入端留在边界上，几何测试期间选中 AI 回复
    /// 就不必碰编辑器缓冲或 OS 剪贴板。
    pub(super) fn ai_copy_selection_with(
        &mut self,
        cut: bool,
        write: impl FnOnce(&str) -> bool,
    ) -> bool {
        let Some(text) = self.ai.panel.selected_text(&self.ai.layout) else {
            return false;
        };
        if text.is_empty() {
            return true;
        }
        if write(&text) {
            self.show_global_notice(if cut {
                "AI 回复已复制（回复内容不可剪切）"
            } else {
                "已复制 AI 文本"
            });
        } else {
            self.show_global_notice("复制失败，请重试");
        }
        true
    }

    pub(super) fn ai_selected_text(&self) -> Option<String> {
        self.ai.panel.selected_text(&self.ai.layout)
    }

    /// 右键是否落在已选中的渲染范围内。调用方可借此让原生上下文菜单的
    /// 复制动作优先于单条消息的菜单，而不必另造一个命中矩形。
    pub(super) fn ai_selection_contains(&self, x: f32, y: f32) -> bool {
        let Some(selection) = self.ai.panel.text_selection else {
            return false;
        };
        if selection.collapsed() || selection.anchor.0 != selection.focus.0 {
            return false;
        }
        self.ai
            .panel
            .selection_rects(&self.ai.layout, selection.anchor.0, self.ai.panel.scroll)
            .into_iter()
            .any(|rect| rect.contains(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::ai::session::{AiConversation, AiStoredMessage};

    #[test]
    fn assistant_drag_selection_copies_rendered_reply_without_os_clipboard() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-ai-selection-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let settings = Arc::new(mochi_core::settings::SettingsService::new(Some(
            root.join("settings.json"),
        )));
        let mut app = App::with_settings(settings).unwrap();
        app.state.ai_panel_open = true;
        app.state.right_panel = RightPanel::Assistant;
        app.ai.panel.active = Some(AiConversation {
            id: "selection-test".into(),
            title: "选择测试".into(),
            messages: vec![
                AiStoredMessage::new("assistant", "前缀 **复制这段** 后缀"),
                AiStoredMessage::new("user", "下一条消息不应吞进选区"),
            ],
            ..Default::default()
        });

        // 与快照验证使用同一个软件渲染目标。这样在任何指针命中测试之前
        // 都跑完了完整的 App 绘制/布局路径。
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.paint(windows::Win32::Foundation::HWND::default())
            .unwrap();

        let message_index = app
            .ai
            .layout
            .messages
            .iter()
            .position(|message| message.role == "assistant")
            .unwrap();
        let message = &app.ai.layout.messages[message_index];
        let fragments = message.body.selectable_text(Some(&message.horizontal));
        assert!(fragments.len() >= 2, "styled reply should expose text runs");
        let origin = app
            .ai
            .layout
            .body_origin(message_index, app.ai.panel.scroll)
            .unwrap();
        let first = fragments.first().unwrap();
        let last = fragments.last().unwrap();
        let start = (
            origin.0 + first.rect.left + 1.0,
            origin.1 + (first.rect.top + first.rect.bottom) / 2.0,
        );
        let end = (
            origin.0 + last.rect.right - 1.0,
            origin.1 + (last.rect.top + last.rect.bottom) / 2.0,
        );

        app.on_click(start.0, start.1);
        assert_eq!(
            app.drag.map(|drag| drag.target),
            Some(DragTarget::AiTextSelect)
        );
        assert!(app.on_mouse_move(end.0, end.1));
        // 越过下一个气泡时，钳制到锚点消息的最后一个可视 run；
        // 选中的回复仍可复制，而不是变成一个无效的跨消息范围消失掉。
        let next = &app.ai.layout.messages[1];
        let next_origin = app.ai.layout.body_origin(1, app.ai.panel.scroll).unwrap();
        let next_fragment = next.body.selectable_text(None).first().unwrap().clone();
        assert!(app.on_mouse_move(
            next_origin.0 + next_fragment.rect.left + 1.0,
            next_origin.1 + (next_fragment.rect.top + next_fragment.rect.bottom) / 2.0,
        ));
        assert_eq!(app.ai.panel.text_selection.unwrap().focus.0, message_index);
        app.end_drag();
        assert!(!app.ai.panel.text_selection.unwrap().dragging);
        let selected = app.ai_selected_text().unwrap();
        assert_eq!(selected, "前缀 复制这段 后缀");

        let mut copied = None;
        assert!(app.ai_copy_selection_with(false, |text| {
            copied = Some(text.to_owned());
            true
        }));
        assert_eq!(copied.as_deref(), Some(selected.as_str()));

        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn assistant_link_click_keeps_link_action_available() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-ai-link-selection-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let settings = Arc::new(mochi_core::settings::SettingsService::new(Some(
            root.join("settings.json"),
        )));
        let mut app = App::with_settings(settings).unwrap();
        app.state.ai_panel_open = true;
        app.state.right_panel = RightPanel::Assistant;
        app.ai.panel.active = Some(AiConversation {
            id: "link-selection-test".into(),
            title: "链接测试".into(),
            messages: vec![AiStoredMessage::new(
                "assistant",
                "打开 [会话](mochi://ai-session?session=link-selection-test&label=链接测试)",
            )],
            ..Default::default()
        });
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.paint(windows::Win32::Foundation::HWND::default())
            .unwrap();
        let message_index = app
            .ai
            .layout
            .messages
            .iter()
            .position(|message| message.role == "assistant")
            .unwrap();
        let message = &app.ai.layout.messages[message_index];
        let link = message
            .body
            .selectable_text(None)
            .into_iter()
            .find(|fragment| fragment.text == "会话")
            .unwrap();
        let origin = app
            .ai
            .layout
            .body_origin(message_index, app.ai.panel.scroll)
            .unwrap();
        let x = origin.0 + (link.rect.left + link.rect.right) / 2.0;
        let y = origin.1 + (link.rect.top + link.rect.bottom) / 2.0;
        app.on_click(x, y);
        assert!(app.drag.is_none(), "a link click must not start text drag");
        assert!(app.ai.panel.text_selection.is_none());
        assert!(app.ai.layout.link_at(app.ai.panel.scroll, x, y).is_some());

        app.ai.panel.active.as_mut().unwrap().messages = vec![AiStoredMessage::new(
            "assistant",
            &"长回复不会覆盖底部控件的点击区域。\n\n".repeat(100),
        )];
        app.ai.panel.scroll = 0.0;
        app.ai.panel.stick_to_bottom = false;
        app.paint(HWND::default()).unwrap();
        let agent = app.ai.layout.rect_of(assistant::Hit::AgentPicker).unwrap();
        let x = (agent.left + agent.right) / 2.0;
        let y = (agent.top + agent.bottom) / 2.0;
        assert!(app.ai.layout.text_point_at(0.0, x, y).is_none());
        app.on_click(x, y);
        assert!(
            app.menu.is_some(),
            "clipped long replies must not swallow the agent picker"
        );
        assert!(app.drag.is_none());
        app.menu = None;
        let input = app.ai.layout.rect_of(assistant::Hit::Input).unwrap();
        app.on_click(input.left + 30.0, (input.top + input.bottom) / 2.0);
        assert_eq!(app.focus, Focus::AiInput);

        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
