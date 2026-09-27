use super::*;
use mochi_core::ai::tools::host::ToolHost;

#[test]
fn focus_refresh_updates_preferences_without_replacing_active_work() {
    let root = std::env::temp_dir().join(format!(
        "mochi-focus-settings-{}",
        mochi_core::paths::random_base36(12)
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let path = root.join("profile/settings.json");
    {
        let mut app =
            App::with_settings(Arc::new(SettingsService::new(Some(path.clone())))).unwrap();
        app.settings.set("app.appearance.themeMode", "light");
        app.settings.flush().unwrap();
        app.load_chrome_settings();
        app.shell.open_workspace(&workspace, || {}).unwrap();
        app.setup_ai(HWND::default(), &workspace);
        app.ai_new_session();
        let session = app.ai.panel.active.as_ref().unwrap().id.clone();
        let note = workspace.join("note.md");
        std::fs::write(&note, "saved note").unwrap();
        app.shell.open_file(&note);
        app.shell.active_buffer_mut().unwrap().insert("unsaved ");
        app.prefs.providers.form = Some(providers::Form::new(mochi_core::ai::models::AiProvider {
            id: "draft".into(),
            name: "unsaved provider form".into(),
            protocol: "openai-completions".into(),
            ..Default::default()
        }));
        let (_tx, rx) = channel();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        app.ai.run = Some(RunHandle {
            rx,
            cancel: cancel.clone(),
        });

        let peer = SettingsService::new(Some(path.clone()));
        peer.set("app.appearance.themeMode", "dark");
        peer.set("app.appearance.accentColor", "#a855f7");
        peer.set("app.sidebar.width", "420");
        peer.set("app.typography.fontSize", "20");
        peer.set("workspace.lastPath", "D:/a-different-workspace");
        peer.set("search.history", r#"["from Electron"]"#);
        peer.set("keyboard.shortcuts", r#"{"Ctrl+P":"Ctrl+Alt+P"}"#);
        peer.set("mochi-ai",&serde_json::json!({"version":3,"state":{
            "providers":[{"id":"peer","name":"Peer","baseUrl":"http://localhost:1234/v1","model":"peer-model","apiKey":"","stream":true}],
            "currentProviderId":"peer","shellWhitelist":["python"],
            "permissions":[{"action":"execute_command","allowed":true,"requiresConfirmation":true}]
        }}).to_string());
        peer.flush().unwrap();
        // 应用窗口重新获得焦点前，请求可能已经读到了新版本号。
        app.settings.reload().unwrap();
        assert_eq!(
            app.settings.get("app.sidebar.width").as_deref(),
            Some("420")
        );
        assert!(app.refresh_shared_settings());
        assert!(app.state.dark);
        assert_eq!(theme::configured_palette(app.state.dark).accent, 0xa855f7);
        assert_eq!(app.state.sidebar_width, 420.0);
        assert_eq!(
            crate::ui::settings_values::number("typography.fontSize", 0.0),
            20.0
        );
        assert_eq!(crate::ui::shortcuts::binding("Ctrl+P"), "Ctrl+Alt+P");
        assert_eq!(app.search_history, ["from Electron"]);
        assert_eq!(app.prefs.providers.selected.as_deref(), Some("peer"));
        assert_eq!(
            app.prefs.providers.form.as_ref().unwrap().fields[0].text(),
            "unsaved provider form"
        );
        assert_eq!(app.shell.workspace().unwrap().root, workspace);
        assert_eq!(app.active_file_path(), Some(note));
        assert_eq!(
            app.shell.active_buffer_mut().unwrap().text(),
            "unsaved saved note"
        );
        assert!(app.shell.active_buffer_mut().unwrap().dirty());
        assert_eq!(app.ai.panel.active.as_ref().unwrap().id, session);
        assert!(app.ai.run.is_some());
        assert!(!cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(app
            .ai
            .permissions
            .as_ref()
            .unwrap()
            .action_permissions()
            .is_allowed(mochi_core::ai::permission::AiToolAction::ExecuteCommand));
        assert_eq!(app.ai.host.as_ref().unwrap().shell_whitelist(), ["python"]);
        assert!(!app.refresh_shared_settings());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_provider_form_saves_only_edited_fields_and_does_not_revive_deleted_provider() {
    let root = std::env::temp_dir().join(format!(
        "mochi-provider-shared-form-{}",
        mochi_core::paths::random_base36(12)
    ));
    let path = root.join("settings.json");
    {
        let mut app =
            App::with_settings(Arc::new(SettingsService::new(Some(path.clone())))).unwrap();
        let original = mochi_core::ai::models::AiProvider {
            id: "provider".into(),
            name: "Original".into(),
            base_url: "http://localhost:1234/v1".into(),
            model: "original-model".into(),
            api_key: String::new(),
            stream: true,
            protocol: "openai-completions".into(),
        };
        mochi_core::ai::providers::save(&app.settings, original.clone()).unwrap();
        let mut form = providers::Form::new(original);
        form.editing_existing = true;
        form.fields[0].set_text("Edited locally");
        app.prefs.providers.form = Some(form);
        let peer = SettingsService::new(Some(path));
        let mut updated = mochi_core::ai::providers::selected(&peer).unwrap();
        updated.model = "new remote model".into();
        updated.stream = false;
        mochi_core::ai::providers::save(&peer, updated).unwrap();
        app.refresh_shared_settings();
        app.save_provider_form().unwrap();
        let saved = mochi_core::ai::providers::selected(&app.settings).unwrap();
        assert_eq!(saved.name, "Edited locally");
        assert_eq!(saved.model, "new remote model");
        assert!(!saved.stream);

        let mut form = providers::Form::new(saved);
        form.editing_existing = true;
        form.fields[0].set_text("Draft after remote deletion");
        app.prefs.providers.form = Some(form);
        mochi_core::ai::providers::remove(&peer, "provider").unwrap();
        app.refresh_shared_settings();
        assert!(app.save_provider_form().is_err());
        assert!(app.prefs.providers.form.is_some());
        assert!(mochi_core::ai::providers::list(&app.settings).is_empty());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_resize_and_navigation_collapse_persist_in_shared_buckets() {
    let root = std::env::temp_dir().join(format!(
        "mochi-geometry-settings-{}",
        mochi_core::paths::random_base36(12)
    ));
    let path = root.join("settings.json");
    {
        let mut app =
            App::with_settings(Arc::new(SettingsService::new(Some(path.clone())))).unwrap();
        app.state.navigation_width = 240.0;
        app.state.sidebar_width = 380.0;
        app.state.ai_panel_width = 460.0;
        app.split.ratio = 0.625;
        for target in [
            DragTarget::Navigation,
            DragTarget::Sidebar,
            DragTarget::AiPanel,
            DragTarget::Split,
        ] {
            app.drag = Some(Drag {
                target,
                grab_offset: 0.0,
            });
            app.end_drag();
        }
        app.nav_layout
            .entries
            .push((Rect::from_size(0.0, 0.0, 24.0, 24.0), NavHit::Collapse));
        app.on_navigation_click(12.0, 12.0);
        let reader = SettingsService::new(Some(path));
        assert_eq!(reader.get("app.navigation.width").as_deref(), Some("240"));
        assert_eq!(reader.get("app.sidebar.width").as_deref(), Some("380"));
        assert_eq!(
            reader.get("app.editorLayout.aiPanelWidth").as_deref(),
            Some("460")
        );
        assert_eq!(
            reader.get("app.editorLayout.splitRatio").as_deref(),
            Some("0.625")
        );
        assert_eq!(
            reader.get("app.navigation.collapsed").as_deref(),
            Some("true")
        );
        let values: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(reader.file_path()).unwrap()).unwrap();
        assert!(values.get("app.sidebar.width").is_none());
        assert!(values.get("workspace-storage").is_some());
    }
    std::fs::remove_dir_all(root).unwrap();
}
