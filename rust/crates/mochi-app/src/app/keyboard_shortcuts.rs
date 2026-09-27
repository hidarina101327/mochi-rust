//! 识别应用快捷键，并分派对应的页面和面板操作。
use super::*;

impl App {
    /// `utils/keyboard.ts` 的 `defaultShortcuts`。返回是否消费。
    /// `on_edit_key` 之前调：快捷键优先于焦点控件（Ctrl+S 在输入框里也要能存）。
    pub fn on_accelerator(
        &mut self,
        hwnd: HWND,
        key: u16,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        self.window_motion.keyboard = true;
        if self.global_import.is_some() {
            return self.import_picker_key(key, shift, ctrl, alt);
        }
        if self.desktop_manager_input_active() && self.desktop_manager_key(key, shift, ctrl, alt) {
            return true;
        }
        if self.notification_open() {
            if alt && key == 0x73 {
                self.close_notifications();
                return true;
            }
            return self.notification_key(key, shift, ctrl);
        }
        if self.automation.panel.is_some() {
            if self.dialog.is_none() {
                return self.automation_key(key, shift);
            }
            return alt || (ctrl && !matches!(key, 0x41 | 0x43 | 0x56 | 0x58 | 0x59 | 0x5a));
        }
        if self.template_picker.is_some() {
            if key == 0x1b || (alt && key == 0x73) {
                self.template_picker = None;
                self.focus = Focus::Main;
            }
            return true;
        }
        if self.object_picker.is_some() {
            if alt && key == 0x73 {
                self.finish_object_picker(None);
                return true;
            }
            return !alt && self.object_picker_key(key, shift, ctrl);
        }
        if let Some(form) = self.export_form.as_mut() {
            let action = if alt && key == 0x73 {
                Some(crate::ui::export_dialog::Action::Cancel)
            } else {
                form.key(key, shift)
            };
            self.export_form_action(action);
            return true;
        }
        if self.commands.review.is_some() {
            if alt && key == 0x73 {
                self.commands.review = None;
                return true;
            }
            return self.command_review_key(key, ctrl);
        }
        self.suppress_alt_char = false;
        if !alt && self.canvas_key(key, shift, ctrl) {
            return true;
        }
        if self.workflows_active() && (self.focus == Focus::Workflow || self.focus == Focus::Main) {
            if ctrl && matches!(key, 0x43 | 0x56 | 0x58) {
                return match key {
                    0x43 => self.clipboard_copy(false),
                    0x58 => self.clipboard_copy(true),
                    _ => self.clipboard_paste_mode(true),
                };
            }
            if self.workflows_key(key, shift, ctrl) {
                return true;
            }
            if ctrl && !alt {
                return true;
            }
        }
        use crate::ui::shortcuts::{Chord, Mapping};
        let input = Chord {
            key,
            ctrl,
            alt,
            shift,
        };
        let effective = match crate::ui::shortcuts::translate(input) {
            Mapping::Original(c) => c,
            Mapping::Suppressed => return false,
            Mapping::Unchanged => input,
        };
        let handled = if effective
            == (Chord {
                key: 32,
                ctrl: true,
                alt: false,
                shift: true,
            }) {
            self.show_capture();
            true
        } else if effective.alt
            && !effective.ctrl
            && !effective.shift
            && matches!(effective.key, 0x25 | 0x27)
        {
            self.navigate_tab(if effective.key == 0x25 {
                NavigationDirection::Back
            } else {
                NavigationDirection::Forward
            })
        } else if effective.alt {
            self.on_alt_shortcut(effective.key, effective.shift, effective.ctrl)
        } else {
            self.on_shortcut(hwnd, effective.key, effective.shift, effective.ctrl)
        };
        self.suppress_alt_char = alt && handled;
        handled
    }

    /// 日程页主区（不在输入框里）拥有 Ctrl+Z / Ctrl+Y。
    fn schedule_page_focused(&self) -> bool {
        self.focus == Focus::Main
            && self.dialog.is_none()
            && self.state.view == WorkspaceView::Schedule
            && self.content() == MainContent::Standalone(WorkspaceView::Schedule)
    }

    pub fn take_suppressed_alt_char(&mut self) -> bool {
        std::mem::take(&mut self.suppress_alt_char)
    }

    pub fn on_shortcut(&mut self, hwnd: HWND, key: u16, shift: bool, ctrl: bool) -> bool {
        if self.image_preview.is_some() {
            return true;
        }
        if ctrl && self.focus == Focus::TableCell {
            let format = match (key, shift) {
                (0x42, false) => Some(Format::Bold),
                (0x49, false) => Some(Format::Italic),
                (0x55, false) => Some(Format::Underline),
                (0x58, true) => Some(Format::Strike),
                _ => None,
            };
            if let (Some(format), Some(edit)) = (format, self.table_editing.as_mut()) {
                crate::ui::rich::apply_format(&mut edit.field.buffer, format);
                return true;
            }
        }
        if self.export_form.is_some() {
            return true;
        }
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_OEM_5, VK_TAB};
        if !ctrl {
            return false;
        }
        if self.title_editing.is_some()
            && !matches!(key, 0x41 | 0x43 | 0x56 | 0x58 | 0x5a | 0x59)
            && !self.commit_title()
        {
            return true;
        }
        if self.table_editing.is_some()
            && !matches!(key, 0x41 | 0x43 | 0x56 | 0x58 | 0x5a | 0x59)
            && !self.commit_table_cell()
        {
            return true;
        }
        let k = key as u8 as char;
        match (k, shift) {
            ('C', true) if self.editing_file() => self.apply_last_color(false),
            (' ', false) => {
                self.start_inline_prediction(true);
                true
            }
            ('L', false) if self.editor_ai.ghost.is_some() => self.accept_prediction(1),
            ('K', false) if self.editor_ai.ghost.is_some() => self.accept_prediction(2),
            ('F', true) => {
                if self.search.is_some() {
                    self.close_search();
                } else {
                    self.open_search();
                }
                true
            }
            ('L', true) => {
                if let Some(descriptor) = app_settings::descriptor("appearance.themeMode") {
                    let next = if self.state.dark { "light" } else { "dark" };
                    self.app_settings
                        .write(descriptor, &SettingValue::Text(next.into()));
                    let _ = self.app_settings.flush();
                    self.apply_setting_side_effects("appearance.themeMode");
                    self.invalidate_main();
                }
                true
            }
            ('J', false)
                if self.state.view == WorkspaceView::Schedule
                    && self.content() == MainContent::Standalone(WorkspaceView::Schedule)
                    && self.focus != Focus::AiInput =>
            {
                self.agenda_ai_plan();
                true
            }
            ('J', false) => {
                self.toggle_ai_panel();
                true
            }
            ('P', true) => {
                self.toggle_command(command::Mode::Commands);
                true
            }
            ('P', false) => {
                self.toggle_command(command::Mode::Files);
                true
            }
            ('S', false) => {
                self.save_active();
                true
            }
            ('W', false) => {
                if let Some(i) = self.shell.active_tab() {
                    let path = self.active_file_path();
                    self.shell.close_tab(i);
                    if self.state.view == WorkspaceView::QuickNote
                        && self.active_file_path() != path
                    {
                        self.state.view = WorkspaceView::Editor;
                    }
                    self.invalidate_main();
                    self.sync_state();
                }
                true
            }
            ('N', false) => {
                if let Some(root) = self.shell.tree_root() {
                    self.open_create_dialog(root, false);
                }
                true
            }
            ('O', false) => {
                self.open_workspace_dialog(hwnd);
                true
            }
            ('E', true) => {
                self.toggle_source_mode();
                true
            }
            ('E', false) if self.editing_file() => {
                self.apply_format(Format::Code);
                true
            }
            ('X', true) if self.editing_file() => {
                self.apply_format(Format::Strike);
                true
            }
            ('B', true) if self.editing_file() => {
                self.apply_format(Format::CodeBlock);
                true
            }
            ('8', true) if self.editing_file() => {
                self.apply_format(Format::BulletList);
                true
            }
            ('9', true) if self.editing_file() => {
                self.apply_format(Format::OrderedList);
                true
            }
            ('7', true) if self.editing_file() => {
                self.apply_format(Format::TaskList);
                true
            }
            _ if key == 0xbe && shift && self.editing_file() => {
                self.apply_format(Format::Quote);
                true
            }
            // 编辑器快捷键：只在主编辑区有焦点且正在编辑文件时生效
            ('B', false) if self.editing_file() => {
                self.apply_format(Format::Bold);
                true
            }
            ('I', false) if self.editing_file() => {
                self.apply_format(Format::Italic);
                true
            }
            ('U', false) if self.editing_file() => {
                self.apply_format(Format::Underline);
                true
            }
            ('Z', false) if self.schedule_page_focused() => self.schedule_undo(),
            ('Z', true) | ('Y', false) if self.schedule_page_focused() => self.schedule_redo(),
            ('Z', false) if self.focus == Focus::Main && self.file_open() => {
                self.undo_redo(false);
                true
            }
            ('Z', true) | ('Y', false) if self.focus == Focus::Main && self.file_open() => {
                self.undo_redo(true);
                true
            }
            // 文档内查找 / 替换。表格文件没有正文 buffer，同样用查找条。
            ('F', false) if self.file_open() || self.base_viewer_open() => {
                if self.dialog.is_none()
                    && self.menu.is_none()
                    && self.search.is_none()
                    && self.command.is_none()
                {
                    self.open_find(false);
                }
                true
            }
            ('H', false) if self.file_open() => {
                self.open_find(true);
                true
            }
            // 剪贴板：编辑器与所有单行输入框共用
            ('C', false) => self.clipboard_copy(false),
            ('X', false) => self.clipboard_copy(true),
            ('V', false) => self.clipboard_paste(),
            ('V', true) => self.clipboard_paste_mode(true),
            _ if key == VK_TAB.0 => {
                let n = self.shell.tabs().len();
                if let (Some(i), true) = (self.shell.active_tab(), n > 0) {
                    let next = if shift { (i + n - 1) % n } else { (i + 1) % n };
                    self.shell.select_tab(next);
                    self.remember_split_active();
                    self.invalidate_main();
                    self.sync_state();
                }
                true
            }
            _ if key == VK_OEM_5.0 => {
                // Ctrl+\：显示/隐藏侧边栏
                self.state.sidebar_visible = !self.state.sidebar_visible;
                self.invalidate_main();
                true
            }
            _ => false,
        }
    }

    /// 与 TSX 的 onToggleAIPanel 同义：开着助手就关面板，否则开到助手。
    pub(super) fn toggle_ai_panel(&mut self) {
        if self.ai.float.take().is_some() {
            self.invalidate_main();
            return;
        }
        if self.state.ai_panel_open && self.state.right_panel == RightPanel::Assistant {
            self.state.ai_panel_open = false;
        } else {
            self.state.ai_panel_open = true;
            self.set_right_panel(RightPanel::Assistant);
        }
        self.invalidate_main();
    }
}
