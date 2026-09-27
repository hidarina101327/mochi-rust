//! 处理设置页面导航、字段提交和设置变更后的副作用。
use super::*;

impl App {
    pub(super) fn on_settings_nav_click(&mut self, x: f32, y: f32) {
        self.commit_settings_field();
        let Some(hit) = self.prefs.nav_layout.hit(x, y) else {
            return;
        };
        if hit == settings::NavHit::Scrollbar {
            let Some(bar) = self.prefs.nav_layout.scrollbar else {
                return;
            };
            if bar.thumb.contains(x, y) {
                self.drag = Some(Drag {
                    target: DragTarget::SettingsNav,
                    grab_offset: y - bar.thumb.top,
                });
            } else {
                let travel = (bar.track.height() - bar.thumb.height()).max(1.0);
                let ratio =
                    ((y - bar.track.top - bar.thumb.height() / 2.0) / travel).clamp(0.0, 1.0);
                self.prefs.nav_scroll = bar.max * ratio;
                self.invalidate_main();
            }
            return;
        }
        let Some((current_tab, current_section)) = self.settings_overlay.clone() else {
            return;
        };
        let (tab, section, scroll) = match hit {
            settings::NavHit::Scrollbar => unreachable!("scrollbar is handled above"),
            settings::NavHit::Tab(i) => {
                let tab = settings::TABS[i].0.to_owned();
                let section = if tab == "customization"
                    && settings::is_customization_section(&current_section)
                {
                    current_section
                } else {
                    "appearance".to_owned()
                };
                let scroll = if tab == "customization" && current_tab == "customization" {
                    self.prefs
                        .content_layout
                        .section_scroll(&section)
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                (tab, section, scroll)
            }
            settings::NavHit::Section(i) => {
                let section = settings::CUSTOMIZATION_SECTIONS[i].0.to_owned();
                let scroll = self
                    .prefs
                    .content_layout
                    .section_scroll(&section)
                    .unwrap_or(0.0);
                ("customization".to_owned(), section, scroll)
            }
        };
        self.settings_overlay = Some((tab, section));
        self.prefs.scroll = scroll.clamp(0.0, self.prefs.content_layout.max_scroll());
        self.invalidate_main();
    }

    pub(super) fn on_settings_content_click(&mut self, x: f32, y: f32) {
        if self.prefs.content_layout.search_rect.contains(x, y) {
            self.commit_settings_field();
            let rect = self.prefs.content_layout.search_rect;
            self.prefs.search.click(x - rect.left - 12.0, shift_down());
            self.prefs.scroll = 0.0;
            self.focus = Focus::SettingsSearch;
            return;
        }
        if self.settings_tab().is_some_and(|(tab, _)| tab == "general")
            && self.prefs.content_layout.update_check_rect.contains(x, y)
        {
            self.start_update_check(true);
            return;
        }
        if self.prefs.content_layout.body.contains(x, y) {
            if let Some(action) = self.prefs.content_layout.navigation.hit(x, y) {
                self.commit_settings_field();
                self.edit_navigation_preferences(action);
                return;
            }
        }
        if self
            .settings_tab()
            .is_some_and(|(tab, _)| tab == "customization")
        {
            if let Some((index, _)) = self
                .prefs
                .content_layout
                .background_buttons
                .iter()
                .enumerate()
                .find(|(_, (r, _))| r.contains(x, y))
            {
                if index == 0 {
                    if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                        let save = (|| -> anyhow::Result<()> {
                            let meta = std::fs::metadata(&path)?;
                            if meta.len() > 16 * 1024 * 1024 {
                                anyhow::bail!("背景图不能超过 16 MB")
                            };
                            let size = crate::ui::imginfo::dimensions(&path).ok_or_else(|| {
                                anyhow::anyhow!("请选择 PNG、JPEG、GIF、BMP 或 WebP 图片")
                            })?;
                            if u64::from(size.0) * u64::from(size.1) > 32_000_000 {
                                anyhow::bail!("背景图像素超过限制")
                            }
                            let data = mochi_core::settings::background_image_data_url(&path)?;
                            self.settings.set("background.imagePath", &data);
                            self.settings.set("app.background.enabled", "true");
                            Ok(())
                        })();
                        if let Err(error) = save {
                            self.state.status_text = error.to_string();
                        }
                    }
                } else {
                    self.settings.remove("background.imagePath");
                    self.settings.set("app.background.enabled", "false");
                }
                let _ = self.settings.flush();
                self.apply_setting_side_effects("background.enabled");
                return;
            }
        }
        if self
            .settings_tab()
            .is_some_and(|(tab, _)| tab == "shortcuts")
            && self.prefs.content_layout.body.contains(x, y)
        {
            if let Some((_, i)) = self
                .prefs
                .content_layout
                .shortcuts
                .iter()
                .find(|(r, _)| r.contains(x, y))
            {
                let (key, description, _) = settings::GLOBAL_SHORTCUTS[*i];
                let mut field = TextField::new(key);
                field.set_text(&crate::ui::shortcuts::binding(key));
                field.select_all();
                self.dialog = Some(Dialog {
                    title: format!("修改快捷键 · {description}"),
                    description: "输入组合键文本，例如 Ctrl+Shift+P 或 Alt+Q；留空恢复默认。"
                        .into(),
                    field: Some(field),
                    error: String::new(),
                    note: None,
                    buttons: vec![
                        DialogButton {
                            label: "取消".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::Dismiss,
                        },
                        DialogButton {
                            label: "保存".into(),
                            kind: ButtonKind::Primary,
                            action: DialogAction::SetShortcut(key),
                        },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
                return;
            }
        }
        if self.settings_tab().is_some_and(|(tab, _)| tab == "plugins") {
            let area = if self.settings_overlay.is_some() {
                let modal = self.settings_overlay_rect();
                let nav_right = modal.left + 224.0_f32.min(modal.width() * 0.34);
                Rect::new(nav_right + 1.0, modal.top, modal.right, modal.bottom)
            } else {
                self.editor_area
            };
            let top = area.top + 150.0;
            if Rect::new(area.left + 24.0, top, area.left + 194.0, top + 36.0).contains(x, y) {
                let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
                    self.state.status_text = "请先打开工作区再导入插件".into();
                    return;
                };
                if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                    match mochi_core::plugins::PluginService::new(root).install_zip(&path) {
                        Ok(plugin) => {
                            self.state.status_text =
                                format!("已导入原生插件：{}", plugin.manifest.name)
                        }
                        Err(error) => self.state.status_text = format!("导入插件失败：{error}"),
                    }
                }
            } else if Rect::new(area.left + 190.0, top, area.left + 370.0, top + 36.0)
                .contains(x, y)
            {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let service = mochi_core::plugins::PluginService::new(root);
                    let _ = std::fs::create_dir_all(service.root());
                    platform::open_external(&service.root().to_string_lossy());
                }
            } else if Rect::new(area.left + 386.0, top, area.left + 556.0, top + 36.0)
                .contains(x, y)
            {
                self.show_global_notice(
                    "在通用助手中描述要创建的插件，助手会按需加载“插件创作” Skill。",
                );
            } else if Rect::new(area.left + 572.0, top, area.left + 722.0, top + 36.0)
                .contains(x, y)
            {
                let Some(source) = platform::pick_folder(HWND(self.hwnd_raw as *mut _)) else {
                    return;
                };
                let name = format!(
                    "{}.zip",
                    source
                        .file_name()
                        .and_then(|name| name.to_str())
                        .filter(|name| !name.is_empty())
                        .unwrap_or("mochi-plugin")
                );
                if let Some(target) = platform::save_file(HWND(self.hwnd_raw as *mut _), &name) {
                    let workspace = self
                        .shell
                        .workspace()
                        .map(|ws| ws.root.clone())
                        .unwrap_or_else(|| source.clone());
                    match mochi_core::plugins::PluginService::new(workspace)
                        .package_dir(&source, &target)
                    {
                        Ok(()) => {
                            self.state.status_text = format!("插件已打包：{}", target.display())
                        }
                        Err(error) => self.state.status_text = format!("打包插件失败：{error}"),
                    }
                }
            } else if y >= area.top + 246.0 {
                let index = ((y - (area.top + 246.0)) / 40.0).floor() as usize;
                let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
                    self.state.status_text = "请先打开工作区再管理插件".into();
                    return;
                };
                let service = mochi_core::plugins::PluginService::new(root);
                let Ok(plugins) = service.list() else {
                    return;
                };
                let Some(plugin) = plugins.get(index) else {
                    return;
                };
                let row_top = area.top + 246.0 + index as f32 * 40.0;
                if Rect::new(
                    area.right - 240.0,
                    row_top + 4.0,
                    area.right - 174.0,
                    row_top + 28.0,
                )
                .contains(x, y)
                {
                    self.plugin_preview = Some(plugin.manifest.id.clone());
                    self.state.status_text = format!("已打开原生插件：{}", plugin.manifest.name);
                } else if Rect::new(
                    area.right - 166.0,
                    row_top + 4.0,
                    area.right - 100.0,
                    row_top + 28.0,
                )
                .contains(x, y)
                {
                    match service.set_enabled(&plugin.manifest.id, !plugin.enabled) {
                        Ok(()) => {
                            self.state.status_text = format!(
                                "插件 {} 已{}",
                                plugin.manifest.name,
                                if plugin.enabled { "禁用" } else { "启用" }
                            )
                        }
                        Err(error) => self.state.status_text = format!("更新插件状态失败：{error}"),
                    }
                } else if Rect::new(
                    area.right - 92.0,
                    row_top + 4.0,
                    area.right - 34.0,
                    row_top + 28.0,
                )
                .contains(x, y)
                {
                    match service.uninstall(&plugin.manifest.id) {
                        Ok(()) => {
                            if self.plugin_preview.as_deref() == Some(plugin.manifest.id.as_str()) {
                                self.plugin_preview = None;
                            }
                            self.state.status_text = format!("已移除插件：{}", plugin.manifest.name)
                        }
                        Err(error) => self.state.status_text = format!("移除插件失败：{error}"),
                    }
                }
            }
            return;
        }
        if self
            .settings_tab()
            .is_some_and(|(tab, section)| tab == "ai" && section != "parameters")
        {
            self.on_provider_click(x, y);
            return;
        }
        if let Some(index) = self.prefs.content_layout.reset_hit(x, y) {
            self.commit_settings_field();
            if let Some(d) = app_settings::descriptors().get(index) {
                self.app_settings.reset(d);
                if let Err(error) = self.app_settings.flush() {
                    self.state.status_text = format!("设置保存失败：{error}");
                }
                self.apply_setting_side_effects(&d.key);
            }
            return;
        }
        let hit = self.prefs.content_layout.hit(x, y);
        // 点到别处：先把正在编辑的框提交掉
        if self
            .prefs
            .editing
            .as_ref()
            .map(|e| Some(e.index) != hit)
            .unwrap_or(false)
        {
            self.commit_settings_field();
        }
        let Some(index) = hit else {
            self.focus = Focus::Main;
            return;
        };
        let Some(d) = app_settings::descriptors().get(index) else {
            return;
        };
        // 点击色块可打开预设颜色，旁边的十六进制值支持自定义输入。
        let value_type = if d.value_type == SettingValueType::Color
            && self
                .prefs
                .content_layout
                .control_rect(index)
                .is_some_and(|r| x >= r.left + 36.0)
        {
            SettingValueType::String
        } else {
            d.value_type
        };
        match value_type {
            SettingValueType::Boolean => {
                let on = matches!(self.app_settings.read(d), SettingValue::Bool(true));
                self.app_settings.write(d, &SettingValue::Bool(!on));
                let _ = self.app_settings.flush();
                self.apply_setting_side_effects(&d.key);
            }
            SettingValueType::Enum => {
                let Some(r) = self.prefs.content_layout.control_rect(index) else {
                    return;
                };
                let current = self.app_settings.read(d).to_storage();
                let items: Vec<MenuItem<MenuAction>> = d
                    .options
                    .iter()
                    .flatten()
                    .map(|o| {
                        let mut item = MenuItem::new(
                            settings::enum_option_label(d, o),
                            MenuAction::SetEnum(index, o.clone()),
                        );
                        if *o == current {
                            item = item.icon(Icon::CHECK);
                        }
                        item
                    })
                    .collect();
                self.menu = Some(Menu::open_at(
                    items,
                    r.left,
                    r.bottom + 4.0,
                    self.renderer.viewport(),
                ));
            }
            SettingValueType::Color => {
                self.commit_settings_field();
                let Some(r) = self.prefs.content_layout.control_rect(index) else {
                    return;
                };
                let colors = [
                    ("深色", "#111827"),
                    ("石墨", "#1F2937"),
                    ("灰色", "#64748B"),
                    ("浅灰", "#E5E7EB"),
                    ("白色", "#FFFFFF"),
                    ("红色", "#EF4444"),
                    ("橙色", "#F97316"),
                    ("黄色", "#EAB308"),
                    ("绿色", "#22C55E"),
                    ("青色", "#14B8A6"),
                    ("蓝色", "#3B82F6"),
                    ("紫色", "#A855F7"),
                    ("粉色", "#EC4899"),
                ];
                let current = self.app_settings.read(d).to_storage();
                let mut items = vec![MenuItem::new(
                    "恢复默认颜色",
                    MenuAction::SetColorSetting(index, d.default_value.to_storage()),
                )];
                items.extend(colors.into_iter().map(|(name, value)| {
                    let mut item = MenuItem::new(
                        format!("●  {name}"),
                        MenuAction::SetColorSetting(index, value.into()),
                    );
                    if current.eq_ignore_ascii_case(value) {
                        item = item.icon(Icon::CHECK);
                    }
                    item
                }));
                self.menu = Some(Menu::open_anchored(items, r, self.renderer.viewport()));
            }
            SettingValueType::Number | SettingValueType::String => {
                if self.prefs.editing.as_ref().map(|e| e.index) == Some(index) {
                    // 已经在编辑这个框：定位光标
                    if let (Some(e), Some(r)) = (
                        self.prefs.editing.as_mut(),
                        self.prefs.content_layout.control_rect(index),
                    ) {
                        e.field.click(x - (r.left + 12.0), shift_down());
                        self.drag = Some(Drag {
                            target: DragTarget::SettingsFieldSelect,
                            grab_offset: 0.0,
                        });
                    }
                    return;
                }
                let mut field = TextField::new("");
                field.style = TextStyle::Label;
                field.set_text(&self.app_settings.read(d).to_storage());
                if let Some(r) = self.prefs.content_layout.control_rect(index) {
                    field.click(x - r.left - 12.0, shift_down());
                }
                self.prefs.editing = Some(EditingField { index, field });
                self.focus = Focus::SettingsField;
                self.drag = Some(Drag {
                    target: DragTarget::SettingsFieldSelect,
                    grab_offset: 0.0,
                });
            }
        }
    }

    /// 提交设置输入框：按描述符校验/夹取后写入。非法输入丢弃、保留原值——
    /// 与 `coerce` 的语义一致，不做「静默改成别的值」。
    pub(super) fn commit_settings_field(&mut self) {
        let Some(e) = self.prefs.editing.take() else {
            return;
        };
        if self.focus == Focus::SettingsField {
            self.focus = Focus::Main;
        }
        let Some(d) = app_settings::descriptors().get(e.index) else {
            return;
        };
        let raw = e.field.text().trim().to_owned();
        let json = match d.value_type {
            SettingValueType::Number => match raw.parse::<f64>() {
                Ok(n) => serde_json::json!(n),
                Err(_) => {
                    self.state.status_text = "请输入有效数字；原设置已保留".into();
                    self.invalidate_main();
                    return;
                }
            },
            _ => serde_json::json!(raw),
        };
        match app_settings::coerce(d, &json) {
            Ok(coerced) => {
                self.app_settings.write(d, &coerced.value);
                if let Err(error) = self.app_settings.flush() {
                    self.state.status_text = format!("设置保存失败：{error}");
                }
                self.apply_setting_side_effects(&d.key);
                if coerced.clamped {
                    self.state.status_text = format!(
                        "{}已调整到允许范围：{}",
                        d.label,
                        coerced.value.to_storage()
                    );
                }
            }
            Err(error) => {
                self.state.status_text = format!("{error}；原设置已保留");
                self.invalidate_main();
            }
        }
    }

    /// 少数设置改完要立刻反映到外壳上（其余的由各面板在读取时生效）。
    pub(super) fn apply_setting_side_effects(&mut self, key: &str) {
        if key.starts_with("webClipper.") {
            self.state.status_text = match mochi_core::web_clipper::native::register(&self.settings)
            {
                Ok(()) => "网页剪藏设置已保存；在浏览器扩展中点击“连接墨池”".into(),
                Err(error) => format!("网页剪藏设置：{error}"),
            };
        }
        if key.starts_with("notifications.") {
            self.refresh_notification_preferences();
        }
        if key == "ai.memoryAutoExtract" {
            self.memory_cancel_if_disabled();
        }
        crate::ui::settings_values::load(&self.app_settings);
        if key.starts_with("background.") {
            match self.settings.background_image_path() {
                Ok(path) => {
                    self.background = path.and_then(|path| {
                        crate::ui::imginfo::dimensions(&path)
                            .map(|size| (path.to_string_lossy().into_owned(), size))
                    })
                }
                Err(error) => self.state.status_text = format!("背景图读取失败：{error}"),
            }
        }
        if key.starts_with("git.") {
            self.shell.apply_git_settings();
            self.panels.version.source = None;
        }
        if [
            "appearance.",
            "assistant.",
            "typography.",
            "headings.",
            "code.",
            "tables.",
            "lists.",
            "editorLayout.",
        ]
        .iter()
        .any(|prefix| key.starts_with(prefix))
        {
            crate::ui::editor_preferences::set(crate::ui::editor_preferences::Preferences::read(
                &self.app_settings,
            ));
            if let Err(e) = self.renderer.refresh_text_formats() {
                self.state.status_text = format!("更新字体失败：{e}");
            }
            self.doc.invalidate();
            self.split.doc.invalidate();
            self.source.invalidate();
            self.split.source.invalidate();
            self.editor_ai.invalidate();
        }
        match key {
            "general.startAtLogin" => self.sync_login_startup(true),
            "navigation.itemOrder" | "navigation.hiddenItems" => {
                self.nav.scroll = 0.0;
            }
            "sidebar.sortOrder" => {
                let mode = crate::ui::settings_values::text(key, "manual");
                self.shell.set_sort_mode(&mode);
            }
            "sidebar.showFavoriteParents" => {
                let enabled = crate::ui::settings_values::boolean(key, false);
                let changed = self.shell.show_favorite_parents() != enabled;
                self.shell.set_favorite_show_parents(enabled);
                if changed && self.shell.favorites_selected() {
                    self.side.scroll = 0.0;
                }
            }
            "editorLayout.aiPanelWidth" | "editorLayout.splitRatio" => {
                if let Some(SettingValue::Number(n)) =
                    app_settings::descriptor(key).map(|d| self.app_settings.read(d))
                {
                    if key.ends_with("aiPanelWidth") {
                        self.state.ai_panel_width = n as f32;
                    } else {
                        self.split.ratio = (n as f32).clamp(0.2, 0.8);
                    }
                }
            }
            "appearance.themeMode" => {
                // system 跟随 Windows 的「应用模式」（Electron 用 nativeTheme.shouldUseDarkColors）
                let v = app_settings::descriptor(key)
                    .map(|d| self.app_settings.read(d).to_storage())
                    .unwrap_or_default();
                self.state.dark = match v.as_str() {
                    "dark" => true,
                    "light" => false,
                    _ => platform::system_prefers_dark(),
                };
            }
            "navigation.collapsed" => {
                let v = app_settings::descriptor(key).map(|d| self.app_settings.read(d));
                self.state.navigation_collapsed = matches!(v, Some(SettingValue::Bool(true)));
            }
            "navigation.width" | "sidebar.width" => {
                if let Some(SettingValue::Number(n)) =
                    app_settings::descriptor(key).map(|d| self.app_settings.read(d))
                {
                    if key == "navigation.width" {
                        self.state.navigation_width = n as f32;
                    } else {
                        self.state.sidebar_width = n as f32;
                    }
                }
            }
            "tabs.compact" => {
                let v = app_settings::descriptor(key).map(|d| self.app_settings.read(d));
                self.state.compact_tab_bar = matches!(v, Some(SettingValue::Bool(true)));
            }
            "outline.placement" => {
                let placement = app_settings::descriptor(key)
                    .map(|d| self.app_settings.read(d).to_storage())
                    .unwrap_or_default();
                self.state.outline_in_ai_sidebar = placement == "ai-sidebar";
                self.outline_left_active = placement == "left-sidebar";
                if placement == "ai-sidebar" {
                    self.state.right_panel = RightPanel::Outline;
                }
                self.refresh_right_panel_if_stale();
            }
            _ => {}
        }
        if key.starts_with("appearance.") {
            self.desktop_refresh_theme();
            if let Some(capture) = &self.capture {
                capture.refresh_theme(self.state.dark);
            }
        }
        self.invalidate_main();
    }

    /// 启动时把会影响外壳几何/主题的设置读进 `ChromeState`。
    pub(super) fn load_chrome_settings(&mut self) {
        self.refresh_notification_preferences();
        self.apply_setting_side_effects("background.enabled");
        crate::ui::shortcuts::load(self.settings.get("keyboard.shortcuts").as_deref());
        self.apply_setting_side_effects("typography.fontSize");
        self.apply_setting_side_effects("editorLayout.aiPanelWidth");
        self.apply_setting_side_effects("editorLayout.splitRatio");
        self.apply_setting_side_effects("sidebar.sortOrder");
        self.apply_setting_side_effects("sidebar.showFavoriteParents");
        for key in [
            "appearance.themeMode",
            "navigation.collapsed",
            "navigation.width",
            "sidebar.width",
            "tabs.compact",
            "tabs.fixedWidth",
            "outline.placement",
        ] {
            self.apply_setting_side_effects(key);
        }
        self.settings_revision = self.settings.revision();
    }

    /// 重新应用共享偏好设置时，不要替换当前工作区、
    /// 对话、服务方表单或文档编辑内容。
    pub fn refresh_shared_settings(&mut self) -> bool {
        if let Err(error) = self.settings.reload() {
            self.state.status_text = format!("共享设置读取失败：{error}");
            return true;
        }
        if self.settings.revision() == self.settings_revision {
            // 另一个工作区关闭后重新取得所有权，或重试启动时遇到的冲突。
            // 重复注册不会产生额外影响；仅切换焦点不会创建通知。
            if let Ok(chord) = crate::ui::shortcuts::Chord::parse(&crate::ui::shortcuts::binding(
                "Ctrl+Shift+Space",
            )) {
                let _ = self.register_capture_key(chord);
            }
            return false;
        }
        self.load_chrome_settings();
        self.shell.apply_git_settings();
        self.panels.version.source = None;
        self.reload_providers();
        if let Some(permissions) = &self.ai.permissions {
            permissions.set_action_permissions(self.prefs.providers.actions.clone());
        }
        self.search_history = self
            .settings
            .get("search.history")
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        self.editor_ai.invalidate();
        self.memory_cancel_if_disabled();
        let key = crate::ui::shortcuts::binding("Ctrl+Shift+Space");
        if let Ok(chord) = crate::ui::shortcuts::Chord::parse(&key) {
            let _ = self.register_capture_key(chord);
        }
        self.invalidate_main();
        true
    }

    pub(super) fn persist_chrome_setting(&mut self, key: &str, value: SettingValue) {
        if let Some(descriptor) = app_settings::descriptor(key) {
            self.app_settings.write(descriptor, &value);
            if let Err(error) = self.settings.flush() {
                self.state.status_text = format!("设置保存失败：{error}");
            }
        }
    }
}
