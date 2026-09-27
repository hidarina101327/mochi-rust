//! 通过真实 App 处理器和 D2D 验证应用流程，仅使用快照中的
//! 测试数据。这不模拟前台 IME 或全局快捷键的测试。
use super::*;
use anyhow::{ensure, Context};
impl App {
    pub(super) fn verify_blank_lines(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        let mut results = Vec::new();
        for (name, seed, newline) in [
            ("empty", "", "\n"),
            ("text", "第一行", "\n"),
            ("crlf", "前文\r\n第一行", "\r\n"),
        ] {
            let path = folder.join(format!("连续换行验收-{name}.md"));
            std::fs::write(&path, seed)?;
            ensure!(self.shell.open_file(&path));
            self.sync_state();
            self.paint(HWND::default())?;
            self.focus = Focus::Main;
            self.editor_engaged = true;
            self.shell
                .active_buffer_mut()
                .unwrap()
                .set_cursor(seed.len(), false);
            self.after_doc_edit(true);
            let caret_top = |app: &Self| -> anyhow::Result<f32> {
                Ok(app
                    .doc
                    .caret_rect(
                        app.editor_area,
                        app.shell.active().unwrap().buffer().unwrap(),
                        0.0,
                    )
                    .context("blank-line caret missing")?
                    .top)
            };
            let mut tops = vec![caret_top(self)?];
            for count in 1..=4 {
                ensure!(self.on_edit_key(0x0d, false, false));
                let expected = format!("{seed}{}", newline.repeat(count));
                ensure!(self.shell.active().unwrap().buffer().unwrap().text() == expected);
                self.paint(HWND::default())?;
                let top = caret_top(self)?;
                ensure!(
                    top >= tops.last().unwrap()
                        + crate::ui::draw::TextStyle::Document.line_height()
                        - 0.1,
                    "{name}: Enter {count} did not advance a full visual line"
                );
                tops.push(top);
                if count == 2 {
                    self.verify_frame(output, &format!("{name}-second-enter"), snapshot)?;
                }
            }
            // 按上、下键应能依次到达新插入的每一行空行。
            for top in tops[..4].iter().rev() {
                ensure!(self.on_edit_key(0x26, false, false));
                ensure!((caret_top(self)? - top).abs() < 0.1);
            }
            for top in tops.iter().skip(1).take(4) {
                ensure!(self.on_edit_key(0x28, false, false));
                ensure!((caret_top(self)? - top).abs() < 0.1);
            }
            // 每次按 Enter 都是一个撤销步骤，且可以重做。
            for remaining in (0..4).rev() {
                ensure!(self.on_shortcut(HWND::default(), 0x5a, false, true));
                ensure!(
                    self.shell.active().unwrap().buffer().unwrap().text()
                        == format!("{seed}{}", newline.repeat(remaining))
                );
            }
            for _ in 0..4 {
                ensure!(self.on_shortcut(HWND::default(), 0x5a, true, true));
            }
            self.on_ime_commit("后文");
            let expected = format!("{seed}{}后文", newline.repeat(4));
            ensure!(self.shell.active().unwrap().buffer().unwrap().text() == expected);
            let typed_top = caret_top(self)?;
            ensure!(
                typed_top >= tops[4] - 0.1,
                "typing collapsed preceding blank lines"
            );
            self.verify_frame(output, &format!("{name}-typed"), snapshot)?;
            ensure!(self.on_edit_key(0x41, false, true));
            let mut copied = String::new();
            self.clipboard_copy_with(false, |text| {
                copied = text.into();
                true
            });
            ensure!(copied == expected, "copy collapsed blank lines");
            self.shell
                .active_buffer_mut()
                .unwrap()
                .set_cursor(expected.len(), false);
            self.toggle_source_mode();
            self.toggle_source_mode();
            ensure!(self.save_active());
            ensure!(std::fs::read_to_string(&path)? == expected);
            self.shell.close_tab(self.shell.active_tab().unwrap());
            ensure!(self.shell.open_file(&path));
            self.sync_state();
            self.shell
                .active_buffer_mut()
                .unwrap()
                .set_cursor(expected.len(), false);
            self.after_doc_edit(true);
            ensure!(self.shell.active().unwrap().buffer().unwrap().text() == expected);
            ensure!(
                (caret_top(self)? - typed_top).abs() < 0.1,
                "reopening changed blank-line geometry"
            );
            self.verify_frame(output, &format!("{name}-reopened"), snapshot)?;
            results.push(serde_json::json!({"case":name,"caretTops":tops,"reopenedCaretTop":caret_top(self)?,"passed":true}));
        }
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({"passed":true,"cases":results}))?,
        )?;
        Ok(())
    }

    pub(super) fn verify_wysiwyg(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        let path = folder.join("所见即所得编辑验收.md");
        let raw="---\ntitle: 元数据保持原样\n---\n# 在墨池，直接书写\n\n这是一段 **可直接编辑的粗体**，以及 *斜体*、<u>下划线</u>和 <mark>重点高亮</mark>。\n\n## 保留熟悉的文档效果\n\n> 点击正文即可编辑，标题、引用和列表保持原来的样式。\n\n- [x] 中文与 emoji 😀\n- [ ] 列表回车续项与撤销\n\n```rust\nfn main() {\n    println!(\"你好，墨池\");\n}\n```\n\n| 内容 | 状态 |\n| --- | --- |\n| 富文本编辑 | 已接入 |\n\n公式 $\\frac{1}{2}+x^2$ 和 [文档链接](note.md)。\n";
        std::fs::write(&path, raw)?;
        ensure!(self.shell.open_file(&path));
        self.sync_state();
        self.paint(HWND::default())?;
        self.verify_frame(output, "initial", snapshot)?;
        let at = raw.find("直接编辑").unwrap();
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(at, false);
        self.after_doc_edit(true);
        let before = self
            .doc
            .caret_rect(
                self.editor_area,
                self.shell.active().unwrap().buffer().unwrap(),
                0.0,
            )
            .context("rich caret missing")?;
        self.on_click(before.left, before.top + before.height() / 2.0);
        ensure!(self.editor_engaged, "click did not engage editor");
        ensure!(
            self.shell.active().unwrap().buffer().unwrap().cursor() == at,
            "click landed in hidden syntax"
        );
        self.paint(HWND::default())?;
        let after = self
            .doc
            .caret_rect(
                self.editor_area,
                self.shell.active().unwrap().buffer().unwrap(),
                0.0,
            )
            .context("focused caret missing")?;
        ensure!(
            (before.left - after.left).abs() < 0.1 && (before.top - after.top).abs() < 0.1,
            "focusing changed layout"
        );
        for _ in 0..4 {
            ensure!(self.on_edit_key(0x27, true, false));
        }
        let mut copied = String::new();
        self.clipboard_copy_with(false, |s| {
            copied = s.into();
            true
        });
        ensure!(copied == "直接编辑", "rich copy leaked syntax: {copied}");
        self.verify_frame(output, "selection", snapshot)?;
        self.on_char('新');
        ensure!(
            self.shell
                .active()
                .unwrap()
                .buffer()
                .unwrap()
                .text()
                .contains("**可新的粗体**"),
            "selected replacement damaged bold"
        );
        self.on_shortcut(HWND::default(), 0x5a, false, true);
        ensure!(
            self.shell.active().unwrap().buffer().unwrap().text() == raw,
            "undo changed original Markdown"
        );
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(at, false);
        self.on_ime_composition("zhongwen", 8);
        self.verify_frame(output, "composition", snapshot)?;
        ensure!(
            self.shell.active().unwrap().buffer().unwrap().text() == raw,
            "uncommitted input changed buffer"
        );
        ensure!(
            std::fs::read_to_string(&path)? == raw,
            "uncommitted input changed file"
        );
        self.on_ime_commit("中文😀");
        let expected = raw.replacen("直接编辑", "中文😀直接编辑", 1);
        ensure!(
            self.shell.active().unwrap().buffer().unwrap().text() == expected,
            "IME inserted at wrong source position"
        );
        ensure!(self.save_active());
        ensure!(std::fs::read_to_string(&path)? == expected);
        self.verify_frame(output, "edited", snapshot)?;
        self.toggle_source_mode();
        ensure!(self.shell.active().unwrap().source_mode());
        self.toggle_source_mode();
        ensure!(!self.shell.active().unwrap().source_mode());
        ensure!(
            self.shell.active().unwrap().buffer().unwrap().text() == expected,
            "mode switch rewrote Markdown"
        );
        self.verify_frame(output, "roundtrip", snapshot)?;
        // 编辑代码块正文时，卡片和光标位置都应保持不变。
        let code = self
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .find("println!")
            .unwrap();
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(code, false);
        self.after_doc_edit(true);
        self.on_char(' ');
        self.verify_frame(output, "code", snapshot)?;
        ensure!(self
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("```rust\nfn main() {\n     println!"));
        self.on_shortcut(HWND::default(), 0x5a, false, true);
        // 富文本表格单元格使用相同的映射方式，并将编辑内容一次提交。
        self.shell.active_mut().unwrap().scroll = 0.0;
        self.paint(HWND::default())?;
        let cells = self.doc.table_cells(
            self.editor_area,
            self.shell.active().unwrap().buffer().unwrap().text(),
            0.0,
        );
        let cell = cells.get(2).context("table cell missing")?;
        self.shell.active_mut().unwrap().scroll =
            (cell.rect.top - self.editor_area.top - 130.0).max(0.0);
        self.paint(HWND::default())?;
        let cells = self.doc.table_cells(
            self.editor_area,
            self.shell.active().unwrap().buffer().unwrap().text(),
            self.shell.active_scroll(),
        );
        let cell = cells[2].clone();
        self.begin_table_cell(cell.clone(), cell.rect.left + 14.0);
        self.table_editing
            .as_mut()
            .unwrap()
            .field
            .buffer
            .select_all();
        ensure!(self.on_shortcut(HWND::default(), 0x42, false, true));
        self.verify_frame(output, "table", snapshot)?;
        ensure!(self.commit_table_cell());
        ensure!(self
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("| **富文本编辑** | 已接入 |"));
        self.paint(HWND::default())?;
        let formula = self
            .list
            .cmds()
            .iter()
            .find_map(|cmd| match cmd {
                crate::ui::draw::DrawCmd::Math { rect, tex, .. } if tex.contains("frac") => {
                    Some(*rect)
                }
                _ => None,
            })
            .context("formula not rendered")?;
        self.on_click(
            (formula.left + formula.right) / 2.0,
            (formula.top + formula.bottom) / 2.0,
        );
        ensure!(
            self.dialog.as_ref().is_some_and(|d| d.title == "编辑公式"),
            "formula click did not open its editor"
        );
        self.verify_frame(output, "formula", snapshot)?;
        let action = self
            .dialog
            .as_ref()
            .unwrap()
            .buttons
            .last()
            .unwrap()
            .action
            .clone();
        self.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("x^2+1");
        self.run_dialog_action(action);
        ensure!(self
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("公式 $x^2+1$"));
        ensure!(self.save_active());
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"passed":true,"stableFocusLayout":true,"copy":copied,"imeAndSavePassed":true,"modeRoundtripPassed":true,"workspace":folder,"dpi":std::env::var("MOCHI_SNAPSHOT_DPI").unwrap_or("144".into())}),
            )?,
        )?;
        Ok(())
    }

    pub(super) fn verify_workflow(
        &mut self,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        let path = self
            .active_file_path()
            .context("verification document missing")?;
        let original = std::fs::read_to_string(&path)?;
        let root = self.shell.workspace().unwrap().root.clone();
        self.setup_ai(HWND::default(), &root);
        self.focus = Focus::Main;
        self.editor_engaged = true;
        let buffer = self.shell.active_buffer_mut().unwrap();
        buffer.set_cursor(buffer.text().len(), false);
        self.on_ime_composition("输入验证", 4);
        ensure!(
            std::fs::read_to_string(&path)? == original,
            "composition wrote to disk"
        );
        self.on_ime_commit("隔离输入验证🙂");
        ensure!(self.save_active(), "save failed");
        ensure!(
            std::fs::read_to_string(&path)? == format!("{original}隔离输入验证🙂"),
            "source changed unexpectedly"
        );
        self.on_shortcut(HWND::default(), 0x5a, false, true);
        ensure!(
            self.shell.active_buffer_mut().unwrap().text() == original,
            "undo did not restore exact source"
        );
        ensure!(self.save_active(), "undo save failed");
        self.editor_engaged = false;
        self.verify_frame(output, "document", snapshot)?;
        self.verify_export_options(output, snapshot, &path)?;
        self.verify_ai_locator(output, snapshot, &path)?;

        let link = path.with_file_name("关联笔记.md");
        std::fs::write(
            &link,
            "# 关联笔记\n\n[[墨池原生重写]]\n\n中文检索验收 unique_probe\n",
        )?;
        let index = self.shell.workspace().unwrap().index.clone();
        index.index_files([&path, &link])?;
        self.on_files_changed();
        ensure!(index.get_backlinks(&path)?.len() == 1, "backlink missing");
        self.on_shortcut(HWND::default(), 0x46, true, true);
        ensure!(self.search.is_some(), "global search shortcut failed");
        for ch in "unique_probe".chars() {
            self.on_char(ch);
        }
        self.on_timer(HWND::default(), platform::TIMER_SEARCH);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.search.as_ref().is_some_and(|s| s.result.is_none())
            && std::time::Instant::now() < deadline
        {
            self.take_search_result();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let result = self
            .search
            .as_ref()
            .and_then(|s| s.result.as_ref())
            .context("search timed out")?;
        ensure!(
            result.stats.total_files == 1 && result.groups[0].path.ends_with("关联笔记.md"),
            "search returned wrong file"
        );
        self.verify_frame(output, "search", snapshot)?;
        self.close_search();

        self.verify_nav(NavHit::Item(NavItem::Schedule))?;
        self.reload_schedule();
        let (project, _) = self
            .agenda_store()
            .context("schedule unavailable")?
            .mutate(mochi_core::agenda::Source::User, |ed| {
                ed.create_project(mochi_core::agenda::ProjectDraft {
                    name: "验收项目".into(),
                    ..Default::default()
                })
            })?;
        self.reload_schedule();
        self.agenda_new(mochi_core::agenda::Kind::Task, None, None);
        {
            let panel = self.sched.view.panel.as_mut().context("panel missing")?;
            panel
                .field_mut(crate::ui::agenda::panel::Field::Title)
                .unwrap()
                .set_text("原生对照评审");
            panel
                .field_mut(crate::ui::agenda::panel::Field::Note)
                .unwrap()
                .set_text("第一行：检查重构\n第二行：保留中文与换行");
            panel.picks.project = Some(project);
            panel.focus = None;
        }
        self.focus = Focus::Main;
        self.verify_frame(output, "schedule-form", snapshot)?;
        let rect = self
            .sched
            .layout
            .rect_of(&crate::ui::agenda::Hit::Panel(
                crate::ui::agenda::panel::Hit::Create,
            ))
            .context("schedule create button missing")?;
        self.on_schedule_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        ensure!(
            self.sched.view.panel.as_ref().is_some_and(|p| !p.is_new()),
            "schedule create did not complete"
        );
        let data = self
            .agenda_store()
            .context("schedule unavailable")?
            .load()?;
        ensure!(
            data.tasks
                .iter()
                .any(|t| t.title == "原生对照评审" && t.note.contains('\n')),
            "multiline schedule task missing"
        );
        self.verify_frame(output, "schedule", snapshot)?;
        self.verify_nav(NavHit::Item(NavItem::MochiAi))?;
        self.ai_select_agent("通用助手");
        ensure!(
            self.ai.panel.selected_agent_id.as_deref() == Some("通用助手"),
            "Agent selection failed"
        );
        self.verify_frame(output, "ai", snapshot)?;
        let picker = self
            .ai
            .layout
            .rect_of(assistant::Hit::AgentPicker)
            .context("Agent picker missing")?;
        self.on_assistant_click(
            HWND::default(),
            (picker.left + picker.right) / 2.0,
            (picker.top + picker.bottom) / 2.0,
        );
        ensure!(self.menu.is_some(), "Agent menu did not open");
        self.verify_frame(output, "ai-agents", snapshot)?;
        self.menu = None;
        self.ai_set_apply_mode(true);
        ensure!(self.dialog.is_some(), "auto-edit mode skipped confirmation");
        self.verify_frame(output, "ai-auto-confirm", snapshot)?;
        self.run_dialog_action(DialogAction::Dismiss);
        self.ai_new_session();
        self.ai
            .panel
            .active
            .as_mut()
            .unwrap()
            .messages
            .push(AiStoredMessage::new(
                "user",
                "查看已保存的会话，未配置模型时仍应可读。",
            ));
        let mut reply = AiStoredMessage::new("assistant", "以下是隔离验收的计划展示示例。");
        reply.set("trace",serde_json::json!([{"kind":"plan","steps":[{"title":"核对原文","status":"done"},{"title":"完成验收","status":"in_progress"}]}]));
        self.ai.panel.active.as_mut().unwrap().messages.push(reply);
        self.ai_persist_active();
        self.ai_mount_session(&path);
        ensure!(
            AiDocumentMountService::new(&root)
                .mounts_for_document(&path.to_string_lossy())
                .len()
                == 1,
            "session mount missing"
        );
        ensure!(
            std::fs::read_to_string(&path)? == original,
            "mount modified document"
        );
        self.verify_frame(output, "ai-offline-history", snapshot)?;
        self.verify_nav(NavHit::Settings)?;
        self.open_settings("shortcuts");
        self.verify_frame(output, "shortcuts", snapshot)?;
        self.state.dark = true;
        self.state.view = WorkspaceView::Editor;
        self.shell.open_file(&path);
        self.invalidate_main();
        self.sync_state();
        self.verify_frame(output, "dark", snapshot)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed": true, "workspace": self.shell.workspace().unwrap().root,
                "checks": ["App composition commit and exact save", "undo preserves Markdown", "PDF export with AI questions/answers and comments", "Markdown export without comments or implicit conversation reads", "native AI locator card click, target highlight and expiry", "backlink", "shortcut to async global search", "navigation hit testing", "schedule task/project and multiline notes", "Agent selection/menu", "auto-mode confirmation", "session mount without document rewrite", "offline history and plan display", "AI/settings navigation", "D2D light/dark rendering"],
                "notCovered": ["physical keyboard input", "foreground IME candidate window", "OS global shortcut registration"]
            }))?,
        )?;
        Ok(())
    }
    fn verify_nav(&mut self, hit: NavHit) -> anyhow::Result<()> {
        self.paint(HWND::default())?;
        let r = self
            .nav_layout
            .rect_of(hit)
            .context("navigation button missing")?;
        self.on_click((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        Ok(())
    }
    fn verify_export_options(
        &mut self,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
        previous: &Path,
    ) -> anyhow::Result<()> {
        let root = self.shell.workspace().unwrap().root.clone();
        let path = previous.with_file_name("导出附加验收.md");
        let source="# 导出附加验收\r\n\r\nmochi://ai-locate?session=export-verification&message=answer\r\n\r\n原文结束。\r\n";
        std::fs::write(&path, source)?;
        let mut answer =
            AiStoredMessage::new("assistant", "本轮答案验证：**完整保留**，公式 $x^2$。");
        answer.set("id", serde_json::json!("answer"));
        let mut hidden = AiStoredMessage::new("assistant", "不应导出隐藏消息");
        hidden.set("hidden", serde_json::json!(true));
        AiSessionService::new(&root).save_session(&AiConversation {
            id: "export-verification".into(),
            messages: vec![
                AiStoredMessage::new("user", "本轮问题验证：如何导出？"),
                hidden,
                answer,
            ],
            ..Default::default()
        })?;
        let comment = sidecars::DocumentComment {
            resolved: false,
            id: "root".into(),
            parent_id: None,
            target_type: "document".into(),
            author: "验收作者".into(),
            content: "评论正文验证。".into(),
            created_at: "2026-09-05T00:00:00Z".into(),
            updated_at: None,
            anchor: None,
            attachments: vec![sidecars::CommentAttachment {
                id: "attachment".into(),
                name: "附件清单验证.pdf".into(),
                path: "not-read-private-file.pdf".into(),
                size: None,
            }],
        };
        let mut comment = comment;
        comment.anchor = Some(sidecars::CommentAnchor {
            kind: "text".into(),
            selected_text: "评论对象验证".into(),
            block_text: "原文结束。".into(),
            block_type: "paragraph".into(),
            block_index: 0,
            start_offset: 0,
            end_offset: 0,
            prefix: String::new(),
            suffix: String::new(),
        });
        let reply = sidecars::DocumentComment {
            resolved: false,
            id: "reply".into(),
            parent_id: Some("root".into()),
            content: "评论回复验证。".into(),
            attachments: Vec::new(),
            anchor: None,
            ..comment.clone()
        };
        sidecars::save_comments(&path.to_string_lossy(), vec![comment, reply])?;
        self.shell.open_file(&path);
        self.sync_state();
        for format in ["pdf", "markdown"] {
            self.run_menu_action(MenuAction::ExportDocument(path.clone(), format.into()));
            if format == "pdf" {
                for choice in [3, 6] {
                    let form = self.export_form.as_ref().context("export form missing")?;
                    let rect = form
                        .parts(self.renderer.viewport())
                        .into_iter()
                        .find(|(i, _)| *i == choice)
                        .unwrap()
                        .1;
                    self.on_click(
                        (rect.left + rect.right) / 2.0,
                        (rect.top + rect.bottom) / 2.0,
                    );
                }
            }
            self.verify_frame(output, &format!("export-{format}-options"), snapshot)?;
            let rect = self
                .export_form
                .as_ref()
                .unwrap()
                .parts(self.renderer.viewport())
                .into_iter()
                .find(|(i, _)| *i == 8)
                .unwrap()
                .1;
            self.on_click(
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0,
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let exported = loop {
                if let Some(event) = self.file_jobs.take().into_iter().next() {
                    match event.result.map_err(anyhow::Error::msg)? {
                        crate::file_runtime::Payload::Exported(path) => break path,
                        _ => anyhow::bail!("unexpected export event"),
                    }
                }
                ensure!(std::time::Instant::now() < deadline, "export timed out");
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let bytes = std::fs::read(exported)?;
            if format == "pdf" {
                ensure!(bytes.starts_with(b"%PDF"), "missing PDF output");
            } else {
                let value = String::from_utf8(bytes)?;
                ensure!(
                    value.contains("原文结束")
                        && !value.contains("mochi://")
                        && !value.contains("评论正文")
                        && !value.contains("本轮答案"),
                    "Markdown defaults leaked extras"
                );
            }
            ensure!(
                std::fs::read_to_string(&path)? == source,
                "export modified source"
            );
        }
        self.shell.open_file(previous);
        self.sync_state();
        Ok(())
    }
    pub(super) fn verify_frame(
        &mut self,
        output: &Path,
        name: &str,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        self.paint(HWND::default())?;
        let target = output.with_file_name(format!(
            "{}-{name}.png",
            output.file_stem().unwrap_or_default().to_string_lossy()
        ));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.renderer.save_snapshot(snapshot, &target)?;
        Ok(())
    }
    fn verify_ai_locator(
        &mut self,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
        previous: &Path,
    ) -> anyhow::Result<()> {
        let root = self.shell.workspace().unwrap().root.clone();
        let service = AiSessionService::new(&root);
        let mut index = service.load_index();
        index.sessions.push(mochi_core::ai::session::AiSessionMeta {
            id: "export-verification".into(),
            title: "定位验收会话".into(),
            ..Default::default()
        });
        service.save_index(&index)?;
        let reference = mochi_core::ai::locator::Locator {
            session_id: "export-verification".into(),
            message_id: "answer".into(),
            title: "定位验收会话".into(),
            snippet: "点击卡片，查看本轮问题与答案。".into(),
        };
        let url = reference.to_url();
        let source = format!("# AI 定位验收\n\n{url}\n\n原始定位链接保持在 Markdown 文件中。\n");
        let path = previous.with_file_name("AI定位验收.md");
        std::fs::write(&path, &source)?;
        self.shell.open_file(&path);
        self.editor_engaged = false;
        self.state.ai_panel_open = false;
        self.invalidate_main();
        self.sync_state();
        self.verify_frame(output, "ai-locator-card", snapshot)?;
        let area = self.editor_area;
        let x = area.left + crate::ui::editor_preferences::current().padding_left + 8.0;
        let y = (area.top as i32..area.bottom as i32)
            .step_by(2)
            .map(|y| y as f32)
            .find(|y| {
                self.doc
                    .link_at(area, &source, self.shell.active_scroll(), x, *y)
                    .as_deref()
                    == Some(url.as_str())
            })
            .context("locator card hit region missing")?;
        self.on_click(x, y);
        self.verify_frame(output, "ai-locator-target", snapshot)?;
        ensure!(
            self.ai
                .panel
                .active
                .as_ref()
                .is_some_and(|c| c.id == reference.session_id),
            "locator opened wrong session"
        );
        ensure!(
            self.ai.panel.located.as_deref() == Some(reference.message_id.as_str()),
            "locator failed to highlight target"
        );
        self.on_timer(HWND::default(), platform::TIMER_AI_LOCATOR);
        ensure!(
            self.ai.panel.located.is_none(),
            "locator highlight did not expire"
        );
        ensure!(
            std::fs::read_to_string(&path)? == source,
            "locator rewrote source"
        );
        self.state.ai_panel_open = false;
        self.shell.open_file(previous);
        self.invalidate_main();
        self.sync_state();
        Ok(())
    }
}
