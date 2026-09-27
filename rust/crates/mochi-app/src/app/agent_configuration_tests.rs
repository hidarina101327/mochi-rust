use super::*;
use mochi_core::ai::session::{AiConversation, AiStoredMessage};

#[test]
fn bundled_agent_update_preview_preserves_edits_and_applies_only_reviewed_content() {
    use crate::ui::draw::DrawCmd;
    use mochi_core::ai::agent_config::AgentConfigService;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-agent-update-ui-{}",
        mochi_core::paths::random_base36(12)
    ));
    let path = root.join("Agent配置/Agents/通用助手.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // 使用明确较长的本地定义：目前逐步推出的内置 Agent 定义很短，
    // 因此滚动测试不能依赖上游定义的长度。
    let original_text = format!(
        "---\nname: 通用助手\n---\n这是用户的旧提示词。\n{}",
        "保留用户自定义的操作习惯。\n".repeat(50)
    );
    let original = original_text.as_str();
    std::fs::write(&path, original).unwrap();
    let service = AgentConfigService::new(&root);
    service.ensure_seeds().unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.shell.open_workspace(&root, || {}).unwrap();
    let snapshot = app.renderer.prepare_snapshot(1100, 800, 96.0).unwrap();
    app.load_chrome_settings();
    app.state.view = WorkspaceView::AgentConfig;
    app.check_agent_updates(true);
    let notice_count = app.notifications.view.history.entries.len();
    assert!(notice_count > 0);
    app.check_agent_updates(true);
    assert_eq!(app.notifications.view.history.entries.len(), notice_count);
    app.paint(HWND::default()).unwrap();
    let banner = app.agent.update_banner;
    assert!(banner.height() > 0.0);
    click(&mut app, banner);
    assert!(app.agent.update_review.is_some());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert!(app
        .list
        .cmds()
        .iter()
        .any(|cmd| matches!(cmd, DrawCmd::Text { text, .. } if text.contains("未知（旧工作区）"))));
    if let Some(output) = std::env::var_os("MOCHI_AGENT_UPDATE_QA") {
        let output = PathBuf::from(output);
        std::fs::create_dir_all(&output).unwrap();
        app.renderer
            .save_snapshot(&snapshot, &output.join("agent-update-preview.png"))
            .unwrap();
    }
    let body = app.agent.layout.body;
    app.on_wheel(body.left + 100.0, body.top + 60.0, -480);
    app.paint(HWND::default()).unwrap();
    assert!(app.agent.scroll > 0.0);
    let action = |app: &App, wanted| {
        app.agent
            .layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == wanted)
            .unwrap()
            .0
    };
    // 旧预览不能覆盖另一个编辑器随后保存的内容。
    std::fs::write(&path, "预览后保存的内容").unwrap();
    let apply = action(&app, agent_config::Hit::UpdateApply);
    click(&mut app, apply);
    assert!(app.agent.error.as_ref().unwrap().contains("预览后"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "预览后保存的内容");
    let back = action(&app, agent_config::Hit::UpdateBack);
    click(&mut app, back);
    // 已有基准版本后，仅审核上游内容时，必须先显示与本地版本的差异，
    // 用户确认后才能应用。
    let manifest_path = root.join("Agent配置/.bundled-agents/state.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["agents"]["Agents/通用助手.md"]["baseline"] = original.into();
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    app.reload_agent_config();
    app.paint(HWND::default()).unwrap();
    let banner = app.agent.update_banner;
    click(&mut app, banner);
    let mode = action(&app, agent_config::Hit::UpdateDiffMode);
    click(&mut app, mode);
    assert!(app.agent.update_upstream);
    let apply = action(&app, agent_config::Hit::UpdateApply);
    click(&mut app, apply);
    assert!(!app.agent.update_upstream);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "预览后保存的内容");
    let skip = action(&app, agent_config::Hit::UpdateSkip);
    click(&mut app, skip);
    assert!(service.agent_updates(false).unwrap().is_empty());
    assert!(app.agent.updates[0].skipped);
    // 即使编辑器已收起，未保存的源码草稿也必须保留。
    app.open_agent_source(&path.to_string_lossy());
    app.agent
        .source_editor
        .as_mut()
        .unwrap()
        .field
        .buffer
        .insert("未保存的编辑");
    app.agent.source_editor.as_mut().unwrap().dirty = true;
    app.paint(HWND::default()).unwrap();
    let banner = app.agent.update_banner;
    click(&mut app, banner);
    let apply = action(&app, agent_config::Hit::UpdateApply);
    click(&mut app, apply);
    assert!(app.agent.error.as_ref().unwrap().contains("未保存"));
    app.open_agent_source(&path.to_string_lossy());
    app.save_agent_source();
    app.paint(HWND::default()).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    let banner = app.agent.update_banner;
    click(&mut app, banner);
    let incoming = app.agent.update_review.as_ref().unwrap().incoming.clone();
    if let Some(output) = std::env::var_os("MOCHI_AGENT_UPDATE_QA") {
        app.state.status_text.clear();
        app.status_bar.toast = Default::default();
        app.paint(HWND::default()).unwrap();
        app.renderer
            .save_snapshot(
                &snapshot,
                &PathBuf::from(output).join("agent-update-local-changes.png"),
            )
            .unwrap();
    }
    let apply = action(&app, agent_config::Hit::UpdateApply);
    click(&mut app, apply);
    assert!(app.agent.update_review.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), incoming);
    assert!(service.agent_updates(true).unwrap().is_empty());
    let backups = std::fs::read_dir(root.join("Agent配置/.bundled-agents/backups"))
        .unwrap()
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read_to_string(backups[0].as_ref().unwrap().path()).unwrap(),
        saved
    );
    drop(app);
    let _ = std::fs::remove_dir_all(root);
}

fn click(app: &mut App, rect: Rect) {
    app.on_click(
        (rect.left + rect.right) / 2.0,
        (rect.top + rect.bottom) / 2.0,
    );
    app.paint(HWND::default()).unwrap();
}

#[test]
fn source_editor_scrolls_and_navigation_rows_render_hover_feedback() {
    use crate::ui::draw::DrawCmd;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-ai-refinements-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("TOOL.md");
    let text = (0..220)
        .map(|n| format!("工具配置第 {n:03} 行：保持滚动位置与编辑光标一致。\n"))
        .collect::<String>();
    std::fs::write(&source, &text).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    app.shell.open_workspace(&root, || {}).unwrap();
    let snapshot = app.renderer.prepare_snapshot(1440, 960, 96.0).unwrap();
    app.load_chrome_settings();
    let save = |app: &mut App, name: &str| {
        if let Some(output) = std::env::var_os("MOCHI_AI_REFINEMENT_QA") {
            let output = PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            app.renderer
                .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
                .unwrap();
        }
    };
    app.open_agent_source(&source.to_string_lossy());
    app.paint(HWND::default()).unwrap();
    let area = app.agent.source_editor.as_ref().unwrap().area;
    let before = app
        .agent
        .source_editor
        .as_ref()
        .unwrap()
        .field
        .multiline_caret(area);
    assert!(before.top >= area.top);
    let buttons = app
        .list
        .cmds()
        .iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Text {
                rect, text, align, ..
            } if text == "返回" || text == "保存" => {
                assert_eq!(*align, Align::Center);
                Some((text.clone(), *rect))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(buttons.len(), 2);
    save(&mut app, "source-top");
    app.on_wheel(area.left + 100.0, area.top + 100.0, -600);
    app.paint(HWND::default()).unwrap();
    let after = app
        .agent
        .source_editor
        .as_ref()
        .unwrap()
        .field
        .multiline_caret(area);
    assert!(after.top < before.top - 100.0);
    app.paint(HWND::default()).unwrap();
    assert_eq!(
        after,
        app.agent
            .source_editor
            .as_ref()
            .unwrap()
            .field
            .multiline_caret(area)
    );
    assert_eq!(app.agent.nav_scroll, 0.0);
    assert_eq!(app.agent.source_editor.as_ref().unwrap().field.text(), text);
    save(&mut app, "source-scrolled");
    app.on_click(area.left + 30.0, area.top + 15.0);
    let cursor = app
        .agent
        .source_editor
        .as_ref()
        .unwrap()
        .field
        .buffer
        .cursor();
    assert!(cursor > text.lines().next().unwrap().len() * 3);
    let edited = format!("{text}\n保存按钮验证");
    let editor = app.agent.source_editor.as_mut().unwrap();
    editor.field.set_text(&edited);
    editor.dirty = true;
    let save_button = buttons.iter().find(|(text, _)| text == "保存").unwrap().1;
    app.on_click(save_button.right - 1.0, save_button.top + 4.0);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), edited);
    assert!(!app.agent.source_editor.as_ref().unwrap().dirty);
    app.state.status_text.clear();
    app.status_bar.toast = Default::default();

    app.state.view = WorkspaceView::MochiAi;
    app.ai.panel.active = Some(AiConversation {
        id: "hover-session".into(),
        title: "算法求职学习计划".into(),
        messages: vec![
            AiStoredMessage::new("user", "如何安排未来三个月的算法练习？"),
            AiStoredMessage::new("assistant", "先复习基础，再按专题练习，最后进行模拟面试。"),
            AiStoredMessage::new("user", "每天可以投入两小时。"),
        ],
        ..Default::default()
    });
    assert!(app.ai_persist_active());
    app.paint(HWND::default()).unwrap();
    let row = app
        .ai
        .layout
        .rect_of(assistant::Hit::NavigateMessage(1))
        .unwrap();
    let hover = app
        .window_hover_rect(row.left + 15.0, row.top + 10.0)
        .unwrap();
    assert_eq!(
        app.cursor_for(row.left + 15.0, row.top + 10.0),
        Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND)
    );
    assert!(app.on_mouse_move(row.left + 15.0, row.top + 10.0));
    std::thread::sleep(std::time::Duration::from_millis(120));
    app.paint(HWND::default()).unwrap();
    assert!(app.list.cmds().iter().any(|cmd| matches!(cmd, DrawCmd::RoundedRectAlpha { rect, alpha, .. } if *rect == hover && *alpha > 0.05)));
    save(&mut app, "conversation-hover");
    app.open_settings("ai");
    assert!(app
        .window_hover_rect(row.left + 15.0, row.top + 10.0)
        .is_none());
    app.close_settings();

    let file = root.join("算法学习计划.md");
    std::fs::write(&file, "# 算法求职学习计划\n\n## 总原则\n\n每天坚持练习。\n\n## 每日节奏\n\n复习、练习和总结。\n\n## 十二周路线图\n\n### 基础巩固\n\n掌握基础数据结构。\n\n### 核心专题\n\n动态规划与搜索。\n").unwrap();
    app.state.view = WorkspaceView::Editor;
    app.settings.set("app.outline.placement", "editor-right");
    app.settings.set("app.outline.visible", "true");
    assert!(app.shell.open_file_with_mode(&file, true));
    app.paint(HWND::default()).unwrap();
    assert!(!app.outline_area.is_empty());
    let body = outline::body(app.outline_area);
    let x = body.left + 24.0;
    let y = body.top + theme::ROW_HEIGHT * 2.0 + 8.0;
    let hover = app.window_hover_rect(x, y).unwrap();
    assert_eq!(
        app.cursor_for(x, y),
        Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND)
    );
    assert_eq!(
        outline::hit(app.outline_area, app.doc.headings().len(), 0, x, y),
        Some(2)
    );
    assert!(app.on_mouse_move(x, y));
    std::thread::sleep(std::time::Duration::from_millis(120));
    app.paint(HWND::default()).unwrap();
    assert!(app.list.cmds().iter().any(|cmd| matches!(cmd, DrawCmd::RoundedRectAlpha { rect, alpha, .. } if *rect == hover && *alpha > 0.05)));
    save(&mut app, "outline-hover");
    app.on_pointer_leave();
    std::thread::sleep(std::time::Duration::from_millis(120));
    app.paint(HWND::default()).unwrap();
    assert!(!app.list.cmds().iter().any(|cmd| matches!(cmd, DrawCmd::RoundedRectAlpha { rect, alpha, .. } if *rect == hover && *alpha > 0.0)));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ai_workspace_navigation_settings_and_agent_sections_preserve_context() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!(
        "mochi-ai-navigation-{}",
        mochi_core::paths::random_base36(12)
    ));
    std::fs::create_dir_all(&root).unwrap();
    for width in [1024, 1440, 1920] {
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join(format!("settings-{width}.json")),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        mochi_core::ai::agent_config::AgentConfigService::new(&root)
            .ensure_seeds()
            .unwrap();
        let snapshot = app.renderer.prepare_snapshot(width, 1000, 96.0).unwrap();
        app.load_chrome_settings();
        // 恢复的旧版设置标签页不能拦截专用工作区的侧栏操作。
        app.shell.open_special(
            TabKind::Settings {
                tab: "general".into(),
                section: "appearance".into(),
            },
            "设置",
        );
        app.state.view = WorkspaceView::MochiAi;
        app.ai.panel.active = Some(AiConversation {
            id: "keep-this-session".into(), title: "界面调整".into(),
            messages: vec![
                AiStoredMessage::new("user", "请整理这份项目文档，并列出下一步计划。"),
                AiStoredMessage::new("assistant", "## 项目进展\n\n已完成需求整理与页面结构调整。\n\n接下来可以依次推进：\n\n1. 核对各项功能的交互。\n2. 检查窄窗口下的阅读体验。\n3. 汇总本轮修改结果。\n\n所有文档都保留在当前工作区，方便继续查阅。"),
            ], ..Default::default()
        });
        app.ai.panel.input.set_text("保留这条未发送的草稿");
        app.sync_state();
        app.paint(HWND::default()).unwrap();
        let save = |app: &mut App, name: &str| {
            if let Some(output) = std::env::var_os("MOCHI_AI_NAV_QA") {
                let output = PathBuf::from(output);
                std::fs::create_dir_all(&output).unwrap();
                app.renderer
                    .save_snapshot(&snapshot, &output.join(format!("{name}-{width}.png")))
                    .unwrap();
            }
        };
        assert!(app
            .nav_layout
            .rect_of(NavHit::Item(NavItem::AgentConfig))
            .is_none());
        assert!(app
            .nav_layout
            .rect_of(NavHit::Item(NavItem::Knowledge))
            .is_none());
        let settings = app.ai.layout.rect_of(assistant::Hit::Settings).unwrap();
        let agents = app.ai.layout.rect_of(assistant::Hit::AgentConfig).unwrap();
        assert!(agents.left > settings.right);
        save(&mut app, "ai");
        click(&mut app, settings);
        assert_eq!(app.state.view, WorkspaceView::MochiAi);
        assert_eq!(app.settings_tab().unwrap().0, "ai");
        save(&mut app, "ai-settings");
        let close = settings::close_rect(app.settings_overlay_rect());
        click(&mut app, close);
        assert_eq!(app.state.view, WorkspaceView::MochiAi);
        click(&mut app, agents);
        assert_eq!(app.state.view, WorkspaceView::AgentConfig);
        save(&mut app, "agents");
        let skills = app
            .agent
            .layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == agent_config::Hit::SectionCard(1))
            .unwrap()
            .0;
        click(&mut app, skills);
        assert_eq!(app.agent.section, 1);
        assert!(!app.agent.data.cards(1).is_empty());
        save(&mut app, "skills");
        // 侧栏可滚动浏览所有定义，固定的返回按钮仍可点击。
        let body = app.agent.nav_layout.body;
        app.on_wheel(body.left + 48.0, body.top + 80.0, -720);
        app.paint(HWND::default()).unwrap();
        assert!(app.agent.nav_scroll > 0.0);
        let skills = app
            .agent
            .nav_layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == agent_config::NavHit::Section(1))
            .unwrap()
            .0;
        app.agent.nav_scroll = (app.agent.nav_scroll + skills.top - body.top - 8.0).max(0.0);
        app.paint(HWND::default()).unwrap();
        // 覆盖反馈中报告的情况：侧栏打开时切换到源码编辑器。
        let source = app.agent.data.cards(0)[0].source_path.clone();
        app.open_agent_source(&source);
        let editor = app.agent.source_editor.as_mut().unwrap();
        let draft = format!("{}\n保留未保存的配置草稿", editor.field.text());
        editor.field.set_text(&draft);
        editor.dirty = true;
        app.paint(HWND::default()).unwrap();
        let skills = app
            .agent
            .nav_layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == agent_config::NavHit::Section(1))
            .unwrap()
            .0;
        click(&mut app, skills);
        assert_eq!(app.agent.section, 1);
        assert!(app.agent.source_editor.is_none());
        app.open_agent_source(&source);
        assert_eq!(
            app.agent.source_editor.as_ref().unwrap().field.text(),
            draft
        );
        assert!(app.agent.source_editor.as_ref().unwrap().dirty);
        app.set_agent_config_section(1);
        let back = app
            .agent
            .nav_layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == agent_config::NavHit::Back)
            .unwrap()
            .0;
        click(&mut app, back);
        assert_eq!(app.state.view, WorkspaceView::MochiAi);
        assert_eq!(
            app.ai.panel.active.as_ref().unwrap().id,
            "keep-this-session"
        );
        assert_eq!(app.ai.panel.input.text(), "保留这条未发送的草稿");
        // 所有设置入口都使用相同的浮层行为。
        for view in [
            WorkspaceView::Home,
            WorkspaceView::Schedule,
            WorkspaceView::Inbox,
            WorkspaceView::Recent,
            WorkspaceView::AgentConfig,
            WorkspaceView::Editor,
        ] {
            app.state.view = view;
            app.open_settings("ai");
            app.paint(HWND::default()).unwrap();
            assert_eq!(app.state.view, view);
            app.close_settings();
            assert_eq!(app.state.view, view);
        }
        app.state.view = WorkspaceView::Home;
        app.paint(HWND::default()).unwrap();
        let header = app.nav_layout.rect_of(NavHit::TypeHeader(0)).unwrap();
        assert!(header.height() >= 36.0);
        click(&mut app, header);
        assert_eq!(app.state.view, WorkspaceView::Editor);
        let library = app
            .nav_layout
            .entries
            .iter()
            .find(|(_, hit)| matches!(hit, NavHit::Library(_)))
            .unwrap()
            .0;
        assert_eq!(
            app.window_hover_rect(library.left + 8.0, library.top + 8.0),
            Some(library)
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}
