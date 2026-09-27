//! 管理文档和数据表的查找栏、查询及结果定位。
use super::*;

impl App {
    /// 查找条叠在编辑区（含工具栏）右上角。开着就每帧按当前文档重算匹配——
    /// 文档在别处被改（撤销、AI 写入）时计数才不会过期。
    pub(super) fn paint_find_bar(&mut self, chrome: &Chrome, p: &Palette) {
        if self.find.is_none() {
            return;
        }
        let find_only = self.base_viewer_open();
        let text = if find_only {
            None
        } else {
            self.shell
                .active()
                .and_then(|t| t.buffer())
                .map(|b| b.text().to_owned())
        };
        if !find_only && text.is_none() {
            return;
        }
        let Some(f) = self.find.as_mut() else { return };
        if find_only {
            f.replace_mode = false;
        } else if let Some(text) = text.as_deref() {
            f.refresh(text, true);
        }
        let editor = chrome.tree.rect(chrome.editor);
        self.find_layout = findbar::layout_ex(editor, f.replace_mode, find_only);
        let focus = match self.focus {
            Focus::FindQuery => findbar::FocusTarget::Query,
            Focus::FindReplacement => findbar::FocusTarget::Replacement,
            _ => findbar::FocusTarget::None,
        };
        findbar::paint(&mut self.list, &self.find_layout, f, focus, p);
    }

    /// 打开查找条（已开着就切页签并聚焦）。TSX 打开时不把选区带进查找框，只全选旧查询，照它。
    pub(super) fn open_find(&mut self, replace_mode: bool) {
        if !replace_mode
            && (self.state.view == WorkspaceView::MochiAi
                || (self.state.ai_panel_open
                    && self.state.right_panel == RightPanel::Assistant
                    && matches!(self.focus, Focus::AiInput | Focus::AiMessageQuery)))
        {
            if !self.ai.panel.search_open {
                self.ai_toggle_search();
            } else {
                self.focus = Focus::AiMessageQuery;
                self.ai.panel.search_query.buffer.select_all();
            }
            return;
        }
        if self.base_viewer_open() {
            if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
                state.close_menu();
            }
            if self.find.is_none() {
                self.find = Some(findbar::State::new(false));
            } else if let Some(f) = self.find.as_mut() {
                f.replace_mode = false;
            }
            if let Some(f) = self.find.as_mut() {
                f.query.select_all();
            }
            self.focus = Focus::FindQuery;
            self.refresh_base_find();
            return;
        }
        let text = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
            .unwrap_or_default();
        match self.find.as_mut() {
            Some(f) => {
                f.replace_mode = replace_mode;
                f.refresh(&text, true);
            }
            None => {
                self.find = Some(findbar::State::new(replace_mode));
            }
        }
        if let Some(f) = self.find.as_mut() {
            f.query.select_all();
        }
        self.focus = if replace_mode {
            Focus::FindReplacement
        } else {
            Focus::FindQuery
        };
    }

    pub(super) fn close_find(&mut self) {
        self.find = None;
        self.base_find_hits.clear();
        if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
            state.query.clear();
        }
        if matches!(self.focus, Focus::FindQuery | Focus::FindReplacement) {
            self.focus = Focus::Main;
            self.editor_engaged = true;
        }
    }

    pub(super) fn refresh_base_find(&mut self) {
        let query = self
            .find
            .as_ref()
            .map(|f| f.query.text().to_owned())
            .unwrap_or_default();
        let hits = match self.viewer_content_mut() {
            Some(viewer::Content::Base(state)) => {
                state.query = query.clone();
                state.search_cells(&query)
            }
            _ => Vec::new(),
        };
        let count = hits.len();
        self.base_find_hits = hits;
        if let Some(f) = self.find.as_mut() {
            f.set_match_count(count);
        }
    }

    pub(super) fn focus_base_find_hit(&mut self, index: usize) {
        let Some(&(table, record, field)) = self.base_find_hits.get(index) else {
            return;
        };
        let area = self.editor_area;
        if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
            state.focus_cell(table, record, field, area);
        }
    }

    /// 查询变了：重算匹配并选中第一个。
    pub(super) fn find_query_changed(&mut self) {
        if self.base_viewer_open() {
            self.refresh_base_find();
            return;
        }
        let Some(text) = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
        else {
            return;
        };
        let Some(f) = self.find.as_mut() else { return };
        f.refresh(&text, false);
        let first = f.active.map(|i| f.matches[i]);
        if let Some(range) = first {
            self.select_find_match(range);
        }
    }

    /// 编辑之后（撤销、替换……）匹配表要跟着文档变。
    pub(super) fn find_refresh_after_edit(&mut self) {
        if self.find.is_none() {
            return;
        }
        let Some(text) = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
        else {
            return;
        };
        if let Some(f) = self.find.as_mut() {
            f.refresh(&text, true);
        }
    }

    /// 把一个匹配设成编辑器选区并滚进视野（`setTextSelection` + `scrollMatchIntoView`）。
    pub(super) fn select_find_match(&mut self, (a, b): (usize, usize)) {
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(a, false);
            buffer.set_cursor(b, true);
        }
        self.editor_engaged = true;
        // 主区不拿焦点，光标也不画；但活动块要露出来，选区高亮才精确
        let keep = self.focus;
        self.focus = Focus::Main;
        self.after_edit(true);
        self.focus = keep;
    }

    pub(super) fn find_navigate(&mut self, direction: i32) {
        if self.base_viewer_open() {
            self.refresh_base_find();
            let index = {
                let Some(f) = self.find.as_mut() else { return };
                f.navigate(direction).and(f.active)
            };
            if let Some(index) = index {
                self.focus_base_find_hit(index);
            }
            return;
        }
        let Some(text) = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
        else {
            return;
        };
        let Some(f) = self.find.as_mut() else { return };
        f.refresh(&text, true);
        if let Some(range) = f.navigate(direction) {
            self.select_find_match(range);
        }
    }

    pub(super) fn find_replace_current(&mut self) {
        let Some(text) = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
        else {
            return;
        };
        let Some(f) = self.find.as_mut() else { return };
        f.refresh(&text, true);
        if f.matches.is_empty() {
            return;
        }
        let index = f.active.filter(|i| *i < f.matches.len()).unwrap_or(0);
        let (a, b) = f.matches[index];
        let replacement = f.replacement.text().to_owned();
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.replace_range(a..b, &replacement);
        }
        let text = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
            .unwrap_or_default();
        let Some(f) = self.find.as_mut() else { return };
        f.refresh(&text, false);
        f.active = if f.matches.is_empty() {
            None
        } else {
            Some(index.min(f.matches.len() - 1))
        };
        let next = f.active.map(|i| f.matches[i]);
        match next {
            Some(range) => self.select_find_match(range),
            None => {
                let keep = self.focus;
                self.focus = Focus::Main;
                self.after_edit(true);
                self.focus = keep;
            }
        }
    }

    pub(super) fn find_replace_all(&mut self) {
        let Some(text) = self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned())
        else {
            return;
        };
        let Some(f) = self.find.as_mut() else { return };
        f.refresh(&text, true);
        if f.matches.is_empty() {
            return;
        }
        let replacement = f.replacement.text().to_owned();
        // 从后往前替换，前面的偏移才不会失效。整体算一步撤销
        let mut out = text.clone();
        for (a, b) in f.matches.iter().rev() {
            out.replace_range(*a..*b, &replacement);
        }
        if let Some(buffer) = self.shell.active_buffer_mut() {
            let len = buffer.text().len();
            buffer.replace_range(0..len, &out);
            buffer.set_cursor(0, false);
        }
        if let Some(f) = self.find.as_mut() {
            f.matches.clear();
            f.active = None;
        }
        let keep = self.focus;
        self.focus = Focus::Main;
        self.after_edit(true);
        self.focus = keep;
    }

    pub(super) fn on_find_click(&mut self, hit: findbar::Hit, x: f32) {
        match hit {
            findbar::Hit::TabFind => {
                if let Some(f) = self.find.as_mut() {
                    f.replace_mode = false;
                }
                self.focus = Focus::FindQuery;
            }
            findbar::Hit::TabReplace => {
                if self.base_viewer_open() {
                    return;
                }
                if let Some(f) = self.find.as_mut() {
                    f.replace_mode = true;
                }
                self.focus = Focus::FindReplacement;
            }
            findbar::Hit::Close => self.close_find(),
            findbar::Hit::Query => {
                let left = self.find_layout.query_text_left() + 15.0;
                if let Some(f) = self.find.as_mut() {
                    f.query.click(x - left, false);
                }
                self.focus = Focus::FindQuery;
            }
            findbar::Hit::Replacement => {
                let left = self.find_layout.replacement_text_left() + 15.0;
                if let Some(f) = self.find.as_mut() {
                    f.replacement.click(x - left, false);
                }
                self.focus = Focus::FindReplacement;
            }
            findbar::Hit::Prev => self.find_navigate(-1),
            findbar::Hit::Next => self.find_navigate(1),
            findbar::Hit::ReplaceOne => self.find_replace_current(),
            findbar::Hit::ReplaceAll => self.find_replace_all(),
            findbar::Hit::Inside => {}
        }
    }

    /// 查找条里的按键。返回是否消费。
    pub(super) fn on_find_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
        if key == VK_ESCAPE.0 {
            self.close_find();
            return true;
        }
        let replacement_focused = self.focus == Focus::FindReplacement;
        if key == VK_RETURN.0 {
            if replacement_focused {
                self.find_replace_current();
            } else {
                self.find_navigate(if shift { -1 } else { 1 });
            }
            return true;
        }
        let Some(field) = self.focused_field_mut() else {
            return false;
        };
        match field.key(key, shift, ctrl) {
            FieldKey::Edited => {
                if !replacement_focused {
                    self.find_query_changed();
                }
                true
            }
            FieldKey::Ignored => false,
            _ => true,
        }
    }
}
