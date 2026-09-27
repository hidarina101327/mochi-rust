//! 脚本提案始终需要审批，包括预览和列入白名单的语言。
use super::{host::ToolHost, ToolArgs, ToolExecutor, ToolOutcome};
use crate::{
    ai::permission::AiToolAction,
    script::{self, ScriptSpec},
};
use serde_json::json;
use std::sync::Arc;

pub struct ScriptToolExecutor {
    host: Arc<dyn ToolHost>,
}
impl ScriptToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }
}
impl ToolExecutor for ScriptToolExecutor {
    fn handles(&self, name: &str) -> bool {
        matches!(name, "script_environment" | "script_run")
    }
    fn required_actions(
        &self,
        name: &str,
        _args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        if name == "script_environment" {
            vec![]
        } else {
            vec![(AiToolAction::ExecuteCommand, None)]
        }
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "script_environment" => Ok(script::environment()),
            "script_run" => {
                let mut spec: ScriptSpec = serde_json::from_value(args.value().clone())
                    .map_err(|e| format!("脚本参数错误：{e}"))?;
                spec.validate()?;
                let cwd = spec.working_directory(self.host.workspace_root())?;
                spec.cwd = Some(cwd.to_string_lossy().into_owned());
                let runtime = script::runtime_path(spec.language, &script::application_dir()?)?;
                self.host.propose_shell_command(json!({"pendingShellCommand":{
                    "id":format!("script-{}-{}",crate::jstime::now_millis(),crate::paths::random_base36(8)),
                    "kind":"script", "command":spec.code, "cwd":spec.cwd, "summary":spec.summary,
                    "program":spec.language.name(), "runtime":runtime, "timeoutMs":spec.timeout_ms,
                    "script":spec, "status":"pending", "sandboxed":false, "previewIsEnforcedReadOnly":false
                }, "message":"脚本仅已提交审批，尚未运行；请等待批准后的真实结果，不要重复提交。"}))
            }
            _ => Err("未知脚本工具".into()),
        }
    }
}

#[cfg(test)]
mod tests;
