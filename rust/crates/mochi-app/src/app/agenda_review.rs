//! 审批墨池 AI 生成的日程待办提案：预览、批准（严格模式应用，可撤销）或拒绝。
use super::*;
use mochi_core::agenda::{proposal::MESSAGE_KEY, Proposal, Source};

pub(super) fn preview_markdown(raw: &serde_json::Value) -> String {
    match Proposal::from_value(raw) {
        Ok(proposal) => proposal.preview_markdown(),
        Err(_) => "日程待办变更".into(),
    }
}

impl App {
    pub(super) fn open_schedule_review(&mut self, raw: serde_json::Value) -> bool {
        if raw["status"].as_str() != Some("pending") {
            return false;
        }
        let Ok(proposal) = Proposal::from_value(&raw) else {
            self.ai.panel.error = "日程待办提案格式无效，请重新生成".into();
            return false;
        };
        let Some(root) = self.shell.workspace().map(|w| w.root.clone()) else {
            return false;
        };
        if raw["workspaceRoot"]
            .as_str()
            .is_some_and(|value| Path::new(value) != root)
        {
            self.ai.panel.error = "请回到生成此计划的工作区再处理".into();
            return false;
        }
        let mut raw = raw;
        raw["workspaceRoot"] = serde_json::json!(root);
        let mut preview = proposal
            .operations
            .iter()
            .take(5)
            .map(|op| crate::ui::text::ellipsize(&op.summary, TextStyle::Label, 255.0))
            .collect::<Vec<_>>()
            .join("\n");
        if proposal.operations.len() > 5 {
            preview.push_str(&format!(
                "\n共 {} 项，完整预览在会话正文中。",
                proposal.operations.len()
            ));
        }
        if proposal.has_delete() {
            preview.push_str("\n包含删除：删除的记录会进回收站，可以恢复。");
        }
        self.dialog = Some(Dialog {
            title: format!("确认计划 · {} 项变更", proposal.operations.len()),
            description: proposal.summary.clone(),
            field: None,
            error: String::new(),
            note: Some(preview.into()),
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "拒绝".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::ReviewScheduleDiff {
                        raw: raw.clone(),
                        approve: false,
                    },
                },
                DialogButton {
                    label: "应用计划".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::ReviewScheduleDiff { raw, approve: true },
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
        true
    }

    pub(super) fn apply_schedule_review(&mut self, raw: serde_json::Value, approve: bool) {
        let result = (|| -> anyhow::Result<Proposal> {
            let mut proposal = Proposal::from_value(&raw)?;
            let root = self
                .shell
                .workspace()
                .map(|w| w.root.clone())
                .ok_or_else(|| anyhow::anyhow!("工作区已关闭"))?;
            anyhow::ensure!(
                raw["workspaceRoot"]
                    .as_str()
                    .is_some_and(|value| Path::new(value) == root),
                "工作区已切换，请回到原工作区处理"
            );
            let current = self
                .ai
                .panel
                .active
                .as_ref()
                .and_then(|conversation| {
                    conversation.messages.iter().find_map(|message| {
                        let value = message.get(MESSAGE_KEY)?;
                        (value["id"].as_str() == Some(proposal.id.as_str())).then(|| value.clone())
                    })
                })
                .ok_or_else(|| anyhow::anyhow!("原提案消息已不存在"))?;
            anyhow::ensure!(current["status"] == "pending", "此计划已处理，请勿重复应用");
            anyhow::ensure!(
                current["operations"] == raw["operations"],
                "提案已变化，请重新查看"
            );
            if approve {
                let store = self
                    .agenda_store()
                    .ok_or_else(|| anyhow::anyhow!("工作区已关闭"))?;
                let change = proposal.to_change();
                // 严格模式：提案生成后用户又改过的记录会拒绝应用，绝不覆盖。
                store
                    .apply(Source::Ai, &change, true)
                    .map_err(|e| anyhow::anyhow!("{e}（相关记录在提案生成后被改过）"))?;
                self.agenda_push_undo(change);
            }
            proposal.status = if approve { "applied" } else { "rejected" }.into();
            Ok(proposal)
        })();
        match result {
            Ok(proposal) => {
                if let Some(conversation) = self.ai.panel.active.as_mut() {
                    for message in &mut conversation.messages {
                        if let Some(value) = message
                            .get(MESSAGE_KEY)
                            .filter(|value| value["id"].as_str() == Some(proposal.id.as_str()))
                        {
                            let mut updated = value.clone();
                            updated["status"] = serde_json::json!(proposal.status);
                            let summary = format!(
                                "{} 项日程待办变更已{}",
                                proposal.operations.len(),
                                if approve { "应用" } else { "拒绝" }
                            );
                            updated["summary"] = serde_json::json!(summary);
                            message.set_content(&format!(
                                "{summary}\n\n{}",
                                preview_markdown(&updated)
                            ));
                            message.set(MESSAGE_KEY, updated);
                        }
                    }
                }
                self.ai_persist_active();
                self.close_dialog();
                self.reload_schedule();
                self.publish_notification(
                    crate::ui::notifications::Category::Schedule,
                    if approve {
                        "计划已应用"
                    } else {
                        "计划已拒绝"
                    },
                    &proposal.summary,
                );
            }
            Err(error) => {
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.error = error.to_string();
                    dialog.note =
                        Some(format!("无法应用计划：{error}\n请重新生成或检查原条目。").into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::agenda::{AgendaStore, ChangeSet, Clock, Editor, TaskDraft};

    fn propose(store: &AgendaStore, title: &str) -> serde_json::Value {
        let before = store.load().unwrap();
        let mut after = before.clone();
        let mut ed = Editor::new(&mut after, Clock::system());
        ed.create_task(TaskDraft {
            title: title.into(),
            ..Default::default()
        })
        .unwrap();
        let change = ChangeSet::diff(&before, &after);
        Proposal::from_change(&change, &before, &after, title).to_value()
    }

    #[test]
    fn agenda_approval_applies_once_is_undoable_and_persists_its_resolution() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-agenda-review-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.ai_new_session();
            let store = app.agenda_store().unwrap();
            let pending = propose(&store, "完成个人学习计划");
            let transcript = vec![AiMessage::tool_result(
                "plan",
                "agenda_plan_day",
                &serde_json::json!({"ok": true, "data": {MESSAGE_KEY: pending}}).to_string(),
            )];
            assert_eq!(app.ai_append_pending_cards(&transcript), 1);
            assert_eq!(app.ai_append_pending_cards(&transcript), 0);
            let raw = app
                .ai
                .panel
                .active
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .find_map(|message| message.get(MESSAGE_KEY).cloned())
                .unwrap();
            assert!(store.load().unwrap().tasks.is_empty());
            assert!(app.open_schedule_review(raw.clone()));
            app.apply_schedule_review(raw.clone(), true);
            assert!(app.dialog.is_none());
            assert_eq!(store.load().unwrap().tasks.len(), 1);
            assert_eq!(
                app.sched.view.data.as_ref().unwrap().tasks[0].title,
                "完成个人学习计划"
            );
            app.apply_schedule_review(raw, true);
            assert_eq!(store.load().unwrap().tasks.len(), 1);
            assert!(app.schedule_undo());
            assert!(store.load().unwrap().tasks.is_empty());
            let conversation = app.ai.panel.active.as_ref().unwrap();
            let saved = AiSessionService::new(&root)
                .load_session(&conversation.id)
                .unwrap();
            assert!(saved.messages.iter().any(|message| message
                .get(MESSAGE_KEY)
                .is_some_and(|value| value["status"] == "applied")));

            let rejected = propose(&store, "暂不执行的计划");
            let transcript = vec![AiMessage::tool_result(
                "second-plan",
                "agenda_batch",
                &serde_json::json!({"ok": true, "data": {MESSAGE_KEY: rejected}}).to_string(),
            )];
            assert_eq!(app.ai_append_pending_cards(&transcript), 1);
            let raw = app
                .ai
                .panel
                .active
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .rev()
                .find_map(|message| message.get(MESSAGE_KEY).cloned())
                .unwrap();
            assert!(app.open_schedule_review(raw.clone()));
            app.apply_schedule_review(raw, false);
            assert!(store.load().unwrap().tasks.is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
