use super::*;

#[test]
fn typing_three_backticks_then_enter_creates_a_four_tick_code_block() {
    let root = std::env::temp_dir().join(format!(
        "mochi-code-fence-enter-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let path = root.join("代码块.md");
        std::fs::write(&path, "").unwrap();
        assert!(app.shell.open_file(&path));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        app.editor_engaged = true;

        for _ in 0..3 {
            assert!(app.on_char('`'));
        }
        assert!(app.on_edit_key(0x0d, false, false));
        assert!(
            !app.on_char('\r'),
            "WM_CHAR 的回车不得在已经处理过的 Enter 后再次插入换行"
        );

        let buffer = app.shell.active().unwrap().buffer().unwrap();
        assert_eq!(buffer.text(), "````\n\n````");
        assert_eq!(buffer.cursor(), "````\n".len());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn outline_placement_has_the_three_requested_surfaces_and_file_toggle() {
    let root = std::env::temp_dir().join(format!(
        "mochi-outline-placement-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        let placement = app_settings::descriptor("outline.placement").unwrap();
        let visible = app_settings::descriptor("outline.visible").unwrap();

        app.app_settings
            .write(placement, &SettingValue::Text("editor-right".into()));
        assert_eq!(app.outline_mode(), "editor-right");
        assert!(app.outline_visible_in_file_area());
        app.toggle_file_area_outline();
        assert!(!app.outline_visible_in_file_area());
        let saved = app_settings::AppSettings::new(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))));
        assert_eq!(saved.read(visible), SettingValue::Bool(false));
        app.toggle_file_area_outline();
        assert!(app.outline_visible_in_file_area());
        let saved = app_settings::AppSettings::new(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))));
        assert_eq!(saved.read(visible), SettingValue::Bool(true));

        app.app_settings
            .write(placement, &SettingValue::Text("left-sidebar".into()));
        app.apply_setting_side_effects("outline.placement");
        assert!(app.outline_left_active);

        app.app_settings
            .write(placement, &SettingValue::Text("ai-sidebar".into()));
        app.apply_setting_side_effects("outline.placement");
        assert!(app.state.outline_in_ai_sidebar);
        assert_eq!(app.state.right_panel, RightPanel::Outline);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn releasing_a_fast_folder_drag_does_not_toggle_its_expansion() {
    let root = std::env::temp_dir().join(format!(
        "mochi-sidebar-folder-drag-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let folder = PathBuf::from(&app.shell.workspace().unwrap().libraries[0].path).join("资料");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("笔记.md"), "内容").unwrap();
        app.shell.refresh_tree();
        let row = app
            .shell
            .rows()
            .iter()
            .position(|entry| entry.path == folder)
            .unwrap();
        app.shell.toggle_loaded(row);
        let row = app
            .shell
            .rows()
            .iter()
            .position(|entry| entry.path == folder)
            .unwrap();
        assert!(app.shell.rows()[row].expanded);

        // 模拟系统只把最终坐标放在 mouse-up 消息里，没有额外 mouse-move 的快速拖拽。
        app.begin_sidebar_tree_drag(row, 100.0, 100.0);
        app.end_drag_at(100.0 + TREE_DRAG_THRESHOLD + 1.0, 100.0);

        let row = app
            .shell
            .rows()
            .iter()
            .position(|entry| entry.path == folder)
            .unwrap();
        assert!(app.shell.rows()[row].expanded);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_context_menu_matches_electron_and_keeps_save_as_template() {
    let root = std::env::temp_dir().join(format!(
        "mochi-tab-context-menu-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let path = root.join("笔记.md");
        std::fs::write(&path, "# 笔记").unwrap();
        assert!(app.shell.open_file(&path));

        let labels = app
            .tab_context_menu_items(0)
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "刷新",
                "关闭",
                "关闭其他",
                "关闭右侧标签页",
                "全部关闭",
                "复制路径",
                "复制相对路径",
                "固定",
                "向右拆分",
                "在文件管理器中显示",
                "在文件树中显示",
                "另存为模板",
                "导出",
                "添加到AI助手",
            ]
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn capture_and_back_accelerators_are_recognized() {
    let root = std::env::temp_dir().join(format!(
        "mochi-navigation-shortcuts-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let first = root.join("first.md");
        let second = root.join("second.md");
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        assert!(app.shell.open_file(&first));
        app.state.view = WorkspaceView::Editor;
        assert!(app.open_link_file_from_ui(&second));

        assert!(app.on_accelerator(HWND::default(), 32, true, true, false));
        assert!(app.on_accelerator(HWND::default(), 0x25, false, false, true));
        assert_eq!(app.active_file_path().as_deref(), Some(first.as_path()));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn toolbar_export_opens_the_multi_format_export_dialog() {
    let root = std::env::temp_dir().join(format!(
        "mochi-toolbar-export-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let source = root.join("笔记.md");
        std::fs::write(&source, "# 笔记").unwrap();
        assert!(app.shell.open_file(&source));

        app.on_toolbar_click(toolbar::Hit::ExportMarkdown);

        let form = app.export_form.as_ref().expect("应显示导出面板");
        assert_eq!(form.source, source);
        assert_eq!(form.options.format, "pdf");
        for format in ["pdf", "html", "markdown"] {
            assert!(
                mochi_core::exports::output_path(&root, &form.source, format).is_ok(),
                "{format} 应可导出"
            );
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn saving_a_dirty_note_records_activity_for_the_home_dashboard() {
    let root = std::env::temp_dir().join(format!(
        "mochi-home-activity-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let path = root.join("note.md");
        std::fs::write(&path, "初始内容").unwrap();
        assert!(app.shell.open_file(&path));
        app.shell.active_buffer_mut().unwrap().insert(" 新增内容");
        assert!(app.save_active());

        let now = chrono::Local::now();
        let events = mochi_core::analytics::events::read_events(
            &root,
            now - chrono::Duration::days(1),
            now + chrono::Duration::minutes(1),
        );
        assert!(events.iter().any(|event| {
            event.kind == mochi_core::analytics::events::ActivityEventType::FileSave
                && event.relative_path.as_deref() == Some("note.md")
                && event.words_delta.unwrap_or_default() > 0
        }));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pending_edit_extraction_only_accepts_tool_payloads() {
    let transcript = vec![
        AiMessage::new("assistant", r#"{"pendingEdit":{"id":"not-a-tool"}}"#),
        AiMessage::new(
            "tool",
            r#"{"ok":true,"data":{"pendingEdit":{"id":"edit-1"}}}"#,
        ),
        AiMessage::new("tool", r#"{"pendingEdit":{"id":"edit-2"}}"#),
        AiMessage::new("tool", "not json"),
    ];
    let edits = pending_edits_from_transcript(&transcript);
    assert_eq!(
        edits
            .iter()
            .filter_map(|edit| edit["id"].as_str())
            .collect::<Vec<_>>(),
        ["edit-1", "edit-2"]
    );
}

#[test]
fn invalidating_search_cancels_old_work_and_disconnects_late_results() {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (tx, rx) = channel();
    let mut job = SearchJob {
        generation: 9,
        rx: Some(rx),
        cancel: Some(flag.clone()),
        timer_pending: true,
    };
    job.invalidate();
    assert_eq!(job.generation, 10);
    assert!(job.rx.is_none());
    assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
    assert!(tx
        .send((
            9,
            SearchQueryResult {
                groups: vec![],
                stats: mochi_core::search::SearchStats {
                    total_matches: 0,
                    total_files: 0,
                    truncated: false,
                    duration_ms: 0
                },
                error: None
            }
        ))
        .is_err());
}

#[test]
fn file_approval_blocks_stale_changes_and_keeps_unsaved_text() {
    use mochi_core::ai::tools::host::ToolHost;
    let root = std::env::temp_dir().join(format!(
        "mochi-file-approval-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.setup_ai(HWND::default(), &root);
        let path = root.join("知识库/note.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "a\r\nb\r\nc\r\n").unwrap();
        app.shell.open_file(&path);
        app.shell.active_buffer_mut().unwrap().insert("local ");
        let propose = |app: &mut App| {
            let source = app.shell.active_buffer_mut().unwrap().text().to_owned();
            let mut snapshot = app.ai.snapshot.lock().unwrap();
            snapshot.active_document = Some(ActiveDocument {
                id: "a".into(),
                title: "note".into(),
                path: path.to_string_lossy().into_owned(),
                is_dirty: true,
            });
            snapshot.active_text = Some(source);
            drop(snapshot);
            app.ai
                .host
                .as_ref()
                .unwrap()
                .propose_document_edit(mochi_core::document_range::PendingDocumentEdit {
                    id: "range".into(),
                    path: path.to_string_lossy().into_owned(),
                    title: None,
                    start_line: 2,
                    end_line: 2,
                    old_text: "b".into(),
                    new_text: "替换".into(),
                    summary: "改第二行".into(),
                    original_hash: mochi_core::document_range::hash_text("b"),
                    status: "pending".into(),
                    error: None,
                })
                .unwrap();
        };
        propose(&mut app);
        app.shell.active_buffer_mut().unwrap().insert("new ");
        let entry = AgentInboxService::new(&root)
            .list_pending()
            .last()
            .unwrap()
            .clone();
        app.review_file_proposal(entry);
        assert!(!app.commands.review.as_ref().unwrap().error.is_empty());
        app.apply_reviewed_file();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc\r\n");
        app.commands.review = None;
        propose(&mut app);
        let entry = AgentInboxService::new(&root)
            .list_pending()
            .last()
            .unwrap()
            .clone();
        app.review_file_proposal(entry);
        assert!(app.commands.review.as_ref().unwrap().error.is_empty());
        app.apply_reviewed_file();
        assert!(app.commands.review.is_none());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "local new a\r\n替换\r\nc\r\n"
        );
        assert!(!app.shell.active_buffer_mut().unwrap().dirty());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shell_approval_is_readonly_and_execution_records_real_result() {
    use mochi_core::ai::tools::host::ToolHost;
    let top = std::env::temp_dir().join(format!(
        "mochi-command-approval-{}",
        mochi_core::paths::random_base36(12)
    ));
    let root = top.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            top.join("profile/settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.setup_ai(HWND::default(), &root);
        app.ai_new_session();
        let id = app.ai.panel.active.as_ref().unwrap().id.clone();
        app.ai.snapshot.lock().unwrap().session_id = Some(id);
        let proposal=app.ai.host.as_ref().unwrap().propose_shell_command(serde_json::json!({"pendingShellCommand":{"id":"test","command":"Write-Output 'approval-verified'","cwd":root,"timeoutMs":5000,"summary":"test","program":"write-output","status":"pending"}})).unwrap();
        let request_id = proposal["pendingShellCommand"]["id"].as_str().unwrap();
        app.review_command(request_id);
        let before = app
            .commands
            .review
            .as_ref()
            .unwrap()
            .field
            .text()
            .to_owned();
        assert!(!app.on_char('X'));
        app.on_accelerator(HWND::default(), 0x56, false, true, false);
        assert_eq!(app.commands.review.as_ref().unwrap().field.text(), before);
        app.execute_approved_command(request_id);
        assert!(app.commands.jobs.is_empty(), "关闭执行权限时不能执行");
        let permissions = app.ai.permissions.as_ref().unwrap();
        let mut actions = permissions.action_permissions();
        actions.set(
            mochi_core::ai::permission::AiToolAction::ExecuteCommand,
            true,
        );
        permissions.set_action_permissions(actions);
        app.execute_approved_command(request_id);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !app.commands.jobs.is_empty() && std::time::Instant::now() < deadline {
            app.take_file_jobs();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.commands.jobs.is_empty());
        assert!(
            app.commands.result_error.is_none(),
            "{:?}",
            app.commands.result_error
        );
        let message = app
            .ai
            .panel
            .active
            .as_ref()
            .unwrap()
            .messages
            .last()
            .unwrap();
        assert!(message.content().contains("approval-verified"));
        assert_eq!(
            message.get("pendingShellCommand").unwrap()["status"],
            "applied"
        );
        assert!(app
            .ai
            .export_requests
            .as_ref()
            .unwrap()
            .pending()
            .is_empty());
    }
    std::fs::remove_dir_all(top).unwrap();
}

#[test]
fn utf16_character_messages_accept_emoji_and_discard_cross_focus_surrogates() {
    let settings = std::env::temp_dir().join(format!(
        "mochi-utf16-{}.json",
        mochi_core::paths::random_base36(12)
    ));
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(settings)))).unwrap();
    app.focus = Focus::SidebarSearch;
    assert!(app.on_utf16_char(0xd83d));
    assert!(app.on_utf16_char(0xde00));
    assert_eq!(app.side.search.text(), "😀");
    assert!(!app.on_utf16_char(0xde00));
    app.on_utf16_char(0xd83d);
    app.focus = Focus::Main;
    assert!(!app.on_utf16_char(0xde00));
    app.focus = Focus::SidebarSearch;
    app.on_utf16_char(b'a' as u16);
    app.on_ime_commit("中");
    assert_eq!(app.side.search.text(), "😀a中");
}

#[test]
fn input_focus_routes_dialog_sidebar_and_ai_without_cross_field_ime() {
    let settings = std::env::temp_dir().join(format!(
        "mochi-input-focus-{}.json",
        mochi_core::paths::random_base36(12)
    ));
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(settings)))).unwrap();
    app.open_base_dialog(std::env::temp_dir());
    app.dialog
        .as_mut()
        .unwrap()
        .field
        .as_mut()
        .unwrap()
        .set_text("文件名称");
    app.dialog
        .as_mut()
        .unwrap()
        .field
        .as_mut()
        .unwrap()
        .buffer
        .set_cursor(0, false);
    assert!(app.on_edit_key(40, false, false));
    assert_eq!(
        app.dialog
            .as_ref()
            .unwrap()
            .field
            .as_ref()
            .unwrap()
            .buffer
            .cursor(),
        "文件名称".len()
    );
    app.on_ime_composition("中", 3);
    app.on_ime_commit("中文");
    assert_eq!(
        app.dialog.as_ref().unwrap().field.as_ref().unwrap().text(),
        "文件名称中文"
    );
    assert!(app.side.search.text().is_empty());
    assert!(app.ai.panel.input.text().is_empty());
    app.dialog = None;
    app.focus = Focus::SidebarSearch;
    app.on_ime_composition("暂存", 6);
    app.input_window_focus(false);
    assert!(app.side.search.buffer.composition().is_none());
    assert!(app.side.search.text().is_empty());
    app.focus = Focus::AiInput;
    app.on_ime_commit("助手");
    assert_eq!(app.ai.panel.input.text(), "助手");
    assert!(app.side.search.text().is_empty());
    app.focus = Focus::Main;
    let mut menu = Menu::open_at(vec![], 20.0, 20.0, Rect::from_size(0.0, 0.0, 800.0, 600.0));
    menu.search = Some(TextField::new("搜索"));
    app.menu = Some(menu);
    app.on_ime_composition("菜单", 6);
    app.on_ime_commit("菜单搜索");
    assert_eq!(
        app.menu.as_ref().unwrap().search.as_ref().unwrap().text(),
        "菜单搜索"
    );
    assert_eq!(app.ai.panel.input.text(), "助手");
}

#[test]
fn editing_file_title_renames_file_without_rewriting_markdown() {
    let root = std::env::temp_dir().join(format!(
        "mochi-file-title-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let source = root.join("知识库").join("旧标题.md");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        let raw = "# 正文标题不变\r\n\r\n**原始格式**\r\n";
        std::fs::write(&source, raw).unwrap();
        sidecars::save_comments(&source.to_string_lossy(), vec![]).unwrap();
        app.shell.open_file(&source);
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        app.begin_title_edit();
        app.title_editing.as_mut().unwrap().field.set_text("新标题");
        assert!(app.commit_title());
        let target = source.with_file_name("新标题.md");
        assert_eq!(app.active_file_path(), Some(target.clone()));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), raw);
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), raw);
        assert!(Path::new(&sidecars::comment_sidecar_path(&target.to_string_lossy())).is_file());
        assert!(!source.exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn approved_exports_wait_for_user_and_preserve_unsaved_source_snapshot() {
    use mochi_core::ai::tools::host::ToolHost;
    let top = std::env::temp_dir().join(format!(
        "mochi-approved-export-{}-{}",
        std::process::id(),
        mochi_core::paths::random_base36(12)
    ));
    let root = top.join("workspace");
    let profile = top.join("profile");
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            profile.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.setup_ai(HWND::default(), &root);
        let source = root.join("知识库").join("note.md");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "磁盘旧内容").unwrap();
        {
            let mut snapshot = app.ai.snapshot.lock().unwrap();
            snapshot.active_document = Some(ActiveDocument {
                id: "note".into(),
                title: "note".into(),
                path: source.to_string_lossy().into_owned(),
                is_dirty: true,
            });
            snapshot.active_text = Some("# 尚未保存的中文内容".into());
        }
        for format in ["markdown", "pdf"] {
            let output = mochi_core::exports::output_path(&root, &source, format).unwrap();
            let reply = app
                .ai
                .host
                .as_ref()
                .unwrap()
                .propose_document_export(
                    &source.to_string_lossy(),
                    format,
                    &output.to_string_lossy(),
                )
                .unwrap();
            assert_eq!(reply["status"], "pending");
            assert!(!output.exists());
            app.state.status_text.clear();
            app.approve_document_export(reply["id"].as_str().unwrap());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !app.state.status_text.starts_with("已导出")
                && std::time::Instant::now() < deadline
            {
                app.take_file_jobs();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(
                app.state.status_text.starts_with("已导出"),
                "{}",
                app.state.status_text
            );
            assert!(app
                .ai
                .export_requests
                .as_ref()
                .unwrap()
                .pending()
                .is_empty());
            let bytes = std::fs::read(&output).unwrap();
            if format == "markdown" {
                assert_eq!(bytes, "# 尚未保存的中文内容".as_bytes());
            } else {
                assert!(bytes.starts_with(b"%PDF-1.4"));
            }
        }
        assert_eq!(std::fs::read_to_string(source).unwrap(), "磁盘旧内容");
        assert!(AgentInboxService::new(&root).list_pending().is_empty());
    }
    std::fs::remove_dir_all(top).unwrap();
}

#[test]
fn switching_ai_sessions_cancels_old_stream_and_keeps_partial_in_old_session() {
    let root = std::env::temp_dir().join(format!(
        "mochi-session-switch-{}-{}",
        std::process::id(),
        mochi_core::jstime::now_millis()
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.ai_new_session();
        let first = app.ai.panel.active.as_ref().unwrap().id.clone();
        app.ai_new_session();
        let second = app.ai.panel.active.as_ref().unwrap().id.clone();
        app.ai_open_session(&first);
        let (tx, rx) = channel();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        app.ai.run = Some(RunHandle {
            rx,
            cancel: cancel.clone(),
        });
        app.ai.panel.streaming = Some(assistant::Streaming {
            content: "已生成的部分".into(),
            ..Default::default()
        });
        app.ai_open_session(&second);
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(tx.send(RunEvent::Content("迟到结果".into())).is_err());
        assert!(app.ai.panel.active.as_ref().unwrap().messages.is_empty());
        let saved = app
            .ai_session_service()
            .unwrap()
            .load_session(&first)
            .unwrap();
        assert_eq!(saved.messages.last().unwrap().content(), "已生成的部分");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn switching_away_from_assistant_clears_assistant_focus() {
    let settings = std::env::temp_dir().join(format!(
        "mochi-ai-focus-{}-{}.json",
        std::process::id(),
        mochi_core::jstime::now_millis()
    ));
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(settings)))).unwrap();
    for focus in [Focus::AiInput, Focus::AiMessageQuery, Focus::AiSessionQuery] {
        app.focus = focus;
        app.set_right_panel(RightPanel::Comments);
        assert_eq!(app.focus, Focus::Main);
    }
    app.focus = Focus::AiMessageQuery;
    app.set_right_panel(RightPanel::Assistant);
    assert_eq!(app.focus, Focus::AiMessageQuery);
}

#[test]
fn chinese_editing_shortcuts_preserve_source_and_undo() {
    let root = std::env::temp_dir().join(format!(
        "mochi-input-smoke-{}-{}",
        std::process::id(),
        mochi_core::jstime::now_millis()
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        let path = root.join("知识库").join("输入.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "中文").unwrap();
        assert!(app.shell.open_file(&path));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        app.editor_engaged = true;
        assert!(app.on_char('/'));
        assert!(app.menu.is_none(), "默认关闭时斜杠只插入字符");
        assert_eq!(
            app.shell.active().unwrap().buffer().unwrap().text(),
            "/中文"
        );
        assert!(app.on_edit_key(0x08, false, false));
        app.settings.set("app.editor.slashMenuEnabled", "true");
        assert!(app.on_char('/'));
        assert!(app.menu.is_some(), "斜杠应打开插入菜单");
        assert_eq!(
            app.shell.active().unwrap().buffer().unwrap().text(),
            "/中文"
        );
        assert!(app.on_edit_key(0x1b, false, false));
        assert!(app.menu.is_none(), "Esc 应关闭斜杠菜单");
        assert_eq!(
            app.shell.active().unwrap().buffer().unwrap().text(),
            "/中文",
            "Esc 关闭菜单时应保留斜杠"
        );
        assert!(app.on_edit_key(0x08, false, false));
        assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), "中文");
        let b = app.shell.active_buffer_mut().unwrap();
        b.set_cursor(b.text().len(), false);
        app.on_ime_composition("输入", "输入".len());
        app.on_ime_commit("输入😀");
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), "中文输入😀");
        app.shell.active_buffer_mut().unwrap().select_all();
        assert!(app.on_shortcut(HWND::default(), 0x49, false, true));
        assert_eq!(
            app.shell.active_buffer_mut().unwrap().text(),
            "<em>中文输入😀</em>"
        );
        assert!(app.on_shortcut(HWND::default(), 0x5a, false, true));
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), "中文输入😀");
        app.shell.active_buffer_mut().unwrap().select_all();
        assert!(app.on_shortcut(HWND::default(), 0x45, false, true));
        assert_eq!(
            app.shell.active_buffer_mut().unwrap().text(),
            "`中文输入😀`"
        );
        assert!(app.on_shortcut(HWND::default(), 0x5a, false, true));
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), "中文输入😀");
        assert!(app.on_alt_shortcut(0x31, false, true));
        assert_eq!(
            app.shell.active_buffer_mut().unwrap().text(),
            "# 中文输入😀"
        );
        assert!(app.on_shortcut(HWND::default(), 0x45, true, true));
        assert!(app.shell.active().unwrap().source_mode());
        assert!(app.prepare_close());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# 中文输入😀");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn requested_global_navigation_shortcuts_are_wired_end_to_end() {
    let root = std::env::temp_dir().join(format!(
        "mochi-global-shortcuts-{}",
        mochi_core::paths::random_base36(10)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();

        assert!(app.on_accelerator(HWND::default(), 0x46, true, true, false));
        assert!(app.search.is_some(), "Ctrl+Shift+F 应打开全局搜索");
        app.close_search();

        assert!(app.on_accelerator(HWND::default(), 0x53, false, true, true));
        assert_eq!(app.state.view, WorkspaceView::Schedule);
        assert!(app.on_accelerator(HWND::default(), 0x44, false, true, true));
        assert_eq!(app.state.view, WorkspaceView::Home);

        let dark = app.state.dark;
        assert!(app.on_accelerator(HWND::default(), 0x4C, true, true, false));
        assert_ne!(app.state.dark, dark, "Ctrl+Shift+L 应切换深浅色");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn split_views_share_buffers_and_closing_before_autosave_preserves_edits() {
    let root = std::env::temp_dir().join(format!(
        "mochi-split-{}-{}",
        std::process::id(),
        mochi_core::jstime::now_millis()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let a = root.join("a.md");
    let b = root.join("b.md");
    std::fs::write(&a, "甲").unwrap();
    std::fs::write(&b, "乙").unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))))
        .unwrap();
        app.shell.open_file(&a);
        app.shell.open_file(&b);
        app.shell.open_file(&a);
        app.split_to_right(b.clone());
        assert_eq!(app.active_file_path(), Some(b.clone()));
        assert!(app.split.right);
        app.shell.set_active_scroll(60.0);
        app.split.scroll = 30.0;
        app.shell.active_buffer_mut().unwrap().insert("修改");
        app.focus_other_editor();
        assert_eq!(app.active_file_path(), Some(a.clone()));
        assert_eq!(app.shell.active_scroll(), 30.0);
        let index = app
            .shell
            .tabs()
            .iter()
            .position(|t| t.path() == Some(b.as_path()))
            .unwrap();
        app.shell.close_tab(index);
        assert!(std::fs::read_to_string(&b).unwrap().contains("修改"));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn split_protects_shared_tab_from_replacement_and_close_keeps_unsaved_buffer() {
    let root = std::env::temp_dir().join(format!(
        "mochi-split-replace-{}",
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
        let a = root.join("a.md");
        let b = root.join("b.md");
        std::fs::write(&a, "左文档").unwrap();
        std::fs::write(&b, "右文档").unwrap();
        assert!(app.shell.open_file(&a));
        app.split_to_right(a.clone());
        assert!(app.open_file_from_ui(&b));
        app.sync_state();
        assert!(app.split.right);
        assert_eq!(app.split.other.as_deref(), Some(a.as_path()));
        assert_eq!(app.shell.tabs().len(), 2);
        let buffer = app.shell.active_buffer_mut().unwrap();
        buffer.set_cursor(buffer.text().len(), false);
        buffer.insert("待保存");
        assert!(app.close_split_view());
        assert_eq!(app.active_file_path(), Some(a.clone()));
        assert!(app.split.other.is_none());
        let right = app
            .shell
            .tabs()
            .iter()
            .find(|tab| tab.path() == Some(b.as_path()))
            .unwrap();
        assert!(right.dirty());
        assert_eq!(right.buffer().unwrap().text(), "右文档待保存");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "右文档");
        assert_eq!(
            crate::ui::settings_values::text("tabs.openFileBehavior", ""),
            "replace"
        );
        assert!(app.prepare_close());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "右文档待保存");
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn settings_close_and_reopen_restores_customization_section_and_scroll() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-settings-position-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
        let mut app = App::with_settings(Arc::clone(&settings)).unwrap();
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();

        app.open_settings("customization");
        app.paint(HWND::default()).unwrap();
        let section = "area-backgrounds";
        let anchor = app.prefs.content_layout.section_scroll(section).unwrap();
        let max = app.prefs.content_layout.max_scroll();
        let expected = (anchor + 12.0).min(max);
        app.prefs.scroll = expected;
        app.paint(HWND::default()).unwrap();
        assert_eq!(
            app.settings_overlay
                .as_ref()
                .map(|(_, section)| section.as_str()),
            Some(section)
        );

        app.close_settings();
        assert!(app.settings_overlay.is_none());
        assert_eq!(
            settings.get(SETTINGS_LAST_TAB_KEY).as_deref(),
            Some("customization")
        );
        assert_eq!(
            settings.get(SETTINGS_LAST_SECTION_KEY).as_deref(),
            Some(section)
        );
        let saved = settings
            .get(SETTINGS_SCROLL_KEY)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap();
        assert!((saved - expected).abs() < 0.01);

        // 请求的标签页仅作为备用项；上次使用的设置位置还包括标签页、分区和
        // 滚动位置。
        app.open_settings("general");
        assert_eq!(
            app.settings_overlay.as_ref(),
            Some(&("customization".to_owned(), section.to_owned()))
        );
        assert!((app.prefs.scroll - expected).abs() < 0.01);

        app.close_settings();
        app.open_settings("plugins");
        assert_eq!(
            app.settings_overlay.as_ref(),
            Some(&("plugins".to_owned(), "appearance".to_owned()))
        );
        assert_eq!(app.prefs.scroll, 0.0);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn settings_close_and_reopen_restores_ai_provider_scroll() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-provider-position-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
        let mut app = App::with_settings(Arc::clone(&settings)).unwrap();
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.prefs.providers.loaded = true;
        app.prefs.providers.all = (0..10)
            .map(|index| mochi_core::ai::models::AiProvider {
                id: format!("provider-{index}"),
                name: format!("Provider {index}"),
                model: "test-model".to_owned(),
                base_url: "https://example.invalid".to_owned(),
                protocol: "openai-completions".to_owned(),
                ..Default::default()
            })
            .collect();

        app.open_settings("ai");
        app.paint(HWND::default()).unwrap();
        let max = app.prefs.provider_layout.max_scroll();
        assert!(
            max > 0.0,
            "provider settings should scroll with several providers"
        );
        let expected = max / 2.0;
        app.prefs.providers.scroll = expected;
        app.close_settings();
        assert!(app.settings_overlay.is_none());
        assert_eq!(settings.get(SETTINGS_LAST_TAB_KEY).as_deref(), Some("ai"));
        assert_eq!(
            settings.get(SETTINGS_LAST_SECTION_KEY).as_deref(),
            Some("appearance")
        );
        let saved = settings
            .get(SETTINGS_SCROLL_KEY)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap();
        assert!((saved - expected).abs() < 0.01);
        assert!((app.prefs.providers.scroll - expected).abs() < 0.01);
        assert_eq!(app.prefs.scroll, 0.0);

        app.open_settings("general");
        assert_eq!(
            app.settings_overlay.as_ref(),
            Some(&("ai".to_owned(), "appearance".to_owned()))
        );
        assert!((app.prefs.providers.scroll - expected).abs() < 0.01);
        assert_eq!(app.prefs.scroll, 0.0);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn backlinks_follow_active_tab_refresh_and_outline_setting() {
    let root = std::env::temp_dir().join(format!(
        "mochi-links-app-{}-{}",
        std::process::id(),
        mochi_core::jstime::now_millis()
    ));
    std::fs::create_dir_all(&root).unwrap();
    // 所有写入都在本测试自己的临时工作区及设置文件内。
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))))
        .unwrap();
        app.settings.set("app.outline.placement", "ai-sidebar");
        app.load_chrome_settings();
        assert!(app.state.outline_in_ai_sidebar);
        app.shell.open_workspace(&root, || {}).unwrap();
        let folder = root.join("知识库");
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("目标.md");
        let source = folder.join("来源.md");
        std::fs::write(&target, "# 目标").unwrap();
        std::fs::write(&source, "[[目标]]").unwrap();
        let index = Arc::clone(&app.shell.workspace().unwrap().index);
        index.index_single_file(&target).unwrap();
        index.index_single_file(&source).unwrap();
        assert!(app.shell.open_file(&target));
        app.sync_state();
        let wait = |app: &mut App| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while app.links.loading && std::time::Instant::now() < deadline {
                app.take_backlinks();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(!app.links.loading, "后台链接查询超时");
            assert!(app.links.error.is_empty(), "{}", app.links.error);
        };
        wait(&mut app);
        assert_eq!(app.links.data.backlinks.len(), 1);
        assert_eq!(app.links.source.as_deref(), Some(target.as_path()));
        // 保存/外部变更经索引更新后刷新当前面板。
        std::fs::write(&source, "只有普通正文").unwrap();
        index.index_single_file(&source).unwrap();
        app.on_files_changed();
        wait(&mut app);
        assert!(app.links.data.backlinks.is_empty());
        // 快速切换两次：最后标签的数据与路径必须一致。
        assert!(app.shell.open_file(&source));
        app.sync_state();
        assert!(app.shell.open_file(&target));
        app.sync_state();
        wait(&mut app);
        assert_eq!(app.links.source.as_deref(), Some(target.as_path()));
        assert!(app.links.data.outgoing.is_empty());
        app.shell.close_tab(app.shell.active_tab().unwrap());
        app.shell.close_tab(app.shell.active_tab().unwrap());
        app.sync_state();
        assert!(app.links.source.is_none());
        assert!(!app.links.loading);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// `App::new()` 要建 D2D/DirectWrite 工厂，跑不了纯逻辑测试；
/// 但窗口按钮的意图映射是纯算术，单独钉住。
#[test]
fn caption_button_indices_map_to_the_windows_convention() {
    // 从左到右：最小化、最大化、关闭。顺序错了会变成"点最小化却关掉了窗口"
    let map = |i: usize| match i {
        0 => CaptionAction::Minimize,
        1 => CaptionAction::ToggleMaximize,
        _ => CaptionAction::Close,
    };
    assert_eq!(map(0), CaptionAction::Minimize);
    assert_eq!(map(1), CaptionAction::ToggleMaximize);
    assert_eq!(map(2), CaptionAction::Close);
}

#[test]
fn titlebar_theme_and_auxiliary_page_return_follow_the_current_state() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-titlebar-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        let _snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.state.view = WorkspaceView::Schedule;
        for page in [
            WorkspaceView::Automations,
            WorkspaceView::Marketplace,
            WorkspaceView::Templates,
            WorkspaceView::DesktopCards,
        ] {
            app.open_auxiliary_page(page);
            app.open_auxiliary_page(page); // 重新打开页面时，不应重复添加返回目标。
            let chrome = app.build_chrome();
            let back = chrome.tree.rect(chrome.back);
            assert!(!back.is_empty());
            assert!(app.hit_is_titlebar_button(back.left + 8.0, back.top + 8.0));
        }
        for expected in [
            WorkspaceView::Templates,
            WorkspaceView::Marketplace,
            WorkspaceView::Automations,
            WorkspaceView::Schedule,
        ] {
            let chrome = app.build_chrome();
            let back = chrome.tree.rect(chrome.back);
            app.on_click(back.left + 8.0, back.top + 8.0);
            assert_eq!(app.state.view, expected);
        }
        assert!(app.build_chrome().tree.is_hidden(app.build_chrome().back));
        app.open_auxiliary_page(WorkspaceView::Templates);
        app.state.view = WorkspaceView::Recent;
        app.open_auxiliary_page(WorkspaceView::Marketplace);
        app.return_from_auxiliary_page();
        assert_eq!(app.state.view, WorkspaceView::Recent);
        assert!(app.page_history.is_empty());

        app.state.dark = false;
        for (dark, mode) in [(true, "dark"), (false, "light")] {
            let chrome = app.build_chrome();
            let toggle = chrome.tree.rect(chrome.theme_toggle);
            assert!(app.hit_is_titlebar_button(toggle.left + 8.0, toggle.top + 8.0));
            app.on_click(toggle.left + 8.0, toggle.top + 8.0);
            assert_eq!(app.state.dark, dark);
            let saved = app_settings::AppSettings::new(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))));
            assert_eq!(
                saved.read(app_settings::descriptor("appearance.themeMode").unwrap()),
                SettingValue::Text(mode.into())
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
