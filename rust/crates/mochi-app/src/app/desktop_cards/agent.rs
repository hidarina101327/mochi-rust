//! 管理桌面卡片中的控制台 Agent 请求。
use super::*;

impl App {
    pub(in crate::app) fn desktop_console_control(
        &mut self,
        action: &str,
        args: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        use serde_json::json;
        if action == "open" {
            self.open_desktop_manager();
            return Ok(json!({"opened":true}));
        }
        if !self.desktop.loaded {
            return Err("桌面卡片尚未加载".into());
        }
        let id = args["id"].as_str().ok_or("缺少卡片 id")?;
        let index = self
            .desktop
            .config
            .cards
            .iter()
            .position(|c| c.id == id)
            .ok_or("卡片不存在")?;
        match action {
            "edit" => {
                if self.desktop.panel.as_ref().is_some_and(|p| p.dirty) {
                    return Err("卡片编辑器有未保存内容".into());
                }
                let page = args["data"]["pageId"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        self.desktop.config.cards[index]
                            .pages
                            .first()
                            .map(|p| p.id.clone())
                    })
                    .ok_or("卡片没有页面")?;
                if !self.desktop.config.cards[index]
                    .pages
                    .iter()
                    .any(|p| p.id == page)
                {
                    return Err("页面不存在".into());
                }
                self.desktop_manage_page(id, &page);
            }
            "focus" => {
                if !self.desktop.config.cards[index].enabled {
                    return Err("卡片隐藏中，请先 show".into());
                }
                if self.desktop.paused || !self.desktop.windows.contains(id) {
                    return Err("卡片窗口尚未显示，请稍后重试".into());
                }
                self.desktop.windows.locate(id);
            }
            "show" | "hide" => {
                if self.desktop.panel.is_some()
                    || self.desktop.job.is_some()
                    || !self.desktop.pending.is_empty()
                    || self.desktop.unsaved
                {
                    return Err("卡片正在编辑或保存，请稍后重试".into());
                }
                let root = self.desktop.root.clone().ok_or("工作区未加载")?;
                let path = model::DesktopConfig::path(&root);
                let perms = self.ai.permissions.as_ref().ok_or("权限服务不可用")?;
                perms
                    .assert_tool_action_allowed(
                        mochi_core::ai::permission::AiToolAction::WriteFile,
                        Some(&path.to_string_lossy()),
                    )
                    .map_err(|e| e.message)?;
                if perms.path_requires_proposal(&path.to_string_lossy())
                    && !self.console_automatic()
                {
                    return Err("卡片配置需要审批".into());
                }
                if model::DesktopConfig::load(&root).map_err(|e| e.to_string())?
                    != self.desktop.config
                {
                    return Err("卡片配置已在外部更新，请重新读取".into());
                }
                let mut config = self.desktop.config.clone();
                config.cards[index].enabled = action == "show";
                config.save(&root).map_err(|e| e.to_string())?;
                self.desktop.config = config;
                self.desktop.revision += 1;
                self.desktop_sync_windows();
                self.desktop_refresh();
            }
            _ => return Err("未知卡片操作".into()),
        }
        Ok(json!({"applied":true,"id":id,"action":action}))
    }
    pub(in crate::app) fn desktop_agent_requests(&mut self) {
        let Some(request) = self.ai.host.as_ref().and_then(|h| h.take_desktop_request()) else {
            return;
        };
        let reject = |message: &str| {
            let _ = request.reply.send(Err(message.into()));
        };
        if Instant::now() > request.deadline {
            reject("请求已过期，请重试");
            return;
        }
        if !self.desktop.loaded {
            reject("桌面卡片尚未加载");
            return;
        }
        if request.name == "desktop_cards_get" {
            let _ = request
                .reply
                .send(Ok(mochi_core::ai::tools::desktop_tools::snapshot(
                    &self.desktop.config,
                )));
            return;
        }
        if self.desktop.panel.is_some()
            || self.desktop.job.is_some()
            || !self.desktop.pending.is_empty()
            || self.desktop.unsaved
            || self.desktop.saving_editor
        {
            reject("卡片正在编辑或保存；请结束编辑后重新读取并重试");
            return;
        }
        let Some(root) = self.desktop.root.clone() else {
            reject("请先打开工作区");
            return;
        };
        let result = match request.name.as_str() {
            "desktop_cards_update" => {
                mochi_core::ai::tools::desktop_tools::update(&self.desktop.config, &request.args)
            }
            "desktop_cards_batch" => {
                mochi_core::ai::tools::desktop_tools::batch(&self.desktop.config, &request.args)
            }
            _ => Err("未知桌面卡片工具".into()),
        };
        match result {
            Ok(config) => {
                self.desktop.saving_editor = true;
                self.desktop.agent_reply = Some(request.reply);
                self.desktop_enqueue(runtime::Job::Save(
                    root,
                    config,
                    self.desktop.revision + 1,
                    true,
                ));
            }
            Err(e) => {
                let _ = request.reply.send(Err(e));
            }
        }
    }
}
