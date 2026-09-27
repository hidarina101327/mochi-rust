use super::*;
use crate::ai::tools::{host::ActiveDocument, ToolRegistry};
use crate::ai::{
    models::{AiToolCall, AiToolFunction},
    permission::{AiPermissionLevel, AiPermissionService},
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
struct Host {
    root: PathBuf,
    propose: bool,
    proposals: Mutex<Vec<PendingFileOperation>>,
    changes: Mutex<Vec<FileChange>>,
}
impl ToolHost for Host {
    fn workspace_root(&self) -> &Path {
        &self.root
    }
    fn active_document(&self) -> Option<ActiveDocument> {
        Some(ActiveDocument {
            id: "base".into(),
            title: "test".into(),
            path: self.root.join("test.mcb").to_string_lossy().into(),
            is_dirty: self.propose,
        })
    }
    fn should_propose(&self, _: &str) -> bool {
        self.propose
    }
    fn submit_proposal(&self, p: PendingFileOperation) -> Result<Value, String> {
        self.proposals.lock().unwrap().push(p);
        Ok(json!({"queuedForApproval":true}))
    }
    fn notify_change(&self, c: FileChange) {
        self.changes.lock().unwrap().push(c);
    }
}
struct Fixture {
    host: Arc<Host>,
    registry: ToolRegistry,
    permissions: Arc<AiPermissionService>,
}
impl Fixture {
    fn new(propose: bool) -> Self {
        let root = std::env::temp_dir().join(base::create_id("mochi-auto-tools"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("test.mcb"),
            include_str!("../../../../../../../tests/fixtures/base-v1.mcb"),
        )
        .unwrap();
        let host = Arc::new(Host {
            root: root.clone(),
            propose,
            proposals: Mutex::new(vec![]),
            changes: Mutex::new(vec![]),
        });
        let permissions = Arc::new(AiPermissionService::new(&root));
        let registry = ToolRegistry::new(permissions.clone())
            .with(Arc::new(BaseAutomationToolExecutor::new(host.clone())));
        Self {
            host,
            registry,
            permissions,
        }
    }
    fn raw(&self) -> String {
        std::fs::read_to_string(self.host.root.join("test.mcb")).unwrap()
    }
    fn call(&self, name: &str, args: Value) -> Value {
        serde_json::from_str(&self.registry.execute(&AiToolCall {
            id: "call".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: format!("base_automation_{name}"),
                arguments: args.to_string(),
            },
        }))
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.host.root);
    }
}
fn args() -> Value {
    json!({"tableId":"table_main","viewId":"view_grid","rule":{"id":"auto_test","name":"标记完成","enabled":true,"trigger":{"type":"recordCreated"},"conditions":[],"actions":[{"type":"updateRecord","values":{"checked":{"type":"literal","value":true}}}]},"recordIds":["record_a"]})
}
#[test]
fn agent_put_always_saves_draft_and_returns_activation_guidance() {
    let f = Fixture::new(false);
    let r = f.call("put", args());
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["data"]["rule"]["enabled"], false);
    assert_eq!(r["data"]["applied"], true);
    let list = f.call("list", json!({}));
    assert_eq!(list["data"]["views"][0]["rules"][0]["id"], "auto_test");
    assert!(list["data"]["guide"]
        .as_str()
        .unwrap()
        .contains("base_automation_test"));
    assert_eq!(f.host.changes.lock().unwrap().len(), 1);
}
#[test]
fn test_never_writes_or_proposes_and_run_is_proposed_with_original_guard() {
    let f = Fixture::new(true);
    let original = f.raw();
    let test = f.call("test", args());
    assert_eq!(test["ok"], true, "{test}");
    assert_eq!(test["data"]["dryRun"], true);
    assert_eq!(f.raw(), original);
    assert!(f.host.proposals.lock().unwrap().is_empty());
    let run = f.call("run", args());
    assert_eq!(run["data"]["applied"], false, "{run}");
    assert_eq!(f.raw(), original);
    let p = f.host.proposals.lock().unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].previous_content.as_deref(), Some(original.as_str()));
    assert!(f.host.changes.lock().unwrap().is_empty());
}
#[test]
fn run_once_logs_without_installing_or_enabling_rule() {
    let f = Fixture::new(false);
    let r = f.call("run", args());
    assert_eq!(r["ok"], true, "{r}");
    let doc = base::parse_base_document(&f.raw()).unwrap();
    assert!(automation::rules(&doc.tables[0].views[0])
        .unwrap()
        .is_empty());
    assert_eq!(automation::runtime(&doc).runs.len(), 1);
    assert_eq!(doc.tables[0].records[0].values["checked"], true);
}
#[test]
fn invalid_action_does_not_write_or_propose() {
    let f = Fixture::new(true);
    let original = f.raw();
    let mut a = args();
    a["rule"]["actions"][0]["values"] = json!({"amount":{"type":"literal","value":"not a number"}});
    let r = f.call("put", a);
    assert_eq!(r["ok"], false);
    assert_eq!(f.raw(), original);
    assert!(f.host.proposals.lock().unwrap().is_empty());
}
#[test]
fn workspace_escape_and_non_base_paths_are_rejected() {
    let f = Fixture::new(false);
    for path in ["../outside.mcb", "test.md", ".mochi/private.mcb"] {
        let r = f.call("list", json!({"path":path}));
        assert_eq!(r["ok"], false, "{r}");
    }
}
#[test]
fn delete_removes_only_requested_rule() {
    let f = Fixture::new(false);
    assert_eq!(f.call("put", args())["ok"], true);
    let before = base::parse_base_document(&f.raw()).unwrap().tables[0]
        .records
        .clone();
    assert_eq!(
        f.call(
            "delete",
            json!({"tableId":"table_main","viewId":"view_grid","ruleId":"auto_test"})
        )["ok"],
        true
    );
    let d = base::parse_base_document(&f.raw()).unwrap();
    assert_eq!(d.tables[0].records, before);
    assert!(automation::rules(&d.tables[0].views[0]).unwrap().is_empty());
}
#[test]
fn permissions_classify_test_as_read_and_every_mutation_as_write() {
    let f = Fixture::new(false);
    let executor = BaseAutomationToolExecutor::new(f.host.clone());
    let a = ToolArgs::from_value(args());
    for name in ["base_automation_list", "base_automation_test"] {
        assert_eq!(executor.required_actions(name, &a).len(), 1);
    }
    for name in [
        "base_automation_put",
        "base_automation_run",
        "base_automation_delete",
    ] {
        let actions = executor.required_actions(name, &a);
        assert!(actions.iter().any(|(a, _)| *a == AiToolAction::WriteFile));
    }
    f.permissions
        .set_folder_permission(&f.host.root.to_string_lossy(), AiPermissionLevel::ReadOnly)
        .unwrap();
    assert_eq!(f.call("test", args())["ok"], true);
    for name in ["put", "run", "delete"] {
        assert_eq!(f.call(name, args())["ok"], false);
    }
    f.permissions
        .set_folder_permission(&f.host.root.to_string_lossy(), AiPermissionLevel::Invisible)
        .unwrap();
    assert_eq!(f.call("list", json!({}))["ok"], false);
}
