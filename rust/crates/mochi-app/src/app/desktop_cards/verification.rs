//! 生成桌面卡片功能验证所需的页面快照。
use super::*;

impl App {
    pub fn desktop_snapshot(scenario: &str, output: &Path) -> Result<()> {
        let root =
            std::env::temp_dir().join(format!("mochi-desktop-visual-{}", std::process::id()));
        std::fs::create_dir_all(&root).map_err(|e| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
        })?;
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))?;
        app.state.dark = scenario.contains("dark");
        let width = if scenario.contains("narrow") {
            760.0
        } else {
            1200.0
        };
        let height = if scenario.contains("narrow") {
            600.0
        } else {
            800.0
        };
        let target =
            app.renderer
                .prepare_snapshot((width * 1.5) as u32, (height * 1.5) as u32, 144.0)?;
        let mut config = DesktopConfig::default();
        if !scenario.contains("empty") {
            let mut card = model::Card::new("今日工作台", Module::Schedule);
            card.pages.push(model::Page::new(Module::Inbox));
            card.pages.push(model::Page::new(Module::Pomodoro));
            config.cards.push(card);
            config
                .cards
                .push(model::Card::new("阅读与收藏", Module::Favorites));
            config
                .cards
                .push(model::Card::new("桌面时钟", Module::Clock));
        }
        app.desktop.config = config.clone();
        app.desktop.loaded = true;
        app.desktop.root = Some(root.clone());
        app.desktop.panel = Some(ui::State::workspace(&config));
        app.state.view = WorkspaceView::DesktopCards;
        if scenario.contains("templates") {
            app.desktop.panel.as_mut().unwrap().creating = Some(true);
        }
        if scenario.contains("appearance") {
            app.desktop.panel.as_mut().unwrap().editor_tab = 0;
        }
        if scenario.contains("folder") {
            let mut card = model::Card::new("项目资料", Module::Folder);
            card.pages[0].folder.path = "D:\\资料\\项目文档".into();
            app.desktop.panel = Some(ui::State::workspace(&DesktopConfig {
                cards: vec![card],
                ..Default::default()
            }));
            let panel = app.desktop.panel.as_mut().unwrap();
            panel.editor_tab = 1;
            panel.preferences_mode = scenario.contains("preferences");
        }
        for (key, module) in [
            ("weather", Module::Weather),
            ("music", Module::Music),
            ("search", Module::Search),
        ] {
            if scenario.contains(key) {
                let mut card = model::Card::new(module.label(), module);
                card.pages[0].utility.location = "深圳".into();
                card.pages[0].utility.query = "项目计划".into();
                app.desktop.panel = Some(ui::State::workspace(&DesktopConfig {
                    cards: vec![card],
                    ..Default::default()
                }));
                app.desktop.panel.as_mut().unwrap().editor_tab = 1;
            }
        }
        if scenario.contains("source-picker") {
            let io = |e: std::io::Error| {
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
            };
            std::fs::create_dir_all(root.join("知识库/逆向/资料")).map_err(io)?;
            std::fs::create_dir_all(root.join("Agent配置/Agents")).map_err(io)?;
            std::fs::write(root.join("知识库/逆向/逆向工程概述.mc"), "{}").map_err(io)?;
            std::fs::write(root.join("Agent配置/Agents/隐藏配置.md"), "{}").map_err(io)?;
            let mut picker = crate::ui::object_picker::State::new(
                "选择知识库、文件夹或文件（可多选）",
                "搜索名称或路径，多选后点击确定",
                model::collection::source_candidates(&root),
                Vec::new(),
            );
            picker.set_documents_only(true);
            picker.query.set_text("逆向");
            let mut list = DrawList::new();
            let area = app.renderer.viewport();
            crate::ui::object_picker::paint(
                &mut picker,
                &mut list,
                area,
                crate::ui::theme::tokens().palette(app.state.dark),
            );
            app.renderer.present(HWND::default(), 0xf5f6f7, &list)?;
            if let Some(parent) = output.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            return app.renderer.save_snapshot(&target, output);
        }
        app.paint(HWND::default())?;
        if let Some(parent) = output.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        app.renderer.save_snapshot(&target, output)?;
        println!("snapshot {}", output.display());
        Ok(())
    }
}
