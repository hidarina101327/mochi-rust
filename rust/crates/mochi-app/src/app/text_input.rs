//! 处理字符输入，并将 UTF-16 输入转换为文本编辑事件。
use super::*;

impl App {
    /// 普通字符输入（`WM_CHAR`）。
    ///
    /// 控制字符要挡掉：退格、Tab、Esc 都会以 `WM_CHAR` 再来一遍，
    /// 不挡的话文本里会混进 `\u{8}` 这类看不见的字节。
    pub fn on_char(&mut self, ch: char) -> bool {
        if self.global_import.is_some() {
            return true;
        }
        if self.desktop.saving_editor && self.desktop_manager_active() {
            return true;
        }
        let desktop_input = self.desktop_manager_input_active();
        if let Some(panel) = self.desktop.panel.as_mut().filter(|_| desktop_input) {
            panel.char(ch);
            return true;
        }
        if self.notification_open() {
            return true;
        }
        if self.automation.panel.is_some() && self.dialog.is_none() {
            return true;
        }
        if let Some(dialog) = self.object_picker.as_mut() {
            return dialog.state.char(ch);
        }
        if let Some(search) = self.menu.as_mut().and_then(Menu::search_mut) {
            let changed = search.char(ch);
            if changed {
                if let Some(menu) = self.menu.as_mut() {
                    menu.search_changed();
                }
            }
            return changed;
        }
        if self.image_preview.is_some() {
            return true;
        }
        if self.table_picker.is_some() {
            return true;
        }
        if self.export_form.is_some() {
            return true;
        }
        if self.commands.review.is_some() {
            return false;
        }
        if self.menu.is_some() {
            return false;
        }
        if let Some(dialog) = self.mapped_folder.as_mut() {
            let changed = match dialog.active {
                mapped_folder::Field::Name => dialog.name.char(ch),
                mapped_folder::Field::Include => dialog.include.char(ch),
                mapped_folder::Field::Exclude => dialog.exclude.char(ch),
            };
            if changed {
                dialog.error.clear();
            }
            return changed;
        }
        if let Some(dialog) = self.link_create.as_mut() {
            let changed = match dialog.active {
                link_create::Field::Name => dialog.name.char(ch),
                link_create::Field::Url => dialog.url.char(ch),
            };
            if changed {
                dialog.error.clear();
            }
            return changed;
        }
        match self.focus {
            Focus::CanvasText => {
                let changed = self.focused_field_mut().is_some_and(|field| field.char(ch));
                if changed {
                    self.canvas_text_changed();
                }
                changed
            }
            Focus::Workflow => {
                if self.workflows.view.searching {
                    self.workflows.view.scroll = 0.;
                    self.workflows.view.search.char(ch)
                } else {
                    self.workflows.view.editor != Some(crate::ui::workflows::Editor::Result)
                        && self.workflows.view.field.char(ch)
                }
            }
            Focus::DocumentTitle => self
                .title_editing
                .as_mut()
                .is_some_and(|e| e.field.char(ch)),
            Focus::NavNewLibrary => {
                if let Some(c) = self.nav.creating.as_mut() {
                    if c.field.char(ch) {
                        c.error.clear();
                        return true;
                    }
                }
                false
            }
            Focus::SidebarSearch => {
                if self.side.search.char(ch) {
                    self.side.scroll = 0.0;
                    return true;
                }
                false
            }
            Focus::SidebarEditor => {
                if let Some(e) = self.side.editing.as_mut() {
                    if e.field.char(ch) {
                        e.error.clear();
                        return true;
                    }
                }
                false
            }
            Focus::Dialog => {
                if let Some(f) = self.dialog.as_mut().and_then(|d| d.field.as_mut()) {
                    if f.char(ch) {
                        if let Some(d) = self.dialog.as_mut() {
                            d.error.clear();
                        }
                        return true;
                    }
                }
                false
            }
            Focus::Search => {
                let Some(s) = self.search.as_mut() else {
                    return false;
                };
                if s.query.char(ch) {
                    self.search_changed();
                    return true;
                }
                false
            }
            Focus::SettingsField => self
                .prefs
                .editing
                .as_mut()
                .map(|e| e.field.char(ch))
                .unwrap_or(false),
            Focus::SettingsSearch => {
                let changed = self.prefs.search.char(ch);
                if changed {
                    self.prefs.scroll = 0.0;
                }
                changed
            }
            Focus::VersionMessage => self.panels.version.message.char(ch),
            Focus::CommentCompose => self
                .panels
                .comments
                .pending
                .as_mut()
                .map(|p| p.field.char(ch))
                .unwrap_or(false),
            Focus::AiInput => self.ai.panel.input.char(ch),
            Focus::AiMessageQuery => {
                let changed = self.ai.panel.search_query.char(ch);
                if changed {
                    self.ai_search_changed();
                }
                changed
            }
            Focus::ScheduleForm | Focus::ScheduleQuick | Focus::ScheduleQuery => {
                self.agenda_char(ch)
            }
            Focus::AiSessionQuery => {
                let changed = self.ai.workspace.query.char(ch);
                self.ai.workspace.scroll = 0.0;
                changed
            }
            Focus::ProviderField => self
                .prefs
                .providers
                .form
                .as_mut()
                .is_some_and(|f| f.fields[f.focus].char(ch)),
            Focus::TableCell => self.table_editing.as_mut().is_some_and(|e| {
                if ch.is_control() {
                    return false;
                }
                crate::ui::rich::insert(&mut e.field.buffer, &ch.to_string());
                true
            }),
            Focus::InboxEdit => self
                .views
                .inbox
                .editing
                .as_mut()
                .map(|(_, f)| f.char(ch))
                .unwrap_or(false),
            Focus::RecentQuery => {
                if self.views.recent.query.char(ch) {
                    self.views.recent.scroll = 0.0;
                    return true;
                }
                false
            }
            Focus::FindQuery => {
                let edited = self
                    .find
                    .as_mut()
                    .map(|f| f.query.char(ch))
                    .unwrap_or(false);
                if edited {
                    self.find_query_changed();
                }
                edited
            }
            Focus::FindReplacement => self
                .find
                .as_mut()
                .map(|f| f.replacement.char(ch))
                .unwrap_or(false),
            Focus::Command => {
                let Some(c) = self.command.as_mut() else {
                    return false;
                };
                if c.query.char(ch) {
                    c.query_changed();
                    return true;
                }
                false
            }
            Focus::AgentSource => self.agent.source_editor.as_mut().is_some_and(|editor| {
                let changed = editor.field.char(ch);
                editor.dirty |= changed;
                changed
            }),
            Focus::Main => {
                let content = self.content();
                if content != MainContent::Source && content != MainContent::Document {
                    return false;
                }
                // 渲染视图：没点进去之前敲字不算（TipTap 打开文件后也不自动聚焦）
                if content == MainContent::Document && !self.editor_engaged {
                    return false;
                }
                // Enter 已在 WM_KEYDOWN / on_edit_key 中处理；Windows 仍可能随后投递
                // 一个 WM_CHAR('\r')，这里若再接受会把普通换行重复插入一次。
                if ch.is_control() && ch != '\n' && ch != '\t' {
                    return false;
                }
                let open_slash_menu = ch == '/'
                    && self.settings.get("app.editor.slashMenuEnabled").as_deref() == Some("true")
                    && content == MainContent::Document
                    && self
                        .shell
                        .active()
                        .and_then(|tab| tab.buffer())
                        .is_some_and(|buffer| !buffer.has_selection());
                let text = if ch == '\r' {
                    "\n".to_owned()
                } else {
                    ch.to_string()
                };
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    let is_newline = ch == '\r' || ch == '\n';
                    let completed_code = if content == MainContent::Document && is_newline {
                        crate::app::editor_blocks::normalize_chinese_code_fence_on_newline(buffer);
                        crate::app::editor_blocks::complete_code_fence_on_newline(buffer)
                            || crate::app::editor_blocks::complete_inline_code_on_newline(buffer)
                    } else {
                        false
                    };
                    if !completed_code
                        && content == MainContent::Document
                        && !crate::ui::editor_preferences::current().live_line_source
                    {
                        self.doc.insert(buffer, &text);
                    } else if !completed_code {
                        buffer.insert(&text);
                    }
                }
                if content == MainContent::Document {
                    self.after_doc_edit(true);
                    if open_slash_menu {
                        let _ = self.open_slash_insert_menu();
                    }
                } else {
                    self.after_edit(true);
                }
                true
            }
        }
    }

    pub fn on_utf16_char(&mut self, unit: u16) -> bool {
        if (0xd800..=0xdbff).contains(&unit) {
            self.high_surrogate = Some((unit, self.focus, self.active_file_path()));
            return true;
        }
        let high = self.high_surrogate.take();
        if (0xdc00..=0xdfff).contains(&unit) {
            if let Some((high, focus, path)) = high {
                if focus == self.focus && path == self.active_file_path() {
                    if let Some(ch) = char::from_u32(
                        0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00),
                    ) {
                        return self.on_char(ch);
                    }
                }
            }
            return false;
        }
        char::from_u32(u32::from(unit)).is_some_and(|ch| self.on_char(ch))
    }
}
