//! 管理输入焦点、输入法组合输入和光标位置。
use super::*;

impl App {
    /// 当前拿着键盘焦点的输入框（不是主区时）。IME 三个入口都走它分派。
    pub(super) fn focused_field_mut(&mut self) -> Option<&mut TextField> {
        if self.global_import.is_some() {
            return None;
        }
        if self.desktop.saving_editor && self.desktop_manager_active() {
            return None;
        }
        if self.desktop_manager_input_active() {
            return self
                .desktop
                .panel
                .as_mut()
                .and_then(|panel| panel.focused_field_mut());
        }
        if self.object_picker.is_some() {
            return self
                .object_picker
                .as_mut()
                .map(|dialog| &mut dialog.state.query);
        }
        if self.menu.is_some() {
            return self.menu.as_mut().and_then(Menu::search_mut);
        }
        match self.focus {
            Focus::CanvasText => match self.shell.active_mut().map(|tab| &mut tab.kind) {
                Some(TabKind::Viewer {
                    content: viewer::Content::Canvas(state),
                    ..
                }) => state.editor.as_mut().map(|edit| &mut edit.field),
                _ => None,
            },
            Focus::Workflow => {
                if self.workflows.view.searching {
                    Some(&mut self.workflows.view.search)
                } else {
                    (self.workflows.view.editor != Some(crate::ui::workflows::Editor::Result))
                        .then_some(&mut self.workflows.view.field)
                }
            }
            Focus::DocumentTitle => self.title_editing.as_mut().map(|e| &mut e.field),
            Focus::NavNewLibrary => self.nav.creating.as_mut().map(|c| &mut c.field),
            Focus::SidebarSearch => Some(&mut self.side.search),
            Focus::SidebarEditor => self.side.editing.as_mut().map(|e| &mut e.field),
            Focus::Dialog => {
                if let Some(dialog) = self.mapped_folder.as_mut() {
                    return Some(match dialog.active {
                        mapped_folder::Field::Name => &mut dialog.name,
                        mapped_folder::Field::Include => &mut dialog.include,
                        mapped_folder::Field::Exclude => &mut dialog.exclude,
                    });
                }
                if let Some(dialog) = self.link_create.as_mut() {
                    return Some(match dialog.active {
                        link_create::Field::Name => &mut dialog.name,
                        link_create::Field::Url => &mut dialog.url,
                    });
                }
                self.dialog.as_mut().and_then(|d| d.field.as_mut())
            }
            Focus::Search => self.search.as_mut().map(|s| &mut s.query),
            Focus::SettingsField => self.prefs.editing.as_mut().map(|e| &mut e.field),
            Focus::SettingsSearch => Some(&mut self.prefs.search),
            Focus::VersionMessage => Some(&mut self.panels.version.message),
            Focus::CommentCompose => self.panels.comments.pending.as_mut().map(|p| &mut p.field),
            Focus::AiInput => Some(&mut self.ai.panel.input),
            Focus::AiMessageQuery => Some(&mut self.ai.panel.search_query),
            Focus::ScheduleForm | Focus::ScheduleQuick | Focus::ScheduleQuery => {
                self.agenda_field_mut()
            }
            Focus::AiSessionQuery => Some(&mut self.ai.workspace.query),
            Focus::ProviderField => self
                .prefs
                .providers
                .form
                .as_mut()
                .map(|f| &mut f.fields[f.focus]),
            Focus::TableCell => self.table_editing.as_mut().map(|e| &mut e.field),
            Focus::InboxEdit => self.views.inbox.editing.as_mut().map(|(_, f)| f),
            Focus::RecentQuery => Some(&mut self.views.recent.query),
            Focus::FindQuery => self.find.as_mut().map(|f| &mut f.query),
            Focus::FindReplacement => self.find.as_mut().map(|f| &mut f.replacement),
            Focus::Command => self.command.as_mut().map(|c| &mut c.query),
            Focus::AgentSource => self.agent.source_editor.as_mut().map(|e| &mut e.field),
            Focus::Main => None,
        }
    }

    /// 主区是否接收输入法/键盘的编辑输入。
    pub(super) fn main_accepts_input(&self) -> bool {
        match self.content() {
            MainContent::Source => true,
            MainContent::Document => self.editor_engaged,
            _ => false,
        }
    }

    pub fn on_ime_composition(&mut self, text: &str, cursor: usize) {
        if self.global_import.is_some() {
            return;
        }
        if self.desktop.saving_editor && self.desktop_manager_active() {
            return;
        }
        let desktop_input = self.desktop_manager_input_active();
        if let Some(panel) = self.desktop.panel.as_mut().filter(|_| desktop_input) {
            if let Some(f) = panel.focused_field_mut() {
                f.buffer.set_composition(text, cursor);
            }
            return;
        }
        if self.notification_open() {
            return;
        }
        if self.template_picker.is_some() || self.table_picker.is_some() {
            return;
        }
        if self.object_picker.is_some() || (self.menu.is_some() && self.slash_trigger.is_none()) {
            if let Some(field) = self.focused_field_mut() {
                field.buffer.set_composition(text, cursor);
            }
            return;
        }
        if self.automation.panel.is_some() && self.dialog.is_none() {
            return;
        }
        if self.image_preview.is_some() {
            return;
        }
        if self.export_form.is_some() {
            return;
        }
        if self.commands.review.is_some() {
            return;
        }
        if self.focus == Focus::TableCell {
            if let Some(e) = self.table_editing.as_mut() {
                crate::ui::rich::compose(&mut e.field.buffer, text, cursor);
            }
            return;
        }
        if self.focus != Focus::Main {
            if let Some(f) = self.focused_field_mut() {
                f.buffer.set_composition(text, cursor);
            }
            return;
        }
        if !self.main_accepts_input() {
            return;
        }
        let rich = self.content() == MainContent::Document
            && !crate::ui::editor_preferences::current().live_line_source;
        if let Some(buffer) = self.shell.active_buffer_mut() {
            if rich {
                crate::ui::rich::compose(buffer, text, cursor);
            } else {
                buffer.set_composition(text, cursor);
            }
        }
        self.after_edit(true);
    }

    pub fn on_ime_commit(&mut self, text: &str) {
        if self.global_import.is_some() {
            return;
        }
        if self.desktop.saving_editor && self.desktop_manager_active() {
            return;
        }
        let desktop_input = self.desktop_manager_input_active();
        if let Some(panel) = self.desktop.panel.as_mut().filter(|_| desktop_input) {
            if let Some(f) = panel.focused_field_mut() {
                f.buffer.commit_composition(text);
            }
            panel.commit_focused_field();
            return;
        }
        if self.notification_open() {
            return;
        }
        if self.template_picker.is_some() || self.table_picker.is_some() {
            return;
        }
        if self.object_picker.is_some() || (self.menu.is_some() && self.slash_trigger.is_none()) {
            self.high_surrogate = None;
            if let Some(field) = self.focused_field_mut() {
                field.buffer.commit_composition(text);
            }
            self.after_field_edit();
            return;
        }
        if self.automation.panel.is_some() && self.dialog.is_none() {
            return;
        }
        if self.image_preview.is_some() {
            return;
        }
        if text.is_empty() {
            self.on_ime_cancel();
            return;
        }
        if self.export_form.is_some() {
            return;
        }
        if self.commands.review.is_some() {
            return;
        }
        self.high_surrogate = None;
        if self.focus == Focus::TableCell {
            if let Some(e) = self.table_editing.as_mut() {
                e.field.buffer.cancel_composition();
                crate::ui::rich::insert(&mut e.field.buffer, text);
            }
            return;
        }
        if self.focus != Focus::Main {
            if let Some(f) = self.focused_field_mut() {
                f.buffer.commit_composition(text);
            }
            self.after_field_edit();
            return;
        }
        if !self.main_accepts_input() {
            return;
        }
        let rich = self.content() == MainContent::Document
            && !crate::ui::editor_preferences::current().live_line_source;
        if let Some(buffer) = self.shell.active_buffer_mut() {
            if rich {
                buffer.cancel_composition();
                crate::ui::rich::insert(buffer, text);
            } else {
                buffer.commit_composition(text);
            }
        }
        self.after_edit(true);
    }

    pub fn on_ime_cancel(&mut self) {
        self.high_surrogate = None;
        if self.focus != Focus::Main
            || self.object_picker.is_some()
            || (self.menu.is_some() && self.slash_trigger.is_none())
        {
            if let Some(f) = self.focused_field_mut() {
                f.buffer.cancel_composition();
            }
            return;
        }
        if !self.main_accepts_input() {
            return;
        }
        if !self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .is_some_and(|b| b.composition().is_some())
        {
            return;
        }
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.cancel_composition();
        }
        self.after_edit(true);
    }

    /// 渲染并裁剪后的光标矩形是输入法定位几何的唯一来源。
    pub fn caret_in_client(&self) -> Option<Rect> {
        self.list.caret_rect()
    }
}
