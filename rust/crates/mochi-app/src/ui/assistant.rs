//! Provider 配置走设置页，不在助手面板内嵌。

mod composer;
mod control_hints;
mod geometry;
mod models;
mod painting;
mod trace_layout;
mod trace_paint;

pub use composer::HEADER_H;
#[allow(unused_imports)]
pub use composer::INPUT_H;
use composer::{
    composer_text_height, pending_selection_height, user_message_body_width, AVATAR_GAP,
    AVATAR_SIZE, COMPOSER_FOOTER_H, INPUT_PAD_BOTTOM, INPUT_PAD_X, MSG_PAD_X, MSG_PAD_Y, NAV_W,
    PENDING_EDIT_CARD_H, ROLE_H, SCROLL_BOTTOM_H, SCROLL_BOTTOM_W, STREAMING_REASONING_TAIL_CHARS,
    TRACE_HEADER_H, TRACE_MAX_COLLAPSED_CHARS, TRACE_PAD_X, TRACE_PAD_Y, TRACE_ROW_GAP,
};
use control_hints::{control_tooltip, paint_control_tooltip};
pub use geometry::layout;
#[allow(unused_imports)]
pub use models::TextSelection;
pub use models::{Hit, LaidFollowUp, LaidMessage, Layout, MessageAction, State, Streaming};
pub use painting::paint;
use trace_layout::{
    activity_height, format_duration, json_string, legacy_streaming_trace, persisted_trace,
    plan_height, plan_steps, streaming_reasoning_text, thinking_height, tool_height,
    trace_duration, trace_height, trace_kind, wrapped_height, wrapped_plain_lines,
};
pub use trace_layout::{plan_lines, trace_summary};
use trace_paint::{paint_activity, paint_trace_card, paint_wrapped_text};

use mochi_core::ai::agent_runner::AgentTraceStep;
use mochi_core::ai::session::{AiConversation, AiSessionMeta};
use std::collections::HashSet;

use super::ai_images::{image_layout, pending_height, pending_rects, ImagePreview};
use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

pub(super) fn message_spacing() -> f32 {
    crate::ui::settings_values::number("assistant.messageSpacing", 26.0).clamp(0.0, 64.0)
}

pub(super) fn message_padding() -> f32 {
    crate::ui::settings_values::number("assistant.messagePadding", 10.0).clamp(4.0, 32.0)
}

pub(super) fn message_radius() -> f32 {
    crate::ui::settings_values::number("assistant.messageRadius", 10.0).clamp(0.0, 24.0)
}

pub(super) fn card_radius() -> f32 {
    crate::ui::settings_values::number("assistant.cardRadius", 8.0).clamp(0.0, 24.0)
}

pub(super) fn user_bubble_color(p: &Palette) -> u32 {
    crate::ui::settings_values::color(
        "assistant.userBubbleColor",
        theme::mix(p.accent, p.surface, 0.10),
    )
}

/// 文档选区与已持久化评审卡片的协议/绘制辅助。
/// 副作用归 App 层；本面板只负责几何与外观。
pub(crate) mod context;

#[cfg(test)]
mod message_action_tests;

#[cfg(test)]
mod tests;
