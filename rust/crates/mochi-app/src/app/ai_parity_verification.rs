//! 使用本地合成对话进行真实 D2D 检查；不会连接模型服务。
use super::*;
use anyhow::{ensure, Context};
use mochi_core::ai::agent_runner::AgentTraceStep;

impl App {
    pub(super) fn verify_ai_parity(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
        narrow: bool,
    ) -> anyhow::Result<()> {
        let workspace_root = self
            .shell
            .workspace()
            .context("snapshot workspace")?
            .root
            .clone();
        mochi_core::ai::agent_config::AgentConfigService::new(&workspace_root).ensure_seeds()?;
        let file = folder.join("研究笔记.md");
        std::fs::write(&file, "# 研究笔记\n\n将选中文本发送给 AI，可以保留文档位置和行号。\n\n## 需要完成\n\n- 核对知识库材料\n- 整理实现方案\n")?;
        self.shell.open_file(&file);
        self.sync_state();
        let now = Self::now_ms();
        let message = |id: &str, role: &str, text: &str| {
            let mut m = AiStoredMessage::new(role, text);
            m.set("id", serde_json::json!(id));
            m.set("timestamp", serde_json::json!(now));
            m
        };
        let plan = AgentTraceStep::Plan {
            steps: vec![
                serde_json::json!({"title":"阅读知识库材料","status":"done"}),
                serde_json::json!({"title":"对照版本差异","status":"done"}),
                serde_json::json!({"title":"整理实现与验证清单","status":"done"}),
            ],
            note: Some("本地界面验收示例".into()),
        };
        let thinking = AgentTraceStep::Thinking { text: "先检查已有材料中的页面结构，再按消息渲染、上下文和操作反馈逐项核对。对于代码与数学公式，保留原文并分别提供复制入口。".repeat(4), duration_ms: Some(2640) };
        let tool = AgentTraceStep::Tool {
            name: "file_read".into(),
            summary: Some("研究笔记.md · 已读取 24 行".into()),
            ok: true,
            duration_ms: 42,
            result_preview: None,
        };
        let failed = AgentTraceStep::Tool {
            name: "mcp__docs__search".into(),
            summary: Some("补充资料检索".into()),
            ok: false,
            duration_ms: 1032,
            result_preview: Some("连接暂时不可用。已继续使用当前知识库资料。".into()),
        };
        let mut answer = message("answer", "assistant", "## 已完成的整理\n\n这里保留自然的段落节奏，支持 **加粗**、*强调*、~~删除线~~、`行内代码` 和 [来源文档](研究笔记.md)。\n\n> 先确认信息来源，再给出清晰的执行步骤。\n\n- [x] 阅读材料\n- [x] 检查渲染\n- [ ] 完成最终复核\n\n```rust\nfn favorite_documents() -> Vec<String> {\n    let message = \"收藏默认仅显示文档；开启设置后显示父目录\";\n    println!(\"{message}\");\n    vec![\"知识库/长目录/跨版本对齐/用于检验横向滚动的很长文件名称与更多描述信息.md\".into()]\n}\n```\n\n| 功能 | 展示方式 | 操作 |\n| :--- | :--- | ---: |\n| 思考过程 | 默认收起，可展开 | 回看 |\n| 代码与表格 | 语法高亮、横向滚动 | 复制 |\n| 长文本 | 保持自然段落间距 | 阅读 |\n\n公式使用原生渲染：$E = mc^2$。\n\n$$\n\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}\n$$\n\n可以继续提出问题，也可以将这轮问答挂载到笔记。\n");
        answer.set(
            "trace",
            serde_json::to_value(vec![
                plan.clone(),
                thinking.clone(),
                tool.clone(),
                failed.clone(),
            ])?,
        );
        answer.set("model", serde_json::json!("示例模型"));
        answer.set(
            "usage",
            serde_json::json!({"promptTokens":720,"completionTokens":480,"totalTokens":1200}),
        );
        answer.set("usageSource", serde_json::json!("provider"));
        self.ai.panel.active = Some(AiConversation {
            id: "ai-parity-session".into(),
            title: "知识库整理与界面对齐".into(),
            created_at: now,
            updated_at: now,
            messages: vec![
                message(
                    "question",
                    "user",
                    "请阅读材料，整理一份清晰的实现与验证清单。",
                ),
                answer,
            ],
        });
        self.ai_persist_active();
        self.ai.panel.agent_name = "通用助手".into();
        self.ai.panel.loaded_skills = vec!["文档整理".into(), "知识库检索".into()];
        self.ai.panel.provider_missing = false;
        self.ai.panel.scroll = 0.0;
        self.ai.panel.stick_to_bottom = false;
        self.state.view = if narrow {
            WorkspaceView::Editor
        } else {
            WorkspaceView::MochiAi
        };
        self.state.ai_panel_open = narrow;
        self.state.right_panel = RightPanel::Assistant;
        self.state.ai_panel_width = 440.0;
        if narrow {
            self.state.sidebar_visible = false;
            self.state.navigation_collapsed = true;
        }
        self.paint(HWND::default())?;
        ensure!(
            self.ai.layout.messages.len() == 2,
            "historical message layout missing"
        );
        ensure!(self.ai.layout.messages[1].trace.len() == 4);
        ensure!(
            !self.ai.layout.messages[1].trace_expanded,
            "history should start collapsed"
        );
        self.verify_frame(output, "history", snapshot)?;
        let agent_control = self
            .ai
            .layout
            .rect_of(assistant::Hit::AgentPicker)
            .context("agent picker")?;
        self.on_assistant_click(
            HWND::default(),
            (agent_control.left + agent_control.right) * 0.5,
            (agent_control.top + agent_control.bottom) * 0.5,
        );
        let menu = self.menu.as_ref().context("agent menu")?;
        ensure!(
            menu.rect.bottom <= agent_control.top || menu.rect.top >= agent_control.bottom,
            "agent dropdown overlaps its trigger"
        );
        self.verify_frame(output, "agent-menu", snapshot)?;
        self.menu = None;
        let toggle = self
            .ai
            .layout
            .rect_of(assistant::Hit::TraceToggle(1))
            .context("trace toggle")?;
        self.on_assistant_click(
            HWND::default(),
            (toggle.left + toggle.right) * 0.5,
            (toggle.top + toggle.bottom) * 0.5,
        );
        self.paint(HWND::default())?;
        ensure!(
            self.ai.panel.expanded_traces.contains(&1),
            "trace toggle click not connected"
        );
        ensure!(self.ai.layout.messages[1].trace_height > 100.0);
        self.verify_frame(output, "trace", snapshot)?;
        self.ai.panel.expanded_traces.clear();
        self.paint(HWND::default())?;
        let mut kinds = Vec::new();
        for index in 0..20 {
            let Some(payload) = self.ai.layout.content_copy(&self.ai.panel, 1, index) else {
                break;
            };
            ensure!(!payload.text.is_empty());
            kinds.push(payload.kind);
            ensure!(self.ai_content_copy_with(1, index, |_| true));
        }
        ensure!(
            kinds.contains(&crate::ui::ai_markdown::CopyKind::Code),
            "code copy missing"
        );
        ensure!(
            kinds.contains(&crate::ui::ai_markdown::CopyKind::Table),
            "table copy missing"
        );
        ensure!(
            kinds.contains(&crate::ui::ai_markdown::CopyKind::Formula),
            "formula copy missing"
        );
        self.status_bar.toast = Default::default();
        self.status_bar.timer = None;
        self.state.status_text.clear();
        self.ai.panel.scroll =
            (self.ai.layout.messages[1].top + 330.0).min(self.ai.layout.max_scroll());
        self.paint(HWND::default())?;
        self.verify_frame(output, "markdown", snapshot)?;
        self.ai_toggle_search();
        self.ai.panel.search_query.set_text("阅读");
        self.ai_search_changed();
        self.paint(HWND::default())?;
        ensure!(self.ai.panel.search_result_index == 0);
        self.ai_search_next(1);
        ensure!(self.ai.panel.search_result_index == 1);
        self.paint(HWND::default())?;
        self.verify_frame(output, "search", snapshot)?;
        self.ai_close_search();
        self.ai.panel.streaming = Some(assistant::Streaming {
            model: "示例模型".into(),
            status: "正在整理结果…".into(),
            reasoning: "正在核对剩余条目，并将验证结果整理为可执行清单。".repeat(12),
            content: "已经完成材料核对，正在整理最后的说明。".into(),
            trace: vec![
                AgentTraceStep::Plan {
                    steps: vec![
                        serde_json::json!({"title":"阅读知识库材料","status":"done"}),
                        serde_json::json!({"title":"检查文档结构","status":"done"}),
                        serde_json::json!({"title":"汇总验证结果","status":"in_progress"}),
                    ],
                    note: None,
                },
                thinking,
                tool,
            ],
            tools: vec![
                ("file_read".into(), Some(true), Some("已读取材料".into())),
                ("search_documents".into(), None, Some("查找关联笔记".into())),
            ],
            ..Default::default()
        });
        self.ai.panel.stick_to_bottom = true;
        self.ai.panel.scroll = f32::MAX;
        self.paint(HWND::default())?;
        ensure!(self.ai.layout.messages.last().unwrap().streaming);
        self.verify_frame(output, "streaming", snapshot)?;
        self.ai.panel.stick_to_bottom = false;
        self.ai.panel.scroll = 0.0;
        self.paint(HWND::default())?;
        ensure!(
            self.ai
                .layout
                .rect_of(assistant::Hit::ScrollToBottom)
                .is_some(),
            "return to latest missing"
        );
        self.ai.panel.streaming = None;
        self.ai.panel.scroll = 0.0;
        self.ai
            .panel
            .input
            .set_text("继续整理，并在最终清单中保留来源链接。\n请同时列出需要复核的条目。");
        self.ai.panel.pending_files = vec![file.clone()];
        self.focus = Focus::AiInput;
        self.paint(HWND::default())?;
        self.verify_frame(output, "composer", snapshot)?;
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(0, false);
            buffer.set_cursor(14.min(buffer.text().len()), true);
        }
        let selection = self
            .ai_current_selection()
            .context("selection context snapshot")?;
        let draft = self.ai.panel.input.text().to_owned();
        self.run_menu_action(MenuAction::SelectionToAssistant);
        ensure!(
            self.ai.panel.pending_selections.len() == 1 && self.ai.panel.input.text() == draft,
            "selection attachment replaced the user's draft"
        );
        self.state.view = if narrow {
            WorkspaceView::Editor
        } else {
            WorkspaceView::MochiAi
        };
        self.state.ai_panel_open = narrow;
        self.paint(HWND::default())?;
        self.verify_frame(output, "selection", snapshot)?;
        let mut selected_message = message("selection-user", "user", "请整理这部分内容。");
        Self::ai_attach_selection_contexts(&mut selected_message, &[selection.clone()]);
        self.ai
            .panel
            .active
            .as_mut()
            .unwrap()
            .messages
            .push(selected_message);
        self.ai.panel.pending_selections.clear();
        self.ai.panel.pending_files.clear();
        self.ai.panel.input.clear();
        let root = self.shell.workspace().unwrap().root.clone();
        let entry = AgentInboxService::new(&root).add(serde_json::json!({"id":"verification-op","kind":"overwrite","path":file,"title":"整理研究笔记","summary":"调整第二段说明","content":"# 修订示例\n\n只供审批预览，不会写入原文。","previousContent":std::fs::read_to_string(&file)?,"status":"pending"}),None,Some("界面验收"))?;
        let transcript = vec![AiMessage::tool_result("verification-call","file_write",&serde_json::json!({"ok":true,"data":{"status":"pending","inboxId":entry.id(),"operationId":"verification-op"}}).to_string())];
        ensure!(
            self.ai_append_pending_cards(&transcript) == 1,
            "file proposal card missing"
        );
        ensure!(
            self.ai_append_pending_cards(&transcript) == 0,
            "file proposal card duplicated"
        );
        self.ai.panel.scroll = f32::MAX;
        self.paint(HWND::default())?;
        self.verify_frame(output, "approval", snapshot)?;
        let review_index = self.ai.panel.visible_messages().len() - 1;
        let original = std::fs::read_to_string(&file)?;
        let review = self
            .ai
            .layout
            .rect_of(assistant::Hit::PendingCardReview(review_index))
            .context("file review button")?;
        self.on_assistant_click(
            HWND::default(),
            (review.left + review.right) * 0.5,
            (review.top + review.bottom) * 0.5,
        );
        ensure!(
            self.commands.review.is_some(),
            "file card click did not open native review after scrolling"
        );
        ensure!(
            std::fs::read_to_string(&file)? == original,
            "opening proposal review changed the source file"
        );
        self.commands.review = None;
        self.ai_scroll_to_message(review_index - 1);
        self.paint(HWND::default())?;
        let locate = self
            .ai
            .layout
            .rect_of(assistant::Hit::SelectionLocate(review_index - 1, 0))
            .context("selection chip")?;
        self.on_assistant_click(
            HWND::default(),
            (locate.left + locate.right) * 0.5,
            (locate.top + locate.bottom) * 0.5,
        );
        ensure!(
            self.shell
                .active()
                .and_then(|t| t.buffer())
                .is_some_and(|b| b.has_selection()),
            "selection chip did not select source lines"
        );
        self.state.ai_panel_open = true;
        self.set_right_panel(RightPanel::AgentInbox);
        self.paint(HWND::default())?;
        let inbox_index = self
            .panels
            .inbox
            .entries
            .iter()
            .position(|item| item.id() == entry.id())
            .context("pending file inbox entry")?;
        let reject = self
            .panels
            .inbox_layout
            .rect_of(inbox::Hit::Reject(inbox_index))
            .context("inbox reject button")?;
        self.on_right_panel_click(
            (reject.left + reject.right) * 0.5,
            (reject.top + reject.bottom) * 0.5,
        );
        ensure!(
            self.ai
                .panel
                .active
                .as_ref()
                .unwrap()
                .messages
                .last()
                .unwrap()
                .get("pendingFileOperation")
                .unwrap()["status"]
                == "rejected",
            "rejecting from inbox left the conversation card pending"
        );
        ensure!(
            std::fs::read_to_string(&file)? == original,
            "rejecting a proposal changed the source file"
        );
        self.set_right_panel(RightPanel::Assistant);
        self.state.status_text.clear();
        self.state.view = if narrow {
            WorkspaceView::Editor
        } else {
            WorkspaceView::MochiAi
        };
        self.state.ai_panel_open = narrow;
        self.ai.panel.scroll = 0.0;
        self.paint(HWND::default())?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":true,"realD2D":true,"isolatedWorkspace":true,"providerContacted":false,"historyTraceToggle":true,"codeTableFormulaCopy":true,"messageSearch":true,"streamingTimeline":true,"returnToLatest":true,"multilineComposer":true,"selectionContextAndLocate":true,"fileProposalReview":true,"inboxRejectionUpdatesCard":true,"narrowPanel":narrow}),
            )?,
        )?;
        self.renderer.save_snapshot(snapshot, output)?;
        Ok(())
    }
}
