//! 绘制首页仪表板及可交互内容。
use super::*;

/// 画首页。`scroll` 是已滚过的像素。
pub fn paint(list: &mut DrawList, area: Rect, page: &Layout, scroll: f32, p: &Palette) {
    paint_interactive(list, area, page, scroll, p, None, None);
}

pub fn paint_interactive(
    list: &mut DrawList,
    area: Rect,
    page: &Layout,
    scroll: f32,
    p: &Palette,
    hover: Option<usize>,
    focus: Option<usize>,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);
    let dy = -scroll;

    for (index, block) in page.blocks.iter().enumerate() {
        let r = shift(block_rect(block), dy);
        // 视口外的块整块跳过——首页有几十个块和几百个热力格
        if r.bottom < area.top || r.top > area.bottom {
            continue;
        }
        let interactive = block_action(block).is_some();
        let highlighted = hover == Some(index) || focus == Some(index);
        if interactive {
            list.push_clip(r);
            if highlighted {
                list.rounded_rect(r, 6.0, theme::mix(p.foreground, p.area_main_default, 0.055));
            }
        }
        match block {
            Block::Panel { quiet, .. } => {
                if *quiet {
                    list.rounded_rect(
                        r,
                        10.0,
                        theme::mix(p.surface_muted, p.area_main_default, 0.5),
                    );
                } else {
                    list.rect(
                        Rect::new(
                            r.left + panel_padding(),
                            r.bottom - 1.0,
                            r.right - panel_padding(),
                            r.bottom,
                        ),
                        theme::mix(p.border, p.area_main_default, 0.5),
                    );
                }
            }
            Block::SectionTitle { text, .. } => {
                list.text(r, text.clone(), TextStyle::Title, p.foreground);
            }
            Block::Headline { text, .. } => {
                list.text(r, text.clone(), TextStyle::Heading2, p.foreground);
            }
            Block::Greeting { text, .. } => {
                list.text(
                    r,
                    text::ellipsize(text, TextStyle::Display, r.width()),
                    TextStyle::Display,
                    p.foreground,
                );
            }
            Block::Caption { text, .. } => {
                list.text(
                    r,
                    text::ellipsize(text, TextStyle::Caption, r.width()),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            Block::ActionLink { text, .. } => {
                list.text_aligned(
                    r.inset(Edges::xy(6.0, 0.0)),
                    text.clone(),
                    TextStyle::Caption,
                    if highlighted { p.foreground } else { p.muted },
                    Align::Trailing,
                );
            }
            Block::PromptRow { text, .. } => {
                list.text(
                    Rect::new(r.left + 12.0, r.top + 7.0, r.right - 34.0, r.bottom - 7.0),
                    text::ellipsize(text, TextStyle::Label, (r.width() - 46.0).max(1.0)),
                    TextStyle::Label,
                    p.foreground,
                );
                list.icon_centered(
                    Rect::new(r.right - 28.0, r.top + 7.0, r.right - 8.0, r.bottom - 7.0),
                    Icon::CHEVRON_RIGHT,
                    14.0,
                    p.muted,
                );
            }
            Block::DetailRow { label, value, .. } => {
                list.text(
                    Rect::new(r.left, r.top, r.left + r.width() * 0.52, r.bottom),
                    label.clone(),
                    TextStyle::Caption,
                    p.muted,
                );
                list.text_aligned(
                    Rect::new(r.left + r.width() * 0.45, r.top, r.right, r.bottom),
                    text::ellipsize(value, TextStyle::Label, r.width() * 0.55),
                    TextStyle::Label,
                    p.foreground,
                    Align::Trailing,
                );
            }
            Block::ActionTile {
                icon,
                title,
                action,
                ..
            } => {
                let primary = *action == Action::NewNote;
                let foreground = if primary { p.background } else { p.foreground };
                if primary {
                    list.rounded_rect(
                        r,
                        7.0,
                        if highlighted {
                            theme::mix(p.foreground, p.background, 0.85)
                        } else {
                            p.foreground
                        },
                    );
                } else {
                    list.rounded_border(
                        r.inset(Edges::xy(0.5, 0.5)),
                        7.0,
                        theme::mix(
                            p.border,
                            p.area_main_default,
                            if highlighted { 1.0 } else { 0.65 },
                        ),
                    );
                }
                let icon_box =
                    Rect::new(r.left + 14.0, r.top + 14.0, r.left + 34.0, r.bottom - 14.0);
                list.icon_centered(icon_box, *icon, 17.0, foreground);
                let title_rect =
                    Rect::new(r.left + 42.0, r.top + 14.0, r.right - 10.0, r.bottom - 14.0);
                list.text(
                    title_rect,
                    text::ellipsize(title, TextStyle::Label, title_rect.width()),
                    TextStyle::Label,
                    foreground,
                );
            }
            Block::DocumentRow {
                title,
                subtitle,
                mtime_ms,
                favorite,
                ..
            } => {
                list.rect(
                    Rect::new(r.left + 48.0, r.bottom - 1.0, r.right, r.bottom),
                    theme::mix(p.border, p.area_main_default, 0.3),
                );
                let icon_box = Rect::new(r.left + 8.0, r.top + 12.0, r.left + 36.0, r.top + 40.0);
                list.icon_centered(icon_box, Icon::FILE_TEXT, 18.0, p.muted);
                let right_reserved = if *favorite { 32.0 } else { 8.0 };
                let text_right = (r.right - right_reserved).max(r.left + 44.0);
                let title_rect = Rect::new(r.left + 48.0, r.top + 7.0, text_right, r.top + 28.0);
                list.text(
                    title_rect,
                    text::ellipsize(title, TextStyle::Label, title_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
                let timestamp = relative_time(*mtime_ms, now_ms());
                let time_width = (text::measure(&timestamp, TextStyle::Caption) + 8.0)
                    .min((r.width() - 56.0).max(0.0) * 0.45);
                let time_left = text_right - time_width;
                let subtitle_rect = Rect::new(
                    r.left + 48.0,
                    r.top + 29.0,
                    (time_left - 12.0).max(r.left + 48.0),
                    r.bottom - 6.0,
                );
                list.text(
                    subtitle_rect,
                    text::ellipsize(subtitle, TextStyle::Caption, subtitle_rect.width()),
                    TextStyle::Caption,
                    p.muted,
                );
                if *favorite {
                    list.icon_centered(
                        Rect::new(r.right - 28.0, r.top + 18.0, r.right - 8.0, r.top + 38.0),
                        Icon::STAR,
                        14.0,
                        p.accent,
                    );
                }
                list.text_aligned(
                    Rect::new(
                        time_left,
                        r.top + 29.0,
                        r.right - right_reserved,
                        r.bottom - 6.0,
                    ),
                    text::ellipsize(&timestamp, TextStyle::Caption, time_width),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
            }
            Block::InboxRow {
                content,
                created_at_ms,
                ..
            } => {
                let icon_box = Rect::new(r.left + 8.0, r.top + 10.0, r.left + 32.0, r.top + 34.0);
                list.icon_centered(icon_box, Icon::INBOX, 16.0, p.muted);
                let time_width = 74.0;
                let content_rect = Rect::new(
                    r.left + 42.0,
                    r.top + 8.0,
                    (r.right - time_width).max(r.left + 43.0),
                    r.bottom - 8.0,
                );
                list.text(
                    content_rect,
                    text::ellipsize(content, TextStyle::Label, content_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
                list.text_aligned(
                    Rect::new(
                        (r.right - time_width).max(content_rect.right),
                        r.top + 8.0,
                        r.right - 8.0,
                        r.bottom - 8.0,
                    ),
                    relative_time(*created_at_ms, now_ms()),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
            }
            Block::LibraryRow {
                name, type_name, ..
            } => {
                let icon_box = Rect::new(r.left + 8.0, r.top + 8.0, r.left + 32.0, r.top + 32.0);
                list.icon_centered(icon_box, Icon::FOLDER, 17.0, p.muted);
                let type_width =
                    (text::measure(type_name, TextStyle::Caption) + 12.0).min(r.width() * 0.38);
                let type_rect = Rect::new(
                    (r.right - type_width).max(r.left + 40.0),
                    r.top + 8.0,
                    r.right - 8.0,
                    r.bottom - 8.0,
                );
                list.text_aligned(
                    type_rect,
                    text::ellipsize(type_name, TextStyle::Caption, type_rect.width()),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
                let name_rect = Rect::new(
                    r.left + 42.0,
                    r.top + 8.0,
                    (type_rect.left - 8.0).max(r.left + 43.0),
                    r.bottom - 8.0,
                );
                list.text(
                    name_rect,
                    text::ellipsize(name, TextStyle::Label, name_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
            }
            Block::SessionRow {
                title,
                meta,
                pinned,
                ..
            } => {
                let icon_box = Rect::new(r.left + 8.0, r.top + 8.0, r.left + 32.0, r.top + 32.0);
                list.icon_centered(icon_box, Icon::MESSAGE_SQUARE, 16.0, p.muted);
                let right_reserved = if *pinned { 30.0 } else { 8.0 };
                let text_right = (r.right - right_reserved).max(r.left + 44.0);
                let title_rect = Rect::new(r.left + 42.0, r.top + 4.0, text_right, r.top + 23.0);
                list.text(
                    title_rect,
                    text::ellipsize(title, TextStyle::Label, title_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
                let meta_rect = Rect::new(r.left + 42.0, r.top + 23.0, text_right, r.bottom - 4.0);
                list.text(
                    meta_rect,
                    text::ellipsize(meta, TextStyle::Caption, meta_rect.width()),
                    TextStyle::Caption,
                    p.muted,
                );
                if *pinned {
                    list.icon_centered(
                        Rect::new(r.right - 28.0, r.top + 12.0, r.right - 8.0, r.top + 32.0),
                        Icon::PIN,
                        14.0,
                        p.accent,
                    );
                }
            }
            Block::ScheduleRow {
                time,
                title,
                status,
                kind,
                priority,
                ..
            } => {
                let done = matches!(status.as_str(), "done" | "completed");
                let high_priority = matches!(priority.as_str(), "high" | "urgent") && !done;
                let marker = Rect::new(r.left + 4.0, r.top + 11.0, r.left + 7.0, r.bottom - 11.0);
                list.rounded_rect(
                    marker,
                    1.5,
                    if high_priority {
                        p.danger
                    } else {
                        theme::mix(p.muted, p.area_main_default, 0.4)
                    },
                );
                let display_status = schedule_status_label(status);
                let status_width =
                    (text::measure(display_status, TextStyle::Caption) + 8.0).min(r.width() * 0.3);
                let status_rect = Rect::new(
                    r.right - status_width - 8.0,
                    r.top + 34.0,
                    r.right - 8.0,
                    r.bottom - 8.0,
                );
                list.text_aligned(
                    status_rect,
                    text::ellipsize(display_status, TextStyle::Caption, status_rect.width()),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
                if high_priority {
                    let rect = Rect::new(r.right - 26.0, r.top + 8.0, r.right - 8.0, r.top + 28.0);
                    list.text_aligned(
                        rect,
                        "高".to_owned(),
                        TextStyle::Caption,
                        p.danger,
                        Align::Center,
                    );
                }
                let title_rect = Rect::new(
                    r.left + 18.0,
                    r.top + 9.0,
                    r.right - if high_priority { 32.0 } else { 8.0 },
                    r.top + 29.0,
                );
                let title_color = if done { p.muted } else { p.foreground };
                list.text(
                    title_rect,
                    text::ellipsize(title, TextStyle::Label, title_rect.width()),
                    TextStyle::Label,
                    title_color,
                );
                let meta_rect = Rect::new(
                    r.left + 18.0,
                    r.top + 34.0,
                    status_rect.left - 10.0,
                    r.bottom - 8.0,
                );
                let meta = if kind.is_empty() {
                    time.clone()
                } else {
                    format!("{time} · {}", schedule_kind_label(kind))
                };
                list.text(
                    meta_rect,
                    text::ellipsize(&meta, TextStyle::Caption, meta_rect.width()),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            Block::JournalPreview {
                date,
                excerpt,
                words,
                exists,
                ..
            } => {
                let icon_box = Rect::new(r.left + 12.0, r.top + 12.0, r.left + 42.0, r.top + 42.0);
                list.icon_centered(icon_box, Icon::CALENDAR_DAYS, 17.0, p.muted);
                let date_rect =
                    Rect::new(r.left + 52.0, r.top + 12.0, r.right - 12.0, r.top + 32.0);
                list.text(
                    date_rect,
                    text::ellipsize(date, TextStyle::Label, date_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
                let excerpt_rect =
                    Rect::new(r.left + 12.0, r.top + 54.0, r.right - 12.0, r.top + 79.0);
                list.text(
                    excerpt_rect,
                    text::ellipsize(excerpt, TextStyle::Label, excerpt_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
                let state = if *exists {
                    format!("{} 字", compact(*words as i64))
                } else {
                    "尚未创建".to_owned()
                };
                list.text(
                    Rect::new(
                        r.left + 12.0,
                        r.bottom - 28.0,
                        r.right - 12.0,
                        r.bottom - 8.0,
                    ),
                    state,
                    TextStyle::Caption,
                    p.muted,
                );
            }
            Block::GraphNodeRow {
                title, link_count, ..
            } => {
                let icon_box = Rect::new(r.left + 8.0, r.top + 8.0, r.left + 32.0, r.top + 32.0);
                list.icon_centered(icon_box, Icon::NETWORK, 16.0, p.muted);
                let links = format!("{link_count} 条链接");
                let links_width = text::measure(&links, TextStyle::Caption) + 8.0;
                let links_rect = Rect::new(
                    (r.right - links_width).max(r.left + 42.0),
                    r.top + 8.0,
                    r.right - 8.0,
                    r.bottom - 8.0,
                );
                list.text_aligned(
                    links_rect,
                    links,
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
                let title_rect = Rect::new(
                    r.left + 42.0,
                    r.top + 8.0,
                    (links_rect.left - 8.0).max(r.left + 43.0),
                    r.bottom - 8.0,
                );
                list.text(
                    title_rect,
                    text::ellipsize(title, TextStyle::Label, title_rect.width()),
                    TextStyle::Label,
                    p.foreground,
                );
            }
            Block::Empty { text, .. } => {
                list.text(
                    r,
                    text::ellipsize(text, TextStyle::Caption, r.width()),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            Block::Badge { text, accent, .. } => {
                let bg = if *accent {
                    theme::mix(p.accent, p.surface, 0.12)
                } else {
                    p.surface_muted
                };
                list.rect(r, bg);
                list.text_aligned(
                    r,
                    text.clone(),
                    TextStyle::Caption,
                    if *accent { p.accent } else { p.muted },
                    Align::Center,
                );
            }
            Block::Tile {
                label,
                value,
                unit,
                delta,
                ..
            } => {
                list.rect(
                    Rect::new(r.left, r.top + 8.0, r.left + 1.0, r.bottom - 8.0),
                    theme::mix(p.border, p.area_main_default, 0.65),
                );
                let inner = r.inset(Edges::xy(10.0, 8.0));
                let cap = TextStyle::Caption.line_height();
                list.text(
                    Rect::new(inner.left, inner.top, inner.right, inner.top + cap),
                    label.clone(),
                    TextStyle::Caption,
                    p.muted,
                );
                let vh = TextStyle::Large.line_height();
                let vy = inner.top + cap + 2.0;
                let unit_width = if unit.is_empty() {
                    0.0
                } else {
                    text::measure(unit, TextStyle::Caption) + 4.0
                };
                list.text(
                    Rect::new(inner.left, vy, inner.right - unit_width, vy + vh),
                    text::ellipsize(
                        value,
                        TextStyle::Large,
                        (inner.width() - unit_width).max(1.0),
                    ),
                    TextStyle::Large,
                    p.foreground,
                );
                if !unit.is_empty() {
                    list.text(
                        Rect::new(inner.right - unit_width, vy, inner.right, vy + vh),
                        unit.clone(),
                        TextStyle::Caption,
                        p.muted,
                    );
                }
                if let Some(d) = delta {
                    if *d != 0 {
                        // 增减用颜色区分：涨用强调色，跌用 danger。
                        // 只写数字不带方向的话，「-120」在中文语境里容易被读成范围
                        let dy2 = vy + vh;
                        list.text(
                            Rect::new(inner.left, dy2, inner.right, dy2 + cap),
                            format!("较昨日 {}", signed(*d)),
                            TextStyle::Caption,
                            p.muted,
                        );
                    }
                }
            }
            Block::Bars { bars, .. } => {
                let n = bars.len().max(1);
                let slot = r.width() / n as f32;
                let bar_w = (slot - 2.0).max(1.0);
                for (i, (_, ratio, highlight)) in bars.iter().enumerate() {
                    let x = r.left + i as f32 * slot;
                    // 有数据但比例极小的柱子也要看得见，给 2px 底
                    let h = if *ratio > 0.0 {
                        (r.height() * ratio).max(2.0)
                    } else {
                        0.0
                    };
                    if h <= 0.0 {
                        continue;
                    }
                    let color = if *highlight {
                        p.accent
                    } else {
                        theme::mix(p.accent, p.surface, 0.45)
                    };
                    list.rect(Rect::new(x, r.bottom - h, x + bar_w, r.bottom), color);
                }
                // 首尾两个标签，中间不标——柱子太密，标满会糊成一片
                if let (Some(first), Some(last)) = (bars.first(), bars.last()) {
                    let ly = r.bottom + 2.0;
                    let lh = TextStyle::Caption.line_height();
                    list.text(
                        Rect::new(r.left, ly, r.left + 80.0, ly + lh),
                        first.0.clone(),
                        TextStyle::Caption,
                        p.muted,
                    );
                    list.text_aligned(
                        Rect::new(r.right - 80.0, ly, r.right, ly + lh),
                        last.0.clone(),
                        TextStyle::Caption,
                        p.muted,
                        Align::Trailing,
                    );
                }
            }
            Block::StorageBar {
                segments, total, ..
            } => {
                list.rounded_rect(r, 7.0, p.surface_muted);
                if *total > 0 {
                    let mut left = r.left;
                    let palette = [
                        p.accent,
                        theme::mix(p.accent, p.surface, 0.25),
                        theme::mix(p.accent, p.surface, 0.5),
                        theme::mix(p.accent, p.surface, 0.7),
                        p.danger,
                        p.muted,
                    ];
                    for (index, (_, size)) in segments.iter().enumerate() {
                        if *size == 0 {
                            continue;
                        }
                        let width = (r.width() * (*size as f32 / *total as f32)).max(1.0);
                        let right = (left + width).min(r.right);
                        if right <= left {
                            break;
                        }
                        list.rect(
                            Rect::new(left, r.top, right, r.bottom),
                            palette[index % palette.len()],
                        );
                        left = right;
                        if left >= r.right {
                            break;
                        }
                    }
                }
            }
            Block::Heatmap { cells, .. } => {
                let cols = cells.iter().map(|(col, _, _)| *col + 1).max().unwrap_or(1);
                let cell = ((r.width() - (cols.saturating_sub(1) as f32 * heat_gap()))
                    / cols as f32)
                    .clamp(2.0, heat_cell());
                for (col, row, intensity) in cells {
                    let x = r.left + *col as f32 * (cell + heat_gap());
                    let y = r.top + *row as f32 * (heat_cell() + heat_gap());
                    if x > area.right {
                        break;
                    }
                    // 零活跃也要画一个底格，否则热力图会变成一堆孤立的点，
                    // 看不出「哪些天没写」这个同样重要的信息
                    let color = if *intensity <= 0.0 {
                        p.surface_muted
                    } else {
                        theme::mix(p.accent, p.surface_muted, 0.25 + intensity * 0.75)
                    };
                    list.rect(Rect::new(x, y, x + cell, y + cell), color);
                }
            }
            Block::HourlyHeatmap { cells, .. } => {
                let label_width = 24.0;
                let grid_left = r.left + label_width;
                let grid_width = (r.width() - label_width).max(1.0);
                let slot_width = grid_width / 24.0;
                let grid_height = (r.height() - 18.0).max(1.0);
                let row_height = grid_height / 7.0;
                let cell_width = (slot_width - 2.0).max(2.0);
                let cell_height = (row_height - 2.0).max(2.0);
                for day in 0..7usize {
                    let y = r.top + day as f32 * row_height;
                    list.text(
                        Rect::new(r.left, y, grid_left - 4.0, y + cell_height),
                        weekday_label(day).to_owned(),
                        TextStyle::Caption,
                        p.muted,
                    );
                    for hour in 0..24usize {
                        let intensity = cells
                            .iter()
                            .find(|(cell_hour, cell_day, _)| *cell_hour == hour && *cell_day == day)
                            .map(|(_, _, value)| *value)
                            .unwrap_or(0.0);
                        let x = grid_left + hour as f32 * slot_width;
                        let color = if intensity <= 0.0 {
                            p.surface_muted
                        } else {
                            theme::mix(p.accent, p.surface_muted, 0.22 + intensity * 0.78)
                        };
                        list.rounded_rect(
                            Rect::new(
                                x,
                                y,
                                (x + cell_width).min(r.right),
                                (y + cell_height).min(r.bottom - 18.0),
                            ),
                            2.0,
                            color,
                        );
                    }
                }
                let label_y = r.bottom - 16.0;
                for (hour, label) in [(0usize, "0"), (6, "6"), (12, "12"), (18, "18"), (23, "23")] {
                    let x = grid_left + hour as f32 * slot_width;
                    list.text(
                        Rect::new(x, label_y, (x + 24.0).min(r.right), r.bottom),
                        label.to_owned(),
                        TextStyle::Caption,
                        p.muted,
                    );
                }
            }
        }
        if interactive {
            if focus == Some(index) {
                list.rounded_border(r.inset(Edges::xy(1.0, 1.0)), 6.0, p.accent);
            }
            list.pop_clip();
        }
    }

    list.pop_clip();
}
