//! 审批副作用由 App 处理；assistant 只保存面板状态和几何。

use std::path::{Path, PathBuf};

use mochi_core::ai::agent_inbox::{AgentInboxService, InboxEntry};
use mochi_core::ai::models::AiMessage;
use mochi_core::ai::session::AiStoredMessage;
use serde_json::Value;

use super::*;
use crate::ui::assistant::context::{
    self, attach_selection_contexts, build_selection_prompt, pending_card_from_message,
    pending_card_from_value, remove_pending_selection, selection_from_document, PendingCardKind,
    ReviewTarget, SelectionContext,
};

fn result_data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

fn result_field(value: &Value, key: &str) -> Option<Value> {
    result_data(value).get(key).cloned()
}

fn result_string(value: &Value, key: &str) -> Option<String> {
    result_field(value, key)
        .and_then(|value| value.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
}

fn field_object(value: &Value, key: &str) -> Option<Value> {
    result_field(value, key).filter(|value| value.is_object())
}

fn with_string(mut value: Value, key: &str, source: Option<String>) -> Value {
    if let (Some(object), Some(source)) = (value.as_object_mut(), source) {
        object
            .entry(key.to_owned())
            .or_insert(Value::String(source));
    }
    value
}

fn proposal_operation(entry: &InboxEntry) -> Option<Value> {
    let value = entry.to_value();
    let mut operation = value.get("operation")?.as_object()?.clone();
    operation.insert("inboxId".into(), Value::String(entry.id().to_owned()));
    if let Some(id) = operation.get("id").cloned() {
        operation.entry("operationId").or_insert(id);
    }
    Some(Value::Object(operation))
}

fn candidate_block_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .get(key)
        .or_else(|| value.get("blockEdit").and_then(|edit| edit.get(key)))
}

/// 把工具返回的操作与持有它的持久化收件箱条目配对。
/// 批量块响应为每个操作带一个 `inboxId`，但旧宿主可能只暴露
/// 操作 id 或块载荷本身。
fn pending_entry_for_candidate<'a>(
    value: &Value,
    pending_entries: &'a [InboxEntry],
) -> Option<&'a InboxEntry> {
    let inbox_id = value.get("inboxId").and_then(Value::as_str);
    if let Some(entry) =
        inbox_id.and_then(|id| pending_entries.iter().find(|entry| entry.id() == id))
    {
        return Some(entry);
    }

    let operation_id = value
        .get("operationId")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str);
    if let Some(entry) = operation_id.and_then(|id| {
        pending_entries.iter().find(|entry| {
            let value = entry.to_value();
            value["operation"]["id"].as_str() == Some(id)
        })
    }) {
        return Some(entry);
    }

    let block_id = candidate_block_field(value, "blockId").and_then(Value::as_str);
    let original_hash = candidate_block_field(value, "originalHash").and_then(Value::as_str);
    let path = candidate_block_field(value, "path").and_then(Value::as_str);
    if block_id.is_some() || original_hash.is_some() {
        return pending_entries.iter().find(|entry| {
            let entry_value = entry.to_value();
            let operation = &entry_value["operation"];
            entry.kind() == "block-edit"
                && block_id.is_some_and(|expected| {
                    candidate_block_field(operation, "blockId").and_then(Value::as_str)
                        == Some(expected)
                })
                && original_hash.is_some_and(|expected| {
                    candidate_block_field(operation, "originalHash").and_then(Value::as_str)
                        == Some(expected)
                })
                && path.is_none_or(|expected| {
                    entry
                        .path()
                        .replace('\\', "/")
                        .eq_ignore_ascii_case(&expected.replace('\\', "/"))
                })
        });
    }
    None
}

/// 为工具返回的操作补充持久身份和展示字段。
/// `pendingFileOperations[]` 是首选来源；收件箱查询让旧的/无头的
/// 信封保持兼容，而不用凭空编一个审批 id。
fn enrich_pending_operation(
    value: Value,
    pending_entries: &[InboxEntry],
    fallback_inbox_id: Option<&str>,
    fallback_operation_id: Option<&str>,
) -> Option<Value> {
    let matched = pending_entry_for_candidate(&value, pending_entries);
    let matched_inbox_id = matched.map(|entry| entry.id().to_owned());
    let matched_operation_id = matched.and_then(|entry| {
        let entry_value = entry.to_value();
        entry_value["operation"]["id"].as_str().map(str::to_owned)
    });
    let mut operation = value.as_object()?.clone();
    if let Some(entry) = matched {
        if let Some(Value::Object(canonical)) = proposal_operation(entry) {
            for (key, value) in canonical {
                operation.entry(key).or_insert(value);
            }
        }
    }
    let operation_inbox_id = operation
        .get("inboxId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let inbox_id = matched_inbox_id
        .or(operation_inbox_id)
        .or_else(|| fallback_inbox_id.map(str::to_owned));
    if let Some(inbox_id) = inbox_id {
        operation
            .entry("inboxId")
            .or_insert(Value::String(inbox_id));
    }
    let operation_field_id = operation
        .get("operationId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(matched_operation_id)
        .or_else(|| {
            operation
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| fallback_operation_id.map(str::to_owned));
    if let Some(operation_id) = operation_field_id {
        operation
            .entry("operationId")
            .or_insert(Value::String(operation_id));
    }
    Some(Value::Object(operation))
}

fn proposal_key(field: &str, value: &Value) -> String {
    let id = value
        .get("inboxId")
        .or_else(|| value.get("requestId"))
        .or_else(|| value.get("operationId"))
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if id.is_empty() {
        format!("{field}:{value}")
    } else {
        format!("{field}:{id}")
    }
}

fn same_card(field: &str, candidate: &Value, message: &AiStoredMessage) -> bool {
    let Some(existing) = message.get(field).filter(|value| value.is_object()) else {
        return false;
    };
    let candidate_key = proposal_key(field, candidate);
    !candidate_key.ends_with(":{}") && candidate_key == proposal_key(field, existing)
}

fn append_candidate(
    candidates: &mut Vec<(String, PendingCardKind, Value)>,
    keys: &mut Vec<String>,
    field: &str,
    kind: PendingCardKind,
    value: Value,
) {
    if !value.is_object() {
        return;
    }
    let key = proposal_key(field, &value);
    if keys.iter().any(|existing| existing == &key) {
        return;
    }
    keys.push(key);
    candidates.push((field.to_owned(), kind, value));
}

impl App {
    /// 读取当前编辑器的选区，返回 Electron 兼容的 `DocumentSelectionContext` JSON。
    pub(super) fn ai_current_selection(&self) -> Option<Value> {
        let tab = self.shell.active()?;
        let path = tab.path()?.to_string_lossy().replace('\\', "/");
        let title = tab.title.clone();
        let buffer = tab.buffer()?;
        selection_from_document(&path, Some(&title), buffer.text(), buffer.selection())
            .map(|selection| selection.to_value())
    }

    pub(super) fn ai_remove_pending_selection(pending: &mut Vec<Value>, index: usize) -> bool {
        remove_pending_selection(pending, index)
    }

    /// 给新建 user 消息写入数组字段和旧版单选区字段。
    pub(super) fn ai_attach_selection_contexts(
        message: &mut AiStoredMessage,
        pending: &[Value],
    ) -> usize {
        attach_selection_contexts(message, pending)
    }

    pub(super) fn ai_selection_prompt(pending: &[Value], user_request: &str) -> Option<String> {
        build_selection_prompt(pending, user_request)
    }

    /// 点击已发送的选区卡后打开原文、选中对应行，并把焦点交给编辑器。
    ///
    /// 路径必须落在当前工作区内；行号是首选定位依据，保存的字节偏移只作
    /// 同一文件内容下的回退。这样跨端换行或文档重排时不会把选区定位到别处。
    pub(super) fn ai_locate_selection_value(&mut self, value: &Value) -> bool {
        let Some(selection) = SelectionContext::from_value(value) else {
            self.state.status_text = "选区上下文无效，请重新选择".into();
            return false;
        };
        let Some(path) = self.ai_selection_path(&selection.path) else {
            self.state.status_text = "选区文档不在当前工作区内或已不存在".into();
            return false;
        };
        if !self.open_file_from_ui(&path) {
            self.state.status_text = "无法打开选区文档".into();
            return false;
        }
        self.state.view = WorkspaceView::Editor;
        self.editor_engaged = true;

        let range = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .and_then(|buffer| {
                if !selection.blocks.is_empty() {
                    let document =
                        mochi_core::document_blocks::document_from_source(&path, buffer.text())
                            .ok()?;
                    let blocks = selection
                        .blocks
                        .iter()
                        .map(|block| document.find_block(block.id.as_str()).ok())
                        .collect::<Option<Vec<_>>>()?;
                    return Some((
                        blocks.iter().map(|b| b.source_span.start).min()?,
                        blocks.iter().map(|b| b.source_span.end).max()?,
                    ));
                }
                context::line_byte_range(buffer.text(), selection.start_line, selection.end_line)
                    .or_else(|| {
                        (selection.to <= buffer.text().len()
                            && selection.from <= selection.to
                            && buffer.text().is_char_boundary(selection.from)
                            && buffer.text().is_char_boundary(selection.to))
                        .then_some((selection.from, selection.to))
                    })
            });
        let Some((start, end)) = range else {
            self.state.status_text = "无法在当前文档中定位选区行".into();
            self.invalidate_main();
            self.sync_state();
            return false;
        };
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(start, false);
            buffer.set_cursor(end, true);
        }
        self.focus = Focus::Main;
        if self.content() == MainContent::Document {
            self.after_doc_selection_change(false);
        } else {
            self.after_source_selection_change(false);
        }
        self.state.status_text = format!(
            "已定位到 {} 第 {}-{} 行",
            selection.display_name(),
            selection.start_line,
            selection.end_line
        );
        self.invalidate_main();
        self.sync_state();
        true
    }

    fn ai_selection_path(&self, raw: &str) -> Option<PathBuf> {
        let workspace_root = self.shell.workspace()?.root.canonicalize().ok()?;
        let candidate = Path::new(raw);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            workspace_root.join(candidate)
        };
        let resolved = candidate.canonicalize().ok()?;
        (resolved.is_file() && resolved.starts_with(&workspace_root)).then_some(resolved)
    }

    /// 从本轮 tool transcript 收集持久化审批卡。
    ///
    /// `submit_proposal` 的 native 返回值只有 `{status, operationId, inboxId,
    /// message}`，所以文件操作要用 inboxId 回读真实 operation。块工具的批量
    /// 回执使用 `pendingFileOperations[]`/`pendingBlockEdits[]`；这里把每项
    /// 展开成独立卡片，并用 Inbox 的持久化身份去重。已有 `pendingEdit` 由
    /// app.rs 单独追加，这里跳过它以免同一提案出现两张卡。
    /// 返回追加的可见消息数量。
    pub(super) fn ai_append_pending_cards(&mut self, transcript: &[AiMessage]) -> usize {
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            return 0;
        };
        let inbox = AgentInboxService::new(&root);
        let pending_entries = inbox.list_pending();
        let mut candidates = Vec::new();
        let mut keys = Vec::new();
        for message in transcript.iter().filter(|message| message.role == "tool") {
            let Some(content) = message.content.as_deref() else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(content) else {
                continue;
            };
            let data = result_data(&value);

            let mut file_values = Vec::new();
            let has_batch_file_values = data
                .get("pendingFileOperations")
                .and_then(Value::as_array)
                .map(|values| {
                    file_values.extend(values.iter().filter(|value| value.is_object()).cloned());
                    !values.is_empty()
                })
                .unwrap_or(false);
            if let Some(value) = data
                .get("pendingFileOperation")
                .filter(|value| value.is_object())
                .cloned()
            {
                file_values.push(value);
            }
            if file_values.is_empty() {
                if let Some(inbox_id) = data.get("inboxId").and_then(Value::as_str) {
                    if let Some(operation) = pending_entries
                        .iter()
                        .find(|entry| entry.id() == inbox_id)
                        .and_then(proposal_operation)
                    {
                        file_values.push(operation);
                    }
                }
            }
            if data.get("pendingEdit").is_none() {
                let fallback_inbox_id = (!has_batch_file_values)
                    .then(|| result_string(&value, "inboxId"))
                    .flatten();
                let fallback_operation_id = (!has_batch_file_values)
                    .then(|| result_string(&value, "operationId"))
                    .flatten();
                for raw_operation in file_values {
                    let Some(operation) = enrich_pending_operation(
                        raw_operation,
                        &pending_entries,
                        fallback_inbox_id.as_deref(),
                        fallback_operation_id.as_deref(),
                    ) else {
                        continue;
                    };
                    append_candidate(
                        &mut candidates,
                        &mut keys,
                        "pendingFileOperation",
                        PendingCardKind::FileOperation,
                        operation,
                    );
                }
            }

            // 块编辑只有在能关联到一条持久的收件箱操作时才可执行。
            // AppHost 通常会同时给出两个数组；这个回退覆盖只返回
            // pendingBlockEdit(s) 的旧宿主，同时避免产生孤儿审批卡。
            let mut block_values = Vec::new();
            if let Some(values) = data.get("pendingBlockEdits").and_then(Value::as_array) {
                block_values.extend(values.iter().filter(|value| value.is_object()).cloned());
            }
            if let Some(value) = data
                .get("pendingBlockEdit")
                .filter(|value| value.is_object())
                .cloned()
            {
                block_values.push(value);
            }
            for block_value in block_values {
                let Some(operation) =
                    enrich_pending_operation(block_value, &pending_entries, None, None)
                else {
                    continue;
                };
                let linked = operation
                    .get("inboxId")
                    .or_else(|| operation.get("operationId"))
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty());
                if linked {
                    append_candidate(
                        &mut candidates,
                        &mut keys,
                        "pendingFileOperation",
                        PendingCardKind::FileOperation,
                        operation,
                    );
                }
            }

            if let Some(mut shell) = field_object(&value, "pendingShellCommand") {
                let request_id = shell
                    .get("requestId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                shell = with_string(shell, "id", request_id);
                append_candidate(
                    &mut candidates,
                    &mut keys,
                    "pendingShellCommand",
                    PendingCardKind::ShellCommand,
                    shell,
                );
            }
            for (field, kind) in [
                ("pendingPluginChange", PendingCardKind::PluginChange),
                ("pendingPluginPermission", PendingCardKind::PluginPermission),
                ("pendingScheduleDiff", PendingCardKind::ScheduleDiff),
                ("pendingConsoleAction", PendingCardKind::ConsoleAction),
            ] {
                if let Some(candidate) = field_object(&value, field) {
                    append_candidate(&mut candidates, &mut keys, field, kind, candidate);
                }
            }
        }

        // 一次规划轮常常创建多个任务。它们是同一个逻辑上的日程变更，
        // 而不是一摞一模一样的审批卡。呈现为一张聚合卡，
        // 但保留每一条操作。
        let mut schedule_values = Vec::new();
        candidates.retain(|(_, kind, value)| {
            if *kind == PendingCardKind::ScheduleDiff {
                schedule_values.push(value.clone());
                false
            } else {
                true
            }
        });
        for all_applied in [false, true] {
            let matching: Vec<_> = schedule_values
                .iter()
                .filter(|value| {
                    (value.get("status").and_then(Value::as_str) == Some("applied")) == all_applied
                })
                .collect();
            if matching.is_empty() {
                continue;
            }
            // 同一份工具记录再次送达时（包括卡片已处理之后），
            // 聚合身份保持不变。
            use std::hash::{Hash, Hasher};
            let mut source_ids: Vec<_> = matching
                .iter()
                .map(|value| {
                    value
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| value["operations"].to_string())
                })
                .collect();
            source_ids.sort();
            let mut identity = std::collections::hash_map::DefaultHasher::new();
            source_ids.hash(&mut identity);
            let batch_id = format!("schedule-batch-{:x}", identity.finish());
            let operations = matching
                .iter()
                .flat_map(|value| {
                    value
                        .get("operations")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .cloned()
                })
                .collect::<Vec<_>>();
            let summary = if all_applied {
                format!("已自动执行 {} 项日程变更", operations.len())
            } else {
                format!("{} 项日程变更等待批准", operations.len())
            };
            candidates.push((
                "pendingScheduleDiff".into(),
                PendingCardKind::ScheduleDiff,
                serde_json::json!({
                    "id": batch_id,
                    "title": "日程变更",
                    "summary": summary,
                    "status": if all_applied { "applied" } else { "pending" },
                    "workspaceRoot": self.shell.workspace().map(|workspace| workspace.root.to_string_lossy().into_owned()),
                    "operations": operations,
                }),
            ));
        }
        if candidates.is_empty() {
            return 0;
        }
        let now = mochi_core::jstime::now_millis();
        let mut resolved_existing = false;
        let appended = {
            let Some(conversation) = self.ai.panel.active.as_mut() else {
                return 0;
            };
            let mut appended = 0usize;
            for (index, (field, kind, value)) in candidates.into_iter().enumerate() {
                if let Some(message) = conversation
                    .messages
                    .iter_mut()
                    .find(|message| same_card(&field, &value, message))
                {
                    if kind == PendingCardKind::ScheduleDiff
                        && value["status"] == "applied"
                        && message
                            .get(&field)
                            .is_some_and(|existing| existing["status"] == "pending")
                    {
                        let summary = value["summary"].as_str().unwrap_or("日程计划已应用");
                        message.set_content(&format!(
                            "{summary}\n\n{}",
                            super::agenda_review::preview_markdown(&value)
                        ));
                        message.set(&field, value);
                        resolved_existing = true;
                    }
                    continue;
                }
                let card = pending_card_from_value(&field, kind, value.clone(), None);
                let summary = card
                    .as_ref()
                    .map(|card| card.summary.clone())
                    .unwrap_or_else(|| "待审批操作".into());
                let content = if kind == PendingCardKind::ScheduleDiff {
                    format!(
                        "{}\n\n{}",
                        summary,
                        super::agenda_review::preview_markdown(&value)
                    )
                } else if kind == PendingCardKind::ConsoleAction {
                    format!(
                        "{summary}\n\n```json\n{}\n```",
                        serde_json::to_string_pretty(&value["args"]).unwrap_or_default()
                    )
                } else {
                    format!("{}：{summary}", kind.label())
                };
                let mut message = AiStoredMessage::new("assistant", &content);
                message.set(
                    "id",
                    Value::String(format!("msg-{now}-pending-card-{index}")),
                );
                message.set("timestamp", Value::from(now + index as i64));
                message.set(&field, value);
                conversation.messages.push(message);
                appended += 1;
            }
            if appended > 0 || resolved_existing {
                conversation.updated_at = now;
            }
            appended
        };
        if appended > 0 || resolved_existing {
            let _ = self.ai_persist_active();
        }
        appended
    }

    /// 用消息可见下标打开对应的真实 review 面板。
    ///
    /// 文件、Shell 与日程提案分别进入各自的预览与批准入口。
    pub(super) fn ai_review_pending_message(&mut self, visible_index: usize) -> bool {
        let Some(card) = self.ai.panel.active.as_ref().and_then(|conversation| {
            conversation
                .messages
                .iter()
                .filter(|message| mochi_core::ai::locator::visible(message))
                .nth(visible_index)
                .and_then(pending_card_from_message)
        }) else {
            return false;
        };
        match card.review {
            ReviewTarget::File {
                inbox_id,
                operation_id,
            } => {
                let Some(root) = self
                    .shell
                    .workspace()
                    .map(|workspace| workspace.root.clone())
                else {
                    self.ai.panel.error = "没有工作区，无法查看文件审批".into();
                    return false;
                };
                let service = AgentInboxService::new(&root);
                let entry = service.list_pending().into_iter().find(|entry| {
                    inbox_id.as_deref() == Some(entry.id())
                        || operation_id.as_deref() == entry.to_value()["operation"]["id"].as_str()
                });
                let Some(entry) = entry else {
                    self.ai.panel.error = "文件提案已处理或收件箱记录不存在，请刷新后重试".into();
                    return false;
                };
                self.review_file_proposal(entry);
                true
            }
            ReviewTarget::Shell { request_id } => {
                self.review_command(&request_id);
                if self.commands.review.is_some() {
                    true
                } else {
                    self.ai.panel.error = "命令审批记录已处理或不存在，请刷新待批准操作".into();
                    false
                }
            }
            ReviewTarget::Schedule => self.open_schedule_review(card.raw),
            ReviewTarget::Console => self.open_console_review(card.raw),
            ReviewTarget::None => false,
        }
    }
}
