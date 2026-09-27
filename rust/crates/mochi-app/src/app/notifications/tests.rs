use super::*;
use crate::ui::draw::DrawCmd;

fn with_app(run: impl FnOnce(&mut App, &Path)) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-notifications-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("note.md");
    std::fs::write(&path, "# 原文\n\n内容保持不变\n").unwrap();
    {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        assert!(app.shell.open_file_with_mode(&path, true));
        app.state.view = WorkspaceView::Editor;
        app.sync_state();
        app.renderer.prepare_snapshot(1280, 800, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        app.notifications.view.history = ui::History::default();
        run(&mut app, &root);
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn click(app: &mut App, hit: Hit) {
    let lay = app.notification_layout();
    let rect = lay
        .controls
        .iter()
        .find(|(_, h)| *h == hit)
        .map(|(r, _)| *r)
        .or_else(|| {
            lay.rows
                .iter()
                .find(|(_, id)| hit == Hit::Notice(*id))
                .map(|(r, _)| *r)
        })
        .unwrap();
    app.on_click(rect.left + 5.0, rect.top + 5.0);
}

fn set_preference(app: &App, key: &str, value: bool) {
    app.app_settings.write(
        app_settings::descriptor(key).unwrap(),
        &SettingValue::Bool(value),
    );
}

#[test]
fn global_notice_clears_title_buttons_and_workspace_toolbar() {
    with_app(|app, _| {
        for ai_open in [false, true] {
            app.state.ai_panel_open = ai_open;
            app.show_global_notice("当前按名称排序，切换为手动排序后才能调整顺序");
            app.status_bar.toast.expires = i64::MAX;
            app.paint(HWND::default()).unwrap();
            let chrome = app.build_chrome();
            let safe_top = chrome
                .tree
                .rect(chrome.title_bar)
                .bottom
                .max(chrome.tree.rect(chrome.tab_bar).bottom)
                .max(app.toolbar_area.bottom)
                .max(app.split.toolbar.bottom)
                .max(chrome.tree.rect(chrome.right_sidebar_toolbar).bottom);
            let message = app
                .list
                .cmds()
                .iter()
                .find_map(|cmd| match cmd {
                    DrawCmd::Text { rect, text, .. } if text.starts_with("当前按名称排序") => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .expect("visible notice text");
            assert!(message.top >= safe_top + 16.0);
        }
    });
}

#[test]
fn notifications_titlebar_has_robot_and_bell_with_stable_hit_regions() {
    with_app(|app, _| {
        let chrome = app.build_chrome();
        let bell = chrome.tree.rect(chrome.notifications_toggle);
        let robot = chrome.tree.rect(chrome.ai_toggle);
        assert_eq!(bell.right, robot.left);
        assert!(app.hit_is_titlebar_button(bell.left + 3.0, bell.top + 3.0));
        assert!(app.hit_is_titlebar_button(robot.left + 3.0, robot.top + 3.0));
        assert_eq!(
            chrome.hit(bell.left + 3.0, bell.top + 3.0),
            Some(NodeKey::TitleBarNotifications)
        );
        assert!(app.list.cmds().iter().any(|c| matches!(c, DrawCmd::Icon { icon, rect, .. } if *icon == Icon::BOT && rect.top < 34.0)));
        assert!(!app.list.cmds().iter().any(
            |c| matches!(c, DrawCmd::Text { text, rect, .. } if text == "AI" && rect.top < 34.0)
        ));
        let ai_was_open = app.state.ai_panel_open;
        app.on_click(robot.left + 5.0, robot.top + 5.0);
        assert_ne!(app.state.ai_panel_open, ai_was_open);
        app.on_click(bell.left + 5.0, bell.top + 5.0);
        assert!(app.notification_open());
        assert_eq!(app.notifications.view.mode, Mode::Popover);
    });
}

#[test]
fn notifications_popup_more_filters_read_and_persistence_work_end_to_end() {
    with_app(|app, root| {
        app.show_global_notice("文件导出完成");
        app.publish_notification(Category::Assistant, "AI 回复完成", "查看助手回复");
        assert_eq!(app.notifications.view.history.unread(), 2);
        assert!(app
            .take_timer_requests()
            .iter()
            .any(|(id, _)| *id == platform::TIMER_NOTIFICATIONS));
        app.toggle_notifications();
        assert_eq!(app.notifications.view.history.unread(), 2);
        click(app, Hit::More);
        assert_eq!(app.notifications.view.mode, Mode::Center);
        click(app, Hit::Filter(ui::Filter::Category(Category::Document)));
        let lay = app.notification_layout();
        assert_eq!(lay.rows.len(), 1);
        click(app, Hit::Notice(lay.rows[0].1));
        assert!(app.notifications.view.selected.is_some());
        assert_eq!(app.notifications.view.history.unread(), 1);
        app.on_timer(HWND::default(), platform::TIMER_NOTIFICATIONS);
        assert!(!app.notifications.view.save_failed);
        let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
        let restored = State::new(&settings);
        assert_eq!(
            restored.view.history.entries,
            app.notifications.view.history.entries
        );
        click(app, Hit::MarkAllRead);
        assert_eq!(app.notifications.view.history.unread(), 0);
        app.paint(HWND::default()).unwrap();
        assert!(app.list.finish().is_ok());
    });
}

#[test]
fn notifications_modal_blocks_text_ime_shortcuts_mouse_and_underlying_scroll() {
    with_app(|app, _| {
        app.focus = Focus::Main;
        app.editor_engaged = true;
        let before = app
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .to_owned();
        let selection = app.shell.active().unwrap().buffer().unwrap().selection();
        let scroll = app.shell.active_scroll();
        app.toggle_notifications();
        click(app, Hit::More);
        assert!(app.on_char('x'));
        app.on_ime_composition("拼音", 3);
        app.on_ime_commit("误输入");
        assert!(app.on_accelerator(HWND::default(), 0x41, false, true, false));
        app.on_double_click(500.0, 500.0);
        app.on_right_click(500.0, 500.0);
        app.on_wheel(10.0, 600.0, -120);
        assert!(app.on_horizontal_wheel(10.0, 600.0, -120));
        app.paint(HWND::default()).unwrap();
        assert!(app.list.caret_rect().is_none());
        let buffer = app.shell.active().unwrap().buffer().unwrap();
        assert_eq!(buffer.text(), before);
        assert_eq!(buffer.selection(), selection);
        assert_eq!(app.shell.active_scroll(), scroll);
        assert!(app.menu.is_none());
        assert!(app.on_edit_key(0x1b, false, false));
        assert!(!app.notification_open());
        assert_eq!(app.focus, Focus::Main);
        assert!(app.on_char('x'));
        assert_ne!(app.shell.active().unwrap().buffer().unwrap().text(), before);
    });
}

#[test]
fn notifications_outside_click_is_consumed_and_overlay_scrollbar_can_drag() {
    with_app(|app, _| {
        for i in 0..50 {
            app.publish_notification(Category::System, "通知", &format!("消息 {i}"));
        }
        app.focus = Focus::SidebarSearch;
        app.toggle_notifications();
        click(app, Hit::More);
        let lay = app.notification_layout();
        let bar = lay.bars()[0].1;
        app.on_click(bar.thumb.left + 1.0, bar.thumb.top + 2.0);
        assert!(app.is_dragging());
        app.on_mouse_move(bar.thumb.left + 1.0, lay.body.bottom + 100.0);
        app.end_drag_at(bar.thumb.left + 1.0, lay.body.bottom + 100.0);
        assert!(!app.is_dragging());
        assert_eq!(app.notifications.view.scroll, lay.max_scroll);
        app.on_click(1.0, 400.0);
        assert!(!app.notification_open());
        assert_eq!(app.focus, Focus::SidebarSearch);
    });
}

#[test]
fn notifications_progress_updates_do_not_fill_history_and_status_is_recorded_once() {
    with_app(|app, _| {
        for i in 0..20 {
            app.show_global_progress("正在导入", i, 20);
        }
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.notifications.view.history.unread(), 0);
        app.state.status_text = "导入完成".into();
        app.paint(HWND::default()).unwrap();
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.notifications.view.history.entries.len(), 1);
        assert_eq!(app.notifications.view.history.entries[0].occurrences, 1);
        app.show_global_notice("文件保存失败");
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.notifications.view.history.entries.len(), 2);
        assert_eq!(app.notifications.view.history.entries[0].occurrences, 1);
        app.show_global_notice("已复制");
        assert_eq!(app.status_bar.toast.message, "已复制");
        assert_eq!(app.notifications.view.history.entries.len(), 2);
    });
}

#[test]
fn notifications_mute_from_details_persists_and_settings_can_restore_each_category() {
    with_app(|app, root| {
        set_preference(app, AGGREGATE_KEY, false);
        for category in Category::ALL {
            app.publish_notification(category, "新通知", "需要提醒");
            assert_eq!(app.take_native_notification().unwrap().category, category);
            app.open_notification_center();
            let id = app.notifications.view.history.entries[0].id;
            click(app, Hit::Notice(id));
            click(app, Hit::ToggleCategory(category));
            assert!(!app.notification_category_enabled(category));
            assert!(app.notifications.view.muted.contains(&category));
            app.publish_notification(category, "关闭后", "仅保留记录");
            assert!(app.notifications.view.history.entries[0].read);
            assert!(app.take_native_notification().is_none());
            let restored = State::new(&Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))));
            assert!(restored.view.muted.contains(&category));

            click(app, Hit::Settings);
            assert!(!app.notification_open());
            assert_eq!(app.settings_tab().unwrap().0, "notifications");
            app.paint(HWND::default()).unwrap();
            let index = app_settings::descriptors()
                .iter()
                .position(|d| d.key == category.setting_key())
                .unwrap();
            let mut rect = app.prefs.content_layout.control_rect(index).unwrap();
            for _ in 0..12 {
                let body = app.prefs.content_layout.body;
                if rect.top >= body.top && rect.bottom <= body.bottom {
                    break;
                }
                app.on_wheel(body.left + 20.0, body.top + 20.0, -120);
                app.paint(HWND::default()).unwrap();
                rect = app.prefs.content_layout.control_rect(index).unwrap();
            }
            app.on_settings_content_click(rect.left + 3.0, rect.top + 3.0);
            assert!(
                app.notification_category_enabled(category),
                "{category:?}: {rect:?}"
            );
            assert!(!app.notifications.view.muted.contains(&category));
            app.publish_notification(category, "重新开启", "再次提醒");
            assert!(!app.notifications.view.history.entries[0].read);
            assert_eq!(app.take_native_notification().unwrap().category, category);
            app.close_settings();
        }
    });
}

#[test]
fn notifications_aggregate_burst_keeps_first_push_and_all_local_messages() {
    with_app(|app, root| {
        assert!(enabled(&app.app_settings, AGGREGATE_KEY));
        app.publish_notification(Category::Document, "导入完成", "文件一");
        app.publish_notification(Category::Assistant, "回复完成", "回答二");
        assert_eq!(app.take_native_notification().unwrap().message, "文件一");
        app.publish_notification(Category::System, "操作通知", "消息三");
        app.publish_notification(Category::System, "操作通知", "消息三");
        assert!(app.take_native_notification().is_none());
        assert_eq!(app.notifications.view.history.unread(), 3);
        assert_eq!(app.notifications.view.history.entries[0].occurrences, 2);

        app.notifications.last_native_activity = Some(std::time::Instant::now() - BURST_GAP);
        app.publish_notification(Category::Schedule, "日程提醒", "安静后恢复提醒");
        assert_eq!(
            app.take_native_notification().unwrap().message,
            "安静后恢复提醒"
        );
        app.save_notifications();
        let restored = State::new(&Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))));
        assert_eq!(
            restored.view.history.entries,
            app.notifications.view.history.entries
        );
    });
}

#[test]
fn notifications_aggregate_switch_and_muted_messages_do_not_lose_reminders() {
    with_app(|app, root| {
        set_preference(app, Category::System.setting_key(), false);
        app.publish_notification(Category::System, "已静音", "不占用聚合窗口");
        app.publish_notification(Category::Document, "完成", "首条推送");
        assert_eq!(app.take_native_notification().unwrap().message, "首条推送");
        set_preference(app, AGGREGATE_KEY, false);
        app.apply_setting_side_effects(AGGREGATE_KEY);
        for message in ["第二条", "第三条"] {
            app.publish_notification(Category::Document, "完成", message);
            assert_eq!(app.take_native_notification().unwrap().message, message);
        }
        app.save_notifications();
        let settings = AppSettings::new(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))));
        assert!(!enabled(&settings, AGGREGATE_KEY));
        set_preference(app, WINDOWS_KEY, false);
        app.apply_setting_side_effects(WINDOWS_KEY);
        app.publish_notification(Category::Document, "完成", "仅在中心显示");
        assert!(app.take_native_notification().is_none());
        set_preference(app, WINDOWS_KEY, true);
        set_preference(app, AGGREGATE_KEY, true);
        app.publish_notification(Category::Document, "完成", "重新开启后推送");
        assert!(app.take_native_notification().is_some());
    });
}

#[test]
fn notifications_windows_switch_and_duplicate_delivery_preserve_local_history() {
    with_app(|app, _| {
        app.publish_notification(Category::Schedule, "日程提醒", "开会");
        assert!(app.take_native_notification().is_some());
        app.publish_notification(Category::Schedule, "日程提醒", "开会");
        assert!(app.take_native_notification().is_none());
        assert_eq!(app.notifications.view.history.entries[0].occurrences, 2);
        set_preference(app, WINDOWS_KEY, false);
        app.apply_setting_side_effects(WINDOWS_KEY);
        app.publish_notification(Category::Assistant, "回复完成", "查看回答");
        assert!(app.take_native_notification().is_none());
        assert_eq!(app.notifications.view.history.unread(), 2);
        set_preference(app, WINDOWS_KEY, true);
        app.publish_notification(Category::Document, "导入完成", "保留记录");
        // 发布到 Win32 消息之间若设置有变动，以最新设置为准。
        set_preference(app, Category::Document.setting_key(), false);
        assert!(app.take_native_notification().is_none());
        app.refresh_notification_preferences();
        assert!(app.notifications.view.history.entries[0].read);
        assert_eq!(app.notifications.view.history.entries.len(), 3);
    });
}

#[test]
fn notifications_mute_suppresses_banners_and_automation_uses_its_own_switch() {
    with_app(|app, _| {
        set_preference(app, Category::Automation.setting_key(), false);
        app.apply_setting_side_effects(Category::Automation.setting_key());
        app.show_global_notice("表格自动化已完成");
        app.paint(HWND::default()).unwrap();
        assert_eq!(
            app.notifications.view.history.entries[0].category,
            Category::Automation
        );
        assert!(app.notifications.view.history.entries[0].read);
        assert!(app.status_bar.toast.message.is_empty());
        assert!(app.take_native_notification().is_none());
        app.state.status_text = "工作流执行失败".into();
        app.paint(HWND::default()).unwrap();
        assert!(app.status_bar.toast.message.is_empty());
        app.show_global_notice("文件保存失败");
        assert_eq!(app.status_bar.toast.message, "文件保存失败");
        assert_eq!(
            app.take_native_notification().unwrap().category,
            Category::Document
        );
    });
}

#[test]
fn notifications_windows_failure_keeps_message_readable_and_activation_preserves_focus() {
    with_app(|app, _| {
        app.publish_notification(
            Category::System,
            "Windows 通知测试",
            "投递失败时仍能查看完整消息",
        );
        app.deliver_native_notification(HWND::default());
        assert!(app.notifications.view.windows_failed);
        assert_eq!(app.notifications.view.history.unread(), 1);
        app.focus = Focus::SidebarSearch;
        app.open_notification_center();
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.notifications.view.mode, Mode::Center);
        assert!(app.list.cmds().iter().any(
            |c| matches!(c, DrawCmd::Text { text, .. } if text.contains("Windows 提醒暂不可用"))
        ));
        app.close_notifications();
        assert_eq!(app.focus, Focus::SidebarSearch);
    });
}
