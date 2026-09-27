//! 处理菜单输入，并将菜单项分派为具体操作。
use super::*;

impl App {
    /// 分派页面输入前，先匹配与帧绘制一致的覆盖层顺序。
    pub(super) fn menu_input_active(&self) -> bool {
        self.menu.is_some()
            && self.global_import.is_none()
            && !self.desktop_manager_modal()
            && !self.notification_open()
            && self.automation.panel.is_none()
            && self.template_picker.is_none()
            && self.object_picker.is_none()
            && self.table_picker.is_none()
            && self.image_preview.is_none()
            && self.export_form.is_none()
            && self.commands.review.is_none()
            && self.mapped_folder.is_none()
            && self.link_create.is_none()
            && self.dialog.is_none()
            && self.search.is_none()
            && self.command.is_none()
    }

    pub(super) fn menu_click(&mut self, x: f32, y: f32) {
        let Some(mut menu) = self.menu.take() else {
            return;
        };
        if menu.begin_scrollbar_drag(x, y) || menu.search_hit(x, y) {
            self.menu = Some(menu);
        } else if let Some((depth, index)) = menu.hit_path(x, y) {
            self.menu = Some(menu);
            self.activate_menu_item(depth, index);
        } else {
            self.toolbar_menu = None;
            self.slash_trigger = None;
        }
    }

    pub(super) fn menu_key(&mut self, key: u16, shift: bool, ctrl: bool) {
        if key == 0x1b {
            self.menu = None;
            self.toolbar_menu = None;
            self.slash_trigger = None;
            return;
        }
        if self.slash_trigger.is_some() && !ctrl {
            let index = match key {
                0x31..=0x39 => Some((key - 0x31) as usize),
                0x30 => Some(9),
                _ => None,
            };
            if let Some(index) = index {
                if self
                    .menu
                    .as_ref()
                    .and_then(|menu| menu.items.get(index))
                    .is_some_and(|item| !item.disabled && item.shortcut.is_some())
                {
                    self.activate_menu_item(0, index);
                }
                return;
            }
        }
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        if key == 38 || key == 40 {
            menu.move_selection(if key == 38 { -1 } else { 1 });
        } else if key == 13 {
            let mut depth = 0;
            let mut selected = &*menu;
            while let Some(child) = selected.submenu.as_deref() {
                selected = child;
                depth += 1;
            }
            if let Some(index) = selected.hover {
                self.activate_menu_item(depth, index);
            }
        } else if let Some(search) = menu.search_mut() {
            let before = search.text().to_owned();
            let _ = search.key(key, shift, ctrl);
            if before != search.text() {
                menu.search_changed();
            }
        }
    }

    pub(super) fn remove_slash_trigger(&mut self) {
        let Some((path, offset)) = self.slash_trigger.take() else {
            return;
        };
        if self.active_file_path().as_ref() != Some(&path) {
            return;
        }
        let removed = self
            .shell
            .active_buffer_mut()
            .is_some_and(|buffer| Self::remove_slash_at(buffer, offset));
        if removed {
            self.after_doc_edit(false);
        }
    }

    pub(super) fn activate_menu_item(&mut self, depth: usize, index: usize) {
        let action = self
            .menu
            .as_ref()
            .and_then(|menu| Self::menu_action_at(menu, depth, index))
            .filter(|item| !item.disabled)
            .map(|item| item.action.clone());
        if let Some(action) = action {
            if Self::menu_action_opens_submenu(&action) {
                self.menu_submenu_target = Some((depth, index));
                self.run_menu_action(action);
                self.menu_submenu_target = None;
                return;
            }
            self.menu = None;
            self.menu_submenu_target = None;
            self.toolbar_menu = None;
            self.remove_slash_trigger();
            self.run_menu_action(action);
        }
    }

    pub(super) fn menu_action_at<'a>(
        menu: &'a Menu<MenuAction>,
        depth: usize,
        index: usize,
    ) -> Option<&'a MenuItem<MenuAction>> {
        if depth == 0 {
            return menu.items.get(index);
        }
        menu.submenu
            .as_deref()
            .and_then(|submenu| Self::menu_action_at(submenu, depth - 1, index))
    }

    pub(super) fn menu_action_opens_submenu(action: &MenuAction) -> bool {
        match action {
            MenuAction::BlockParagraphMenu
            | MenuAction::BlockFormatMenu
            | MenuAction::BlockAiMenu
            | MenuAction::BlockColorMenu(_)
            | MenuAction::FileIcon(_)
            | MenuAction::SortFiles
            | MenuAction::AiPermission(_)
            | MenuAction::AiSession(
                _,
                ai_sessions::SessionAction::Projects | ai_sessions::SessionAction::Colors,
            )
            | MenuAction::AiMessage(_, _, assistant::MessageAction::Mount) => true,
            MenuAction::ExportFile(path) => path.extension().is_some_and(|ext| {
                ["md", "mc", "markdown", "txt"]
                    .iter()
                    .any(|candidate| ext.eq_ignore_ascii_case(candidate))
            }),
            _ => false,
        }
    }

    pub(super) fn open_menu_submenu(&mut self, items: Vec<MenuItem<MenuAction>>) {
        let Some((depth, index)) = self.menu_submenu_target else {
            return;
        };
        if let Some(menu) = self.menu.as_mut() {
            menu.open_submenu_at(depth, index, items, self.renderer.viewport());
        }
    }

    pub(super) fn open_searchable_menu_submenu(
        &mut self,
        items: Vec<MenuItem<MenuAction>>,
        placeholder: &str,
    ) {
        let Some((depth, index)) = self.menu_submenu_target else {
            return;
        };
        if let Some(menu) = self.menu.as_mut() {
            menu.open_searchable_submenu_at(
                depth,
                index,
                items,
                self.renderer.viewport(),
                placeholder,
            );
        }
    }

    pub(super) fn run_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::HideTitleBarEntry(key) => {
                if let Some(descriptor) = app_settings::descriptor(key) {
                    self.app_settings
                        .write(descriptor, &SettingValue::Bool(false));
                    if let Err(error) = self.app_settings.flush() {
                        self.state.status_text = format!("标题栏设置保存失败：{error}");
                    }
                    self.apply_setting_side_effects(key);
                }
            }
            MenuAction::HideNavigation(item) => {
                use crate::ui::navigation_preferences::{Action, Preferences};
                if !Preferences::read().hidden.contains(&item) {
                    self.edit_navigation_preferences(Action::Toggle(item));
                }
            }
            MenuAction::AiSession(id, action) => self.ai_session_action(&id, action),
            MenuAction::ToggleFavorite(path) => self.toggle_favorite(&path),
            MenuAction::AiMessage(sid, mid, action) => {
                self.ai_message_action(&sid, &mid, action, 800.0, 220.0)
            }
            MenuAction::MountAiMessage(sid, mid, path) => {
                self.ai_mount_message(&sid, &mid, &[path])
            }
            MenuAction::PickAiMessageDocuments(sid, mid) => {
                self.open_object_picker(object_picker_host::Purpose::Message(sid, mid))
            }
            MenuAction::SelectAgent(id) => self.ai_select_agent(&id),
            MenuAction::SetAgentFollowUpFrequency(freq) => {
                if let Some(ws) = self.shell.workspace() {
                    let agent_id = self
                        .ai
                        .panel
                        .selected_agent_id
                        .as_deref()
                        .unwrap_or(mochi_core::ai::agent_config::GENERAL_ASSISTANT_ID);
                    let svc = mochi_core::ai::agent_config::AgentConfigService::new(&ws.root);
                    if let Err(e) = svc.update_agent_frequency(agent_id, freq) {
                        self.show_global_notice(&format!("保存设置失败: {e}"));
                    } else {
                        self.ai.panel.follow_up_frequency = freq;
                        if freq == mochi_core::ai::FollowUpFrequency::Never {
                            self.cancel_follow_up_jobs();
                        }
                        self.invalidate_main();
                    }
                }
            }
            MenuAction::OpenAgentDefinitions => {
                self.agent.section = 0;
                self.state.view = WorkspaceView::AgentConfig;
                self.invalidate_main();
                self.sync_state();
            }
            MenuAction::MountAiSession(path) => self.ai_mount_session(&path),
            MenuAction::PickAiMountDocument => {
                if let Some(id) = self.ai.panel.active.as_ref().map(|c| c.id.clone()) {
                    self.open_object_picker(object_picker_host::Purpose::Session(id));
                }
            }
            MenuAction::LibraryScope(index, kind) => {
                if let Some(index) = index {
                    self.shell.select_library(index);
                } else {
                    self.shell.select_type(&kind);
                }
                self.side.search.clear();
                self.side.scroll = 0.0;
                self.state.view = WorkspaceView::Editor;
                self.invalidate_main();
                self.sync_state();
            }
            MenuAction::CloseTabs(index, mode) => {
                let closing = self
                    .shell
                    .tabs()
                    .iter()
                    .enumerate()
                    .filter(|(i, t)| {
                        !t.pinned
                            && match mode {
                                "others" => *i != index,
                                "right" => *i > index,
                                _ => true,
                            }
                    })
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                for i in closing.into_iter().rev() {
                    self.shell.close_tab(i);
                }
                self.invalidate_main();
                self.sync_state();
            }
            MenuAction::ExportDocument(source, format) => {
                self.export_form = Some(crate::ui::export_dialog::State::new(source, format));
                self.focus = Focus::Dialog;
            }
            MenuAction::SaveAsTemplate(path) => {
                let content = self
                    .shell
                    .tabs()
                    .iter()
                    .find(|tab| tab.path() == Some(path.as_path()))
                    .and_then(|tab| tab.buffer())
                    .map(|buffer| buffer.text().to_owned())
                    .or_else(|| std::fs::read_to_string(&path).ok());
                match content {
                    Some(content) => self.open_template_dialog(content),
                    None => self.show_global_notice("只能将文本类文档另存为模板"),
                }
            }
            MenuAction::FileIcon(path) => {
                let icons = crate::ui::icons::catalog();
                let mut items = vec![MenuItem::new(
                    "恢复默认图标",
                    MenuAction::SetFileIcon(path.clone(), None),
                )
                .icon(Icon::FILE)];
                if !icons.is_empty() {
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_nanos() as usize)
                        .unwrap_or(0);
                    let random = icons[nanos % icons.len()];
                    items.push(
                        MenuItem::new(
                            "随机选择",
                            MenuAction::SetFileIcon(path.clone(), Some(random.0.into())),
                        )
                        .icon(Icon::SPARKLES)
                        .separated(),
                    );
                }
                items.extend(icons.into_iter().map(|icon| {
                    MenuItem::new(
                        icon.0,
                        MenuAction::SetFileIcon(path.clone(), Some(icon.0.into())),
                    )
                    .icon(icon)
                }));
                self.open_searchable_menu_submenu(items, "搜索图标…");
            }
            MenuAction::SetFileIcon(path, value) => {
                self.save_file_icon(path, value);
            }
            MenuAction::SortFiles => {
                let items = [
                    ("自定义（拖拽顺序）", "manual"),
                    ("名称（升序）", "name"),
                    ("名称（降序）", "name-desc"),
                ]
                .into_iter()
                .map(|(name, value)| MenuItem::new(name, MenuAction::SetSort(value.into())))
                .collect();
                self.open_menu_submenu(items);
            }
            MenuAction::SetSort(mode) => {
                self.shell.set_sort_mode(&mode);
                self.persist_chrome_setting("sidebar.sortOrder", SettingValue::Text(mode));
                self.apply_setting_side_effects("sidebar.sortOrder");
            }
            MenuAction::ImportFiles(parent) => {
                let parent = if parent.as_os_str().is_empty() {
                    self.shell.tree_root()
                } else {
                    Some(parent)
                };
                if let (Some(parent), Some(source)) =
                    (parent, platform::pick_file(HWND(self.hwnd_raw as *mut _)))
                {
                    self.file_jobs
                        .submit(parent.clone(), self.hwnd_raw, move || {
                            let target = mochi_core::link_files::import_file(&source, &parent)?;
                            Ok(crate::file_runtime::Payload::Created(target))
                        });
                }
            }
            MenuAction::ExportFile(source) => {
                if source.extension().is_some_and(|ext| {
                    ["md", "mc", "markdown", "txt"]
                        .iter()
                        .any(|e| ext.eq_ignore_ascii_case(e))
                }) {
                    let items = [("PDF", "pdf"), ("HTML", "html"), ("Markdown", "markdown")]
                        .into_iter()
                        .map(|(label, format)| {
                            MenuItem::new(
                                label,
                                MenuAction::ExportDocument(source.clone(), format.into()),
                            )
                        })
                        .collect();
                    self.open_menu_submenu(items);
                    return;
                }
                if let Some(target) = platform::save_file(
                    HWND(self.hwnd_raw as *mut _),
                    &source.file_name().unwrap_or_default().to_string_lossy(),
                ) {
                    if source != target {
                        if let Err(e) = std::fs::copy(source, target) {
                            self.state.status_text = e.to_string();
                        }
                    }
                }
            }
            MenuAction::NewSubdocument(parent) => {
                self.dialog = Some(Dialog {
                    title: "新建子文档".into(),
                    description: parent
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    field: Some(TextField::new("子文档名称")),
                    error: String::new(),
                    note: None,
                    buttons: vec![
                        DialogButton {
                            label: "取消".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::Dismiss,
                        },
                        DialogButton {
                            label: "创建".into(),
                            kind: ButtonKind::Primary,
                            action: DialogAction::CreateSubdocument(parent),
                        },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
            }
            MenuAction::Properties(path) => {
                let description = match std::fs::metadata(&path) {
                    Ok(m) => format!(
                        "{}\n大小：{}\n类型：{}",
                        path.display(),
                        viewer::format_size(m.len()),
                        if m.is_dir() { "文件夹" } else { "文件" }
                    ),
                    Err(e) => e.to_string(),
                };
                self.dialog = Some(Dialog {
                    title: "属性".into(),
                    description,
                    field: None,
                    error: String::new(),
                    note: None,
                    buttons: vec![DialogButton {
                        label: "关闭".into(),
                        kind: ButtonKind::Primary,
                        action: DialogAction::Dismiss,
                    }],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
            }
            MenuAction::AiPermission(path) => {
                let items = [
                    ("继承父级", "inherit"),
                    ("不可见", "invisible"),
                    ("只读", "readonly"),
                    ("建议修改（需批准）", "suggest"),
                    ("允许修改", "modify"),
                ]
                .into_iter()
                .map(|(label, level)| {
                    MenuItem::new(
                        label,
                        MenuAction::SetAiPermission(path.clone(), level.into()),
                    )
                })
                .collect();
                self.open_menu_submenu(items);
            }
            MenuAction::SetAiPermission(path, level) => {
                if let Some(svc) = &self.ai.permissions {
                    let result = if level == "inherit" {
                        svc.remove_folder_permission(&path.to_string_lossy())
                    } else {
                        svc.set_folder_permission(
                            &path.to_string_lossy(),
                            mochi_core::ai::permission::AiPermissionLevel::from_wire(&level),
                        )
                    };
                    self.state.status_text = match result {
                        Ok(()) => "AI 权限已更新".into(),
                        Err(e) => e.to_string(),
                    };
                }
            }
            MenuAction::FileToAssistant(path) => {
                self.add_ai_attachments(vec![path]);
                self.state.ai_panel_open = true;
                self.set_right_panel(RightPanel::Assistant);
                self.focus = Focus::AiInput;
            }
            MenuAction::BaseCellToAssistant(path, context) => {
                let content = context.content;
                let selection = assistant::context::SelectionContext {
                    path: path.to_string_lossy().replace('\\', "/"),
                    title: Some(context.title),
                    start_line: 1,
                    end_line: 1,
                    from: 0,
                    to: content.len(),
                    text: content.clone(),
                    original_hash: assistant::context::hash_text(&content),
                    blocks: Vec::new(),
                }
                .to_value();
                assistant::context::add_pending_selection(
                    &mut self.ai.panel.pending_selections,
                    selection,
                );
                self.state.ai_panel_open = true;
                self.set_right_panel(RightPanel::Assistant);
                self.focus = Focus::AiInput;
            }
            MenuAction::BaseCellInsert(record, field, direction) => {
                let inserted = match self.viewer_content_mut() {
                    Some(viewer::Content::Base(state)) => {
                        state.insert_next_to_cell(record, field, direction)
                    }
                    _ => false,
                };
                if inserted {
                    self.schedule_autosave();
                    self.state.status_text = "已在单元格旁新增内容".into();
                }
                self.sync_state();
            }
            MenuAction::SelectionAi(template, append) => {
                if let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) {
                    let text = buffer.selected_text();
                    let language = if text.chars().any(|c| ('\u{4e00}'..='\u{9fa5}').contains(&c)) {
                        "英文"
                    } else {
                        "中文"
                    };
                    self.start_selection_ai(
                        template
                            .replace("{{selection}}", text)
                            .replace("{{targetLanguage}}", language),
                        append,
                    );
                }
            }
            MenuAction::SelectionToAssistant => {
                if let Some(selection) = self.ai_current_selection() {
                    assistant::context::add_pending_selection(
                        &mut self.ai.panel.pending_selections,
                        selection,
                    );
                }
                self.state.ai_panel_open = true;
                self.set_right_panel(RightPanel::Assistant);
                self.focus = Focus::AiInput;
            }
            MenuAction::SplitRight(path) => self.split_to_right(path),
            MenuAction::PinTab(i) => {
                if let Some(tab) = self.shell.tabs_mut().get_mut(i) {
                    tab.pinned = !tab.pinned;
                }
            }
            MenuAction::CloseTab(i) => {
                self.shell.close_tab(i);
                self.invalidate_main();
                self.sync_state();
            }
            MenuAction::NewLink(parent) => self.open_link_dialog(parent),
            MenuAction::NewBase(parent) => {
                let parent = if parent.as_os_str().is_empty() {
                    self.shell.tree_root()
                } else {
                    Some(parent)
                };
                if let Some(parent) = parent {
                    self.open_base_dialog(parent);
                }
            }
            MenuAction::NewCanvas(parent) => {
                let parent = if parent.as_os_str().is_empty() {
                    self.shell.tree_root()
                } else {
                    Some(parent)
                };
                if let Some(parent) = parent {
                    self.create_default_canvas(parent);
                }
            }
            MenuAction::AgendaPick(slot, value) => self.agenda_pick(slot, value),
            MenuAction::NewFile(parent) => {
                let parent = if parent.as_os_str().is_empty() {
                    self.shell.tree_root()
                } else {
                    Some(parent)
                };
                if let Some(parent) = parent {
                    self.open_template_picker(parent);
                }
            }
            MenuAction::NewFolder(parent) if parent.as_os_str().is_empty() => {
                let Some(root) = self.shell.tree_root() else {
                    return;
                };
                self.open_create_dialog(root, true);
            }
            MenuAction::NewFolder(parent) => {
                self.start_sidebar_edit(EditKind::NewFolder { parent }, "")
            }
            MenuAction::NewMappedFolder(parent) => {
                let parent = if parent.as_os_str().is_empty() {
                    self.shell.tree_root()
                } else {
                    Some(parent)
                };
                let Some(parent) = parent else {
                    return;
                };
                let Some(library) = self.shell.library_root_for_path(&parent) else {
                    self.show_global_notice("请在一个具体知识库内新增映射文件夹");
                    return;
                };
                self.open_mapped_folder_dialog(library);
            }
            MenuAction::DuplicateFile(source) => match self.shell.duplicate_file(&source) {
                Ok(copy) => {
                    self.show_global_notice(format!(
                        "已复制为副本：{}",
                        copy.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    self.sync_state();
                }
                Err(error) => self.show_global_notice(format!("复制副本失败：{error}")),
            },
            MenuAction::RenamePath(path) => self.open_rename_dialog(path),
            MenuAction::DeletePath(path) => self.open_delete_dialog(path),
            MenuAction::Refresh => {
                self.shell.refresh_tree();
                self.sync_state();
            }
            MenuAction::RefreshTab(path) => {
                if path.is_file() {
                    self.shell.reload_file(&path);
                    self.invalidate_main();
                    self.sync_state();
                }
            }
            MenuAction::Rename(row) => {
                let Some(name) = self.shell.rows().get(row).map(|r| r.name.clone()) else {
                    return;
                };
                self.start_sidebar_edit(EditKind::Rename { row }, &name);
            }
            MenuAction::Delete(row) => {
                let Some(path) = self.shell.rows().get(row).map(|row| row.path.clone()) else {
                    return;
                };
                self.open_delete_dialog(path);
            }
            MenuAction::CopyPath(p) => {
                let copied = platform::copy_to_clipboard(&p.to_string_lossy());
                self.show_global_notice(if copied {
                    "已复制路径"
                } else {
                    "复制失败，请重试"
                });
            }
            MenuAction::CopyRelativePath(path) => {
                let relative = self
                    .shell
                    .workspace()
                    .and_then(|workspace| path.strip_prefix(&workspace.root).ok())
                    .map(|relative| relative.to_string_lossy().replace('\\', "/"));
                let copied = relative.as_deref().is_some_and(platform::copy_to_clipboard);
                self.show_global_notice(if copied {
                    "已复制相对路径"
                } else {
                    "无法复制相对路径：文件不在当前工作区内"
                });
            }
            MenuAction::CopyMochiUrl(p, is_dir) => {
                let kind = if is_dir {
                    ResourceKind::Directory
                } else {
                    ResourceKind::File
                };
                let root = self.shell.workspace().map(|ws| ws.root.clone());
                let copied = platform::copy_to_clipboard(&build_mochi_resource_url(
                    &p,
                    kind,
                    root.as_deref(),
                ));
                self.show_global_notice(if copied {
                    "已复制 Mochi 链接"
                } else {
                    "复制失败，请重试"
                });
            }
            MenuAction::ObjectPresentation(start, text) => {
                self.set_object_presentation(start, text)
            }
            MenuAction::CopyObjectLink(url) => {
                let copied = platform::copy_to_clipboard(&url);
                self.show_global_notice(if copied {
                    "已复制"
                } else {
                    "复制失败，请重试"
                });
            }
            MenuAction::InsertObject => {
                self.open_object_picker(object_picker_host::Purpose::Insert)
            }
            MenuAction::ShowInExplorer(p) => platform::show_in_explorer(&p),
            MenuAction::ShowInFileTree(path) => self.show_in_file_tree(&path),
            MenuAction::PickInboxLibrary(i) => self.views.inbox.target_library = i,
            MenuAction::SetEnum(index, value) => {
                if let Some(d) = app_settings::descriptors().get(index) {
                    if d.key == "ai.editApplyMode" && value == "auto" {
                        self.dialog = Some(Dialog {
                            title: "允许 AI 自动应用修改？".into(),
                            description: "允许 AI 直接修改有写权限的文件。".into(),
                            field: None,
                            error: String::new(),
                            note: Some(
                                "“建议”路径也会直接应用；只读和不可见路径仍受保护。\n可随时切回“批准”模式。"
                                    .into(),
                            ),
                            buttons: vec![
                                DialogButton {
                                    label: "取消".into(),
                                    kind: ButtonKind::Ghost,
                                    action: DialogAction::Dismiss,
                                },
                                DialogButton {
                                    label: "允许自动应用".into(),
                                    kind: ButtonKind::Danger,
                                    action: DialogAction::EnableAutomaticAiEdits,
                                },
                            ],
                            dismiss: DialogAction::Dismiss,
                            hover: None,
                        });
                        self.focus = Focus::Dialog;
                        return;
                    }
                    self.app_settings.write(d, &SettingValue::Text(value));
                    let _ = self.app_settings.flush();
                    let key = d.key.clone();
                    self.apply_setting_side_effects(&key);
                }
            }
            MenuAction::CodeLanguage(start, lang) => {
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    if let Some(range) = code_blocks::language_range(buffer.text(), start) {
                        buffer.replace_range(range, if lang == "plaintext" { "" } else { &lang });
                        self.after_doc_edit(false);
                        self.editor_engaged = false;
                    }
                }
            }
            MenuAction::OpenTab(p) => {
                if self.open_file_from_ui(&p) {
                    self.invalidate_main();
                    self.sync_state();
                }
            }
            MenuAction::Format(f) => self.apply_format(f),
            MenuAction::SelectProviderModel(provider_id, model) => {
                if let Some(form) = self
                    .prefs
                    .providers
                    .form
                    .as_mut()
                    .filter(|form| form.id == provider_id)
                {
                    form.fields[2].set_text(&model);
                    form.focus = 2;
                    self.prefs.providers.status = format!("已选择模型：{model}");
                }
            }
            MenuAction::SelectProviderProtocol(provider_id, protocol) => {
                // 模型列表或连接测试都属于上一个协议。应用本次修改前先取消；
                // 自定义端点可以在协议变化时保持不变，所以模型响应守卫里的
                // 端点检查不足以拦下过期结果。
                self.cancel_provider_models();
                self.cancel_provider_test();
                if let Some(form) = self
                    .prefs
                    .providers
                    .form
                    .as_mut()
                    .filter(|form| form.id == provider_id)
                {
                    if form.apply_protocol(&protocol) {
                        self.prefs.providers.status = format!(
                            "已选择协议：{}；请确认 API Key 后保存",
                            providers::protocol_label(&protocol)
                        );
                        self.focus = Focus::ProviderField;
                    }
                }
            }
            MenuAction::SelectProviderPreset(provider_id, index) => {
                if self
                    .prefs
                    .providers
                    .form
                    .as_ref()
                    .is_some_and(|form| form.id == provider_id)
                {
                    self.cancel_provider_models();
                    self.cancel_provider_test();
                    if self
                        .prefs
                        .providers
                        .form
                        .as_mut()
                        .unwrap()
                        .apply_preset(index)
                    {
                        self.prefs.providers.status = if index.is_some() {
                            "已填入预设，请确认模型和 API Key 后保存；更换地址会清空原密钥".into()
                        } else {
                            "自定义配置：保留当前内容，可自由修改".into()
                        };
                        self.focus = Focus::ProviderField;
                    }
                }
            }
            MenuAction::WorkflowProvider(node_id, value) => {
                let v = &self.workflows.view;
                if v.run.is_none()
                    && v.selected_node
                        .and_then(|i| v.draft.as_ref()?.nodes.get(i))
                        .is_some_and(|n| n.id == node_id)
                {
                    let result = self.workflows.view.edit_parameter("/config/provider_id");
                    match result {
                        Ok(()) => {
                            if let Some(value) = value {
                                self.workflows.view.field.set_text(&value);
                                if let Err(error) = self.workflows.view.apply_node() {
                                    self.workflows.view.error = error;
                                }
                            }
                            self.focus = Focus::Workflow;
                        }
                        Err(error) => self.workflows.view.error = error,
                    }
                }
            }
            MenuAction::WorkflowReference(node_id, path, reference) => {
                let v = &mut self.workflows.view;
                if v.run.is_none()
                    && v.editor == Some(crate::ui::workflows::Editor::Parameter)
                    && v.parameter == path
                    && v.selected_node
                        .and_then(|i| v.draft.as_ref()?.nodes.get(i))
                        .is_some_and(|n| n.id == node_id)
                {
                    v.field.buffer.insert(&format!("{{{{ {reference} }}}}"));
                    self.focus = Focus::Workflow;
                }
            }
            MenuAction::WorkflowAction(hit) => {
                if let Err(error) = self.workflows_action(hit) {
                    self.workflows.view.error = error;
                }
            }
            MenuAction::InsertMath => self.insert_editor_math(),
            MenuAction::ImageWidth(start) => self.open_image_width(start),
            MenuAction::PreviewEditorImage(start) => {
                self.image_preview = self.editor_image_source(start);
            }
            MenuAction::CopyEditorImage(start) => self.copy_editor_image(start),
            MenuAction::CommentSelection => self.begin_selection_comment(),
            MenuAction::BlockParagraphMenu => self.open_block_paragraph_menu(),
            MenuAction::BlockFormatMenu => self.open_block_format_menu(),
            MenuAction::BlockAiMenu => self.open_block_ai_menu(),
            MenuAction::BlockAlign(alignment) => self.apply_block_alignment(alignment),
            MenuAction::BlockIndent(outdent) => self.apply_block_indent(outdent),
            MenuAction::BlockColorMenu(highlight) => self.open_block_color_menu(highlight),
            MenuAction::Block(start, after) => {
                if self.commit_table_cell() {
                    self.edit_block(start, after);
                }
            }
            MenuAction::Container(start, action) => self.container_action(start, action),
            MenuAction::Table(at, action) => {
                if !self.commit_table_cell() {
                    return;
                }
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    if table_edit::apply(buffer, at, action) {
                        self.after_doc_edit(true);
                        self.focus = Focus::Main;
                        self.editor_engaged = action == table_edit::Action::ParagraphAfter;
                    }
                }
            }
            MenuAction::EditorCopy => {
                self.clipboard_copy(false);
            }
            MenuAction::EditorCut => {
                self.clipboard_copy(true);
            }
            MenuAction::EditorPaste => {
                if self.focus != Focus::TableCell {
                    self.focus = Focus::Main;
                    self.editor_engaged = true;
                }
                self.clipboard_paste();
            }
            MenuAction::EditorPastePlain => {
                if self.focus != Focus::TableCell {
                    self.focus = Focus::Main;
                    self.editor_engaged = true;
                }
                self.clipboard_paste_mode(true);
            }
            MenuAction::EditorSelectAll => {
                let rendered_document = self.content() == MainContent::Document;
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    if rendered_document {
                        self.doc.select_current_block_or_all(buffer);
                    } else {
                        buffer.select_all();
                    }
                    self.focus = Focus::Main;
                    self.editor_engaged = true;
                    self.after_doc_selection_change(true);
                }
            }
            MenuAction::TextColor(highlight, color) => {
                self.settings.set(
                    if highlight {
                        "mochi:last-highlight-color"
                    } else {
                        "mochi:last-text-color"
                    },
                    &format!("#{color:06X}"),
                );
                let _ = self.settings.flush();
                self.apply_text_color(highlight, color);
            }
            MenuAction::SetColorSetting(index, value) => {
                if let Some(descriptor) = app_settings::descriptors().get(index) {
                    self.app_settings
                        .write(descriptor, &SettingValue::Text(value));
                    let _ = self.app_settings.flush();
                    self.apply_setting_side_effects(&descriptor.key);
                }
            }
        }
    }
}
