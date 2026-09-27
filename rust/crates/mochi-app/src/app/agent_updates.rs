//! 检查 Agent 更新，并处理更新详情和用户操作。
use super::*;
use mochi_core::ai::agent_config::AgentConfigService;

impl App {
    pub(in crate::app) fn check_agent_updates(&mut self, notify: bool) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let service = AgentConfigService::new(root);
        match service.agent_updates(true) {
            Ok(updates) => self.agent.updates = updates,
            Err(error) => {
                self.agent.error = Some(format!("检查 Agent 更新失败：{error}"));
                return;
            }
        }
        if notify {
            match service.take_agent_update_notices() {
                Ok(names) if !names.is_empty() => self.publish_notification(
                    crate::ui::notifications::Category::Assistant,
                    "内置 Agent 定义可更新",
                    &format!("发现 {} 项定义差异。打开「AI 定义」查看 diff，选择更新或跳过；本地配置尚未修改。", names.len()),
                ),
                Err(error) => self.agent.error = Some(format!("记录 Agent 更新提示失败：{error}")),
                _ => {}
            }
        }
    }

    pub(super) fn open_agent_update(&mut self, index: usize) {
        let Some(update) = self.agent.updates.get(index).cloned() else {
            return;
        };
        // 重新读取列表：列表打开期间，另一个编辑器可能已经保存了文件。
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let result = AgentConfigService::new(root)
            .agent_updates(true)
            .and_then(|updates| {
                let update = updates
                    .into_iter()
                    .find(|u| u.relative_path == update.relative_path)
                    .ok_or_else(|| anyhow::anyhow!("定义已变化，请刷新更新列表"))?;
                Ok((update.diff()?, update))
            });
        match result {
            Ok((diff, update)) => {
                self.park_agent_source();
                self.agent.update_review = Some(update);
                self.agent.update_diff = diff;
                self.agent.update_upstream = false;
                self.agent.scroll = 0.0;
                self.agent.error = None;
                self.focus = Focus::Main;
            }
            Err(error) => self.agent.error = Some(format!("读取 Agent 差异失败：{error}")),
        }
        self.invalidate_main();
    }

    pub(super) fn on_agent_update_action(&mut self, hit: agent_config::Hit) {
        use agent_config::Hit;
        let Some(review) = self.agent.update_review.clone() else {
            return;
        };
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let service = AgentConfigService::new(root);
        match hit {
            Hit::UpdateBack => {
                self.agent.update_review = None;
                self.agent.scroll = 0.0;
                self.reload_agent_config();
            }
            Hit::UpdatePrevious | Hit::UpdateNext => {
                let len = self.agent.updates.len();
                if len > 0 {
                    let current = self
                        .agent
                        .updates
                        .iter()
                        .position(|u| u.relative_path == review.relative_path)
                        .unwrap_or(0);
                    let next = if hit == Hit::UpdateNext {
                        (current + 1) % len
                    } else {
                        (current + len - 1) % len
                    };
                    self.open_agent_update(next);
                }
            }
            Hit::UpdateDiffMode => {
                let upstream = !self.agent.update_upstream && review.baseline.is_some();
                let result = if upstream {
                    review.upstream_diff().map(|s| s.unwrap_or_default())
                } else {
                    review.diff()
                };
                match result {
                    Ok(diff) => {
                        self.agent.update_diff = diff;
                        self.agent.update_upstream = upstream;
                        self.agent.scroll = 0.0;
                    }
                    Err(error) => self.agent.error = Some(format!("读取差异失败：{error}")),
                }
            }
            Hit::UpdateApply if self.agent.update_upstream => {
                self.on_agent_update_action(Hit::UpdateDiffMode);
            }
            Hit::UpdateApply | Hit::UpdateSkip => {
                if hit == Hit::UpdateApply
                    && self
                        .agent
                        .source_drafts
                        .get(&review.source_path)
                        .is_some_and(|e| e.dirty)
                {
                    self.agent.error = Some(
                        "此 Agent 有未保存的编辑，请先返回源文件保存，再查看最新差异。".into(),
                    );
                    self.agent.scroll = 0.0;
                    self.invalidate_main();
                    return;
                }
                let result = if hit == Hit::UpdateApply {
                    service
                        .apply_agent_update(&review)
                        .map(|backup| format!("Agent 已更新；原定义备份：{}", backup.display()))
                } else {
                    service
                        .skip_agent_update(&review)
                        .map(|_| "已跳过此版本；下一版有变化时会再次提示。".into())
                };
                match result {
                    Ok(message) => {
                        self.agent.update_review = None;
                        self.agent.scroll = 0.0;
                        self.state.status_text = message;
                        self.reload_agent_config();
                        self.reload_ai_agents();
                    }
                    Err(error) => {
                        self.agent.error = Some(format!("Agent 更新未完成：{error}"));
                        self.agent.scroll = 0.0;
                    }
                }
            }
            _ => {}
        }
        self.invalidate_main();
    }
}
