//! 根据当前页面状态绘制主内容区。
use super::*;

impl App {
    /// 主区。分派只有这一个 match——加一种内容不需要动别处。
    pub(super) fn paint_main(&mut self, chrome: &Chrome, p: &Palette) {
        let mut area = chrome.tree.rect(chrome.editor);
        let content = self.content();
        self.outline_area = Rect::ZERO;
        self.outline_toggle_rect = Rect::ZERO;
        self.links_layout = backlinks::Layout::default();
        area = self.paint_split(area, p);

        // 工具栏：只在编辑文件（渲染视图）时显示，贴在编辑区顶部
        self.toolbar_area = Rect::ZERO;
        if content == MainContent::Document
            && self.shell.active().and_then(|t| t.buffer()).is_some()
        {
            let mut state = self
                .shell
                .active()
                .and_then(|t| t.buffer())
                .map(|b| toolbar::State::at(b.text(), b.cursor()))
                .unwrap_or_default();
            if let Some(edit) = self.table_editing.as_ref() {
                state = toolbar::State::at(edit.field.text(), edit.field.buffer.cursor());
                state.heading_level = 0;
            }
            let probe = Rect::new(area.left, area.top, area.right, area.top + toolbar::HEIGHT);
            let lay = toolbar::layout_mode(
                probe,
                state.heading_level,
                crate::ui::settings_values::boolean("editor.simpleDocumentMode", true),
            );
            let bar = Rect::new(area.left, area.top, area.right, area.top + lay.height);
            self.toolbar_area = bar;
            area.top = bar.bottom;
            state.hover = self.toolbar_hover;
            state.heading_open = self.toolbar_menu == Some(ToolbarMenu::Heading);
            state.insert_open = self.toolbar_menu == Some(ToolbarMenu::Insert);
            self.list.push_clip(bar);
            toolbar::paint(&mut self.list, bar, &lay, &state, p);
            let plugin_toolbar_slots = self
                .shell
                .workspace()
                .and_then(|ws| {
                    mochi_core::plugins::PluginService::new(&ws.root)
                        .registered_slots()
                        .ok()
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|slot| {
                    slot.contribution.slot == mochi_core::plugins::PluginSlot::EditorToolbar
                })
                .collect::<Vec<_>>();
            let mut right = bar.right - 12.0;
            for slot in plugin_toolbar_slots.iter().rev().take(2) {
                let width =
                    (crate::ui::text::measure(&slot.contribution.title, TextStyle::Caption) + 32.0)
                        .min(150.0);
                let button = Rect::new(right - width, bar.top + 6.0, right, bar.bottom - 6.0);
                self.list.rounded_border(button, 5.0, p.border);
                self.list.icon_centered(
                    Rect::new(
                        button.left + 6.0,
                        button.top,
                        button.left + 24.0,
                        button.bottom,
                    ),
                    crate::ui::icons::Icon::PUZZLE,
                    14.0,
                    p.muted,
                );
                self.list.text(
                    Rect::new(
                        button.left + 27.0,
                        button.top,
                        button.right - 7.0,
                        button.bottom,
                    ),
                    &slot.contribution.title,
                    TextStyle::Caption,
                    p.foreground,
                );
                right = button.left - 6.0;
            }
            self.list.pop_clip();
        }
        if (content == MainContent::Document
            || content == MainContent::Special && self.base_viewer_open())
            && crate::ui::settings_values::boolean("editorLayout.cardEnabled", false)
        {
            let margin = crate::ui::settings_values::number("editorLayout.cardMargin", 32.0)
                .min(area.width() / 4.0)
                .min(area.height() / 4.0);
            area = Rect::new(
                area.left + margin,
                area.top + margin,
                area.right - margin,
                area.bottom - margin,
            );
            self.list.rounded_rect_alpha(
                area,
                8.0,
                if self.state.dark { 0x1e1e1e } else { 0xffffff },
                crate::ui::settings_values::number("background.uiOpacity", 85.0) / 100.0,
            );
        }
        if content == MainContent::Document {
            let mode = self.outline_mode();
            if mode == "editor-right" && area.width() > 500.0 {
                let width = app_settings::descriptor("outline.width")
                    .and_then(|d| {
                        if let SettingValue::Number(n) = self.app_settings.read(d) {
                            Some(n as f32)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(270.0)
                    .min(area.width() * 0.40);
                if self.outline_visible_in_file_area() {
                    self.outline_area =
                        Rect::new(area.right - width, area.top, area.right, area.bottom);
                    self.outline_toggle_rect = Rect::new(
                        self.outline_area.right - 30.0,
                        self.outline_area.top + 4.0,
                        self.outline_area.right - 6.0,
                        self.outline_area.top + 28.0,
                    );
                    area.right -= width + 24.0;
                } else {
                    // 大纲收起后正文恢复全宽，但在文件区右上角保留紧凑入口。
                    self.outline_toggle_rect = Rect::new(
                        area.right - 88.0,
                        area.top + 4.0,
                        area.right - 6.0,
                        area.top + 28.0,
                    );
                }
            }
        }
        // 小记复用同一套文档标题、正文宽度和命中坐标；不要再插入工作区页头。
        if content == MainContent::Document {
            area = crate::ui::editor_preferences::current().aligned_area(area);
        }
        self.editor_area = area;
        let scroll = self.shell.active_scroll();

        match content {
            MainContent::Standalone(WorkspaceView::MochiAi) => {
                self.ai.panel.standalone = true;
                self.ai.panel.automatic_edits =
                    crate::ui::settings_values::text("ai.editApplyMode", "approve") == "auto";
                self.ai.panel.provider_missing =
                    ai_runtime::load_provider(&self.settings).is_none();
                self.ai.panel.search_focused = self.focus == Focus::AiMessageQuery;
                let layout_scroll = self.ai.panel.scroll;
                let mut lay = assistant::layout(&self.ai.panel, area);
                self.ai.panel.horizontal.sync(&lay);
                self.ai.panel.apply_locator(&lay);
                self.ai.panel.scroll = self.ai.panel.scroll.min(lay.max_scroll());
                if layout_scroll != self.ai.panel.scroll {
                    lay = assistant::layout(&self.ai.panel, area);
                }
                if let Some(r) = lay.rect_of(assistant::Hit::SearchInput) {
                    self.ai
                        .panel
                        .search_query
                        .sync_singleline_scroll(r.width() - 138.0);
                }
                if self.focus == Focus::AiInput {
                    if let Some(r) = lay.rect_of(assistant::Hit::Input) {
                        self.ai.panel.input.sync_multiline_scroll(r);
                    }
                }
                assistant::paint(
                    &mut self.list,
                    area,
                    &self.ai.panel,
                    &lay,
                    self.focus == Focus::AiInput,
                    p,
                );
                self.ai.layout = lay;
            }
            MainContent::Home => {
                if self.home_dirty && self.home_rx.is_none() {
                    if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                        self.start_home_analytics(HWND(self.hwnd_raw as *mut _), root);
                    }
                }
                self.home.paint(&mut self.list, area, p);
            }
            MainContent::Document => {
                if self.hwnd_raw != 0 {
                    self.doc.set_progressive(true);
                }
                let tab = self.shell.active_tab();
                let buffer = self.shell.active().and_then(|t| t.buffer());
                // 没点进编辑器就不露源码、不画光标——刚打开文件时整篇都应是渲染态。
                // 查找条拿着焦点时选区（当前匹配）仍要显示，只是不画光标
                let find_focused = matches!(self.focus, Focus::FindQuery | Focus::FindReplacement);
                let caret = self.focus == Focus::Main && self.editor_engaged;
                let active = caret || (find_focused && self.editor_engaged);
                let cursor = buffer.filter(|_| active).map(|b| b.display_cursor());
                let composed = buffer
                    .filter(|b| b.composition().is_some())
                    .map(|b| b.display_text().0);
                let base_dir = self
                    .shell
                    .active()
                    .and_then(|t| t.path())
                    .and_then(|p| p.parent().map(Path::to_path_buf));
                self.doc.set_base_dir(base_dir.as_deref());
                if let Some(path) = self.active_file_path() {
                    self.doc.set_code_document(&path);
                }
                if let Some(composed) = composed.as_deref() {
                    // 输入法预览不会提交进 TextBuffer，因此没有自己的内容版本号。
                    self.doc.ensure(area, tab, Some(composed), cursor);
                } else if let Some(buffer) = buffer {
                    self.doc.ensure_buffer(area, tab, buffer, cursor);
                } else {
                    self.doc.ensure(area, tab, None, cursor);
                }
                let mut area = area;
                if let Some(plan) = self.doc.chunk_plan() {
                    let bar = Rect::new(
                        area.left,
                        area.top,
                        area.right,
                        (area.top + crate::ui::large_document::BAR_HEIGHT).min(area.bottom),
                    );
                    crate::ui::large_document::paint(
                        &mut self.list,
                        bar,
                        plan,
                        self.doc.chunk_index(),
                        self.doc.is_chunked(),
                        p,
                    );
                    area.top = bar.bottom;
                    self.editor_area = area;
                }
                self.doc.hide_title(self.title_editing.is_some());
                self.doc.paint(&mut self.list, area, scroll, p);
                if let Some(buffer) = buffer {
                    self.doc
                        .paint_overlay(&mut self.list, area, buffer, scroll, active, caret, p);
                    if let Some(edit) = self.table_editing.as_mut() {
                        if let Some(cell) = self
                            .doc
                            .table_cells(area, buffer.text(), scroll)
                            .into_iter()
                            .find(|c| c.range.start == edit.cell.range.start)
                        {
                            edit.cell.rect = cell.rect;
                        }
                        self.list.push_clip(area);
                        edit.paint(&mut self.list, self.focus == Focus::TableCell, p);
                        self.list.pop_clip();
                    }
                }
                self.paint_title_edit(area, p);
                self.links.scroll = 0.0;
                if crate::ui::settings_values::boolean("editor.showDocumentLinks", false) {
                    let top = area.top + self.doc.content_height() - scroll;
                    let tail = backlinks::layout_tail(
                        &self.links,
                        Rect::new(area.left + 12.0, top, area.right - 12.0, top + 100000.0),
                        area,
                    );
                    self.doc.set_tail_height(tail.content_height + 160.0);
                    backlinks::paint(&mut self.list, &self.links, &[], None, &tail, p);
                    self.links_layout = tail;
                } else {
                    self.doc.set_tail_height(0.0);
                }
                if !self.outline_area.is_empty() {
                    outline::paint(
                        &mut self.list,
                        self.outline_area,
                        self.doc.headings(),
                        self.doc.active_heading(scroll),
                        self.outline_scroll,
                        p,
                    );
                }
                if !self.outline_toggle_rect.is_empty() {
                    if self.outline_area.is_empty() {
                        self.list
                            .rounded_rect(self.outline_toggle_rect, 6.0, p.surface_muted);
                        self.list.text(
                            Rect::new(
                                self.outline_toggle_rect.left + 8.0,
                                self.outline_toggle_rect.top,
                                self.outline_toggle_rect.right - 27.0,
                                self.outline_toggle_rect.bottom,
                            ),
                            "大纲",
                            TextStyle::Caption,
                            p.muted,
                        );
                    }
                    self.list.icon(
                        Rect::new(
                            self.outline_toggle_rect.right - 22.0,
                            self.outline_toggle_rect.top + 4.0,
                            self.outline_toggle_rect.right - 6.0,
                            self.outline_toggle_rect.bottom - 4.0,
                        ),
                        Icon::EYE,
                        p.muted,
                    );
                }
                self.paint_prediction(area, p);
                self.paint_find_bar(chrome, p);
            }
            MainContent::Source => {
                self.doc.set_tail_height(0.0);
                let Some(index) = self.shell.active_tab() else {
                    return;
                };
                let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
                    return;
                };
                self.source.ensure(area, index, buffer);
                self.source.paint(
                    &mut self.list,
                    area,
                    buffer,
                    scroll,
                    self.focus == Focus::Main,
                    p,
                );
                self.paint_find_bar(chrome, p);
            }
            MainContent::Special => {
                if let Some((tab, section)) = self.settings_tab() {
                    if tab == "ai" && section != "parameters" {
                        self.paint_provider_settings(area, p);
                        return;
                    }
                    if tab == "plugins" {
                        self.list.text(
                            Rect::new(
                                area.left + 24.0,
                                area.top + 16.0,
                                area.right - 24.0,
                                area.top + 48.0,
                            ),
                            "第三方库",
                            TextStyle::Large,
                            p.foreground,
                        );
                        self.list.text(
                            Rect::new(
                                area.left + 24.0,
                                area.top + 72.0,
                                area.right - 24.0,
                                area.top + 104.0,
                            ),
                            "暂不支持",
                            TextStyle::Label,
                            p.foreground,
                        );
                        self.list.text(
                            Rect::new(
                                area.left + 24.0,
                                area.top + 110.0,
                                area.right - 24.0,
                                area.top + 136.0,
                            ),
                            "导入后安装到当前工作区 .mochi/plugins；不加载 HTML/JS 或 Electron。",
                            TextStyle::Caption,
                            p.muted,
                        );
                        for (left, right, label) in [
                            (24.0, 174.0, "导入原生 ZIP"),
                            (190.0, 370.0, "打开原生插件目录"),
                            (386.0, 556.0, "插件创作指南"),
                            (572.0, 722.0, "打包插件工程"),
                        ] {
                            let r = Rect::new(
                                area.left + left,
                                area.top + 150.0,
                                area.left + right,
                                area.top + 186.0,
                            );
                            self.list.rounded_border(r, 8.0, p.border);
                            self.list.text(
                                Rect::new(r.left + 10.0, r.top, r.right, r.bottom),
                                label,
                                TextStyle::Caption,
                                p.foreground,
                            );
                        }
                        if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                            match mochi_core::plugins::PluginService::new(&root).list() {
                                Ok(plugins) if plugins.is_empty() => self.list.text(
                                    Rect::new(
                                        area.left + 24.0,
                                        area.top + 210.0,
                                        area.right - 24.0,
                                        area.top + 236.0,
                                    ),
                                    "尚未安装原生插件。",
                                    TextStyle::Label,
                                    p.muted,
                                ),
                                Ok(plugins) => {
                                    self.list.text(
                                        Rect::new(
                                            area.left + 24.0,
                                            area.top + 210.0,
                                            area.right - 24.0,
                                            area.top + 236.0,
                                        ),
                                        "已安装的原生插件",
                                        TextStyle::Label,
                                        p.foreground,
                                    );
                                    for (index, plugin) in plugins.iter().enumerate() {
                                        let y = area.top + 246.0 + index as f32 * 40.0;
                                        self.list.rounded_rect(
                                            Rect::new(
                                                area.left + 24.0,
                                                y,
                                                area.right - 24.0,
                                                y + 32.0,
                                            ),
                                            6.0,
                                            p.surface,
                                        );
                                        let remove = Rect::new(
                                            area.right - 92.0,
                                            y + 4.0,
                                            area.right - 34.0,
                                            y + 28.0,
                                        );
                                        let toggle = Rect::new(
                                            area.right - 166.0,
                                            y + 4.0,
                                            area.right - 100.0,
                                            y + 28.0,
                                        );
                                        let open = Rect::new(
                                            area.right - 240.0,
                                            y + 4.0,
                                            area.right - 174.0,
                                            y + 28.0,
                                        );
                                        self.list.rounded_border(open, 5.0, p.border);
                                        self.list.rounded_border(toggle, 5.0, p.border);
                                        self.list.rounded_border(remove, 5.0, p.border);
                                        self.list.text(
                                            Rect::new(
                                                area.left + 36.0,
                                                y,
                                                open.left - 10.0,
                                                y + 32.0,
                                            ),
                                            format!(
                                                "{}  ·  v{}  ·  {}",
                                                plugin.manifest.name,
                                                plugin.manifest.version,
                                                if plugin.enabled {
                                                    "已启用"
                                                } else {
                                                    "已禁用"
                                                }
                                            ),
                                            TextStyle::Caption,
                                            p.foreground,
                                        );
                                        self.list.text(
                                            open,
                                            "打开",
                                            TextStyle::Caption,
                                            p.foreground,
                                        );
                                        self.list.text(
                                            toggle,
                                            if plugin.enabled { "禁用" } else { "启用" },
                                            TextStyle::Caption,
                                            p.foreground,
                                        );
                                        self.list.text(
                                            remove,
                                            "移除",
                                            TextStyle::Caption,
                                            p.danger,
                                        );
                                    }
                                    if let Some(id) = self.plugin_preview.as_deref() {
                                        if let Some(plugin) =
                                            plugins.iter().find(|plugin| plugin.manifest.id == id)
                                        {
                                            let y = area.top + 266.0 + plugins.len() as f32 * 40.0;
                                            match mochi_core::plugins::PluginService::new(root)
                                                .load_ui(plugin)
                                            {
                                                Ok(Some(ui)) => {
                                                    self.list.rounded_rect(
                                                        Rect::new(
                                                            area.left + 24.0,
                                                            y,
                                                            area.right - 24.0,
                                                            y + 120.0,
                                                        ),
                                                        8.0,
                                                        p.surface,
                                                    );
                                                    self.list.text(
                                                        Rect::new(
                                                            area.left + 40.0,
                                                            y + 12.0,
                                                            area.right - 40.0,
                                                            y + 38.0,
                                                        ),
                                                        ui.title,
                                                        TextStyle::Label,
                                                        p.foreground,
                                                    );
                                                    self.list.text(
                                                        Rect::new(
                                                            area.left + 40.0,
                                                            y + 40.0,
                                                            area.right - 40.0,
                                                            y + 62.0,
                                                        ),
                                                        format!("原生声明式界面 · {}", ui.kind),
                                                        TextStyle::Caption,
                                                        p.muted,
                                                    );
                                                    if let Some(section) = ui.sections.first() {
                                                        self.list.text(
                                                            Rect::new(
                                                                area.left + 40.0,
                                                                y + 68.0,
                                                                area.right - 40.0,
                                                                y + 90.0,
                                                            ),
                                                            &section.title,
                                                            TextStyle::Caption,
                                                            p.foreground,
                                                        );
                                                        self.list.text(
                                                            Rect::new(
                                                                area.left + 52.0,
                                                                y + 92.0,
                                                                area.right - 40.0,
                                                                y + 112.0,
                                                            ),
                                                            section
                                                                .cards
                                                                .iter()
                                                                .take(3)
                                                                .cloned()
                                                                .collect::<Vec<_>>()
                                                                .join("  ·  "),
                                                            TextStyle::Caption,
                                                            p.muted,
                                                        );
                                                    }
                                                }
                                                Ok(None) => self.list.text(
                                                    Rect::new(
                                                        area.left + 24.0,
                                                        y,
                                                        area.right - 24.0,
                                                        y + 32.0,
                                                    ),
                                                    "该插件没有声明式 UI。",
                                                    TextStyle::Caption,
                                                    p.muted,
                                                ),
                                                Err(error) => self.list.text(
                                                    Rect::new(
                                                        area.left + 24.0,
                                                        y,
                                                        area.right - 24.0,
                                                        y + 32.0,
                                                    ),
                                                    format!("无法加载插件界面：{error}"),
                                                    TextStyle::Caption,
                                                    p.danger,
                                                ),
                                            }
                                        }
                                    }
                                }
                                Err(error) => self.list.text(
                                    Rect::new(
                                        area.left + 24.0,
                                        area.top + 210.0,
                                        area.right - 24.0,
                                        area.top + 236.0,
                                    ),
                                    format!("无法读取插件：{error}"),
                                    TextStyle::Caption,
                                    p.danger,
                                ),
                            }
                        }
                        return;
                    }
                    let section_opt = if tab == "customization" {
                        Some(section.as_str())
                    } else {
                        None
                    };
                    let mut lay = settings::content_layout_filtered(
                        area,
                        &tab,
                        section_opt,
                        self.prefs.scroll,
                        self.prefs.search.text(),
                    );
                    let max = lay.max_scroll();
                    let clamped = self.prefs.scroll.clamp(0.0, max);
                    if clamped != self.prefs.scroll {
                        self.prefs.scroll = clamped;
                        lay = settings::content_layout_filtered(
                            area,
                            &tab,
                            section_opt,
                            clamped,
                            self.prefs.search.text(),
                        );
                    }
                    settings::sync_editing_scroll(&lay, self.prefs.editing.as_mut());
                    let app_settings = &self.app_settings;
                    let read = |d: &app_settings::SettingDescriptor| app_settings.read(d);
                    let model = settings::ContentModel {
                        tab: &tab,
                        section: section_opt,
                        query: self.prefs.search.text(),
                        read: &read,
                        editing: self.prefs.editing.as_ref(),
                    };
                    settings::paint_content(
                        &mut self.list,
                        area,
                        &lay,
                        &model,
                        self.prefs.scroll,
                        p,
                    );
                    if !lay.search_rect.is_empty() {
                        let mut search = self.prefs.search.clone();
                        search.paint(
                            &mut self.list,
                            lay.search_rect,
                            self.focus == Focus::SettingsSearch,
                            p,
                            FieldLook::dialog(p),
                        );
                    }
                    self.prefs.content_layout = lay;
                } else if let Some((path, content)) = self.viewer_tab() {
                    let path = path.to_path_buf();
                    let cache = match (content, self.shell.workspace()) {
                        (viewer::Content::Link(s), Some(ws))
                            if s.cached && s.content.is_empty() =>
                        {
                            mochi_core::link_files::read_cache(&ws.root, &s.url)
                        }
                        _ => None,
                    };
                    let is_pdf = matches!(content, viewer::Content::Pdf(_));
                    let is_sheet = matches!(content,viewer::Content::Spreadsheet(s)if !s.requested);
                    let is_exam = matches!(content, viewer::Content::Exam(_));
                    if let Some(cache) = cache {
                        if let Some(viewer::Content::Link(s)) = self.viewer_content_for(&path) {
                            s.content = cache.content;
                        }
                    }
                    if is_pdf {
                        // 工作线程按需起；可见页没渲染的点名去要
                        self.ensure_pdf_job(&path);
                        self.request_visible_pdf_pages(area);
                    }
                    if is_sheet {
                        self.start_sheet_job(&path, None);
                    }
                    if is_exam {
                        self.load_exam_drafts();
                    }
                    let Some(TabKind::Viewer { content, .. }) =
                        self.shell.active().map(|tab| &tab.kind)
                    else {
                        return;
                    };
                    let lay = viewer::layout(area, content);
                    let src = path.to_string_lossy().into_owned();
                    let model = viewer::Model {
                        content,
                        path: &path,
                        image_src: &src,
                        now_ms: Self::now_ms().max(0) as u64,
                        hover: self.viewer_hover,
                    };
                    viewer::paint(&mut self.list, area, &lay, &model, p);
                    self.viewer_layout = lay;
                    self.paint_find_bar(chrome, p);
                } else if let Some(text) = crate::view::placeholder_text(MainContent::Special) {
                    crate::view::placeholder(&mut self.list, area, text, p);
                }
            }
            MainContent::Standalone(WorkspaceView::Schedule) => self.paint_schedule(area, p),
            MainContent::Standalone(WorkspaceView::Inbox) => self.paint_inbox(area, p),
            MainContent::Standalone(WorkspaceView::Recent) => self.paint_recent(area, p),
            MainContent::Standalone(WorkspaceView::Templates) => self.paint_templates(area, p),
            MainContent::Standalone(WorkspaceView::Marketplace) => self.paint_marketplace(area, p),
            MainContent::Standalone(WorkspaceView::DesktopCards) => self.paint_desktop_manager(p),
            MainContent::Standalone(WorkspaceView::Plugin) => self.paint_plugin_workspace(area, p),
            MainContent::Standalone(WorkspaceView::AgentConfig) => self.paint_agent_config(area, p),
            MainContent::Standalone(WorkspaceView::Automations) => self.paint_workflows(area, p),
            empty => {
                if let Some(text) = crate::view::placeholder_text(empty) {
                    crate::view::placeholder(&mut self.list, area, text, p);
                }
            }
        }
    }
}
