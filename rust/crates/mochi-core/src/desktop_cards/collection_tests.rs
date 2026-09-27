use super::*;
#[test]
fn desktop_collections_union_sources_filter_types_and_persist_item_overrides() {
    let root = std::env::temp_dir().join(format!("mochi-collection-{}", new_id("test")));
    fs::create_dir_all(root.join("library/sub")).unwrap();
    for name in [
        "library/a.md",
        "library/sub/b.md",
        "library/table.mcb",
        "library/board.mcanvas",
    ] {
        fs::write(root.join(name), "source must remain unchanged").unwrap();
    }
    let mut config = DesktopConfig::new();
    let mut card = Card::new("Documents", Module::Document);
    let p = &mut card.pages[0];
    p.sources = vec!["library".into(), "library/sub/b.md".into()];
    p.presentation.show_modified = true;
    p.limit = 0;
    p.item_styles.insert(
        item_key("", "library/a.md"),
        ItemStyle {
            foreground: Some(0x123456),
            background: Some(0xeeeeff),
        },
    );
    config.cards.push(card);
    config.validate().unwrap();
    let snapshot = build_snapshot(&root, &config).unwrap();
    let rows = &snapshot.pages.values().next().unwrap().rows;
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| r.meta.always_detail && !r.detail.is_empty()));
    for (module, path) in [
        (Module::Base, "library/table.mcb"),
        (Module::Canvas, "library/board.mcanvas"),
    ] {
        let mut page = Page::new(module);
        page.sources = vec!["library".into()];
        let rows = collection::rows(&root, &page, 100, 100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].meta.path, path);
    }
    config.save(&root).unwrap();
    assert_eq!(DesktopConfig::load(&root).unwrap(), config);
    assert_eq!(
        fs::read_to_string(root.join("library/a.md")).unwrap(),
        "source must remain unchanged"
    );
    config.cards[0].pages[0].sources = vec!["../outside".into()];
    assert!(config.validate().is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn desktop_shortcut_reorder_inserts_instead_of_swapping_and_keeps_originals() {
    let mut s = studio::Studio::default();
    s.add_shortcuts(&[
        "C:/a.lnk".into(),
        "C:/b.lnk".into(),
        "C:/c.lnk".into(),
        "C:/d.lnk".into(),
    ]);
    let first = s.nodes[0].id.clone();
    studio::move_shortcut(&mut s, &first, 3);
    assert_eq!(
        s.nodes
            .iter()
            .map(|n| n.target.as_str())
            .collect::<Vec<_>>(),
        vec!["C:/b.lnk", "C:/c.lnk", "C:/a.lnk", "C:/d.lnk"]
    );
    studio::move_shortcut(&mut s, &first, 0);
    assert_eq!(s.nodes[0].id, first);
    assert!(Module::ALL.contains(&Module::Document));
    assert!(!Module::ALL.contains(&Module::QuickNav));
    assert_eq!(Module::from_wire("quickNav"), Some(Module::QuickNav));

    let mut card = Card::new("Legacy icons", Module::Shortcuts);
    card.pages[0].studio = s;
    card.pages[0].studio.nodes[0].foreground = Some(0xffffff);
    card.pages[0].studio.nodes[0].background = Some(0x234567);
    let key = format!("node:{}", card.pages[0].studio.nodes[0].id);
    let mut config = DesktopConfig::default();
    config.cards.push(card);
    config.migrate_templates();
    let migrated = config.clone();
    config.migrate_templates();
    assert_eq!(config, migrated, "migration is idempotent");
    let page = &config.cards[0].pages[0];
    assert!(page.presentation.grid);
    assert_eq!(page.item_styles[&key].background, Some(0x234567));
    assert!(page.studio.nodes[0].foreground.is_none());
}

#[test]
fn desktop_sources_include_empty_directories_and_exclude_application_configuration() {
    use crate::object_reference::ObjectKind;
    let root = std::env::temp_dir().join(format!("mochi-source-candidates-{}", std::process::id()));
    for folder in [
        "知识库/逆向/空文件夹",
        "Agent配置/Agents",
        "AI提示词/Skills",
        ".mochi/cache/desktop-icons",
        "schedule",
    ] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    for file in [
        "知识库/逆向/概述.mc",
        "知识库/逆向/表格.mcb",
        "Agent配置/Agents/不应出现.md",
        "AI提示词/Skills/也不应出现.md",
        ".mochi/cache/desktop-icons/cache.png",
        "schedule/task.json",
    ] {
        std::fs::write(root.join(file), [0xff, 0xfe]).unwrap();
    }
    let items = super::collection::source_candidates(&root);
    assert!(items
        .iter()
        .any(|c| c.title == "逆向" && c.kind == ObjectKind::Directory));
    assert!(items
        .iter()
        .any(|c| c.title == "空文件夹" && c.kind == ObjectKind::Directory));
    assert!(items
        .iter()
        .any(|c| c.title == "概述.mc" && c.kind == ObjectKind::Document));
    assert!(!items.iter().any(|c| c.title.contains("不应出现")
        || c.title == "Agent配置"
        || c.title == "cache.png"
        || c.title == "task.json"));
    assert!(!super::collection::source_relative_allowed(
        std::path::Path::new("../Agent配置")
    ));
    std::fs::remove_dir_all(root).unwrap();
}
