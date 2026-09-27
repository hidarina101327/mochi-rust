use super::*;

fn app() -> App {
    let root = std::env::temp_dir().join(format!(
        "mochi-desktop-tests-{}-{}",
        std::process::id(),
        uuid_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap()
}

#[test]
fn desktop_agent_update_persists_then_acknowledges_and_refreshes_live_state() {
    use mochi_core::ai::tools::host::ToolHost;
    let mut app = app();
    let root = std::env::temp_dir().join(format!("mochi-desktop-agent-{}", uuid_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let host = Arc::new(crate::ai_runtime::AppHost::new(
        &root,
        Arc::new(std::sync::Mutex::new(Default::default())),
        Arc::new(mochi_core::ai::permission::AiPermissionService::new(&root)),
        || {},
    ));
    app.ai.host = Some(host.clone());
    app.desktop.root = Some(root.clone());
    app.desktop.loaded = true;
    let mut args = mochi_core::ai::tools::desktop_tools::snapshot(&app.desktop.config);
    let mut card = model::Card::new("Agent 卡片", Module::Inbox);
    card.enabled = false;
    args["config"]["cards"] = serde_json::json!([card]);
    let thread = std::thread::spawn(move || host.desktop_cards("desktop_cards_update", &args));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !thread.is_finished() && Instant::now() < deadline {
        app.desktop_agent_requests();
        app.desktop_take_results();
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = thread.join().unwrap().unwrap();
    assert_eq!(result["config"]["cards"][0]["title"], "Agent 卡片");
    assert_eq!(app.desktop.config.cards[0].title, "Agent 卡片");
    assert_eq!(DesktopConfig::load(&root).unwrap(), app.desktop.config);
    assert!(!app.desktop.saving_editor);
    let host = app.ai.host.as_ref().unwrap().clone();
    let args = serde_json::json!({"revision":result["revision"],"operations":[
        {"op":"update","id":result["config"]["cards"][0]["id"],"changes":{"title":"批量修改的卡片","appearance":{"opacity":70}}}
    ]});
    let thread = std::thread::spawn(move || host.desktop_cards("desktop_cards_batch", &args));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !thread.is_finished() && Instant::now() < deadline {
        app.desktop_agent_requests();
        app.desktop_take_results();
        std::thread::sleep(Duration::from_millis(10));
    }
    let updated = thread.join().unwrap().unwrap();
    assert_eq!(updated["config"]["cards"][0]["title"], "批量修改的卡片");
    assert_eq!(app.desktop.config.cards[0].appearance.opacity, 70);
    assert_eq!(DesktopConfig::load(&root).unwrap(), app.desktop.config);
    let before = app.desktop.config.clone();
    app.open_desktop_manager();
    let host = app.ai.host.as_ref().unwrap().clone();
    let args = serde_json::json!({"revision":updated["revision"],"operations":[{"op":"delete","id":updated["config"]["cards"][0]["id"]}]});
    let thread = std::thread::spawn(move || host.desktop_cards("desktop_cards_batch", &args));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !thread.is_finished() && Instant::now() < deadline {
        app.desktop_agent_requests();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(thread.join().unwrap().unwrap_err().contains("正在编辑"));
    assert_eq!(DesktopConfig::load(&root).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_manager_draft_does_not_mutate_live_cards() {
    let mut app = app();
    app.desktop
        .config
        .cards
        .push(model::Card::new("原卡片", Module::Schedule));
    app.open_desktop_manager();
    app.desktop.panel.as_mut().unwrap().config.cards[0].title = "草稿改名".into();
    assert_eq!(app.desktop.config.cards[0].title, "原卡片");
    assert!(app.desktop.windows.is_empty());
}

#[test]
fn desktop_save_stays_open_and_apply_closes_without_global_notice() {
    let mut app = app();
    let mut card = model::Card::new("编辑", Module::Home);
    card.enabled = false;
    app.desktop.config.cards.push(card);
    app.desktop.panel = Some(ui::State::new(&app.desktop.config));
    let notice = app.status_bar.toast.message.clone();
    for keep in [true, false] {
        app.desktop.keep_editor_open = keep;
        app.desktop.saving_editor = true;
        let mut config = app.desktop.config.clone();
        config.cards[0].title = "已保存的名称".into();
        let (tx, rx) = channel();
        app.desktop.job = Some(rx);
        tx.send(runtime::Output::saved_for_test(
            app.desktop.epoch,
            config.clone(),
        ))
        .unwrap();
        app.desktop_take_results();
        assert_eq!(app.desktop.config, config);
        assert_eq!(app.desktop.panel.is_some(), keep);
        if keep {
            assert_eq!(app.focus, Focus::Dialog);
            assert!(!app.desktop.panel.as_ref().unwrap().dirty);
            assert_eq!(app.desktop.editing_base, Some(config));
        } else {
            assert_eq!(app.focus, Focus::Main);
        }
        assert!(!app.desktop.saving_editor);
        assert_eq!(app.status_bar.toast.message, notice);
    }
}

#[test]
fn desktop_titlebar_entry_is_directly_left_of_notifications() {
    let chrome = Chrome::build(&ChromeState::default(), Rect::new(0.0, 0.0, 1200.0, 800.0));
    let card = chrome.tree.rect(chrome.desktop_toggle);
    let bell = chrome.tree.rect(chrome.notifications_toggle);
    assert!((card.right - bell.left).abs() < 0.1);
    assert_eq!(
        chrome.hit((card.left + card.right) / 2.0, card.top + 12.0),
        Some(NodeKey::TitleBarDesktop)
    );
}

#[test]
fn stale_workspace_job_cannot_replace_current_config() {
    let mut app = app();
    app.desktop.epoch = 5;
    let (tx, rx) = channel();
    app.desktop.job = Some(rx);
    let mut old = DesktopConfig::default();
    old.cards.push(model::Card::new("旧工作区", Module::Home));
    tx.send(runtime::Output::loaded_for_test(4, old)).unwrap();
    app.desktop_take_results();
    assert!(app.desktop.config.cards.is_empty());
    assert!(!app.desktop.loaded);
}

#[test]
fn disabled_cards_have_no_refresh_job() {
    let mut app = app();
    let mut card = model::Card::new("隐藏", Module::Home);
    card.enabled = false;
    app.desktop.config.cards.push(card);
    app.desktop.root = Some(PathBuf::from("never-read"));
    app.desktop.loaded = true;
    app.desktop_refresh();
    assert!(app.desktop.job.is_none());
    assert!(app.desktop.pending.is_empty());
}

#[test]
fn refresh_requests_coalesce_while_io_is_running() {
    let mut app = app();
    let (_tx, rx) = channel();
    app.desktop.job = Some(rx);
    for _ in 0..50 {
        app.desktop_enqueue(runtime::Job::Snapshot(
            PathBuf::from("never-read"),
            DesktopConfig::default(),
        ));
    }
    assert!(app.desktop.pending.is_empty());
    assert!(app.desktop.refresh_needed);
}

#[test]
fn desktop_live_pomodoro_uses_runtime_state_without_a_disk_read() {
    let mut app = app();
    let card = model::Card::new("专注", Module::Pomodoro);
    let page = card.active_page().unwrap();
    let key = model::page_key(&card.id, &page.id);
    let mut snap = model::PageSnapshot::empty(page);
    snap.rows.push(model::Row {
        id: "pomodoro:status".into(),
        ..Default::default()
    });
    app.desktop.snapshot.pages.insert(key.clone(), snap);
    app.desktop.config.cards.push(card);
    app.panels.pomodoro.start();
    assert!(app.desktop_update_live());
    assert_eq!(app.desktop.snapshot.pages[&key].rows[0].title, "专注中");
    app.panels.pomodoro.remaining = 120;
    app.panels.pomodoro.running = false;
    app.panels.pomodoro.end_at = None;
    assert!(app.desktop_update_live());
    assert_eq!(app.desktop.snapshot.pages[&key].rows[0].title, "已暂停");
    assert!(app.desktop.snapshot.pages[&key].rows[0]
        .detail
        .starts_with("02:00"));
    assert!(!app.desktop_update_live());
    assert!(app.desktop.job.is_none());
}

#[test]
fn desktop_library_route_accepts_canonical_windows_paths() {
    let mut app = app();
    let root = std::env::temp_dir().join(format!("mochi-desktop-library-route-{}", uuid_v4()));
    std::fs::create_dir_all(&root).unwrap();
    app.shell.open_workspace(&root, || {}).unwrap();
    let index = app
        .shell
        .create_library("knowledge-base", "目标知识库")
        .unwrap();
    let path = PathBuf::from(&app.shell.workspace().unwrap().libraries[index].path);
    let doc = path.join("文档.md");
    std::fs::write(&doc, "文档").unwrap();
    app.shell.open_file(&doc);
    app.desktop_open_folder(&path.canonicalize().unwrap());
    assert_eq!(app.shell.selected_library(), Some(index));
    assert!(matches!(
        app.shell.active().unwrap().kind,
        crate::shell::TabKind::Library { .. }
    ));

    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn desktop_widget_event_chains_are_bounded_and_interval_bindings_are_independent() {
    use model::studio::{Action, Binding, Kind, Node, Trigger};
    let mut app = app();
    let mut card = model::Card::new("事件", Module::Custom);
    let mut label = Node::new(Kind::Text);
    label.id = "label".into();
    label.title = "原文".into();
    let mut button = Node::new(Kind::Button);
    button.id = "button".into();
    button.events = vec![
        Binding {
            trigger: Trigger::Interval,
            action: Action::SetText,
            target: "label".into(),
            value: "一分钟".into(),
            seconds: 60,
            ..Default::default()
        },
        Binding {
            trigger: Trigger::Interval,
            action: Action::SetText,
            target: "label".into(),
            value: "一小时".into(),
            seconds: 3600,
            ..Default::default()
        },
        Binding {
            trigger: Trigger::Click,
            action: Action::Emit,
            target: "loop".into(),
            ..Default::default()
        },
        Binding {
            trigger: Trigger::Custom,
            action: Action::Emit,
            target: "loop".into(),
            name: "loop".into(),
            ..Default::default()
        },
    ];
    card.pages[0].studio.nodes = vec![label, button];
    app.desktop.config.cards.push(card);
    app.desktop.widget_second = 60;
    app.desktop_widget_event(0, "button", Trigger::Interval);
    assert_eq!(
        app.desktop.config.cards[0].pages[0].studio.nodes[0].title,
        "一分钟"
    );
    app.desktop_widget_event(0, "button", Trigger::Click);
    assert_eq!(app.desktop.config.cards[0].pages[0].studio.nodes.len(), 2);
}

#[test]
fn desktop_context_settings_selects_the_target_card_and_page() {
    unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
        .ok()
        .unwrap();
    }
    let mut app = app();
    let _target = app.renderer.prepare_snapshot(760, 600, 96.0).unwrap();
    let mut first = model::Card::new("A", Module::Home);
    first.enabled = false;
    let mut target = model::Card::new("B", Module::Inbox);
    target.enabled = false;
    target.pages.push(model::Page::new(Module::Shortcuts));
    for _ in 0..18 {
        target.pages.push(model::Page::new(Module::Recent));
    }
    let card = target.id.clone();
    let page = target.pages.last().unwrap().id.clone();
    app.desktop.config.cards = vec![first, target];
    app.desktop_manage_page(&card, &page);
    let panel = app.desktop.panel.as_ref().unwrap();
    assert_eq!(panel.selected_card_ref().unwrap().id, card);
    assert_eq!(panel.selected_page_ref().unwrap().id, page);
    assert_eq!(panel.editor_tab, 1);
    let layout = panel.layout(app.desktop_manager_area());
    let selected = layout
        .page_rows
        .iter()
        .find(|(_, i)| Some(*i) == panel.selected_page)
        .unwrap()
        .0;
    assert!(selected.left >= layout.page_tabs.left - 0.1);
    assert!(selected.right <= layout.page_tabs.right - 102.0 + 0.1);
}

#[test]
fn desktop_workspace_draft_survives_titlebar_navigation_without_capturing_other_pages() {
    unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
    }
    .ok()
    .unwrap();
    let mut app = app();
    let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
    app.desktop.loaded = true;
    let mut card = model::Card::new("原卡片", Module::Inbox);
    card.enabled = false;
    app.desktop.config.cards.push(card);
    app.open_desktop_manager();
    assert_eq!(app.state.view, WorkspaceView::DesktopCards);
    assert!(app.desktop.panel.as_ref().unwrap().embedded);
    let area = app.desktop_manager_area();
    assert!(area.left > 0.0 && area.top > 0.0);
    assert!(app.desktop_manager_captures_pointer(area.left + 20.0, area.top + 20.0));
    let panel = app.desktop.panel.as_mut().unwrap();
    panel.config.cards[0].title = "保留草稿".into();
    panel.dirty = true;
    panel.focus_field = Some(ui::Field::CardName);
    app.focus = Focus::AiInput;
    assert!(!app.desktop_manager_input_active());
    assert!(app.on_char('问'));
    assert_eq!(app.ai.panel.input.text(), "问");
    app.focus = Focus::Main;
    let chrome = app.build_chrome();
    let target = chrome.tree.rect(chrome.templates_toggle);
    assert!(!app.desktop_manager_captures_pointer(target.left + 10.0, target.top + 10.0));

    app.on_click(target.left + 10.0, target.top + 10.0);
    assert_eq!(app.state.view, WorkspaceView::Templates);
    assert!(!app.desktop_manager_active());
    app.focus = Focus::AiInput;
    assert!(app.on_char('好'));
    assert_eq!(app.ai.panel.input.text(), "问好");
    app.open_desktop_manager();
    assert_eq!(
        app.desktop.panel.as_ref().unwrap().config.cards[0].title,
        "保留草稿"
    );
    assert!(app.desktop.panel.as_ref().unwrap().dirty);
    assert_eq!(app.desktop.config.cards[0].title, "原卡片");
}

#[test]
fn desktop_workspace_save_keeps_page_and_does_not_steal_focus_after_navigation() {
    let mut app = app();
    app.desktop.loaded = true;
    let mut card = model::Card::new("卡片", Module::Inbox);
    card.enabled = false;
    app.desktop.config.cards.push(card);
    app.open_desktop_manager();
    for hidden in [false, true] {
        app.desktop.keep_editor_open = true;
        app.desktop.saving_editor = true;
        let mut config = app.desktop.config.clone();
        config.cards[0].title = "已保存".into();
        if hidden {
            app.state.view = WorkspaceView::Home;
            app.focus = Focus::AiInput;
        }
        let (tx, rx) = channel();
        app.desktop.job = Some(rx);
        tx.send(runtime::Output::saved_for_test(
            app.desktop.epoch,
            config.clone(),
        ))
        .unwrap();
        app.desktop_take_results();
        assert_eq!(app.desktop.config, config);
        let panel = app.desktop.panel.as_ref().unwrap();
        assert!(panel.embedded && !panel.dirty);
        assert_eq!(app.focus, if hidden { Focus::AiInput } else { Focus::Main });
        assert_eq!(
            app.state.view,
            if hidden {
                WorkspaceView::Home
            } else {
                WorkspaceView::DesktopCards
            }
        );
    }
}
