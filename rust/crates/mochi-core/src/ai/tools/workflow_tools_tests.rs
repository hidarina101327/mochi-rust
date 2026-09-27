use super::*;
use crate::ai::tools::host::HeadlessHost;
use crate::workflows::Trigger;
use serde_json::Value;
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    executor: WorkflowToolExecutor,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "mochi-workflow-agent-{}",
            crate::paths::random_base36(16)
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            executor: WorkflowToolExecutor::new(Arc::new(HeadlessHost::new(&root))),
            store: Store::open(&root).unwrap(),
            root,
        }
    }
    fn call(&self, tool: &str, args: Value) -> ToolOutcome {
        self.executor.call(tool, &ToolArgs::from_value(args))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn batch_creates_reads_and_updates_multiple_workflows_atomically() {
    let f = Fixture::new();
    let mut a = Workflow::blank();
    let b = Workflow::blank();
    let result = f
        .call(
            "workflow_batch",
            json!({"operations":[{"op":"save","definition":a},{"op":"save","definition":b}]}),
        )
        .unwrap();
    assert_eq!(result["applied"], true);
    let read = f.call("workflow_get", json!({"ids":[a.id,b.id]})).unwrap();
    assert_eq!(read["workflows"].as_array().unwrap().len(), 2);
    assert_eq!(read["workflows"][0]["revision"], 1);
    let old = a.clone();
    a.name = "改名后的工作流".into();
    let error = f.call("workflow_batch", json!({"operations":[{"op":"save","definition":a,"revision":1},{"op":"delete","id":b.id,"revision":99}]})).unwrap_err();
    assert!(error.contains("第 2 项"));
    assert_eq!(f.store.get(&a.id).unwrap().definition, old);
    assert_eq!(f.store.get(&a.id).unwrap().revision, 1);
    assert!(f.store.get(&b.id).is_ok());
    f.call("workflow_batch", json!({"operations":[{"op":"save","definition":a,"revision":1},{"op":"delete","id":b.id,"revision":1}]})).unwrap();
    assert_eq!(f.store.get(&a.id).unwrap().revision, 2);
    assert!(f.store.get(&b.id).is_err());
}

#[test]
fn batch_schedule_and_folder_management_preserve_revision_rules() {
    let f = Fixture::new();
    let mut a = Workflow::blank();
    a.trigger = Trigger::Interval { minutes: 30 };
    let folder = f
        .call("workflow_folders", json!({"action":"save","name":"日报"}))
        .unwrap();
    f.call(
        "workflow_batch",
        json!({"operations":[
            {"op":"save","definition":a},
            {"op":"set_schedule","id":a.id,"revision":1,"enabled":true},
            {"op":"move","id":a.id,"revision":1,"folder_id":folder["id"]}
        ]}),
    )
    .unwrap();
    assert!(f.store.get(&a.id).unwrap().enabled);
    let list = f.call("workflow_list", json!({})).unwrap();
    assert_eq!(list["workflows"][0]["folder_id"], folder["id"]);
    assert_eq!(list["folders"][0]["name"], "日报");
    a.name = "新的名称".into();
    f.call("workflow_save", json!({"definition":a,"revision":1}))
        .unwrap();
    assert!(f.store.get(&a.id).unwrap().enabled);
    assert!(f
        .call(
            "workflow_set_schedule",
            json!({"id":a.id,"revision":1,"enabled":false})
        )
        .is_err());
    f.call(
        "workflow_set_schedule",
        json!({"id":a.id,"revision":2,"enabled":false}),
    )
    .unwrap();
    assert!(!f.store.get(&a.id).unwrap().enabled);
    f.call(
        "workflow_folders",
        json!({"action":"delete","id":folder["id"]}),
    )
    .unwrap();
    assert_eq!(f.store.summaries().unwrap()[0].folder_id, None);
}

#[test]
fn running_flow_rejects_entire_delete_batch_and_retains_history() {
    let f = Fixture::new();
    let a = Workflow::blank();
    let b = Workflow::blank();
    f.store.save(&a, None).unwrap();
    f.store.save(&b, None).unwrap();
    let run = f.call("workflow_run", json!({"id":b.id})).unwrap();
    assert!(f.call("workflow_batch", json!({"operations":[{"op":"delete","id":a.id,"revision":1},{"op":"delete","id":b.id,"revision":1}]})).is_err());
    assert!(f.store.get(&a.id).is_ok());
    assert!(f.store.get(&b.id).is_ok());
    assert_eq!(
        f.call("workflow_history", json!({"runId":run["runId"]}))
            .unwrap()["status"],
        "queued"
    );
    f.call("workflow_delete", json!({"id":a.id,"revision":1}))
        .unwrap();
    assert!(f.store.get(&a.id).is_err());
}

#[test]
fn invalid_or_unknown_batch_operations_do_not_write() {
    let f = Fixture::new();
    let a = Workflow::blank();
    for operations in [
        json!([]),
        json!([{"op":"unknown"}]),
        json!([
            {"op":"save","definition":a}, {"op":"set_schedule","id":a.id,"revision":1,"enabled":true}
        ]),
        json!([{ "op":"save", "definition":a, "typo":true }]),
    ] {
        assert!(f
            .call("workflow_batch", json!({"operations":operations}))
            .is_err());
        assert!(f.store.list().unwrap().is_empty());
    }
    assert!(f.call("workflow_get", json!({"ids":[]})).is_err());
    assert!(f
        .call("workflow_get", json!({"ids":["x"],"id":"y"}))
        .is_err());
}
