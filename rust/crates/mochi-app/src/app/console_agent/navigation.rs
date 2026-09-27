//! 生成控制台页面快照，并处理页面导航。
use super::*;
impl App {
    fn console_snapshot(&self) -> Value {
        let visible = |p: &Path| {
            self.ai
                .permissions
                .as_ref()
                .is_some_and(|s| s.is_path_visible(&p.to_string_lossy()))
        };
        let tabs = self
            .shell
            .tabs()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.path().is_none_or(visible))
            .map(|(i, t)| json!({"index":i,"title":t.title,"path":t.path(),"dirty":t.dirty()}))
            .collect::<Vec<_>>();
        let active = self
            .shell
            .active_tab()
            .filter(|i| tabs.iter().any(|t| t["index"] == *i));
        json!({"workspace":self.shell.workspace().map(|w|&w.root),"page":format!("{:?}",self.state.view),"activeTab":active,"tabs":tabs,"split":{"other":self.split.other.as_deref().filter(|p|visible(p)),"rightFocused":self.split.right,"ratio":self.split.ratio},"settings":self.settings_overlay,"dialogOpen":self.dialog.is_some(),"aiPanelOpen":self.state.ai_panel_open})
    }
    pub(super) fn console_navigation(
        &mut self,
        hwnd: HWND,
        name: &str,
        action: &str,
        a: &Value,
    ) -> CResult<Value> {
        let d = data(a);
        match name {
            "console_state" => return Ok(versioned(self.console_snapshot())),
            "console_workspace" => {
                if action == "switch" {
                    ensure!(
                        !self.shell.tabs().iter().any(|t| t.dirty()),
                        "工作区有未保存的标签，请先保存"
                    );
                    let target = self.console_external(text(a, "path")?, true)?;
                    ensure!(target.is_dir(), "工作区必须是目录");
                    self.open_workspace(hwnd, target, true)?;
                }
            }
            "console_navigate" => {
                self.focus = Focus::Main;
                self.state.view = match text(d, "module")? {
                    "home" => {
                        self.reload_home_dashboard();
                        WorkspaceView::Home
                    }
                    "editor" => WorkspaceView::Editor,
                    "inbox" => WorkspaceView::Inbox,
                    "knowledge" => {
                        self.open_knowledge();
                        self.state.view
                    }
                    "favorites" => {
                        self.open_favorites();
                        self.state.view
                    }
                    "recent" => {
                        self.reload_recent();
                        WorkspaceView::Recent
                    }
                    "templates" => {
                        self.reload_templates();
                        WorkspaceView::Templates
                    }
                    "schedule" => {
                        self.reload_schedule();
                        WorkspaceView::Schedule
                    }
                    "ai" => WorkspaceView::MochiAi,
                    "agents" => {
                        self.reload_agent_config();
                        WorkspaceView::AgentConfig
                    }
                    "quick_note" => {
                        ensure!(self.ensure_quick_note_tab(), "无法打开随手记");
                        WorkspaceView::QuickNote
                    }
                    "workflows" => {
                        self.workflows_open();
                        WorkspaceView::Automations
                    }
                    "desktop_cards" => {
                        self.open_desktop_manager();
                        WorkspaceView::DesktopCards
                    }
                    "marketplace" => {
                        self.open_marketplace();
                        WorkspaceView::Marketplace
                    }
                    "english" => {
                        self.plugin_workspace = Some("english-lab".into());
                        WorkspaceView::Plugin
                    }
                    "settings" => {
                        self.open_settings(d["section"].as_str().unwrap_or("general"));
                        self.state.view
                    }
                    "notifications" => {
                        self.open_notification_center();
                        self.state.view
                    }
                    _ => bail!("未知 module，请按 Skill 中的模块名称调用"),
                };
            }
            "console_tabs" => {
                if matches!(action, "unsplit" | "focus_pane") {
                    if action == "unsplit" {
                        if self.split.other.is_some() {
                            ensure!(self.close_split_view(), "分屏中有未完成的编辑");
                        }
                    } else {
                        ensure!(self.split.other.is_some(), "当前没有分屏");
                        let right = d["right"].as_bool().context("缺少 right")?;
                        if self.split.right != right {
                            ensure!(self.focus_other_editor(), "分屏无法切换焦点");
                        }
                    }
                } else {
                    if action == "split" {
                        if let Some(r) = d["ratio"].as_f64() {
                            ensure!(
                                r.is_finite() && (0.2..=0.8).contains(&r),
                                "ratio 范围 0.2–0.8"
                            );
                        }
                    }
                    let previous = self.shell.active_tab();
                    let path = self.console_target(a, false)?;
                    if matches!(action, "open" | "locate" | "split") {
                        ensure!(self.shell.open_file_with_mode(&path, true), "打开文档失败");
                    }
                    let index = self
                        .shell
                        .tabs()
                        .iter()
                        .position(|t| t.path() == Some(path.as_path()))
                        .context("标签未打开")?;
                    if action == "close" {
                        ensure!(
                            !self.shell.tabs()[index].dirty(),
                            "标签有未保存内容，请先保存"
                        );
                        self.shell.close_tab(index);
                    } else {
                        self.shell.select_tab(index);
                    }
                    if action == "split" {
                        ensure!(
                            self.shell.tabs()[index].buffer().is_some(),
                            "分屏需要文本类文档"
                        );
                        if let Some(previous) = previous {
                            self.shell.select_tab(previous);
                        }
                        self.split_to_right(path);
                        ensure!(self.split.other.is_some(), "分屏未完成，请先结束当前编辑");
                        if let Some(r) = d["ratio"].as_f64() {
                            ensure!(
                                r.is_finite() && (0.2..=0.8).contains(&r),
                                "ratio 范围 0.2–0.8"
                            );
                            self.split.ratio = r as f32;
                        }
                    }
                    if action == "locate" {
                        let line = d["line"].as_u64().context("缺少 line（从 1 开始）")?;
                        ensure!(line > 0, "line 从 1 开始");
                        let b = self
                            .shell
                            .active_buffer_mut()
                            .context("目标不是文本编辑器")?;
                        ensure!(
                            line as usize <= b.text().split('\n').count(),
                            "行号超出文档范围"
                        );
                        let offset = b
                            .text()
                            .split_inclusive('\n')
                            .take(line as usize - 1)
                            .map(str::len)
                            .sum::<usize>()
                            .min(b.text().len());
                        b.set_cursor(offset, false);
                        self.doc.invalidate();
                        self.source.invalidate();
                        self.editor_engaged = true;
                        if self.shell.active().is_some_and(|t| t.source_mode()) {
                            let i = self.shell.active_tab().unwrap();
                            let b = self.shell.active().and_then(|t| t.buffer()).unwrap();
                            self.source.ensure(self.editor_area, i, b);
                            self.scroll_caret_into_view();
                        } else {
                            self.after_doc_selection_change(true);
                        }
                    }
                    self.state.view = WorkspaceView::Editor;
                }
                self.sync_state();
            }
            "calendar_navigate" => {
                self.state.view = WorkspaceView::Schedule;
                self.reload_schedule();
                if let Some(view) = d["view"]
                    .as_str()
                    .and_then(crate::ui::agenda::View::from_wire)
                {
                    self.sched.view.set_view(view);
                }
                if let Some(date) = d["date"].as_str() {
                    let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")?;
                    self.sched.view.goto_day(date);
                }
                if let Some(id) = a["id"].as_str() {
                    ensure!(self.agenda_open(id), "日程待办条目不存在");
                }
                self.sync_state();
            }
            "workflow_open" => {
                ensure!(!self.workflows.view.dirty, "工作流有未保存编辑");
                self.workflows_open();
                if let Some(id) = a["id"].as_str() {
                    let saved = self
                        .workflows
                        .store
                        .as_ref()
                        .context("工作流不可用")?
                        .get(id)
                        .map_err(anyhow::Error::msg)?;
                    self.workflows.view.open(saved);
                }
            }
            "desktop_cards_control" => {
                let path = mochi_core::desktop_cards::DesktopConfig::path(self.console_root()?);
                self.console_path(&path.to_string_lossy(), matches!(action, "show" | "hide"))?;
                return self
                    .desktop_console_control(action, a)
                    .map_err(anyhow::Error::msg);
            }
            _ => bail!("未知导航工具"),
        }
        Ok(versioned(self.console_snapshot()))
    }
}
