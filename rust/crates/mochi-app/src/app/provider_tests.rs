use super::*;

fn reveal_provider(app: &mut App, hit: providers::Hit) -> Rect {
    // 走和 UI 相同的滚轮处理来滚动；横向溢出时仍然要点空，而不是点中旁边的 AI 面板。
    if !matches!(hit, providers::Hit::Add | providers::Hit::Options) {
        for _ in 0..40 {
            let rect = app
                .prefs
                .provider_layout
                .entries
                .iter()
                .find(|(_, entry)| *entry == hit)
                .unwrap()
                .0;
            let body = app.prefs.provider_layout.body;
            if rect.top >= body.top && rect.bottom <= body.bottom - 32.0 {
                break;
            }
            app.on_wheel(
                (body.left + body.right) / 2.0,
                (body.top + body.bottom) / 2.0,
                if rect.top < body.top { 120 } else { -120 },
            );
            app.paint(HWND::default()).unwrap();
        }
    }
    let rect = app
        .prefs
        .provider_layout
        .entries
        .iter()
        .find(|(_, entry)| *entry == hit)
        .unwrap()
        .0;
    let x = (rect.left + rect.right) / 2.0;
    let y = (rect.top + rect.bottom) / 2.0;
    assert!(
        app.settings_overlay_rect().contains(x, y),
        "{hit:?} is outside the settings overlay"
    );
    assert_eq!(app.prefs.provider_layout.hit(x, y), Some(hit));
    rect
}

fn click_provider(app: &mut App, hit: providers::Hit) {
    let rect = reveal_provider(app, hit);
    let x = (rect.left + rect.right) / 2.0;
    let y = (rect.top + rect.bottom) / 2.0;
    app.on_click(x, y);
    app.paint(HWND::default()).unwrap();
}

#[test]
fn provider_forms_can_be_created_and_edited_from_both_settings_entries() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-provider-ui-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    for width in [1200, 1024] {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(format!("settings-{width}.json")),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let snapshot = app.renderer.prepare_snapshot(width, 800, 96.0).unwrap();
        app.load_chrome_settings();
        app.state.dark = width == 1024;
        app.state.ai_panel_open = true;
        app.state.right_panel = RightPanel::Assistant;
        app.open_settings("general");
        assert_eq!(
            app.settings_overlay.as_ref().map(|(tab, _)| tab.as_str()),
            Some("general")
        );
        assert!(
            !app.shell
                .tabs()
                .iter()
                .any(|tab| matches!(tab.kind, TabKind::Settings { .. })),
            "settings must not occupy a file tab"
        );
        app.paint(HWND::default()).unwrap();
        let ai_tab = settings::TABS
            .iter()
            .position(|(id, _, _)| *id == "ai")
            .unwrap();
        let rect = app
            .prefs
            .nav_layout
            .rect_of(settings::NavHit::Tab(ai_tab))
            .unwrap();
        app.on_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        app.paint(HWND::default()).unwrap();
        click_provider(&mut app, providers::Hit::Add);
        assert!(app.prefs.providers.form.is_some());
        assert!(app.on_edit_key(13, false, false));
        assert!(app.prefs.providers.form.is_some());
        assert!(!app.prefs.providers.status.is_empty());
        assert!(app.prefs.providers.all.is_empty());
        click_provider(&mut app, providers::Hit::Preset);
        let menu = app.menu.as_ref().unwrap();
        assert_eq!(menu.items[0].label, "自定义");
        assert!(menu.search.is_some());
        let preset_row = menu
            .items
            .iter()
            .position(|item| item.label == "DeepSeek")
            .unwrap();
        if let Some(output) = std::env::var_os("MOCHI_PROVIDER_TEST_ARTIFACTS") {
            let output = PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            app.renderer
                .save_snapshot(
                    &snapshot,
                    &output.join(format!("provider-{width}-presets.png")),
                )
                .unwrap();
        }
        app.activate_menu_item(0, preset_row);
        assert_eq!(
            app.prefs.providers.form.as_ref().unwrap().value().name,
            "DeepSeek"
        );
        assert!(
            app.prefs.providers.all.is_empty(),
            "preset selection must not save automatically"
        );
        app.paint(HWND::default()).unwrap();
        for (i, text) in [
            "UI provider",
            "http://localhost:1234/v1",
            "test-model",
            "test-key",
        ]
        .into_iter()
        .enumerate()
        {
            click_provider(&mut app, providers::Hit::Field(i));
            assert!(app.on_edit_key(0x41, false, true));
            for ch in text.chars() {
                assert!(app.on_char(ch));
            }
        }
        app.paint(HWND::default()).unwrap();
        if let Some(output) = std::env::var_os("MOCHI_PROVIDER_TEST_ARTIFACTS") {
            let save = reveal_provider(&mut app, providers::Hit::Save);
            assert!(app.on_mouse_move(save.left + 4.0, save.top + 4.0));
            assert_eq!(
                app.prefs.providers.interaction.hover,
                Some(providers::Hit::Save)
            );
            assert!(app
                .take_timer_requests()
                .iter()
                .any(|(id, _)| *id == platform::TIMER_PROVIDER_INTERACTION));
            std::thread::sleep(std::time::Duration::from_millis(80));
            app.paint(HWND::default()).unwrap();
            let output = PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            app.renderer
                .save_snapshot(
                    &snapshot,
                    &output.join(format!("provider-{width}-form.png")),
                )
                .unwrap();
        }
        click_provider(&mut app, providers::Hit::Save);
        assert!(app.prefs.providers.status.contains("已保存"));
        assert!(
            app.prefs.providers.form.is_none(),
            "{}",
            app.prefs.providers.status
        );
        assert_eq!(app.prefs.providers.all.len(), 1);
        let rect = app.ai.layout.rect_of(assistant::Hit::Settings).unwrap();
        app.on_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        app.paint(HWND::default()).unwrap();
        click_provider(&mut app, providers::Hit::Edit(0));
        click_provider(&mut app, providers::Hit::Field(2));
        assert!(app.on_edit_key(0x41, false, true));
        for ch in "updated-model".chars() {
            assert!(app.on_char(ch));
        }
        assert!(app.on_edit_key(13, false, false));
        app.paint(HWND::default()).unwrap();
        assert!(
            app.prefs.providers.form.is_none(),
            "{}",
            app.prefs.providers.status
        );
        assert_eq!(app.prefs.providers.all[0].model, "updated-model");
        if let Some(output) = std::env::var_os("MOCHI_PROVIDER_TEST_ARTIFACTS") {
            app.renderer
                .save_snapshot(
                    &snapshot,
                    &PathBuf::from(output).join(format!("provider-{width}-list.png")),
                )
                .unwrap();
        }
        let reloaded = SettingsService::new(Some(root.join(format!("settings-{width}.json"))));
        let all = mochi_core::ai::providers::list(&reloaded);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "UI provider");
        assert_eq!(all[0].model, "updated-model");
        assert_eq!(all[0].api_key, "test-key");
    }
    std::fs::remove_dir_all(root).unwrap();
}
