use super::*;
use crate::ui::navigation_preferences::{Action, HIDDEN_KEY};

fn with_fixture(test: impl FnOnce(&mut App, &Path, &crate::gfx::Snapshot)) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-nav-drag-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.shell.open_workspace(&root, || {}).unwrap();
    for name in ["资料库", "项目文档", "学习笔记"] {
        app.shell.create_library("knowledge-base", name).unwrap();
    }
    app.state.view = WorkspaceView::Editor;
    let snapshot = app.renderer.prepare_snapshot(1400, 1000, 96.0).unwrap();
    app.paint(HWND::default()).unwrap();
    test(&mut app, &root, &snapshot);
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

fn library(app: &App, name: &str) -> mochi_core::domain::Library {
    app.shell
        .workspace()
        .unwrap()
        .libraries
        .iter()
        .find(|l| l.name == name)
        .unwrap()
        .clone()
}

fn rect(app: &App, name: &str) -> Rect {
    let id = library(app, name).id;
    let index = app
        .nav
        .painted_libraries
        .iter()
        .position(|painted| painted == &id)
        .unwrap();
    app.nav_layout.rect_of(NavHit::Library(index)).unwrap()
}

fn names(app: &App) -> Vec<String> {
    app.shell
        .workspace()
        .unwrap()
        .libraries
        .iter()
        .filter(|l| l.kind == "knowledge-base")
        .map(|l| l.name.clone())
        .collect()
}

fn press(app: &mut App, name: &str) -> (f32, f32) {
    let r = rect(app, name);
    let point = (r.left + 70.0, (r.top + r.bottom) / 2.0);
    app.on_click(point.0, point.1);
    assert!(app.nav.library_drag.is_some());
    assert!(app.is_dragging());
    point
}

fn hold(app: &mut App) {
    app.nav.library_drag.as_mut().unwrap().pressed_at =
        Instant::now() - Duration::from_millis(u64::from(HOLD_MS) + 1);
    app.on_timer(HWND::default(), platform::TIMER_NAVIGATION_DRAG);
    assert!(app.navigation_library_drag_active());
}

fn save_frame(app: &mut App, snapshot: &crate::gfx::Snapshot, name: &str) {
    app.paint(HWND::default()).unwrap();
    if let Some(folder) = std::env::var_os("MOCHI_NAV_QA_DIR") {
        let folder = PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        app.renderer
            .save_snapshot(snapshot, &folder.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn short_click_selects_but_motion_before_hold_cancels_without_reordering() {
    with_fixture(|app, _, _| {
        let original = names(app);
        let selected = app.shell.selected_library();
        let point = press(app, "资料库");
        assert_eq!(
            app.shell.selected_library(),
            selected,
            "press must not switch the document"
        );
        assert!(app
            .take_timer_requests()
            .contains(&(platform::TIMER_NAVIGATION_DRAG, HOLD_MS)));
        app.end_drag_at(point.0, point.1);
        let selected = app.shell.selected_library().unwrap();
        assert_eq!(
            app.shell.workspace().unwrap().libraries[selected].name,
            "资料库"
        );
        app.paint(HWND::default()).unwrap();
        press(app, "学习笔记");
        let target = rect(app, "资料库");
        app.on_mouse_move(target.left + 70.0, target.top + 2.0);
        app.nav.library_drag.as_mut().unwrap().pressed_at = Instant::now() - Duration::from_secs(1);
        app.on_timer(HWND::default(), platform::TIMER_NAVIGATION_DRAG);
        app.end_drag_at(target.left + 70.0, target.top + 2.0);
        assert_eq!(names(app), original);
        assert_eq!(app.shell.selected_library(), Some(selected));
        assert!(!app.is_dragging());
    });
}

#[test]
fn held_drag_persists_order_and_preserves_selected_library_and_dirty_document() {
    with_fixture(|app, root, snapshot| {
        let selected = library(app, "学习笔记");
        let note = PathBuf::from(&selected.path).join("保留的笔记.md");
        std::fs::write(&note, "# 排序验证\n\n当前笔记保持打开。\n").unwrap();
        assert!(app.shell.open_file(&note));
        app.shell
            .active_buffer_mut()
            .unwrap()
            .insert("未保存内容\n");
        let content = app.shell.active_buffer_mut().unwrap().text().to_owned();
        app.paint(HWND::default()).unwrap();
        press(app, "学习笔记");
        hold(app);
        let target = rect(app, "资料库");
        app.on_mouse_move(target.left + 70.0, target.top + 2.0);
        save_frame(app, snapshot, "navigation-drag");
        app.end_drag_at(target.left + 70.0, target.top + 2.0);
        assert_eq!(names(app), ["墨池", "学习笔记", "资料库", "项目文档"]);
        let index = app.shell.selected_library().unwrap();
        assert_eq!(
            app.shell.workspace().unwrap().libraries[index].id,
            selected.id
        );
        assert_eq!(app.shell.active().unwrap().path(), Some(note.as_path()));
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), content);
        assert!(app.shell.active().unwrap().dirty());
        save_frame(app, snapshot, "navigation-reordered");
        let reopened = mochi_core::workspace::WorkspaceService::new(root)
            .unwrap()
            .open()
            .unwrap();
        let disk_names: Vec<_> = reopened
            .libraries
            .iter()
            .filter(|l| l.kind == "knowledge-base")
            .map(|l| l.name.as_str())
            .collect();
        assert_eq!(disk_names, ["墨池", "学习笔记", "资料库", "项目文档"]);
        assert!(note.is_file(), "reorder must not relocate files");

        // 向下移动时，以移除源条目后的目标位置为准。
        press(app, "学习笔记");
        hold(app);
        let target = rect(app, "项目文档");
        app.end_drag_at(target.left + 70.0, target.bottom - 2.0);
        assert_eq!(names(app), ["墨池", "资料库", "项目文档", "学习笔记"]);
    });
}

#[test]
fn escape_capture_loss_and_outside_release_do_not_reorder() {
    with_fixture(|app, _, _| {
        let original = names(app);
        for mode in [0, 1, 2] {
            press(app, "学习笔记");
            hold(app);
            let target = rect(app, "资料库");
            app.on_mouse_move(target.left + 70.0, target.top + 2.0);
            match mode {
                0 => {
                    assert!(app.on_edit_key(0x1b, false, false));
                }
                1 => {
                    assert!(app.cancel_navigation_library_drag());
                }
                _ => {
                    app.end_drag_at(800.0, target.top);
                }
            }
            assert!(!app.is_dragging());
            app.end_drag_at(target.left + 70.0, target.top + 2.0);
            assert_eq!(names(app), original);
        }
    });
}

#[test]
fn pending_click_and_drop_keep_the_painted_library_identity_after_refresh() {
    with_fixture(|app, _, _| {
        let source = library(app, "学习笔记");
        let target = library(app, "资料库");
        let point = press(app, "资料库");
        app.shell
            .reorder_library(&source.id, &target.id, false)
            .unwrap();
        app.end_drag_at(point.0, point.1);
        let index = app.shell.selected_library().unwrap();
        assert_eq!(
            app.shell.workspace().unwrap().libraries[index].id,
            target.id
        );
        app.paint(HWND::default()).unwrap();
        press(app, "学习笔记");
        hold(app);
        let drop = rect(app, "项目文档");
        // 下一帧绘制前刷新就会改变实时行号。
        app.shell
            .reorder_library(&target.id, &source.id, false)
            .unwrap();
        app.end_drag_at(drop.left + 70.0, drop.bottom - 2.0);
        assert_eq!(names(app), ["墨池", "资料库", "项目文档", "学习笔记"]);
    });
}

#[test]
fn edge_hold_scrolls_and_wheel_before_hold_cancels() {
    with_fixture(|app, _, _| {
        for i in 0..15 {
            app.shell
                .create_library("knowledge-base", &format!("更多资料{i}"))
                .unwrap();
        }
        app.paint(HWND::default()).unwrap();
        press(app, "资料库");
        hold(app);
        let area = app.nav_layout.scroll_area;
        app.on_mouse_move(area.left + 70.0, area.bottom - 5.0);
        app.on_timer(HWND::default(), platform::TIMER_NAVIGATION_DRAG);
        assert!(app.nav.scroll > 0.0);
        assert!(app
            .take_timer_requests()
            .contains(&(platform::TIMER_NAVIGATION_DRAG, 40)));
        app.cancel_navigation_library_drag();
        app.nav.scroll = 0.0;
        app.paint(HWND::default()).unwrap();
        let point = press(app, "资料库");
        let original = names(app);
        app.on_wheel(point.0, point.1, -120);
        app.end_drag_at(point.0, point.1);
        assert_eq!(names(app), original);
    });
}

#[test]
fn menu_hide_persists_in_collapsed_and_expanded_navigation_and_settings_restore_it() {
    with_fixture(|app, root, snapshot| {
        for (collapsed, item) in [(false, NavItem::Inbox), (true, NavItem::Recent)] {
            app.state.navigation_collapsed = collapsed;
            app.paint(HWND::default()).unwrap();
            let r = app.nav_layout.rect_of(NavHit::Item(item)).unwrap();
            app.on_right_click(r.left + 15.0, r.top + 15.0);
            assert_eq!(
                app.menu.as_ref().unwrap().items[0].action,
                MenuAction::HideNavigation(item)
            );
            if !collapsed {
                save_frame(app, snapshot, "navigation-hide-menu");
            }
            app.activate_menu_item(0, 0);
            app.paint(HWND::default()).unwrap();
            assert!(app.nav_layout.rect_of(NavHit::Item(item)).is_none());
            let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))));
            let value = reopened.read(app_settings::descriptor(HIDDEN_KEY).unwrap());
            assert!(
                matches!(value, SettingValue::Text(hidden) if hidden.split(',').any(|id| id == item.id()))
            );
            // 已有的设置开关可以恢复被隐藏的条目。
            app.edit_navigation_preferences(Action::Toggle(item));
            app.paint(HWND::default()).unwrap();
            assert!(app.nav_layout.rect_of(NavHit::Item(item)).is_some());
        }
    });
}

#[test]
fn reorder_preserves_other_types_and_latest_metadata_and_aborts_on_invalid_storage() {
    with_fixture(|app, root, _| {
        let source = library(app, "学习笔记");
        let target = library(app, "资料库");
        let path = mochi_core::paths::libraries_file(root);
        let mut disk = app.shell.workspace().unwrap().libraries.clone();
        let other = mochi_core::domain::Library {
            id: "other-kind".into(),
            name: "其他类型".into(),
            kind: "problem-set".into(),
            ..Default::default()
        };
        disk.insert(2, other.clone());
        disk.iter_mut().find(|l| l.id == target.id).unwrap().icon = Some("book".into());
        mochi_core::json2::write(&path, &disk).unwrap();
        assert!(app
            .shell
            .reorder_library(&source.id, &target.id, false)
            .unwrap());
        assert_eq!(app.shell.workspace().unwrap().libraries[2], other);
        assert_eq!(library(app, "资料库").icon.as_deref(), Some("book"));
        assert!(app
            .shell
            .reorder_library(&source.id, &other.id, false)
            .is_err());
        let original = names(app);
        std::fs::write(&path, "broken json").unwrap();
        assert!(app
            .shell
            .reorder_library(&target.id, &source.id, false)
            .is_err());
        assert_eq!(names(app), original);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken json");
    });
}
