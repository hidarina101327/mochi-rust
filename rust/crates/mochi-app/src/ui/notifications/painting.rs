//! 绘制通知图标、时间信息和通知条目。
use super::{Category, Filter, Hit, Layout, Mode, Notice, State};
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::{Edges, Rect},
    theme::{self, Palette},
};

pub fn blue(p: &Palette) -> u32 {
    if theme::is_dark(p) {
        0x60a5fa
    } else {
        0x2563eb
    }
}

fn icon(category: Category) -> Icon {
    match category {
        Category::Document => Icon::FILE_TEXT,
        Category::Assistant => Icon::BOT,
        Category::Schedule => Icon::CALENDAR,
        Category::System => Icon::INFO,
        Category::Automation => Icon::GIT_BRANCH,
    }
}

fn timestamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn dot(list: &mut DrawList, x: f32, y: f32, p: &Palette) {
    list.rounded_rect(Rect::from_size(x, y, 6.0, 6.0), 3.0, blue(p));
}

fn paint_row(list: &mut DrawList, rect: Rect, n: &Notice, state: &State, p: &Palette) {
    let hit = Hit::Notice(n.id);
    let active = state.hover == Some(hit) || state.focused == Some(hit);
    if active {
        list.rounded_rect(rect.inset(Edges::xy(0.0, 2.0)), 5.0, p.surface_muted);
    }
    if state.focused == Some(hit) {
        list.rounded_border(rect.inset(Edges::xy(1.0, 3.0)), 5.0, blue(p));
    }
    list.icon_centered(
        Rect::from_size(rect.left + 10.0, rect.top + 12.0, 24.0, 24.0),
        icon(n.category),
        17.0,
        p.muted,
    );
    let x = rect.left + 44.0;
    list.text(
        Rect::new(x, rect.top + 7.0, rect.right - 24.0, rect.top + 31.0),
        &n.title,
        TextStyle::Title,
        p.foreground,
    );
    list.text(
        Rect::new(x, rect.top + 31.0, rect.right - 24.0, rect.top + 53.0),
        n.message.replace(['\r', '\n'], " "),
        TextStyle::Caption,
        p.muted,
    );
    let repeated = if n.occurrences > 1 {
        format!(" · {} 次", n.occurrences)
    } else {
        String::new()
    };
    list.text(
        Rect::new(x, rect.top + 54.0, rect.right - 24.0, rect.bottom - 5.0),
        format!(
            "{} · {}{repeated}",
            n.category.label(),
            timestamp(n.created_at)
        ),
        TextStyle::Tiny,
        p.muted,
    );
    if !n.read {
        dot(list, rect.right - 17.0, rect.top + 16.0, p);
    }
    list.hline(
        x,
        rect.right - 10.0,
        rect.bottom - 1.0,
        theme::mix(p.border, p.surface, 0.55),
    );
}

pub fn paint(list: &mut DrawList, state: &State, layout: &Layout, viewport: Rect, p: &Palette) {
    if !state.is_open() {
        return;
    }
    if state.mode == Mode::Center {
        list.rect_alpha(viewport, 0x000000, 0.30);
    }
    let r = layout.frame;
    list.rounded_rect_alpha(
        Rect::new(r.left - 2.0, r.top + 3.0, r.right + 2.0, r.bottom + 5.0),
        9.0,
        0x000000,
        0.10,
    );
    list.rounded_rect(r, 8.0, p.surface);
    list.rounded_border(r, 8.0, p.border);
    list.push_clip(r);
    let title = if state.mode == Mode::Center {
        "通知中心"
    } else {
        "通知"
    };
    let title_height = TextStyle::Large.line_height().max(28.0);
    let title_extra = title_height - 28.0;
    list.text(
        Rect::new(
            r.left + 20.0,
            r.top + 10.0,
            r.right - 140.0,
            r.top + 10.0 + title_height,
        ),
        title,
        TextStyle::Large,
        p.foreground,
    );
    let unread = state.history.unread();
    let subtitle = if state.windows_failed {
        "Windows 提醒暂不可用，消息已保留在此处".into()
    } else if unread > 0 {
        format!("{unread} 条未读")
    } else {
        "暂无未读消息".into()
    };
    list.text(
        Rect::new(
            r.left + 20.0,
            r.top + 37.0 + title_extra,
            r.right - 20.0,
            r.top + 56.0 + title_extra,
        ),
        subtitle,
        TextStyle::Tiny,
        p.muted,
    );

    for (rect, hit) in &layout.controls {
        let selected = matches!(hit, Hit::Filter(f) if *f == state.filter);
        if state.hover == Some(*hit) || selected {
            list.rounded_rect(*rect, 4.0, p.surface_muted);
        }
        if state.focused == Some(*hit) {
            list.rounded_border(*rect, 4.0, blue(p));
        }
        let color = if selected { p.foreground } else { p.muted };
        match hit {
            Hit::Close => list.icon_centered(*rect, Icon::X, 17.0, color),
            Hit::MarkAllRead => {
                list.text_aligned(*rect, "全部已读", TextStyle::Caption, color, Align::Center)
            }
            Hit::More => list.text_aligned(
                *rect,
                "更多通知 →",
                TextStyle::Label,
                blue(p),
                Align::Center,
            ),
            Hit::Back => list.text_aligned(
                *rect,
                "← 返回列表",
                TextStyle::Caption,
                color,
                Align::Center,
            ),
            Hit::ToggleCategory(category) => list.text_aligned(
                *rect,
                if state.muted.contains(category) {
                    "恢复此类消息提醒"
                } else {
                    "不再提示此类消息"
                },
                TextStyle::Caption,
                blue(p),
                Align::Center,
            ),
            Hit::Settings => list.text_aligned(
                *rect,
                "通知设置",
                TextStyle::Caption,
                blue(p),
                Align::Center,
            ),
            Hit::Filter(f) => {
                list.text_aligned(*rect, f.label(), TextStyle::Caption, color, Align::Center);
                if *f != Filter::All
                    && state
                        .history
                        .entries
                        .iter()
                        .any(|n| !n.read && f.matches(n))
                {
                    dot(list, rect.right - 12.0, rect.top + 5.0, p);
                }
            }
            _ => {}
        }
    }
    list.push_clip(layout.body);
    if let Some(n) = state.selected_notice() {
        let x = layout.body.left + 12.0;
        let y = layout.body.top - layout.scroll;
        let title_height = TextStyle::Large.line_height().max(28.0);
        let title_extra = title_height - 28.0;
        list.text(
            Rect::new(x, y, layout.body.right - 12.0, y + title_height),
            &n.title,
            TextStyle::Large,
            p.foreground,
        );
        list.text(
            Rect::new(
                x,
                y + 32.0 + title_extra,
                layout.body.right - 12.0,
                y + 54.0 + title_extra,
            ),
            format!(
                "{} · {} · {}",
                n.category.label(),
                timestamp(n.created_at),
                if state.muted.contains(&n.category) {
                    "此类消息已关闭提醒"
                } else {
                    "已读"
                },
            ),
            TextStyle::Caption,
            p.muted,
        );
        for (i, line) in layout.detail_lines.iter().enumerate() {
            let top = y + 72.0 + title_extra + i as f32 * 24.0;
            if top + 24.0 < layout.body.top || top > layout.body.bottom {
                continue;
            }
            list.text(
                Rect::new(x, top, layout.body.right - 12.0, top + 24.0),
                line,
                TextStyle::Label,
                p.foreground,
            );
        }
    } else if layout.rows.is_empty() {
        let y = layout.body.top + (layout.body.height() - 86.0).max(0.0) * 0.4;
        list.icon_centered(
            Rect::new(layout.body.left, y, layout.body.right, y + 34.0),
            Icon::BELL,
            24.0,
            p.muted,
        );
        list.text_aligned(
            Rect::new(layout.body.left, y + 40.0, layout.body.right, y + 66.0),
            if state.filter == Filter::Unread {
                "未读消息已处理完"
            } else {
                "这里暂时没有通知"
            },
            TextStyle::Label,
            p.foreground,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(layout.body.left, y + 68.0, layout.body.right, y + 88.0),
            "新消息会在铃铛处显示蓝点",
            TextStyle::Tiny,
            p.muted,
            Align::Center,
        );
    } else {
        for (rect, id) in &layout.rows {
            if rect.bottom <= layout.body.top || rect.top >= layout.body.bottom {
                continue;
            }
            if let Some(n) = state.history.entries.iter().find(|n| n.id == *id) {
                paint_row(list, *rect, n, state, p);
            }
        }
    }
    list.pop_clip();
    state.scrollbar.paint(list, &layout.bars(), p);
    if state.mode == Mode::Center {
        list.hline(r.left + 20.0, r.right - 20.0, r.bottom - 41.0, p.border);
        let footer = Rect::new(
            r.left + 20.0,
            r.bottom - 36.0,
            r.right - 112.0,
            r.bottom - 8.0,
        );
        let hint = if state.save_failed {
            "通知保存失败，正在重试"
        } else {
            "保留最近 200 条通知 · 点击消息查看详情"
        };
        list.text(
            footer,
            crate::ui::text::ellipsize(hint, TextStyle::Tiny, footer.width()),
            TextStyle::Tiny,
            p.muted,
        );
    }
    list.pop_clip();
}
