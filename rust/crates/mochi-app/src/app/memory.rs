//! 主回复完成与可选的记忆提取各自拥有独立生命周期。
use super::*;
impl App {
    pub(super) fn memory_cancel_if_disabled(&mut self) {
        if !crate::memory_runtime::enabled(&self.settings) {
            self.ai.pending_memory = None;
            for job in &self.ai.memory_jobs {
                job.cancel();
            }
        }
    }
    pub(super) fn collect_memory_jobs(&mut self) {
        self.memory_cancel_if_disabled();
        self.ai.memory_jobs.retain(|job| {
            if let Some(result) = job.finish() {
                // 模型/API 的诊断信息不要塞进对话，也不要回传给模型。
                // 失败只是本地一个可选任务的失败。
                if result.error.is_some() {
                    eprintln!("自动记忆本轮未更新；主对话不受影响");
                }
                false
            } else {
                true
            }
        });
    }
    pub(super) fn start_memory_job(&mut self, content: String) {
        self.collect_memory_jobs();
        let Some(exchange) = self.ai.pending_memory.take() else {
            return;
        };
        if self
            .shell
            .workspace()
            .is_none_or(|ws| ws.root != exchange.root)
        {
            return;
        }
        let Some(permissions) = self.ai.permissions.clone() else {
            return;
        };
        if !mochi_core::ai::memory_extract::worth_extracting(&exchange.user_text)
            || content.trim().is_empty()
            || permissions
                .assert_tool_action_allowed(
                    mochi_core::ai::permission::AiToolAction::WriteFile,
                    None,
                )
                .is_err()
        {
            return;
        }
        // 可选的模型调用与应用的请求启动预算共用。预算耗尽就跳过提取；
        // 绝不让已完成的主回复失败或被拖延。
        if !self.allow_ai_request() {
            return;
        }
        let hwnd_raw = self.hwnd_raw;
        if let Some(job) = crate::memory_runtime::spawn(
            exchange,
            content,
            self.settings.clone(),
            permissions,
            move || unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_raw as *mut _)),
                    platform::WM_APP_AI_EVENT,
                    WPARAM(0),
                    LPARAM(0),
                );
            },
        ) {
            self.ai.memory_jobs.push(job);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_memory_does_not_bypass_the_request_start_limit() {
        let root = std::env::temp_dir().join(format!(
            "mochi-memory-limit-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            app.app_settings.write(
                app_settings::descriptor("ai.rateLimitEnabled").unwrap(),
                &SettingValue::Bool(true),
            );
            app.app_settings.write(
                app_settings::descriptor("ai.maxRequestsPerMinute").unwrap(),
                &SettingValue::Number(1.0),
            );
            assert!(app.allow_ai_request());
            app.ai.pending_memory = Some(crate::memory_runtime::Exchange {
                root: app.shell.workspace().unwrap().root.clone(),
                session_id: "session".into(),
                user_text: "今后请始终使用中文回答技术问题".into(),
                provider: mochi_core::ai::models::AiProvider {
                    base_url: "http://127.0.0.1:9".into(),
                    model: "must-not-call".into(),
                    protocol: "openai-completions".into(),
                    ..Default::default()
                },
            });
            app.start_memory_job("好的".into());
            assert!(app.ai.memory_jobs.is_empty());
            assert!(app.ai.pending_memory.is_none());
            assert!(!mochi_core::memory::MemoryStore::db_path(&root).exists());
        }
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn main_reply_finishes_before_extraction_and_disabling_cancels_even_if_reenabled() {
        let _guard = crate::memory_runtime::TEST_LOCK.lock().unwrap();
        let root = std::env::temp_dir().join(format!(
            "mochi-memory-app-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            app.ai_new_session();
            app.ai
                .panel
                .active
                .as_mut()
                .unwrap()
                .messages
                .push(AiStoredMessage::new(
                    "user",
                    "以后请统一用中文回答我的技术问题",
                ));
            let (url, seen, release, server) = crate::memory_runtime::tests::server();
            app.ai.pending_memory = Some(crate::memory_runtime::Exchange {
                root: app.shell.workspace().unwrap().root.clone(),
                session_id: app.ai.panel.active.as_ref().unwrap().id.clone(),
                user_text: "以后请统一用中文回答我的技术问题".into(),
                provider: mochi_core::ai::models::AiProvider {
                    base_url: url,
                    model: "test".into(),
                    protocol: "openai-completions".into(),
                    ..Default::default()
                },
            });
            let (tx, rx) = channel();
            let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.ai.run = Some(RunHandle {
                rx,
                cancel: cancelled,
            });
            app.ai.panel.streaming = Some(assistant::Streaming::default());
            tx.send(RunEvent::Runtime {
                agent_id: None,
                agent_name: "测试助手".into(),
                skills: Vec::new(),
                model: "resolved-memory-model".into(),
            })
            .unwrap();
            tx.send(RunEvent::Done {
                content: "主回复已完成".into(),
                transcript: Vec::new(),
                trace: Vec::new(),
                finish_reason: "stop".into(),
                usage: serde_json::Value::Null,
                usage_source: "estimated".into(),
            })
            .unwrap();
            app.take_ai_events();
            assert!(app.ai.run.is_none());
            assert!(app.ai.panel.streaming.is_none());
            assert_eq!(
                app.ai
                    .panel
                    .active
                    .as_ref()
                    .unwrap()
                    .messages
                    .last()
                    .unwrap()
                    .content(),
                "主回复已完成"
            );
            assert_eq!(app.ai.memory_jobs.len(), 1);
            let session_id = &app.ai.panel.active.as_ref().unwrap().id;
            let saved = app
                .ai_session_service()
                .unwrap()
                .load_session(session_id)
                .unwrap();
            assert_eq!(saved.messages.last().unwrap().content(), "主回复已完成");
            let request = seen
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            assert_eq!(request["model"], "resolved-memory-model");
            let descriptor = app_settings::descriptor("ai.memoryAutoExtract").unwrap();
            app.app_settings
                .write(descriptor, &SettingValue::Bool(false));
            app.apply_setting_side_effects("ai.memoryAutoExtract");
            app.app_settings
                .write(descriptor, &SettingValue::Bool(true));
            app.apply_setting_side_effects("ai.memoryAutoExtract");
            release.send(()).unwrap();
            server.join().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !app.ai.memory_jobs.is_empty() {
                assert!(std::time::Instant::now() < deadline);
                app.collect_memory_jobs();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(mochi_core::memory::MemoryStore::open(&root)
                .unwrap()
                .list(None)
                .unwrap()
                .is_empty());
            assert_eq!(
                app.ai
                    .panel
                    .active
                    .as_ref()
                    .unwrap()
                    .messages
                    .last()
                    .unwrap()
                    .content(),
                "主回复已完成"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
