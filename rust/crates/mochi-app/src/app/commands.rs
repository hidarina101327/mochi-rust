//! 原生命令审批：完整只读预览、显式执行、可取消、结果写回原会话。
use super::*;
use crate::{
    export_requests::{Queue, Request},
    ui::command_review::{self, Hit, ReviewKind},
};
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Default)]
pub(super) struct State {
    pub review: Option<command_review::State>,
    pub pressed: Option<Hit>,
    pub jobs: HashMap<String, Arc<AtomicBool>>,
    pub close_after: bool,
    pub result_error: Option<String>,
    unsaved: Vec<(PathBuf, Request, Arc<Queue>)>,
}
impl App {
    pub(super) fn review_command(&mut self, id: &str) {
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        if let Some(request) = self.ai.export_requests.as_ref().and_then(|q| {
            q.pending()
                .into_iter()
                .find(|r| r.id == id && r.format == "shell")
        }) {
            self.commands.review = Some(command_review::State::new(request));
            self.commands.pressed = None;
            self.drag = None;
        }
    }

    pub(super) fn review_document_export(&mut self, id: &str) {
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        let request = self.ai.export_requests.as_ref().and_then(|queue| {
            queue
                .pending()
                .into_iter()
                .find(|request| request.id == id && request.format != "shell")
        });
        let Some(request) = request else {
            self.state.status_text = "导出请求已处理或不存在".into();
            return;
        };
        self.commands.review = Some(command_review::State::document_export(request));
        self.commands.pressed = None;
        self.drag = None;
    }
    pub(super) fn command_review_click(&mut self, x: f32, y: f32) {
        let l = command_review::layout(self.renderer.viewport());
        self.commands.pressed = l
            .entries
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h);
    }
    pub fn release_command_review(&mut self, x: f32, y: f32) -> bool {
        if self.commands.review.is_none() {
            return false;
        }
        let l = command_review::layout(self.renderer.viewport());
        let hit = l
            .entries
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h);
        if let Some(hit) = hit {
            if self.commands.pressed.take() == Some(hit) {
                self.command_review_action(hit);
            }
        }
        true
    }
    pub(super) fn command_review_key(&mut self, key: u16, ctrl: bool) -> bool {
        let Some(review) = self.commands.review.as_mut() else {
            return false;
        };
        let l = command_review::layout(self.renderer.viewport());
        match key {
            27 => self.command_review_action(Hit::Cancel),
            13 if ctrl => {
                if l.entries.iter().any(|(_, h)| *h == Hit::Execute) {
                    self.command_review_action(Hit::Execute)
                }
            }
            0x43 if ctrl => {
                let copy_text = match review.kind() {
                    ReviewKind::DocumentExport => review.field.text(),
                    ReviewKind::FileChange => review
                        .proposed_text
                        .as_deref()
                        .unwrap_or(&review.request.source),
                    ReviewKind::ShellCommand | ReviewKind::Script => &review.request.source,
                };
                let copied = platform::copy_to_clipboard(copy_text);
                self.show_global_notice(if copied {
                    "已复制"
                } else {
                    "复制失败，请重试"
                });
            }
            33 | 38 => review.field.scroll_multiline(
                l.field,
                if key == 33 {
                    l.field.height() * 0.9
                } else {
                    40.0
                },
            ),
            34 | 40 => review.field.scroll_multiline(
                l.field,
                if key == 34 {
                    -l.field.height() * 0.9
                } else {
                    -40.0
                },
            ),
            36 => review.field.scroll_multiline(l.field, f32::MAX),
            35 => review.field.scroll_multiline(l.field, -f32::MAX),
            _ => {}
        }
        true
    }
    pub(super) fn command_review_action(&mut self, action: Hit) {
        let Some(review) = self.commands.review.as_ref() else {
            return;
        };
        let id = review.request.id.clone();
        let session_id = review.request.session_id.clone();
        let review_kind = review.kind();
        let copy_text = match review_kind {
            ReviewKind::DocumentExport => review.field.text().to_owned(),
            ReviewKind::FileChange => review
                .proposed_text
                .clone()
                .unwrap_or_else(|| review.request.source.clone()),
            ReviewKind::ShellCommand | ReviewKind::Script => review.request.source.clone(),
        };
        if action == Hit::Execute && !review.error.is_empty() {
            return;
        }
        if let Some(entry) = review.file.clone() {
            let entries = if review.files.is_empty() {
                vec![entry.clone()]
            } else {
                review.files.clone()
            };
            match action {
                Hit::Execute => self.apply_reviewed_file(),
                Hit::Reject => {
                    if let Err(error) = self.reject_inbox_approval_entries(&entries) {
                        self.state.status_text = error.clone();
                        if let Some(review) = self.commands.review.as_mut() {
                            review.error = error;
                        }
                        return;
                    }
                    self.commands.review = None;
                    self.state.status_text = if entries.len() > 1 {
                        format!("已拒绝 {} 项文档修改", entries.len())
                    } else {
                        "已拒绝文件修改".into()
                    };
                    self.refresh_right_panel();
                }
                Hit::Cancel => self.commands.review = None,
                Hit::Copy => {
                    let copied = platform::copy_to_clipboard(&review.request.source);
                    self.show_global_notice(if copied {
                        "已复制"
                    } else {
                        "复制失败，请重试"
                    });
                }
            }
            return;
        }
        if review_kind == ReviewKind::DocumentExport {
            match action {
                Hit::Copy => {
                    let copied = platform::copy_to_clipboard(&copy_text);
                    self.show_global_notice(if copied {
                        "已复制导出信息"
                    } else {
                        "复制失败，请重试"
                    });
                }
                Hit::Cancel => self.commands.review = None,
                Hit::Reject => {
                    if let Some(queue) = &self.ai.export_requests {
                        if let Err(error) = queue.reject_pending(&id) {
                            let error = error.to_string();
                            self.state.status_text = error.clone();
                            if let Some(review) = self.commands.review.as_mut() {
                                review.error = error;
                            }
                            return;
                        }
                    }
                    self.commands.review = None;
                    self.state.status_text = "已拒绝导出请求".into();
                    self.refresh_right_panel();
                }
                Hit::Execute => {
                    if self.approve_document_export(&id) {
                        self.commands.review = None;
                        self.refresh_right_panel();
                    }
                }
            }
            return;
        }
        match action {
            Hit::Copy => {
                let copied = platform::copy_to_clipboard(&review.request.source);
                self.show_global_notice(if copied {
                    "已复制"
                } else {
                    "复制失败，请重试"
                });
            }
            Hit::Cancel => self.commands.review = None,
            Hit::Reject => {
                if let Some(queue) = &self.ai.export_requests {
                    if let Err(error) = queue.reject_pending(&id) {
                        let error = error.to_string();
                        self.state.status_text = error.clone();
                        if let Some(review) = self.commands.review.as_mut() {
                            review.error = error;
                        }
                        return;
                    }
                }
                self.ai_reject_shell_card(&id, session_id.as_deref());
                self.commands.review = None;
                self.refresh_right_panel();
            }
            Hit::Execute => self.execute_approved_command(&id),
        }
    }
    pub(super) fn execute_approved_command(&mut self, id: &str) {
        if !self.ai.permissions.as_ref().is_some_and(|p| {
            p.is_action_allowed(mochi_core::ai::permission::AiToolAction::ExecuteCommand)
        }) {
            if let Some(review) = self.commands.review.as_mut() {
                review.error = "执行命令权限已关闭；未执行。".into();
            }
            return;
        }
        let (Some(queue), Some(root)) = (
            self.ai.export_requests.clone(),
            self.shell.workspace().map(|w| w.root.clone()),
        ) else {
            return;
        };
        let request = match queue.claim(id) {
            Ok(r) => r,
            Err(e) => {
                let error = e.to_string();
                self.state.status_text = error.clone();
                if let Some(review) = self.commands.review.as_mut() {
                    review.error = error;
                }
                return;
            }
        };
        if request.format != "shell" {
            queue.release(id);
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.commands.jobs.insert(id.into(), cancel.clone());
        self.commands.review = None;
        self.commands.pressed = None;
        self.refresh_right_panel();
        self.state.status_text = "已批准，正在执行命令…".into();
        self.file_jobs.submit(root.clone(), self.hwnd_raw, move || {
            let timeout = request.shell_data()["timeoutMs"]
                .as_u64()
                .filter(|n| *n > 0)
                .map(|n| n.min(300_000));
            let data = request.shell_data();
            let result = if data.get("script").is_some() || data["kind"] == "script" {
                mochi_core::script::run_approved(
                    &data,
                    &request.source,
                    &request.output,
                    &root,
                    &cancel,
                )
            } else {
                mochi_core::shell::run_command_cancellable(
                    &request.source,
                    Path::new(&request.output),
                    timeout,
                    &cancel,
                )
            };
            let saved = queue.complete_shell(&request.id, &result);
            let (request, error) = match saved {
                Ok(r) => (r, None),
                Err(e) => {
                    let mut r = request;
                    r.state = if result.ok { "applied" } else { "error" }.into();
                    let mut data = r.shell_data();
                    data["result"] = serde_json::to_value(&result)?;
                    r.content = Some(data.to_string());
                    (r, Some(e.to_string()))
                }
            };
            Ok(crate::file_runtime::Payload::CommandFinished {
                root,
                request,
                queue,
                error,
            })
        });
    }
    pub(super) fn finish_command(
        &mut self,
        root: PathBuf,
        request: Request,
        queue: Arc<Queue>,
        error: Option<String>,
    ) {
        self.commands.jobs.remove(&request.id);
        let status = if request.state == "applied" {
            "命令已执行"
        } else {
            "命令执行失败或已取消"
        };
        match self
            .save_command_result(&root, &request)
            .and_then(|_| queue.remove(&request.id))
        {
            Ok(()) => {
                self.commands.unsaved.retain(|(_, r, _)| r.id != request.id);
                if self.commands.unsaved.is_empty() {
                    self.commands.result_error = None;
                }
                self.state.status_text = error
                    .map(|e| format!("{status}；审批记录保存失败：{e}"))
                    .unwrap_or_else(|| status.into());
            }
            Err(e) => {
                self.state.status_text = format!("命令已结束，但结果保存失败：{e}");
                self.commands.result_error = Some(e.to_string());
                if !self
                    .commands
                    .unsaved
                    .iter()
                    .any(|(_, r, _)| r.id == request.id)
                {
                    self.commands.unsaved.push((root, request, queue));
                }
            }
        }
    }
    fn save_command_result(&mut self, root: &Path, request: &Request) -> anyhow::Result<()> {
        let id = request
            .session_id
            .clone()
            .unwrap_or_else(|| format!("ai-conversation-{}", request.id));
        let svc = AiSessionService::new(root);
        svc.initialize()?;
        let active = self.shell.workspace().is_some_and(|w| w.root == root)
            && self.ai.panel.active.as_ref().is_some_and(|c| c.id == id);
        let mut conversation = if active {
            self.ai.panel.active.clone()
        } else {
            svc.load_session(&id)
        }
        .unwrap_or_else(|| AiConversation {
            id: id.clone(),
            title: "命令执行记录".into(),
            created_at: request.created_at,
            updated_at: request.created_at,
            messages: Vec::new(),
        });
        if !conversation.messages.iter().any(|m| {
            m.get("nativeCommandId").and_then(serde_json::Value::as_str) == Some(&request.id)
        }) {
            let data = request.shell_data();
            let result = data["result"].clone();
            let call_id = format!("approved-{}", request.id);
            let is_script = data.get("script").is_some() || data["kind"] == "script";
            let tool_name = if is_script { "script_run" } else { "shell_run" };
            let arguments = if is_script {
                data["script"].clone()
            } else {
                serde_json::json!({"command":request.source,"cwd":request.output})
            };
            let mut call = AiStoredMessage::new("assistant", "");
            call.set("hidden", true.into());
            call.set("tool_calls",serde_json::json!([{"id":call_id,"type":"function","function":{"name":tool_name,"arguments":arguments.to_string()}}]));
            conversation.messages.push(call);
            let mut response = AiStoredMessage::new("tool", &result.to_string());
            response.set("hidden", true.into());
            response.set("tool_call_id", call_id.into());
            response.set("name", tool_name.into());
            conversation.messages.push(response);
            let output = format!(
                "{}{}（退出码 {}）\n\n{}\n{}\n{}",
                if is_script { "脚本" } else { "命令" },
                if request.state == "applied" {
                    "已执行"
                } else {
                    "执行失败或已取消"
                },
                result["exitCode"],
                result["stdout"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(4000)
                    .collect::<String>(),
                result["stderr"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(2000)
                    .collect::<String>(),
                result["error"].as_str().unwrap_or("")
            );
            if let Some(message) = conversation.messages.iter_mut().find(|message| {
                message.get("pendingShellCommand").is_some_and(|pending| {
                    pending["id"].as_str() == Some(&request.id)
                        || pending["requestId"].as_str() == Some(&request.id)
                })
            }) {
                message.set_content(&output);
                message.set("nativeCommandId", request.id.clone().into());
                message.set("pendingShellCommand", data);
            } else {
                let mut message = AiStoredMessage::new("assistant", &output);
                message.set("id", request.id.clone().into());
                message.set("nativeCommandId", request.id.clone().into());
                message.set("timestamp", mochi_core::jstime::now_millis().into());
                message.set("pendingShellCommand", data);
                conversation.messages.push(message);
            }
        }
        conversation.updated_at = mochi_core::jstime::now_millis();
        svc.save_session(&conversation)?;
        let mut index = svc.load_index();
        let count = conversation
            .messages
            .iter()
            .filter(|m| !m.is_hidden())
            .count()
            .min(i32::MAX as usize) as i32;
        if let Some(meta) = index.sessions.iter_mut().find(|s| s.id == id) {
            meta.updated_at = conversation.updated_at;
            meta.message_count = count;
        } else {
            index.sessions.insert(
                0,
                AiSessionMeta {
                    id: id.clone(),
                    title: conversation.title.clone(),
                    created_at: conversation.created_at,
                    updated_at: conversation.updated_at,
                    message_count: count,
                    ..Default::default()
                },
            );
        }
        svc.save_index(&index)?;
        if active {
            self.ai.panel.active = Some(conversation);
            self.ai.panel.sessions = index.sessions;
        }
        Ok(())
    }
    pub(super) fn recover_command_results(&mut self) {
        let mut pending = std::mem::take(&mut self.commands.unsaved);
        if let (Some(queue), Some(root)) = (
            self.ai.export_requests.clone(),
            self.shell.workspace().map(|w| w.root.clone()),
        ) {
            pending.extend(
                queue
                    .completed_shells()
                    .into_iter()
                    .map(|r| (root.clone(), r, queue.clone())),
            );
        }
        let mut seen = HashSet::new();
        for (root, request, queue) in pending {
            if seen.insert(request.id.clone()) {
                self.finish_command(root, request, queue, None);
            }
        }
    }
    pub fn take_deferred_close(&mut self) -> bool {
        if self.commands.close_after
            && self.commands.jobs.is_empty()
            && self.commands.result_error.is_none()
        {
            self.commands.close_after = false;
            true
        } else {
            false
        }
    }
    pub(super) fn stop_commands_for_close(&mut self) -> bool {
        if !self.commands.jobs.is_empty() {
            for cancel in self.commands.jobs.values() {
                cancel.store(true, Ordering::Relaxed);
            }
            self.commands.close_after = true;
            self.state.status_text = "正在停止命令并保存结果，随后关闭窗口…".into();
            false
        } else if self.commands.result_error.is_some() {
            self.recover_command_results();
            self.commands.result_error.is_none()
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_approval_executes_exact_source_and_persists_script_tool_result() {
        use mochi_core::ai::tools::{script_tools::ScriptToolExecutor, ToolArgs, ToolExecutor};
        let parent = std::env::temp_dir().join(format!(
            "mochi-script-approval-{}",
            mochi_core::paths::random_base36(16)
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
            app.ai_new_session();
            let session = app.ai.panel.active.as_ref().unwrap().id.clone();
            app.ai.snapshot.lock().unwrap().session_id = Some(session);
            let args = serde_json::json!({"language":"powershell","code":"param()\nWrite-Output 'script-审批成功'","summary":"只输出测试标记，不修改文档","intent":"inspect","timeoutMs":5000,"input":{"path":"中文样本.csv"}});
            let tool = ScriptToolExecutor::new(app.ai.host.as_ref().unwrap().clone());
            let proposal = tool
                .call("script_run", &ToolArgs::from_value(args.clone()))
                .unwrap();
            let id = proposal["pendingShellCommand"]["id"].as_str().unwrap();
            app.review_command(id);
            let review = app.commands.review.as_ref().unwrap();
            assert_eq!(review.kind(), ReviewKind::Script);
            assert!(review.field.text().contains("不是安全沙箱"));
            assert!(review.field.text().contains("中文样本.csv"));
            assert!(review
                .field
                .text()
                .ends_with(args["code"].as_str().unwrap()));
            app.execute_approved_command(id);
            assert!(app.commands.jobs.is_empty());
            let permissions = app.ai.permissions.as_ref().unwrap();
            let mut actions = permissions.action_permissions();
            actions.set(
                mochi_core::ai::permission::AiToolAction::ExecuteCommand,
                true,
            );
            permissions.set_action_permissions(actions);
            app.execute_approved_command(id);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !app.commands.jobs.is_empty() && std::time::Instant::now() < deadline {
                app.take_file_jobs();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(app.commands.jobs.is_empty());
            assert!(app.commands.result_error.is_none());
            let messages = &app.ai.panel.active.as_ref().unwrap().messages;
            assert!(messages
                .iter()
                .any(|m| m.get("name").and_then(serde_json::Value::as_str) == Some("script_run")));
            let result = messages
                .iter()
                .find(|m| m.get("nativeCommandId").and_then(serde_json::Value::as_str) == Some(id))
                .unwrap();
            assert!(
                result.content().contains("script-审批成功"),
                "{}",
                result.content()
            );
            assert_eq!(
                result.get("pendingShellCommand").unwrap()["script"]["code"],
                args["code"]
            );
            assert_eq!(
                result.get("pendingShellCommand").unwrap()["status"],
                "applied"
            );
            assert!(app
                .ai
                .export_requests
                .as_ref()
                .unwrap()
                .pending()
                .is_empty());
        }
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn rejecting_shell_command_updates_request_session_when_another_session_is_active() {
        let parent = std::env::temp_dir().join(format!(
            "mochi-command-reject-session-{}",
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

            let target_id = "target-session".to_owned();
            let active_id = "active-session".to_owned();
            let cwd = root.to_string_lossy().into_owned();
            let request = app
                .ai
                .export_requests
                .as_ref()
                .unwrap()
                .add_for_session(
                    "Write-Output approval",
                    "shell",
                    &cwd,
                    Some("{}".into()),
                    Some(target_id.clone()),
                )
                .unwrap();
            let request_id = request.id.clone();

            let mut target_message = AiStoredMessage::new("assistant", "等待命令审批");
            target_message.set(
                "pendingShellCommand",
                serde_json::json!({
                    "id": request_id.clone(),
                    "requestId": request.id.clone(),
                    "command": request.source.clone(),
                    "cwd": cwd.clone(),
                    "status": "pending"
                }),
            );
            let target = AiConversation {
                id: target_id.clone(),
                title: "原会话".into(),
                messages: vec![target_message],
                ..Default::default()
            };
            let mut active_message = AiStoredMessage::new("assistant", "当前会话的命令");
            active_message.set(
                "pendingShellCommand",
                serde_json::json!({
                    "id": "unrelated-command",
                    "status": "pending"
                }),
            );
            let active = AiConversation {
                id: active_id.clone(),
                title: "当前会话".into(),
                messages: vec![active_message],
                ..Default::default()
            };
            let service = AiSessionService::new(&root);
            service.save_session(&target).unwrap();
            service.save_session(&active).unwrap();
            let mut index = service.load_index();
            index.sessions.extend([
                AiSessionMeta {
                    id: target_id.clone(),
                    title: target.title.clone(),
                    ..Default::default()
                },
                AiSessionMeta {
                    id: active_id.clone(),
                    title: active.title.clone(),
                    ..Default::default()
                },
            ]);
            service.save_index(&index).unwrap();
            app.ai.panel.active = Some(active);

            app.review_command(&request_id);
            assert_eq!(
                app.commands
                    .review
                    .as_ref()
                    .and_then(|review| review.request.session_id.as_deref()),
                Some(target_id.as_str())
            );
            app.command_review_action(Hit::Reject);

            assert!(app
                .ai
                .export_requests
                .as_ref()
                .unwrap()
                .pending()
                .is_empty());
            assert_eq!(
                app.ai
                    .panel
                    .active
                    .as_ref()
                    .map(|conversation| conversation.id.as_str()),
                Some(active_id.as_str())
            );
            assert_eq!(
                app.ai.panel.active.as_ref().unwrap().messages[0]
                    .get("pendingShellCommand")
                    .unwrap()["status"],
                "pending"
            );
            let saved = service.load_session(&target_id).unwrap();
            assert_eq!(
                saved.messages[0].get("pendingShellCommand").unwrap()["status"],
                "rejected"
            );
        }
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn document_export_review_uses_export_details_and_rejects_without_shell_side_effects() {
        let parent = std::env::temp_dir().join(format!(
            "mochi-export-review-{}",
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
            let source = root.join("note.md");
            let output = root.join("note.html");
            std::fs::write(&source, "# 审批详情\n").unwrap();
            let request = app
                .ai
                .export_requests
                .as_ref()
                .unwrap()
                .add(
                    &source.to_string_lossy(),
                    "html",
                    &output.to_string_lossy(),
                    Some("# 审批详情\n".into()),
                )
                .unwrap();

            app.review_document_export(&request.id);
            let review = app.commands.review.as_ref().unwrap();
            assert_eq!(review.kind(), ReviewKind::DocumentExport);
            assert!(review.field.text().contains("导出格式：html"));
            assert!(review
                .field
                .text()
                .contains(&output.to_string_lossy().to_string()));

            app.command_review_action(Hit::Reject);
            assert!(app.commands.review.is_none());
            assert!(app
                .ai
                .export_requests
                .as_ref()
                .unwrap()
                .pending()
                .is_empty());
            assert_eq!(app.state.status_text, "已拒绝导出请求");
            assert!(!output.exists());
        }
        let _ = std::fs::remove_dir_all(parent);
    }
}
