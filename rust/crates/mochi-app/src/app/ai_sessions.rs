//! 定义 AI 会话列表支持的固定、重命名、项目归类和删除等操作。
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SessionAction {
    Pin,
    Rename,
    Projects,
    NewProject,
    Project(Option<String>),
    Colors,
    Color(Option<String>),
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::ai::agent_runner::AgentTraceStep;

    #[test]
    fn session_metadata_and_order_survive_reopen() {
        let root = std::env::temp_dir().join(format!(
            "mochi-session-actions-{}",
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
            let id = app.ai.panel.active.as_ref().unwrap().id.clone();
            app.ai_session_action(&id, SessionAction::Pin);
            app.ai_session_action(&id, SessionAction::Color(Some("#3C5A78".into())));
            app.ai_session_action(&id, SessionAction::Rename);
            app.dialog
                .as_mut()
                .unwrap()
                .field
                .as_mut()
                .unwrap()
                .set_text("重新命名的讨论");
            app.ai_submit_session_name(&id, false);
            app.ai_session_action(&id, SessionAction::NewProject);
            app.dialog
                .as_mut()
                .unwrap()
                .field
                .as_mut()
                .unwrap()
                .set_text("知识整理");
            app.ai_submit_session_name(&id, true);
            app.ai_persist_active();
            let svc = AiSessionService::new(&root);
            let index = svc.load_index();
            assert!(index.sessions[0].pinned);
            assert_eq!(index.sessions[0].color.as_deref(), Some("#3C5A78"));
            assert_eq!(
                index.sessions[0].project_id.as_deref(),
                Some(index.projects[0].id.as_str())
            );
            assert_eq!(svc.load_session(&id).unwrap().title, "重新命名的讨论");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_approval_status_updates_the_original_conversation() {
        let root = std::env::temp_dir().join(format!(
            "mochi-file-card-status-{}",
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
            let original_id = app.ai.panel.active.as_ref().unwrap().id.clone();
            for field in ["pendingEdit", "pendingFileOperation"] {
                let mut message = AiStoredMessage::new("assistant", "修改建议");
                message.set(
                    field,
                    serde_json::json!({"inboxId":"original-proposal", "status":"pending"}),
                );
                app.ai.panel.active.as_mut().unwrap().messages.push(message);
            }
            let mut batch = AiStoredMessage::new("assistant", "批量修改建议");
            batch.set(
                "pendingFileOperations",
                serde_json::json!([
                    {"inboxId":"original-proposal", "status":"pending"},
                    {"inboxId":"other-proposal", "status":"pending"}
                ]),
            );
            app.ai.panel.active.as_mut().unwrap().messages.push(batch);
            app.ai_persist_active();
            app.ai_new_session();
            let current = app.ai.panel.active.as_ref().unwrap().clone();
            app.ai_set_pending_edit_status("original-proposal", "rejected", None);
            assert_eq!(app.ai.panel.active.as_ref().unwrap(), &current);
            let saved = AiSessionService::new(&root)
                .load_session(&original_id)
                .unwrap();
            assert_eq!(
                saved.messages[0].get("pendingEdit").unwrap()["status"],
                "rejected"
            );
            assert_eq!(
                saved.messages[1].get("pendingFileOperation").unwrap()["status"],
                "rejected"
            );
            assert_eq!(
                saved.messages[2].get("pendingFileOperations").unwrap()[0]["status"],
                "rejected"
            );
            assert_eq!(
                saved.messages[2].get("pendingFileOperations").unwrap()[1]["status"],
                "pending"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_block_cards_persist_and_update_after_switch_and_restart() {
        let root = std::env::temp_dir().join(format!(
            "mochi-batch-block-cards-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let original_id;
        let entry_ids;
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.ai_new_session();
            original_id = app.ai.panel.active.as_ref().unwrap().id.clone();

            let inbox = AgentInboxService::new(&root);
            let mut cards = Vec::new();
            let mut block_edits = Vec::new();
            for (index, block_id) in [
                "block_00000000-0000-4000-8000-000000000001",
                "block_00000000-0000-4000-8000-000000000002",
            ]
            .into_iter()
            .enumerate()
            {
                let edit = serde_json::json!({
                    "path": "note.md",
                    "blockId": block_id,
                    "originalHash": format!("hash-{index}"),
                    "oldText": format!("旧块 {index}"),
                    "newText": format!("新块 {index}"),
                    "summary": format!("逐块修改 {index}")
                });
                let operation = serde_json::json!({
                    "id": format!("block-op-{index}"),
                    "kind": "block-edit",
                    "path": "note.md",
                    "title": format!("块修改 {index}"),
                    "summary": format!("逐块修改 {index}"),
                    "status": "pending",
                    "blockEdit": edit,
                    "blockId": block_id,
                    "originalHash": format!("hash-{index}"),
                    "oldText": format!("旧块 {index}"),
                    "newText": format!("新块 {index}")
                });
                let entry = inbox.add(operation, None, Some("AI 助手")).unwrap();
                let mut card = entry.to_value()["operation"].clone();
                card["inboxId"] = serde_json::json!(entry.id());
                cards.push(card);
                block_edits.push(cards.last().unwrap()["blockEdit"].clone());
            }
            entry_ids = inbox
                .list_pending()
                .into_iter()
                .map(|entry| entry.id().to_owned())
                .collect::<Vec<_>>();
            assert_eq!(entry_ids.len(), 2);

            let transcript = vec![AiMessage::tool_result(
                "batch-call",
                "document_block_propose_edit",
                &serde_json::json!({
                    "ok": true,
                    "data": {
                        "status": "pending",
                        "count": 2,
                        "pendingBlockEdits": block_edits,
                        "pendingFileOperations": cards
                    }
                })
                .to_string(),
            )];
            assert_eq!(app.ai_append_pending_cards(&transcript), 2);
            assert_eq!(
                app.ai
                    .panel
                    .active
                    .as_ref()
                    .unwrap()
                    .messages
                    .iter()
                    .filter(|message| message.get("pendingFileOperation").is_some())
                    .count(),
                2
            );
            app.ai_persist_active();

            // 模拟先打开另一个对话，再从全局收件箱处理项目。
            // 只处理当前选中的条目。
            app.ai_new_session();
            let current_id = app.ai.panel.active.as_ref().unwrap().id.clone();
            inbox.resolve(&entry_ids[0], "rejected", None).unwrap();
            app.ai_set_pending_edit_status(&entry_ids[0], "rejected", None);
            assert_eq!(
                app.ai.panel.active.as_ref().unwrap().id,
                current_id,
                "status changes must not switch the active conversation"
            );
            assert!(app.ai.panel.active.as_ref().unwrap().messages.is_empty());

            let saved = AiSessionService::new(&root)
                .load_session(&original_id)
                .unwrap();
            let saved_cards = saved
                .messages
                .iter()
                .filter_map(|message| message.get("pendingFileOperation"))
                .collect::<Vec<_>>();
            assert_eq!(saved_cards.len(), 2);
            assert_eq!(
                saved_cards
                    .iter()
                    .find(|card| card["inboxId"].as_str() == Some(entry_ids[0].as_str()))
                    .unwrap()["status"],
                "rejected"
            );
            assert_eq!(
                saved_cards
                    .iter()
                    .find(|card| card["inboxId"].as_str() == Some(entry_ids[1].as_str()))
                    .unwrap()["status"],
                "pending"
            );
            assert_eq!(inbox.list_pending().len(), 1);
        }

        // 新建的 App 和会话服务也必须读取到相同的卡片状态。
        {
            let mut restarted = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            restarted.shell.open_workspace(&root, || {}).unwrap();
            restarted.ai_open_session(&original_id);
            let cards = restarted
                .ai
                .panel
                .active
                .as_ref()
                .unwrap()
                .messages
                .iter()
                .filter_map(|message| message.get("pendingFileOperation"))
                .collect::<Vec<_>>();
            assert_eq!(cards.len(), 2);
            assert_eq!(
                cards
                    .iter()
                    .find(|card| card["inboxId"].as_str() == Some(entry_ids[0].as_str()))
                    .unwrap()["status"],
                "rejected"
            );
            assert_eq!(
                cards
                    .iter()
                    .find(|card| card["inboxId"].as_str() == Some(entry_ids[1].as_str()))
                    .unwrap()["status"],
                "pending"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_keeps_ordered_trace_and_only_final_round_content() {
        let root = std::env::temp_dir().join(format!(
            "mochi-live-trace-{}",
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
            let (tx, rx) = channel();
            app.ai.run = Some(RunHandle {
                rx,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            app.ai.panel.streaming = Some(assistant::Streaming::default());
            let thinking = AgentTraceStep::Thinking {
                text: "先读取已附加的文档".into(),
                duration_ms: Some(1250),
            };
            let tool = AgentTraceStep::Tool {
                name: "file_read".into(),
                summary: Some("读入 24 行".into()),
                ok: true,
                duration_ms: 35,
                result_preview: None,
            };
            tx.send(RunEvent::Runtime {
                agent_id: None,
                agent_name: "通用助手".into(),
                skills: vec![],
                model: "test-model".into(),
            })
            .unwrap();
            tx.send(RunEvent::ModelStart).unwrap();
            tx.send(RunEvent::Reasoning("先读取已附加的文档".into()))
                .unwrap();
            tx.send(RunEvent::Content("中间过程".into())).unwrap();
            tx.send(RunEvent::Trace(thinking.clone())).unwrap();
            tx.send(RunEvent::Tool {
                name: "file_read".into(),
                ok: None,
                summary: None,
            })
            .unwrap();
            tx.send(RunEvent::Trace(tool.clone())).unwrap();
            app.take_ai_events();
            let st = app.ai.panel.streaming.as_ref().unwrap();
            assert_eq!(st.trace, vec![thinking.clone(), tool.clone()]);
            assert!(st.reasoning.is_empty());
            assert!(st.content.is_empty());
            tx.send(RunEvent::ModelStart).unwrap();
            tx.send(RunEvent::Content("最终回答".into())).unwrap();
            tx.send(RunEvent::Done {
                content: "最终回答".into(),
                transcript: vec![],
                trace: vec![thinking, tool],
                finish_reason: "stop".into(),
                usage: serde_json::json!({"totalTokens": 450}),
                usage_source: "provider".into(),
            })
            .unwrap();
            app.take_ai_events();
            let saved = AiSessionService::new(&root)
                .load_session(&app.ai.panel.active.as_ref().unwrap().id)
                .unwrap();
            let answer = saved.messages.last().unwrap();
            assert_eq!(answer.content(), "最终回答");
            assert_eq!(answer.get("trace").unwrap().as_array().unwrap().len(), 2);
            assert_eq!(answer.get("model").unwrap(), "test-model");
            assert_eq!(answer.get("usage").unwrap()["totalTokens"], 450);
            app.ai.panel.streaming = Some(assistant::Streaming {
                reasoning: "尚未完成的思考".into(),
                ..Default::default()
            });
            app.ai_cancel();
            let partial = app
                .ai
                .panel
                .active
                .as_ref()
                .unwrap()
                .messages
                .last()
                .unwrap();
            assert_eq!(partial.get("interrupted").unwrap(), true);
            assert_eq!(partial.get("trace").unwrap()[0]["text"], "尚未完成的思考");
            let before = app.ai.panel.active.as_ref().unwrap().messages.clone();
            let (tx, rx) = channel();
            app.ai.run = Some(RunHandle {
                rx,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            app.ai.panel.streaming = Some(assistant::Streaming::default());
            tx.send(RunEvent::Done {
                content: String::new(),
                transcript: vec![],
                trace: vec![],
                finish_reason: "stop".into(),
                usage: serde_json::json!({"totalTokens": 12}),
                usage_source: "provider".into(),
            })
            .unwrap();
            app.take_ai_events();
            assert_eq!(
                app.ai.panel.active.as_ref().unwrap().messages,
                before,
                "an empty reply must not replace usage on the previous answer"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

impl App {
    pub(super) fn ai_session_context(&mut self, x: f32, y: f32) -> bool {
        if self.state.view != WorkspaceView::MochiAi || self.ai.panel.is_streaming() {
            return false;
        }
        let Some(id) = self
            .ai
            .workspace_side
            .rows
            .iter()
            .find(|(r, _)| self.ai.workspace_side.list.contains(x, y) && r.contains(x, y))
            .map(|(_, m)| m.id.clone())
        else {
            return false;
        };
        self.ai_session_menu(&id, x, y);
        true
    }

    fn ai_session_menu(&mut self, id: &str, x: f32, y: f32) {
        let Some(meta) = self.ai.panel.sessions.iter().find(|m| m.id == id) else {
            return;
        };
        let item =
            |label: &str, action| MenuItem::new(label, MenuAction::AiSession(id.into(), action));
        let mut items = vec![
            item(
                if meta.pinned {
                    "取消置顶"
                } else {
                    "置顶会话"
                },
                SessionAction::Pin,
            )
            .icon(Icon::PIN),
            item("重命名", SessionAction::Rename).icon(Icon::PENCIL),
            item("移动到项目…", SessionAction::Projects).icon(Icon::FOLDER),
            item("标记颜色…", SessionAction::Colors).icon(Icon::PALETTE),
        ];
        // 链接构造统一放在核心对象引用模块中，
        // 让会话链接与对象选择器共用校验和编码逻辑。
        if let Some(url) =
            mochi_core::object_reference::build_ai_session_reference(&meta.id, Some(&meta.title))
        {
            items.push(
                MenuItem::new("复制为 Mochi 链接", MenuAction::CopyObjectLink(url))
                    .icon(Icon::LINK2)
                    .separated(),
            );
        }
        items.push(
            item("删除会话", SessionAction::Delete)
                .icon(Icon::TRASH2)
                .danger()
                .separated(),
        );
        self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
    }

    pub(super) fn ai_session_action(&mut self, id: &str, action: SessionAction) {
        if self.ai.panel.is_streaming() {
            return;
        }
        let Some(svc) = self.ai_session_service() else {
            return;
        };
        let mut index = svc.load_index();
        let Some(position) = index.sessions.iter().position(|m| m.id == id) else {
            return;
        };
        let meta = &index.sessions[position];
        let item =
            |label: String, action| MenuItem::new(label, MenuAction::AiSession(id.into(), action));
        match action {
            SessionAction::Rename => {
                let mut field = TextField::new("会话名称");
                field.set_text(&meta.title);
                field.buffer.select_all();
                self.ai_session_dialog(
                    "重命名会话",
                    &meta.title,
                    Some(field),
                    "保存",
                    DialogAction::RenameAiSession(id.into()),
                );
                return;
            }
            SessionAction::Delete => {
                self.ai_ask_delete_session(id);
                return;
            }
            SessionAction::Projects => {
                let mut items = vec![item("新建项目并移入…".into(), SessionAction::NewProject)];
                items.extend(index.projects.iter().map(|project| {
                    item(
                        format!(
                            "{}{}",
                            if meta.project_id.as_deref() == Some(&project.id) {
                                "✓ "
                            } else {
                                ""
                            },
                            project.name
                        ),
                        SessionAction::Project(Some(project.id.clone())),
                    )
                }));
                if meta.project_id.is_some() {
                    items.push(
                        item("从项目中移出".into(), SessionAction::Project(None)).separated(),
                    );
                }
                self.open_menu_submenu(items);
                return;
            }
            SessionAction::NewProject => {
                self.ai_session_dialog(
                    "新建 AI 项目",
                    "创建项目并将此会话移入",
                    Some(TextField::new("项目名称")),
                    "创建",
                    DialogAction::NewAiSessionProject(id.into()),
                );
                return;
            }
            SessionAction::Colors => {
                let mut items = [
                    ("红色", "#C2554D"),
                    ("橙色", "#C77B3E"),
                    ("黄色", "#C9A227"),
                    ("绿色", "#4F7A5B"),
                    ("蓝色", "#3C5A78"),
                    ("紫色", "#6B5B95"),
                    ("灰色", "#8A8A8A"),
                ]
                .into_iter()
                .map(|(label, color)| {
                    item(
                        format!(
                            "{}{}",
                            if meta.color.as_deref() == Some(color) {
                                "✓ "
                            } else {
                                ""
                            },
                            label
                        ),
                        SessionAction::Color(Some(color.into())),
                    )
                })
                .collect::<Vec<_>>();
                items.push(item("清除颜色".into(), SessionAction::Color(None)).separated());
                self.open_menu_submenu(items);
                return;
            }
            SessionAction::Pin => index.sessions[position].pinned = !meta.pinned,
            SessionAction::Color(color) => index.sessions[position].color = color,
            SessionAction::Project(project) => {
                if project
                    .as_ref()
                    .is_some_and(|id| !index.projects.iter().any(|p| &p.id == id))
                {
                    return;
                }
                index.sessions[position].project_id = project;
            }
        }
        match svc.save_index(&index) {
            Ok(()) => {
                self.ai.panel.sessions = index.sessions;
                self.ai.workspace.projects = index.projects;
            }
            Err(error) => self.ai.panel.error = format!("更新会话失败：{error}"),
        }
    }

    fn ai_session_dialog(
        &mut self,
        title: &str,
        description: &str,
        field: Option<TextField>,
        label: &str,
        action: DialogAction,
    ) {
        self.dialog = Some(Dialog {
            title: title.into(),
            description: description.into(),
            field,
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: label.into(),
                    kind: if label == "删除" || label == "清空" {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    },
                    action,
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn ai_ask_delete_session(&mut self, id: &str) {
        if self.ai.panel.is_streaming() {
            return;
        }
        let title = self
            .ai
            .panel
            .sessions
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.title.clone())
            .unwrap_or_default();
        self.ai_session_dialog(
            "删除会话",
            &format!("确定删除“{title}”及其消息挂载吗？"),
            None,
            "删除",
            DialogAction::DeleteAiSession(id.into()),
        );
    }

    pub(super) fn ai_ask_clear_session(&mut self) {
        if self.ai.panel.is_streaming() {
            return;
        }
        if let Some(id) = self
            .ai
            .panel
            .active
            .as_ref()
            .filter(|c| !c.messages.is_empty())
            .map(|c| c.id.clone())
        {
            self.ai_session_dialog(
                "清空会话",
                "将移除当前会话的全部消息。",
                None,
                "清空",
                DialogAction::ClearAiSession(id),
            );
        }
    }

    pub(super) fn ai_submit_session_name(&mut self, id: &str, project: bool) {
        if self.ai.panel.is_streaming() {
            return;
        }
        let name = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().trim().to_owned())
            .unwrap_or_default();
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(!name.is_empty(), "名称不能为空");
            let svc = self
                .ai_session_service()
                .ok_or_else(|| anyhow::anyhow!("工作区已关闭"))?;
            let mut index = svc.load_index();
            let meta = index
                .sessions
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or_else(|| anyhow::anyhow!("会话已不存在"))?;
            if project {
                let now = Self::now_ms();
                let project = ai_session::AiProject {
                    id: ai_session::new_project_id(),
                    name: name.clone(),
                    created_at: now,
                    updated_at: now,
                    order: now,
                    ..Default::default()
                };
                meta.project_id = Some(project.id.clone());
                index.projects.push(project);
            } else {
                let mut conversation = svc
                    .load_session(id)
                    .ok_or_else(|| anyhow::anyhow!("会话文件无法读取"))?;
                conversation.title = name.clone();
                svc.save_session(&conversation)?;
                meta.title = name.clone();
            }
            svc.save_index(&index)?;
            if !project {
                self.cancel_title_job(id);
                if let Some(c) = self.ai.panel.active.as_mut().filter(|c| c.id == id) {
                    c.title = name.clone();
                }
            }
            self.ai.panel.sessions = index.sessions;
            self.ai.workspace.projects = index.projects;
            Ok(())
        })();
        match result {
            Ok(()) => self.close_dialog(),
            Err(error) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = error.to_string();
                }
            }
        }
    }
}
