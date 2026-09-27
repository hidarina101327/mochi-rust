//! 定义导出对话框的状态、布局和操作命中检测。
use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    text,
    theme::Palette,
};
use mochi_core::exports::enrichment::{ConversationMode, Options};
use std::path::PathBuf;
#[derive(Debug, Clone)]
pub struct State {
    pub source: PathBuf,
    pub options: Options,
    pub focus: usize,
    pub hover: Option<usize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Cancel,
    Submit,
}

fn large_title_extra() -> f32 {
    (TextStyle::Large.line_height() - 28.0).max(0.0)
}

impl State {
    pub fn new(source: PathBuf, format: String) -> Self {
        let focus = ["pdf", "html", "markdown"]
            .iter()
            .position(|s| *s == format)
            .unwrap_or(0);
        Self {
            source,
            options: Options {
                format,
                ..Default::default()
            },
            focus,
            hover: None,
        }
    }
    pub fn rect(&self, viewport: Rect) -> Rect {
        let w = 400.0_f32.min(viewport.width() - 24.0);
        let h = 498.0 + large_title_extra();
        Rect::from_size(
            (viewport.left + viewport.right - w) / 2.0,
            (viewport.top + viewport.bottom - h) / 2.0,
            w,
            h,
        )
    }
    pub fn parts(&self, viewport: Rect) -> Vec<(usize, Rect)> {
        let r = self.rect(viewport);
        let title_extra = large_title_extra();
        let left = r.left + 24.0;
        let right = r.right - 24.0;
        let width = (right - left - 16.0) / 3.0;
        let mut out = (0..3)
            .map(|i| {
                (
                    i,
                    Rect::from_size(
                        left + i as f32 * (width + 8.0),
                        r.top + 88.0 + title_extra,
                        width,
                        36.0,
                    ),
                )
            })
            .collect::<Vec<_>>();
        if self.options.format != "markdown" {
            out.push((
                3,
                Rect::new(
                    left,
                    r.top + 138.0 + title_extra,
                    right,
                    r.top + 194.0 + title_extra,
                ),
            ));
        }
        for i in 0..3 {
            out.push((
                4 + i,
                Rect::new(
                    left,
                    r.top + 228.0 + title_extra + i as f32 * 62.0,
                    right,
                    r.top + 284.0 + title_extra + i as f32 * 62.0,
                ),
            ));
        }
        out.push((
            7,
            Rect::new(
                right - 168.0,
                r.bottom - 60.0,
                right - 88.0,
                r.bottom - 24.0,
            ),
        ));
        out.push((
            8,
            Rect::new(right - 80.0, r.bottom - 60.0, right, r.bottom - 24.0),
        ));
        out
    }
    pub fn hit(&self, viewport: Rect, x: f32, y: f32) -> Option<usize> {
        if !self.rect(viewport).contains(x, y) {
            return Some(7);
        }
        let r = self.rect(viewport);
        if Rect::from_size(r.right - 40.0, r.top + 16.0, 24.0, 24.0).contains(x, y) {
            return Some(7);
        }
        self.parts(viewport)
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(i, _)| i)
    }
    pub fn activate(&mut self, index: usize) -> Option<Action> {
        self.focus = index;
        match index {
            0..=2 => {
                self.options.format = ["pdf", "html", "markdown"][index].into();
            }
            3 => {
                if self.options.format != "markdown" {
                    self.options.comments = !self.options.comments
                }
            }
            4..=6 => {
                self.options.conversation = [
                    ConversationMode::None,
                    ConversationMode::Answer,
                    ConversationMode::QuestionAnswer,
                ][index - 4]
            }
            7 => return Some(Action::Cancel),
            8 => return Some(Action::Submit),
            _ => {}
        }
        None
    }
    pub fn key(&mut self, key: u16, shift: bool) -> Option<Action> {
        match key {
            0x1b => Some(Action::Cancel),
            0x0d => Some(Action::Submit),
            0x20 => self.activate(self.focus),
            0x09 => {
                loop {
                    self.focus = if shift {
                        (self.focus + 8) % 9
                    } else {
                        (self.focus + 1) % 9
                    };
                    if self.focus != 3 || self.options.format != "markdown" {
                        break;
                    }
                }
                None
            }
            _ => None,
        }
    }
    pub fn paint(&self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        let r = self.rect(viewport);
        let title_extra = large_title_extra();
        list.rect_alpha(viewport, 0, 0.4);
        list.rounded_rect(r, 8.0, p.surface);
        list.rounded_border(r, 8.0, p.border);
        list.icon_centered(
            Rect::from_size(r.right - 40.0, r.top + 16.0, 24.0, 24.0),
            super::icons::Icon::X,
            16.0,
            p.muted,
        );
        let left = r.left + 24.0;
        let right = r.right - 24.0;
        list.text(
            Rect::new(left, r.top + 20.0, right, r.top + 48.0 + title_extra),
            "导出",
            TextStyle::Large,
            p.foreground,
        );
        let name = self
            .source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        list.text(
            Rect::new(
                left,
                r.top + 51.0 + title_extra,
                right,
                r.top + 75.0 + title_extra,
            ),
            text::ellipsize(&name, TextStyle::Label, right - left),
            TextStyle::Label,
            p.muted,
        );
        list.text(
            Rect::new(
                left,
                r.top + 201.0 + title_extra,
                right,
                r.top + 226.0 + title_extra,
            ),
            "AI 对话定位块",
            TextStyle::Label,
            p.foreground,
        );
        if self.options.format == "markdown" {
            list.text(
                Rect::new(
                    left,
                    r.top + 143.0 + title_extra,
                    right,
                    r.top + 190.0 + title_extra,
                ),
                "Markdown 保留原文格式，不附带评论。",
                TextStyle::Caption,
                p.muted,
            );
        }
        for (index, box_) in self.parts(viewport) {
            let selected = match index {
                0..=2 => self.options.format == ["pdf", "html", "markdown"][index],
                3 => self.options.comments,
                4 => self.options.conversation == ConversationMode::None,
                5 => self.options.conversation == ConversationMode::Answer,
                6 => self.options.conversation == ConversationMode::QuestionAnswer,
                _ => false,
            };
            let fill = if index == 8 {
                p.accent
            } else if self.hover == Some(index) {
                p.surface_muted
            } else {
                p.surface
            };
            if index == 8 {
                list.glass_button(box_, 5.0, p, self.hover == Some(index));
            } else {
                list.rounded_rect(box_, 5.0, fill);
            }
            if index != 8 || self.focus == index {
                list.rounded_border(
                    box_,
                    5.0,
                    if selected || self.focus == index {
                        p.accent
                    } else {
                        p.border
                    },
                );
            }
            let label = [
                "PDF",
                "HTML",
                "Markdown",
                "保留评论",
                "不写入 AI 对话",
                "仅答案",
                "问题－答案",
                "取消",
                "导出",
            ][index];
            let inset = if index < 7 { 30.0 } else { 16.0 };
            if index < 7 {
                let circle = Rect::from_size(box_.left + 10.0, box_.top + 12.0, 12.0, 12.0);
                list.rounded_border(circle, if index == 3 { 2.0 } else { 6.0 }, p.border);
                if selected {
                    if index == 3 {
                        list.icon_centered(circle, super::icons::Icon::CHECK, 12.0, p.accent);
                    } else {
                        list.rounded_rect(
                            Rect::from_size(circle.left + 3.0, circle.top + 3.0, 6.0, 6.0),
                            3.0,
                            p.accent,
                        );
                    }
                }
            }
            list.text(
                Rect::new(
                    box_.left + inset,
                    box_.top,
                    box_.right - 8.0,
                    box_.top + 36.0,
                ),
                label,
                TextStyle::Label,
                if index == 8 {
                    p.button_foreground()
                } else {
                    p.foreground
                },
            );
            let detail = match index {
                3 => "附带评论对象、回复和附件名称。",
                4 => "导出时移除定位块，不附带会话内容。",
                5 => "仅插入定位块对应的 AI 回复。",
                6 => "插入同一轮的用户问题与 AI 回复。",
                _ => "",
            };
            if !detail.is_empty() {
                list.text(
                    Rect::new(
                        box_.left + inset,
                        box_.top + 31.0,
                        box_.right - 8.0,
                        box_.bottom - 3.0,
                    ),
                    detail,
                    TextStyle::Caption,
                    p.muted,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choices_keyboard_and_geometry_stay_inside_the_dialog() {
        let mut state = State::new("笔记.md".into(), "pdf".into());
        assert_eq!(state.options.conversation, ConversationMode::None);
        assert!(!state.options.comments);
        state.activate(3);
        state.activate(6);
        assert!(state.options.comments);
        assert_eq!(state.options.conversation, ConversationMode::QuestionAnswer);
        state.activate(2);
        assert!(state.options.comments); // Markdown 格式下隐藏；切回其他格式时恢复。
        assert!(!state
            .parts(Rect::from_size(0.0, 0.0, 1200.0, 800.0))
            .iter()
            .any(|(i, _)| *i == 3));
        state.key(9, false);
        assert_eq!(state.focus, 4);
        for viewport in [
            Rect::from_size(0.0, 0.0, 1200.0, 800.0),
            Rect::from_size(0.0, 0.0, 800.0, 600.0),
        ] {
            let r = state.rect(viewport);
            assert!(r.top >= viewport.top && r.bottom <= viewport.bottom);
            for (i, b) in state.parts(viewport) {
                assert!(
                    b.left >= r.left
                        && b.right <= r.right
                        && b.top >= r.top
                        && b.bottom <= r.bottom
                );
                assert_eq!(
                    state.hit(viewport, (b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0),
                    Some(i)
                );
            }
        }
        assert_eq!(state.key(27, false), Some(Action::Cancel));
        assert_eq!(state.key(13, false), Some(Action::Submit));
    }
}
