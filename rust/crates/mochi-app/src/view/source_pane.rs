//! 维护和绘制文档源码视图，并提供光标和滚动位置。
use super::*;

/// 源码编辑面。排版 key 里带**内容长度**：每敲一个字都要重排（行会变），
/// 切换标签、拖窗口宽度也要重排，三者共用一个判据。长度相同而内容不同的编辑
/// （比如替换一个等长的字）靠 [`Self::invalidate`] 显式失效。
#[derive(Default)]
pub struct SourcePane {
    pub(super) layout: editor::EditorLayout,
    pub(super) key: Option<(usize, u32, usize)>,
    /// 上下移动光标时保持不变的目标横坐标。
    pub(super) desired_x: f32,
}

impl SourcePane {
    pub fn invalidate(&mut self) {
        self.key = None;
    }

    pub fn ensure(&mut self, area: Rect, tab: usize, buffer: &TextBuffer) {
        let width = editor::content_width(area);
        let key = (tab, width.round() as u32, buffer.text().len());
        if self.key == Some(key) {
            return;
        }
        let (shown, _) = buffer.display_text();
        self.layout = editor::layout(&shown, theme::EDIT_STYLE, width);
        self.key = Some(key);
    }

    pub fn paint(
        &self,
        list: &mut DrawList,
        area: Rect,
        buffer: &TextBuffer,
        scroll: f32,
        focused: bool,
        p: &Palette,
    ) {
        editor::paint(
            list,
            area,
            buffer,
            &self.layout,
            theme::EDIT_STYLE,
            scroll,
            focused,
            p,
        );
    }

    pub fn caret_rect(&self, area: Rect, buffer: &TextBuffer, scroll: f32) -> Rect {
        editor::caret_rect(area, &self.layout, buffer, theme::EDIT_STYLE, scroll)
    }

    pub fn max_scroll(&self, area: Rect) -> f32 {
        editor::max_scroll(&self.layout, area)
    }

    /// 点击定位光标。
    pub fn click(
        &mut self,
        area: Rect,
        buffer: &mut TextBuffer,
        scroll: f32,
        x: f32,
        y: f32,
        extend: bool,
    ) {
        let style = theme::EDIT_STYLE;
        let line = self
            .layout
            .line_at(y - area.top - editor::EDITOR_PADDING + scroll, style);
        let offset = self
            .layout
            .offset_at(line, x - area.left - editor::EDITOR_PADDING, style);
        buffer.set_cursor(offset, extend);
        self.desired_x = self.layout.locate(offset, style).1;
    }

    /// 重设目标横坐标。左右移动或编辑之后调（必须在重排之后）。
    pub fn sync_desired_x(&mut self, cursor: usize) {
        self.desired_x = self.layout.locate(cursor, theme::EDIT_STYLE).1;
    }

    /// 编辑相关的按键。
    ///
    /// `buffer` 显式传进来而不是从 `Shell` 里取：`SourcePane` 和 `Shell` 是
    /// `App` 上两个互不相干的字段，分别可变借用是合法的——原先两者揉在一个
    /// struct 里，只能靠 `mem::take` 把排版结果搬出去再搬回来。
    pub fn handle_key(
        &self,
        buffer: &mut TextBuffer,
        key: u16,
        shift: bool,
        ctrl: bool,
    ) -> KeyOutcome {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB,
            VK_UP,
        };
        let style = theme::EDIT_STYLE;

        match key {
            k if k == VK_LEFT.0 => {
                buffer.move_left(shift);
                KeyOutcome::MOVED
            }
            k if k == VK_RIGHT.0 => {
                buffer.move_right(shift);
                KeyOutcome::MOVED
            }
            k if k == VK_UP.0 || k == VK_DOWN.0 => {
                let delta = if k == VK_UP.0 { -1 } else { 1 };
                let next = self
                    .layout
                    .move_vertical(buffer.cursor(), delta, self.desired_x, style);
                buffer.set_cursor(next, shift);
                KeyOutcome::MOVED_VERTICALLY
            }
            k if k == VK_HOME.0 => {
                let (line, _) = self.layout.locate(buffer.cursor(), style);
                let (start, _) = self.layout.line_bounds(line);
                buffer.set_cursor(if ctrl { 0 } else { start }, shift);
                KeyOutcome::MOVED
            }
            k if k == VK_END.0 => {
                let (line, _) = self.layout.locate(buffer.cursor(), style);
                let (_, end) = self.layout.line_bounds(line);
                buffer.set_cursor(if ctrl { buffer.text().len() } else { end }, shift);
                KeyOutcome::MOVED
            }
            k if k == VK_BACK.0 => {
                buffer.delete_backward();
                KeyOutcome::EDITED
            }
            k if k == VK_DELETE.0 => {
                buffer.delete_forward();
                KeyOutcome::EDITED
            }
            k if k == VK_RETURN.0 => {
                buffer.insert("\n");
                KeyOutcome::EDITED
            }
            k if k == VK_TAB.0 => {
                buffer.insert("  ");
                KeyOutcome::EDITED
            }
            _ if ctrl && key == b'A' as u16 => {
                buffer.select_all();
                KeyOutcome::MOVED
            }
            _ => KeyOutcome::IGNORED,
        }
    }
}
