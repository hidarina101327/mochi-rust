//! Agent 选择、审批模式和会话挂载点；界面选择只影响下一次固定请求，
//! 不会扩大正在运行的工具注册表。
use super::*;
impl App {
    pub(super) fn reload_ai_agents(&mut self) {
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let agents = mochi_core::ai::agent_config::AgentConfigService::new(&ws.root).load_agents();
        let requested = self
            .ai
            .panel
            .selected_agent_id
            .clone()
            .or_else(|| self.settings.get("ai.selectedAgentId"));
        let chosen = requested
            .as_ref()
            .and_then(|id| agents.iter().find(|a| &a.id == id))
            .or_else(|| {
                agents
                    .iter()
                    .find(|a| a.id == mochi_core::ai::agent_config::GENERAL_ASSISTANT_ID)
            });
        self.ai.panel.selected_agent_id = chosen.map(|a| a.id.clone());
        self.ai.panel.agent_name = chosen
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "通用助手".into());
        self.ai.panel.follow_up_frequency = chosen
            .map(|a| a.follow_up_frequency)
            .unwrap_or(mochi_core::ai::FollowUpFrequency::Medium);
        self.ai.panel.agents = agents.into_iter().map(|a| (a.id, a.name)).collect();
    }
    pub(super) fn ai_pick_agent(&mut self, rect: Rect) {
        if self.ai.panel.is_streaming() {
            return;
        }
        self.reload_ai_agents();
        let mut items = self
            .ai
            .panel
            .agents
            .iter()
            .map(|(id, name)| {
                let item = MenuItem::new(name, MenuAction::SelectAgent(id.clone()));
                if self.ai.panel.selected_agent_id.as_ref() == Some(id) {
                    item.icon(Icon::CHECK)
                } else {
                    item
                }
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            items.push(MenuItem::new(
                "配置 Agent…",
                MenuAction::OpenAgentDefinitions,
            ));
        }
        let current_freq = self.ai.panel.follow_up_frequency;
        let frequencies = [
            (
                mochi_core::ai::FollowUpFrequency::Never,
                "追问频率：从不 (never)",
            ),
            (
                mochi_core::ai::FollowUpFrequency::Low,
                "追问频率：较少 (low)",
            ),
            (
                mochi_core::ai::FollowUpFrequency::Medium,
                "追问频率：中等 (medium)",
            ),
            (
                mochi_core::ai::FollowUpFrequency::High,
                "追问频率：较多 (high)",
            ),
            (
                mochi_core::ai::FollowUpFrequency::Aggressive,
                "追问频率：激进 (aggressive)",
            ),
        ];
        for (i, (freq, label)) in frequencies.into_iter().enumerate() {
            let mut item = MenuItem::new(label, MenuAction::SetAgentFollowUpFrequency(freq));
            if i == 0 {
                item = item.separated();
            }
            if current_freq == freq {
                item = item.icon(Icon::CHECK);
            }
            items.push(item);
        }
        self.menu = Some(Menu::open_anchored(items, rect, self.renderer.viewport()));
    }
    pub(super) fn ai_select_agent(&mut self, id: &str) {
        if self.ai.panel.is_streaming() {
            return;
        }
        self.settings.set("ai.selectedAgentId", id);
        let _ = self.settings.flush();
        self.ai.panel.selected_agent_id = Some(id.to_string());
        self.reload_ai_agents();
        if self.ai.panel.follow_up_frequency == mochi_core::ai::FollowUpFrequency::Never {
            self.cancel_follow_up_jobs();
        }
        self.ai.panel.loaded_skills.clear();
    }
    pub(super) fn ai_set_apply_mode(&mut self, automatic: bool) {
        if self.ai.panel.is_streaming() {
            return;
        }
        if let Some(index) = app_settings::descriptors()
            .iter()
            .position(|d| d.key == "ai.editApplyMode")
        {
            self.run_menu_action(MenuAction::SetEnum(
                index,
                if automatic { "auto" } else { "approve" }.into(),
            ));
        }
    }
    pub(super) fn ai_pick_mount(&mut self, rect: Rect) {
        if self
            .ai
            .panel
            .active
            .as_ref()
            .is_none_or(|c| c.messages.is_empty())
        {
            return;
        }
        let mut items = self
            .shell
            .tabs()
            .iter()
            .filter_map(|t| {
                t.path().map(|path| {
                    MenuItem::new(&t.title, MenuAction::MountAiSession(path.to_path_buf()))
                })
            })
            .collect::<Vec<_>>();
        items.push(MenuItem::new(
            "选择其它文档…",
            MenuAction::PickAiMountDocument,
        ));
        self.menu = Some(Menu::open_anchored(items, rect, self.renderer.viewport()));
    }
    pub(super) fn ai_mount_session(&mut self, path: &Path) {
        let result = (|| -> anyhow::Result<()> {
            let ws = self
                .shell
                .workspace()
                .ok_or_else(|| anyhow::anyhow!("没有工作区"))?;
            let root = ws.root.canonicalize()?;
            let target = path.canonicalize()?;
            anyhow::ensure!(
                mochi_core::paths::path_is_within(&root, &target) && target.is_file(),
                "请选择当前工作区内的文档"
            );
            let session = self
                .ai
                .panel
                .active
                .as_ref()
                .filter(|c| !c.messages.is_empty())
                .ok_or_else(|| anyhow::anyhow!("会话为空"))?;
            let snippet = session
                .messages
                .iter()
                .rev()
                .find(|m| !m.is_hidden() && !m.content().is_empty())
                .map(|m| m.content().chars().take(160).collect::<String>())
                .unwrap_or_default();
            AiDocumentMountService::new(&ws.root).add_mounts(vec![
                mochi_core::ai::document_mounts::CreateMountInput {
                    session_id: session.id.clone(),
                    scope: mochi_core::ai::document_mounts::MountScope::Session,
                    message_id: None,
                    user_message_id: None,
                    document_path: path.to_string_lossy().replace('\\', "/"),
                    session_title: session.title.clone(),
                    snippet,
                    created_at: None,
                },
            ])?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.refresh_right_panel();
                self.state.status_text = "已挂载会话，原文档内容未修改".into();
            }
            Err(error) => self.ai.panel.error = error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_switching_approval_confirmation_and_mounting_preserve_boundaries() {
        let parent = std::env::temp_dir().join(format!(
            "mochi-ai-controls-{}",
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
            // 专用的内置 Agent 已停用；仍可选择用户创建的 Agent。
            std::fs::write(
                root.join("Agent配置/Agents/学习助手.md"),
                "---\nname: 学习助手\n---\n测试自定义 Agent",
            )
            .unwrap();
            app.reload_ai_agents();
            app.ai_select_agent("学习助手");
            assert_eq!(app.ai.panel.selected_agent_id.as_deref(), Some("学习助手"));
            assert_eq!(
                app.settings.get("ai.selectedAgentId").as_deref(),
                Some("学习助手")
            );
            app.ai.panel.streaming = Some(assistant::Streaming::default());
            app.ai_select_agent("日程助手");
            assert_eq!(app.ai.panel.selected_agent_id.as_deref(), Some("学习助手"));
            app.ai_set_apply_mode(true);
            assert!(app.dialog.is_none());
            app.ai.panel.streaming = None;
            let setting = app_settings::descriptor("ai.editApplyMode").unwrap();
            app.ai_set_apply_mode(true);
            assert!(app.dialog.is_some());
            assert_ne!(app.app_settings.read(setting).to_storage(), "auto");
            app.run_dialog_action(DialogAction::Dismiss);
            app.ai_set_apply_mode(true);
            app.run_dialog_action(DialogAction::EnableAutomaticAiEdits);
            assert_eq!(app.app_settings.read(setting).to_storage(), "auto");
            app.ai_set_apply_mode(false);
            assert_eq!(app.app_settings.read(setting).to_storage(), "approve");
            let path = root.join("原文.md");
            std::fs::write(&path, "# 原始正文\r\n").unwrap();
            app.ai_new_session();
            app.ai
                .panel
                .active
                .as_mut()
                .unwrap()
                .messages
                .push(AiStoredMessage::new("user", "测试会话"));
            app.ai_persist_active();
            app.ai_mount_session(&path);
            assert_eq!(
                AiDocumentMountService::new(&root)
                    .mounts_for_document(&path.to_string_lossy())
                    .len(),
                1
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "# 原始正文\r\n");
            let outside = parent.join("外部.md");
            std::fs::write(&outside, "不能挂载").unwrap();
            app.ai_mount_session(&outside);
            assert!(app.ai.panel.error.contains("工作区内"));
            assert!(AiDocumentMountService::new(&root)
                .mounts_for_document(&outside.to_string_lossy())
                .is_empty());
        }
        let _ = std::fs::remove_dir_all(parent);
    }
}
