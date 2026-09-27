//! 追问生成、任务收集、点击填入与交互弹窗。
use super::*;
use mochi_core::ai::{
    agent_config::{AgentConfigService, GENERAL_ASSISTANT_ID},
    compute_response_revision, get_message_follow_up,
    session::AiSessionService,
    set_message_follow_up, FollowUpFrequency, FollowUpState, FollowUpStatus,
};

impl App {
    pub(super) fn cancel_follow_up_jobs(&mut self) {
        for job in &self.ai.follow_up_jobs {
            job.cancel();
        }
        self.ai.follow_up_jobs.clear();
    }

    pub(super) fn collect_follow_up_jobs(&mut self) {
        if self.ai.panel.follow_up_frequency == FollowUpFrequency::Never {
            self.cancel_follow_up_jobs();
            return;
        }
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            self.ai.follow_up_jobs.clear();
            return;
        };
        let session_svc = AiSessionService::new(&root);
        let mut dirty = false;

        self.ai.follow_up_jobs.retain(|job| {
            if let Some(res) = job.finish() {
                let target = res.target;
                let state = match res.outcome {
                    Ok(questions) if (3..=5).contains(&questions.len()) => {
                        FollowUpState::new_ready(&target.revision, questions)
                    }
                    Ok(_) => FollowUpState::new_skipped(&target.revision),
                    Err(_) => FollowUpState::new_failed(&target.revision),
                };

                // 优先检查当前激活会话
                let mut saved = false;
                if let Some(active) = self.ai.panel.active.as_mut() {
                    if active.id == target.session_id {
                        if let Some(msg) = active
                            .messages
                            .iter_mut()
                            .find(|m| m.id() == Some(&target.message_id) && m.role() == "assistant")
                        {
                            let current_rev =
                                compute_response_revision(&target.message_id, msg.content());
                            if current_rev == target.revision {
                                set_message_follow_up(msg, state.clone());
                                saved = true;
                                dirty = true;
                            }
                        }
                    }
                }

                // 若非当前激活会话，落入目标已保存文件（前提是目标消息未被删除且版本仍一致）
                if !saved {
                    if let Some(mut conv) = session_svc.load_session(&target.session_id) {
                        if let Some(msg) = conv
                            .messages
                            .iter_mut()
                            .find(|m| m.id() == Some(&target.message_id) && m.role() == "assistant")
                        {
                            let current_rev =
                                compute_response_revision(&target.message_id, msg.content());
                            if current_rev == target.revision {
                                set_message_follow_up(msg, state);
                                let _ = session_svc.save_session(&conv);
                            }
                        }
                    }
                }

                false
            } else {
                true
            }
        });

        if dirty {
            self.ai_persist_active();
            self.invalidate_main();
        }
    }

    pub(super) fn start_follow_up_job(&mut self, reply_index: usize, content: String) {
        if self.ai.panel.follow_up_frequency == FollowUpFrequency::Never {
            return;
        }
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let (active_id, msg_id, revision, session_messages) = {
            let Some(active) = self.ai.panel.active.as_mut() else {
                return;
            };
            let Some(message) = active.messages.get_mut(reply_index) else {
                return;
            };
            if message.role() != "assistant" || message.is_hidden() {
                return;
            }
            if message
                .get("interrupted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return;
            }
            let Some(msg_id) = message.id().map(str::to_owned) else {
                return;
            };
            let revision = compute_response_revision(&msg_id, &content);

            // 幂等检查：同一个版本最多发起一次生成尝试
            if let Some(existing) = get_message_follow_up(message) {
                if existing.response_revision == revision {
                    return;
                }
            }
            if self.ai.follow_up_jobs.iter().any(|j| {
                j.target.session_id == active.id
                    && j.target.message_id == msg_id
                    && j.target.revision == revision
            }) {
                return;
            }

            // 认领：写入 generating 状态
            set_message_follow_up(message, FollowUpState::new_generating(&revision));
            (
                active.id.clone(),
                msg_id,
                revision,
                active.messages[..reply_index].to_vec(),
            )
        };
        self.ai_persist_active();

        let agent_svc = AgentConfigService::new(&root);
        let agent_id = self
            .ai
            .panel
            .selected_agent_id
            .as_deref()
            .unwrap_or(GENERAL_ASSISTANT_ID);
        let agent = agent_svc.find_agent(Some(agent_id));
        let frequency = agent
            .as_ref()
            .map(|a| a.follow_up_frequency)
            .unwrap_or(self.ai.panel.follow_up_frequency);
        if frequency == FollowUpFrequency::Never {
            return;
        }

        // 提取本轮用户问题与历史上下文
        let current_user_text = session_messages
            .iter()
            .rev()
            .find(|m| m.role() == "user" && !m.is_hidden())
            .map(|m| m.content().to_owned())
            .unwrap_or_default();

        let Some(req) = mochi_core::ai::follow_up::build_follow_up_request(
            frequency,
            agent.as_ref(),
            &session_messages,
            &current_user_text,
            &content,
        ) else {
            return;
        };

        // 读取当前 Provider
        let Some(provider) = ai_runtime::load_provider(&self.settings) else {
            return;
        };
        let hwnd_raw = self.hwnd_raw;
        let target = crate::follow_up_runtime::FollowUpTarget {
            session_id: active_id,
            message_id: msg_id,
            revision,
        };

        let job = crate::follow_up_runtime::spawn(target, provider, req, move || unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd_raw as *mut _)),
                platform::WM_APP_AI_EVENT,
                WPARAM(0),
                LPARAM(0),
            );
        });
        self.ai.follow_up_jobs.push(job);
    }

    pub(super) fn ai_click_follow_up(&mut self, msg_idx: usize, q_idx: usize) {
        if self.ai.panel.is_streaming() || self.ai.run.is_some() {
            return;
        }
        let Some(active) = self.ai.panel.active.as_ref() else {
            return;
        };
        let records = active
            .messages
            .iter()
            .filter(|m| mochi_core::ai::locator::visible(m))
            .collect::<Vec<_>>();
        let Some(record) = records.get(msg_idx) else {
            return;
        };
        let Some(state) = get_message_follow_up(record) else {
            return;
        };
        if state.status != FollowUpStatus::Ready {
            return;
        }
        let Some(sug) = state.suggestions.get(q_idx) else {
            return;
        };
        let question = sug.text.clone();

        let current_draft = self.ai.panel.input.text();
        let trimmed_draft = current_draft.trim();
        let trimmed_q = question.trim();

        if trimmed_draft.is_empty() {
            self.ai.panel.input.set_text(&question);
            self.focus = Focus::AiInput;
            self.invalidate_main();
        } else if trimmed_draft == trimmed_q {
            self.focus = Focus::AiInput;
            self.invalidate_main();
        } else {
            // 已有其他非空草稿：弹出确认弹窗
            self.dialog = Some(Dialog {
                title: "替换当前输入内容？".into(),
                description: String::new(),
                field: None,
                error: String::new(),
                note: None,
                buttons: vec![
                    DialogButton {
                        label: "取消".into(),
                        kind: ButtonKind::Ghost,
                        action: DialogAction::Dismiss,
                    },
                    DialogButton {
                        label: "替换".into(),
                        kind: ButtonKind::Primary,
                        action: DialogAction::ReplaceAiDraft {
                            session_id: active.id.clone(),
                            text: question,
                        },
                    },
                ],
                dismiss: DialogAction::Dismiss,
                hover: None,
            });
            self.focus = Focus::Dialog;
            self.invalidate_main();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_follow_up_rules_test() {
        let root = std::env::temp_dir().join(format!(
            "mochi-follow-up-test-{}",
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

            let mut conv = mochi_core::ai::AiConversation::default();
            conv.id = "conv-1".into();
            let mut msg = mochi_core::ai::AiStoredMessage::new("assistant", "这是回答");
            msg.set("id", serde_json::json!("msg-1"));
            let revision = compute_response_revision("msg-1", "这是回答");
            let state = FollowUpState::new_ready(
                &revision,
                vec![
                    "第一步操作".into(),
                    "第二步操作".into(),
                    "第三步操作".into(),
                ],
            );
            set_message_follow_up(&mut msg, state);
            conv.messages.push(msg);
            app.ai.panel.active = Some(conv);

            // 规则 1：输入框为空时，填入完整问题并聚焦
            app.ai.panel.input.set_text("");
            app.ai_click_follow_up(0, 0);
            assert_eq!(app.ai.panel.input.text(), "第一步操作");
            assert_eq!(app.focus, Focus::AiInput);
            assert!(app.dialog.is_none());

            // 规则 2：输入框已有相同问题，不重复写入，只聚焦
            app.ai_click_follow_up(0, 0);
            assert_eq!(app.ai.panel.input.text(), "第一步操作");
            assert_eq!(app.focus, Focus::AiInput);
            assert!(app.dialog.is_none());

            // 规则 3：输入框已有其他非空草稿，显示确认弹窗
            app.ai.panel.input.set_text("用户自己的草稿");
            app.ai_click_follow_up(0, 1);
            // 原草稿在确认前不得修改
            assert_eq!(app.ai.panel.input.text(), "用户自己的草稿");
            assert!(app.dialog.is_some());
            let replace_action = {
                let d = app.dialog.as_ref().unwrap();
                assert_eq!(d.title, "替换当前输入内容？");
                assert_eq!(d.buttons[0].label, "取消");
                assert_eq!(d.buttons[1].label, "替换");
                d.buttons[1].action.clone()
            };

            // 取消后原草稿保持不变
            app.close_dialog();
            assert_eq!(app.ai.panel.input.text(), "用户自己的草稿");

            // 确认替换后草稿更新
            if let DialogAction::ReplaceAiDraft { session_id, text } = &replace_action {
                assert_eq!(session_id, "conv-1");
                assert_eq!(text, "第二步操作");
                app.ai.panel.input.set_text(text);
                app.focus = Focus::AiInput;
            }
            assert_eq!(app.ai.panel.input.text(), "第二步操作");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
