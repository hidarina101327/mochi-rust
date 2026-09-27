use super::*;
use std::fs;
use tempfile::tempdir;

fn config(root: &Path, name: &str, id: &str) -> DeviceConfig {
    DeviceConfig::new(
        root.join(format!("{name}-bare")),
        root.join(format!("{name}-workspace")),
        id,
    )
}

fn file(path: &str, bytes: &[u8]) -> SnapshotFile {
    SnapshotFile::new(path, bytes).unwrap()
}

fn init_remote(root: &Path) -> PathBuf {
    let remote = root.join("remote.git");
    Repository::init_bare(&remote).unwrap();
    remote
}

fn empty_device(config: DeviceConfig, transport: Arc<LocalTransport>) -> DeviceRepo {
    fs::create_dir_all(&config.bare_path).unwrap();
    Repository::init_bare(&config.bare_path).unwrap();
    DeviceRepo::open_with_transport(config, transport).unwrap()
}

#[test]
fn devices_merge_different_documents_and_push_idempotently() {
    let temp = tempdir().unwrap();
    let id = "block-snapshot-fully-preserved";
    let remote = init_remote(temp.path());
    let transport_a = Arc::new(LocalTransport::new(&remote).unwrap());
    let transport_b = Arc::new(LocalTransport::new(&remote).unwrap());

    let device_a =
        DeviceRepo::init_with_transport(config(temp.path(), "a", id), transport_a).unwrap();
    let base = device_a.head().unwrap().unwrap();
    assert!(matches!(
        device_a.push().unwrap(),
        PushOutcome::Pushed { head } if head == base
    ));

    let device_b = empty_device(config(temp.path(), "b", id), transport_b);
    assert!(matches!(
        device_b.pull().unwrap(),
        PullOutcome::FastForward { head } if head == base
    ));

    let a_head = device_a
        .commit_snapshot(Snapshot::new([file("a.md", b"from A")]).unwrap(), "A edit")
        .unwrap();
    device_a.push().unwrap();

    let b_head = device_b
        .commit_snapshot(Snapshot::new([file("b.md", b"from B")]).unwrap(), "B edit")
        .unwrap();
    assert_ne!(a_head, b_head);
    assert!(
        device_b.push().is_err(),
        "stale push must not force remote history"
    );
    assert_eq!(device_b.head().unwrap(), Some(b_head));

    let merged = device_b.pull().unwrap();
    let merge_head = match merged {
        PullOutcome::Merged { head } => head,
        other => panic!("expected clean merge, got {other:?}"),
    };
    let snapshot = device_b.snapshot().unwrap();
    assert_eq!(
        snapshot
            .files
            .iter()
            .find(|file| file.path == "a.md")
            .unwrap()
            .bytes,
        b"from A"
    );
    assert_eq!(
        snapshot
            .files
            .iter()
            .find(|file| file.path == "b.md")
            .unwrap()
            .bytes,
        b"from B"
    );
    assert_eq!(
        fs::read(device_b.workspace_path().join("a.md")).unwrap(),
        b"from A"
    );
    assert_eq!(
        fs::read(device_b.workspace_path().join("b.md")).unwrap(),
        b"from B"
    );

    assert!(matches!(
        device_b.push().unwrap(),
        PushOutcome::Pushed { head } if head == merge_head
    ));
    assert!(matches!(
        device_b.push().unwrap(),
        PushOutcome::AlreadyUpToDate { head } if head == merge_head
    ));
    assert!(matches!(
        device_a.pull().unwrap(),
        PullOutcome::FastForward { head } if head == merge_head
    ));
    assert_eq!(device_a.snapshot().unwrap(), snapshot);
}

#[test]
fn conflicting_same_file_reports_versions_without_updating_head_or_workspace() {
    let temp = tempdir().unwrap();
    let remote = init_remote(temp.path());
    let id = "block-conflict";
    let device_a = DeviceRepo::init_with_transport(
        config(temp.path(), "a", id),
        Arc::new(LocalTransport::new(&remote).unwrap()),
    )
    .unwrap();
    let base = device_a.head().unwrap().unwrap();
    device_a.push().unwrap();
    let device_b = empty_device(
        config(temp.path(), "b", id),
        Arc::new(LocalTransport::new(&remote).unwrap()),
    );
    device_b.pull().unwrap();

    let ours = device_a
        .commit_snapshot(Snapshot::new([file("same.md", b"A")]).unwrap(), "A")
        .unwrap();
    device_a.push().unwrap();
    let theirs = device_b
        .commit_snapshot(Snapshot::new([file("same.md", b"B")]).unwrap(), "B")
        .unwrap();
    assert_eq!(device_b.head().unwrap(), Some(theirs));
    let before_workspace = fs::read(device_b.workspace_path().join("same.md")).unwrap();
    assert_eq!(before_workspace, b"B");
    assert!(device_b.push().is_err());

    let report = match device_b.pull().unwrap() {
        PullOutcome::Conflict { report } => report,
        other => panic!("expected conflict, got {other:?}"),
    };
    assert_eq!(report.base_head, Some(base));
    assert_eq!(report.ours_head, theirs);
    assert_eq!(report.theirs_head, ours);
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].path, "same.md");
    assert_eq!(device_b.head().unwrap(), Some(theirs));
    assert_eq!(
        fs::read(device_b.workspace_path().join("same.md")).unwrap(),
        b"B"
    );

    let mut resolution = report.resolution();
    assert_eq!(
        resolution.unresolved_paths().collect::<Vec<_>>(),
        vec!["same.md"]
    );
    resolution.resolve_file("same.md", b"A + B").unwrap();
    assert_eq!(resolution.unresolved_paths().count(), 0);
    let resolved = device_b
        .commit_resolution(resolution, "resolve same.md")
        .unwrap();
    assert_eq!(device_b.head().unwrap(), Some(resolved));
    assert_eq!(
        fs::read(device_b.workspace_path().join("same.md")).unwrap(),
        b"A + B"
    );
    device_b.push().unwrap();

    let remote_repo = Repository::open_bare(&remote).unwrap();
    assert_eq!(
        ref_target(&remote_repo, "refs/heads/main").unwrap(),
        Some(resolved)
    );
    let commit = remote_repo.find_commit(resolved).unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(0).unwrap(), theirs);
    assert_eq!(commit.parent_id(1).unwrap(), ours);
    // 两个冲突版本都还能通过 merge 父提交找到；解决方案没有丢弃任何一个。
    assert!(remote_repo.find_commit(ours).is_ok());
    assert!(remote_repo.find_commit(theirs).is_ok());
}

#[derive(Clone)]
struct FailingTransport;

impl Transport for FailingTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            kind: TransportKind::LocalBare,
            implemented: true,
            network_enabled: false,
            note: "test failure adapter".into(),
        }
    }

    fn fetch(&self, _request: &FetchRequest) -> Result<FetchResponse> {
        bail!("injected fetch failure")
    }

    fn push(&self, _request: &PushRequest) -> Result<PushResponse> {
        bail!("injected push failure")
    }
}

#[test]
fn transport_failure_does_not_move_local_head_or_workspace() {
    let temp = tempdir().unwrap();
    let config = config(temp.path(), "device", "failure-block");
    let mut device = DeviceRepo::init(config).unwrap();
    let before = device.head().unwrap().unwrap();
    device
        .commit_snapshot(Snapshot::new([file("note.md", b"safe")]).unwrap(), "local")
        .unwrap();
    let after_commit = device.head().unwrap().unwrap();
    assert_ne!(before, after_commit);
    assert!(device.push().is_err());
    assert_eq!(device.head().unwrap(), Some(after_commit));
    assert_eq!(
        fs::read(device.workspace_path().join("note.md")).unwrap(),
        b"safe"
    );

    device.set_transport(Arc::new(FailingTransport));
    assert!(device.pull().is_err());
    assert_eq!(device.head().unwrap(), Some(after_commit));
    assert_eq!(
        fs::read(device.workspace_path().join("note.md")).unwrap(),
        b"safe"
    );
}

#[test]
fn block_id_comment_keeps_the_complete_identifier() {
    let temp = tempdir().unwrap();
    let id = format!("block-{}-末尾", "x".repeat(1024));
    let device = DeviceRepo::init(config(temp.path(), "device", &id)).unwrap();
    assert_eq!(device.persisted_block_id().unwrap(), id);
    let repo = Repository::open_bare(device.bare_path()).unwrap();
    let head = device.head().unwrap().unwrap();
    let files = commit_files(&repo, head).unwrap();
    assert_eq!(
        parse_block_id(files.get(BLOCK_ID_COMMENT_PATH).unwrap()).unwrap(),
        id
    );
    assert!(files
        .get(BLOCK_ID_COMMENT_PATH)
        .unwrap()
        .windows(id.len())
        .any(|window| window == id.as_bytes()));
}

#[test]
fn invalid_paths_and_symlink_traversal_are_rejected() {
    for path in [
        "../outside.md",
        "a/../../outside.md",
        "/absolute.md",
        "C:/absolute.md",
        ".git/config",
        "nested/.git/config",
        ".mochi/secrets.json",
        "refs/heads/main",
        "a//b",
    ] {
        assert!(
            SnapshotFile::new(path, b"x").is_err(),
            "accepted unsafe path {path}"
        );
    }

    let temp = tempdir().unwrap();
    let device = DeviceRepo::init(config(temp.path(), "device", "symlink-block")).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::fs::symlink_dir;
        let outside = temp.path().join("outside");
        let inside = device.workspace_path().join("linked");
        fs::create_dir_all(&outside).unwrap();
        if symlink_dir(&outside, &inside).is_ok() {
            let result = device.commit_snapshot(
                Snapshot::new([file("linked/escaped.md", b"must fail")]).unwrap(),
                "symlink",
            );
            assert!(result.is_err());
            assert!(!outside.join("escaped.md").exists());
        }
    }
}

#[test]
fn external_workspace_edit_or_delete_blocks_fast_forward() {
    let temp = tempdir().unwrap();
    let remote = init_remote(temp.path());
    let id = "workspace-guard";
    let device_a = DeviceRepo::init_with_transport(
        config(temp.path(), "a", id),
        Arc::new(LocalTransport::new(&remote).unwrap()),
    )
    .unwrap();
    device_a.push().unwrap();
    device_a
        .commit_snapshot(Snapshot::new([file("base.md", b"base")]).unwrap(), "base")
        .unwrap();
    let base = device_a.head().unwrap().unwrap();
    device_a.push().unwrap();

    let device_b = empty_device(
        config(temp.path(), "b", id),
        Arc::new(LocalTransport::new(&remote).unwrap()),
    );
    device_b.pull().unwrap();
    assert_eq!(device_b.head().unwrap(), Some(base));

    fs::write(device_b.workspace_path().join("base.md"), b"external edit").unwrap();
    device_a
        .commit_snapshot(
            Snapshot::new([file("remote.md", b"remote")]).unwrap(),
            "remote",
        )
        .unwrap();
    let remote_head = device_a.head().unwrap().unwrap();
    device_a.push().unwrap();

    let before = device_b.head().unwrap().unwrap();
    assert!(device_b.pull().is_err());
    assert_eq!(device_b.head().unwrap(), Some(before));
    assert_eq!(
        fs::read(device_b.workspace_path().join("base.md")).unwrap(),
        b"external edit"
    );
    assert!(!device_b.workspace_path().join("remote.md").exists());

    // 被跟踪文件的删除同样有这道保护。
    fs::write(device_b.workspace_path().join("base.md"), b"base").unwrap();
    fs::remove_file(device_b.workspace_path().join("base.md")).unwrap();
    assert!(device_b.pull().is_err());
    assert_eq!(device_b.head().unwrap(), Some(before));
    assert!(!device_b.workspace_path().join("base.md").exists());
    assert_eq!(device_a.head().unwrap(), Some(remote_head));
}

#[test]
fn unchanged_snapshot_still_rejects_external_workspace_edit() {
    let temp = tempdir().unwrap();
    let device = DeviceRepo::init(config(temp.path(), "no-op-guard", "no-op-block")).unwrap();
    device
        .commit_snapshot(
            Snapshot::new([file("note.md", b"tracked")]).unwrap(),
            "tracked",
        )
        .unwrap();
    fs::write(
        device.workspace_path().join("note.md"),
        b"edited outside sync",
    )
    .unwrap();

    let result = device.commit_snapshot(Snapshot::empty(), "no-op");
    assert!(result.is_err());
    assert_ne!(
        fs::read(device.workspace_path().join("note.md")).unwrap(),
        b"tracked"
    );
    assert_eq!(
        device.snapshot().unwrap().files,
        vec![file("note.md", b"tracked")]
    );
}

#[test]
fn path_collisions_are_rejected_for_case_and_parent_files() {
    assert!(Snapshot::new([file("docs/readme.md", b"x"), file("DOCS/README.md", b"y"),]).is_err());
    assert!(Snapshot::new([file("a", b"x"), file("a/b", b"y")]).is_err());
    assert!(Snapshot::new([file("Docs", b"x"), file("docs/readme.md", b"y")]).is_err());
}

#[test]
fn cleanup_failure_retains_recovery_path_as_warning() {
    let temp = tempdir().unwrap();
    let target = temp.path().join("note.md");
    let backup = temp.path().join("backup-entry");
    fs::create_dir(&backup).unwrap();
    let mut transaction = WorkspaceTransaction {
        changes: vec![PendingWorkspaceWrite {
            target,
            staged: None,
            backup: Some(backup.clone()),
            had_original: true,
            installed: false,
        }],
    };

    let warning = transaction
        .finish()
        .expect("directory backup cannot be removed as a file");
    assert!(warning.recovery_paths.contains(&backup));
    assert!(backup.exists());
    assert!(warning
        .details
        .contains("cleanup failed after the commit point"));
}

#[test]
fn git_cli_local_bare_transport_push_pull_conflict_and_failure_are_isolated() {
    // 运行时适配器是可选的。这个夹具保持可移植：
    // 不依赖外部 Git 二进制也能编译本 crate。
    let git_available = Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !git_available {
        return;
    }

    let temp = tempdir().unwrap();
    let remote = init_remote(temp.path());
    let id = "git-cli-local-fixture";
    let transport_a = Arc::new(GitCliTransport::local(&remote).unwrap());
    let device_a =
        DeviceRepo::init_with_transport(config(temp.path(), "git-cli-a", id), transport_a).unwrap();
    assert!(device_a.transport_capabilities().implemented);
    assert!(!device_a.transport_capabilities().network_enabled);
    device_a.push().unwrap();

    let device_b = DeviceRepo::create_empty_with_transport(
        config(temp.path(), "git-cli-b", id),
        Arc::new(GitCliTransport::local(&remote).unwrap()),
    )
    .unwrap();
    assert!(matches!(
        device_b.pull().unwrap(),
        PullOutcome::FastForward { .. }
    ));

    let base = device_b.head().unwrap().unwrap();
    let a_head = device_a
        .commit_snapshot(Snapshot::new([file("same.md", b"A")]).unwrap(), "A")
        .unwrap();
    device_a.push().unwrap();
    let b_head = device_b
        .commit_snapshot(Snapshot::new([file("same.md", b"B")]).unwrap(), "B")
        .unwrap();

    // 过期的 push 会被显式的远端 head 比对拒绝，
    // 本设备仍停在自己的提交上。
    assert!(device_b.push().is_err());
    assert_eq!(device_b.head().unwrap(), Some(b_head));
    assert_eq!(
        fs::read(device_b.workspace_path().join("same.md")).unwrap(),
        b"B"
    );

    let report = match device_b.pull().unwrap() {
        PullOutcome::Conflict { report } => report,
        other => panic!("expected Git CLI conflict, got {other:?}"),
    };
    assert_eq!(report.base_head, Some(base));
    assert_eq!(report.ours_head, b_head);
    assert_eq!(report.theirs_head, a_head);
    assert_eq!(device_b.head().unwrap(), Some(b_head));
    assert_eq!(
        fs::read(device_b.workspace_path().join("same.md")).unwrap(),
        b"B"
    );

    let mut resolution = report.resolution();
    resolution.resolve_file("same.md", b"A+B").unwrap();
    let merge_head = device_b
        .commit_resolution(resolution, "merge Git CLI conflict")
        .unwrap();
    device_b.push().unwrap();
    assert_eq!(
        ref_target(&Repository::open_bare(&remote).unwrap(), "refs/heads/main").unwrap(),
        Some(merge_head)
    );

    // 可执行文件缺失是确定的传输失败：碰不到远端、开不了网络 socket，
    // 本地分支和物化的工作区也都不会动。
    let before_failure = device_b.head().unwrap().unwrap();
    let before_bytes = fs::read(device_b.workspace_path().join("same.md")).unwrap();
    let failing_git = temp.path().join("does-not-exist-git");
    let failing_transport = GitCliTransport::from_config(
        GitCliTransportConfig::local(&remote).with_executable(failing_git),
    )
    .unwrap();
    let mut device_b = device_b;
    device_b.set_transport(Arc::new(failing_transport));
    assert!(device_b.push().is_err());
    assert_eq!(device_b.head().unwrap(), Some(before_failure));
    assert_eq!(
        fs::read(device_b.workspace_path().join("same.md")).unwrap(),
        before_bytes
    );
}

#[test]
fn git_cli_endpoint_validation_rejects_credentials_options_and_helper_protocols() {
    for endpoint in [
        "https://user@example.test/blocks.git",
        "https://example.test/blocks.git?token=secret",
        "--upload-pack=evil",
        "ext::sh -c evil",
        "ssh://example.test/repo%2Fpath",
    ] {
        assert!(
            GitCliTransport::new(endpoint).is_err(),
            "accepted unsafe Git endpoint {endpoint}"
        );
    }
}
