//! 根据 AI 提供的位置线索定位工作区中的记录。
use super::*;
use mochi_core::ai::locator::{self, Locator};
impl App {
    pub(super) fn ai_locate(&mut self, url: &str) {
        let fail = |app: &mut Self, message: &str| {
            app.state.status_text = format!("定位失败：{message}");
        };
        let Some(reference) = Locator::parse(url) else {
            fail(self, "定位标识无效");
            return;
        };
        if !locator::safe_session_id(&reference.session_id) {
            fail(self, "会话标识无效");
            return;
        }
        let Some(service) = self.ai_session_service() else {
            fail(self, "工作区不可用");
            return;
        };
        if !service
            .load_index()
            .sessions
            .iter()
            .any(|s| s.id == reference.session_id)
        {
            fail(self, "该会话可能已被删除");
            return;
        }
        let same = self
            .ai
            .panel
            .active
            .as_ref()
            .is_some_and(|c| c.id == reference.session_id);
        let conversation = if same {
            self.ai.panel.active.clone()
        } else {
            self.shell
                .workspace()
                .and_then(|ws| locator::load_session(&ws.root, &reference.session_id))
        };
        let Some(conversation) = conversation else {
            fail(self, "会话无法读取");
            return;
        };
        let found = conversation
            .messages
            .iter()
            .any(|m| locator::visible(m) && m.id() == Some(reference.message_id.as_str()));
        if !same {
            self.ai_cancel();
            self.ai.panel.active = Some(conversation);
            self.refresh_ai_images();
            self.ai.panel.loaded_skills.clear();
            self.settings
                .set("ai.activeSessionId", &reference.session_id);
        }
        self.ai.panel.show_conversations = false;
        self.ai.panel.error.clear();
        self.ai.panel.locator_pending = found.then_some(reference.message_id);
        self.ai.panel.located = None;
        self.ai.panel.locator_timer = found.then_some(2500);
        self.ai.panel.stick_to_bottom = false;
        self.state.ai_panel_open = true;
        self.set_right_panel(RightPanel::Assistant);
        self.editor_engaged = false;
        self.invalidate_main();
        self.sync_state();
        if found {
            self.state.status_text = "已定位到 AI 问答".into();
        } else {
            fail(self, "该问答可能已被删除");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locating_is_read_only_highlights_the_target_and_rejects_missing_records() {
        let parent = std::env::temp_dir().join(format!(
            "mochi-ai-locate-{}",
            mochi_core::paths::random_base36(12)
        ));
        let root = parent.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                parent.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            let svc = AiSessionService::new(&root);
            let mut conversation = AiConversation {
                id: "target".into(),
                title: "定位会话".into(),
                ..Default::default()
            };
            for i in 0..20 {
                for role in ["user", "assistant"] {
                    let mut m = AiStoredMessage::new(role, &format!("第 {i} 轮 {role}"));
                    m.set("id", serde_json::json!(format!("{role}-{i}")));
                    conversation.messages.push(m);
                }
            }
            let mut internal = AiStoredMessage::new("assistant", "INTERNAL");
            internal.set("id", serde_json::json!("internal"));
            internal.set("tool_calls", serde_json::json!([{"id":"call"}]));
            conversation.messages.push(internal);
            svc.save_session(&conversation).unwrap();
            let mut index = svc.load_index();
            index.sessions.push(mochi_core::ai::session::AiSessionMeta {
                id: "target".into(),
                ..Default::default()
            });
            svc.save_index(&index).unwrap();
            let session_file = svc.base_path().join("sessions/target.json");
            let original = std::fs::read(&session_file).unwrap();
            let url = Locator {
                session_id: "target".into(),
                message_id: "assistant-19".into(),
                ..Default::default()
            }
            .to_url();
            app.open_link(&url);
            assert!(app.state.ai_panel_open);
            assert_eq!(app.ai.panel.active.as_ref().unwrap().id, "target");
            let layout = assistant::layout(&app.ai.panel, Rect::from_size(0.0, 0.0, 420.0, 650.0));
            app.ai.panel.apply_locator(&layout);
            assert!(app.ai.panel.scroll > 0.0);
            assert_eq!(app.ai.panel.located.as_deref(), Some("assistant-19"));
            assert!(!app.ai.panel.stick_to_bottom);
            assert!(app
                .take_timer_requests()
                .contains(&(platform::TIMER_AI_LOCATOR, 2500)));
            app.ai.panel.active.as_mut().unwrap().messages[39].set_content("未落盘的新答案");
            app.open_link(&url);
            assert_eq!(
                app.ai.panel.active.as_ref().unwrap().messages[39].content(),
                "未落盘的新答案"
            );
            for bad in [
                "mochi://ai-locate?session=..%2Foutside&message=m",
                "mochi://ai-locate?session=missing&message=m",
                "mochi://ai-locate?session=target&message=internal",
            ] {
                app.open_link(bad);
                assert!(app.state.status_text.starts_with("定位失败"));
                assert_eq!(app.ai.panel.active.as_ref().unwrap().id, "target");
            }
            app.on_timer(HWND::default(), platform::TIMER_AI_LOCATOR);
            assert!(app.ai.panel.located.is_none());
            assert_eq!(std::fs::read(&session_file).unwrap(), original);
        }
        std::fs::remove_dir_all(parent).unwrap();
    }
}
