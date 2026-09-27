//! 定义 AI 助手输入区和消息区域使用的尺寸与间距。
use super::*;

pub const HEADER_H: f32 = 56.0;

pub(super) const MSG_PAD_X: f32 = 20.0;

pub(super) const MSG_PAD_Y: f32 = 24.0;

pub(super) const ROLE_H: f32 = 18.0;

pub(super) const INPUT_PAD_X: f32 = 20.0;

pub(super) const INPUT_PAD_BOTTOM: f32 = 12.0;

pub(super) const COMPOSER_FOOTER_H: f32 = 32.0;

const COMPOSER_MIN_TEXT_H: f32 = 34.0;

const COMPOSER_MAX_TEXT_H: f32 = 140.0;

const INPUT_WRAPPER_H: f32 = 8.0 + 36.0 + 8.0;

pub const INPUT_H: f32 = 1.0 + 12.0 + INPUT_WRAPPER_H + INPUT_PAD_BOTTOM;

const AGENT_TOOLS_H: f32 = 0.0;

pub(super) const PENDING_EDIT_CARD_H: f32 = 116.0;

pub(super) const AVATAR_SIZE: f32 = 24.0;

pub(super) const AVATAR_GAP: f32 = 8.0;

pub(super) const TRACE_HEADER_H: f32 = 30.0;

pub(super) const TRACE_ROW_GAP: f32 = 4.0;

pub(super) const TRACE_PAD_X: f32 = 10.0;

pub(super) const TRACE_PAD_Y: f32 = 8.0;

pub(super) const TRACE_MAX_COLLAPSED_CHARS: usize = 240;

pub(super) const STREAMING_REASONING_TAIL_CHARS: usize = 600;

pub(super) const SCROLL_BOTTOM_H: f32 = 30.0;

pub(super) const SCROLL_BOTTOM_W: f32 = 112.0;

pub(super) const NAV_W: f32 = 220.0;

pub(super) fn composer_text_height(s: &State, width: f32) -> f32 {
    s.input
        .multiline_height(width)
        .clamp(COMPOSER_MIN_TEXT_H, COMPOSER_MAX_TEXT_H)
}

fn wrapper_height(s: &State, area: Rect) -> f32 {
    let width = (area.width() - INPUT_PAD_X * 2.0 - 24.0).max(40.0);
    composer_text_height(s, width)
}

fn selection_chip_area(area: Rect) -> Rect {
    Rect::new(
        area.left + INPUT_PAD_X,
        0.0,
        (area.right - INPUT_PAD_X).max(area.left + INPUT_PAD_X),
        0.0,
    )
}

fn pending_selection_layout(s: &State, area: Rect, top: f32) -> context::SelectionChipsLayout {
    context::layout_selection_chips(&s.pending_selections, selection_chip_area(area), top, true)
}

pub(super) fn pending_selection_height(s: &State, area: Rect) -> f32 {
    let layout = pending_selection_layout(s, area, 0.0);
    if layout.height > 0.0 {
        layout.height + 8.0
    } else {
        0.0
    }
}

/// 用户消息用紧凑的右对齐气泡。在换行上限处只排一次 Markdown，
/// 以最宽的渲染 run 作为自然宽度；长内容仍然最多增长到常规的 80% 上限。
pub(super) fn user_message_body_width(source: &str, max_width: f32, standalone: bool) -> f32 {
    let max_width = max_width.max(40.0);
    let natural_width = crate::ui::ai_markdown::layout(source, max_width, standalone)
        .selectable_text(None)
        .into_iter()
        .map(|fragment| fragment.rect.right)
        .reduce(f32::max)
        .unwrap_or(max_width);
    (natural_width.ceil() + 1.0).clamp(40.0, max_width)
}
