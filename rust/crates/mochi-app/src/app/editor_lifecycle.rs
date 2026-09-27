//! 管理编辑中的复制、粘贴、字段变更以及撤销和重做。
use super::*;

impl App {
    /// 主编辑区是否正在编辑一个文件（渲染视图或源码模式）。编辑器快捷键的前提。
    pub(super) fn editing_file(&self) -> bool {
        let content = self.content();
        self.focus == Focus::Main
            && (content == MainContent::Source
                || (content == MainContent::Document && self.editor_engaged))
            && self.shell.active().and_then(|t| t.buffer()).is_some()
    }

    /// Ctrl+C / Ctrl+X。没有选区时不动剪贴板（与浏览器一致），返回是否消费。
    pub(super) fn clipboard_copy(&mut self, cut: bool) -> bool {
        if self.focus == Focus::Main
            && self.settings_overlay.is_none()
            && self.object_picker.is_none()
            && self.ai_copy_selection(cut)
        {
            return true;
        }
        if self.focus == Focus::TableCell
            || self.editing_file()
                && self.content() == MainContent::Document
                && !crate::ui::editor_preferences::current().live_line_source
        {
            let owner = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow() };
            return self.clipboard_copy_formatted_with(cut, |plain, html| {
                platform::copy_payload(owner, plain, html)
            });
        }
        self.clipboard_copy_with(cut, platform::copy_to_clipboard)
    }

    pub(super) fn clipboard_copy_with(
        &mut self,
        cut: bool,
        write: impl FnOnce(&str) -> bool,
    ) -> bool {
        if self.focus == Focus::TableCell {
            let Some(edit) = self.table_editing.as_mut() else {
                return false;
            };
            if !edit.field.buffer.has_selection() {
                return true;
            }
            if !write(&edit.selected_text()) {
                self.state.status_text = "复制失败，原内容已保留".into();
                return true;
            }
            if cut {
                crate::ui::rich::insert(&mut edit.field.buffer, "");
            }
            self.show_global_notice(if cut { "已剪切" } else { "已复制" });
            return true;
        }
        if self.editing_file() {
            let rich = self.content() == MainContent::Document
                && !crate::ui::editor_preferences::current().live_line_source;
            let Some(buffer) = self.shell.active_buffer_mut() else {
                return false;
            };
            if !buffer.has_selection() {
                return true;
            }
            let selected = if rich {
                self.doc.selected_text(buffer)
            } else {
                buffer.selected_text().to_owned()
            };
            if !write(&selected) {
                self.state.status_text = "复制失败，原内容已保留".into();
                return true;
            }
            if cut {
                if rich {
                    crate::ui::rich::insert(buffer, "");
                } else {
                    buffer.delete_backward();
                }
                self.after_edit(true);
            }
            self.show_global_notice(if cut { "已剪切" } else { "已复制" });
            return true;
        }
        let Some(field) = self.focused_field_mut() else {
            return false;
        };
        if !field.buffer.has_selection() {
            return true;
        }
        if !write(field.buffer.selected_text()) {
            self.state.status_text = "复制失败，原内容已保留".into();
            return true;
        }
        if cut {
            field.buffer.delete_backward();
            self.after_field_edit();
        }
        self.show_global_notice(if cut { "已剪切" } else { "已复制" });
        true
    }

    /// Ctrl+V。
    pub(super) fn clipboard_paste(&mut self) -> bool {
        self.clipboard_paste_mode(false)
    }

    /// 单行输入框内容变了之后的联动（全局搜索去抖、查找条重算匹配）。
    pub(super) fn after_field_edit(&mut self) {
        if let Some(menu) = self.menu.as_mut() {
            menu.search_changed();
            return;
        }
        if self.focus == Focus::CanvasText {
            self.canvas_text_changed();
            return;
        }
        if let Some(dialog) = self.object_picker.as_mut() {
            dialog.state.query_changed();
            return;
        }
        match self.focus {
            Focus::Search => self.search_changed(),
            Focus::FindQuery => self.find_query_changed(),
            Focus::AiMessageQuery => self.ai_search_changed(),
            Focus::Command => {
                if let Some(c) = self.command.as_mut() {
                    c.query_changed();
                }
            }
            _ => {}
        }
    }

    pub(super) fn undo_redo(&mut self, redo: bool) {
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        let changed = if redo { buffer.redo() } else { buffer.undo() };
        if changed {
            self.after_edit(true);
        }
    }

    /// 编辑之后挂自动保存定时器。`save.autoSaveEnabled` 关掉则不挂。
    pub(super) fn schedule_autosave(&mut self) {
        if self.autosave_pending {
            return;
        }
        let enabled = app_settings::descriptor("save.autoSaveEnabled")
            .map(|d| matches!(self.app_settings.read(d), SettingValue::Bool(true)))
            .unwrap_or(true);
        if !enabled {
            return;
        }
        let delay = app_settings::descriptor("save.autoSaveDelay")
            .map(|d| match self.app_settings.read(d) {
                SettingValue::Number(n) => n,
                _ => 2000.0,
            })
            .unwrap_or(2000.0)
            .clamp(500.0, 60_000.0) as u32;
        self.autosave_pending = true;
        self.autosave_request = Some(delay);
    }

    /// 自动保存到点：把所有脏标签写盘。
    pub(super) fn on_autosave_timer(&mut self) {
        self.autosave_pending = false;
        // 输入法组合中不落盘，避免把半个词存进去；等提交后下一次编辑再挂
        if self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.composition().is_some())
            .unwrap_or(false)
        {
            self.schedule_autosave();
            return;
        }
        let activities = self.dirty_text_save_activities();
        if let Err(e) = self.shell.save_dirty_tabs() {
            self.state.status_text = format!("自动保存失败：{e}");
            return;
        }
        self.record_text_save_activities(activities);
        self.sync_state();
    }

    /// 渲染视图 ⇄ 源码编辑（Ctrl+Shift+E）。渲染视图本身已可编辑（`ui::live`），
    /// 需要查看文档头部或所有标记符时，切换到源码模式。
    pub fn toggle_source_mode(&mut self) {
        self.editor_ai.invalidate();
        if let Some((path, viewer::Content::Exam(s))) = self.viewer_tab() {
            let kind = TabKind::File {
                path: path.to_path_buf(),
                buffer: crate::ui::editor::TextBuffer::new(&s.raw),
                source_mode: true,
            };
            if let Some(tab) = self.shell.active_mut() {
                tab.kind = kind;
                tab.scroll = 0.0;
            }
            self.focus = Focus::Main;
            self.invalidate_main();
            return;
        }
        let exam_source=self.shell.active().is_some_and(|tab|matches!(&tab.kind,TabKind::File{path,source_mode:true,..}if path.extension().is_some_and(|ext|ext.eq_ignore_ascii_case("exam"))));
        if exam_source {
            if !self.save_active() {
                return;
            }
            if let Some(tab) = self.shell.active_mut() {
                if let TabKind::File { path, buffer, .. } = &tab.kind {
                    tab.kind = TabKind::Viewer {
                        path: path.clone(),
                        content: viewer::Content::Exam(exam_view::State::new(
                            buffer.text().to_owned(),
                        )),
                    };
                    tab.scroll = 0.0;
                }
            }
            self.invalidate_main();
            return;
        }
        if let Some(tab) = self.shell.active_mut() {
            if let TabKind::File { source_mode, .. } = &mut tab.kind {
                *source_mode = !*source_mode;
                tab.scroll = 0.0;
            }
        }
        self.invalidate_main();
    }

    pub fn save_active(&mut self) -> bool {
        let activity = self.active_text_save_activity();
        let path = self.active_file_path();
        let saved = self.shell.save_active();
        if !saved {
            if let Some(path) = path {
                if self.shell.has_disk_conflict(&path) {
                    self.show_save_conflict(path);
                }
            }
            self.state.status_text = self.shell.status().into();
        }
        self.doc.invalidate();
        self.split.doc.invalidate();
        self.source.invalidate();
        self.split.source.invalidate();
        self.status_bar.stats = None;
        if self.sync_document_format_changes() {
            self.sync_state();
        }
        if saved {
            self.record_text_save_activities(activity.into_iter().collect());
        }
        saved
    }

    /// 编辑操作之后统一收尾：重排、必要时重设目标横坐标、把光标滚进视野。
    ///
    /// 顺序不能改——目标横坐标要从**重排之后**的行里取，否则删掉一行后
    /// 光标的 x 是按旧行算的。
    pub(super) fn after_edit(&mut self, reset_desired_x: bool) {
        self.status_bar.invalidate_word_count();
        self.split.doc.invalidate();
        self.split.source.invalidate();
        if self.content() == MainContent::Document {
            self.after_doc_edit(reset_desired_x);
            return;
        }
        if self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.dirty())
            .unwrap_or(false)
        {
            self.schedule_autosave();
        }
        self.source.invalidate();
        self.after_source_selection_change(reset_desired_x);
    }

    pub(super) fn after_source_selection_change(&mut self, reset_desired_x: bool) {
        let area = self.editor_area;
        let Some(index) = self.shell.active_tab() else {
            return;
        };
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        self.source.ensure(area, index, buffer);

        if reset_desired_x {
            let cursor = buffer.cursor();
            self.source.sync_desired_x(cursor);
        }
        self.scroll_caret_into_view();
    }

    /// 渲染视图里编辑之后的收尾：按新内容与新光标重排（活动块可能变了），
    /// 重设目标横坐标，把光标滚进视野。
    pub(super) fn after_doc_edit(&mut self, reset_desired_x: bool) {
        self.status_bar.invalidate_word_count();
        self.schedule_prediction();
        self.split.doc.invalidate();
        self.split.source.invalidate();
        if self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.dirty())
            .unwrap_or(false)
        {
            self.schedule_autosave();
        }
        // 内容修订号（以及输入法预览指纹）也能覆盖等长替换。保留旧几何供局部重排用。
        self.after_doc_selection_change(reset_desired_x);
        self.find_refresh_after_edit();
    }

    /// 仅移动光标/选区时复用正文几何；块源码模式跨块时仍由 ensure 更新布局。
    pub(super) fn after_doc_selection_change(&mut self, reset_desired_x: bool) {
        let area = self.editor_area;
        let tab = self.shell.active_tab();
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        // IME 组合中把组合串算进显示文本，光标位置也跟着
        let cursor = buffer.display_cursor();
        if buffer.composition().is_some() {
            let (shown, _) = buffer.display_text();
            self.doc.ensure(area, tab, Some(&shown), Some(cursor));
        } else {
            self.doc.ensure_buffer(area, tab, buffer, Some(cursor));
        }
        if reset_desired_x {
            self.doc.sync_desired_x(cursor);
        }
        // 把光标滚进视野
        if area.is_empty() {
            return;
        }
        let Some(tab_ref) = self.shell.active() else {
            return;
        };
        let Some(buffer) = tab_ref.buffer() else {
            return;
        };
        let Some(caret) = self.doc.caret_rect(area, buffer, tab_ref.scroll) else {
            return;
        };
        let mut scroll = tab_ref.scroll;
        if caret.top < area.top {
            scroll -= area.top - caret.top;
        } else if caret.bottom > area.bottom {
            scroll += caret.bottom - area.bottom;
        }
        let scroll = scroll.clamp(0.0, self.doc.max_scroll(area));
        if let Some(t) = self.shell.active_mut() {
            t.scroll = scroll;
        }
    }

    /// 光标跑出视口就把它滚回来。不做的话，往下打字光标会走到屏幕外，
    /// 用户只能一边打一边手动滚。
    pub(super) fn scroll_caret_into_view(&mut self) {
        let area = self.editor_area;
        if area.is_empty() {
            return;
        }
        let Some(tab) = self.shell.active() else {
            return;
        };
        let Some(buffer) = tab.buffer() else { return };
        let caret = self.source.caret_rect(area, buffer, tab.scroll);
        let mut scroll = tab.scroll;
        if caret.top < area.top {
            scroll -= area.top - caret.top;
        } else if caret.bottom > area.bottom {
            scroll += caret.bottom - area.bottom;
        }
        let scroll = scroll.clamp(0.0, self.source.max_scroll(area));
        if let Some(tab) = self.shell.active_mut() {
            tab.scroll = scroll;
        }
    }

    /// 主区内容变了（换标签、换库、切模式），两套排版一起失效。
    ///
    /// 两个都失效而不是只失效"当前那个"：切模式时当前是哪个正在变，
    /// 挑着失效必然会漏。重排本身是惰性的，多失效一个不花钱。
    pub(super) fn invalidate_main(&mut self) {
        self.reset_scrollbars();
        self.editor_ai.invalidate();
        self.editor_ai.timer = None;
        self.doc.invalidate();
        self.source.invalidate();
        self.split.doc.invalidate();
        self.split.source.invalidate();
        // 换了标签/视图：新内容先以渲染态示人，点进去再露源码
        self.editor_engaged = false;
    }
}
