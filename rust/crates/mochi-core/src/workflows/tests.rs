use super::*;
use serde_json::{json, Value};

#[test]
fn workflow_unread_is_opt_in_and_only_successful_documents_are_marked() {
    let fx = Fixture::new();
    let host = fx.host();
    let mut flow = linear(&["start", "file_write", "end"]);
    flow.nodes[1].inputs = json!({"path":"reports/new.md","content":"# 新文档"});
    let result = fx.run(&flow, &host);
    assert_eq!(result.status, "succeeded");
    let unread = crate::document_unread::Store::new(&fx.root);
    assert!(unread.snapshot().unwrap().is_empty());
    flow.nodes[1].config = json!({"overwrite":true,"mark_unread":true});
    flow.id = new_id("unread-update");
    let result = fx.run(&flow, &host);
    assert_eq!(result.status, "succeeded");
    let path = fx.root.join("reports/new.md");
    let state = unread.snapshot().unwrap();
    assert!(state.has(&path));
    unread
        .read(&path, &state.documents[&crate::document_unread::key(&path)])
        .unwrap();
    flow.nodes[1].config["overwrite"] = json!(false);
    flow.id = new_id("unread-failed");
    assert_eq!(fx.run(&flow, &host).status, "failed");
    assert!(unread.snapshot().unwrap().is_empty());
}

#[test]
fn workflow_folders_persist_without_changing_execution_or_revision() {
    let fixture = Fixture::new();
    let flow = Workflow::blank();
    let saved = fixture.approved(&flow);
    let folder = fixture.store.save_folder(None, "每日工作").unwrap();
    assert!(fixture.store.save_folder(None, "每日工作").is_err());
    assert!(fixture.store.save_folder(None, "  ").is_err());
    fixture
        .store
        .move_to_folder(&flow.id, Some(&folder.id))
        .unwrap();
    let store = Store::open(&fixture.root).unwrap();
    assert_eq!(
        store.summaries().unwrap()[0].folder_id.as_deref(),
        Some(folder.id.as_str())
    );
    assert_eq!(store.get(&flow.id).unwrap().revision, saved.revision);
    assert!(store.get(&flow.id).unwrap().approved);
    store.save_folder(Some(&folder.id), "日报").unwrap();
    assert_eq!(store.folders().unwrap()[0].name, "日报");
    assert!(store.move_to_folder(&flow.id, Some("missing")).is_err());
    store.delete_folder(&folder.id).unwrap();
    assert!(store.folders().unwrap().is_empty());
    assert_eq!(store.summaries().unwrap()[0].folder_id, None);
    assert_eq!(store.get(&flow.id).unwrap().definition, flow);
}

#[test]
fn workflow_input_wire_passes_typed_start_output_to_named_target_input() {
    let fixture = Fixture::new();
    let mut flow = Workflow::blank();
    flow.defaults = json!({"a": [1, 2], "name": "中文"});
    flow.nodes[1].inputs =
        json!({"a": "$nodes.start.output.a", "name": "$nodes.start.output.name"});
    let run = fixture.run(&flow, &fixture.host());
    assert_eq!(run.status, "succeeded");
    assert_eq!(run.output, flow.defaults);
    assert_eq!(run.nodes["start"].output, flow.defaults);
}

#[test]
fn workflow_start_keeps_explicit_input_fields_when_exposing_defaults() {
    let fixture = Fixture::new();
    let mut flow = Workflow::blank();
    flow.defaults = json!({"name": "日报", "count": 2});
    flow.nodes[0].inputs = json!({"title": "$input.name", "count": 3});
    flow.nodes[1].inputs = json!({"result": "$nodes.start.output"});
    let run = fixture.run(&flow, &fixture.host());
    assert_eq!(run.status, "succeeded");
    assert_eq!(
        run.output["result"],
        json!({"name": "日报", "count": 3, "title": "日报"})
    );
}
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

struct Fixture {
    root: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(new_id("mochi-workflow-test"));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::open(&root).unwrap();
        Self { root, store }
    }
    fn approved(&self, flow: &Workflow) -> SavedWorkflow {
        let saved = self.store.save(flow, None).unwrap();
        self.store.approve(&flow.id, saved.revision, true).unwrap();
        self.store.get(&flow.id).unwrap()
    }
    fn run(&self, flow: &Workflow, host: &dyn NodeHost) -> Run {
        self.approved(flow);
        self.store
            .enqueue(&flow.id, json!({}), "test", None)
            .unwrap()
            .unwrap();
        execute(
            &self.store,
            self.store.claim().unwrap().unwrap(),
            host,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }
    fn host(&self) -> native_host::NativeHost {
        native_host::NativeHost::new(
            &self.root,
            Arc::new(crate::settings::SettingsService::new(Some(
                self.root.join("settings.json"),
            ))),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert!(self
            .root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mochi-workflow-test_"));
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
struct Echo;
impl NodeHost for Echo {
    fn call(&self, _: &Node, input: &Value, _: Arc<AtomicBool>, _: &str) -> Result<Value> {
        Ok(input.clone())
    }
}
fn linear(kinds: &[&str]) -> Workflow {
    let mut flow = Workflow::blank();
    flow.nodes.clear();
    flow.edges.clear();
    for (i, kind) in kinds.iter().enumerate() {
        flow.nodes
            .push(Node::new(&format!("n{i}"), kind, i as f32 * 260., 80.));
        if i > 0 {
            flow.edges.push(Edge {
                id: format!("e{i}"),
                source: format!("n{}", i - 1),
                target: format!("n{i}"),
                source_handle: None,
            });
        }
    }
    flow
}
#[test]
fn graph_roundtrip_and_catalog() {
    let flow = Workflow::blank();
    assert_eq!(validate(&flow).unwrap(), vec!["start", "end"]);
    assert_eq!(
        serde_json::from_str::<Workflow>(&serde_json::to_string(&flow).unwrap()).unwrap(),
        flow
    );
    assert_eq!(
        catalog::describe()["kinds"].as_array().unwrap().len(),
        catalog::KINDS.len()
    );
}
#[test]
fn rejects_cycle_unreachable_duplicate_and_missing_edge() {
    let mut f = linear(&["start", "json", "json", "end"]);
    f.edges.push(Edge {
        id: "cycle".into(),
        source: "n2".into(),
        target: "n1".into(),
        source_handle: None,
    });
    assert!(validate(&f).is_err());
    f.edges.pop();
    f.edges.remove(0);
    assert!(validate(&f).is_err());
    f = Workflow::blank();
    f.nodes[1].id = "start".into();
    assert!(validate(&f).is_err());
}
#[test]
fn validates_ancestor_references_and_script_source() {
    let mut f = linear(&["start", "json", "end"]);
    f.nodes[1].inputs = json!({"x":"$nodes.n2.output.text"});
    assert!(validate(&f).is_err());
    f.nodes[1].inputs = json!({"x":"$nodes.n0.output.text"});
    assert!(validate(&f).is_ok());
    f.nodes[1].kind = "script".into();
    f.nodes[1].config = json!({"code":"{{ $input.code }}"});
    assert!(validate(&f).is_err());
}
#[test]
fn typed_refs_templates_unicode_and_missing_values() {
    let context = json!({"input":{"items":[1,"中文"],"ok":true}});
    assert_eq!(
        resolve(&json!("$input.items"), &context).unwrap(),
        json!([1, "中文"])
    );
    assert_eq!(
        resolve(&json!("值 {{ $input.items.1 }}"), &context).unwrap(),
        json!("值 中文")
    );
    assert!(resolve(&json!("$input.nope"), &context).is_err());
    assert!(resolve(&json!("{{ $input.ok"), &context).is_err());
}
#[test]
fn import_is_trusted_and_cas_preserves_other_editor() {
    let fx = Fixture::new();
    let mut f = Workflow::blank();
    let s = fx.approved(&f);
    let imported = fx
        .store
        .import(&serde_json::to_string(&f).unwrap())
        .unwrap();
    assert_ne!(imported.definition.id, f.id);
    assert!(imported.approved && !imported.enabled);
    assert!(fx
        .store
        .enqueue(&imported.definition.id, json!({}), "test", None)
        .unwrap()
        .is_some());
    f.name = "改名".into();
    let s2 = fx.store.save(&f, Some(s.revision)).unwrap();
    assert!(s2.approved);
    assert!(fx.store.save(&f, Some(s.revision)).is_err());
    f.nodes[1].inputs = json!({"result":"新内容"});
    let s3 = fx.store.save(&f, Some(s2.revision)).unwrap();
    assert!(s3.approved && !s3.enabled);
}
#[test]
fn semantic_edits_keep_schedules_enabled_until_explicitly_paused() {
    let fx = Fixture::new();
    let mut f = Workflow::blank();
    f.trigger = Trigger::Interval { minutes: 60 };
    let s = fx.approved(&f);
    f.nodes[1].position.x += 30.;
    let s = fx.store.save(&f, Some(s.revision)).unwrap();
    assert!(s.approved);
    f.nodes[1].inputs = json!({"result":"edited"});
    let s = fx.store.save(&f, Some(s.revision)).unwrap();
    assert!(s.approved && s.enabled);
    runtime::schedule(&fx.store, now()).unwrap();
    assert!(fx.store.claim().unwrap().is_some());
    fx.store.pause(&f.id).unwrap();
    assert!(!fx.store.get(&f.id).unwrap().enabled);
    assert!(fx
        .store
        .enqueue(&f.id, json!({}), "schedule", Some("later"))
        .is_err());
}

#[test]
fn legacy_unapproved_workflows_run_without_reimporting_or_approval() {
    let fx = Fixture::new();
    let f = Workflow::blank();
    fx.store.save(&f, None).unwrap();
    let db =
        rusqlite::Connection::open(fx.root.join(".mochi/workflows/workflows.sqlite3")).unwrap();
    db.execute("UPDATE workflows SET approved_hash=NULL", [])
        .unwrap();
    assert!(fx.store.get(&f.id).unwrap().approved);
    assert!(fx.store.list().unwrap()[0].approved);
    assert!(fx.store.summaries().unwrap()[0].approved);
    fx.store
        .enqueue(&f.id, json!({}), "test", None)
        .unwrap()
        .unwrap();
    let run = execute(
        &fx.store,
        fx.store.claim().unwrap().unwrap(),
        &Echo,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(run.status, "succeeded");
}
#[test]
fn runs_persist_inputs_outputs_snapshot_and_history() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "json", "end"]);
    f.nodes[1].inputs = json!({"text":"hello"});
    f.nodes[2].inputs = json!({"result":"$nodes.n1.output.text"});
    let r = fx.run(&f, &Echo);
    assert_eq!(r.status, "succeeded");
    assert_eq!(r.output, json!({"result":"hello"}));
    assert_eq!(fx.store.run(&r.id).unwrap().definition, f);
    assert_eq!(fx.store.history(&f.id).unwrap().len(), 1);
    assert_eq!(r.nodes["n1"].attempts, 1);
}
#[test]
fn branches_skip_unused_path_and_merge_after_decisions() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "condition", "json", "json", "end"]);
    f.nodes[1].inputs = json!({"left":1,"right":1});
    f.nodes[2].inputs = json!({"side":"yes"});
    f.nodes[3].inputs = json!({"side":"no"});
    f.edges = vec![
        Edge {
            id: "a".into(),
            source: "n0".into(),
            target: "n1".into(),
            source_handle: None,
        },
        Edge {
            id: "b".into(),
            source: "n1".into(),
            target: "n2".into(),
            source_handle: Some("true".into()),
        },
        Edge {
            id: "c".into(),
            source: "n1".into(),
            target: "n3".into(),
            source_handle: Some("false".into()),
        },
        Edge {
            id: "d".into(),
            source: "n2".into(),
            target: "n4".into(),
            source_handle: None,
        },
        Edge {
            id: "e".into(),
            source: "n3".into(),
            target: "n4".into(),
            source_handle: None,
        },
    ];
    let r = fx.run(&f, &Echo);
    assert_eq!(r.nodes["n2"].status, "succeeded");
    assert_eq!(r.nodes["n3"].status, "skipped");
    assert_eq!(r.nodes["n4"].status, "succeeded");
}
#[test]
fn foreach_preserves_order_and_output_types() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "json", "end"]);
    f.defaults = json!({"items":[3,1,2]});
    f.nodes[1].for_each = Some("$input.items".into());
    f.nodes[1].inputs = json!({"value":"$item"});
    let r = fx.run(&f, &Echo);
    assert_eq!(
        r.nodes["n1"].output,
        json!({"items":[{"value":3},{"value":1},{"value":2}]})
    );
}
#[test]
fn missing_variable_fails_node_before_side_effect() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "file_write", "end"]);
    f.nodes[1].inputs = json!({"path":"foo.md","content":"$input.missing"});
    let r = fx.run(&f, &fx.host());
    assert_eq!(r.status, "failed");
    assert!(!fx.root.join("foo.md").exists());
    assert_eq!(r.nodes["n1"].status, "failed");
}
#[test]
fn cancel_queued_has_no_side_effect() {
    let fx = Fixture::new();
    let f = Workflow::blank();
    fx.approved(&f);
    let r = fx
        .store
        .enqueue(&f.id, json!({}), "test", None)
        .unwrap()
        .unwrap();
    fx.store.cancel(&r.id).unwrap();
    let r = execute(
        &fx.store,
        fx.store.claim().unwrap().unwrap(),
        &Echo,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(r.status, "cancelled");
}
#[test]
fn scheduled_slot_and_active_run_are_unique() {
    let fx = Fixture::new();
    let mut f = Workflow::blank();
    f.trigger = Trigger::Daily {
        time: "09:00".into(),
        utc_offset_minutes: 480,
    };
    fx.approved(&f);
    let a = fx
        .store
        .enqueue(&f.id, json!({}), "schedule", Some("day:2026-09-20"))
        .unwrap();
    assert!(a.is_some());
    assert!(fx
        .store
        .enqueue(&f.id, json!({}), "schedule", Some("day:2026-09-20"))
        .unwrap()
        .is_none());
    assert!(fx
        .store
        .enqueue(&f.id, json!({}), "manual", None)
        .unwrap()
        .is_none());
    execute(
        &fx.store,
        fx.store.claim().unwrap().unwrap(),
        &Echo,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(fx
        .store
        .enqueue(&f.id, json!({}), "schedule", Some("day:2026-09-20"))
        .unwrap()
        .is_none());
}
#[test]
fn daily_weekly_timezone_and_interval() {
    let stamp = chrono::DateTime::parse_from_rfc3339("2026-09-20T02:00:00Z")
        .unwrap()
        .timestamp_millis();
    assert_eq!(
        due_slot(
            &Trigger::Daily {
                time: "09:00".into(),
                utc_offset_minutes: 480
            },
            stamp
        ),
        Some("day:2026-09-20".into())
    );
    assert!(due_slot(
        &Trigger::Daily {
            time: "11:00".into(),
            utc_offset_minutes: 480
        },
        stamp
    )
    .is_none());
    assert!(due_slot(
        &Trigger::Weekly {
            time: "09:00".into(),
            utc_offset_minutes: 480,
            weekdays: vec![1]
        },
        stamp
    )
    .is_none());
    assert!(due_slot(&Trigger::Interval { minutes: 0 }, stamp).is_none());
}
#[test]
fn write_append_read_notify_and_chart_real_services() {
    let fx = Fixture::new();
    let host = fx.host();
    let mut f = linear(&[
        "start",
        "chart",
        "file_write",
        "file_write",
        "document_append",
        "file_read",
        "notify",
        "end",
    ]);
    f.nodes[2].inputs = json!({"path":"reports/chart.svg","content":"$nodes.n1.output.svg"});
    f.nodes[3].inputs = json!({"path":"reports/daily.mc","content":"# 日报"});
    f.nodes[4].inputs = json!({"path":"reports/daily.mc","content":"测试追加"});
    f.nodes[5].inputs = json!({"path":"reports/daily.mc"});
    let r = fx.run(&f, &host);
    assert_eq!(r.status, "succeeded", "{:?}", r.error);
    assert!(std::fs::read_to_string(fx.root.join("reports/daily.mc"))
        .unwrap()
        .contains("测试追加"));
    assert!(std::fs::read_to_string(fx.root.join("reports/chart.svg"))
        .unwrap()
        .contains("<svg"));
    assert_eq!(fx.store.take_notices().unwrap().len(), 2);
    assert!(fx.store.take_notices().unwrap().is_empty());
}
#[test]
fn trusted_files_use_os_paths_despite_unsaved_buffers() {
    let fx = Fixture::new();
    let host = fx.host();
    let path = fx.root.join("dirty.md");
    std::fs::write(&path, "before").unwrap();
    host.blocked_paths.lock().unwrap().push(path.clone());
    let mut node = Node::new("w", "file_write", 0., 0.);
    node.config = json!({"overwrite":true});
    let cancel = Arc::new(AtomicBool::new(false));
    host.call(
        &node,
        &json!({"path":"dirty.md","content":"after"}),
        cancel.clone(),
        "test",
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "after");
    let workspace = fx.root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let inner = native_host::NativeHost::new(&workspace, host.settings.clone()).unwrap();
    inner
        .call(
            &node,
            &json!({"path":"../escape.md","content":"after"}),
            cancel.clone(),
            "test",
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(fx.root.join("escape.md")).unwrap(),
        "after"
    );
    host.call(
        &node,
        &json!({"path":".mochi/trusted-test.txt","content":"after"}),
        cancel,
        "test",
    )
    .unwrap();
    // 工作流能力不会放松普通的 AI 文件访问限制。
    let ordinary = crate::ai::tools::host::HeadlessHost::new(&workspace);
    assert!(crate::ai::tools::host::resolve_workspace_path(
        &ordinary,
        Some("../escape.md"),
        Default::default()
    )
    .is_err());
}
#[test]
fn input_schema_required_and_types() {
    assert!(
        validation::validate_input(&json!({}), &json!({"type":"object","required":["name"]}))
            .is_err()
    );
    assert!(validation::validate_input(
        &json!({"name":1}),
        &json!({"properties":{"name":{"type":"string"}}})
    )
    .is_err());
}
#[test]
fn mutations_cannot_retry() {
    let mut f = linear(&["start", "document_append", "end"]);
    f.nodes[1].retries = 1;
    assert!(validate(&f).is_err());
}
#[test]
fn failed_node_continue_is_partial_not_success() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "json", "end"]);
    f.nodes[1].config = json!({"operation":"parse"});
    f.nodes[1].inputs = json!({"text":"bad json"});
    f.nodes[1].on_error = "continue".into();
    let r = fx.run(&f, &Echo);
    assert_eq!(r.status, "partial");
    assert_eq!(r.nodes["n2"].status, "succeeded");
}
#[test]
fn trusted_http_can_access_local_services_and_rejects_non_http_schemes() {
    use std::io::{Read, Write};
    let fx = Fixture::new();
    let n = Node::new("http", "http", 0., 0.);
    let cancel = Arc::new(AtomicBool::new(false));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        // 就算 TCP 把响应头拆到多个包里，也要读完整再处理。
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() <= 4096, "request header exceeds test limit");
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nlocal works",
            )
            .unwrap();
    });
    let result = fx
        .host()
        .call(
            &n,
            &json!({"url":format!("http://{address}/")}),
            cancel.clone(),
            "test",
        )
        .unwrap();
    server.join().unwrap();
    assert_eq!(result["text"], "local works");
    assert!(fx
        .host()
        .call(&n, &json!({"url":"file:///C:/secret"}), cancel, "test")
        .is_err());
}
#[test]
fn user_cancellation_is_observed_by_host() {
    let fx = Fixture::new();
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(fx
        .host()
        .call(
            &Node::new("w", "notify", 0., 0.),
            &json!({"title":"x"}),
            cancel,
            "test"
        )
        .is_err());
}

mod integration;
mod lifecycle;
