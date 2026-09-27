//! 根据按键处理编辑器输入和编辑命令。
use super::*;

impl App {
    /// 编辑相关的按键。返回是否消费掉了。
    pub fn on_edit_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        self.window_motion.keyboard = true;
        if self.global_import.is_some() {
            return self.import_picker_key(key, shift, ctrl, false);
        }
        if self.menu_input_active() {
            self.menu_key(key, shift, ctrl);
            return true;
        }
        if key == 0x1b && self.cancel_navigation_library_drag() {
            return true;
        }
        if self.notification_open() {
            return self.notification_key(key, shift, ctrl);
        }
        if self.automation.panel.is_some() && self.dialog.is_none() {
            return self.automation_key(key, shift);
        }
        if key == 0x1b && self.cancel_sidebar_tree_drag() {
            return true;
        }
        if self.template_picker.is_some() {
            if key == 0x1b {
                self.template_picker = None;
                self.focus = Focus::Main;
            } else if key == 0x09 && !ctrl {
                let viewport = self.renderer.viewport();
                self.template_picker
                    .as_mut()
                    .unwrap()
                    .navigate(viewport, shift);
            } else if matches!(key, 0x0d | 0x20) && !ctrl {
                if let Some(hit) = self
                    .template_picker
                    .as_ref()
                    .and_then(|picker| picker.focused)
                {
                    self.activate_template_picker(hit);
                }
            }
            return true;
        }
        if self.object_picker.is_some() {
            return self.object_picker_key(key, shift, ctrl);
        }
        if self.image_preview.is_some() {
            if key == 0x1b {
                self.image_preview = None;
            }
            return true;
        }
        if self.marketplace_key(key, shift) {
            return true;
        }
        if !ctrl && self.wiki_key(key) {
            return true;
        }
        if let Some(picker) = self.table_picker.as_mut() {
            match key {
                0x1b => {
                    self.table_picker = None;
                }
                0x0d => {
                    let (rows, cols) = (picker.rows, picker.cols);
                    self.table_picker = None;
                    self.insert_editor_table(rows, cols);
                }
                0x25 => picker.cols = picker.cols.saturating_sub(1).max(1),
                0x27 => picker.cols = (picker.cols + 1).min(10),
                0x26 => picker.rows = picker.rows.saturating_sub(1).max(1),
                0x28 => picker.rows = (picker.rows + 1).min(10),
                _ => {}
            }
            return true;
        }
        if let Some(form) = self.export_form.as_mut() {
            let action = form.key(key, shift);
            self.export_form_action(action);
            return true;
        }
        if self.commands.review.is_some() {
            return self.command_review_key(key, ctrl);
        }
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
        if self.settings_overlay.is_some() && key == VK_ESCAPE.0 {
            if self.menu.take().is_none() {
                self.close_settings();
            }
            return true;
        }
        if self.focus == Focus::Main && self.dialog.is_none() && key == VK_ESCAPE.0 {
            if let Some(viewer::Content::Base(s)) = self.viewer_content_mut() {
                if s.close_popup() {
                    return true;
                }
            }
        }
        if self.focus == Focus::DocumentTitle {
            let Some(edit) = self.title_editing.as_mut() else {
                return false;
            };
            if ctrl && (key == 0x5a || key == 0x59) {
                if key == 0x59 || shift {
                    edit.field.buffer.redo();
                } else {
                    edit.field.buffer.undo();
                }
                return true;
            }
            return match edit.field.key(key, shift, ctrl) {
                FieldKey::Submit => {
                    self.commit_title();
                    true
                }
                FieldKey::Cancel => {
                    self.title_editing = None;
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Edited => true,
                FieldKey::Ignored => false,
            };
        }
        if self.focus == Focus::Main && self.editor_ai.ghost.is_some() {
            if key == 9 && !ctrl && !shift {
                return self.accept_prediction(0);
            }
            if key == VK_ESCAPE.0 {
                self.editor_ai.invalidate();
                return true;
            }
        }
        if self.focus == Focus::Main
            && self.dialog.is_none()
            && matches!(self.viewer_tab(), Some((_, viewer::Content::Pdf(_))))
        {
            if (key == 0x2e || key == 0x08)
                && crate::ui::settings_values::boolean("editorLayout.annotationsVisible", true)
            {
                self.pdf_delete_selected();
                return true;
            }
            if key == VK_ESCAPE.0 {
                if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
                    s.annotations.draft = None;
                    s.annotations.selected = None;
                    s.annotations.tool = pdf_annotations::Tool::Select;
                }
                self.drag = None;
                return true;
            }
        }
        match self.focus {
            Focus::CanvasText => self.canvas_key(key, shift, ctrl),
            Focus::Workflow => self.workflows_key(key, shift, ctrl),
            Focus::AgentSource => {
                let Some(editor) = self.agent.source_editor.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                match editor.field.multiline_key(editor.area, key, shift, ctrl) {
                    FieldKey::Submit => {
                        if ctrl {
                            self.save_agent_source();
                        }
                        true
                    }
                    FieldKey::Cancel => {
                        self.agent.source_editor = None;
                        self.focus = Focus::Main;
                        true
                    }
                    FieldKey::Edited => {
                        editor.dirty = true;
                        true
                    }
                    FieldKey::Ignored => false,
                }
            }
            Focus::InboxEdit => {
                let Some((_, f)) = self.views.inbox.editing.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                match f.key(key, shift, ctrl) {
                    // textarea 里回车是换行；Ctrl+Enter 保存
                    FieldKey::Submit => {
                        if ctrl {
                            self.save_inbox_edit();
                        }
                        true
                    }
                    FieldKey::Cancel => {
                        self.views.inbox.editing = None;
                        self.focus = Focus::Main;
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::RecentQuery => match self.views.recent.query.key(key, shift, ctrl) {
                FieldKey::Cancel => {
                    self.views.recent.query.clear();
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Submit => true,
                FieldKey::Edited => {
                    self.views.recent.scroll = 0.0;
                    true
                }
                FieldKey::Ignored => false,
            },
            Focus::ScheduleForm | Focus::ScheduleQuick | Focus::ScheduleQuery => {
                self.agenda_field_key(key, shift, ctrl)
            }
            Focus::AiSessionQuery => !matches!(
                self.ai.workspace.query.key(key, shift, ctrl),
                FieldKey::Ignored
            ),
            Focus::AiMessageQuery => match self.ai.panel.search_query.key(key, shift, ctrl) {
                FieldKey::Submit => {
                    self.ai_search_next(if shift { -1 } else { 1 });
                    true
                }
                FieldKey::Cancel => {
                    self.ai_close_search();
                    true
                }
                FieldKey::Edited => {
                    self.ai_search_changed();
                    true
                }
                FieldKey::Ignored => false,
            },
            Focus::ProviderField => {
                if key == 9 {
                    if let Some(f) = self.prefs.providers.form.as_mut() {
                        f.focus = (f.focus + if shift { 3 } else { 1 }) % 4;
                    }
                    return true;
                }
                let result = self
                    .prefs
                    .providers
                    .form
                    .as_mut()
                    .map(|f| f.fields[f.focus].key(key, shift, ctrl));
                match result {
                    Some(FieldKey::Submit) => {
                        self.prefs.providers.status = match self.save_provider_form() {
                            Ok(()) => "✓ 配置已保存".into(),
                            Err(error) => format!("操作失败：{error}"),
                        };
                    }
                    Some(FieldKey::Cancel) => {
                        self.prefs.providers.form = None;
                        self.focus = Focus::Main;
                    }
                    _ => {}
                }
                !matches!(result, None | Some(FieldKey::Ignored))
            }
            Focus::TableCell => {
                if key == 27 {
                    self.table_editing = None;
                    self.focus = Focus::Main;
                    return true;
                }
                if key == 13 || key == 9 {
                    let start = self
                        .table_editing
                        .as_ref()
                        .map(|e| e.cell.range.start)
                        .unwrap_or(0);
                    if self.commit_table_cell() && key == 9 {
                        let cells = self
                            .shell
                            .active()
                            .and_then(|t| t.buffer())
                            .map(|b| {
                                self.doc.table_cells(
                                    self.editor_area,
                                    b.text(),
                                    self.shell.active_scroll(),
                                )
                            })
                            .unwrap_or_default();
                        let next = if shift {
                            cells.into_iter().rev().find(|c| c.range.start < start)
                        } else {
                            cells.into_iter().find(|c| c.range.start > start)
                        };
                        if let Some(cell) = next {
                            let x = cell.rect.left + 12.0;
                            self.begin_table_cell(cell, x);
                        }
                    }
                    return true;
                }
                self.table_editing
                    .as_mut()
                    .is_some_and(|e| e.key(key, shift, ctrl))
            }
            Focus::DocumentTitle => false,
            Focus::AiInput => {
                if key == 13 && !shift {
                    self.ai_send(HWND(self.hwnd_raw as *mut _));
                    return true;
                }
                let rect = self
                    .ai
                    .layout
                    .rect_of(assistant::Hit::Input)
                    .unwrap_or_default();
                match self.ai.panel.input.multiline_key(rect, key, shift, ctrl) {
                    FieldKey::Submit => {
                        // Enter 发送；Shift+Enter 在 TSX 里是换行，单行框先忽略
                        if !shift {
                            let hwnd = HWND(self.hwnd_raw as *mut _);
                            self.ai_send(hwnd);
                        }
                        true
                    }
                    FieldKey::Cancel => {
                        if self.ai.panel.is_streaming() {
                            self.ai_cancel();
                        } else {
                            self.focus = Focus::Main;
                        }
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::VersionMessage => match self.panels.version.message.key(key, shift, ctrl) {
                // textarea 里回车是换行；单行框里当作「无事」，Esc 失焦
                FieldKey::Submit => true,
                FieldKey::Cancel => {
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Edited => true,
                FieldKey::Ignored => false,
            },
            Focus::CommentCompose => {
                let Some(p) = self.panels.comments.pending.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                let rect = self
                    .panels
                    .comments_layout
                    .rect_of(comments::Hit::ComposeField)
                    .unwrap_or_default();
                match p.field.multiline_key(rect, key, shift, ctrl) {
                    // TSX：Ctrl+Enter 发送；单独回车在 textarea 里是换行，这里忽略
                    FieldKey::Submit => {
                        if ctrl {
                            self.submit_comment();
                        }
                        true
                    }
                    FieldKey::Cancel => {
                        self.panels.comments.pending = None;
                        self.focus = Focus::Main;
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::SettingsField => {
                let Some(e) = self.prefs.editing.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                match e.field.key(key, shift, ctrl) {
                    FieldKey::Submit => {
                        self.commit_settings_field();
                        true
                    }
                    FieldKey::Cancel => {
                        self.prefs.editing = None;
                        self.focus = Focus::Main;
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::SettingsSearch => match self.prefs.search.key(key, shift, ctrl) {
                FieldKey::Cancel | FieldKey::Submit => {
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Edited => {
                    self.prefs.scroll = 0.0;
                    true
                }
                FieldKey::Ignored => false,
            },
            Focus::Search => {
                use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_UP};
                let Some(s) = self.search.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                if key == VK_DOWN.0 || key == VK_UP.0 {
                    s.move_selection(if key == VK_DOWN.0 { 1 } else { -1 });
                    return true;
                }
                let before = s.query.text().to_owned();
                match s.query.key(key, shift, ctrl) {
                    FieldKey::Submit => {
                        self.open_selected_search_row();
                        true
                    }
                    FieldKey::Cancel => {
                        self.close_search();
                        true
                    }
                    FieldKey::Edited => {
                        if s.query.text() != before {
                            self.search_changed();
                        }
                        true
                    }
                    FieldKey::Ignored => false,
                }
            }
            Focus::Dialog => {
                if self.mapped_folder.is_some() {
                    if key == 0x09 {
                        let dialog = self.mapped_folder.as_mut().unwrap();
                        dialog.active = match (dialog.active, dialog.advanced) {
                            (mapped_folder::Field::Name, true) => mapped_folder::Field::Include,
                            (mapped_folder::Field::Name, false) => mapped_folder::Field::Name,
                            (mapped_folder::Field::Include, _) => mapped_folder::Field::Exclude,
                            (mapped_folder::Field::Exclude, _) => mapped_folder::Field::Name,
                        };
                        return true;
                    }
                    if key == VK_RETURN.0 {
                        self.create_mapped_folder_from_dialog();
                        return true;
                    }
                    if key == VK_ESCAPE.0 {
                        self.close_mapped_folder_dialog();
                        return true;
                    }
                    let dialog = self.mapped_folder.as_mut().unwrap();
                    let outcome = match dialog.active {
                        mapped_folder::Field::Name => dialog.name.key(key, shift, ctrl),
                        mapped_folder::Field::Include => dialog.include.key(key, shift, ctrl),
                        mapped_folder::Field::Exclude => dialog.exclude.key(key, shift, ctrl),
                    };
                    if outcome == FieldKey::Edited {
                        dialog.error.clear();
                    }
                    return outcome != FieldKey::Ignored;
                }
                if self.link_create.is_some() {
                    if key == 0x09 {
                        // Tab
                        let dialog = self.link_create.as_mut().unwrap();
                        dialog.active = match dialog.active {
                            link_create::Field::Name => link_create::Field::Url,
                            link_create::Field::Url => link_create::Field::Name,
                        };
                        return true;
                    }
                    if key == VK_RETURN.0 {
                        self.create_link_from_dialog();
                        return true;
                    }
                    if key == VK_ESCAPE.0 {
                        self.close_link_create();
                        return true;
                    }
                    let dialog = self.link_create.as_mut().unwrap();
                    let outcome = match dialog.active {
                        link_create::Field::Name => dialog.name.key(key, shift, ctrl),
                        link_create::Field::Url => dialog.url.key(key, shift, ctrl),
                    };
                    if outcome == FieldKey::Edited {
                        dialog.error.clear();
                    }
                    return outcome != FieldKey::Ignored;
                }
                let Some(dialog) = self.dialog.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                let math_rect = dialog
                    .is_math_editor()
                    .then(|| dialog.field_rect(self.renderer.viewport()).unwrap());
                let outcome = match dialog.field.as_mut() {
                    Some(f) if math_rect.is_some() => {
                        f.multiline_key(math_rect.unwrap(), key, shift, ctrl)
                    }
                    Some(f) => f.key(key, shift, ctrl),
                    None if key == 0x20
                        && dialog
                            .buttons
                            .iter()
                            .any(|b| matches!(b.kind, ButtonKind::Checkbox(_))) =>
                    {
                        let action = dialog
                            .buttons
                            .iter()
                            .find(|b| matches!(b.kind, ButtonKind::Checkbox(_)))
                            .unwrap()
                            .action
                            .clone();
                        self.run_dialog_action(action);
                        return true;
                    }
                    None if key == VK_RETURN.0 => FieldKey::Submit,
                    None if key == VK_ESCAPE.0 => FieldKey::Cancel,
                    None => FieldKey::Ignored,
                };
                match outcome {
                    FieldKey::Submit => {
                        // 文件夹导入默认粘贴；其他对话框沿用最后一个主按钮。
                        if let Some(b) = dialog
                            .buttons
                            .iter()
                            .find(|b| {
                                matches!(
                                    b.action,
                                    DialogAction::ImportDropped { mapping: false, .. }
                                )
                            })
                            .or_else(|| dialog.buttons.last())
                        {
                            let action = b.action.clone();
                            self.run_dialog_action(action);
                        }
                        true
                    }
                    FieldKey::Cancel => {
                        let action = dialog.dismiss.clone();
                        self.run_dialog_action(action);
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::SidebarSearch => match self.side.search.key(key, shift, ctrl) {
                FieldKey::Cancel => {
                    self.side.search.clear();
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Submit => true,
                FieldKey::Edited => {
                    self.side.scroll = 0.0;
                    true
                }
                FieldKey::Ignored => false,
            },
            Focus::SidebarEditor => {
                let Some(e) = self.side.editing.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                match e.field.key(key, shift, ctrl) {
                    FieldKey::Submit => {
                        self.confirm_sidebar_edit();
                        true
                    }
                    FieldKey::Cancel => {
                        self.side.editing = None;
                        self.focus = Focus::Main;
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::NavNewLibrary => {
                let Some(c) = self.nav.creating.as_mut() else {
                    self.focus = Focus::Main;
                    return false;
                };
                match c.field.key(key, shift, ctrl) {
                    FieldKey::Submit => {
                        self.confirm_new_library();
                        true
                    }
                    FieldKey::Cancel => {
                        self.nav.creating = None;
                        self.focus = Focus::Main;
                        true
                    }
                    FieldKey::Edited => true,
                    FieldKey::Ignored => false,
                }
            }
            Focus::FindQuery | Focus::FindReplacement => self.on_find_key(key, shift, ctrl),
            Focus::Command => self.on_command_key(key, shift, ctrl),
            Focus::Main => {
                let content = self.content();
                if content == MainContent::Standalone(WorkspaceView::Schedule) && !ctrl {
                    return self.schedule_key(key, shift);
                }
                if content == MainContent::Home && !ctrl {
                    match key {
                        0x09 => return self.home.navigate(self.editor_area, shift),
                        0x0D | 0x20 => {
                            if let Some(action) = self.home.focused_action() {
                                self.run_home_action(action);
                                return true;
                            }
                        }
                        0x1B => return self.home.clear_keyboard_focus(),
                        _ => {}
                    }
                }
                if content == MainContent::Document {
                    if !self.editor_engaged {
                        return false;
                    }
                    // Windows 先把 Enter 送到 WM_KEYDOWN；文档编辑器在这里消费后，
                    // WM_CHAR 不会再到 on_char。因此围栏补全必须放在实际按键路径，
                    // 不能只依赖字符输入路径。
                    if key == VK_RETURN.0 && !ctrl {
                        if let Some(buffer) = self.shell.active_buffer_mut() {
                            crate::app::editor_blocks::normalize_chinese_code_fence_on_newline(
                                buffer,
                            );
                            if crate::app::editor_blocks::complete_code_fence_on_newline(buffer)
                                || crate::app::editor_blocks::complete_inline_code_on_newline(
                                    buffer,
                                )
                            {
                                self.after_doc_edit(true);
                                return true;
                            }
                        }
                    }
                    let Some(buffer) = self.shell.active_buffer_mut() else {
                        return false;
                    };
                    let outcome = self.doc.handle_key(buffer, key, shift, ctrl);
                    if outcome.handled {
                        if outcome.text_changed {
                            self.after_doc_edit(!outcome.keep_desired);
                        } else {
                            self.editor_ai.invalidate();
                            self.editor_ai.timer = None;
                            self.after_doc_selection_change(!outcome.keep_desired);
                        }
                    }
                    return outcome.handled;
                }
                if content != MainContent::Source {
                    return false;
                }
                // `source` 和 `shell` 是两个互不相干的字段，同时借用是合法的
                let Some(buffer) = self.shell.active_buffer_mut() else {
                    return false;
                };
                let outcome = self.source.handle_key(buffer, key, shift, ctrl);
                if outcome.handled {
                    if outcome.text_changed {
                        self.after_edit(!outcome.keep_desired);
                    } else {
                        self.editor_ai.invalidate();
                        self.editor_ai.timer = None;
                        self.after_source_selection_change(!outcome.keep_desired);
                    }
                }
                outcome.handled
            }
        }
    }
}
