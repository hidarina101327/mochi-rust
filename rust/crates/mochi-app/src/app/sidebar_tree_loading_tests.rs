use super::*;

#[test]
fn show_in_file_tree_finds_a_renamed_document_in_collapsed_ancestors() {
    let root = std::env::temp_dir().join(format!(
        "mochi-tree-reveal-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let library = PathBuf::from(&app.shell.workspace().unwrap().libraries[0].path);
        let source = library.join("英语").join("文章").join("旧名称.md");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "# 正文保持不变").unwrap();
        app.shell.open_file(&source);
        let target = app.shell.rename(&source, "文章1.md").unwrap();
        assert_eq!(app.active_file_path(), Some(target.clone()));
        app.shell.collapse_all();

        // 测试已保存的 Windows 路径中混用分隔符的情况。
        let mixed = PathBuf::from(target.to_string_lossy().replacen('\\', "/", 1));
        app.show_in_file_tree(&mixed);
        let row = app
            .shell
            .rows()
            .iter()
            .position(|entry| entry.path == target)
            .unwrap();
        assert_eq!(app.shell.selected(), Some(row));
        assert!(!app.shell.rows().iter().any(|entry| entry.path == source));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "# 正文保持不变");

        let kind = app.shell.workspace().unwrap().libraries[0].kind.clone();
        app.shell.create_library(&kind, "另一个库").unwrap();
        app.show_in_file_tree(&mixed);
        assert_eq!(app.shell.selected_library(), Some(0));
        let row = app
            .shell
            .rows()
            .iter()
            .position(|entry| entry.path == target)
            .unwrap();
        assert_eq!(app.shell.selected(), Some(row));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_load_timer_preserves_an_inline_rename_when_rows_are_inserted_above_it() {
    let root = std::env::temp_dir().join(format!(
        "mochi-tree-timer-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let library = PathBuf::from(&app.shell.workspace().unwrap().libraries[0].path);
        let folder = library.join("folder");
        let target = library.join("target.md");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("child.md"), "child").unwrap();
        std::fs::write(&target, "target").unwrap();
        app.shell.refresh_tree();
        let directory_row = app
            .shell
            .rows()
            .iter()
            .position(|r| r.path == folder)
            .unwrap();
        app.shell.toggle(directory_row);
        let before = app
            .shell
            .rows()
            .iter()
            .position(|r| r.path == target)
            .unwrap();
        app.side.editing = Some(sidebar::Editing::new(
            EditKind::Rename { row: before },
            "edited.md",
        ));
        assert!(app
            .take_timer_requests()
            .iter()
            .any(|(id, _)| *id == platform::TIMER_TREE_LOAD));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.shell.tree_loading_pending() {
            assert!(std::time::Instant::now() < deadline);
            app.on_timer(HWND::default(), platform::TIMER_TREE_LOAD);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let editing = app.side.editing.as_ref().unwrap();
        let EditKind::Rename { row } = editing.kind else {
            panic!("rename was lost")
        };
        assert_eq!(row, before + 1);
        assert_eq!(app.shell.rows()[row].path, target);
        assert_eq!(editing.field.text(), "edited.md");
        assert!(!app
            .take_timer_requests()
            .iter()
            .any(|(id, _)| *id == platform::TIMER_TREE_LOAD));
    }
    std::fs::remove_dir_all(root).unwrap();
}
