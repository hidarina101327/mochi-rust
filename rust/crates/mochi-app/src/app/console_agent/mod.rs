//! 定义控制台 Agent 请求与响应使用的数据类型和辅助函数。
use super::*;
use anyhow::{bail, ensure, Context};
use serde_json::{json, Value};
use std::time::Instant;
type CResult<T> = anyhow::Result<T>;
mod agents;
mod content;
mod learning;
mod navigation;
mod review;
mod structured;
mod system;
#[cfg(test)]
mod tests;
pub(super) use system::SystemState;
#[derive(Debug)]
struct ApprovalRequired;
impl std::fmt::Display for ApprovalRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "此操作需要用户批准")
    }
}
impl std::error::Error for ApprovalRequired {}

fn text<'a>(v: &'a Value, key: &str) -> CResult<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("缺少 {key}"))
}
fn data(v: &Value) -> &Value {
    v.get("data").unwrap_or(&Value::Null)
}
fn revision(v: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.to_string().hash(&mut h);
    format!("{:016x}", h.finish())
}
fn versioned(v: Value) -> Value {
    json!({"revision":revision(&v),"value":v})
}
fn check_revision(args: &Value, current: &Value) -> CResult<()> {
    ensure!(
        args["revision"].as_str() == Some(revision(current).as_str()),
        "数据已变化或缺少 revision，请重新读取后合并"
    );
    Ok(())
}
fn operations(v: &Value) -> CResult<&Vec<Value>> {
    let ops = v["operations"].as_array().context("缺少 operations")?;
    ensure!(
        !ops.is_empty() && ops.len() <= 64,
        "operations 需要 1–64 项"
    );
    Ok(ops)
}

impl App {
    pub(in crate::app) fn console_agent_requests(&mut self, hwnd: HWND) {
        let Some(request) = self.ai.host.as_ref().and_then(|h| h.take_console_request()) else {
            return;
        };
        let result = if self
            .ai
            .run
            .as_ref()
            .is_some_and(|r| r.cancel.load(std::sync::atomic::Ordering::Relaxed))
        {
            Err(anyhow::anyhow!("Agent 已停止，未执行控制台操作"))
        } else if Instant::now() > request.deadline {
            Err(anyhow::anyhow!("请求已过期，请回读状态"))
        } else {
            self.console_dispatch(hwnd, &request.name, &request.args)
        };
        let _ = request.reply.send(result.map_err(|e| format!("{e:#}")));
        // 消息处理器会安排下一次绘制；保留编辑器焦点和撤销状态。
    }
    pub(in crate::app) fn console_automatic(&self) -> bool {
        self.console_jobs.reviewing
            || mochi_core::app_settings::descriptor("ai.editApplyMode")
                .is_some_and(|d| self.app_settings.read(d).to_storage() == "auto")
    }
    fn console_root(&self) -> CResult<PathBuf> {
        Ok(self
            .shell
            .workspace()
            .context("请先打开工作区")?
            .root
            .clone())
    }
    fn console_path(&self, path: &str, write: bool) -> CResult<PathBuf> {
        use mochi_core::ai::{
            permission::AiToolAction,
            tools::host::{resolve_workspace_path, ResolveOptions},
        };
        let host = self.ai.host.as_ref().context("Agent 宿主不可用")?;
        let path = resolve_workspace_path(
            host.as_ref(),
            Some(path),
            ResolveOptions {
                fallback_to_selection: false,
                allow_mochi_dir: true,
            },
        )
        .map_err(anyhow::Error::msg)?;
        if write {
            ensure!(
                !self.shell.tabs().iter().any(|t| t.dirty()
                    && t.path()
                        .is_some_and(|p| mochi_core::paths::paths_equal(p, Path::new(&path)))),
                "目标文档存在未保存编辑，请先保存"
            );
        }
        let perms = self.ai.permissions.as_ref().context("权限服务不可用")?;
        perms
            .assert_tool_action_allowed(
                if write {
                    AiToolAction::WriteFile
                } else {
                    AiToolAction::ReadFile
                },
                Some(&path),
            )
            .map_err(|e| anyhow::anyhow!(e.message))?;
        if write {
            if perms.path_requires_proposal(&path) && !self.console_automatic() {
                return Err(ApprovalRequired.into());
            }
        }
        Ok(PathBuf::from(path))
    }
    fn console_scope(&self, path: &str, write: bool) -> CResult<PathBuf> {
        use mochi_core::ai::permission::AiToolAction;
        let root = self.console_path(path, write)?;
        let perms = self.ai.permissions.as_ref().context("权限服务不可用")?;
        // 模块操作可能会影响子项，包括尚不存在的路径。
        // 允许访问父目录，不代表也允许访问受限的子项。
        for rule in perms.load_permissions() {
            if mochi_core::paths::path_is_within(&root, Path::new(&rule.path)) {
                perms
                    .assert_tool_action_allowed(
                        if write {
                            AiToolAction::WriteFile
                        } else {
                            AiToolAction::ReadFile
                        },
                        Some(&rule.path),
                    )
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                if write && perms.path_requires_proposal(&rule.path) && !self.console_automatic() {
                    return Err(ApprovalRequired.into());
                }
            }
        }
        Ok(root)
    }
    fn console_target(&self, args: &Value, write: bool) -> CResult<PathBuf> {
        let path = args["path"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                self.active_file_path()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .context("缺少目标 path")?;
        self.console_path(&path, write)
    }
    fn console_execute(&mut self, hwnd: HWND, name: &str, args: &Value) -> CResult<Value> {
        use mochi_core::ai::tools::{
            console_tools::{ConsoleToolExecutor, SPECS},
            ToolArgs, ToolExecutor,
        };
        let action = args["action"].as_str().unwrap_or("get");
        let spec = SPECS
            .iter()
            .find(|s| s.0 == name)
            .context("未注册的控制台工具")?;
        ensure!(
            spec.2.split(',').any(|a| a == action),
            "此工具不支持 {action}"
        );
        let host = self.ai.host.clone().context("Agent 宿主不可用")?;
        let perms = self.ai.permissions.as_ref().context("权限服务不可用")?;
        for (action, path) in
            ConsoleToolExecutor(host).required_actions(name, &ToolArgs::from_value(args.clone()))
        {
            perms
                .assert_tool_action_allowed(action, path.as_deref())
                .map_err(|e| anyhow::anyhow!(e.message))?;
        }
        match name {
            "console_state"
            | "console_navigate"
            | "console_tabs"
            | "console_workspace"
            | "calendar_navigate"
            | "desktop_cards_control"
            | "workflow_open" => self.console_navigation(hwnd, name, action, args),
            "inbox_manage"
            | "favorites_manage"
            | "recent_manage"
            | "unread_manage"
            | "templates_manage"
            | "notifications_manage"
            | "home_stats" => self.console_content(name, action, args),
            "base_manage" | "canvas_manage" | "comments_manage" | "annotations_manage" => {
                self.console_structured(name, action, args)
            }
            "ai_sessions_manage" | "agent_definitions_manage" => {
                self.console_agents(name, action, args)
            }
            "english_manage" | "exam_manage" | "pomodoro_manage" => {
                self.console_learning(name, action, args)
            }
            _ => self.console_system(hwnd, name, action, args),
        }
    }
}
