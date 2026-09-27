use super::*;
use crate::ai::tools::{
    host::HeadlessHost, workflow_tools::WorkflowToolExecutor, ToolArgs, ToolExecutor,
};
use std::io::{Read, Write};

#[test]
fn schedule_node_uses_same_store_and_reference_updates() {
    let fx = Fixture::new();
    let host = fx.host();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut n = Node::new("schedule", "schedule_write", 0., 0.);
    host.call(
        &n,
        &json!({"title":"日报复核","date":"2026-09-20"}),
        cancel.clone(),
        "schedule-test",
    )
    .unwrap();
    let store = crate::agenda::AgendaStore::new(&fx.root);
    let snap = store.load().unwrap();
    assert_eq!(snap.tasks.len(), 1);
    assert_eq!(snap.tasks[0].title, "日报复核");
    assert_eq!(snap.tasks[0].due.as_deref(), Some("2026-09-20"));
    n.config = json!({"operation":"update_task"});
    let id = &snap.tasks[0].id;
    host.call(
        &n,
        &json!({"id":id,"updates":{"title":"已更新"}}),
        cancel.clone(),
        "schedule-update",
    )
    .unwrap();
    assert_eq!(store.load().unwrap().tasks[0].title, "已更新");
}
#[test]
fn base_node_creates_and_updates_a_record_via_shared_executor() {
    let fx = Fixture::new();
    let host = fx.host();
    let document = crate::base::create_base_document();
    let table = &document.tables[0];
    let field = &table.fields[0].id;
    let path = fx.root.join("records.mcb");
    std::fs::write(
        &path,
        crate::base::serialize_base_document(&document).unwrap(),
    )
    .unwrap();
    let mut n = Node::new("base", "base_write", 0., 0.);
    let cancel = Arc::new(AtomicBool::new(false));
    host.call(
        &n,
        &json!({"path":"records.mcb","tableId":table.id,"values":{field:"first"}}),
        cancel.clone(),
        "test-base",
    )
    .unwrap();
    let doc = crate::base::parse_base_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc.tables[0].records.len(), 1);
    n.config = json!({"operation":"update"});
    host.call(&n,&json!({"path":"records.mcb","tableId":table.id,"recordId":doc.tables[0].records[0].id,"values":{field:"second"}}),cancel,"test-base-2").unwrap();
    let doc = crate::base::parse_base_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc.tables[0].records[0].values[field], "second");
}
#[cfg(windows)]
#[test]
fn script_runs_with_structured_input_and_reports_exit_error() {
    let fx = Fixture::new();
    let host = fx.host();
    let mut n = Node::new("s", "script", 0., 0.);
    n.config =
        json!({"language":"cmd","summary":"本地测试输出","code":"@echo off\necho {\"count\":3}\n"});
    let result = host
        .call(
            &n,
            &json!({"count":3}),
            Arc::new(AtomicBool::new(false)),
            "script-test",
        )
        .unwrap();
    assert_eq!(result["data"]["count"], 3);
    n.config["code"] = json!("@exit /b 7");
    assert!(host
        .call(
            &n,
            &json!({}),
            Arc::new(AtomicBool::new(false)),
            "script-fail"
        )
        .is_err());
}
#[test]
fn ai_uses_existing_selected_provider_and_model() {
    assert_ai_uses_selected_provider("openai-completions");
}

#[test]
fn ai_uses_selected_anthropic_provider_and_model() {
    assert_ai_uses_selected_provider("anthropic-messages");
}

fn assert_ai_uses_selected_provider(protocol: &'static str) {
    let fx = Fixture::new();
    let host = fx.host();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    crate::ai::providers::save(
        &host.settings,
        crate::ai::models::AiProvider {
            id: "test".into(),
            name: "test".into(),
            base_url: format!("http://{address}/v1"),
            api_key: String::new(),
            model: "shared-test-model".into(),
            stream: false,
            protocol: protocol.into(),
        },
    )
    .unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            request.extend_from_slice(&chunk[..n]);
            if n == 0 {
                break;
            }
            if let Some(pos) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..pos]);
                let len = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= pos + 4 + len {
                    break;
                }
            }
        }
        let request = String::from_utf8_lossy(&request);
        let anthropic = protocol == "anthropic-messages";
        let endpoint = if anthropic {
            "/v1/messages"
        } else {
            "/v1/chat/completions"
        };
        assert!(request.starts_with(&format!("POST {endpoint} ")));
        assert!(request.contains("shared-test-model"));
        let body = if anthropic {
            assert!(request.to_ascii_lowercase().contains("anthropic-version: 2023-06-01"));
            json!({"type":"message","role":"assistant","content":[{"type":"text","text":"整理后的报告"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":5}})
        } else {
            json!({"choices":[{"message":{"content":"整理后的报告"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})
        }.to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    let result = host
        .call(
            &Node::new("ai", "ai", 0., 0.),
            &json!({"prompt":"整理本地模拟资料"}),
            Arc::new(AtomicBool::new(false)),
            "ai-test",
        )
        .unwrap();
    server.join().unwrap();
    assert_eq!(result["text"], "整理后的报告");
    assert_eq!(result["model"], "shared-test-model");
}

#[cfg(windows)]
#[test]
fn trusted_powershell_uses_external_cwd_and_process_policy_with_dirty_editors() {
    let fx = Fixture::new();
    let workspace = fx.root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let host = native_host::NativeHost::new(&workspace, fx.host().settings.clone()).unwrap();
    Store::open(&workspace)
        .unwrap()
        .set_dirty_paths(&[workspace.join("unsaved.md")])
        .unwrap();
    let mut node = Node::new("script", "script", 0., 0.);
    node.config = json!({"language":"powershell","cwd":fx.root,"code":
        "Set-Content -LiteralPath 'trusted.txt' -Value 'works'\n@{policy=[string](Get-ExecutionPolicy -Scope Process);written=(Test-Path -LiteralPath 'trusted.txt')} | ConvertTo-Json -Compress"});
    let output = host
        .call(
            &node,
            &json!({}),
            Arc::new(AtomicBool::new(false)),
            "trusted-script",
        )
        .unwrap();
    assert_eq!(output["data"]["policy"], "Bypass");
    assert_eq!(output["data"]["written"], true);
    assert!(fx.root.join("trusted.txt").exists());
}

#[test]
fn trusted_tool_nodes_apply_file_and_schedule_writes_directly() {
    let fx = Fixture::new();
    let host = fx.host();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut node = Node::new("tool", "tool", 0., 0.);
    node.config = json!({"name":"file_write"});
    let output = host
        .call(
            &node,
            &json!({"path":"tool.txt","content":"written by tool"}),
            cancel.clone(),
            "tool-test",
        )
        .unwrap();
    assert!(output.get("pendingFileOperation").is_none());
    assert_eq!(
        std::fs::read_to_string(fx.root.join("tool.txt")).unwrap(),
        "written by tool"
    );
    node.config = json!({"name":"agenda_batch"});
    let output = host
        .call(
            &node,
            &json!({"operations":[{"op":"create","kind":"task","title":"直接创建日程","dueAt":"2026-08-28"}]}),
            cancel,
            "tool-test",
        )
        .unwrap();
    assert_eq!(output["pendingScheduleDiff"]["status"], "applied");
    assert_eq!(
        crate::agenda::AgendaStore::new(&fx.root)
            .load()
            .unwrap()
            .tasks
            .len(),
        1
    );
}
#[test]
fn agent_can_save_run_and_inspect_without_approval() {
    let fx = Fixture::new();
    let tool = WorkflowToolExecutor::new(Arc::new(HeadlessHost::new(&fx.root)));
    let f = Workflow::blank();
    let saved = tool
        .call(
            "workflow_save",
            &ToolArgs::from_value(json!({"definition":f})),
        )
        .unwrap();
    assert_eq!(saved["approved"], true);
    assert!(tool
        .required_actions("workflow_run", &ToolArgs::from_value(json!({"id":f.id})))
        .is_empty());
    let run = tool
        .call("workflow_run", &ToolArgs::from_value(json!({"id":f.id})))
        .unwrap();
    let details = tool
        .call(
            "workflow_history",
            &ToolArgs::from_value(json!({"runId":run["runId"]})),
        )
        .unwrap();
    assert_eq!(details["status"], "queued");
    tool.call(
        "workflow_cancel",
        &ToolArgs::from_value(json!({"runId":run["runId"]})),
    )
    .unwrap();
    assert!(fx.store.cancelled(run["runId"].as_str().unwrap()));
}
#[test]
fn separate_connections_cannot_claim_same_job() {
    let fx = Fixture::new();
    let f = Workflow::blank();
    fx.approved(&f);
    fx.store.enqueue(&f.id, json!({}), "test", None).unwrap();
    let other = Store::open(&fx.root).unwrap();
    let a = fx.store.claim().unwrap();
    let b = other.claim().unwrap();
    assert!(a.is_some() && b.is_none());
}
#[test]
fn cancelling_a_running_script_reaches_process_runner() {
    #[cfg(windows)]
    {
        let fx = Fixture::new();
        let host = fx.host();
        let mut n = Node::new("s", "script", 0., 0.);
        n.config =
            json!({"language":"powershell","summary":"取消验证","code":"Start-Sleep -Seconds 30"});
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let start = std::time::Instant::now();
        let th = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            stop.store(true, Ordering::Relaxed);
        });
        assert!(host.call(&n, &json!({}), cancel, "cancel-test").is_err());
        th.join().unwrap();
        assert!(start.elapsed().as_secs() < 5);
    }
}
