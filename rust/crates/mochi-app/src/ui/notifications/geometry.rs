//! 计算通知列表、通知条目和命中区域的布局。
use super::{Filter, Hit, Mode, State};
use crate::ui::{
    draw::TextStyle,
    layout::Rect,
    overlay_scrollbar::{Axis, Bar},
    text,
};

pub const ROW_HEIGHT: f32 = 82.0;

fn title_extra_height() -> f32 {
    (TextStyle::Large.line_height() - 28.0).max(0.0)
}

pub struct Layout {
    pub frame: Rect,
    pub body: Rect,
    pub controls: Vec<(Rect, Hit)>,
    pub rows: Vec<(Rect, u64)>,
    pub detail_lines: Vec<String>,
    pub max_scroll: f32,
    pub scroll: f32,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.controls
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
            .or_else(|| {
                self.body
                    .contains(x, y)
                    .then(|| {
                        self.rows
                            .iter()
                            .find(|(r, _)| r.contains(x, y))
                            .map(|(_, id)| Hit::Notice(*id))
                    })
                    .flatten()
            })
    }
    pub fn bars(&self) -> Vec<((), Bar)> {
        Bar::new(
            self.body,
            Axis::Vertical,
            self.max_scroll,
            self.scroll,
            false,
        )
        .map(|b| vec![((), b)])
        .unwrap_or_default()
    }
}

pub fn layout(state: &State, viewport: Rect, anchor: Rect) -> Layout {
    let margin = 12.0_f32
        .min(viewport.width() / 8.0)
        .min(viewport.height() / 8.0);
    let center = state.mode == Mode::Center;
    let width = (if center { 660.0_f32 } else { 380.0_f32 })
        .min((viewport.width() - margin * 2.0).max(0.0));
    let height = (if center { 580.0_f32 } else { 440.0_f32 })
        .min((viewport.height() - margin * 2.0).max(0.0));
    let left = if center {
        viewport.left + (viewport.width() - width) / 2.0
    } else {
        (anchor.right - width).clamp(
            viewport.left + margin,
            (viewport.right - margin - width).max(viewport.left + margin),
        )
    };
    let top = if center {
        viewport.top + (viewport.height() - height) / 2.0
    } else {
        (anchor.bottom + 8.0)
            .min(viewport.bottom - margin - height)
            .max(viewport.top + margin)
    };
    let frame = Rect::from_size(left, top, width, height);
    let mut controls = vec![(
        Rect::from_size(frame.right - 42.0, top + 14.0, 28.0, 28.0),
        Hit::Close,
    )];
    if state.history.unread() > 0 {
        controls.push((
            Rect::from_size(frame.right - 134.0, top + 14.0, 86.0, 28.0),
            Hit::MarkAllRead,
        ));
    }
    let mut body_top = top + 62.0 + title_extra_height();
    let mut detail_lines = Vec::new();
    if let Some(n) = state.selected_notice() {
        controls.push((
            Rect::from_size(left + 16.0, body_top, 90.0, 30.0),
            Hit::Back,
        ));
        controls.push((
            Rect::from_size(frame.right - 168.0, body_top, 152.0, 30.0),
            Hit::ToggleCategory(n.category),
        ));
        body_top += 46.0;
        for paragraph in n.message.split('\n') {
            if paragraph.is_empty() {
                detail_lines.push(String::new());
                continue;
            }
            detail_lines.extend(
                text::wrap_source(paragraph, TextStyle::Label, (width - 48.0).max(1.0))
                    .iter()
                    .map(|runs| runs.iter().map(|r| r.text.as_str()).collect()),
            );
        }
    } else if center {
        let columns = if width < 480.0 { 3 } else { 6 };
        let cell = (width - 32.0).max(0.0) / columns as f32;
        for (i, filter) in Filter::ALL.iter().enumerate() {
            controls.push((
                Rect::from_size(
                    left + 16.0 + (i % columns) as f32 * cell,
                    body_top + (i / columns) as f32 * 36.0,
                    (cell - 4.0).max(0.0),
                    30.0,
                ),
                Hit::Filter(*filter),
            ));
        }
        body_top += Filter::ALL.len().div_ceil(columns) as f32 * 36.0 + 8.0;
    }
    let footer = 44.0;
    let body_bottom = (frame.bottom - footer).max(body_top.min(frame.bottom));
    let body = Rect::new(
        left + 12.0,
        body_top.min(body_bottom),
        frame.right - 12.0,
        body_bottom,
    );
    let entries: Vec<_> = state
        .history
        .entries
        .iter()
        .filter(|n| !center || state.filter.matches(n))
        .take(if center { usize::MAX } else { 5 })
        .collect();
    let content_height = if state.selected_notice().is_some() {
        detail_lines.len() as f32 * 24.0 + 88.0 + title_extra_height()
    } else {
        entries.len() as f32 * ROW_HEIGHT
    };
    let max_scroll = (content_height - body.height()).max(0.0);
    let scroll = if state.scroll.is_finite() {
        state.scroll.clamp(0.0, max_scroll)
    } else {
        0.0
    };
    let rows = if state.selected_notice().is_some() {
        Vec::new()
    } else {
        entries
            .iter()
            .enumerate()
            .map(|(i, n)| {
                (
                    Rect::from_size(
                        body.left,
                        body.top + i as f32 * ROW_HEIGHT - scroll,
                        body.width(),
                        ROW_HEIGHT,
                    ),
                    n.id,
                )
            })
            .collect()
    };
    if !center {
        controls.push((
            Rect::new(
                left + 12.0,
                frame.bottom - 38.0,
                frame.right - 112.0,
                frame.bottom - 8.0,
            ),
            Hit::More,
        ));
    }
    controls.push((
        Rect::from_size(frame.right - 104.0, frame.bottom - 38.0, 92.0, 30.0),
        Hit::Settings,
    ));
    Layout {
        frame,
        body,
        controls,
        rows,
        detail_lines,
        max_scroll,
        scroll,
    }
}
