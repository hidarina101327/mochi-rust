//! 发送和取消 AI 请求，并接收、整理流式响应。
use super::*;

impl App {
    pub(super) fn ai_active_document(&self) -> Option<ActiveDocument> {
        let tab = self.shell.active()?;
        let path = tab.path()?.to_string_lossy().into_owned();
        Some(ActiveDocument {
            id: path.clone(),
            path,
            title: tab.title.clone(),
            is_dirty: tab.buffer().is_some_and(|buffer| buffer.dirty()),
        })
    }

    /// 发送：把输入追加为 user 消息、起后台线程跑 Agent。
    pub(super) fn ai_send(&mut self, hwnd: HWND) {
        self.ai_clear_text_selection();
        if self.ai.panel.is_streaming() {
            return;
        }
        let text = self.ai.panel.input.text().trim().to_owned();
        if text.is_empty()
            && self.ai.panel.pending_images.is_empty()
            && self.ai.panel.pending_files.is_empty()
            && self.ai.panel.pending_selections.is_empty()
        {
            return;
        }
        let Some(provider) = ai_runtime::load_provider(&self.settings) else {
            self.ai.panel.provider_missing = true;
            return;
        };
        let images = match self.prepare_pending_images() {
            Ok(images) => images,
            Err(e) => {
                self.ai.panel.error = format!("准备图片失败：{e}");
                return;
            }
        };
        let selection_prompt = Self::ai_selection_prompt(&self.ai.panel.pending_selections, &text);
        let request_text = selection_prompt.as_deref().unwrap_or(&text);
        let mut attachment = match mochi_core::ai::attachments::prepare(
            request_text,
            &self.ai.panel.pending_files,
            images.iter().map(String::len).sum(),
        ) {
            Ok(value) => value,
            Err(e) => {
                self.ai.panel.error = format!("准备附件失败：{e}");
                return;
            }
        };
        if selection_prompt.is_some() {
            attachment.display = format!(
                "{}{}",
                if text.is_empty() {
                    "已附加选区上下文"
                } else {
                    &text
                },
                attachment
                    .display
                    .strip_prefix(request_text)
                    .unwrap_or_default()
            );
        }
        if !self.allow_ai_request() {
            self.ai.panel.error = "请求过于频繁，请稍后重试或调整频率限制".into();
            return;
        }
        if self.ai.panel.active.is_none() {
            self.ai_new_session();
        }
        let before = self.ai.panel.active.clone();
        let now = mochi_core::jstime::now_millis();
        {
            let Some(c) = self.ai.panel.active.as_mut() else {
                return;
            };
            let mut msg = AiStoredMessage::new("user", &attachment.display);
            Self::ai_attach_selection_contexts(&mut msg, &self.ai.panel.pending_selections);
            msg.set("id", serde_json::json!(format!("msg-{now}")));
            msg.set("timestamp", serde_json::json!(now));
            msg.set(
                "agentId",
                serde_json::json!(self.ai.panel.selected_agent_id),
            );
            if !self.ai.panel.pending_images.is_empty() {
                msg.set("images", serde_json::json!(self.ai.panel.pending_images));
            }
            c.messages.push(msg);
            c.updated_at = now;
        }
        self.ai.panel.error.clear();
        if !self.ai_persist_active() {
            self.ai.panel.active = before;
            return;
        }
        if let Some(session_id) = self
            .ai
            .panel
            .active
            .as_ref()
            .map(|session| session.id.clone())
        {
            self.record_home_activity(mochi_core::analytics::events::ActivityInput {
                kind: Some(mochi_core::analytics::events::ActivityEventType::AiMessage),
                session_id: Some(session_id),
                role: Some("user".into()),
                ..Default::default()
            });
        }
        self.ai.panel.input.clear();
        self.ai.panel.pending_images.clear();
        self.ai.panel.pending_files.clear();
        self.ai.panel.pending_selections.clear();
        self.ai.panel.files_scroll = 0.0;
        self.ai.panel.image_placeholders.clear();
        self.ai.panel.next_image_index = 1;

        // 给模型的对话：跳过 hidden 之外的全部可见消息 + 工具往返（都在 transcript 里）
        let messages: Vec<AiMessage> = self
            .ai
            .panel
            .active
            .as_ref()
            .map(|c| {
                ai_runtime::messages_with_current_attachments(c, images, Some(attachment))
                    .expect("new user message exists")
            })
            .unwrap_or_default();

        // 宿主快照：当前库、当前文档
        {
            self.canvas_finish_editing();
            let active_doc = self.ai_active_document();
            if let Ok(mut snap) = self.ai.snapshot.lock() {
                snap.selected_knowledge_base = self.shell.tree_root();
                snap.active_document = active_doc;
                snap.active_text = self
                    .shell
                    .active()
                    .and_then(|t| t.buffer())
                    .map(|b| b.text().to_owned());
                snap.buffered_documents = self
                    .shell
                    .tabs()
                    .iter()
                    .filter_map(|tab| {
                        let path = tab.path()?.to_string_lossy().into_owned();
                        let buffer = tab.buffer()?;
                        if !buffer.dirty()
                            || snap
                                .active_document
                                .as_ref()
                                .is_some_and(|active| active.path == path)
                        {
                            return None;
                        }
                        Some(ai_runtime::BufferedDocument {
                            document: ActiveDocument {
                                id: path.clone(),
                                path,
                                title: tab.title.clone(),
                                is_dirty: buffer.dirty(),
                            },
                            text: buffer.text().to_owned(),
                        })
                    })
                    .collect();
                snap.typography = crate::ui::editor_preferences::current();
                snap.edit_apply_mode = app_settings::descriptor("ai.editApplyMode")
                    .map(|d| self.app_settings.read(d).to_storage())
                    .unwrap_or_else(|| "approve".into());
                snap.session_id = self.ai.panel.active.as_ref().map(|c| c.id.clone());
                snap.selected_agent_id = self.ai.panel.selected_agent_id.clone();
            }
        }
        let (Some(ws), Some(host)) = (self.shell.workspace(), self.ai.host.clone()) else {
            return;
        };
        self.ai.pending_memory = if crate::memory_runtime::enabled(&self.settings) {
            self.ai
                .panel
                .active
                .as_ref()
                .map(|c| crate::memory_runtime::Exchange {
                    root: ws.root.clone(),
                    session_id: c.id.clone(),
                    user_text: text.clone(),
                    provider: provider.clone(),
                })
        } else {
            None
        };
        let hwnd_raw = hwnd.0 as isize;
        let handle = ai_runtime::spawn_run(
            &ws.root,
            provider,
            messages,
            host,
            Arc::clone(&self.settings),
            Arc::clone(&ws.index),
            Arc::clone(&ws.git),
            move || unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_raw as *mut _)),
                    platform::WM_APP_AI_EVENT,
                    WPARAM(0),
                    LPARAM(0),
                );
            },
        );
        self.ai.run = Some(handle);
        let now = mochi_core::jstime::now_millis();
        self.ai.panel.streaming = Some(assistant::Streaming {
            status: "正在思考".into(),
            started_at_ms: now,
            last_event_at_ms: now,
            ..Default::default()
        });
        self.ai.animation_timer_armed = false;
        self.ai.panel.stick_to_bottom = true;
        // 工作线程发出 ModelStart 之前，界面会先绘制一次。此处立即设置
        // 标记，避免较长的已有会话在追加新消息时短暂显示
        // 旧的滚动位置。
        self.ai.panel.scroll = f32::MAX;
    }

    pub(super) fn ai_cancel(&mut self) {
        self.ai.pending_memory = None;
        let had_run = if let Some(run) = self.ai.run.take() {
            run.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        };
        if had_run {
            self.ai.panel.error = "已取消本次生成".into();
        }
        // 流式执行中断时，仍将已接收的内容保存为回复。
        self.ai_finish_streaming(None);
        if self.ai.panel.stick_to_bottom {
            self.ai.panel.scroll = f32::MAX;
        }
    }

    /// 结束流式：把累积内容落成 assistant 消息（`content` 给定时用它覆盖）。
    pub(super) fn ai_finish_streaming(&mut self, final_content: Option<String>) {
        let Some(mut st) = self.ai.panel.streaming.take() else {
            return;
        };
        let interrupted = final_content.is_none();
        let content = final_content.unwrap_or(st.content);
        if !st.reasoning.trim().is_empty() {
            st.trace
                .push(mochi_core::ai::agent_runner::AgentTraceStep::Thinking {
                    text: st.reasoning,
                    duration_ms: None,
                });
        }
        if content.trim().is_empty() && st.trace.is_empty() {
            return;
        }
        let now = mochi_core::jstime::now_millis();
        if let Some(c) = self.ai.panel.active.as_mut() {
            let mut msg = AiStoredMessage::new("assistant", &content);
            msg.set("id", serde_json::json!(format!("msg-{now}")));
            msg.set("timestamp", serde_json::json!(now));
            if !st.trace.is_empty() {
                msg.set("trace", serde_json::to_value(&st.trace).unwrap_or_default());
            }
            if !st.model.is_empty() {
                msg.set("model", serde_json::json!(st.model));
            }
            if interrupted {
                msg.set("interrupted", serde_json::json!(true));
            }
            c.messages.push(msg);
            c.updated_at = now;
        }
        self.ai_persist_active();
    }

    /// 运行线程投来的事件。一次消息可能积压多条，全部取完。
    pub fn take_ai_events(&mut self) {
        self.collect_memory_jobs();
        self.collect_follow_up_jobs();
        self.collect_title_jobs();
        let mut done: Option<RunEvent> = None;
        let mut settings_changed = false;
        let mut schedule_changed = false;
        {
            let Some(run) = self.ai.run.as_ref() else {
                return;
            };
            while let Ok(ev) = run.rx.try_recv() {
                match ev {
                    RunEvent::Done { .. } | RunEvent::Failed(_) => {
                        done = Some(ev);
                        break;
                    }
                    other => {
                        let Some(st) = self.ai.panel.streaming.as_mut() else {
                            continue;
                        };
                        st.last_event_at_ms = mochi_core::jstime::now_millis();
                        match other {
                            RunEvent::Skills(skills) => self.ai.panel.loaded_skills = skills,
                            RunEvent::Runtime {
                                agent_id,
                                agent_name,
                                skills,
                                model,
                            } => {
                                self.ai.panel.selected_agent_id = agent_id;
                                self.ai.panel.agent_name = agent_name;
                                self.ai.panel.loaded_skills = skills;
                                st.model = model.clone();
                                if let Some(exchange) = self.ai.pending_memory.as_mut() {
                                    exchange.provider.model = model;
                                }
                            }
                            RunEvent::Status(s) => st.status = s,
                            RunEvent::ModelStart => {
                                st.status = "正在思考…".into();
                                st.reasoning.clear();
                                st.content.clear();
                            }
                            RunEvent::Reasoning(d) => st.reasoning.push_str(&d),
                            RunEvent::Content(d) => st.content.push_str(&d),
                            RunEvent::Tool { name, ok, summary } => {
                                schedule_changed |= ok == Some(true) && name.starts_with("agenda_");
                                if ok.is_none() {
                                    st.content.clear();
                                    st.status = format!(
                                        "正在执行 {}",
                                        name.replace("mcp__", "MCP: ").replace("__", " / ")
                                    );
                                }
                                settings_changed |= ok == Some(true)
                                    && matches!(
                                        name.as_str(),
                                        "settings_update" | "settings_reset"
                                    );
                                match st
                                    .tools
                                    .iter_mut()
                                    .rev()
                                    .find(|(n, o, _)| *n == name && o.is_none())
                                {
                                    Some(slot) if ok.is_some() => {
                                        slot.1 = ok;
                                        if summary.is_some() {
                                            slot.2 = summary;
                                        }
                                    }
                                    _ => st.tools.push((name, ok, summary)),
                                }
                            }
                            RunEvent::Trace(step) => {
                                match &step {
                                    mochi_core::ai::agent_runner::AgentTraceStep::Plan {
                                        steps,
                                        ..
                                    } => {
                                        st.plan = assistant::plan_lines(steps);
                                        st.trace.retain(|step| !matches!(step, mochi_core::ai::agent_runner::AgentTraceStep::Plan { .. }));
                                    }
                                    mochi_core::ai::agent_runner::AgentTraceStep::Thinking {
                                        ..
                                    } => st.reasoning.clear(),
                                    _ => {}
                                }
                                st.trace.push(step);
                            }
                            RunEvent::Done { .. } | RunEvent::Failed(_) => unreachable!(),
                        }
                    }
                }
            }
        }
        if schedule_changed {
            self.reload_schedule();
        }
        if settings_changed {
            self.load_chrome_settings();
            self.shell.apply_git_settings();
            self.prefs.providers.loaded = false;
            self.editor_ai.invalidate();
            self.memory_cancel_if_disabled();
        }
        if let Some(ev) = done {
            self.ai.run = None;
            match ev {
                RunEvent::Done {
                    content,
                    transcript,
                    trace,
                    finish_reason,
                    usage,
                    usage_source,
                } => {
                    let extraction_text = (finish_reason != "error").then(|| content.clone());
                    self.publish_notification(
                        crate::ui::notifications::Category::Assistant,
                        if finish_reason == "error" {
                            "AI 回复失败"
                        } else {
                            "AI 回复完成"
                        },
                        if finish_reason == "error" {
                            &content
                        } else {
                            "本次回复已完成，可在 AI 助手中查看。"
                        },
                    );
                    if finish_reason == "error" {
                        self.ai.panel.error = content.clone();
                    }
                    let pending_edits = pending_edits_from_transcript(&transcript);
                    // 工具往返存成隐藏消息，续聊时模型才看得到自己干过什么
                    let hidden: Vec<AiStoredMessage> = transcript
                        .iter()
                        .filter(|m| {
                            m.role == "tool"
                                || (m.role == "assistant"
                                    && m.tool_calls
                                        .as_ref()
                                        .map(|t| !t.is_empty())
                                        .unwrap_or(false))
                        })
                        .map(|m| {
                            let mut s = AiStoredMessage::new(
                                &m.role,
                                m.content.as_deref().unwrap_or_default(),
                            );
                            s.set("hidden", serde_json::json!(true));
                            if let Some(calls) = &m.tool_calls {
                                s.set("tool_calls", serde_json::json!(calls));
                            }
                            if let Some(name) = &m.name {
                                s.set("name", serde_json::json!(name));
                            }
                            if let Some(id) = &m.tool_call_id {
                                s.set("tool_call_id", serde_json::json!(id));
                            }
                            s
                        })
                        .collect();
                    if let Some(c) = self.ai.panel.active.as_mut() {
                        c.messages.extend(hidden);
                    }
                    let summary = assistant::trace_summary(&trace);
                    if !summary.is_empty() {
                        if let Some(c) = self.ai.panel.active.as_mut() {
                            let mut s = AiStoredMessage::new("assistant", &summary);
                            s.set("hidden", serde_json::json!(true));
                            s.set("kind", serde_json::json!("trace"));
                            c.messages.push(s);
                        }
                    }
                    if let Some(st) = self.ai.panel.streaming.as_mut() {
                        st.trace = trace.clone();
                        st.reasoning.clear();
                    }
                    let reply_index = self
                        .ai
                        .panel
                        .active
                        .as_ref()
                        .map_or(0, |c| c.messages.len());
                    self.ai_finish_streaming(Some(content.clone()));
                    if let Some(message) = self
                        .ai
                        .panel
                        .active
                        .as_mut()
                        .and_then(|c| c.messages.get_mut(reply_index))
                    {
                        if message.role() == "assistant" && !message.is_hidden() {
                            message.set("usage", usage);
                            message.set("usageSource", serde_json::json!(usage_source));
                            self.ai_persist_active();
                        }
                    }
                    if finish_reason != "error" {
                        self.start_title_job(reply_index);
                        self.start_follow_up_job(reply_index, content);
                    }
                    // 每个局部编辑都保留为独立的可见消息，和 Electron 的
                    // `deferredPendingEdits` 一样；它们仍指向收件箱中的同一条
                    // PendingFileOperation，批准时复用既有的安全预览/冲突校验。
                    let had_pending_edits = !pending_edits.is_empty();
                    let now = mochi_core::jstime::now_millis();
                    if let Some(c) = self.ai.panel.active.as_mut() {
                        for (index, edit) in pending_edits.iter().cloned().enumerate() {
                            let summary = edit["summary"].as_str().unwrap_or("局部修改建议");
                            let mut message =
                                AiStoredMessage::new("assistant", &format!("建议修改：{summary}"));
                            message.set(
                                "id",
                                serde_json::json!(format!("msg-{now}-pending-{index}")),
                            );
                            message.set("timestamp", serde_json::json!(now + index as i64));
                            message.set("pendingEdit", edit);
                            c.messages.push(message);
                        }
                    }
                    if had_pending_edits {
                        self.ai_persist_active();
                        if crate::ui::settings_values::text("ai.editApplyMode", "approve") == "auto"
                        {
                            for edit in &pending_edits {
                                if let Some(inbox_id) = edit["inboxId"].as_str() {
                                    self.ai_auto_apply_pending_edit(inbox_id);
                                }
                            }
                        }
                    }
                    self.ai_append_pending_cards(&transcript);
                    if let Some(content) = extraction_text {
                        self.start_memory_job(content);
                    } else {
                        self.ai.pending_memory = None;
                    }
                }
                RunEvent::Failed(err) => {
                    self.publish_notification(
                        crate::ui::notifications::Category::Assistant,
                        "AI 请求失败",
                        &err,
                    );
                    self.ai.pending_memory = None;
                    self.ai.panel.error = err;
                    self.ai_finish_streaming(None);
                }
                _ => {}
            }
        }
        if self.ai.panel.stick_to_bottom {
            self.ai.panel.scroll = f32::MAX; // 绘制前会夹到 max_scroll
        }
    }
}
