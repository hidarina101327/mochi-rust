//! 无界面的输入和几何测试：不截图，也不改动用户工作区。
use super::*;

fn with_app(test: impl FnOnce(&mut App, &Path)) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-tab-navigation-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.settings.set("app.tabs.openFileBehavior", "new-tab");
        app.load_chrome_settings();
        app.shell.open_workspace(&root, || {}).unwrap();
        for name in ["a.mc", "b.mc", "c.mc"] {
            std::fs::write(root.join(name), format!("# {name}\n\nA paragraph.\n")).unwrap();
        }
        assert!(app.shell.open_file_with_mode(&root.join("a.mc"), true));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        assert_eq!(app.content(), MainContent::Document);
        app.renderer.prepare_snapshot(1400, 900, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        test(&mut app, &root);
    }
    let _ = std::fs::remove_dir_all(root);
}

fn click(app: &mut App, direction: usize) {
    let chrome = app.build_chrome();
    let rect = tab_bar::navigation_button_rects(chrome.tree.rect(chrome.tab_bar))[direction];
    let (x, y) = (
        (rect.left + rect.right) * 0.5,
        (rect.top + rect.bottom) * 0.5,
    );
    app.on_mouse_move(x, y);
    app.on_click(x, y);
    app.paint(HWND::default()).unwrap();
}

#[test]
fn arrows_navigate_in_place_and_disabled_clicks_never_select_or_close_tabs() {
    with_app(|app, root| {
        click(app, 0);
        assert_eq!(app.active_file_path(), Some(root.join("a.mc")));
        app.open_link("b.mc");
        assert_eq!(
            app.shell.tabs().len(),
            1,
            "normal document links stay in this tab"
        );
        assert_eq!(app.active_file_path(), Some(root.join("b.mc")));
        app.paint(HWND::default()).unwrap();
        click(app, 0);
        assert_eq!(app.active_file_path(), Some(root.join("a.mc")));
        assert!(!app.tab_navigation_enabled(NavigationDirection::Back));
        assert!(app.tab_navigation_enabled(NavigationDirection::Forward));
        click(app, 1);
        assert_eq!(app.active_file_path(), Some(root.join("b.mc")));
        click(app, 1);
        assert_eq!(app.shell.tabs().len(), 1);
        assert_eq!(app.shell.active_tab(), Some(0));
    });
}

#[test]
fn file_tree_new_tab_preference_and_replace_history_remain_distinct() {
    with_app(|app, root| {
        assert!(app.open_file_from_ui(&root.join("b.mc")));
        assert_eq!(app.shell.tabs().len(), 2);
        assert!(!app.tab_navigation_enabled(NavigationDirection::Back));
        app.settings.set("app.tabs.openFileBehavior", "replace");
        app.load_chrome_settings();
        assert!(app.open_file_from_ui(&root.join("c.mc")));
        assert_eq!(app.shell.tabs().len(), 2);
        app.sync_state();
        click(app, 0);
        assert_eq!(app.active_file_path(), Some(root.join("b.mc")));
        assert_eq!(app.shell.active_tab(), Some(1));
        assert_eq!(
            app.shell.tabs()[0].path(),
            Some(root.join("a.mc").as_path())
        );
    });
}

#[test]
fn keyboard_matches_buttons_and_settings_modal_blocks_background_navigation() {
    with_app(|app, root| {
        app.open_link("b.mc");
        assert!(app.on_accelerator(HWND::default(), 0x25, false, false, true));
        assert_eq!(app.active_file_path(), Some(root.join("a.mc")));
        assert!(app.on_accelerator(HWND::default(), 0x27, false, false, true));
        assert_eq!(app.active_file_path(), Some(root.join("b.mc")));
        app.settings_overlay = Some(("general".into(), "".into()));
        app.on_accelerator(HWND::default(), 0x25, false, false, true);
        assert_eq!(app.active_file_path(), Some(root.join("b.mc")));
    });
}

#[test]
fn returning_to_chunked_document_restores_page_and_scroll_without_full_layout() {
    with_app(|app, root| {
        let source: String = (0..220)
            .map(|i| format!("## 标题 {i}\n\n词义{i} **内容**\n\n"))
            .collect();
        let path = root.join("large.mc");
        std::fs::write(&path, &source).unwrap();
        assert!(app.open_link_file_from_ui(&path));
        app.invalidate_main();
        app.sync_state();
        app.paint(HWND::default()).unwrap();
        for _ in 0..1000 {
            if !app.doc.is_loading() {
                break;
            }
            app.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
        }
        assert!(app.doc.is_chunked());
        assert!(app.doc.select_chunk(app.editor_area, 3));
        for _ in 0..1000 {
            if !app.doc.is_loading() {
                break;
            }
            app.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
        }
        app.shell.set_active_scroll(120.0);
        // 光标会固定留在第 0 页：不能用光标位置还原阅读位置，
        // 因为光标可能远在视口之外。
        app.shell.active_buffer_mut().unwrap().set_cursor(0, false);
        app.open_link("b.mc");
        app.paint(HWND::default()).unwrap();
        click(app, 0);
        for _ in 0..1000 {
            if !app.doc.is_loading() {
                break;
            }
            app.on_timer(HWND::default(), platform::TIMER_DOCUMENT_LAYOUT);
        }
        assert_eq!(app.doc.chunk_index(), 3);
        assert_eq!(app.shell.active_scroll(), 120.0);
        assert!(app.doc.is_chunked());
        assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), source);
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    });
}

#[test]
fn history_navigation_preserves_other_split_document_and_shared_edits() {
    with_app(|app, root| {
        app.open_link("b.mc");
        app.split_to_right(root.join("b.mc"));
        click(app, 0);
        assert_eq!(app.active_file_path(), Some(root.join("a.mc")));
        assert_eq!(app.split.other, Some(root.join("b.mc")));
        assert!(app.split.right);
        click(app, 1);
        assert_eq!(app.active_file_path(), Some(root.join("b.mc")));
        let active = app.shell.active_tab().unwrap();
        app.shell
            .active_buffer_mut()
            .unwrap()
            .insert("live split edit");
        assert_eq!(app.shell.text_tab_index(&root.join("b.mc")), Some(active));
        app.paint(HWND::default()).unwrap();
        assert!(app.focus_other_editor());
        assert!(app
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("live split edit"));
    });
}
