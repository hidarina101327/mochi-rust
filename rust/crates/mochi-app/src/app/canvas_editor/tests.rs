use super::*;
use canvas_view::{Hit, Tool};

fn with_canvas(test: impl FnOnce(&mut App, &Path)) {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    let _com = Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
    let root = std::env::temp_dir().join(format!(
        "mochi-canvas-test-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let path = root.join("自由笔记.mcanvas");
        std::fs::write(&path, r#"{"version":1,"cards":[]}"#).unwrap();
        assert!(app.shell.open_file(&path));
        app.state.view = WorkspaceView::Editor;
        app.focus = Focus::Main;
        app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.sync_state();
        app.paint(HWND::default()).unwrap();
        test(&mut app, &path);
    }
    std::fs::remove_dir_all(root).unwrap();
}

fn click_tool(app: &mut App, hit: Hit) {
    app.paint(HWND::default()).unwrap();
    let rect = app
        .viewer_layout
        .entries
        .iter()
        .find(|(_, h)| *h == viewer::Hit::Canvas(hit))
        .unwrap()
        .0;
    app.on_click(
        (rect.left + rect.right) / 2.0,
        (rect.top + rect.bottom) / 2.0,
    );
    app.paint(HWND::default()).unwrap();
}
fn document(app: &App) -> &mochi_core::canvas::CanvasDocument {
    match app.viewer_tab().unwrap().1 {
        viewer::Content::Canvas(s) => &s.document,
        _ => panic!("canvas not active"),
    }
}

fn canvas_host(app: &mut App, path: &Path) -> std::sync::mpsc::Receiver<()> {
    let (tx, rx) = std::sync::mpsc::channel();
    let permissions = Arc::new(AiPermissionService::new(path.parent().unwrap()));
    permissions
        .set_folder_permission(
            &path.parent().unwrap().to_string_lossy(),
            mochi_core::ai::permission::AiPermissionLevel::Modify,
        )
        .unwrap();
    let active = app
        .ai_active_document()
        .expect("a canvas must be part of AI context");
    assert_eq!(Path::new(&active.path), path);
    let snapshot = Arc::new(Mutex::new(HostSnapshot {
        active_document: Some(active),
        ..Default::default()
    }));
    app.ai.host = Some(Arc::new(AppHost::new(
        path.parent().unwrap(),
        snapshot.clone(),
        permissions.clone(),
        move || {
            let _ = tx.send(());
        },
    )));
    app.ai.snapshot = snapshot;
    app.ai.permissions = Some(permissions);
    rx
}

fn canvas_call(
    app: &mut App,
    wake: &std::sync::mpsc::Receiver<()>,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    use mochi_core::ai::{
        models::{AiToolCall, AiToolFunction},
        tools::{canvas_tools::CanvasToolExecutor, ToolRegistry},
    };
    let registry = ToolRegistry::new(app.ai.permissions.clone().unwrap()).with(Arc::new(
        CanvasToolExecutor::new(app.ai.host.clone().unwrap()),
    ));
    let name = name.to_owned();
    let job = std::thread::spawn(move || {
        registry.execute(&AiToolCall {
            id: "draw-test".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name,
                arguments: args.to_string(),
            },
        })
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !job.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "canvas tool did not finish"
        );
        if wake
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_ok()
        {
            app.on_files_changed();
        }
    }
    serde_json::from_str(&job.join().unwrap()).unwrap()
}

#[test]
fn canvas_ai_tools_draw_save_render_and_undo_through_the_real_host() {
    with_canvas(|app, path| {
        let wake = canvas_host(app, path);
        let before = document(app).clone();
        let context = canvas_call(app, &wake, "canvas_get", serde_json::json!({}));
        assert_eq!(context["ok"], true);
        assert!(
            context["data"]["suggestedBounds"]["width"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        let drawing: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../mochi-core/src/canvas/drawing/cat.json"
        )))
        .unwrap();
        let args =
            serde_json::json!({"expectedRevision":context["data"]["revision"],"drawing":drawing});
        let result = canvas_call(app, &wake, "canvas_draw", args.clone());
        assert_eq!(result["ok"], true, "{result}");
        let after = document(app).clone();
        assert!(after.strokes.len() > 10);
        assert_eq!(after.texts[0].text, "今天也要开心");
        assert_eq!(
            mochi_core::canvas::parse(&std::fs::read_to_string(path).unwrap()).unwrap(),
            after
        );
        assert_eq!(
            result["data"]["revision"],
            mochi_core::canvas::drawing::revision(&after)
        );
        assert_eq!(canvas_call(app, &wake, "canvas_draw", args)["ok"], false);
        assert_eq!(document(app), &after);
        for (name, width, dark) in [
            ("ai-light", 780, false),
            ("ai-dark", 780, true),
            ("ai-narrow", 360, false),
        ] {
            let snapshot = app.renderer.prepare_snapshot(width, 480, 96.0).unwrap();
            let area = Rect::from_size(0.0, 0.0, width as f32, 480.0);
            let mut s = match app.viewer_tab().unwrap().1 {
                viewer::Content::Canvas(s) => s.clone(),
                _ => unreachable!(),
            };
            let layout = canvas_view::layout(&s, area);
            if width < 600 {
                s.fit_to_content(layout.body);
            }
            let palette = theme::configured_palette(dark);
            let mut list = DrawList::new();
            canvas_view::paint(&mut list, area, &s, &layout, None, &palette);
            list.finish().unwrap();
            app.renderer
                .present(HWND::default(), palette.background, &list)
                .unwrap();
            if let Some(output) = std::env::var_os("MOCHI_CANVAS_QA_DIR") {
                let dir = PathBuf::from(output);
                std::fs::create_dir_all(&dir).unwrap();
                app.renderer
                    .save_snapshot(&snapshot, &dir.join(format!("canvas-{name}.png")))
                    .unwrap();
            }
        }
        app.focus = Focus::Main;
        assert!(app.canvas_key(0x5a, false, true));
        assert_eq!(document(app), &before);
        assert_eq!(
            mochi_core::canvas::parse(&std::fs::read_to_string(path).unwrap()).unwrap(),
            before
        );
        assert!(app.canvas_key(0x5a, true, true));
        assert_eq!(document(app), &after);
    });
}

#[test]
fn canvas_ai_keeps_original_target_after_tab_switch_and_rejects_external_changes() {
    with_canvas(|app, path| {
        let wake = canvas_host(app, path);
        let context = canvas_call(app, &wake, "canvas_get", serde_json::json!({}));
        let other = path.parent().unwrap().join("另一个画布.mcanvas");
        std::fs::write(&other, r#"{"version":2,"cards":[]}"#).unwrap();
        assert!(app.shell.open_file_with_mode(&other, true));
        app.sync_state();
        let args = serde_json::json!({"expectedRevision":context["data"]["revision"],"drawing":{"texts":[{"text":"只写在原画布","x":30,"y":40}]}});
        assert_eq!(canvas_call(app, &wake, "canvas_draw", args)["ok"], true);
        assert!(document(app).texts.is_empty());
        let original = mochi_core::canvas::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(original.texts[0].text, "只写在原画布");
        let context = canvas_call(app, &wake, "canvas_get", serde_json::json!({}));
        let mut outside = original.clone();
        outside.texts[0].text = "外部修改".into();
        std::fs::write(path, mochi_core::canvas::serialize(&outside).unwrap()).unwrap();
        let result = canvas_call(
            app,
            &wake,
            "canvas_draw",
            serde_json::json!({"expectedRevision":context["data"]["revision"],"drawing":{"texts":[{"text":"不能覆盖","x":0,"y":0}]}}),
        );
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("磁盘"));
        assert_eq!(
            mochi_core::canvas::parse(&std::fs::read_to_string(path).unwrap()).unwrap(),
            outside
        );
    });
}

#[test]
fn canvas_ai_respects_folder_permissions_and_explicit_auto_apply() {
    use mochi_core::ai::permission::AiPermissionLevel;
    with_canvas(|app, path| {
        let wake = canvas_host(app, path);
        let permissions = app.ai.permissions.clone().unwrap();
        let root = path.parent().unwrap().to_string_lossy();
        permissions
            .set_folder_permission(&root, AiPermissionLevel::Suggest)
            .unwrap();
        if let Some(viewer::Content::Canvas(s)) = app.viewer_content_mut() {
            s.document.viewport.zoom = 4.0;
            s.document.viewport.x = -200.0;
        }
        let context = canvas_call(app, &wake, "canvas_get", serde_json::json!({}));
        let bounds = &context["data"]["suggestedBounds"];
        assert_eq!(bounds["x"], -194.0);
        assert!(
            bounds["width"].as_f64().unwrap() <= f64::from(app.viewer_layout.body.width()) / 4.0
        );
        let args = serde_json::json!({"expectedRevision":context["data"]["revision"],"drawing":{"texts":[{"text":"允许后绘制","x":-194,"y":20}]}});
        assert_eq!(
            canvas_call(app, &wake, "canvas_draw", args.clone())["ok"],
            false
        );
        assert!(document(app).texts.is_empty());
        app.ai.snapshot.lock().unwrap().edit_apply_mode = "auto".into();
        let result = canvas_call(app, &wake, "canvas_draw", args.clone());
        assert_eq!(result["ok"], true, "{result}");
        permissions
            .set_folder_permission(&root, AiPermissionLevel::ReadOnly)
            .unwrap();
        assert_eq!(canvas_call(app, &wake, "canvas_draw", args)["ok"], false);
        assert_eq!(document(app).texts.len(), 1);
    });
}

#[test]
fn canvas_routes_pointer_ime_clipboard_and_undo_to_the_saved_document() {
    with_canvas(|app, path| {
        click_tool(app, Hit::Tool(Tool::Pen));
        let body = app.viewer_layout.body;
        app.on_click(body.left + 80.0, body.top + 80.0);
        app.on_mouse_move(body.left + 130.0, body.top + 90.0);
        app.end_drag_at(body.left + 180.0, body.top + 100.0);
        assert_eq!(document(app).strokes[0].points.len(), 3);
        assert_eq!(document(app).strokes[0].points.last().unwrap().x, 180.0);
        click_tool(app, Hit::Tool(Tool::Text));
        let body = app.viewer_layout.body;
        app.on_click(body.left + 230.0, body.top + 120.0);
        app.end_drag();
        assert_eq!(app.focus, Focus::CanvasText);
        app.on_ime_composition("中文", 2);
        assert!(document(app).texts[0].text.is_empty());
        app.on_ime_commit("中文😀");
        assert!(app.on_edit_key(13, false, false));
        assert!(app.insert_clipboard_content("第二行\r\n第三行", true));
        app.paint(HWND::default()).unwrap();
        assert!(app.caret_in_client().is_some());
        assert!(app.on_accelerator(HWND::default(), 13, false, true, false));
        assert_eq!(document(app).texts[0].text, "中文😀\n第二行\n第三行");
        let saved = mochi_core::canvas::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(&saved, document(app));
        assert!(app.on_accelerator(HWND::default(), 0x5a, false, true, false));
        assert!(document(app).texts.is_empty());
        assert_eq!(document(app).strokes.len(), 1);
        assert!(app.on_accelerator(HWND::default(), 0x5a, true, true, false));
        assert_eq!(document(app).texts.len(), 1);
        app.focus = Focus::AiInput;
        assert!(!app.canvas_key(0x2e, false, false));
        assert_eq!(document(app).texts.len(), 1);
        app.focus = Focus::Main;
        if let Some(viewer::Content::Canvas(state)) = app.viewer_content_mut() {
            state.edit_text(0);
        }
        app.focus = Focus::CanvasText;
        assert!(!app.canvas_key(0x09, false, true));
        assert_eq!(app.focus, Focus::Main);
    });
}

#[test]
fn canvas_real_d2d_previews_cover_light_dark_and_narrow_layouts() {
    with_canvas(|app, _| {
        let mut state = canvas_view::State::parse(r#"{"version":1,"cards":[]}"#).unwrap();
        for (point, size, color, value) in [
            ((52.0, 46.0), 2, 0, "让想法自由生长"),
            (
                (56.0, 114.0),
                0,
                0,
                "文字、涂鸦与资料，在同一张无限画布上。\n想到哪里，就记到哪里。",
            ),
            ((84.0, 262.0), 1, 1, "01  捕捉灵感\n随手写下一个念头"),
            ((530.0, 262.0), 1, 3, "02  连接想法\n让思考慢慢展开"),
        ] {
            state.font_size = size;
            state.color = color;
            state.create_text(mochi_core::canvas::Point {
                x: point.0,
                y: point.1,
            });
            if point.1 < 200.0 {
                let index = state.editor.as_ref().unwrap().index;
                state.document.texts[index].width = 700.0;
            }
            state.editor.as_mut().unwrap().field.buffer.insert(value);
            state.finish_editing();
        }
        let body = Rect::from_size(0.0, 0.0, 960.0, 620.0);
        state.color = 1;
        state.width = 0;
        state.set_tool(Tool::Pen);
        for points in [
            vec![
                (390.0, 320.0),
                (425.0, 310.0),
                (465.0, 312.0),
                (494.0, 321.0),
            ],
            vec![(479.0, 307.0), (494.0, 321.0), (478.0, 329.0)],
            vec![
                (75.0, 204.0),
                (151.0, 210.0),
                (233.0, 206.0),
                (304.0, 210.0),
            ],
        ] {
            state.pointer_down(body, points[0].0, points[0].1, false);
            for p in points {
                state.pointer_move(body, p.0, p.1);
            }
            state.release();
        }
        state.set_tool(Tool::Select);
        for (name, width, dark) in [
            ("light", 960, false),
            ("dark", 960, true),
            ("narrow", 360, false),
            ("editing", 960, false),
        ] {
            let snapshot = app.renderer.prepare_snapshot(width, 620, 96.0).unwrap();
            let area = Rect::from_size(0.0, 0.0, width as f32, 620.0);
            let mut s = state.clone();
            if name == "editing" {
                s.edit_text(1);
                s.editor
                    .as_mut()
                    .unwrap()
                    .field
                    .buffer
                    .set_composition("继续记录", 4);
            }
            let layout = canvas_view::layout(&s, area);
            if width < 600 {
                s.fit_to_content(layout.body);
            }
            let palette = theme::configured_palette(dark);
            let mut list = DrawList::new();
            canvas_view::paint(&mut list, area, &s, &layout, None, &palette);
            list.finish().unwrap();
            app.renderer
                .present(HWND::default(), palette.background, &list)
                .unwrap();
            if let Some(output) = std::env::var_os("MOCHI_CANVAS_QA_DIR") {
                let dir = PathBuf::from(output);
                std::fs::create_dir_all(&dir).unwrap();
                app.renderer
                    .save_snapshot(&snapshot, &dir.join(format!("canvas-{name}.png")))
                    .unwrap();
            }
        }
    });
}
