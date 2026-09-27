//! 定义桌面卡片的配色、背景和行布局尺寸。
use super::{Hit, State};
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    theme::{self, Palette},
};
use mochi_core::desktop_cards::GLASS_BACKGROUND;

const COLORS: [Option<u32>; 8] = [
    None,
    Some(0x20242b),
    Some(0xffffff),
    Some(0x64748b),
    Some(0x2563eb),
    Some(0x15803d),
    Some(0xbe123c),
    Some(0x9333ea),
];
const BACKGROUNDS: [Option<u32>; 9] = [
    Some(GLASS_BACKGROUND),
    None,
    Some(0xf5f5f2),
    Some(0xe7e2d8),
    Some(0xd6e5ee),
    Some(0xdde7df),
    Some(0xe4dfe9),
    Some(0x30343b),
    Some(0x152c42),
];
const LABEL_WIDTH: f32 = 60.0;
const ROW_HEIGHT: f32 = 26.0;
const TRACK_TOP: f32 = 78.0;
const COLOR_SIZE: f32 = 20.0;

pub(super) fn track_rect(body: Rect, row: usize) -> Rect {
    Rect::from_size(
        body.left + LABEL_WIDTH + 16.0,
        body.top + TRACK_TOP + row as f32 * ROW_HEIGHT,
        (body.width() - LABEL_WIDTH - 76.0).clamp(1.0, 150.0),
        ROW_HEIGHT,
    )
}

pub fn controls(body: Rect) -> Vec<(Rect, Hit)> {
    let mut controls = Vec::new();
    for (row, hit, minus, plus) in [
        (0, Hit::OpacityTrack(0.0), Hit::Opacity(-1), Hit::Opacity(1)),
        (
            1,
            Hit::FontSizeTrack(0.0),
            Hit::FontSize(-1),
            Hit::FontSize(1),
        ),
    ] {
        let track = track_rect(body, row);
        controls.push((track, hit));
        controls.push((
            Rect::from_size(track.left - 16.0, track.top, 12.0, ROW_HEIGHT),
            minus,
        ));
        controls.push((
            Rect::from_size(track.right + 4.0, track.top, 12.0, ROW_HEIGHT),
            plus,
        ));
    }
    for (i, hit) in COLORS
        .into_iter()
        .map(Hit::FontColor)
        .chain([Hit::FontCustom])
        .enumerate()
    {
        controls.push((
            Rect::from_size(
                body.left + LABEL_WIDTH + i as f32 * 22.0,
                body.top + TRACK_TOP + 2.0 * ROW_HEIGHT,
                COLOR_SIZE,
                COLOR_SIZE,
            ),
            hit,
        ));
    }
    for (i, color) in BACKGROUNDS.into_iter().enumerate() {
        controls.push((
            Rect::from_size(
                body.left + LABEL_WIDTH + i as f32 * 22.0,
                body.top + 156.0,
                COLOR_SIZE,
                COLOR_SIZE,
            ),
            Hit::BackgroundColor(color),
        ));
    }
    controls.push((
        Rect::from_size(
            body.left + LABEL_WIDTH + BACKGROUNDS.len() as f32 * 22.0,
            body.top + 156.0,
            COLOR_SIZE,
            COLOR_SIZE,
        ),
        Hit::BackgroundCustom,
    ));
    let width = (body.width().min(370.0) - LABEL_WIDTH - 8.0) / 3.0;
    for i in 0..3 {
        controls.push((
            Rect::from_size(
                body.left + LABEL_WIDTH + i as f32 * width,
                body.top + 188.0,
                width - 4.0,
                28.0,
            ),
            Hit::Spacing(i as u8),
        ));
    }
    let width = body.width().min(370.0) / 3.0;
    for (i, hit) in [Hit::ToggleDetails, Hit::ToggleSeparators, Hit::ToggleBorder]
        .into_iter()
        .enumerate()
    {
        controls.push((
            Rect::from_size(
                body.left + i as f32 * width,
                body.top + 222.0,
                width - 4.0,
                28.0,
            ),
            hit,
        ));
    }
    controls
}

/// 绘制出的滑轨与其命中区域共用同一水平范围。
pub fn track_hit_value(body: Rect, row: usize, x: f32) -> f32 {
    let track = track_rect(body, row);
    ((x - track.left) / track.width()).clamp(0.0, 1.0)
}

pub fn paint(list: &mut DrawList, state: &State, body: Rect, p: &Palette) {
    let Some(card) = state.selected_card_ref() else {
        return;
    };
    for (row, label, value, min, max, suffix, active) in [
        (
            0,
            "不透明度",
            card.appearance.opacity,
            35,
            100,
            "%",
            matches!(state.hover, Some(Hit::OpacityTrack(_) | Hit::Opacity(_)))
                || matches!(state.focused, Some(Hit::OpacityTrack(_))),
        ),
        (
            1,
            "字号",
            card.appearance.font_size,
            8,
            72,
            " px",
            matches!(state.hover, Some(Hit::FontSizeTrack(_) | Hit::FontSize(_)))
                || matches!(state.focused, Some(Hit::FontSizeTrack(_))),
        ),
    ] {
        let track = track_rect(body, row);
        list.text(
            Rect::new(body.left, track.top, body.left + LABEL_WIDTH, track.bottom),
            label,
            TextStyle::Caption,
            p.muted,
        );
        paint_track(list, track, value, min, max, active, p);
        list.text(
            Rect::from_size(track.right + 20.0, track.top, 40.0, ROW_HEIGHT),
            format!("{value}{suffix}"),
            TextStyle::Caption,
            p.foreground,
        );
    }
    list.text(
        Rect::from_size(
            body.left,
            body.top + TRACK_TOP + 2.0 * ROW_HEIGHT,
            LABEL_WIDTH,
            COLOR_SIZE,
        ),
        "字体颜色",
        TextStyle::Caption,
        p.muted,
    );
    list.text(
        Rect::from_size(body.left, body.top + 156.0, LABEL_WIDTH, 20.0),
        "背景颜色",
        TextStyle::Caption,
        p.muted,
    );
    list.text(
        Rect::from_size(body.left, body.top + 188.0, LABEL_WIDTH, 28.0),
        "行距",
        TextStyle::Caption,
        p.muted,
    );
    if body.width() >= 650.0 {
        paint_preview(list, card, body, p);
    }
    for (rect, hit) in controls(body) {
        let hovered = state.hover == Some(hit) || state.focused == Some(hit);
        match hit {
            Hit::FontColor(color) | Hit::BackgroundColor(color) => {
                let selected = if matches!(hit, Hit::BackgroundColor(_)) {
                    card.appearance.background_color == color
                } else {
                    card.appearance.font_color == color
                };
                list.rounded_rect(rect, 10.0, color.unwrap_or(p.surface_muted));
                if matches!(hit, Hit::BackgroundColor(Some(GLASS_BACKGROUND))) {
                    list.rounded_rect_alpha(
                        Rect::new(
                            rect.left + 3.0,
                            rect.top + 2.0,
                            rect.right - 3.0,
                            rect.top + 9.0,
                        ),
                        4.0,
                        0xffffff,
                        0.28,
                    );
                }
                list.rounded_border(
                    rect,
                    10.0,
                    if selected || hovered {
                        p.accent
                    } else {
                        p.border
                    },
                );
                if color.is_none() {
                    list.text_aligned(rect, "随", TextStyle::Caption, p.foreground, Align::Center);
                } else if selected {
                    let contrast = if color.is_some_and(|c| {
                        ((c >> 16) & 255) * 299 + ((c >> 8) & 255) * 587 + (c & 255) * 114 > 150000
                    }) {
                        0x20242b
                    } else {
                        0xffffff
                    };
                    list.icon_centered(rect, Icon::CHECK, 12.0, contrast);
                }
                if selected {
                    list.rounded_border(
                        Rect::new(
                            rect.left - 1.0,
                            rect.top - 1.0,
                            rect.right + 1.0,
                            rect.bottom + 1.0,
                        ),
                        11.0,
                        p.accent,
                    );
                }
            }
            Hit::FontCustom | Hit::BackgroundCustom => {
                let selected = if hit == Hit::BackgroundCustom {
                    !BACKGROUNDS.contains(&card.appearance.background_color)
                } else {
                    !COLORS.contains(&card.appearance.font_color)
                };
                list.rounded_rect(
                    rect,
                    5.0,
                    if hovered || selected {
                        p.surface_muted
                    } else {
                        p.surface
                    },
                );
                if selected || state.focused == Some(hit) {
                    list.rounded_border(rect, 5.0, p.accent);
                }
                list.icon_centered(
                    rect,
                    Icon::PALETTE,
                    14.0,
                    if hovered || selected {
                        p.accent
                    } else {
                        p.muted
                    },
                );
            }
            Hit::Spacing(value) => {
                let selected = card.appearance.spacing == value;
                list.rounded_rect(
                    rect,
                    5.0,
                    if selected || hovered {
                        p.surface_muted
                    } else {
                        p.surface
                    },
                );
                if selected || state.focused == Some(hit) {
                    list.rounded_border(rect, 5.0, p.muted);
                }
                list.text_aligned(
                    rect,
                    ["紧凑", "舒适", "宽松"][value as usize],
                    TextStyle::Caption,
                    p.foreground,
                    Align::Center,
                );
            }
            Hit::ToggleDetails | Hit::ToggleBorder | Hit::ToggleSeparators => {
                let (selected, label) = match hit {
                    Hit::ToggleDetails => (card.appearance.show_details, "详情"),
                    Hit::ToggleBorder => (card.appearance.show_border, "边框"),
                    _ => (card.appearance.show_separators, "分隔线"),
                };
                if hovered {
                    list.rounded_rect(rect, 5.0, p.surface_muted);
                }
                if state.focused == Some(hit) {
                    list.rounded_border(rect, 5.0, p.muted);
                }
                let check = Rect::from_size(rect.left + 4.0, rect.top + 6.0, 16.0, 16.0);
                list.rounded_border(check, 4.0, p.muted);
                if selected {
                    list.icon_centered(check, Icon::CHECK, 12.0, p.foreground);
                }
                list.text(
                    Rect::new(rect.left + 26.0, rect.top, rect.right, rect.bottom),
                    label,
                    TextStyle::Caption,
                    p.foreground,
                );
            }
            Hit::Opacity(delta) | Hit::FontSize(delta) => {
                let active = match hit {
                    Hit::Opacity(_) => {
                        matches!(state.hover, Some(Hit::Opacity(_) | Hit::OpacityTrack(_)))
                    }
                    _ => matches!(state.hover, Some(Hit::FontSize(_) | Hit::FontSizeTrack(_))),
                };
                if active {
                    if hovered {
                        list.rounded_rect(rect, 3.0, p.surface_muted);
                    }
                    list.icon_centered(
                        rect,
                        if delta < 0 { Icon::MINUS } else { Icon::PLUS },
                        10.0,
                        if hovered { p.accent } else { p.muted },
                    );
                }
            }
            _ => {}
        }
    }
}

fn paint_track(
    list: &mut DrawList,
    rect: Rect,
    value: u8,
    min: u8,
    max: u8,
    active: bool,
    p: &Palette,
) {
    let center = (rect.top + rect.bottom) / 2.0;
    let rail = Rect::new(rect.left, center - 2.0, rect.right, center + 2.0);
    let fraction = ((value as f32 - min as f32) / (max - min) as f32).clamp(0.0, 1.0);
    let thumb_x = rect.left + rect.width() * fraction;
    list.rounded_rect(
        rail,
        2.0,
        theme::mix(p.border, p.surface_muted, if active { 0.85 } else { 0.55 }),
    );
    if fraction > 0.0 {
        list.rounded_rect(
            Rect::new(rail.left, rail.top, thumb_x, rail.bottom),
            2.0,
            p.accent,
        );
    }
    let radius = if active { 5.0 } else { 4.0 };
    let thumb = Rect::from_size(
        thumb_x - radius,
        center - radius,
        radius * 2.0,
        radius * 2.0,
    );
    list.rounded_rect(
        thumb,
        radius,
        if active { p.accent_hover } else { p.accent },
    );
}

fn paint_preview(list: &mut DrawList, card: &super::DesktopCard, body: Rect, p: &Palette) {
    let r = Rect::from_size(body.right - 260.0, body.top + 78.0, 260.0, 172.0);
    let a = &card.appearance;
    list.push_clip(r);
    list.rounded_rect(r, 6.0, p.surface_muted);
    let bg = a.background_color.unwrap_or(p.surface);
    let dark = ((bg >> 16) & 255) * 299 + ((bg >> 8) & 255) * 587 + (bg & 255) * 114 < 145000;
    let fg = a
        .font_color
        .unwrap_or(if dark { 0xf0f2f4 } else { 0x252b32 });
    let fg = theme::mix(fg, p.surface_muted, a.opacity as f32 / 100.0);
    let bg = theme::mix(bg, p.surface_muted, a.opacity as f32 / 100.0);
    list.rounded_rect(r, 6.0, bg);
    if a.background_color == Some(GLASS_BACKGROUND) {
        list.glass_sheen(r);
    }
    if a.show_border {
        list.rounded_border(r, 6.0, theme::mix(fg, bg, 0.2));
    }
    list.text(
        Rect::from_size(r.left + 16.0, r.top + 8.0, 150.0, 26.0),
        "收集箱",
        TextStyle::Body16,
        fg,
    );
    list.text_aligned(
        Rect::new(r.right - 80.0, r.top + 8.0, r.right - 12.0, r.top + 34.0),
        "外观预览",
        TextStyle::Caption,
        theme::mix(fg, bg, 0.65),
        Align::Trailing,
    );
    let row_height = a.font_size as f32
        + 8.0
        + 2.0 * a.row_padding as f32
        + if a.show_details { 22.0 } else { 0.0 };
    for (i, title) in ["整理今天的想法", "留一点时间阅读"].iter().enumerate() {
        let top = r.top + 40.0 + i as f32 * row_height;
        let check = Rect::from_size(r.right - 30.0, top + (row_height - 14.0) / 2.0, 14.0, 14.0);
        list.rounded_border(check, 3.0, theme::mix(fg, bg, 0.5));
        let start = list.cmds().len();
        list.text(
            Rect::new(
                r.left + 16.0,
                top,
                r.right - 44.0,
                top + row_height - if a.show_details { 22.0 } else { 0.0 },
            ),
            crate::ui::text::ellipsize(
                title,
                TextStyle::Body16,
                (r.width() - 52.0) * 16.0 / a.font_size as f32,
            ),
            TextStyle::Body16,
            fg,
        );
        list.scale_text_since(start, a.font_size as f32 / 16.0);
        if a.show_details {
            list.text(
                Rect::new(
                    r.left + 16.0,
                    top + row_height - 22.0,
                    r.right - 44.0,
                    top + row_height,
                ),
                "日常 · 待处理",
                TextStyle::Caption,
                theme::mix(fg, bg, 0.65),
            );
        }
        if a.show_separators {
            list.hline(
                r.left + 40.0,
                r.right - 16.0,
                top + row_height - 2.0,
                theme::mix(fg, bg, 0.1),
            );
        }
    }
    list.pop_clip();
}
