use super::*;
use crate::ui::workflows::Hit as WorkflowHit;
use mochi_core::workflows::{self as core, Store};

fn fixture() -> (App, PathBuf) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(core::new_id("mochi-menu-routing"));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.shell.open_workspace(&root, || {}).unwrap();
    app.workflows.root = Some(root.clone());
    app.workflows.store = Some(Store::open(&root).unwrap());
    (app, root)
}

fn save(app: &mut App, snapshot: &crate::gfx::Snapshot, name: &str) {
    app.paint(HWND::default()).unwrap();
    if let Some(dir) = std::env::var_os("MOCHI_MENU_TEST_ARTIFACTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.renderer
            .save_snapshot(snapshot, &dir.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn workflow_variable_menu_scrolls_to_last_without_moving_or_editing_the_page() {
    for dpi in [96.0, 144.0] {
        let (mut app, root) = fixture();
        let snapshot = app.renderer.prepare_snapshot(1200, 900, dpi).unwrap();
        app.state.view = WorkspaceView::Automations;
        app.workflows_action(WorkflowHit::New).unwrap();
        for i in 0..60 {
            app.workflows.view.draft.as_mut().unwrap().defaults[format!("输入_{i:02}")] =
                serde_json::json!(i);
        }
        app.workflows.view.select_node(1);
        app.paint(HWND::default()).unwrap();
        app.workflows_action(WorkflowHit::VariablePicker("/inputs/result".into()))
            .unwrap();
        save(&mut app, &snapshot, &format!("variables-{dpi}-open"));
        let menu = app.menu.as_ref().unwrap();
        assert!(menu.items.len() > 60);
        let last = menu.items.len() - 1;
        let expected = match &menu.items[last].action {
            MenuAction::WorkflowAction(WorkflowHit::SetVariable(_, _, value)) => value.clone(),
            action => panic!("unexpected variable action: {action:?}"),
        };
        let rect = menu.rect;
        let point = (rect.left + 30.0, rect.bottom - 12.0);
        let before = (
            app.workflows.view.pan,
            app.workflows.view.form_scroll,
            app.workflows.view.panel_scroll,
        );
        let draft = app.workflows.view.draft.clone();
        for _ in 0..50 {
            app.on_wheel(point.0, point.1, -120);
        }
        assert_eq!(
            before,
            (
                app.workflows.view.pan,
                app.workflows.view.form_scroll,
                app.workflows.view.panel_scroll
            )
        );
        assert_eq!(draft, app.workflows.view.draft);
        let row = app.menu.as_ref().unwrap().item_rect(last).unwrap();
        assert!(row.bottom <= rect.bottom);
        let click = (row.left + 20.0, row.bottom - 2.0);
        app.on_mouse_move(click.0, click.1);
        assert_eq!(app.menu.as_ref().unwrap().hover, Some(last));
        save(&mut app, &snapshot, &format!("variables-{dpi}-last"));
        app.on_click(click.0, click.1);
        assert!(app.menu.is_none());
        assert_eq!(
            app.workflows.view.draft.as_ref().unwrap().nodes[1].inputs["result"],
            expected
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn provider_click(app: &mut App, hit: providers::Hit) {
    for _ in 0..30 {
        let rect = app
            .prefs
            .provider_layout
            .entries
            .iter()
            .find(|(_, h)| *h == hit)
            .unwrap()
            .0;
        let body = app.prefs.provider_layout.body;
        if matches!(hit, providers::Hit::Add | providers::Hit::Options)
            || (rect.top >= body.top && rect.bottom <= body.bottom)
        {
            app.on_click(
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0,
            );
            app.paint(HWND::default()).unwrap();
            return;
        }
        app.on_wheel(
            body.left + 20.0,
            body.top + 20.0,
            if rect.top < body.top { 120 } else { -120 },
        );
        app.paint(HWND::default()).unwrap();
    }
    panic!("provider control was never visible: {hit:?}");
}

#[test]
fn provider_menu_drag_search_ime_and_keyboard_stay_inside_the_settings_overlay() {
    let (mut app, root) = fixture();
    let snapshot = app.renderer.prepare_snapshot(1024, 800, 96.0).unwrap();
    app.open_settings("ai");
    app.paint(HWND::default()).unwrap();
    provider_click(&mut app, providers::Hit::Add);
    provider_click(&mut app, providers::Hit::Preset);
    save(&mut app, &snapshot, "providers-open");
    let rect = app.menu.as_ref().unwrap().rect;
    let scroll = app.prefs.providers.scroll;
    // 12 DIP 宽的轨道是拖拽目标，绝不是提供方选项。
    app.on_click(rect.right - 7.0, rect.bottom - 15.0);
    assert!(app.is_dragging());
    app.on_mouse_move(rect.right - 7.0, 1000.0);
    app.end_drag_at(rect.right - 7.0, 1000.0);
    assert!(!app.is_dragging());
    assert_eq!(app.prefs.providers.scroll, scroll);
    assert!(app.menu.is_some());
    save(&mut app, &snapshot, "providers-scrolled");
    app.on_ime_commit("DeepSeek");
    assert_eq!(
        app.menu.as_ref().unwrap().search.as_ref().unwrap().text(),
        "DeepSeek"
    );
    app.on_edit_key(40, false, false);
    let menu = app.menu.as_ref().unwrap();
    let selected = menu.hover.unwrap();
    assert_eq!(menu.items[selected].label, "DeepSeek");
    let row = menu.item_rect(selected).unwrap();
    assert!(row.top >= menu.search_rect().unwrap().bottom && row.bottom <= menu.rect.bottom);
    save(&mut app, &snapshot, "providers-filtered");
    app.on_edit_key(13, false, false);
    assert!(app.menu.is_none());
    assert!(app.settings_overlay.is_some());
    assert_eq!(
        app.prefs.providers.form.as_ref().unwrap().value().name,
        "DeepSeek"
    );
    assert!(
        app.prefs.providers.all.is_empty(),
        "selection must not save a new provider"
    );
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
