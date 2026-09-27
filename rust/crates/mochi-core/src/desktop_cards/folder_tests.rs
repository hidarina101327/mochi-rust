use super::*;
use crate::desktop_cards::folder::{self, FolderConfig, FolderSort, FolderStack};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn workspace(tag: &str) -> PathBuf {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let raw = std::env::temp_dir().join(format!(
        "mochi-folder-{tag}-{}-{sequence}",
        std::process::id()
    ));
    let root = raw;
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn config_for(path: &std::path::Path) -> FolderConfig {
    FolderConfig {
        path: path.to_string_lossy().into_owned(),
        ..FolderConfig::default()
    }
}

fn page_for(path: &std::path::Path) -> Page {
    let mut page = Page::new(Module::Folder);
    page.folder = config_for(path);
    page
}

fn rows(snapshot: &PageSnapshot) -> Vec<String> {
    snapshot.rows.iter().map(|row| row.title.clone()).collect()
}

#[test]
fn folder_defaults_refreshes_entries_and_never_reads_file_contents() {
    let root = workspace("refresh");
    fs::create_dir(root.join("subfolder")).unwrap();
    fs::write(root.join("alpha.txt"), b"private body must stay unopened").unwrap();
    let before = fs::metadata(root.join("alpha.txt")).unwrap();

    let page = page_for(&root);
    assert!(page.presentation.grid);
    assert_eq!(page.limit, 0);
    assert_eq!(
        page.options,
        vec!["files".to_string(), "folders".to_string()]
    );
    let mut card = Card::new("Folder", Module::Folder);
    card.pages[0].folder = page.folder.clone();
    let mut desktop = DesktopConfig::default();
    let key = page_key(&card.id, &card.pages[0].id);
    desktop.cards.push(card);
    let builder = SnapshotBuilder::new(&root);

    let first = builder.build(&desktop).unwrap();
    let first_page = &first.pages[&key];
    assert_eq!(rows(first_page), vec!["subfolder", "alpha"]);
    assert!(first_page
        .subtitle
        .contains(&root.to_string_lossy().to_string()));
    assert!(matches!(
        &first_page.rows[1].action,
        Some(Action::OpenFolderEntry(path)) if path.ends_with("alpha.txt")
    ));
    assert_eq!(
        first_page.rows[1].meta.path,
        root.join("alpha.txt").to_string_lossy().into_owned()
    );

    fs::write(root.join("beta.md"), b"added after the prior refresh").unwrap();
    let second = builder.build(&desktop).unwrap();
    assert!(rows(&second.pages[&key]).contains(&"beta".to_string()));

    let after = fs::metadata(root.join("alpha.txt")).unwrap();
    assert_eq!(
        fs::read(root.join("alpha.txt")).unwrap(),
        b"private body must stay unopened"
    );
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_configuration_roundtrips_and_old_pages_get_defaults() {
    let legacy_page = serde_json::to_value(Page::new(Module::Home)).unwrap();
    assert!(
        legacy_page.get("folder").is_none(),
        "ordinary pages must remain readable by older versions"
    );
    let root = workspace("roundtrip");
    let mut card = Card::new("Mapped", Module::Folder);
    card.pages[0].folder = FolderConfig {
        path: root.to_string_lossy().into_owned(),
        filter: "report".into(),
        sort: FolderSort::Modified,
        show_hidden: true,
        ..FolderConfig::default()
    };
    let mut config = DesktopConfig::default();
    config.cards.push(card);
    let json = serde_json::to_string(&config).unwrap();
    assert_eq!(
        serde_json::from_str::<DesktopConfig>(&json).unwrap(),
        config
    );

    let mut old_json = serde_json::to_value(&config).unwrap();
    old_json["cards"][0]["pages"][0]
        .as_object_mut()
        .unwrap()
        .remove("folder");
    let old: DesktopConfig = serde_json::from_value(old_json).unwrap();
    assert_eq!(old.cards[0].pages[0].folder, FolderConfig::default());

    let imported = DesktopConfig::import_json(&json).unwrap();
    assert!(!imported.cards[0].enabled);
    assert_eq!(
        imported.cards[0].pages[0].folder,
        config.cards[0].pages[0].folder
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_scan_and_row_caps_are_reported_in_subtitle() {
    let root = workspace("limits");
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(root.join(name), b"").unwrap();
    }
    let page = page_for(&root);
    let snapshot = folder::snapshot(&page, 2, 1).unwrap();
    assert_eq!(snapshot.rows.len(), 1);
    assert!(snapshot.subtitle.contains("扫描达到本次 2 项上限"));
    assert!(snapshot.subtitle.contains("显示达到本次 1 行上限"));
    assert!(
        snapshot.subtitle.find("扫描达到").unwrap() < snapshot.subtitle.find("文件夹：").unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_filter_sort_extensions_hidden_and_sensitive_directories() {
    let root = workspace("presentation");
    fs::create_dir(root.join("z-folder")).unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::create_dir(root.join(".mochi")).unwrap();
    fs::write(root.join(".secret.txt"), b"").unwrap();
    fs::write(root.join("b.TXT"), b"").unwrap();
    fs::write(root.join("a.md"), b"").unwrap();

    let mut page = page_for(&root);
    page.folder.sort = FolderSort::Type;
    let snapshot = folder::snapshot(&page, 100, 100).unwrap();
    assert_eq!(rows(&snapshot), vec!["z-folder", "a", "b"]);

    page.presentation.show_extensions = true;
    page.presentation.show_modified = true;
    let snapshot = folder::snapshot(&page, 100, 100).unwrap();
    assert_eq!(rows(&snapshot), vec!["z-folder", "a.md", "b.TXT"]);
    assert!(snapshot.rows[1].meta.always_detail);
    assert!(snapshot.rows[1].detail.contains(" "));

    page.folder.filter = "tXt".into();
    let snapshot = folder::snapshot(&page, 100, 100).unwrap();
    assert_eq!(rows(&snapshot), vec!["b.TXT"]);

    page.folder.filter.clear();
    page.folder.show_hidden = true;
    let snapshot = folder::snapshot(&page, 100, 100).unwrap();
    assert!(rows(&snapshot).contains(&".secret.txt".into()));
    assert!(!rows(&snapshot).contains(&".git".into()));
    assert!(!rows(&snapshot).contains(&".mochi".into()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_or_non_directory_paths_remain_configurable_but_show_errors() {
    let root = workspace("missing");
    let missing = root.join("does-not-exist");
    let missing_config = config_for(&missing);
    folder::validate(&missing_config).unwrap();
    let mut page = Page::new(Module::Folder);
    page.folder = missing_config;
    let snapshot = folder::snapshot(&page, 10, 10).unwrap();
    assert!(snapshot.empty_message.starts_with("读取失败："));
    assert!(snapshot.subtitle.contains("does-not-exist"));

    let file = root.join("ordinary.txt");
    fs::write(&file, b"").unwrap();
    page.folder = config_for(&file);
    let snapshot = folder::snapshot(&page, 10, 10).unwrap();
    assert!(snapshot.empty_message.starts_with("读取失败："));
    assert!(snapshot
        .subtitle
        .contains(&file.to_string_lossy().to_string()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resolve_entry_allows_only_root_and_direct_non_link_children() {
    let root = workspace("resolve");
    let child = root.join("child.txt");
    let nested_dir = root.join("nested");
    fs::write(&child, b"content").unwrap();
    fs::create_dir(&nested_dir).unwrap();
    fs::write(nested_dir.join("deep.txt"), b"").unwrap();
    let config = config_for(&root);

    assert_eq!(
        folder::resolve_entry(&config, &root.to_string_lossy()).unwrap(),
        root
    );
    let hidden_root = root.join(".hidden-root");
    fs::create_dir(&hidden_root).unwrap();
    let hidden_config = config_for(&hidden_root);
    assert_eq!(
        folder::resolve_entry(&hidden_config, &hidden_root.to_string_lossy()).unwrap(),
        hidden_root
    );
    assert_eq!(
        folder::resolve_entry(&config, &child.to_string_lossy()).unwrap(),
        child
    );
    assert!(
        folder::resolve_entry(&config, &nested_dir.join("deep.txt").to_string_lossy()).is_err()
    );
    let parent_target = if cfg!(windows) {
        format!("{}\\..", root.display())
    } else {
        format!("{}/..", root.display())
    };
    assert!(folder::resolve_entry(&config, &parent_target).is_err());
    assert!(folder::resolve_entry(&config, r"\\server\share\secret").is_err());
    assert!(folder::resolve_entry(&config, r"\\?\C:\secret").is_err());

    let outside = root.with_file_name(format!(
        "{}-sibling.txt",
        root.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&outside, b"").unwrap();
    assert!(folder::resolve_entry(&config, &outside.to_string_lossy()).is_err());
    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn symlinks_and_path_traversal_are_rejected_or_skipped() {
    let root = workspace("symlinks");
    let outside = workspace("symlink-target");
    let external = outside.join("secret.txt");
    fs::write(&external, b"outside").unwrap();
    let link = root.join("linked.txt");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&external, &link).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&external, &link).is_ok();
    #[cfg(not(any(unix, windows)))]
    let linked = false;

    if linked {
        let page = page_for(&root);
        let snapshot = folder::snapshot(&page, 20, 20).unwrap();
        assert!(!rows(&snapshot).contains(&"linked".into()));
        assert!(snapshot.subtitle.contains("已跳过 1 个链接或云占位项"));
        assert!(folder::resolve_entry(&config_for(&root), &link.to_string_lossy()).is_err());
    }

    let dir_link = root.join("linked-folder");
    #[cfg(unix)]
    let dir_linked = std::os::unix::fs::symlink(&outside, &dir_link).is_ok();
    #[cfg(windows)]
    let dir_linked = std::os::windows::fs::symlink_dir(&outside, &dir_link).is_ok();
    #[cfg(not(any(unix, windows)))]
    let dir_linked = false;
    if dir_linked {
        let snapshot = folder::snapshot(&page_for(&dir_link), 20, 20).unwrap();
        assert!(snapshot.empty_message.starts_with("读取失败："));
        assert!(
            folder::resolve_entry(&config_for(&dir_link), &dir_link.to_string_lossy()).is_err()
        );
    }

    let traversal = if cfg!(windows) {
        format!("{}\\..\\outside", root.display())
    } else {
        format!("{}/../outside", root.display())
    };
    assert!(folder::validate(&FolderConfig {
        path: traversal,
        ..FolderConfig::default()
    })
    .is_err());
    assert!(folder::validate(&FolderConfig {
        path: r"\\server\share\folder".into(),
        ..FolderConfig::default()
    })
    .is_err());
    assert!(folder::validate(&FolderConfig {
        path: r"\\?\C:\device".into(),
        ..FolderConfig::default()
    })
    .is_err());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn a_large_folder_page_does_not_starve_a_later_small_page() {
    let root = workspace("fair-budget");
    let large = root.join("large");
    let small = root.join("small");
    fs::create_dir(&large).unwrap();
    fs::create_dir(&small).unwrap();
    for index in 0..2050 {
        fs::write(large.join(format!("file-{index:04}.txt")), b"").unwrap();
    }
    fs::write(small.join("only.txt"), b"").unwrap();

    let mut first = Card::new("Large", Module::Folder);
    first.pages[0].folder = config_for(&large);
    let first_key = page_key(&first.id, &first.active_page);
    let mut second = Card::new("Small", Module::Folder);
    second.pages[0].folder = config_for(&small);
    let second_key = page_key(&second.id, &second.active_page);
    let mut desktop = DesktopConfig::default();
    desktop.cards.extend([first, second]);

    let snapshot = SnapshotBuilder::new(&root).build(&desktop).unwrap();
    assert_eq!(snapshot.pages[&first_key].rows.len(), 500);
    assert!(snapshot.pages[&first_key]
        .subtitle
        .contains("扫描达到本次 2048 项上限"));
    assert_eq!(rows(&snapshot.pages[&second_key]), vec!["only"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_shared_entry_budget_cannot_be_reset_by_another_folder_page() {
    let root = workspace("shared-budget");
    fs::write(root.join("one.txt"), b"").unwrap();
    fs::write(root.join("two.txt"), b"").unwrap();
    let mut desktop = DesktopConfig::default();
    let mut first = Card::new("First", Module::Folder);
    first.pages[0].folder = config_for(&root);
    let first_key = page_key(&first.id, &first.active_page);
    let mut second = Card::new("Second", Module::Folder);
    second.pages[0].folder = config_for(&root);
    let second_key = page_key(&second.id, &second.active_page);
    desktop.cards.extend([first, second]);

    let snapshot = build_snapshot_with_limits(
        &root,
        &desktop,
        SnapshotLimits {
            max_dir_entries: 1,
            ..SnapshotLimits::default()
        },
    )
    .unwrap();
    assert_eq!(snapshot.pages[&first_key].rows.len(), 1);
    assert!(snapshot.pages[&second_key].rows.is_empty());
    assert!(snapshot.pages[&second_key]
        .subtitle
        .contains("扫描预算已用尽"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_navigation_scopes_snapshots_and_entry_actions_to_current_directory() {
    let root = workspace("navigation-scope");
    let child = root.join("child");
    let nested = child.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(child.join("inside.txt"), b"").unwrap();
    fs::write(root.join("outside.txt"), b"").unwrap();
    let config = config_for(&root);

    let entered = folder::navigate(&config, &child.to_string_lossy()).unwrap();
    assert_eq!(entered.subfolder, "child");
    assert_eq!(
        fs::canonicalize(folder::current_config(&entered).unwrap().path).unwrap(),
        fs::canonicalize(&child).unwrap()
    );
    assert_eq!(
        rows(
            &folder::snapshot(
                &Page {
                    folder: entered.clone(),
                    ..Page::new(Module::Folder)
                },
                100,
                100
            )
            .unwrap()
        ),
        vec!["nested", "inside"]
    );
    assert!(folder::resolve_entry(&entered, &child.join("inside.txt").to_string_lossy()).is_ok());
    assert!(folder::resolve_entry(&entered, &root.join("outside.txt").to_string_lossy()).is_err());
    assert!(
        folder::navigate(&entered, &nested.to_string_lossy())
            .unwrap()
            .subfolder
            == "child/nested"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_navigation_preserves_bounded_view_state_per_directory() {
    let root = workspace("navigation-state");
    let child = root.join("child");
    fs::create_dir(&child).unwrap();
    let mut config = config_for(&root);
    config.manual_order = vec!["root-b.txt".into(), "root-a.txt".into()];
    config.stacks = vec![FolderStack {
        name: "Root pin".into(),
        items: vec!["root-a.txt".into()],
    }];

    let mut entered = folder::navigate(&config, &child.to_string_lossy()).unwrap();
    assert!(entered.manual_order.is_empty());
    assert!(entered.stacks.is_empty());
    entered.manual_order = vec!["child-b.txt".into(), "child-a.txt".into()];
    entered.stacks = vec![FolderStack {
        name: "Child pin".into(),
        items: vec!["child-a.txt".into()],
    }];

    let at_root = folder::parent(&entered).unwrap();
    assert!(at_root.subfolder.is_empty());
    assert_eq!(at_root.manual_order, vec!["root-b.txt", "root-a.txt"]);
    assert_eq!(at_root.stacks[0].name, "Root pin");
    assert_eq!(
        at_root.directory_views["child"].manual_order,
        vec!["child-b.txt", "child-a.txt"]
    );
    let at_child = folder::navigate(&at_root, &child.to_string_lossy()).unwrap();
    assert_eq!(at_child.manual_order, vec!["child-b.txt", "child-a.txt"]);
    assert_eq!(at_child.stacks[0].name, "Child pin");
    let at_root_again = folder::root(&at_child).unwrap();
    assert_eq!(at_root_again.manual_order, vec!["root-b.txt", "root-a.txt"]);
    assert_eq!(at_root_again.stacks[0].name, "Root pin");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_manual_order_and_stack_groups_are_display_only_and_contiguous() {
    let root = workspace("manual-groups");
    for name in ["a.txt", "b.txt", "c.md"] {
        fs::write(root.join(name), b"").unwrap();
    }
    let mut page = page_for(&root);
    page.folder.sort = FolderSort::Manual;
    page.folder.group_by_type = true;
    page.folder.manual_order = vec!["b.txt".into(), "a.txt".into(), "c.md".into()];
    page.folder.stacks = vec![FolderStack {
        name: "Pinned".into(),
        items: vec!["c.md".into(), "a.txt".into()],
    }];

    let snapshot = folder::snapshot(&page, 100, 100).unwrap();
    assert_eq!(rows(&snapshot), vec!["a", "c", "b"]);
    assert_eq!(snapshot.rows[0].meta.group, "叠放：Pinned");
    assert_eq!(snapshot.rows[1].meta.group, "叠放：Pinned");
    assert_eq!(snapshot.rows[2].meta.group, "类型：.txt");
    assert_eq!(
        snapshot
            .rows
            .iter()
            .map(|row| row.meta.path.clone())
            .collect::<Vec<_>>(),
        vec![
            root.join("a.txt").to_string_lossy().into_owned(),
            root.join("c.md").to_string_lossy().into_owned(),
            root.join("b.txt").to_string_lossy().into_owned(),
        ]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_view_state_and_subfolder_validation_are_bounded() {
    let root = workspace("validation");
    let mut config = config_for(&root);
    config.subfolder = "child/../outside".into();
    assert!(folder::validate(&config).is_err());
    config.subfolder = "child\\nested".into();
    assert!(folder::validate(&config).is_err());
    config.subfolder.clear();
    config.directory_views = (0..=folder::MAX_FOLDER_DIRECTORY_VIEWS)
        .map(|index| (format!("d{index}"), Default::default()))
        .collect();
    assert!(folder::validate(&config).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn importing_folder_json_never_enables_auto_organize() {
    let root = workspace("import-auto-organize");
    let mut source = DesktopConfig::new();
    let mut card = Card::new("Imported folder", Module::Folder);
    card.enabled = true;
    card.pages[0].folder = FolderConfig {
        path: root.to_string_lossy().into_owned(),
        auto_organize: true,
        ..FolderConfig::default()
    };
    source.cards.push(card);

    let exported = source.export_json().unwrap();
    let exported_value: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(
        exported_value["cards"][0]["pages"][0]["folder"]["autoOrganize"],
        true
    );
    let imported = DesktopConfig::import_json(&exported).unwrap();
    assert!(!imported.cards[0].enabled);
    assert!(!imported.cards[0].pages[0].folder.auto_organize);
    assert_eq!(
        imported.cards[0].pages[0].folder.path,
        root.to_string_lossy().into_owned()
    );
    let _ = fs::remove_dir_all(root);
}
