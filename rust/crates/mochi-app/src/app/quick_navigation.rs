//! 绘制快速导航面板，并处理条目选择。
use super::*;

impl App {
    /// Electron `quick-navigation/workspace.js` 的原生视图。所有修改都交给 core
    /// 服务写入 Electron 兼容的 `.mochi/extensions-data/quick-navigation/default/data.json`。
    pub(super) fn paint_quick_navigation(&mut self, area: Rect, p: &Palette) {
        self.quick_nav_hits.clear();
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            crate::view::placeholder(&mut self.list, area, "请先打开工作区", p);
            return;
        };
        let service = mochi_core::quick_navigation::Service::new(root);
        let Ok(model) = service.load() else {
            crate::view::placeholder(&mut self.list, area, "快捷导航数据读取失败", p);
            return;
        };
        self.list.text(
            Rect::new(
                area.left + 28.0,
                area.top + 22.0,
                area.right - 300.0,
                area.top + 54.0,
            ),
            "快捷导航",
            TextStyle::Large,
            p.foreground,
        );
        self.list.text(
            Rect::new(
                area.left + 28.0,
                area.top + 58.0,
                area.right - 300.0,
                area.top + 80.0,
            ),
            "在当前工作区管理网页、应用、文件夹与本地文件。",
            TextStyle::Caption,
            p.muted,
        );
        let add_group = Rect::new(
            area.right - 274.0,
            area.top + 26.0,
            area.right - 154.0,
            area.top + 58.0,
        );
        let add_item = Rect::new(
            area.right - 142.0,
            area.top + 26.0,
            area.right - 28.0,
            area.top + 58.0,
        );
        let import_item = Rect::new(
            area.right - 404.0,
            area.top + 26.0,
            area.right - 286.0,
            area.top + 58.0,
        );
        for (rect, label, accent, hit) in [
            (add_group, "＋ 分组", false, QuickNavHit::AddGroup),
            (import_item, "导入文件", false, QuickNavHit::ImportItem),
            (add_item, "＋ 快捷方式", true, QuickNavHit::AddItem),
        ] {
            self.list.rounded_rect(
                rect,
                6.0,
                if accent { p.accent } else { p.surface_elevated },
            );
            self.list.text(
                rect,
                label,
                TextStyle::Caption,
                if accent {
                    p.accent_foreground
                } else {
                    p.foreground
                },
            );
            self.quick_nav_hits.push((rect, hit));
        }
        if let Some(group) = model
            .groups
            .iter()
            .find(|group| group.id == model.ui.selected)
        {
            let rename = Rect::new(
                area.right - 548.0,
                area.top + 62.0,
                area.right - 422.0,
                area.top + 88.0,
            );
            let root = Rect::new(
                area.right - 410.0,
                area.top + 62.0,
                area.right - 304.0,
                area.top + 88.0,
            );
            let delete = Rect::new(
                area.right - 294.0,
                area.top + 62.0,
                area.right - 154.0,
                area.top + 88.0,
            );
            for (rect, label, hit) in [
                (
                    rename,
                    "重命名分组",
                    QuickNavHit::RenameGroup(group.id.clone()),
                ),
                (
                    root,
                    "移至根级",
                    QuickNavHit::MoveGroupToRoot(group.id.clone()),
                ),
                (
                    delete,
                    "删除分组",
                    QuickNavHit::DeleteGroup(group.id.clone()),
                ),
            ] {
                self.list.rounded_rect(rect, 6.0, p.surface_elevated);
                self.list
                    .text(rect, label, TextStyle::Caption, p.foreground);
                self.quick_nav_hits.push((rect, hit));
            }
        }
        let left = Rect::new(
            area.left + 20.0,
            area.top + 96.0,
            area.left + 220.0,
            area.bottom - 20.0,
        );
        self.list.rounded_rect(left, 8.0, p.surface);
        let mut y = left.top + 14.0;
        let count = |selection: &str| -> usize {
            match selection {
                "all" => model.items.len(),
                "favorites" => model.items.iter().filter(|item| item.favorite).count(),
                "recent" => model
                    .items
                    .iter()
                    .filter(|item| item.open_count > 0)
                    .count(),
                id => {
                    let mut ids = vec![id.to_owned()];
                    let mut cursor = 0;
                    while cursor < ids.len() {
                        let parent = ids[cursor].clone();
                        ids.extend(
                            model
                                .groups
                                .iter()
                                .filter(|group| group.parent_id.as_deref() == Some(&parent))
                                .map(|group| group.id.clone()),
                        );
                        cursor += 1;
                    }
                    model
                        .items
                        .iter()
                        .filter(|item| item.group_id.as_ref().is_some_and(|id| ids.contains(id)))
                        .count()
                }
            }
        };
        let mut rows = vec![
            ("all".to_owned(), "▦  全部".to_owned()),
            ("favorites".to_owned(), "★  收藏夹".to_owned()),
            ("recent".to_owned(), "◷  最近使用".to_owned()),
        ];
        let mut groups = model.groups.clone();
        groups.sort_by_key(|group| (group.parent_id.clone(), group.order));
        rows.extend(groups.into_iter().map(|group| {
            let depth = group_path_depth(&model.groups, &group.id);
            (group.id, format!("{}▸  {}", "  ".repeat(depth), group.name))
        }));
        rows.push(("settings".to_owned(), "⚙  设置".to_owned()));
        for (id, label) in rows {
            let rect = Rect::new(left.left + 8.0, y, left.right - 8.0, y + 30.0);
            let is_group = model.groups.iter().any(|group| group.id == id);
            let manual_group = is_group && model.settings.default_sort == "manual";
            if model.ui.selected == id {
                self.list.rounded_rect(rect, 5.0, p.accent_hover);
            }
            self.list.text(
                Rect::new(
                    rect.left + 8.0,
                    rect.top + 5.0,
                    rect.right - if manual_group { 72.0 } else { 42.0 },
                    rect.bottom - 4.0,
                ),
                label,
                TextStyle::Caption,
                if model.ui.selected == id {
                    p.accent_foreground
                } else {
                    p.foreground
                },
            );
            if id != "settings" {
                self.list.text(
                    Rect::new(
                        rect.right - if manual_group { 98.0 } else { 32.0 },
                        rect.top + 5.0,
                        rect.right - if manual_group { 72.0 } else { 5.0 },
                        rect.bottom - 4.0,
                    ),
                    count(&id).to_string(),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            if manual_group {
                let up = Rect::new(
                    rect.right - 62.0,
                    rect.top + 5.0,
                    rect.right - 38.0,
                    rect.bottom - 4.0,
                );
                let down = Rect::new(
                    rect.right - 34.0,
                    rect.top + 5.0,
                    rect.right - 10.0,
                    rect.bottom - 4.0,
                );
                self.list.text(up, "↑", TextStyle::Caption, p.muted);
                self.list.text(down, "↓", TextStyle::Caption, p.muted);
                self.quick_nav_hits
                    .push((up, QuickNavHit::MoveGroup(id.clone(), -1)));
                self.quick_nav_hits
                    .push((down, QuickNavHit::MoveGroup(id.clone(), 1)));
            }
            self.quick_nav_hits.push((rect, QuickNavHit::Select(id)));
            y += 32.0;
            if y + 30.0 > left.bottom {
                break;
            }
        }
        let content = Rect::new(
            left.right + 18.0,
            area.top + 96.0,
            area.right - 22.0,
            area.bottom - 20.0,
        );
        if model.ui.selected == "settings" {
            self.paint_quick_navigation_settings(content, &model, p);
            return;
        }
        let visible = quick_nav_visible_items(&model);
        if visible.is_empty() {
            self.list.rounded_rect(content, 8.0, p.surface);
            self.list.text(
                Rect::new(
                    content.left + 24.0,
                    content.top + 30.0,
                    content.right - 24.0,
                    content.top + 58.0,
                ),
                "这里还没有快捷方式",
                TextStyle::Label,
                p.foreground,
            );
            self.list.text(
                Rect::new(
                    content.left + 24.0,
                    content.top + 62.0,
                    content.right - 24.0,
                    content.top + 84.0,
                ),
                "点击右上角“＋ 快捷方式”添加网页、程序或文件夹。",
                TextStyle::Caption,
                p.muted,
            );
            return;
        }
        let list_layout = model.settings.appearance.layout == "list";
        let configured_columns = usize::from(model.settings.grid.columns.clamp(2, 10));
        let columns = if list_layout { 1 } else { configured_columns };
        let tile_width = if list_layout {
            content.width()
        } else {
            content.width() / columns as f32 - 10.0
        };
        for (index, item) in visible.iter().enumerate() {
            let col = index % columns;
            let row = index / columns;
            let x = content.left + col as f32 * (tile_width + 10.0);
            let configured_rows = model.settings.grid.rows.clamp(2, 8) as f32;
            let tile_height = if list_layout {
                50.0
            } else {
                ((content.height() - (configured_rows - 1.0) * 10.0) / configured_rows)
                    .clamp(76.0, 132.0)
            };
            let row_height = if list_layout {
                58.0
            } else {
                tile_height + 10.0
            };
            let y = content.top + row as f32 * row_height;
            if y + tile_height > content.bottom {
                break;
            }
            let tile = Rect::new(x, y, x + tile_width, y + tile_height);
            self.list.rounded_rect(
                tile,
                8.0,
                item.background_color
                    .as_deref()
                    .and_then(quick_nav_color)
                    .unwrap_or(p.surface),
            );
            let text_left = if model.settings.appearance.fields.icon {
                let icon = Rect::new(x + 14.0, y + 10.0, x + 42.0, y + 38.0);
                self.list.rounded_rect(icon, 6.0, p.surface_elevated);
                self.list
                    .text(icon, quick_nav_icon(item), TextStyle::Caption, p.accent);
                x + 52.0
            } else {
                x + 14.0
            };
            if model.settings.appearance.fields.name {
                self.list.text(
                    Rect::new(text_left, y + 8.0, x + tile_width - 120.0, y + 30.0),
                    &item.name,
                    TextStyle::Label,
                    p.foreground,
                );
            }
            if model.settings.appearance.fields.target {
                self.list.text(
                    Rect::new(
                        text_left,
                        y + if list_layout { 29.0 } else { 38.0 },
                        x + tile_width - 120.0,
                        y + if list_layout { 48.0 } else { 57.0 },
                    ),
                    &item.target,
                    TextStyle::Caption,
                    p.muted,
                );
            }
            if !list_layout && model.settings.appearance.fields.note && !item.note.is_empty() {
                self.list.text(
                    Rect::new(text_left, y + 60.0, x + tile_width - 14.0, y + 78.0),
                    &item.note,
                    TextStyle::Caption,
                    p.muted,
                );
            }
            let action_y = if list_layout { y + 15.0 } else { y + 70.0 };
            let edit = Rect::new(x + 14.0, action_y, x + 52.0, action_y + 21.0);
            let selected_group = model
                .groups
                .iter()
                .any(|group| group.id == model.ui.selected)
                .then(|| model.ui.selected.clone());
            let move_to_selected = selected_group
                .as_deref()
                .filter(|group| item.group_id.as_deref() != Some(*group))
                .map(|_| Rect::new(x + 56.0, action_y, x + 112.0, action_y + 21.0));
            let open = Rect::new(
                x + tile_width - 212.0,
                action_y,
                x + tile_width - 168.0,
                action_y + 21.0,
            );
            let run_as_admin = (item.kind == "application").then(|| {
                Rect::new(
                    x + tile_width - 272.0,
                    action_y,
                    x + tile_width - 218.0,
                    action_y + 21.0,
                )
            });
            let locate = (!item.source_path.trim().is_empty()).then(|| {
                Rect::new(
                    x + tile_width - 332.0,
                    action_y,
                    x + tile_width - 278.0,
                    action_y + 21.0,
                )
            });
            let up = Rect::new(
                x + tile_width - 160.0,
                action_y,
                x + tile_width - 136.0,
                action_y + 21.0,
            );
            let down = Rect::new(
                x + tile_width - 132.0,
                action_y,
                x + tile_width - 108.0,
                action_y + 21.0,
            );
            let favorite = Rect::new(
                x + tile_width - 62.0,
                action_y,
                x + tile_width - 38.0,
                action_y + 21.0,
            );
            let remove = Rect::new(
                x + tile_width - 27.0,
                action_y,
                x + tile_width - 7.0,
                action_y + 21.0,
            );
            self.list.text(edit, "编辑", TextStyle::Caption, p.muted);
            self.quick_nav_hits
                .push((edit, QuickNavHit::EditItem(item.id.clone())));
            if let Some(move_to_selected) = move_to_selected {
                self.list
                    .text(move_to_selected, "移至此组", TextStyle::Caption, p.muted);
                self.quick_nav_hits.push((
                    move_to_selected,
                    QuickNavHit::MoveItemToSelectedGroup(item.id.clone()),
                ));
            }
            self.list.text(open, "打开", TextStyle::Caption, p.accent);
            if let Some(run_as_admin) = run_as_admin {
                self.list
                    .text(run_as_admin, "管理员", TextStyle::Caption, p.muted);
                self.quick_nav_hits
                    .push((run_as_admin, QuickNavHit::RunAsAdmin(item.id.clone())));
            }
            if let Some(locate) = locate {
                self.list.text(locate, "定位", TextStyle::Caption, p.muted);
                self.quick_nav_hits
                    .push((locate, QuickNavHit::ShowInFolder(item.id.clone())));
            }
            if model.settings.default_sort == "manual" {
                self.list.text(up, "↑", TextStyle::Caption, p.muted);
                self.list.text(down, "↓", TextStyle::Caption, p.muted);
                self.quick_nav_hits
                    .push((up, QuickNavHit::MoveItem(item.id.clone(), -1)));
                self.quick_nav_hits
                    .push((down, QuickNavHit::MoveItem(item.id.clone(), 1)));
            }
            self.list.text(
                favorite,
                if item.favorite { "★" } else { "☆" },
                TextStyle::Body,
                if item.favorite { p.accent } else { p.muted },
            );
            self.list.text(remove, "×", TextStyle::Body, p.muted);
            self.quick_nav_hits
                .push((open, QuickNavHit::Open(item.id.clone())));
            self.quick_nav_hits
                .push((favorite, QuickNavHit::ToggleFavorite(item.id.clone())));
            self.quick_nav_hits
                .push((remove, QuickNavHit::Remove(item.id.clone())));
        }
    }

    pub(super) fn on_quick_navigation_click(&mut self, x: f32, y: f32) {
        let hit = self
            .quick_nav_hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, hit)| hit.clone());
        let Some(hit) = hit else {
            return;
        };
        match hit {
            QuickNavHit::AddItem => self.open_quick_nav_dialog(false),
            QuickNavHit::AddGroup => self.open_quick_nav_dialog(true),
            QuickNavHit::ImportItem => {
                if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                    if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                        let source_path = path.to_string_lossy().to_string();
                        let target = if path
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("url"))
                        {
                            std::fs::read_to_string(&path)
                                .ok()
                                .and_then(|contents| {
                                    contents.lines().find_map(|line| {
                                        line.trim().strip_prefix("URL=").map(str::to_owned)
                                    })
                                })
                                .unwrap_or_else(|| source_path.clone())
                        } else {
                            source_path.clone()
                        };
                        let service = mochi_core::quick_navigation::Service::new(root);
                        let result = service.load().and_then(|mut model| {
                            let group_id = model
                                .groups
                                .iter()
                                .any(|group| group.id == model.ui.selected)
                                .then(|| model.ui.selected.clone());
                            let name = path
                                .file_stem()
                                .and_then(|name| name.to_str())
                                .unwrap_or("快捷方式")
                                .to_owned();
                            let kind = if target.starts_with("http://")
                                || target.starts_with("https://")
                            {
                                "web"
                            } else {
                                "file"
                            };
                            service.upsert_item(
                                &mut model,
                                mochi_core::quick_navigation::Item {
                                    id: String::new(),
                                    name,
                                    target,
                                    kind: kind.into(),
                                    group_id,
                                    note: String::new(),
                                    arguments: Vec::new(),
                                    working_directory: String::new(),
                                    source_path,
                                    favorite: false,
                                    order: 0,
                                    open_count: 0,
                                    last_opened_at: None,
                                    created_at: String::new(),
                                    updated_at: String::new(),
                                    background_color: None,
                                    custom_icon: None,
                                },
                            )
                        });
                        match result {
                            Ok(_) => self.show_global_notice("快捷方式已导入"),
                            Err(error) => {
                                self.show_global_notice(format!("导入快捷方式失败：{error}"))
                            }
                        }
                    }
                    self.invalidate_main();
                }
            }
            QuickNavHit::EditItem(id) => self.open_quick_nav_edit_dialog(&id),
            QuickNavHit::RenameGroup(id) => self.open_quick_nav_rename_group_dialog(&id),
            QuickNavHit::DeleteGroup(id) => self.open_quick_nav_delete_group_dialog(&id),
            QuickNavHit::ConfigureBrowser => self.open_quick_nav_browser_dialog(),
            QuickNavHit::MoveGroupToRoot(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service
                        .load()
                        .and_then(|mut model| service.move_group(&mut model, &id, None))
                    {
                        self.show_global_notice(format!("调整分组层级失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::MoveItemToSelectedGroup(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service.load().and_then(|mut model| {
                        let destination = model
                            .groups
                            .iter()
                            .any(|group| group.id == model.ui.selected)
                            .then(|| model.ui.selected.clone());
                        service.move_item(&mut model, &id, destination.as_deref())
                    }) {
                        self.show_global_notice(format!("移动快捷方式失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::Select(selected) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Ok(mut model) = service.load() {
                        model.ui.selected = selected;
                        if let Err(error) = service.save(&model) {
                            self.show_global_notice(format!("保存快捷导航状态失败：{error}"));
                        }
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::Open(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    match service.load().and_then(|mut model| {
                        let item = model
                            .items
                            .iter()
                            .find(|item| item.id == id)
                            .ok_or_else(|| anyhow::anyhow!("快捷方式不存在"))?;
                        let is_web = item.target.starts_with("http://")
                            || item.target.starts_with("https://");
                        let opened = if is_web
                            && model.settings.browser.mode == "custom"
                            && !model.settings.browser.executable_path.trim().is_empty()
                        {
                            platform::open_with_application(
                                &model.settings.browser.executable_path,
                                &item.target,
                                &item.arguments,
                                &item.working_directory,
                            )
                        } else {
                            platform::launch_shortcut(
                                &item.target,
                                &item.arguments,
                                &item.working_directory,
                            )
                        };
                        if !opened {
                            anyhow::bail!("系统未能打开目标")
                        }
                        service.record_open(&mut model, &id)?;
                        Ok(())
                    }) {
                        Ok(()) => self.show_global_notice("已打开快捷方式"),
                        Err(error) => self.show_global_notice(format!("打开失败：{error}")),
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::RunAsAdmin(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    match service.load().and_then(|mut model| {
                        let item = model
                            .items
                            .iter()
                            .find(|item| item.id == id)
                            .ok_or_else(|| anyhow::anyhow!("快捷方式不存在"))?;
                        if item.kind != "application" {
                            anyhow::bail!("只有应用快捷方式可使用管理员运行")
                        }
                        if !platform::launch_shortcut_as_admin(
                            &item.target,
                            &item.arguments,
                            &item.working_directory,
                        ) {
                            anyhow::bail!("系统拒绝或未能以管理员身份启动目标")
                        }
                        service.record_open(&mut model, &id)?;
                        Ok(())
                    }) {
                        Ok(()) => self.show_global_notice("已请求管理员权限启动"),
                        Err(error) => self.show_global_notice(format!("管理员启动失败：{error}")),
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::ShowInFolder(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    match service.load().and_then(|model| {
                        let item = model
                            .items
                            .iter()
                            .find(|item| item.id == id)
                            .ok_or_else(|| anyhow::anyhow!("快捷方式不存在"))?;
                        if item.source_path.trim().is_empty()
                            || !platform::show_item_in_folder(&item.source_path)
                        {
                            anyhow::bail!("无法在资源管理器中定位源文件")
                        }
                        Ok(())
                    }) {
                        Ok(()) => {}
                        Err(error) => self.show_global_notice(format!("定位失败：{error}")),
                    }
                }
            }
            QuickNavHit::ToggleFavorite(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    let result = service
                        .load()
                        .and_then(|mut model| service.toggle_favorite(&mut model, &id));
                    if let Err(error) = result {
                        self.show_global_notice(format!("快捷导航修改失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::Remove(id) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service
                        .load()
                        .and_then(|mut model| service.remove_item(&mut model, &id))
                    {
                        self.show_global_notice(format!("移除快捷方式失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::MoveItem(id, direction) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service
                        .load()
                        .and_then(|mut model| service.reorder_item(&mut model, &id, direction))
                    {
                        self.show_global_notice(format!("调整快捷方式顺序失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::MoveGroup(id, direction) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service
                        .load()
                        .and_then(|mut model| service.reorder_group(&mut model, &id, direction))
                    {
                        self.show_global_notice(format!("调整分组顺序失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::SetSort(sort) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service.load().and_then(|mut model| {
                        model.settings.default_sort = sort;
                        service.save(&model)
                    }) {
                        self.show_global_notice(format!("保存快捷导航设置失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::SetLayout(layout) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service.load().and_then(|mut model| {
                        model.settings.appearance.layout = layout;
                        service.save(&model)
                    }) {
                        self.show_global_notice(format!("保存快捷导航设置失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::SetGridColumns(delta) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service.load().and_then(|mut model| {
                        model.settings.grid.columns = (i16::from(model.settings.grid.columns)
                            + i16::from(delta))
                        .clamp(2, 10) as u8;
                        service.save(&model)
                    }) {
                        self.show_global_notice(format!("保存快捷导航网格设置失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::SetGridRows(delta) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service.load().and_then(|mut model| {
                        model.settings.grid.rows = (i16::from(model.settings.grid.rows)
                            + i16::from(delta))
                        .clamp(2, 8) as u8;
                        service.save(&model)
                    }) {
                        self.show_global_notice(format!("保存快捷导航网格设置失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
            QuickNavHit::ToggleField(field) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::quick_navigation::Service::new(root);
                    if let Err(error) = service.load().and_then(|mut model| {
                        match field.as_str() {
                            "icon" => {
                                model.settings.appearance.fields.icon =
                                    !model.settings.appearance.fields.icon
                            }
                            "name" => {
                                model.settings.appearance.fields.name =
                                    !model.settings.appearance.fields.name
                            }
                            "target" => {
                                model.settings.appearance.fields.target =
                                    !model.settings.appearance.fields.target
                            }
                            "note" => {
                                model.settings.appearance.fields.note =
                                    !model.settings.appearance.fields.note
                            }
                            _ => return Ok(()),
                        }
                        service.save(&model)
                    }) {
                        self.show_global_notice(format!("保存快捷导航设置失败：{error}"));
                    }
                }
                self.invalidate_main();
            }
        }
    }
}
