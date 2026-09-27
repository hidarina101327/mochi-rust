//! 指针操作的回归测试；所有文件操作都使用临时工作区。
use super::*;

fn fixture() -> (App, PathBuf, PathBuf) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-tree-drag-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.open_workspace(HWND::default(), root.clone(), false)
        .unwrap();
    let library = PathBuf::from(&app.shell.workspace().unwrap().libraries[0].path);
    for name in ["a-folder", "b-folder", "c-folder"] {
        let folder = library.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("note.md"), "content").unwrap();
    }
    for i in 0..40 {
        std::fs::write(library.join(format!("note-{i}.md")), "content").unwrap();
    }
    app.shell.set_sort_mode("manual");
    app.shell.refresh_tree();
    app.state.view = WorkspaceView::Editor;
    app.renderer.prepare_snapshot(1400, 900, 96.0).unwrap();
    app.paint(HWND::default()).unwrap();
    (app, root, library)
}

fn rect(app: &App, path: &Path) -> Rect {
    let row = app
        .shell
        .rows()
        .iter()
        .position(|row| row.path == path)
        .unwrap();
    app.side.layout.rect_of(SidebarHit::Row(row)).unwrap()
}
fn folders(app: &App) -> Vec<String> {
    app.shell
        .rows()
        .iter()
        .filter(|r| r.depth == 0 && r.is_dir)
        .map(|r| r.name.clone())
        .collect()
}
fn down(app: &mut App, path: &Path) -> (f32, f32) {
    let r = rect(app, path);
    let (x, y) = (r.left + 100.0, (r.top + r.bottom) / 2.0);
    app.on_click(x, y);
    assert!(app.is_dragging());
    (x, y)
}

#[test]
fn folder_drag_changes_order_through_real_click_move_and_release() {
    let (mut app, root, lib) = fixture();
    let source = lib.join("c-folder");
    let target = lib.join("a-folder");
    let r = rect(&app, &target);
    down(&mut app, &source);
    assert_eq!(app.shell.rows()[app.shell.selected().unwrap()].path, source);
    app.on_mouse_move(r.left + 100.0, r.top + 1.0);
    assert_eq!(
        app.side
            .tree_drag
            .as_ref()
            .unwrap()
            .drop
            .as_ref()
            .unwrap()
            .kind,
        SidebarTreeDropKind::Before
    );
    app.end_drag_at(r.left + 100.0, r.top + 1.0);
    assert!(!app.is_dragging());
    assert_eq!(folders(&app), ["c-folder", "a-folder", "b-folder"]);
    assert!(source.join("note.md").is_file());
    app.shell.refresh_tree();
    assert_eq!(folders(&app), ["c-folder", "a-folder", "b-folder"]);
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn row_gap_is_a_valid_drop_target_and_scrollbar_does_not_steal_tree_drag() {
    let (mut app, root, lib) = fixture();
    let target = rect(&app, &lib.join("b-folder"));
    down(&mut app, &lib.join("c-folder"));
    // 树视图仍持有鼠标捕获时，指针经过新显示的浮层。
    let x = app.side.layout.content.right - 4.0;
    app.on_mouse_move(x, target.top - 0.5);
    assert!(!app.scrollbars.interaction.dragging());
    app.end_drag_at(x, target.top - 0.5);
    assert_eq!(folders(&app), ["a-folder", "c-folder", "b-folder"]);
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn cancelled_folder_drag_never_reorders_or_toggles_on_release() {
    let (mut app, root, lib) = fixture();
    for escape in [true, false] {
        let r = rect(&app, &lib.join("a-folder"));
        down(&mut app, &lib.join("c-folder"));
        app.on_mouse_move(r.left + 100.0, r.top + 1.0);
        if escape {
            assert!(app.on_edit_key(0x1b, false, false));
        } else {
            assert!(app.cancel_sidebar_tree_drag());
        }
        assert!(!app.is_dragging());
        app.end_drag_at(r.left + 100.0, r.top + 1.0);
        assert_eq!(folders(&app), ["a-folder", "b-folder", "c-folder"]);
        assert!(app
            .shell
            .rows()
            .iter()
            .filter(|r| r.is_dir)
            .all(|r| !r.expanded));
    }
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn watcher_refresh_does_not_retarget_a_pending_folder_click() {
    let (mut app, root, lib) = fixture();
    let source = lib.join("b-folder");
    let (x, y) = down(&mut app, &source);
    let added = lib.join("0-new");
    std::fs::create_dir_all(&added).unwrap();
    std::fs::write(added.join("note.md"), "content").unwrap();
    app.shell.refresh_tree();
    app.end_drag_at(x, y);
    assert!(app
        .shell
        .rows()
        .iter()
        .any(|r| r.path == source && r.expanded));
    assert!(app
        .shell
        .rows()
        .iter()
        .any(|r| r.path == added && !r.expanded));
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn watcher_refresh_does_not_retarget_the_painted_drop_row() {
    let (mut app, root, lib) = fixture();
    let source = lib.join("c-folder");
    let target = lib.join("a-folder");
    let r = rect(&app, &target);
    down(&mut app, &source);
    app.on_mouse_move(r.left + 100.0, r.top + 1.0);
    std::fs::create_dir_all(lib.join("0-new")).unwrap();
    app.shell.refresh_tree();
    app.end_drag_at(r.left + 100.0, r.top + 1.0);
    assert_eq!(folders(&app), ["0-new", "c-folder", "a-folder", "b-folder"]);
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn choosing_manual_sort_stays_manual_and_allows_a_folder_drop() {
    let (mut app, root, lib) = fixture();
    let notices = app.notifications.view.history.entries.len();
    app.run_menu_action(MenuAction::SetSort("manual".into()));
    let descriptor = app_settings::descriptor("sidebar.sortOrder").unwrap();
    assert_eq!(
        app.app_settings.read(descriptor),
        SettingValue::Text("manual".into())
    );
    app.load_chrome_settings();
    assert_eq!(app.notifications.view.history.entries.len(), notices);
    app.paint(HWND::default()).unwrap();
    let r = rect(&app, &lib.join("a-folder"));
    down(&mut app, &lib.join("c-folder"));
    app.end_drag_at(r.left + 100.0, r.top + 1.0);
    assert_eq!(folders(&app), ["c-folder", "a-folder", "b-folder"]);
    let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))));
    assert_eq!(
        reopened.read(descriptor),
        SettingValue::Text("manual".into())
    );
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn fixed_sort_only_notifies_after_an_attempt_to_reorder() {
    let (mut app, root, lib) = fixture();
    let notices = app.notifications.view.history.entries.len();
    app.run_menu_action(MenuAction::SetSort("name".into()));
    app.load_chrome_settings();
    app.paint(HWND::default()).unwrap();
    assert_eq!(app.notifications.view.history.entries.len(), notices);
    let target = rect(&app, &lib.join("a-folder"));
    down(&mut app, &lib.join("c-folder"));
    assert_eq!(app.notifications.view.history.entries.len(), notices);
    app.end_drag_at(target.left + 100.0, target.top + 1.0);
    assert_eq!(folders(&app), ["a-folder", "b-folder", "c-folder"]);
    assert!(app.status_bar.toast.message.contains("切换为手动排序"));
    assert_eq!(app.notifications.view.history.entries.len(), notices + 1);
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}
