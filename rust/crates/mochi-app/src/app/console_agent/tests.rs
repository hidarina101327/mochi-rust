use super::*;
use mochi_core::ai::permission::{AiPermissionLevel, AiPermissionService};
struct Fixture {
    app: App,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-console-test-{}",
            mochi_core::paths::random_base36(16)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("test-settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let permissions = Arc::new(AiPermissionService::new(&root));
        permissions
            .save_permissions(vec![mochi_core::domain::AiFolderPermission {
                path: root.to_string_lossy().into_owned(),
                level: "modify".into(),
                inherited: None,
            }])
            .unwrap();
        app.ai.permissions = Some(permissions.clone());
        app.ai.host = Some(Arc::new(crate::ai_runtime::AppHost::new(
            &root,
            Arc::new(std::sync::Mutex::new(Default::default())),
            permissions,
            || {},
        )));
        Self { app, root }
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.app
            .console_dispatch(HWND::default(), name, &args)
            .unwrap_or_else(|e| panic!("{name} {args}: {e:#}"))
    }
}
// 应用退出前，Shell 监视器和 Git 工作线程可能仍持有句柄。测试目录
// 必须位于操作系统临时目录中；绝不能指向用户工作区。
#[test]
fn console_comments_crud_revisions_and_corrupt_file_preservation() {
    let mut f = Fixture::new();
    std::fs::write(f.root.join("note.md"), "text").unwrap();
    let read = f.call("comments_manage", json!({"action":"list","path":"note.md"}));
    let created=f.call("comments_manage",json!({"action":"create","path":"note.md","revision":read["revision"],"data":{"content":"comment"}}));
    let id = created["value"][0]["id"].clone();
    assert!(f
        .app
        .console_dispatch(
            HWND::default(),
            "comments_manage",
            &json!({"action":"delete","path":"note.md","revision":read["revision"],"id":id})
        )
        .is_err());
    let reply=f.call("comments_manage",json!({"action":"reply","path":"note.md","revision":created["revision"],"id":id,"data":{"content":"reply"}}));
    let resolved = f.call(
        "comments_manage",
        json!({"action":"resolve","path":"note.md","revision":reply["revision"],"id":id}),
    );
    assert_eq!(resolved["value"][0]["resolved"], true);
    let deleted = f.call(
        "comments_manage",
        json!({"action":"delete","path":"note.md","revision":resolved["revision"],"id":id}),
    );
    assert!(deleted["value"].as_array().unwrap().is_empty());
    let sidecar = sidecars::comment_sidecar_path(&f.root.join("note.md").to_string_lossy());
    std::fs::write(&sidecar, "broken").unwrap();
    assert!(f.app.console_dispatch(HWND::default(),"comments_manage",&json!({"action":"create","path":"note.md","revision":deleted["revision"],"data":{"content":"x"}})).is_err());
    assert_eq!(std::fs::read_to_string(sidecar).unwrap(), "broken");
}
#[test]
fn console_content_tools_persist_and_reject_restricted_descendants() {
    let mut f = Fixture::new();
    std::fs::write(f.root.join("a.md"), "A").unwrap();
    std::fs::write(f.root.join("b.md"), "B").unwrap();
    f.call("favorites_manage", json!({"action":"add","path":"a.md"}));
    let v = f.call("favorites_manage", json!({"action":"add","path":"b.md"}));
    let paths = v["value"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>();
    let v = f.call(
        "favorites_manage",
        json!({"action":"reorder","revision":v["revision"],"data":{"paths":paths}}),
    );
    assert!(v["value"][0].as_str().unwrap().ends_with("b.md"));
    let marked = f.call(
        "unread_manage",
        json!({"action":"mark","data":{"paths":["a.md"]}}),
    );
    let token = marked.as_object().unwrap().values().next().unwrap();
    f.call(
        "unread_manage",
        json!({"action":"read","path":"a.md","data":{"token":token}}),
    );
    let t = f.call(
        "templates_manage",
        json!({"action":"create","data":{"name":"AgentTest","content":"body","group":"未分组"}}),
    );
    let path = t["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "AgentTest")
        .unwrap()["path"]
        .clone();
    let original = f.call("templates_manage", json!({"action":"get","path":path}));
    f.call("templates_manage",json!({"action":"update","path":path,"revision":original["revision"],"data":{"content":"new"}}));
    f.app
        .ai
        .permissions
        .as_ref()
        .unwrap()
        .set_folder_permission(
            &f.root.join(".mochi/templates/未分组").to_string_lossy(),
            AiPermissionLevel::ReadOnly,
        )
        .unwrap();
    assert!(f
        .app
        .console_dispatch(
            HWND::default(),
            "templates_manage",
            &json!({"action":"create","data":{"name":"Blocked","content":"x"}})
        )
        .is_err());
    let item = f.call(
        "inbox_manage",
        json!({"action":"create","data":{"content":"capture"}}),
    );
    assert!(item["id"].is_string());
    f.call(
        "inbox_manage",
        json!({"action":"update","id":item["id"],"data":{"content":"edited"}}),
    );
    f.call("inbox_manage", json!({"action":"delete","id":item["id"]}));
}
#[test]
fn console_base_batch_and_canvas_pure_delete() {
    let mut f = Fixture::new();
    let doc = mochi_core::base::create_base_document();
    std::fs::write(
        f.root.join("table.mcb"),
        mochi_core::base::serialize_base_document(&doc).unwrap(),
    )
    .unwrap();
    let v = f.call("base_manage", json!({"action":"get","path":"table.mcb"}));
    let changed=f.call("base_manage",json!({"action":"batch","path":"table.mcb","revision":v["revision"],"operations":[{"op":"table_rename","tableId":doc.tables[0].id,"name":"renamed"}]}));
    assert_eq!(changed["value"]["tables"][0]["name"], "renamed");
    let mut doc = mochi_core::canvas::CanvasDocument::empty();
    doc.add_pdf_annotation("paper.pdf", "one").unwrap();
    std::fs::write(
        f.root.join("canvas.mcanvas"),
        mochi_core::canvas::serialize(&doc).unwrap(),
    )
    .unwrap();
    let v = f.call(
        "canvas_manage",
        json!({"action":"get","path":"canvas.mcanvas"}),
    );
    f.call("canvas_manage",json!({"action":"batch","path":"canvas.mcanvas","revision":v["revision"],"operations":[{"kind":"cards","op":"delete","id":doc.cards[0].id}]}));
    assert!(mochi_core::canvas::parse(
        &std::fs::read_to_string(f.root.join("canvas.mcanvas")).unwrap()
    )
    .unwrap()
    .cards
    .is_empty());
}
#[test]
fn console_definition_and_sessions_round_trip_and_menu_protection() {
    let mut f = Fixture::new();
    let svc = mochi_core::ai::agent_config::AgentConfigService::new(&f.root);
    svc.ensure_seeds().unwrap();
    let path = svc.root().join("Agents/test.md");
    f.call("agent_definitions_manage",json!({"action":"save","path":path,"data":{"content":"---\nname: test\ntools: [console_state]\n---\nbody"}}));
    let v = f.call(
        "agent_definitions_manage",
        json!({"action":"get","path":path}),
    );
    assert!(v["value"].as_str().unwrap().contains("console_state"));
    std::fs::write(
        svc.root().join("QuickActions/protected.md"),
        "---\nagent: test\nenabled: false\n---\nmenu",
    )
    .unwrap();
    assert!(f
        .app
        .console_dispatch(
            HWND::default(),
            "agent_definitions_manage",
            &json!({"action":"delete","path":path,"revision":v["revision"]})
        )
        .is_err());
    assert!(path.exists());
    let c = f.call(
        "ai_sessions_manage",
        json!({"action":"create","data":{"title":"from tool"}}),
    );
    let meta = c["value"]["sessions"].as_array().unwrap().last().unwrap();
    let id = meta["id"].clone();
    let c=f.call("ai_sessions_manage",json!({"action":"update","id":id,"revision":c["revision"],"data":{"title":"updated","pinned":true}}));
    assert!(c["value"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == id && s["pinned"] == true));
    f.call(
        "ai_sessions_manage",
        json!({"action":"delete","id":id,"revision":c["revision"]}),
    );
}
#[test]
fn console_learning_timer_and_navigation_are_live() {
    let mut f = Fixture::new();
    std::fs::write(f.root.join("note.md"), "line1\nline2\n").unwrap();
    let state = f.call(
        "console_tabs",
        json!({"action":"locate","path":"note.md","data":{"line":2}}),
    );
    assert!(!state["value"]["tabs"].as_array().unwrap().is_empty());
    assert_eq!(f.app.shell.active().unwrap().buffer().unwrap().cursor(), 6);
    assert_eq!(
        f.call("pomodoro_manage", json!({"action":"start"}))["running"],
        true
    );
    assert!(f.app.panels.pomodoro.running);
    f.call("pomodoro_manage", json!({"action":"pause"}));
    assert!(!f.app.panels.pomodoro.running);
    f.call("pomodoro_manage", json!({"action":"reset"}));
    f.call(
        "english_manage",
        json!({"action":"add_word","data":{"word":"test","meaning":"测试"}}),
    );
    let words = f.call(
        "english_manage",
        json!({"action":"get","data":{"section":"words"}}),
    );
    assert!(words.to_string().contains("测试"));
}

#[test]
fn console_pdf_annotations_exam_and_notification_workflows() {
    let mut f = Fixture::new();
    std::fs::write(f.root.join("paper.pdf"), b"%PDF").unwrap();
    let v = f.call(
        "annotations_manage",
        json!({"action":"list","path":"paper.pdf"}),
    );
    let v=f.call("annotations_manage",json!({"action":"create","path":"paper.pdf","revision":v["revision"],"data":{"type":"rect","page":1,"x":0.1,"y":0.1,"width":0.2,"height":0.3,"color":"#ffcc00"}}));
    let id = v["value"][0]["id"].clone();
    let v=f.call("annotations_manage",json!({"action":"update","id":id,"path":"paper.pdf","revision":v["revision"],"data":{"color":"#ff0000"}}));
    assert_eq!(v["value"][0]["color"], "#ff0000");
    f.call(
        "annotations_manage",
        json!({"action":"delete","id":id,"path":"paper.pdf","revision":v["revision"]}),
    );
    std::fs::write(f.root.join("quiz.exam"),":::exam\n{\"title\":\"test\",\"questions\":[{\"id\":\"q1\",\"type\":\"judge\",\"stem\":\"yes\",\"answer\":true}]}\n:::\n").unwrap();
    f.call(
        "exam_manage",
        json!({"action":"answer","path":"quiz.exam","data":{"questionId":"q1","answer":true}}),
    );
    let v = f.call("exam_manage", json!({"action":"get","path":"quiz.exam"}));
    assert_eq!(v["answers"][0]["q1"], true);
    f.call("exam_manage", json!({"action":"reset","path":"quiz.exam"}));
    f.app.publish_notification(
        crate::ui::notifications::Category::Assistant,
        "Test",
        "Tool notification",
    );
    let v = f.call("notifications_manage", json!({"action":"list"}));
    assert!(!v["entries"].as_array().unwrap().is_empty());
    f.call("notifications_manage", json!({"action":"read"}));
    assert_eq!(
        f.call("notifications_manage", json!({"action":"list"}))["unread"],
        0
    );
}

#[test]
fn console_mappings_and_background_export_report_durable_results() {
    let mut f = Fixture::new();
    let external = std::env::temp_dir().join(format!(
        "mochi-console-external-{}",
        mochi_core::paths::random_base36(16)
    ));
    std::fs::create_dir(&external).unwrap();
    let library = f.root.join("MyLibrary");
    std::fs::create_dir(&library).unwrap();
    let v = f.call("mapped_folders_manage", json!({"action":"list"}));
    let v=f.call("mapped_folders_manage",json!({"action":"add","revision":v["revision"],"data":{"libraryPath":library,"source":external,"name":"External"}}));
    let id = v["value"]["folders"][0]["id"].clone();
    let v = f.call(
        "mapped_folders_manage",
        json!({"action":"update","revision":v["revision"],"id":id,"data":{"name":"Renamed"}}),
    );
    f.call(
        "mapped_folders_manage",
        json!({"action":"delete","revision":v["revision"],"id":id}),
    );
    assert!(external.is_dir());
    // 通过 inspect 调用真实宿主队列，但不接触正在使用的工作区数据库。
    let zip = external.join("input.zip");
    let source = external.join("source");
    std::fs::create_dir_all(source.join("empty")).unwrap();
    mochi_core::console_system::export_workspace(&source, &zip).unwrap();
    let queued = f.call("workspace_transfer", json!({"action":"inspect","path":zip}));
    assert_eq!(queued["status"], "running");
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let v = f.call(
            "workspace_transfer",
            json!({"action":"status","id":queued["jobId"]}),
        );
        if v["status"] != "running" {
            assert_eq!(v["status"], "completed", "{v}");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    std::fs::remove_dir_all(external).unwrap();
}

#[test]
fn console_host_queue_returns_real_response() {
    use mochi_core::ai::tools::host::ToolHost;
    let mut f = Fixture::new();
    let host = f.app.ai.host.clone().unwrap();
    let worker =
        std::thread::spawn(move || host.console("console_state", &json!({"action":"get"})));
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    while !worker.is_finished() && Instant::now() < deadline {
        f.app.console_agent_requests(HWND::default());
        std::thread::yield_now();
    }
    let state = worker.join().unwrap().unwrap();
    assert!(state["value"]["workspace"].is_string());
}

#[test]
fn console_split_keeps_distinct_documents_and_close_refuses_dirty_tabs() {
    let mut f = Fixture::new();
    for name in ["left.md", "right.md"] {
        std::fs::write(f.root.join(name), name).unwrap();
    }
    f.call("console_tabs", json!({"action":"open","path":"left.md"}));
    f.call(
        "console_tabs",
        json!({"action":"split","path":"right.md","data":{"ratio":0.6}}),
    );
    assert!(f.app.active_file_path().unwrap().ends_with("right.md"));
    assert!(f.app.split.other.as_ref().unwrap().ends_with("left.md"));
    f.call(
        "console_tabs",
        json!({"action":"focus_pane","data":{"right":false}}),
    );
    assert!(f.app.active_file_path().unwrap().ends_with("left.md"));
    f.app.shell.active_buffer_mut().unwrap().insert("dirty");
    assert!(f
        .app
        .console_dispatch(
            HWND::default(),
            "console_tabs",
            &json!({"action":"close","path":"left.md"})
        )
        .is_err());
}

#[test]
fn console_approval_executes_once_without_changing_folder_permissions() {
    use mochi_core::ai::session::{AiConversation, AiStoredMessage};
    let mut f = Fixture::new();
    std::fs::write(f.root.join("note.md"), "note").unwrap();
    f.app
        .ai
        .permissions
        .as_ref()
        .unwrap()
        .set_folder_permission(&f.root.to_string_lossy(), AiPermissionLevel::Suggest)
        .unwrap();
    let initial = f.call("comments_manage", json!({"action":"list","path":"note.md"}));
    let result=f.call("comments_manage",json!({"action":"create","path":"note.md","revision":initial["revision"],"data":{"content":"needs review"}}));
    assert_eq!(result["applied"], false);
    let raw = result["pendingConsoleAction"].clone();
    assert_eq!(raw["status"], "pending");
    assert!(
        sidecars::load_comments(&f.root.join("note.md").to_string_lossy())
            .comments
            .is_empty()
    );
    let mut message = AiStoredMessage::new("assistant", "proposal");
    message.set("pendingConsoleAction", raw.clone());
    f.app.ai.panel.active = Some(AiConversation {
        id: "console-review-test".into(),
        title: "review".into(),
        messages: vec![message],
        ..Default::default()
    });
    assert!(f.app.open_console_review(raw.clone()));
    f.app.apply_console_review(raw.clone(), true);
    assert_eq!(
        sidecars::load_comments(&f.root.join("note.md").to_string_lossy())
            .comments
            .len(),
        1
    );
    assert!(!f.app.console_is_reviewing());
    assert_eq!(
        f.app.ai.panel.active.as_ref().unwrap().messages[0]
            .get("pendingConsoleAction")
            .unwrap()["status"],
        "applied"
    );
    f.app.apply_console_review(raw, true);
    assert_eq!(
        sidecars::load_comments(&f.root.join("note.md").to_string_lossy())
            .comments
            .len(),
        1
    );
    assert_eq!(
        f.app
            .ai
            .permissions
            .as_ref()
            .unwrap()
            .folder_permission_level(&f.root.to_string_lossy()),
        Some(AiPermissionLevel::Suggest)
    );
}

#[test]
fn console_proposal_transcript_creates_a_real_review_card() {
    use mochi_core::ai::{models::AiMessage, session::AiConversation};
    let mut f = Fixture::new();
    f.app.ai.panel.active = Some(AiConversation {
        id: "console-card-test".into(),
        ..Default::default()
    });
    let raw = json!({"id":"proposal-1","status":"pending","workspaceRoot":f.root,"toolName":"templates_manage","summary":"创建模板","args":{"action":"create","data":{"name":"Example","content":"Body"}}});
    let messages = vec![AiMessage::tool_result(
        "call-1",
        "templates_manage",
        &json!({"ok":true,"data":{"pendingConsoleAction":raw}}).to_string(),
    )];
    assert_eq!(f.app.ai_append_pending_cards(&messages), 1);
    assert_eq!(f.app.ai_append_pending_cards(&messages), 0);
    let m = f
        .app
        .ai
        .panel
        .active
        .as_ref()
        .unwrap()
        .messages
        .last()
        .unwrap();
    assert!(m.content().contains("Example"));
    assert!(crate::ui::assistant::context::pending_card_from_message(m)
        .unwrap()
        .review
        .is_available());
    assert!(f.app.open_console_review(raw));
    assert!(f.app.dialog.is_some());
}
