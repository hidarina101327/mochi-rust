use super::*;
use crate::ui::draw::DrawCmd;

#[test]
fn rendered_editor_exposes_the_text_cursor_across_its_editable_body() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-editor-cursor-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let path = root.join("cursor.md");
        std::fs::write(&path, "正文\n").unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        assert!(app.shell.open_file(&path));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();

        let x = app.editor_area.left + app.editor_area.width() * 0.5;
        let y = app.editor_area.bottom - 24.0;
        assert_eq!(
            app.cursor_for(x, y),
            Some(windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Explicit native visual acceptance; writes isolated snapshots to target/ux-acceptance"]
fn native_visual_acceptance_snapshots() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ux-acceptance");
    std::fs::create_dir_all(&output).unwrap();
    for scenario in [
        "home-favorites-dark",
        "home-favorites-dark-narrow",
        "settings-overlay-dark",
        "base-detail-dark",
        "ai-parity-dark",
        "ai-parity-dark-narrow",
    ] {
        App::snapshot(scenario, &output.join(format!("{scenario}.png"))).unwrap();
    }
}

#[test]
fn object_picker_selects_multiple_documents_and_persists_base_references() {
    use crate::ui::object_picker as picker;
    use mochi_core::base::*;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-object-flow-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let library = app
            .shell
            .create_library("knowledge-base", "引用测试")
            .unwrap();
        let folder = PathBuf::from(&app.shell.workspace().unwrap().libraries[library].path);
        for name in ["参考一.md", "参考二.mc"] {
            std::fs::write(folder.join(name), "# 内容\n\n测试正文").unwrap();
        }
        let mut base = create_base_document();
        let field = create_base_field(FieldType::Document, "文档");
        let field_id = field.id.clone();
        base.tables[0].fields.push(field);
        base.tables[0].records.push(BaseRecord {
            id: "row-one".into(),
            ..Default::default()
        });
        let path = folder.join("资料表.mcb");
        std::fs::write(&path, serialize_base_document(&base).unwrap()).unwrap();
        assert!(app.shell.open_file(&path));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        let snapshot = app.renderer.prepare_snapshot(1440, 900, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        app.pick_base_references(0, 1);
        for _ in 0..200 {
            app.poll_object_picker();
            if !app.object_picker.as_ref().unwrap().loading {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.object_picker.as_ref().unwrap().loading);
        for ch in "参考".chars() {
            assert!(app.on_char(ch));
        }
        assert_eq!(app.object_picker.as_ref().unwrap().state.visible_count(), 2);
        for row in 0..2 {
            let layout = picker::layout(
                &app.object_picker.as_ref().unwrap().state,
                app.renderer.viewport(),
            );
            let rect = layout
                .entries
                .iter()
                .find(|(_, hit)| *hit == picker::Hit::Candidate(row))
                .unwrap()
                .0;
            app.on_click(rect.left + 30.0, rect.top + 20.0);
        }
        assert_eq!(app.object_picker.as_ref().unwrap().state.selected.len(), 2);
        app.paint(HWND::default()).unwrap();
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ux-acceptance");
        std::fs::create_dir_all(&output).unwrap();
        app.renderer
            .save_snapshot(&snapshot, &output.join("object-picker.png"))
            .unwrap();
        let layout = picker::layout(
            &app.object_picker.as_ref().unwrap().state,
            app.renderer.viewport(),
        );
        let confirm = layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == picker::Hit::Confirm)
            .unwrap()
            .0;
        app.on_click(confirm.left + 10.0, confirm.top + 10.0);
        assert!(app.object_picker.is_none());
        assert!(app.save_active());
        let saved = parse_base_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let refs = saved.tables[0].records[0].values[&field_id]
            .as_array()
            .unwrap();
        assert_eq!(refs.len(), 2);
        for reference in refs {
            assert!(mochi_core::object_reference::ObjectReference::parse(
                reference.as_str().unwrap()
            )
            .unwrap()
            .resolve_path(Some(&root))
            .unwrap()
            .is_file());
        }
        let before = std::fs::read_to_string(&path).unwrap();
        app.pick_base_references(0, 1);
        app.on_click(2.0, 2.0);
        assert!(app.object_picker.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        app.settings.set("app.editorLayout.cardEnabled", "true");
        app.settings.set("app.background.uiOpacity", "25");
        app.load_chrome_settings();
        app.paint(HWND::default()).unwrap();
        assert!(app.list.cmds().iter().any(|command|matches!(command,DrawCmd::RoundedRectAlpha{rect,alpha,..}if *rect==app.editor_area&&(*alpha-0.25).abs()<0.001)));
        assert!(!app
            .list
            .cmds()
            .iter()
            .any(|command| matches!(command,DrawCmd::Rect{rect,..}if *rect==app.editor_area)));
        if let Some(viewer::Content::Base(state)) = app.viewer_content_mut() {
            state.editing = true;
        }
        app.on_base_click(base_view::Hit::NewTemporaryDocument);
        app.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("临时内容");
        let action = app
            .dialog
            .as_ref()
            .unwrap()
            .buttons
            .last()
            .unwrap()
            .action
            .clone();
        app.run_dialog_action(action);
        let temporary = folder.join("资料表.documents/临时内容.md");
        assert_eq!(app.active_file_path().as_deref(), Some(temporary.as_path()));
        assert_eq!(
            std::fs::read_to_string(temporary).unwrap(),
            "# 临时内容\n\n"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_modal_routes_menus_and_drag_selection_above_the_document() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-modal-input-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        let _snapshot = app.renderer.prepare_snapshot(1440, 900, 96.0).unwrap();
        app.open_settings("ai");
        app.prefs.providers.form = Some(providers::Form::new(mochi_core::ai::models::AiProvider {
            name: "Provider selection sample".into(),
            base_url: "https://example.invalid".into(),
            model: "mock".into(),
            api_key: "fake-key-for-test".into(),
            protocol: "openai-completions".into(),
            ..Default::default()
        }));
        app.paint(HWND::default()).unwrap();
        let close = Rect::from_size(
            app.settings_overlay_rect().right - 42.0,
            app.settings_overlay_rect().top + 10.0,
            32.0,
            32.0,
        );
        for (rect, hit) in &app.prefs.provider_layout.entries {
            if matches!(hit, providers::Hit::Add | providers::Hit::Options) {
                assert!(rect.intersect(&close).is_empty());
            }
        }
        let rect = app.prefs.provider_layout.field(0).unwrap();
        app.on_click(rect.left + 12.0, (rect.top + rect.bottom) / 2.0);
        app.on_mouse_move(rect.left + 100.0, (rect.top + rect.bottom) / 2.0);
        assert!(app.prefs.providers.form.as_ref().unwrap().fields[0]
            .buffer
            .has_selection());
        app.end_drag();

        app.open_settings("general");
        app.paint(HWND::default()).unwrap();
        let index = app_settings::descriptors()
            .iter()
            .position(|d| d.key == "tabs.openFileBehavior")
            .unwrap();
        let rect = app.prefs.content_layout.control_rect(index).unwrap();
        app.prefs.scroll = (rect.top - app.prefs.content_layout.body.top - 80.0).max(0.0);
        app.paint(HWND::default()).unwrap();
        let rect = app.prefs.content_layout.control_rect(index).unwrap();
        app.on_click(rect.left + 8.0, (rect.top + rect.bottom) / 2.0);
        let menu = app.menu.as_ref().expect("setting opens a menu");
        let point = (menu.rect.left + 16.0, menu.rect.top + 16.0);
        let choice = menu.items[menu.hit(point.0, point.1).unwrap()]
            .action
            .clone();
        let MenuAction::SetEnum(_, value) = choice else {
            panic!("enum option")
        };
        app.on_click(point.0, point.1);
        assert!(app.menu.is_none());
        assert_eq!(
            app.app_settings
                .read(&app_settings::descriptors()[index])
                .to_storage(),
            value
        );
        app.on_click(rect.left + 8.0, (rect.top + rect.bottom) / 2.0);
        assert!(app.menu.is_some());
        let modal = app.settings_overlay_rect();
        app.on_click(modal.left + 8.0, modal.bottom - 8.0);
        assert!(app.menu.is_none());
        assert!(
            app.settings_overlay.is_some(),
            "outside-menu click dismisses only the menu"
        );
        app.on_wheel(rect.left, rect.top, -120);
        assert!(app.prefs.scroll > 0.0);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn force_append_keeps_replace_preference_and_pinned_tabs_safe() {
    let root = std::env::temp_dir().join(format!(
        "mochi-tab-click-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.settings.set("app.tabs.openFileBehavior", "replace");
        app.load_chrome_settings();
        for name in ["one.md", "two.md", "three.md", "four.md"] {
            std::fs::write(root.join(name), name).unwrap();
        }
        assert!(app.shell.open_file(&root.join("one.md")));
        assert!(app.shell.open_file(&root.join("two.md")));
        assert_eq!(app.shell.tabs().len(), 1);
        assert!(app.shell.open_file_with_mode(&root.join("three.md"), true));
        assert_eq!(app.shell.tabs().len(), 2);
        assert_eq!(
            crate::ui::settings_values::text("tabs.openFileBehavior", ""),
            "replace"
        );
        app.shell.active_mut().unwrap().pinned = true;
        assert!(app.shell.open_file(&root.join("four.md")));
        assert_eq!(app.shell.tabs().len(), 3);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn simple_document_mode_changes_new_documents_without_changing_rename_formats() {
    let root = std::env::temp_dir().join(format!(
        "mochi-document-mode-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let parent = app.shell.tree_root().unwrap();
        app.run_command_action(CommandAction::NewFile);
        assert!(app.dialog.is_none(), "新建文档不应再要求先填写名称");
        let plain = parent.join("未命名文档.md");
        assert_eq!(app.active_file_path().as_deref(), Some(plain.as_path()));
        assert_eq!(std::fs::read_to_string(&plain).unwrap(), "# 未命名文档\n\n");

        app.settings.set("app.editor.simpleDocumentMode", "false");
        app.load_chrome_settings();
        app.start_sidebar_edit(
            EditKind::NewFile {
                parent: parent.clone(),
            },
            "完整笔记",
        );
        assert!(app.confirm_sidebar_edit());
        let full = parent.join("完整笔记.mc");
        assert_eq!(app.active_file_path().as_deref(), Some(full.as_path()));
        app.settings.set("app.editor.simpleDocumentMode", "true");
        app.load_chrome_settings();

        let row = app
            .shell
            .rows()
            .iter()
            .position(|row| row.path == full)
            .unwrap();
        app.start_sidebar_edit(EditKind::Rename { row }, "重新命名");
        assert!(app.confirm_sidebar_edit());
        assert!(parent.join("重新命名.mc").is_file());
        assert!(!parent.join("重新命名.md").exists());

        app.open_create_dialog(parent.clone(), false);
        app.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("显式格式.mc");
        app.run_dialog_action(DialogAction::CreateInDialog {
            parent: parent.clone(),
            folder: false,
        });
        assert!(parent.join("显式格式.mc").is_file());

        app.open_create_dialog(parent.clone(), false);
        app.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("子笔记");
        app.run_dialog_action(DialogAction::CreateSubdocument(plain.clone()));
        let child = app.active_file_path().unwrap();
        assert_eq!(child.file_name().unwrap(), "子笔记.md");
        assert!(child.parent().unwrap().ends_with(".未命名文档.md.sub"));
        assert!(plain.is_file());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn moving_rendered_cursor_cancels_inline_prediction_without_invalidating_stats() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }

    let root = std::env::temp_dir().join(format!(
        "mochi-inline-cursor-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();

        let path = root.join("知识库").join("inline.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "前文\n后文";
        std::fs::write(&path, original).unwrap();
        assert!(app.shell.open_file(&path));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        app.editor_engaged = true;

        let at = original.find('\n').unwrap();
        app.shell.active_buffer_mut().unwrap().set_cursor(at, false);

        // 保留真实 App 布局和渲染流程，但不创建窗口。
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        assert!(app
            .take_timer_requests()
            .iter()
            .any(|(id, _)| *id == platform::TIMER_WORD_COUNT));
        app.paint(HWND::default()).unwrap();
        assert!(!app
            .take_timer_requests()
            .iter()
            .any(|(id, _)| *id == platform::TIMER_WORD_COUNT));
        app.on_timer(HWND::default(), platform::TIMER_WORD_COUNT);
        let stats_before = app.status_bar.stats.clone();
        assert!(stats_before.is_some());

        for key in [
            windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT.0,
            windows::Win32::UI::Input::KeyboardAndMouse::VK_RIGHT.0,
        ] {
            app.shell.active_buffer_mut().unwrap().set_cursor(at, false);
            let ghost = editor_ai::Suggestion {
                snapshot: editor_ai::Snapshot {
                    path: path.clone(),
                    hash: editor_ai::fingerprint(original),
                    range: at..at,
                    kind: editor_ai::Kind::Inline,
                    prefix: original[..at].to_owned(),
                    suffix: original[at..].to_owned(),
                },
                text: "补全内容".into(),
            };
            app.editor_ai.ghost = Some(ghost);
            app.editor_ai.timer = Some(750);

            assert!(app.on_edit_key(key, false, false));
            assert!(app.editor_ai.ghost.is_none());
            assert!(app.editor_ai.timer.is_none());
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().text(),
                original,
                "cursor movement must not change the document body"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
            assert_eq!(app.status_bar.stats, stats_before);
            assert!(!app
                .take_timer_requests()
                .iter()
                .any(|(id, _)| *id == platform::TIMER_EDITOR_AI));
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
