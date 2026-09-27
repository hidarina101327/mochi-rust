//! 处理桌面卡片窗口、托盘菜单和卡片页面的操作。
use super::*;
use desktop_window::EventKind;

impl App {
    pub(super) fn desktop_manage_page(&mut self, card_id: &str, page_id: &str) {
        self.desktop_show_main();
        self.open_desktop_manager();
        let area = self.desktop_manager_area();
        if let Some(panel) = self.desktop.panel.as_mut() {
            if let Some(index) = panel.config.cards.iter().position(|c| c.id == card_id) {
                panel.select_card(index);
                if let Some(page) = panel.config.cards[index]
                    .pages
                    .iter()
                    .position(|p| p.id == page_id)
                {
                    panel.select_page(page);
                }
                panel.editor_tab = 1;
                panel.preferences_mode = false;
                panel.reveal_selected_page(area);
            }
        }
    }
    pub fn desktop_show_main(&mut self) {
        if self.hwnd_raw == 0 {
            return;
        }
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            let hwnd = HWND(self.hwnd_raw as *mut _);
            let _ = ShowWindow(
                hwnd,
                if IsIconic(hwnd).as_bool() {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn desktop_tray_action(&mut self, action: u32) {
        match action {
            1 => self.desktop_show_main(),
            2 => {
                self.desktop_show_main();
                self.open_desktop_manager();
            }
            3 => {
                self.desktop.paused = !self.desktop.paused;
                self.desktop_sync_windows();
                self.desktop_refresh();
                self.desktop_timer_state();
            }
            4 => {
                self.desktop.exit_requested = true;
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(self.hwnd_raw as *mut _)),
                        windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
            _ => {}
        }
    }

    pub fn desktop_handle_events(&mut self) -> bool {
        let events = self.desktop.windows.events();
        let changed = !events.is_empty();
        for event in events {
            if self.desktop.saving_editor {
                continue;
            }
            let Some(index) = self
                .desktop
                .config
                .cards
                .iter()
                .position(|c| c.id == event.card)
            else {
                continue;
            };
            match event.kind {
                EventKind::Capsule => {
                    let a = &mut self.desktop.config.cards[index].appearance;
                    a.capsule = !a.capsule;
                    if a.capsule {
                        a.edge_dock = false;
                    }
                    self.desktop_config_changed();
                }
                EventKind::Folder {
                    page,
                    command,
                    paths,
                } => self.desktop_folder_command(index, &page, command, paths),
                EventKind::Refresh => self.desktop_refresh(),
                EventKind::Timer(reset) => self.desktop_timer_action(reset),
                EventKind::AiSend(text) => self.desktop_ai_send(&event.card, text),
                EventKind::AiStop => {
                    self.ai_cancel();
                    self.desktop_sync_windows();
                }
                EventKind::AiSession(id) => {
                    if !self.ai.panel.is_streaming() {
                        if let Some(id) = id {
                            self.ai_open_session(&id);
                        } else {
                            self.ai_new_session();
                        }
                        self.desktop_sync_windows();
                    }
                }
                EventKind::Drop(paths) => {
                    let active = self.desktop.config.cards[index].active_page.clone();
                    if self.desktop.config.cards[index]
                        .active_page()
                        .is_some_and(|p| p.module == Module::Folder)
                    {
                        self.desktop_folder_command(
                            index,
                            &active,
                            desktop_window::folder::Command::Import,
                            paths,
                        );
                        continue;
                    }
                    if let Some(p) = self.desktop.config.cards[index]
                        .pages
                        .iter_mut()
                        .find(|p| p.id == active)
                    {
                        if p.studio.accept_drop || p.module == Module::Shortcuts {
                            p.studio.add_shortcuts(&paths);
                            self.desktop_config_changed();
                        }
                    }
                }
                EventKind::WidgetRow(node, row) => {
                    if let Some(page) = self.desktop.config.cards[index].active_page().cloned() {
                        let key =
                            format!("{}:node:{}", model::page_key(&event.card, &page.id), node);
                        if let Some(row) = self
                            .desktop
                            .snapshot
                            .pages
                            .get(&key)
                            .and_then(|s| s.rows.iter().find(|r| r.id == row))
                            .cloned()
                        {
                            self.desktop_row_action(&page, row, false);
                        }
                    }
                }
                EventKind::Widget(id, trigger) => self.desktop_widget_event(index, &id, trigger),
                EventKind::ShortcutDelete(id) => {
                    if let Some(p) = self.desktop.config.cards[index].active_page_mut() {
                        if p.module == Module::Shortcuts {
                            p.studio.nodes.retain(|n| n.id != id);
                            p.item_styles.remove(&format!("node:{id}"));
                        }
                    }
                    self.desktop_config_changed();
                }
                EventKind::ShortcutMove(id, target) => {
                    if let Some(p) = self.desktop.config.cards[index].active_page_mut() {
                        if p.module == Module::Shortcuts {
                            model::studio::move_shortcut(&mut p.studio, &id, target);
                        }
                    }
                    self.desktop_config_changed();
                }
                EventKind::ItemStyle { page, item, style } => {
                    if let Some(p) = self.desktop.config.cards[index]
                        .pages
                        .iter_mut()
                        .find(|p| p.id == page)
                    {
                        if style == Default::default() {
                            p.item_styles.remove(&item);
                        } else {
                            p.item_styles.insert(item, style);
                        }
                    }
                    self.desktop_config_changed();
                }
                EventKind::TreeOpen(path) => {
                    self.desktop_show_main();
                    self.open_dashboard_document(std::path::Path::new(&path));
                }
                EventKind::Manage | EventKind::ManagePage(_) => {
                    let page = if let EventKind::ManagePage(id) = event.kind {
                        id
                    } else {
                        self.desktop.config.cards[index].active_page.clone()
                    };
                    self.desktop_manage_page(&event.card, &page);
                }
                EventKind::Hide => {
                    self.desktop.config.cards[index].enabled = false;
                    self.desktop_config_changed();
                }
                EventKind::Undock => {}
                EventKind::Pin => {
                    let c = &mut self.desktop.config.cards[index];
                    c.appearance.pinned = !c.appearance.pinned;
                    self.desktop_config_changed();
                }
                EventKind::CalendarExpanded => {
                    let active = self.desktop.config.cards[index].active_page.clone();
                    if let Some(c) = self.desktop.config.cards[index]
                        .pages
                        .iter_mut()
                        .find(|p| p.id == active)
                    {
                        c.presentation.calendar_expanded = !c.presentation.calendar_expanded;
                    }
                    self.desktop_config_changed();
                }
                EventKind::Lock => {
                    let c = &mut self.desktop.config.cards[index];
                    c.locked = !c.locked;
                    self.desktop_config_changed();
                }
                EventKind::Page(id) => {
                    if self.desktop.config.cards[index]
                        .pages
                        .iter()
                        .any(|p| p.id == id)
                    {
                        self.desktop.config.cards[index].active_page = id;
                        let nodes = self.desktop.config.cards[index]
                            .active_page()
                            .map(|p| {
                                p.studio
                                    .nodes
                                    .iter()
                                    .map(|n| n.id.clone())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        for node in nodes {
                            self.desktop_widget_event(
                                index,
                                &node,
                                model::studio::Trigger::PageEnter,
                            );
                        }
                        self.desktop_config_changed();
                        self.desktop_refresh();
                    }
                }
                EventKind::Geometry {
                    x,
                    y,
                    width,
                    height,
                } => {
                    let c = &mut self.desktop.config.cards[index];
                    c.x = x;
                    c.y = y;
                    c.width = width.clamp(model::MIN_CARD_WIDTH, model::MAX_CARD_WIDTH);
                    c.height = height.clamp(model::MIN_CARD_HEIGHT, model::MAX_CARD_HEIGHT);
                    // 调整窗口大小时，不要立即将几何信息写回原生窗口。
                    self.desktop.revision = self.desktop.revision.wrapping_add(1);
                    self.desktop.unsaved = true;
                    self.desktop.save_at = Some(Instant::now() + Duration::from_millis(700));
                    self.desktop_timer_state();
                }
                EventKind::Open => {
                    if let Some(page) = self.desktop.config.cards[index].active_page().cloned() {
                        self.desktop_open_page(&page);
                    }
                }
                EventKind::Row { page, row, toggle } => {
                    let Some(config) = self.desktop.config.cards[index]
                        .active_page()
                        .filter(|p| p.id == page)
                        .cloned()
                    else {
                        continue;
                    };
                    let selected = self
                        .desktop
                        .snapshot
                        .pages
                        .get(&model::page_key(&event.card, &page))
                        .and_then(|p| p.rows.iter().find(|r| r.id == row))
                        .cloned();
                    if let Some(row) = selected {
                        self.desktop_row_action(&config, row, toggle);
                    }
                }
            }
        }
        self.desktop.windows.arrange(HWND(self.hwnd_raw as *mut _));
        changed
    }

    pub(super) fn desktop_row_action(&mut self, page: &model::Page, row: model::Row, toggle: bool) {
        match row.action {
            Some(model::Action::DesktopUtility(action)) => {
                self.desktop_utility_action(page, &action)
            }
            Some(model::Action::OpenFolderEntry(path)) if page.module == Module::Folder => {
                self.desktop_open_folder_target(&page.folder, &path);
            }
            Some(model::Action::ToggleTask { id, checked })
                if toggle && page.interaction == Interaction::Smart =>
            {
                let Some(root) = self.desktop.root.as_deref() else {
                    return;
                };
                // 用户修改与主编辑器的操作顺序保持一致。有限读取和一次
                // 原子替换文件可保留未知字段，遇到错误 JSON 时也不会擅自清空内容。
                let result = model::toggle_schedule_item(root, &id, checked);
                match result {
                    Ok(true) => {
                        // 只有实际写入成功后才乐观更新界面，并使使用此任务的所有页面缓存失效。
                        for snap in self.desktop.snapshot.pages.values_mut() {
                            for item in &mut snap.rows {
                                if matches!(&item.action,Some(model::Action::ToggleTask{id:other,..}) if other==&id)
                                {
                                    item.checked = Some(checked);
                                    item.action = Some(model::Action::ToggleTask {
                                        id: id.clone(),
                                        checked: !checked,
                                    });
                                }
                            }
                        }
                        self.sched.stamp = None;
                        self.desktop_sync_windows();
                        self.desktop_mark_stale();
                    }
                    Ok(false) => {
                        self.desktop_error("任务已被删除，正在更新卡片".into());
                        self.desktop_mark_stale();
                    }
                    Err(e) => self.desktop_error(format!("任务未能保存：{e}")),
                }
            }
            Some(model::Action::ToggleTask { id, .. }) => {
                self.desktop_open_module(Module::Schedule);
                self.agenda_open(&id);
            }
            Some(model::Action::OpenFile(path)) => {
                let Some(root) = &self.desktop.root else {
                    return;
                };
                match model::safe_source_path(root, &path) {
                    Ok(path) => {
                        self.desktop_show_main();
                        if path.is_dir() {
                            self.desktop_open_folder(&path);
                        } else {
                            self.open_dashboard_document(&path);
                        }
                    }
                    Err(e) => self.desktop_error(format!("关联内容不可用：{e}")),
                }
            }
            Some(model::Action::Capture) if page.interaction == Interaction::Smart => {
                self.show_capture()
            }
            Some(model::Action::OpenModule(module)) => {
                match module.as_str() {
                    "settings" => {
                        self.desktop_show_main();
                        self.open_settings("general");
                    }
                    "desktopCards" => {
                        self.desktop_show_main();
                        self.open_desktop_manager();
                    }
                    "marketplace" => {
                        self.desktop_show_main();
                        self.open_marketplace();
                    }
                    "notifications" => {
                        self.desktop_show_main();
                        self.open_notification_center();
                    }
                    _ => {
                        self.desktop_open_module(Module::from_wire(&module).unwrap_or(page.module))
                    }
                }
                if page.module == Module::Ai {
                    if let Some(id) = row.id.strip_prefix("ai:") {
                        self.ai_open_session(id);
                    }
                }
            }
            _ => self.desktop_open_page(page),
        }
    }

    pub(in crate::app) fn desktop_open_page(&mut self, page: &model::Page) {
        if page.module == Module::Folder {
            match model::folder::current_config(&page.folder) {
                Ok(current) => {
                    self.desktop_open_folder_target(&page.folder, &current.path);
                }
                Err(error) => self.desktop_error(format!("无法打开映射文件夹：{error:#}")),
            }
            return;
        }
        if let Some(source) = &page.source {
            if let Some(root) = &self.desktop.root {
                if let Ok(path) = model::safe_source_path(root, source) {
                    if path.is_file() {
                        self.desktop_show_main();
                        self.open_dashboard_document(&path);
                        return;
                    } else if path.is_dir() {
                        self.desktop_show_main();
                        self.desktop_open_folder(&path);
                        return;
                    }
                }
            }
        }
        self.desktop_open_module(page.module);
    }

    pub(in crate::app) fn desktop_open_module(&mut self, module: Module) {
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        self.desktop_show_main();
        self.focus = Focus::Main;
        match module {
            Module::Custom
            | Module::Clock
            | Module::Shortcuts
            | Module::Folder
            | Module::Weather
            | Module::Music
            | Module::Search => self.open_desktop_manager(),
            Module::Home => {
                self.state.view = WorkspaceView::Home;
                self.reload_home_dashboard();
            }
            Module::Schedule => {
                self.state.view = WorkspaceView::Schedule;
                self.sched.view.query.clear();
                self.reload_schedule();
            }
            Module::Inbox => self.state.view = WorkspaceView::Inbox,
            Module::QuickNote => {
                if self.ensure_quick_note_tab() {
                    self.state.view = WorkspaceView::QuickNote;
                }
            }
            Module::Recent => self.state.view = WorkspaceView::Recent,
            Module::Favorites => self.open_favorites(),
            Module::Ai => self.state.view = WorkspaceView::MochiAi,
            Module::Templates => self.state.view = WorkspaceView::Templates,
            Module::Automations => self.workflows_open(),
            Module::AgentConfig => {
                self.state.view = WorkspaceView::AgentConfig;
                self.agent.section = 0;
            }
            Module::QuickNav | Module::English => {
                self.plugin_workspace = Some(
                    if module == Module::QuickNav {
                        "quick-navigation"
                    } else {
                        "english-lab"
                    }
                    .into(),
                );
                self.state.view = WorkspaceView::Plugin;
            }
            Module::Pomodoro => {
                self.state.ai_panel_open = true;
                self.state.right_panel = RightPanel::Pomodoro;
            }
            Module::Knowledge | Module::Document | Module::Base | Module::Canvas | Module::Exam => {
                self.open_knowledge()
            }
        }
        self.sync_state();
        self.invalidate_main();
    }

    pub(super) fn desktop_open_folder(&mut self, path: &Path) {
        self.open_knowledge();
        let index = self.shell.workspace().and_then(|ws| {
            ws.libraries
                .iter()
                .enumerate()
                .filter(|(_, lib)| {
                    let library = Path::new(&lib.path);
                    let library = if library.is_absolute() {
                        library.to_path_buf()
                    } else {
                        ws.root.join(library)
                    };
                    library
                        .canonicalize()
                        .ok()
                        .is_some_and(|root| mochi_core::paths::path_is_within(&root, path))
                })
                .max_by_key(|(_, lib)| lib.path.len())
                .map(|(i, _)| i)
        });
        if let Some(index) = index {
            self.shell.open_library_landing(index);
        }
        self.sync_state();
        self.invalidate_main();
    }
}
