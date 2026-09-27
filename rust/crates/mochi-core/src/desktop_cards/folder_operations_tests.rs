use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mochi-folder-operations-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create test-owned temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config_at(path: &Path) -> FolderConfig {
    FolderConfig {
        path: path.to_string_lossy().into_owned(),
        ..FolderConfig::default()
    }
}

#[test]
fn create_and_rename_only_touch_safe_direct_children() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    fs::create_dir(&root).unwrap();
    let config = config_at(&root);

    let created = create_subdirectory(&config, "Drafts").unwrap();
    assert!(created.is_dir());
    assert!(create_subdirectory(&config, "../escape").is_err());

    let sibling = root.join("Taken");
    fs::create_dir(&sibling).unwrap();
    assert!(rename_entry(&config, &created.to_string_lossy(), "Taken").is_err());
    let renamed = rename_entry(&config, &created.to_string_lossy(), "Archive").unwrap();
    assert_eq!(renamed.file_name().unwrap(), "Archive");
    assert!(!created.exists());
    assert!(sibling.is_dir(), "existing destination must be preserved");
}

#[test]
fn transfer_staging_is_verified_and_published_without_replacing() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    let input = temp.path().join("incoming");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&input).unwrap();
    let source = input.join("notes.txt");
    fs::write(&source, b"reviewed content").unwrap();
    let config = config_at(&root);

    let plan = plan_transfer(
        &config,
        std::slice::from_ref(&source),
        TransferKind::Copy,
        TransferLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 1);
    assert!(plan.failures.is_empty());
    validate_transfer_plan(&config, &plan).unwrap();
    let staging_root = prepare_transfer_staging(&config, &plan).unwrap();
    validate_transfer_item(&config, &plan, 0).unwrap();

    fs::copy(&source, &plan.items[0].staged_target).unwrap();
    let published = publish_staged_item(&config, &plan, 0).unwrap();
    assert_eq!(fs::read(&published).unwrap(), b"reviewed content");
    assert_eq!(fs::read(&source).unwrap(), b"reviewed content");
    validate_source_manifest(&config, &plan, 0).unwrap();
    assert!(publish_staged_item(&config, &plan, 0).is_err());
    assert_eq!(fs::read(&published).unwrap(), b"reviewed content");
    cleanup_transfer_staging(&config, &plan).unwrap();
    assert!(!staging_root.exists());
}

#[test]
fn changed_source_blocks_publish_and_move_validation() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    let input = temp.path().join("incoming");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&input).unwrap();
    let source = input.join("item.bin");
    fs::write(&source, b"before").unwrap();
    let config = config_at(&root);
    let plan = plan_transfer(
        &config,
        std::slice::from_ref(&source),
        TransferKind::Move,
        TransferLimits::default(),
    )
    .unwrap();
    prepare_transfer_staging(&config, &plan).unwrap();
    fs::copy(&source, &plan.items[0].staged_target).unwrap();
    fs::write(&source, b"changed after review").unwrap();

    assert!(publish_staged_item(&config, &plan, 0).is_err());
    assert!(validate_source_manifest(&config, &plan, 0).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"changed after review");
    assert!(!plan.items[0].target.exists());
}

#[test]
fn changed_source_is_isolated_to_its_transfer_item() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    let input = temp.path().join("incoming");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&input).unwrap();
    let first = input.join("first.txt");
    let second = input.join("second.txt");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let config = config_at(&root);
    let plan = plan_transfer(
        &config,
        &[first.clone(), second.clone()],
        TransferKind::Copy,
        TransferLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 2);

    fs::write(&first, b"changed after planning").unwrap();
    prepare_transfer_staging(&config, &plan).unwrap();
    assert!(validate_transfer_item(&config, &plan, 0).is_err());
    validate_transfer_item(&config, &plan, 1).unwrap();
}

#[test]
fn transfer_rejects_recursive_inputs_and_existing_targets() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    fs::create_dir(&root).unwrap();
    let nested = root.join("nested.txt");
    fs::write(&nested, b"inside").unwrap();
    let config = config_at(&root);

    let recursive = plan_transfer(
        &config,
        std::slice::from_ref(&nested),
        TransferKind::Copy,
        TransferLimits::default(),
    )
    .unwrap();
    assert!(recursive.items.is_empty());
    assert_eq!(recursive.failures.len(), 1);
    assert_eq!(fs::read(&nested).unwrap(), b"inside");

    let input = temp.path().join("incoming");
    fs::create_dir(&input).unwrap();
    let source = input.join("same.txt");
    fs::write(&source, b"source").unwrap();
    fs::write(root.join("same.txt"), b"destination").unwrap();
    let conflict = plan_transfer(
        &config,
        std::slice::from_ref(&source),
        TransferKind::Copy,
        TransferLimits::default(),
    )
    .unwrap();
    assert!(conflict.items.is_empty());
    assert_eq!(fs::read(root.join("same.txt")).unwrap(), b"destination");
}

#[test]
fn transfer_allows_a_child_file_to_move_back_to_parent_but_rejects_ancestor_folder() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    let child = root.join("child");
    fs::create_dir_all(&child).unwrap();
    let source = child.join("notes.txt");
    fs::write(&source, b"from a mapped subfolder").unwrap();
    let config = config_at(&root);

    let plan = plan_transfer(
        &config,
        std::slice::from_ref(&source),
        TransferKind::Move,
        TransferLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 1);
    assert!(plan.failures.is_empty());
    assert_eq!(plan.items[0].target, root.join("notes.txt"));
    validate_transfer_plan(&config, &plan).unwrap();

    let container = temp.path().join("container");
    let nested_root = container.join("mapped");
    fs::create_dir_all(&nested_root).unwrap();
    let nested_config = config_at(&nested_root);
    let ancestor = plan_transfer(
        &nested_config,
        std::slice::from_ref(&container),
        TransferKind::Copy,
        TransferLimits::default(),
    )
    .unwrap();
    assert!(ancestor.items.is_empty());
    assert_eq!(ancestor.failures.len(), 1);
    assert!(ancestor.failures[0]
        .error
        .as_deref()
        .unwrap()
        .contains("祖先"));
}

#[cfg(unix)]
#[test]
fn transfer_rejects_symbolic_links() {
    use std::os::unix::fs::symlink;

    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    let input = temp.path().join("incoming");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&input).unwrap();
    let real = input.join("real.txt");
    fs::write(&real, b"do not follow").unwrap();
    let link = input.join("link.txt");
    symlink(&real, &link).unwrap();
    let config = config_at(&root);

    let plan = plan_transfer(
        &config,
        std::slice::from_ref(&link),
        TransferKind::Copy,
        TransferLimits::default(),
    )
    .unwrap();
    assert!(plan.items.is_empty());
    assert_eq!(plan.failures.len(), 1);
    assert!(root.join("link.txt").symlink_metadata().is_err());
}

#[test]
fn organize_plan_is_review_only_then_applies_independent_moves() {
    let temp = TempRoot::new();
    let root = temp.path().join("mapped");
    fs::create_dir(&root).unwrap();
    let image = root.join("photo.png");
    let text = root.join("readme.txt");
    fs::write(&image, b"image").unwrap();
    fs::write(&text, b"text").unwrap();
    let config = config_at(&root);

    let plan = plan_organize(&config).unwrap();
    assert_eq!(plan.actions.len(), 2);
    assert!(
        image.exists() && text.exists(),
        "planning must not change files"
    );
    let results = apply_organize(&config, &plan);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|result| result.succeeded));
    assert!(root.join("图片/photo.png").is_file());
    assert!(root.join("文档/readme.txt").is_file());
}
