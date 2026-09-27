//! 处理 AI 消息状态、复制操作，以及待处理编辑和命令卡片的交互。
use super::*;
use assistant::MessageAction;
use mochi_core::ai::{locator::Locator, message_ops};

fn update_pending_status(
    value: &mut serde_json::Value,
    inbox_id: &str,
    status: &str,
    error: Option<&str>,
) -> bool {
    match value {
        serde_json::Value::Array(values) => {
            let mut changed = false;
            for value in values {
                changed |= update_pending_status(value, inbox_id, status, error);
            }
            changed
        }
        serde_json::Value::Object(object) => {
            if object.get("inboxId").and_then(|value| value.as_str()) != Some(inbox_id) {
                return false;
            }
            object.insert("status".into(), serde_json::Value::String(status.into()));
            match error {
                Some(value) => {
                    object.insert("error".into(), serde_json::Value::String(value.into()));
                }
                None => {
                    object.remove("error");
                }
            }
            true
        }
        _ => false,
    }
}

impl App {
    pub(super) fn ai_content_copy_with(
        &mut self,
        message: usize,
        action: usize,
        write: impl FnOnce(&crate::ui::ai_markdown::CopyPayload) -> bool,
    ) -> bool {
        let Some(payload) = self.ai.layout.content_copy(&self.ai.panel, message, action) else {
            self.ai_copy_notice("复制失败：回复已变化，请重试");
            return false;
        };
        let ok = write(&payload);
        self.ai_copy_notice(if ok {
            match payload.kind {
                crate::ui::ai_markdown::CopyKind::Formula => "已复制公式",
                crate::ui::ai_markdown::CopyKind::Table => "已复制表格",
                crate::ui::ai_markdown::CopyKind::Code => "已复制代码",
            }
        } else {
            "复制失败，请重试"
        });
        ok
    }
    fn ai_copy_notice(&mut self, text: &str) {
        self.show_global_notice(text);
    }
    pub(super) fn ai_set_pending_edit_status(
        &mut self,
        inbox_id: &str,
        status: &str,
        error: Option<&str>,
    ) {
        let update = |conversation: &mut AiConversation| {
            let mut changed = false;
            for message in &mut conversation.messages {
                // 界面上可见的审批卡片使用单数形式的字段。与此同时，也要保持
                // 已保存或旧版消息中的数组形式一致，
                // 这样批量响应才不会在对应的收件箱条目已经处理后，
                // 仍残留一条待处理记录，哪怕该记录来自另一个
                // 会话，或应用重启前已经处理。
                for field in [
                    "pendingEdit",
                    "pendingFileOperation",
                    "pendingBlockEdit",
                    "pendingFileOperations",
                    "pendingBlockEdits",
                ] {
                    let Some(mut edit) = message.get(field).cloned() else {
                        continue;
                    };
                    if update_pending_status(&mut edit, inbox_id, status, error) {
                        message.set(field, edit);
                        changed = true;
                    }
                }
            }
            changed
        };
        let active_id = self
            .ai
            .panel
            .active
            .as_ref()
            .map(|conversation| conversation.id.clone());
        if self.ai.panel.active.as_mut().is_some_and(update) {
            self.ai_persist_active();
        }
        // 切换对话后，也可以从全局收件箱审核文件提案。
        // 旧版收件箱记录没有 session id，因此要在已保存的对话中按
        // 稳定的收件箱 ID 匹配，并保留其他无关文件。
        let Some(service) = self.ai_session_service() else {
            return;
        };
        for meta in service.load_index().sessions {
            if active_id.as_deref() == Some(meta.id.as_str()) {
                continue;
            }
            let Some(mut conversation) = service.load_session(&meta.id) else {
                continue;
            };
            if update(&mut conversation) {
                if let Err(error) = service.save_session(&conversation) {
                    self.ai.panel.error = format!("保存审批状态失败：{error}");
                }
            }
        }
    }
    /// 将命令审批卡写回创建它的会话。
    ///
    /// 审批弹层可以从待批准列表打开，而当前 AI 面板此时可能已经切到另一会话。
    /// `session_id` 来自队列请求；旧请求没有该字段时保留原先更新活动会话的行为。
    pub(super) fn ai_reject_shell_card(&mut self, request_id: &str, session_id: Option<&str>) {
        let active_id = self
            .ai
            .panel
            .active
            .as_ref()
            .map(|conversation| conversation.id.clone());
        let target_id = session_id
            .map(str::to_owned)
            .or_else(|| active_id.clone())
            .unwrap_or_else(|| format!("ai-conversation-{request_id}"));
        let active = active_id.as_deref() == Some(target_id.as_str());

        if active {
            let changed = self.ai.panel.active.as_mut().is_some_and(|conversation| {
                let mut changed = false;
                for message in &mut conversation.messages {
                    let Some(mut pending) = message.get("pendingShellCommand").cloned() else {
                        continue;
                    };
                    if pending["id"].as_str() == Some(request_id)
                        || pending["requestId"].as_str() == Some(request_id)
                    {
                        pending["status"] = serde_json::json!("rejected");
                        message.set("pendingShellCommand", pending);
                        changed = true;
                    }
                }
                if changed {
                    conversation.updated_at = mochi_core::jstime::now_millis();
                }
                changed
            });
            if changed {
                self.ai_persist_active();
            }
            return;
        }

        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            return;
        };
        let service = AiSessionService::new(&root);
        let Some(mut conversation) = service.load_session(&target_id) else {
            return;
        };
        let mut changed = false;
        for message in &mut conversation.messages {
            let Some(mut pending) = message.get("pendingShellCommand").cloned() else {
                continue;
            };
            if pending["id"].as_str() == Some(request_id)
                || pending["requestId"].as_str() == Some(request_id)
            {
                pending["status"] = serde_json::json!("rejected");
                message.set("pendingShellCommand", pending);
                changed = true;
            }
        }
        if !changed {
            return;
        }

        conversation.updated_at = mochi_core::jstime::now_millis();
        if let Err(error) = service.save_session(&conversation) {
            self.ai.panel.error = format!("保存会话失败：{error}");
            return;
        }
        let mut index = service.load_index();
        let count = conversation
            .messages
            .iter()
            .filter(|message| !message.is_hidden())
            .count()
            .min(i32::MAX as usize) as i32;
        if let Some(meta) = index.sessions.iter_mut().find(|meta| meta.id == target_id) {
            meta.updated_at = conversation.updated_at;
            meta.message_count = count;
        } else {
            index.sessions.insert(
                0,
                AiSessionMeta {
                    id: target_id,
                    title: conversation.title.clone(),
                    created_at: conversation.created_at,
                    updated_at: conversation.updated_at,
                    message_count: count,
                    ..Default::default()
                },
            );
        }
        if let Err(error) = service.save_index(&index) {
            self.ai.panel.error = format!("保存会话索引失败：{error}");
            return;
        }
        self.ai.panel.sessions = index.sessions;
    }
    pub(super) fn ai_pending_edit_action(&mut self, index: usize, approve: bool) {
        let Some(edit) = self
            .ai
            .layout
            .messages
            .get(index)
            .and_then(|message| message.pending_edit.clone())
        else {
            return;
        };
        let inbox_id = edit["inboxId"].as_str().map(str::to_owned);
        let operation_id = edit["operationId"].as_str().map(str::to_owned);
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            self.ai.panel.error = "没有工作区，无法处理修改提案".into();
            return;
        };
        let service = mochi_core::ai::agent_inbox::AgentInboxService::new(&root);
        let entry = service.list_pending().into_iter().find(|entry| {
            inbox_id.as_deref() == Some(entry.id())
                || operation_id.as_deref() == entry.to_value()["operation"]["id"].as_str()
        });
        let Some(entry) = entry else {
            self.ai.panel.error = "修改提案已处理或收件箱记录不存在，请刷新待批准操作".into();
            return;
        };
        if approve {
            // 继续走既有文件预览、工作区路径校验、磁盘/缓冲区冲突校验。
            self.review_file_proposal(entry);
            if let Some(error) = self
                .commands
                .review
                .as_ref()
                .map(|review| review.error.clone())
                .filter(|error| !error.is_empty())
            {
                let status = if error.contains("变化") {
                    "conflict"
                } else {
                    "error"
                };
                self.ai_set_pending_edit_status(
                    inbox_id.as_deref().unwrap_or_default(),
                    status,
                    Some(&error),
                );
            }
        } else {
            match self.reject_inbox_approval_entries(&[entry]) {
                Ok(1) => {
                    self.refresh_right_panel();
                    self.state.status_text = "已拒绝文件修改提案".into();
                }
                Ok(_) => {
                    self.ai.panel.error = "修改提案已被其他审批流程处理，请刷新".into();
                }
                Err(error) => self.ai.panel.error = error.to_string(),
            }
        }
    }
    pub(super) fn ai_auto_apply_pending_edit(&mut self, inbox_id: &str) {
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            self.ai_set_pending_edit_status(
                inbox_id,
                "error",
                Some("没有工作区，无法自动应用修改"),
            );
            return;
        };
        let service = mochi_core::ai::agent_inbox::AgentInboxService::new(&root);
        let Some(entry) = service
            .list_pending()
            .into_iter()
            .find(|entry| entry.id() == inbox_id)
        else {
            self.ai_set_pending_edit_status(inbox_id, "error", Some("收件箱记录不存在"));
            return;
        };
        self.review_file_proposal(entry);
        let error = self
            .commands
            .review
            .as_ref()
            .map(|review| review.error.clone())
            .unwrap_or_else(|| "无法创建文件预览".into());
        if error.is_empty() {
            self.apply_reviewed_file();
        } else {
            match service.resolve_many_if_status(
                &[inbox_id.to_owned()],
                "pending",
                "error",
                Some(&error),
            ) {
                Ok(1) => self.ai_set_pending_edit_status(inbox_id, "error", Some(&error)),
                Ok(_) => self.ai_set_pending_edit_status(
                    inbox_id,
                    "error",
                    Some("审批已被其他流程处理，未覆盖其状态"),
                ),
                Err(resolve_error) => self.ai_set_pending_edit_status(
                    inbox_id,
                    "error",
                    Some(&format!("{error}；记录审批错误失败：{resolve_error}")),
                ),
            }
            self.commands.review = None;
            self.refresh_right_panel();
        }
    }
    pub(super) fn ai_message(
        &self,
        sid: &str,
        mid: &str,
    ) -> anyhow::Result<(&AiConversation, usize)> {
        let c = self
            .ai
            .panel
            .active
            .as_ref()
            .filter(|c| c.id == sid)
            .ok_or_else(|| anyhow::anyhow!("会话已变化，请重试"))?;
        let index = c
            .messages
            .iter()
            .position(|m| m.id() == Some(mid) && message_ops::actionable(m))
            .ok_or_else(|| anyhow::anyhow!("回复已不存在或暂不可操作"))?;
        Ok((c, index))
    }
    pub(super) fn ai_message_text(
        &self,
        sid: &str,
        mid: &str,
        action: MessageAction,
    ) -> anyhow::Result<String> {
        let (c, index) = self.ai_message(sid, mid)?;
        match action {
            MessageAction::Copy => Ok(message_ops::copy_markdown(c.messages[index].content())),
            MessageAction::Insert => {
                let question = c.messages[..index]
                    .iter()
                    .rev()
                    .find(|m| m.role() == "user" && mochi_core::ai::locator::visible(m))
                    .map(|m| m.content())
                    .unwrap_or(c.messages[index].content());
                Ok(Locator {
                    session_id: sid.into(),
                    message_id: mid.into(),
                    title: c.title.clone(),
                    snippet: message_ops::snippet(question),
                }
                .to_url())
            }
            _ => anyhow::bail!("不是复制操作"),
        }
    }
    pub(super) fn ai_message_action(
        &mut self,
        sid: &str,
        mid: &str,
        action: MessageAction,
        x: f32,
        y: f32,
    ) {
        if let Err(e) = self.ai_message(sid, mid) {
            self.ai.panel.error = e.to_string();
            return;
        }
        match action {
            MessageAction::Copy | MessageAction::Insert => {
                match self.ai_message_text(sid, mid, action) {
                    Ok(value) => {
                        let copied = platform::copy_to_clipboard(&value);
                        self.show_global_notice(if copied {
                            if action == MessageAction::Copy {
                                "已复制回复"
                            } else {
                                "已复制定位标识，粘贴到笔记即可生成卡片"
                            }
                        } else {
                            "复制失败，请重试"
                        });
                    }
                    Err(e) => self.ai.panel.error = e.to_string(),
                }
            }
            MessageAction::Mount => {
                let mut items = self
                    .shell
                    .tabs()
                    .iter()
                    .filter_map(|t| {
                        t.path().map(|path| {
                            MenuItem::new(
                                &t.title,
                                MenuAction::MountAiMessage(sid.into(), mid.into(), path.into()),
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                items.push(MenuItem::new(
                    "选择其它文档（可多选）…",
                    MenuAction::PickAiMessageDocuments(sid.into(), mid.into()),
                ));
                if self.menu_submenu_target.is_some() {
                    self.open_menu_submenu(items);
                } else {
                    self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
                }
            }
            MessageAction::Delete => {
                if self.ai.panel.is_streaming() || self.ai.run.is_some() {
                    return;
                }
                let prepared = (|| -> anyhow::Result<_> {
                    let root = &self
                        .shell
                        .workspace()
                        .ok_or_else(|| anyhow::anyhow!("没有工作区"))?
                        .root;
                    let (c, _) = self.ai_message(sid, mid)?;
                    message_ops::Deletion::prepare(root, c, mid)
                })();
                match prepared {
                    Ok(plan) => {
                        self.pending_ai_delete = Some(plan);
                        self.dialog = Some(Dialog {
                            title: "删除这轮问答".into(),
                            description: "将删除提问、回复及中间工具记录。".into(),
                            field: None,
                            error: String::new(),
                            note: Some(
                                "将从会话和后续上下文中移除，并清理消息挂载。此操作不可撤销。"
                                    .into(),
                            ),
                            buttons: vec![
                                DialogButton {
                                    label: "取消".into(),
                                    kind: ButtonKind::Ghost,
                                    action: DialogAction::Dismiss,
                                },
                                DialogButton {
                                    label: "删除".into(),
                                    kind: ButtonKind::Danger,
                                    action: DialogAction::DeleteAiRound,
                                },
                            ],
                            dismiss: DialogAction::Dismiss,
                            hover: None,
                        });
                        self.focus = Focus::Dialog;
                    }
                    Err(e) => self.ai.panel.error = e.to_string(),
                }
            }
        }
    }
    pub(super) fn ai_confirm_delete_round(&mut self) {
        let result = (|| -> anyhow::Result<_> {
            anyhow::ensure!(
                !self.ai.panel.is_streaming() && self.ai.run.is_none(),
                "回复正在生成，请稍后重试"
            );
            let plan = self
                .pending_ai_delete
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("删除请求已过期"))?;
            let root = &self
                .shell
                .workspace()
                .ok_or_else(|| anyhow::anyhow!("工作区已关闭"))?
                .root;
            let active = self
                .ai
                .panel
                .active
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("会话已关闭"))?;
            plan.commit(root, active)?;
            Ok(plan.next.clone())
        })();
        match result {
            Ok(next) => {
                self.ai.panel.active = Some(next);
                self.ai_reset_message_view();
                self.ai.panel.located = None;
                self.ai.panel.locator_pending = None;
                self.ai.panel.hover_message = None;
                if let Some(svc) = self.ai_session_service() {
                    self.ai.panel.sessions = svc.load_index().sessions;
                }
                self.close_dialog();
                self.refresh_right_panel();
                self.state.status_text = "已删除这轮问答及相关消息挂载".into();
            }
            Err(e) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.note = Some(format!("删除失败：{e}").into());
                }
                self.ai.panel.error = e.to_string();
            }
        }
    }
    pub(super) fn ai_mount_message(&mut self, sid: &str, mid: &str, paths: &[PathBuf]) {
        let result = (|| -> anyhow::Result<()> {
            let root = self
                .shell
                .workspace()
                .ok_or_else(|| anyhow::anyhow!("没有工作区"))?
                .root
                .canonicalize()?;
            let (c, index) = self.ai_message(sid, mid)?;
            let question = c.messages[..index]
                .iter()
                .rev()
                .find(|m| m.role() == "user" && mochi_core::ai::locator::visible(m));
            let mut items = Vec::new();
            for path in paths {
                let target = path.canonicalize()?;
                anyhow::ensure!(
                    mochi_core::paths::path_is_within(&root, &target) && target.is_file(),
                    "请选择当前工作区内的文档"
                );
                items.push(mochi_core::ai::document_mounts::CreateMountInput {
                    session_id: sid.into(),
                    scope: mochi_core::ai::document_mounts::MountScope::Message,
                    message_id: Some(mid.into()),
                    user_message_id: question.and_then(|m| m.id().map(str::to_owned)),
                    document_path: target.to_string_lossy().replace('\\', "/"),
                    session_title: c.title.clone(),
                    snippet: message_ops::snippet(
                        question
                            .map(|m| m.content())
                            .unwrap_or(c.messages[index].content()),
                    ),
                    created_at: None,
                });
            }
            if !items.is_empty() {
                AiDocumentMountService::new(root).add_mounts(items)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.refresh_right_panel();
                self.state.status_text = "已挂载问答，原文档内容未修改".into();
            }
            Err(e) => self.ai.panel.error = e.to_string(),
        }
    }
    pub(super) fn ai_message_context(&mut self, x: f32, y: f32) -> bool {
        if !((self.state.ai_panel_open && self.state.right_panel == RightPanel::Assistant)
            || self.state.view == WorkspaceView::MochiAi
            || self.ai.float.is_some())
        {
            return false;
        }
        let Some(index) = self.ai.layout.message_at(self.ai.panel.scroll, x, y) else {
            return false;
        };
        let Some((sid, mid)) = self
            .ai
            .layout
            .messages
            .get(index)
            .and_then(|m| m.action_target.clone())
        else {
            return false;
        };
        let items = [
            ("插入", MessageAction::Insert),
            ("复制", MessageAction::Copy),
            ("删除", MessageAction::Delete),
            ("挂载", MessageAction::Mount),
        ]
        .into_iter()
        .map(|(label, action)| {
            MenuItem::new(
                label,
                MenuAction::AiMessage(sid.clone(), mid.clone(), action),
            )
            .disabled(action == MessageAction::Delete && self.ai.panel.is_streaming())
        })
        .collect();
        self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
        true
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    #[test]
    fn failed_copy_never_deletes_cut_text_from_field_or_document() {
        let root = std::env::temp_dir().join(format!(
            "mochi-cut-copy-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.focus = Focus::AiInput;
            app.ai.panel.input.set_text("不能丢失中文😀");
            app.ai.panel.input.buffer.select_all();
            assert!(app.clipboard_copy_with(true, |_| false));
            assert_eq!(app.ai.panel.input.text(), "不能丢失中文😀");
            assert!(app.clipboard_copy_with(true, |_| true));
            assert_eq!(app.ai.panel.input.text(), "");
            app.shell.open_workspace(&root, || {}).unwrap();
            let file = root.join("知识库").join("剪切.md");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "# 原文\r\n\r\n保留内容").unwrap();
            app.shell.open_file(&file);
            app.sync_state();
            app.state.view = WorkspaceView::Editor;
            app.focus = Focus::Main;
            app.editor_engaged = true;
            app.shell.active_buffer_mut().unwrap().select_all();
            assert!(app.clipboard_copy_with(true, |_| false));
            assert_eq!(
                app.shell.active_buffer_mut().unwrap().text(),
                "# 原文\r\n\r\n保留内容"
            );
            assert!(app.shell.active_buffer_mut().unwrap().has_selection());
            assert_eq!(
                std::fs::read_to_string(&file).unwrap(),
                "# 原文\r\n\r\n保留内容"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn copy_controller_delivers_payload_and_reports_sink_failure_without_os_clipboard() {
        let root = std::env::temp_dir().join(format!(
            "mochi-copy-controller-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.ai.panel.active = Some(AiConversation {
                id: "s".into(),
                messages: vec![mochi_core::ai::session::AiStoredMessage::new(
                    "assistant",
                    "$$x^2$$",
                )],
                ..Default::default()
            });
            app.ai.layout =
                assistant::layout(&app.ai.panel, Rect::from_size(0.0, 0.0, 420.0, 600.0));
            assert!(app.ai_content_copy_with(0, 0, |p| {
                assert_eq!(p.text, "x^2");
                assert!(p.html.is_none());
                true
            }));
            assert_eq!(app.status_bar.toast.message, "已复制公式");
            assert!(!app.ai_content_copy_with(0, 0, |_| false));
            assert!(app.status_bar.toast.message.contains("失败"));
            app.ai.panel.active.as_mut().unwrap().id = "changed".into();
            assert!(!app.ai_content_copy_with(0, 0, |_| panic!(
                "stale content must never reach clipboard sink"
            )));
            assert!(app.status_bar.toast.message.contains("回复已变化"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
