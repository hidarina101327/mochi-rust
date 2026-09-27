use super::*;

#[test]
fn queued_snapshot_runs_original_definition_after_semantic_edit() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "file_write", "end"]);
    f.nodes[1].inputs = json!({"path":"report.md","content":"original"});
    let saved = fx.approved(&f);
    fx.store.enqueue(&f.id, json!({}), "test", None).unwrap();
    f.nodes[1].inputs["content"] = json!("changed");
    fx.store.save(&f, Some(saved.revision)).unwrap();
    let run = execute(
        &fx.store,
        fx.store.claim().unwrap().unwrap(),
        &fx.host(),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(run.status, "succeeded", "{:?}", run.error);
    assert_eq!(
        std::fs::read_to_string(fx.root.join("report.md")).unwrap(),
        "original"
    );
    assert_eq!(
        fx.store.get(&f.id).unwrap().definition.nodes[1].inputs["content"],
        "changed"
    );
}

#[test]
fn foreach_failure_retains_completed_outputs() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "json", "end"]);
    f.defaults = json!({"items":["{\"count\":1}", "bad json", "{}"]});
    f.nodes[1].for_each = Some("$input.items".into());
    f.nodes[1].inputs = json!({"text":"$item"});
    f.nodes[1].config = json!({"operation":"parse"});
    let run = fx.run(&f, &Echo);
    assert_eq!(run.status, "failed");
    assert_eq!(
        run.nodes["n1"].output,
        json!({"items":[{"count":1}],"completed":1})
    );
    assert!(run.nodes["n1"].error.as_ref().unwrap().contains("第 2 项"));
    assert_eq!(
        fx.store.run(&run.id).unwrap().nodes["n1"].output,
        run.nodes["n1"].output
    );
}

#[test]
fn separate_worker_does_not_require_saving_editor_buffers() {
    let fx = Fixture::new();
    let target = fx.root.join("unsaved.md");
    std::fs::write(&target, "before").unwrap();
    fx.store.set_dirty_paths(&[target.clone()]).unwrap();
    let host = fx.host();
    let mut node = Node::new("write", "file_write", 0., 0.);
    node.config = json!({"overwrite":true});
    let input = json!({"path":"unsaved.md","content":"after"});
    host.call(&node, &input, Arc::new(AtomicBool::new(false)), "test")
        .unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
}

#[test]
fn both_templates_write_reports_and_chart_with_mock_external_services() {
    struct MockExternal(native_host::NativeHost);
    impl NodeHost for MockExternal {
        fn call(
            &self,
            node: &Node,
            input: &Value,
            cancel: Arc<AtomicBool>,
            id: &str,
        ) -> Result<Value> {
            match node.kind.as_str() {
                "http" => Ok(json!({"text":"今日完成 3 项任务","url":input["url"]})),
                "ai" => Ok(
                    json!({"text":"# 日报\n今日完成 3 项任务","data":{"report":"# 周报\n已核对本周任务","labels":["任务"],"values":[3]}}),
                ),
                _ => self.0.call(node, input, cancel, id),
            }
        }
    }
    let fx = Fixture::new();
    let host = MockExternal(fx.host());
    let daily = fx.run(&templates::daily_web(), &host);
    assert_eq!(daily.status, "succeeded", "{:?}", daily.error);
    assert!(
        std::fs::read_to_string(daily.output["path"].as_str().unwrap())
            .unwrap()
            .contains("3 项任务")
    );
    let weekly = fx.run(&templates::weekly_documents(), &host);
    assert_eq!(weekly.status, "succeeded", "{:?}", weekly.error);
    let report = std::fs::read_to_string(weekly.output["report"].as_str().unwrap()).unwrap();
    assert!(report.contains("![本周统计]"));
    assert!(
        std::fs::read_to_string(weekly.output["chart"].as_str().unwrap())
            .unwrap()
            .contains("<svg")
    );
    assert_eq!(fx.store.summaries().unwrap().len(), 2);
    assert!(fx.store.take_notices().unwrap().len() >= 2);
}

#[test]
fn monitored_execution_observes_store_cancellation_during_node() {
    struct WaitForCancellation(std::sync::mpsc::Sender<()>);
    impl NodeHost for WaitForCancellation {
        fn call(&self, _: &Node, _: &Value, cancel: Arc<AtomicBool>, _: &str) -> Result<Value> {
            self.0.send(()).unwrap();
            let start = std::time::Instant::now();
            while !cancel.load(Ordering::Relaxed) && start.elapsed().as_secs() < 5 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(cancel.load(Ordering::Relaxed));
            Err("cancelled".into())
        }
    }
    let fx = Fixture::new();
    let f = linear(&["start", "notify", "end"]);
    fx.approved(&f);
    fx.store.enqueue(&f.id, json!({}), "test", None).unwrap();
    let run = fx.store.claim().unwrap().unwrap();
    let store = fx.store.clone();
    let id = run.id.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        runtime::execute_monitored(
            &store,
            run,
            &WaitForCancellation(tx),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    });
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    fx.store.cancel(&id).unwrap();
    assert_eq!(worker.join().unwrap().status, "cancelled");
}

#[test]
fn whole_node_output_is_a_typed_reference() {
    let fx = Fixture::new();
    let mut f = linear(&["start", "json", "end"]);
    f.nodes[1].inputs = json!({"answer":[1,2]});
    f.nodes[2].inputs = json!({"result":"$nodes.n1.output"});
    let run = fx.run(&f, &Echo);
    assert_eq!(run.output, json!({"result":{"answer":[1,2]}}));
}
