use super::*;
use crate::ai::{
    models::{AiToolCall, AiToolFunction},
    permission::{ActionPermissions, AiPermissionService},
    tools::ToolRegistry,
};
use std::path::{Path, PathBuf};
struct Host(PathBuf);
impl ToolHost for Host {
    fn workspace_root(&self) -> &Path {
        &self.0
    }
    fn shell_whitelist(&self) -> Vec<String> {
        vec!["python".into(), "powershell".into(), "cmd".into()]
    }
}
fn invoke(registry: &ToolRegistry, name: &str, args: serde_json::Value) -> serde_json::Value {
    serde_json::from_str(&registry.execute(&AiToolCall {
        id: "c".into(),
        kind: "function".into(),
        function: AiToolFunction {
            name: name.into(),
            arguments: args.to_string(),
        },
    }))
    .unwrap()
}
#[test]
fn script_tool_permission_cannot_be_bypassed_by_preview_or_whitelist() {
    let root = std::env::temp_dir();
    let perms = Arc::new(AiPermissionService::new(&root));
    let registry = ToolRegistry::new(perms.clone()).with(Arc::new(ScriptToolExecutor::new(
        Arc::new(Host(root.clone())),
    )));
    let env = invoke(&registry, "script_environment", json!({}));
    assert_eq!(env["ok"], true);
    let args = json!({"language":"cmd","code":"echo should-not-run","summary":"测试不运行","intent":"preview"});
    assert_eq!(invoke(&registry, "script_run", args.clone())["ok"], false);
    #[cfg(windows)]
    {
        let mut actions = ActionPermissions::default();
        actions.set(AiToolAction::ExecuteCommand, true);
        perms.set_action_permissions(actions);
        let response = invoke(&registry, "script_run", args);
        assert_eq!(response["ok"], true, "{response}");
        let pending = &response["data"]["pendingShellCommand"];
        assert_eq!(pending["status"], "pending");
        assert_eq!(pending["script"]["code"], "echo should-not-run");
        assert_eq!(pending["sandboxed"], false);
        assert!(pending.get("result").is_none());
        assert!(pending["runtime"].as_str().unwrap().ends_with("cmd.exe"));
        assert!(script::approved_spec(
            pending,
            pending["command"].as_str().unwrap(),
            pending["cwd"].as_str().unwrap()
        )
        .is_ok());
    }
}
