use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Instant;

fn fixture() -> (App, PathBuf) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-title-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.shell.open_workspace(&root, || {}).unwrap();
    app.ai_session_service().unwrap().initialize().unwrap();
    app.ai_new_session();
    app.ai.panel.follow_up_frequency = mochi_core::ai::FollowUpFrequency::Never;
    app.ai
        .panel
        .active
        .as_mut()
        .unwrap()
        .messages
        .push(AiStoredMessage::new(
            "user",
            "我想准备求职，如何安排未来三个月的算法练习？",
        ));
    assert!(app.ai_persist_active());
    (app, root)
}

fn pending(app: &mut App, root: &Path) -> Target {
    let mut reply =
        AiStoredMessage::new("assistant", "先复习基础，再按专题练习，最后进行模拟面试。");
    reply.set("id", serde_json::json!("reply-1"));
    reply.set(STATUS_KEY, serde_json::json!("pending"));
    let target = Target {
        root: root.into(),
        session_id: app.ai.panel.active.as_ref().unwrap().id.clone(),
        message_id: "reply-1".into(),
        revision: compute_response_revision("reply-1", reply.content()),
        original_title: "新会话".into(),
    };
    app.ai.panel.active.as_mut().unwrap().messages.push(reply);
    assert!(app.ai_persist_active());
    target
}

#[test]
fn first_reply_generates_and_persists_a_summary_through_the_completion_handler() {
    let (mut app, root) = fixture();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        start.elapsed() < Duration::from_secs(10),
                        "title request never arrived"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let (body_start, length) = loop {
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break (end + 4, length);
            }
        };
        while bytes.len() < body_start + length {
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
        }
        let request: serde_json::Value =
            serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap();
        let body = serde_json::json!({"choices":[{"message":{"role":"assistant","content":"算法求职学习计划"},"finish_reason":"stop"}]}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        request
    });
    app.settings
        .set(ai_runtime::KEY_BASE_URL, &format!("http://{address}/v1"));
    app.settings.set(ai_runtime::KEY_MODEL, "test-model");
    app.ai.panel.streaming = Some(assistant::Streaming::default());
    let (tx, rx) = mpsc::channel();
    app.ai.run = Some(ai_runtime::RunHandle {
        rx,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    tx.send(RunEvent::Done {
        content: "先复习基础，再按专题练习，最后进行模拟面试。".into(),
        transcript: Vec::new(),
        trace: Vec::new(),
        finish_reason: "stop".into(),
        usage: serde_json::json!({}),
        usage_source: "provider".into(),
    })
    .unwrap();
    app.take_ai_events();
    assert_eq!(app.ai.panel.active.as_ref().unwrap().title, "新会话");
    assert_eq!(app.ai.title_jobs.len(), 1);
    let start = Instant::now();
    while !app.ai.title_jobs.is_empty() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
        app.take_ai_events();
    }
    let request = server.join().unwrap();
    assert_eq!(request["stream"], false);
    assert!(request
        .get("tools")
        .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)));
    assert!(request["messages"][1]["content"]
        .as_str()
        .unwrap()
        .contains("三个月"));
    let conversation = app.ai.panel.active.as_ref().unwrap();
    assert_eq!(conversation.title, "算法求职学习计划");
    let id = conversation.id.clone();
    let svc = app.ai_session_service().unwrap();
    assert_eq!(svc.load_session(&id).unwrap().title, conversation.title);
    assert_eq!(svc.load_index().sessions[0].title, conversation.title);
    assert_eq!(
        conversation.messages[1]
            .get(STATUS_KEY)
            .and_then(|v| v.as_str()),
        Some("ready")
    );
    app.start_title_job(1);
    assert!(app.ai.title_jobs.is_empty());
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn late_title_updates_its_own_session_and_preserves_new_messages() {
    let (mut app, root) = fixture();
    let target = pending(&mut app, &root);
    app.ai
        .panel
        .active
        .as_mut()
        .unwrap()
        .messages
        .push(AiStoredMessage::new("user", "每天能学两小时"));
    assert!(app.ai_persist_active());
    app.ai_new_session();
    let new_id = app.ai.panel.active.as_ref().unwrap().id.clone();
    app.apply_session_title(&target, Some("算法学习计划".into()))
        .unwrap();
    assert_eq!(app.ai.panel.active.as_ref().unwrap().id, new_id);
    assert_eq!(app.ai.panel.active.as_ref().unwrap().title, "新会话");
    let saved = app
        .ai_session_service()
        .unwrap()
        .load_session(&target.session_id)
        .unwrap();
    assert_eq!(saved.title, "算法学习计划");
    assert_eq!(saved.messages.len(), 3);
    assert_eq!(saved.messages[2].content(), "每天能学两小时");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_or_failed_titles_never_overwrite_renamed_cleared_or_deleted_sessions() {
    for case in ["renamed", "cleared", "edited", "deleted", "failed"] {
        let (mut app, root) = fixture();
        let target = pending(&mut app, &root);
        match case {
            "renamed" => {
                app.ai.panel.active.as_mut().unwrap().title = "我的计划".into();
                assert!(app.ai_persist_active());
            }
            "cleared" => app.ai_clear_session(),
            "edited" => {
                app.ai.panel.active.as_mut().unwrap().messages[1]
                    .set("content", serde_json::json!("已修改"));
                assert!(app.ai_persist_active());
            }
            "deleted" => app.ai_delete_session(&target.session_id),
            _ => {}
        }
        app.apply_session_title(&target, (case != "failed").then(|| "自动标题".into()))
            .unwrap();
        let svc = app.ai_session_service().unwrap();
        if case == "deleted" {
            assert!(svc.load_session(&target.session_id).is_none());
            assert!(svc.load_index().sessions.is_empty());
        } else {
            let saved = svc.load_session(&target.session_id).unwrap();
            assert_eq!(
                saved.title,
                if case == "renamed" {
                    "我的计划"
                } else {
                    "新会话"
                }
            );
            if case == "failed" {
                assert_eq!(
                    saved.messages[1].get(STATUS_KEY).and_then(|v| v.as_str()),
                    Some("failed")
                );
                assert!(saved.messages[1].content().contains("模拟面试"));
            }
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn title_request_is_bounded_and_rejects_explanations() {
    assert_eq!(
        parse_title("“算法求职学习计划”"),
        Some("算法求职学习计划".into())
    );
    for invalid in ["", "新会话", "<think>思考</think>", "标题\n解释"] {
        assert_eq!(parse_title(invalid), None);
    }
    assert_eq!(parse_title(&"长".repeat(61)), None);
    let request = request(&"问".repeat(4000), &"答".repeat(6000));
    let context: serde_json::Value =
        serde_json::from_str(request.messages[1].content.as_deref().unwrap()).unwrap();
    assert_eq!(context["user"].as_str().unwrap().chars().count(), 3000);
    assert_eq!(context["assistant"].as_str().unwrap().chars().count(), 5000);
    assert!(request.tools.is_empty());
}
