use super::*;

fn with_app(test: impl FnOnce(&mut App, &Path)) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-object-cache-{}",
        mochi_core::paths::random_base36(12),
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        test(&mut app, &root);
    }
    let resolved = root.canonicalize().unwrap();
    assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(resolved
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("mochi-object-cache-"));
    std::fs::remove_dir_all(resolved).unwrap();
}

fn wait_for(app: &mut App, ready: impl Fn(&App) -> bool) {
    for _ in 0..500 {
        app.poll_object_picker();
        if ready(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("object picker scan did not finish");
}

#[test]
fn object_picker_reopen_shows_cache_then_appends_updates_across_purposes() {
    with_app(|app, root| {
        std::fs::write(root.join("b.md"), "cached").unwrap();
        app.open_object_picker(Purpose::Attach);
        // 送达前就关掉，扫描结果仍会写入共享缓存。
        app.finish_object_picker(None);
        wait_for(app, |app| app.object_picker_cache.get(root).is_some());
        let cached = app.object_picker_cache.get(root).unwrap().clone();
        assert!(cached.iter().any(|row| row.title == "b.md"));
        std::fs::write(root.join("a.md"), "new").unwrap();
        app.open_object_picker(Purpose::Insert);
        let dialog = app.object_picker.as_mut().unwrap();
        assert_eq!(dialog.state.candidates, cached);
        assert!(dialog.loading);
        dialog.state.query.set_text(".md");
        let url = dialog
            .state
            .candidates
            .iter()
            .find(|row| row.title == "b.md")
            .unwrap()
            .url
            .clone();
        dialog.state.toggle_url(&url);
        wait_for(app, |app| !app.object_picker.as_ref().unwrap().loading);
        let state = &app.object_picker.as_ref().unwrap().state;
        assert_eq!(state.query.text(), ".md");
        assert_eq!(state.selected, vec![url]);
        let titles = state
            .candidates
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>();
        assert!(
            titles.iter().position(|title| *title == "b.md")
                < titles.iter().position(|title| *title == "a.md")
        );
        app.finish_object_picker(None);
    });
}

#[test]
fn object_picker_draft_records_overlay_cached_disk_rows_without_mutating_cache() {
    use mochi_core::base::*;
    let root = Path::new("D:/workspace");
    let path = root.join("table.mcb");
    let mut document = create_base_document();
    let field = document.tables[0].fields[0].id.clone();
    document.tables[0].records = vec![BaseRecord {
        id: "row-1".into(),
        values: [(field.clone(), Value::String("saved".into()))]
            .into_iter()
            .collect(),
        ..Default::default()
    }];
    let mut cached = vec![];
    objects::replace_record_candidates(&mut cached, &path, root, &document);
    document.tables[0].records[0]
        .values
        .insert(field, Value::String("draft".into()));
    let mut visible = cached.clone();
    overlay_base(&mut visible, root, Some(&(path.clone(), document.clone())));
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].title, "draft");
    assert_eq!(cached[0].title, "saved");
    document.tables[0].records.clear();
    overlay_base(&mut visible, root, Some(&(path, document)));
    assert!(visible.is_empty());
}

#[test]
fn desktop_source_picker_owns_input_above_manager_and_commits_multiple_sources() {
    with_app(|app, root| {
        use mochi_core::desktop_cards::{Card, Module};
        std::fs::create_dir_all(root.join("library/empty-folder")).unwrap();
        std::fs::create_dir_all(root.join("Agent配置/Agents")).unwrap();
        std::fs::write(
            root.join("Agent配置/Agents/private.md"),
            "agent configuration",
        )
        .unwrap();
        std::fs::write(root.join("library/note.md"), "note").unwrap();
        std::fs::write(root.join("board.mcanvas"), "{}").unwrap();
        let card = Card::new("文档", Module::Document);
        let id = card.id.clone();
        let page = card.pages[0].id.clone();
        let mut config = mochi_core::desktop_cards::DesktopConfig::default();
        config.cards.push(card);
        app.desktop.panel = Some(crate::ui::desktop_cards::State::new(&config));
        app.open_object_picker(Purpose::DesktopSources {
            card: id.clone(),
            page: page.clone(),
            module: Module::Document,
        });
        wait_for(app, |app| {
            app.object_picker.as_ref().is_some_and(|d| !d.loading)
        });
        let candidates = &app.object_picker.as_ref().unwrap().state.candidates;
        assert!(!candidates.iter().any(|c| c.title == "board.mcanvas"
            || c.title == "private.md"
            || c.title == "Agent配置"));
        assert!(candidates
            .iter()
            .any(|c| c.title == "library" && c.kind == ObjectKind::Directory));
        assert!(candidates
            .iter()
            .any(|c| c.title == "empty-folder" && c.kind == ObjectKind::Directory));
        assert!(app.on_char('文'));
        assert_eq!(app.object_picker.as_ref().unwrap().state.query.text(), "文");
        assert_ne!(app.desktop.panel.as_ref().unwrap().card_name.text(), "文");
        assert!(app.on_accelerator(HWND::default(), 0x1b, false, false, false));
        assert!(app.object_picker.is_none());
        assert!(app.desktop.panel.is_some());
        app.open_object_picker(Purpose::DesktopSources {
            card: id,
            page,
            module: Module::Document,
        });
        let picker = &mut app.object_picker.as_mut().unwrap().state;
        let urls = picker
            .candidates
            .iter()
            .filter(|c| c.title == "library" || c.title == "note.md")
            .map(|c| c.url.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            urls.len(),
            2,
            "both actual selectable rows must be present on cache reopen"
        );
        for url in urls {
            picker.toggle_url(&url);
        }
        let urls = picker.selected_urls();
        app.finish_object_picker(Some(urls));
        let p = app
            .desktop
            .panel
            .as_ref()
            .unwrap()
            .selected_page_ref()
            .unwrap();
        assert_eq!(p.sources, vec!["library", "library/note.md"]);
        assert!(p.source.is_none());
        assert!(app.desktop.panel.as_ref().unwrap().dirty);
    });
}
