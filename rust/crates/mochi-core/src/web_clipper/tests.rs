use super::*;

#[test]
fn pairing_accepts_only_exact_extension_requests() {
    let id = "abcdefghijklmnopabcdefghijklmnop";
    assert_eq!(
        pairing::extension_from_url(&format!("mochi-clipper://connect?extension={id}")).unwrap(),
        id
    );
    for url in [
        format!("https://connect?extension={id}"),
        format!("mochi-clipper://other?extension={id}"),
        format!("mochi-clipper://connect/path?extension={id}"),
        format!("mochi-clipper://connect?extension={id}&extension={id}"),
        format!("mochi-clipper://connect?extension={id}&command=anything"),
        format!("mochi-clipper://user@connect?extension={id}"),
        format!("mochi-clipper://connect?extension={id}#fragment"),
        "mochi-clipper://connect?extension=bad%22id".into(),
    ] {
        assert!(pairing::extension_from_url(&url).is_err(), "{url}");
    }
}

#[test]
fn pairing_preserves_multiple_browsers_and_revokes_only_one() {
    let (root, settings) = workspace();
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    pairing::approve(&settings, &a).unwrap();
    let peer = SettingsService::new(Some(settings.file_path().to_path_buf()));
    pairing::approve(&peer, &b).unwrap();
    pairing::approve(&settings, &a).unwrap();
    assert_eq!(
        extension_ids(&settings).unwrap(),
        vec![a.clone(), b.clone()]
    );
    let receiver = Receiver::new(Arc::clone(&settings));
    for id in [&a, &b] {
        assert_eq!(
            receiver.handle(
                &format!("chrome-extension://{id}/"),
                json!({"id":id,"version":1,"op":"ping"})
            )["ok"],
            true
        );
    }
    pairing::revoke(&peer, &a).unwrap();
    settings.reload().unwrap();
    assert!(!allowed(&settings, &format!("chrome-extension://{a}/")));
    assert!(allowed(&settings, &format!("chrome-extension://{b}/")));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn simultaneous_browser_approvals_do_not_replace_each_other() {
    let (root, settings) = workspace();
    let threads: Vec<_> = (b'a'..=b'd')
        .map(|letter| {
            let path = settings.file_path().to_path_buf();
            std::thread::spawn(move || {
                pairing::approve(
                    &SettingsService::new(Some(path)),
                    &(letter as char).to_string().repeat(32),
                )
                .unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    settings.reload().unwrap();
    assert_eq!(extension_ids(&settings).unwrap().len(), 4);
    std::fs::remove_dir_all(root).unwrap();
}
use base64::Engine;
use sha2::{Digest, Sha256};
#[test]
fn shared_protocol_contract() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../shared/web-clipper-protocol.json"
    ))
    .unwrap();
    assert_eq!(fixture["version"], VERSION);
    assert_eq!(fixture["host"], HOST);
    assert_eq!(fixture["limits"]["totalBytes"], MAX_TOTAL);
    assert_eq!(fixture["limits"]["imageBytes"], MAX_IMAGE);
    let input: ClipInput = serde_json::from_value(fixture["clip"].clone()).unwrap();
    assert_eq!(input.files[0].name, "document.md");
    #[cfg(windows)]
    {
        let mut bytes = Vec::new();
        native::write_frame(&mut bytes, &fixture["request"]).unwrap();
        assert_eq!(
            native::read_frame(&mut std::io::Cursor::new(bytes))
                .unwrap()
                .unwrap(),
            fixture["request"]
        );
        assert!(native::read_frame(&mut std::io::Cursor::new(u32::MAX.to_le_bytes())).is_err());
    }
}
fn workspace() -> (PathBuf, Arc<SettingsService>) {
    let root = std::env::temp_dir().join(format!(
        "mochi-clip-test-{}-{}",
        std::process::id(),
        crate::paths::random_base36(12)
    ));
    std::fs::create_dir_all(root.join("知识库")).unwrap();
    let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
    (root, settings)
}
fn input(root: &Path, bytes: &[u8]) -> ClipInput {
    ClipInput {
        clip_id: "test-clip-1".into(),
        workspace: workspace_token(root),
        title: "测试文章".into(),
        url: "https://example.com/article".into(),
        author: None,
        captured_at: 123456,
        mode: "article".into(),
        format: "markdown".into(),
        destination: "inbox".into(),
        library_id: None,
        folder: String::new(),
        excerpt: "摘要".into(),
        files: vec![UploadFile {
            name: "document.md".into(),
            size: bytes.len() as u64,
            sha256: hex(&Sha256::digest(bytes)),
        }],
    }
}
#[test]
fn long_article_is_complete_and_retries_are_idempotent() {
    let (root, settings) = workspace();
    let body = "中文长文章\n".repeat(10000);
    let bytes = body.as_bytes();
    let clip = input(&root, bytes);
    storage::begin(&root, clip).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    storage::chunk(&root, "test-clip-1", "document.md", 0, &data).unwrap();
    storage::chunk(&root, "test-clip-1", "document.md", 0, &data).unwrap();
    let first = storage::commit(&root, "test-clip-1", &settings).unwrap();
    assert_eq!(
        first,
        storage::commit(&root, "test-clip-1", &settings).unwrap()
    );
    let svc = crate::capture::CaptureService::new(&root);
    let items = svc.list_items("inbox");
    assert_eq!(items.len(), 1);
    assert!(items[0].content.len() < 1000);
    assert_eq!(
        std::fs::read_to_string(root.join(first["document"].as_str().unwrap())).unwrap(),
        body
    );
    let target = root.join("知识库/归档");
    std::fs::create_dir_all(&target).unwrap();
    let archived = svc.archive_to_note(&items[0].id, &target).unwrap();
    assert_eq!(std::fs::read_to_string(archived).unwrap(), body);
    assert_eq!(svc.count_inbox(), 0);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn rejects_traversal_truncation_and_changed_retry() {
    let (root, settings) = workspace();
    let mut clip = input(&root, b"hello");
    clip.files[0].name = "../document.md".into();
    assert!(storage::begin(&root, clip).is_err());
    storage::begin(&root, input(&root, b"hello")).unwrap();
    assert!(storage::commit(&root, "test-clip-1", &settings).is_err());
    assert!(storage::begin(&root, input(&root, b"changed")).is_err());
    assert!(safe_relative("C:/temp", false).is_err());
    assert!(safe_relative("../escape", false).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn concurrent_capture_instances_do_not_lose_items() {
    let (root, _) = workspace();
    let threads = (0..16)
        .map(|n| {
            let root = root.clone();
            std::thread::spawn(move || {
                crate::capture::CaptureService::new(root)
                    .add(&format!("note {n}"), "app")
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(crate::capture::CaptureService::new(&root).count_inbox(), 16);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn refuses_unbound_origin_and_wrong_workspace() {
    let (root, settings) = workspace();
    settings.set("webClipper.enabled", "true");
    settings.set("webClipper.extensionIds", &"a".repeat(32));
    let receiver = Receiver::new(settings);
    *receiver.workspace.lock().unwrap() = Some(root.clone());
    assert_eq!(
        receiver.handle(
            "https://example.com",
            json!({"version":1,"id":"1","op":"context"})
        )["ok"],
        false
    );
    assert_eq!(
        receiver.handle(
            &format!("chrome-extension://{}/", "a".repeat(32)),
            json!({"version":1,"id":"2","op":"result","workspace":"wrong","clipId":"1"})
        )["ok"],
        false
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn save(root: &Path, settings: &SettingsService, body: &[u8]) -> Value {
    storage::begin(root, input(root, body)).unwrap();
    storage::chunk(
        root,
        "test-clip-1",
        "document.md",
        0,
        &base64::engine::general_purpose::STANDARD.encode(body),
    )
    .unwrap();
    storage::commit(root, "test-clip-1", settings).unwrap()
}

#[test]
fn interrupted_publish_reuses_the_bundle_and_inbox_item() {
    let (root, settings) = workspace();
    let receipt = save(&root, &settings, b"complete article");
    std::fs::remove_file(root.join(".mochi/web-clipper/test-clip-1/receipt.json")).unwrap();
    let retried = storage::commit(&root, "test-clip-1", &settings).unwrap();
    assert_eq!(receipt, retried);
    assert_eq!(crate::capture::CaptureService::new(&root).count_inbox(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_recovers_after_directory_move_before_index_update() {
    let (root, settings) = workspace();
    let receipt = save(&root, &settings, b"recoverable article");
    let source = root.join(receipt["document"].as_str().unwrap());
    let target = root.join("知识库/archived");
    std::fs::create_dir_all(&target).unwrap();
    let bundle = target.join(source.parent().unwrap().file_name().unwrap());
    let doc = bundle.join("document.md");
    std::fs::write(
        root.join(".mochi/web-clipper/test-clip-1/archive.json"),
        serde_json::to_vec(
            &doc.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::rename(source.parent().unwrap(), &bundle).unwrap();
    let svc = crate::capture::CaptureService::new(&root);
    assert_eq!(svc.count_inbox(), 0);
    let items = svc.list_items("archived");
    assert_eq!(items.len(), 1);
    assert_eq!(
        std::fs::canonicalize(items[0].archived_path.as_ref().unwrap()).unwrap(),
        std::fs::canonicalize(&doc).unwrap()
    );
    assert!(svc.remove(&items[0].id).unwrap());
    assert!(
        doc.is_file(),
        "removing history must preserve archived documents"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delete_pending_clip_removes_owned_bundle_only() {
    let (root, settings) = workspace();
    let receipt = save(&root, &settings, b"delete me");
    let svc = crate::capture::CaptureService::new(&root);
    svc.remove("web-test-clip-1").unwrap();
    assert!(!root.join(receipt["document"].as_str().unwrap()).exists());
    assert_eq!(svc.count_inbox(), 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn validates_limits_checksum_and_default_library() {
    let (root, settings) = workspace();
    let mut oversized = input(&root, b"text");
    oversized.files[0].size = MAX_TOTAL + 1;
    assert!(storage::begin(&root, oversized).is_err());
    let mut clip = input(&root, b"text");
    clip.destination = "default".into();
    storage::begin(&root, clip).unwrap();
    storage::chunk(
        &root,
        "test-clip-1",
        "document.md",
        0,
        &base64::engine::general_purpose::STANDARD.encode(b"xxxx"),
    )
    .unwrap();
    assert!(storage::commit(&root, "test-clip-1", &settings).is_err());
    assert_eq!(libraries(&root).unwrap().len(), 1);
    assert_eq!(crate::capture::CaptureService::new(&root).count_inbox(), 0);
    std::fs::remove_dir_all(root).unwrap();
}
