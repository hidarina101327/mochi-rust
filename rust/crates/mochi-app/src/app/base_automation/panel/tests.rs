use super::*;

fn with_app(test: impl FnOnce(&mut App, &Path)) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(model::create_id("mochi-automation-ui"));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("test.mcb");
    std::fs::write(
        &path,
        include_str!("../../../../../../../tests/fixtures/base-v1.mcb"),
    )
    .unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        assert!(app.shell.open_file_with_mode(&path, true));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        app.renderer.prepare_snapshot(1280, 850, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        app.on_base_click(base_view::Hit::Automations);
        assert!(app.automation.panel.is_some());
        test(&mut app, &path);
    }
    let _ = std::fs::remove_dir_all(&root);
}
fn click(app: &mut App, hit: Hit) {
    let viewport = app.renderer.viewport();
    let p = app.automation.panel.as_mut().unwrap();
    let lay = p.layout(viewport);
    let (rect, _) = lay.rows.iter().find(|(_, r)| r.hit == Some(hit)).unwrap();
    p.scroll = (p.scroll + rect.top - lay.body.top).clamp(0.0, lay.max_scroll);
    let lay = p.layout(viewport);
    let (rect, _) = lay.rows.iter().find(|(_, r)| r.hit == Some(hit)).unwrap();
    let r = rect.intersect(&lay.body);
    assert!(!r.is_empty(), "{hit:?}");
    app.on_click(r.left + 4.0, r.top + 4.0);
}
fn valid_draft(app: &mut App, trigger: Trigger) {
    click(app, Hit::New);
    let p = app.automation.panel.as_mut().unwrap();
    let r = p.draft.as_mut().unwrap();
    r.name = "标记完成".into();
    r.trigger = trigger;
    r.actions = vec![Action::UpdateRecord {
        values: BTreeMap::from([(
            "checked".into(),
            Input::Literal {
                value: serde_json::json!(true),
            },
        )]),
    }];
    p.record = Some("record_a".into());
}

#[test]
fn builder_can_preview_save_enable_pause_without_mutating_preview_records() {
    with_app(|app, path| {
        let original = std::fs::read_to_string(path).unwrap();
        valid_draft(app, Trigger::RecordCreated);
        click(app, Hit::Test);
        assert!(app.automation.panel.as_ref().unwrap().preview.is_some());
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
        click(app, Hit::Save);
        let p = app.automation.panel.as_ref().unwrap();
        assert!(!p.draft.as_ref().unwrap().enabled);
        assert!(!p.changed());
        assert!(grants(&app.settings).is_empty());
        click(app, Hit::Enable);
        let p = app.automation.panel.as_ref().unwrap();
        assert!(p.draft.as_ref().unwrap().enabled);
        assert!(!p.changed());
        assert_eq!(grants(&app.settings).len(), 1);
        let parsed = model::parse_base_document(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(parsed.tables[0].records[0].values["checked"], false);
        click(app, Hit::Pause);
        assert!(grants(&app.settings).is_empty());
        assert!(
            !app.automation
                .panel
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .enabled
        );
    });
}
#[test]
fn preview_run_once_updates_open_buffer_logs_and_preserves_scroll() {
    with_app(|app, path| {
        if let Some(viewer::Content::Base(s)) = app.viewer_content_mut() {
            s.scroll_x = 21.0;
            s.scroll_y = 48.0;
        }
        valid_draft(app, Trigger::Manual);
        click(app, Hit::Test);
        click(app, Hit::Run);
        let p = app.automation.panel.as_ref().unwrap();
        assert!(p.preview.is_none());
        assert_eq!(engine::runtime(&p.document).runs.len(), 1);
        assert!(engine::rules(p.view()).unwrap().is_empty());
        let Some((_, viewer::Content::Base(s))) = app.viewer_tab() else {
            panic!()
        };
        assert_eq!(s.scroll_x, 21.0);
        assert_eq!(s.scroll_y, 48.0);
        assert_eq!(s.document.tables[0].records[0].values["checked"], true);
        assert_eq!(s.saved_raw, std::fs::read_to_string(path).unwrap());
    });
}
#[test]
fn stale_disk_and_dirty_buffer_block_manual_write() {
    with_app(|app, path| {
        valid_draft(app, Trigger::Manual);
        click(app, Hit::Test);
        let disk = std::fs::read_to_string(path).unwrap();
        std::fs::write(path, format!("{disk}\n")).unwrap();
        click(app, Hit::Run);
        let p = app.automation.panel.as_ref().unwrap();
        assert!(p.message.iter().any(|s| s.contains("已被修改")));
        assert_eq!(std::fs::read_to_string(path).unwrap(), format!("{disk}\n"));
        std::fs::write(path, &disk).unwrap();
        click(app, Hit::Test);
        if let Some(viewer::Content::Base(s)) = app.viewer_content_mut() {
            s.dirty = true;
        }
        click(app, Hit::Run);
        assert_eq!(std::fs::read_to_string(path).unwrap(), disk);
        assert!(app
            .automation
            .panel
            .as_ref()
            .unwrap()
            .message
            .iter()
            .any(|s| s.contains("未保存")));
    });
}
#[test]
fn narrow_layout_keeps_close_and_controls_separate_and_keyboard_reachable() {
    with_app(|app, _| {
        valid_draft(
            app,
            Trigger::Interval {
                minutes: 60,
                start_at: engine::now_ms(),
            },
        );
        for width in [320.0, 420.0, 780.0, 1280.0] {
            let p = app.automation.panel.as_ref().unwrap();
            let lay = p.layout(Rect::from_size(0.0, 0.0, width, 500.0));
            assert!(lay.close.intersect(&lay.body).is_empty());
            for pair in lay.rows.windows(2) {
                assert!(pair[0].0.intersect(&pair[1].0).is_empty());
            }
            assert!(lay.max_scroll > 0.0);
        }
        for _ in 0..28 {
            app.on_edit_key(0x09, false, false);
            let p = app.automation.panel.as_ref().unwrap();
            let lay = p.layout(app.renderer.viewport());
            let focused = p.focused.unwrap();
            let r = lay.rows[focused].0;
            assert!(r.top >= lay.body.top - 0.1 && r.bottom <= lay.body.bottom + 0.1);
        }
        app.paint(HWND::default()).unwrap();
    });
}
#[test]
fn modal_blocks_navigation_text_ime_and_underlying_scroll() {
    with_app(|app, path| {
        let original = std::fs::read_to_string(path).unwrap();
        let tab = app.shell.active_tab();
        assert!(app.on_accelerator(HWND::default(), 0x25, false, false, true));
        assert_eq!(app.shell.active_tab(), tab);
        assert!(app.on_char('x'));
        app.on_ime_commit("误输入");
        app.on_double_click(12.0, 140.0);
        app.on_right_click(12.0, 140.0);
        assert!(app.menu.is_none());
        app.on_wheel(12.0, 140.0, -120);
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
        assert!(!app.shell.active().unwrap().dirty());
    });
}
#[test]
fn enabled_rule_plans_after_table_closes_and_rejects_stale_commit() {
    with_app(|app, path| {
        valid_draft(app, Trigger::RecordCreated);
        click(app, Hit::Enable);
        app.automation_activate(Hit::Close);
        assert!(app.automation.panel.is_none());
        app.shell.close_tab(app.shell.active_tab().unwrap());
        assert!(app.shell.tabs().is_empty());
        let path = path.canonicalize().unwrap();
        let consent = authorized(&grants(&app.settings), &path);
        let cache = app.automation.cache.get(&path).unwrap().clone();
        let mut doc = model::parse_base_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
        doc.tables[0].records.push(model::BaseRecord {
            id: "new_record".into(),
            values: serde_json::from_value(serde_json::json!({"name":"new","checked":false}))
                .unwrap(),
            ..Default::default()
        });
        let raw = model::serialize_base_document(&doc).unwrap();
        std::fs::write(&path, &raw).unwrap();
        let plan = plan_file(&path, &cache, consent, engine::now_ms())
            .unwrap()
            .unwrap();
        assert!(plan.content.is_some());
        assert_eq!(
            plan.document.tables[0].records.last().unwrap().values["checked"],
            true
        );
        std::fs::write(&path, format!("{raw}\n")).unwrap();
        assert!(app
            .persist_automation(
                &path,
                &plan.original,
                plan.content.as_ref().unwrap(),
                &plan.document
            )
            .is_err());
    });
}

#[test]
fn timer_executes_closed_table_and_dirty_document_defers_without_losing_event() {
    with_app(|app, path| {
        app.automation_timer();
        valid_draft(app, Trigger::RecordCreated);
        click(app, Hit::Enable);
        app.automation_activate(Hit::Close);
        let path = path.canonicalize().unwrap();
        let mut doc = model::parse_base_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
        doc.tables[0].records.push(model::BaseRecord {
            id: "timer_record".into(),
            values: serde_json::from_value(serde_json::json!({"name":"timer","checked":false}))
                .unwrap(),
            ..Default::default()
        });
        let raw = model::serialize_base_document(&doc).unwrap();
        std::fs::write(&path, &raw).unwrap();
        if let Some(viewer::Content::Base(s)) = app.viewer_content_mut() {
            s.dirty = true;
        }
        app.automation_timer();
        assert!(app.automation.job.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        if let Some(viewer::Content::Base(s)) = app.viewer_content_mut() {
            s.dirty = false;
        }
        app.shell.close_tab(app.shell.active_tab().unwrap());
        app.automation_timer();
        assert!(app.automation.job.is_some());
        for _ in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.automation_timer();
            let written =
                model::parse_base_document(&std::fs::read_to_string(&path).unwrap()).unwrap();
            if engine::runtime(&written).runs.len() == 1 {
                assert_eq!(
                    written.tables[0].records.last().unwrap().values["checked"],
                    true
                );
                return;
            }
        }
        panic!("automation worker did not commit");
    });
}
