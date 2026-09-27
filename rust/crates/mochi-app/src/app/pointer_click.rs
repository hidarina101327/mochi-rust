//! 根据点击位置分派应用和文档区域的操作。
use super::*;

impl App {
    pub fn on_click(&mut self, x: f32, y: f32) {
        if let Some(pending) = self.global_import.as_ref() {
            let hit = pending.view.hit(self.renderer.viewport(), x, y);
            self.import_picker_action(hit);
            return;
        }
        self.window_motion.keyboard = false;
        if self.desktop_manager_captures_pointer(x, y) {
            self.desktop_manager_click(x, y);
            return;
        }
        if self.notification_open() {
            self.notification_click(x, y);
            return;
        }
        if self.template_picker.is_some() {
            self.on_template_picker_click(x, y);
            return;
        }
        if self.object_picker.is_some() {
            self.object_picker_click(x, y);
            return;
        }
        if self.image_preview.is_some() {
            self.image_preview = None;
            return;
        }
        if let Some(form) = self.export_form.as_mut() {
            let action = form
                .hit(self.renderer.viewport(), x, y)
                .and_then(|i| form.activate(i));
            self.export_form_action(action);
            return;
        }
        if self.commands.review.is_some() {
            self.command_review_click(x, y);
            return;
        }
        // 模态对话框吞掉一切
        if self.mapped_folder.is_some() {
            self.on_mapped_folder_click(x, y);
            return;
        }
        if self.link_create.is_some() {
            self.on_link_create_click(x, y);
            return;
        }
        if self.dialog.is_some() {
            self.on_dialog_click(x, y);
            return;
        }
        if self.automation.panel.is_some() {
            self.automation_click(x, y);
            return;
        }
        // 菜单画在设置之上，输入自然也要先交给菜单。
        if self.menu_input_active() {
            self.menu_click(x, y);
            return;
        }
        if self.settings_overlay.is_some() {
            let modal = self.settings_overlay_rect();
            let close = settings::close_rect(modal);
            if !modal.contains(x, y) || close.contains(x, y) {
                self.close_settings();
                return;
            }
            if x < modal.left + 224.0_f32.min(modal.width() * 0.34) {
                self.on_settings_nav_click(x, y)
            } else {
                self.on_settings_content_click(x, y)
            }
            return;
        }
        if self.search.is_some() {
            self.on_search_click(x, y);
            return;
        }
        if self.command.is_some() {
            self.on_command_click(x, y);
            return;
        }
        if self.base_detail_open() && self.menu.is_none() {
            let hit = self.viewer_tab().and_then(|(_, content)| match content {
                viewer::Content::Base(state) => {
                    base_view::layout(state, self.renderer.viewport()).hit(x, y)
                }
                _ => None,
            });
            if let Some(hit) = hit {
                self.focus = Focus::Main;
                self.on_base_click(hit);
            }
            return;
        }
        if let Some(picker) = self.table_picker.take() {
            if let Some((rows, cols)) = picker.hit(x, y) {
                self.insert_editor_table(rows, cols);
            }
            return;
        }
        if self.wiki_click(x, y) {
            return;
        }
        if let Some(edit) = self.table_editing.as_mut() {
            if edit.cell.rect.contains(x, y) {
                edit.click(x, y, shift_down());
                self.focus = Focus::TableCell;
                return;
            }
            if !self.commit_table_cell() {
                return;
            }
        }
        if self.focus == Focus::CanvasText && !self.editor_area.contains(x, y) {
            self.canvas_finish_editing();
        }
        if let Some(edit) = self.title_editing.as_mut() {
            let r = self
                .doc
                .title_rect(self.editor_area, self.shell.active_scroll());
            if r.contains(x, y) {
                edit.field.click(x - r.left, shift_down());
                return;
            }
            if !self.commit_title() {
                return;
            }
        }
        // 菜单开着：点在项上就执行，点在外面就关掉（并且这次点击不再往下传——
        // Radix 的行为也是如此，点空白只关菜单）
        if self.menu.is_some() {
            self.menu_click(x, y);
            return;
        }
        if self.ai_float_click(x, y) {
            return;
        }
        // 查找条叠在编辑区上，先于工具栏与 chrome 命中；点在外面不关它（TSX 也不关）
        if !self.ai.layout.messages_rect.contains(x, y) {
            self.ai_clear_text_selection();
        }
        // 覆盖层上的轨道条自己处理边缘点击，不移动焦点、不改动文本选区。
        if self.begin_scrollbar_drag(x, y) {
            return;
        }
        // 先换分栏归属，再做文档控件与工具栏的命中测试。
        // 同一次点击必须作用于刚获得焦点的那个分栏。
        if self.state.view == WorkspaceView::Editor
            && matches!(self.content(), MainContent::Document | MainContent::Source)
        {
            if self.split.close.contains(x, y) {
                self.close_split_view();
                return;
            }
            if self.split.divider_hit.contains(x, y) {
                self.drag = Some(Drag {
                    target: DragTarget::Split,
                    grab_offset: x - (self.split.divider.left + self.split.divider.right) * 0.5,
                });
                return;
            }
            if self.split.header.contains(x, y) {
                self.focus_other_editor();
                return;
            }
            if self.split.active_header.contains(x, y) {
                self.focus = Focus::Main;
                return;
            }
            if self.split.pane_area.contains(x, y) && !self.focus_other_editor() {
                return;
            }
        }
        if self.content() == MainContent::Document {
            if self.click_document_chunk_bar(x, y) {
                return;
            }
            if self.click_comment_bubble(x, y) {
                return;
            }
            if self.begin_editor_control_drag(x, y) {
                return;
            }
            if let Some((rect, start, empty)) =
                self.doc
                    .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
            {
                if rect.contains(x, y) {
                    self.open_block_menu(start, empty, rect.left, rect.bottom);
                    return;
                }
            }
        }
        if self.find.is_some() {
            if let Some(hit) = self.find_layout.hit(x, y) {
                self.on_find_click(hit, x);
                return;
            }
        }
        // 编辑器工具栏在 chrome 的编辑区里，先于 chrome 命中
        if self.toolbar_area.contains(x, y) {
            if let Some(hit) = self.toolbar_layout().hit(x, y) {
                self.on_toolbar_click(hit);
            }
            return;
        }
        let chrome = self.build_chrome();
        // 点到输入框之外就收起新建库输入框（TSX 的 onBlur：有内容则确认，否则取消）
        if self.nav.creating.is_some() && self.nav_layout.hit(x, y) != Some(NavHit::NewLibraryField)
        {
            self.finish_new_library();
        }
        if self.side.editing.is_some() && self.side.layout.hit(x, y) != Some(SidebarHit::Editor) {
            self.finish_sidebar_edit();
        }
        if self.focus == Focus::SidebarSearch
            && self.side.layout.hit(x, y) != Some(SidebarHit::Search)
        {
            self.focus = Focus::Main;
        }
        if self.workflows_active() && self.workflows.view.area.contains(x, y) {
            self.workflows_click(x, y);
            return;
        }
        match chrome.hit(x, y) {
            Some(NodeKey::TitleBarBack) => self.return_from_auxiliary_page(),
            Some(NodeKey::TitleBarThemeToggle) => {
                let mode = if self.state.dark { "light" } else { "dark" };
                self.persist_chrome_setting(
                    "appearance.themeMode",
                    SettingValue::Text(mode.into()),
                );
                self.apply_setting_side_effects("appearance.themeMode");
            }
            Some(NodeKey::TitleBarAiToggle) => self.toggle_ai_panel(),
            Some(NodeKey::TitleBarNotifications) => self.toggle_notifications(),
            Some(NodeKey::TitleBarDesktop) => self.open_desktop_manager(),
            Some(NodeKey::TitleBarMarketplace) => self.open_marketplace(),
            Some(NodeKey::TitleBarTemplates) => {
                if self.commit_title() && self.commit_table_cell() {
                    self.open_auxiliary_page(WorkspaceView::Templates);
                }
            }
            Some(NodeKey::TitleBarAutomations) => {
                if self.commit_title() && self.commit_table_cell() {
                    self.workflows_open();
                }
            }
            Some(NodeKey::TitleBar) | None => {
                // 三个窗口按钮画在标题栏最右侧；WM_NCHITTEST 已经把它们放行成 HTCLIENT
                let viewport = self.renderer.viewport();
                self.caption_action = platform::caption_buttons(viewport)
                    .iter()
                    .position(|b| b.contains(x, y))
                    .map(|i| match i {
                        0 => CaptionAction::Minimize,
                        1 => CaptionAction::ToggleMaximize,
                        _ => CaptionAction::Close,
                    });
            }
            Some(NodeKey::NavigationResize) => {
                let edge = chrome.tree.rect(chrome.navigation).right;
                self.drag = Some(Drag {
                    target: DragTarget::Navigation,
                    grab_offset: x - edge,
                });
            }
            Some(NodeKey::SidebarResize) => {
                let edge = chrome.tree.rect(chrome.sidebar).right;
                self.drag = Some(Drag {
                    target: DragTarget::Sidebar,
                    grab_offset: x - edge,
                });
            }
            Some(NodeKey::RightSidebarResize) => {
                let edge = chrome.tree.rect(chrome.right_sidebar).left;
                self.drag = Some(Drag {
                    target: DragTarget::AiPanel,
                    grab_offset: x - edge,
                });
            }
            Some(NodeKey::RightSidebarToolbar) => {
                if let Some(panel) = chrome.toolbar_button_hit(&self.state, x, y) {
                    self.set_right_panel(panel);
                }
            }
            Some(NodeKey::StatusBar) => self.on_status_click(x, y),
            Some(NodeKey::RightSidebarBody) if self.state.right_panel != RightPanel::Outline => {
                self.on_right_panel_click(x, y);
            }
            Some(NodeKey::RightSidebarBody) if self.state.right_panel == RightPanel::Outline => {
                self.on_outline_click(x, y);
            }
            Some(NodeKey::Editor) if self.content() == MainContent::Source => {
                let (area, scroll) = (self.editor_area, self.shell.active_scroll());
                if !area.contains(x, y) {
                    return;
                }
                let shift = shift_down();
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    self.source.click(area, buffer, scroll, x, y, shift);
                }
                self.focus = Focus::Main;
                self.drag = Some(Drag {
                    target: DragTarget::EditorSelect,
                    grab_offset: 0.0,
                });
            }
            Some(NodeKey::Editor) if self.content() == MainContent::Document => {
                if self.outline_toggle_rect.contains(x, y) {
                    self.toggle_file_area_outline();
                    return;
                }
                if self.outline_area.contains(x, y) {
                    self.on_outline_click(x, y);
                    return;
                }
                if let Some(hit) = self.links_layout.hit(x, y) {
                    self.on_link_hit(hit);
                    return;
                }
                let (area, scroll) = (self.editor_area, self.shell.active_scroll());
                let cell = self.shell.active().and_then(|t| t.buffer()).and_then(|b| {
                    self.doc
                        .table_cells(area, b.text(), scroll)
                        .into_iter()
                        .find(|c| c.rect.contains(x, y))
                });
                if !area.contains(x, y) {
                    return;
                }
                if self.doc.title_rect(area, scroll).contains(x, y) {
                    self.begin_title_edit();
                    return;
                }
                if let Some(cell) = cell {
                    self.begin_table_cell(cell, x);
                    return;
                }
                if !crate::ui::editor_preferences::current().live_line_source {
                    let formula = self
                        .shell
                        .active()
                        .and_then(|t| t.buffer())
                        .and_then(|b| self.doc.math_at(area, b.text(), scroll, x, y));
                    if let Some((range, tex, display)) = formula {
                        if let (Some(path), Some(buffer)) = (
                            self.active_file_path(),
                            self.shell.active().and_then(|t| t.buffer()),
                        ) {
                            let action = DialogAction::EditEditorMath {
                                path,
                                range: (range.start, range.end),
                                original: buffer.text()[range].into(),
                                display,
                            };
                            let field = TextField::formula(&tex);
                            self.dialog = Some(Dialog {
                                title: "编辑公式".into(),
                                description: "编辑源码，实时查看公式效果".into(),
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
                                        action,
                                    },
                                ],
                                dismiss: DialogAction::Dismiss,
                                hover: None,
                            });
                            self.focus = Focus::Dialog;
                            return;
                        }
                    }
                }
                if let Some((start, toggle)) = self.doc.container_at(area, scroll, x, y) {
                    if toggle {
                        self.container_action(start, crate::ui::containers::Action::Toggle);
                    } else {
                        self.open_container_menu(start, x, y);
                    }
                    return;
                }
                // 点在渲染态的链接上：跳转，光标不动（TSX 的 handleClick 返回 true 阻止默认行为）
                let link = self
                    .shell
                    .active()
                    .and_then(|t| t.buffer())
                    .and_then(|b| self.doc.link_at(area, b.text(), scroll, x, y));
                if let Some(target) = link {
                    self.open_link(&target);
                    return;
                }
                // 代码块头部的复制按钮：整段代码进剪贴板，光标不动
                let code = self
                    .shell
                    .active()
                    .and_then(|t| t.buffer())
                    .and_then(|b| self.doc.code_copy_at(area, b.text(), scroll, x, y));
                if let Some(code) = code {
                    let copied = platform::copy_to_clipboard(&code);
                    self.show_global_notice(if copied {
                        "已复制代码"
                    } else {
                        "复制失败，请重试"
                    });
                    return;
                }
                if let Some(hit) = self.doc.code_header_at(area, scroll, x, y) {
                    match hit {
                        code_blocks::Hit::Collapse(start) => {
                            if let Some(buffer) = self.shell.active_buffer_mut() {
                                self.doc.toggle_code(buffer, start);
                                self.after_doc_edit(false);
                            }
                            self.editor_engaged = false;
                        }
                        code_blocks::Hit::Language(start) => {
                            let mut languages = code_blocks::LANGUAGES.to_vec();
                            const COMMON: &[&str] = &[
                                "plaintext",
                                "javascript",
                                "typescript",
                                "python",
                                "rust",
                                "java",
                                "go",
                                "html",
                                "css",
                                "json",
                                "markdown",
                                "sql",
                                "bash",
                            ];
                            languages.sort_by_key(|language| {
                                (
                                    COMMON
                                        .iter()
                                        .position(|common| common == language)
                                        .unwrap_or(COMMON.len()),
                                    language.to_string(),
                                )
                            });
                            let items = languages
                                .into_iter()
                                .map(|lang| {
                                    MenuItem::new(
                                        lang,
                                        MenuAction::CodeLanguage(start, lang.into()),
                                    )
                                })
                                .collect();
                            self.menu = Some(Menu::open_searchable(
                                items,
                                x,
                                y,
                                self.renderer.viewport(),
                                "搜索代码语言…",
                            ));
                        }
                        code_blocks::Hit::Title(start) => {
                            let mut field = TextField::new("代码块名称");
                            field.set_text(&self.doc.code_title(start));
                            self.dialog = Some(Dialog {
                                title: "代码块名称".into(),
                                description: String::new(),
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
                                        action: DialogAction::CodeTitle(start),
                                    },
                                ],
                                dismiss: DialogAction::Dismiss,
                                hover: None,
                            });
                            self.focus = Focus::Dialog;
                        }
                        code_blocks::Hit::Copy(_) => {}
                    }
                    return;
                }
                // 任务勾选框：切换完成态，不移光标
                let toggled = self
                    .shell
                    .active_buffer_mut()
                    .map(|b| self.doc.toggle_task_at(area, b, scroll, x, y))
                    .unwrap_or(false);
                if toggled {
                    self.after_doc_edit(false);
                    return;
                }
                let shift = shift_down();
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    self.doc.click(area, buffer, scroll, x, y, shift);
                }
                self.focus = Focus::Main;
                self.editor_engaged = true;
                self.drag = Some(Drag {
                    target: DragTarget::EditorSelect,
                    grab_offset: 0.0,
                });
                // 落点换了块就要重排（活动块露出源码）
                self.after_doc_edit(false);
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Schedule => {
                self.on_schedule_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Inbox => {
                self.on_inbox_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Recent => {
                self.on_recent_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Templates => {
                self.on_templates_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Marketplace => {
                self.marketplace_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Home => {
                self.on_home_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::Plugin => {
                if self.plugin_workspace.as_deref() == Some("quick-navigation") {
                    self.on_quick_navigation_click(x, y)
                } else if self.plugin_workspace.as_deref() == Some("english-lab") {
                    self.on_english_lab_click(x, y)
                }
            }
            Some(NodeKey::Sidebar) if self.state.view == WorkspaceView::MochiAi => {
                self.on_ai_workspace_sidebar_click(x, y)
            }
            Some(NodeKey::Editor) if self.state.view == WorkspaceView::MochiAi => {
                self.on_assistant_click(HWND(self.hwnd_raw as *mut _), x, y)
            }
            Some(NodeKey::Sidebar) if self.state.view == WorkspaceView::Schedule => {
                self.on_schedule_sidebar_click(x, y)
            }
            Some(NodeKey::Editor) if self.settings_tab().is_some() => {
                self.on_settings_content_click(x, y)
            }
            Some(NodeKey::Sidebar) if self.settings_tab().is_some() => {
                self.on_settings_nav_click(x, y)
            }
            Some(NodeKey::Editor) | Some(NodeKey::Sidebar)
                if self.agent_config_section().is_some() =>
            {
                self.on_agent_config_click(x, y)
            }
            Some(NodeKey::Editor) if self.viewer_tab().is_some() => self.on_viewer_click(x, y),
            Some(NodeKey::TabBar) => {
                let tabs = self.tab_projection();
                let rect = chrome.tree.rect(chrome.tab_bar);
                if let Some(direction) = tab_bar::navigation_hit(rect, x, y) {
                    self.navigate_tab(match direction {
                        tab_bar::Navigation::Back => NavigationDirection::Back,
                        tab_bar::Navigation::Forward => NavigationDirection::Forward,
                    });
                    return;
                }
                if tab_bar::split_button_rect(rect).contains(x, y) {
                    if let Some(path) = self.active_file_path() {
                        self.split_to_right(path);
                    }
                    return;
                }
                match tab_bar::hit_scrolled_with_fixed_width(
                    tab_bar::tabs_content_area(rect),
                    &tabs,
                    self.state.compact_tab_bar,
                    self.tab_fixed_width(),
                    self.tab_scroll,
                    x,
                    y,
                ) {
                    Some(tab_bar::Hit::Select(i)) => {
                        self.shell.select_tab(i);
                        self.remember_split_active();
                    }
                    Some(tab_bar::Hit::Close(i)) => self.shell.close_tab(i),
                    None => {}
                }
                // 换了标签就要重排：内容不同了
                self.invalidate_main();
                self.sync_state();
            }
            Some(NodeKey::Sidebar) => {
                if let Some((files, heads)) = self.outline_tabs {
                    if files.contains(x, y) {
                        self.outline_left_active = false;
                        return;
                    }
                    if heads.contains(x, y) {
                        self.outline_left_active = true;
                        return;
                    }
                    if self.outline_left_active {
                        self.on_outline_click(x, y);
                        return;
                    }
                }
                self.on_sidebar_click(x, y);
            }
            Some(NodeKey::Navigation) => self.on_navigation_click(x, y),
            _ => {}
        }
    }

    /// 双击。`WM_LBUTTONDBLCLK` 之前系统已经发过一次 `WM_LBUTTONDOWN`，
    /// 单击该做的事（开文件、切展开）已经做完，这里只处理**只有双击才做**的：
    /// 侧栏拖动手柄双击复位宽度（`onDoubleClick={() => setSidebarWidth(260)}`）。
    pub fn on_double_click(&mut self, x: f32, y: f32) {
        if self.global_import.is_some() {
            return;
        }
        let desktop_capture = self.desktop_manager_captures_pointer(x, y);
        let desktop_area = self.desktop_manager_area();
        if let Some(p) = self.desktop.panel.as_mut().filter(|_| desktop_capture) {
            let layout = p.layout(desktop_area);
            p.double_click(&layout, x, y);
            return;
        }
        if self.workflows_active() && self.workflows.view.area.contains(x, y) {
            if self.workflows.view.field_rect.contains(x, y)
                && self.workflows.view.editor != Some(crate::ui::workflows::Editor::Result)
            {
                self.focus = Focus::Workflow;
                self.workflows.view.field.buffer.select_word();
            }
            return;
        }
        if self.notification_open() {
            return;
        }
        if self.automation.panel.is_some() {
            return;
        }
        // 第一次点击已经滚动过。第二次点到轨道条上不能再选中覆盖层下面的
        // 词/图片，也不能重置相邻分栏。
        if self.scrollbar_hit(x, y) {
            return;
        }
        if let Some(dialog) = self.object_picker.as_mut() {
            dialog.state.query.buffer.select_word();
            return;
        }
        if self.settings_overlay.is_some() {
            if self.menu.is_some() || self.dialog.is_some() {
                return;
            }
            self.on_click(x, y);
            if let Some(field) = self.focused_field_mut() {
                field.buffer.select_word();
            }
            return;
        }
        if self.image_preview.is_some() {
            return;
        }
        if self.content() == MainContent::Document && self.dialog.is_none() && self.menu.is_none() {
            if let Some(start) =
                self.doc
                    .image_at(self.editor_area, self.shell.active_scroll(), x, y)
            {
                self.image_preview = self.editor_image_source(start);
                self.drag = None;
                return;
            }
        }
        if self.export_form.is_some() {
            return;
        }
        if self.commands.review.is_some() {
            return;
        }
        if self.dialog.is_some() || self.menu.is_some() {
            return;
        }
        if self.canvas_double_click(x, y) {
            return;
        }
        let chrome = self.build_chrome();
        match chrome.hit(x, y) {
            Some(NodeKey::SidebarResize) => {
                self.state.sidebar_width = theme::tokens().layout.sidebar_width;
                self.persist_chrome_setting(
                    "sidebar.width",
                    SettingValue::Number(f64::from(self.state.sidebar_width)),
                );
                self.drag = None;
                self.invalidate_main();
            }
            // 编辑器里双击选词。前一次单击已经把光标放好了
            Some(NodeKey::Editor) if self.editing_file() => {
                self.drag = None;
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    buffer.select_word();
                }
                self.after_edit(true);
            }
            _ => {}
        }
    }

    /// AI 开关这类画在标题栏里的自绘按钮。`WM_NCHITTEST` 要据此放行，
    /// 否则点击会被 `HTCAPTION` 吞成拖动窗口。
    pub fn hit_is_titlebar_button(&mut self, x: f32, y: f32) -> bool {
        let chrome = self.build_chrome();
        self.notification_open()
            || matches!(
                chrome.hit(x, y),
                Some(
                    NodeKey::TitleBarBack
                        | NodeKey::TitleBarThemeToggle
                        | NodeKey::TitleBarAiToggle
                        | NodeKey::TitleBarNotifications
                        | NodeKey::TitleBarDesktop
                        | NodeKey::TitleBarMarketplace
                        | NodeKey::TitleBarTemplates
                        | NodeKey::TitleBarAutomations
                )
            )
    }
}
