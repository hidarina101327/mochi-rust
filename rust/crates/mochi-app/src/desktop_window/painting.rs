//! 绘制桌面卡片窗口的导航列表和页面内容。
use super::{Hit, Spec, State, View};
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text, theme,
};
use chrono::{Datelike, NaiveDate};
use mochi_core::desktop_cards::{Module, GLASS_BACKGROUND};

fn row_height(spec: &Spec, detail: bool) -> f32 {
    spec.appearance.font_size as f32
        + 8.0
        + 2.0 * spec.appearance.row_padding as f32
        + if detail { 22.0 } else { 0.0 }
}
pub(super) fn navigation_extent(spec: &Spec, view: &View, a: Rect) -> f32 {
    if view.tabs.len() <= 1 && !spec.appearance.show_single_page_name {
        return 0.0;
    }
    let axis = if spec.appearance.tabs_left {
        a.width()
    } else {
        a.height()
    };
    if spec.appearance.tabs_ratio == 0 {
        if spec.appearance.tabs_left {
            104.0
        } else {
            54.0
        }
    } else {
        (axis * spec.appearance.tabs_ratio as f32 / 100.0).clamp(48.0, (axis - 100.0).max(48.0))
    }
}
pub(super) fn content_rect(spec: &Spec, view: &View, a: Rect) -> Rect {
    let extent = navigation_extent(spec, view, a);
    let left = if spec.appearance.tabs_left && extent > 0.0 {
        extent + 8.0
    } else {
        16.0
    };
    let top = if !spec.appearance.tabs_left && extent > 0.0 {
        extent + 8.0
    } else {
        36.0
    };
    Rect::new(a.left + left, a.top + top, a.right - 16.0, a.bottom - 16.0)
}
pub(super) fn scroll_body(spec: &Spec, view: &View, a: Rect) -> Rect {
    let mut body = content_rect(spec, view, a);
    if view.module == Some(Module::Schedule) && view.presentation.schedule_view == 1 {
        body.top += 66.0;
    }
    if matches!(
        view.module,
        Some(Module::Folder | Module::Weather | Module::Music | Module::Search)
    ) {
        body.top += 32.0;
    }
    body
}
#[derive(Clone)]
struct Entry {
    rect: Rect,
    row: usize,
    grid: bool,
}
struct Content {
    entries: Vec<Entry>,
    headings: Vec<(Rect, String)>,
    cells: Vec<(Rect, String, usize)>,
    height: f32,
}
fn visible_rows(view: &View) -> Vec<usize> {
    view.rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            !view.collapsed.iter().any(|path| {
                row.meta.path != *path && row.meta.path.starts_with(&format!("{path}/"))
            })
        })
        .map(|(i, _)| i)
        .collect()
}
fn month(view: &View) -> NaiveDate {
    let today = chrono::Local::now().date_naive();
    let months = today.year() * 12 + today.month0() as i32 + view.month_offset;
    NaiveDate::from_ymd_opt(months.div_euclid(12), months.rem_euclid(12) as u32 + 1, 1)
        .unwrap_or(today)
}
fn content(spec: &Spec, view: &View, body: Rect) -> Content {
    let mut c = Content {
        entries: vec![],
        headings: vec![],
        cells: vec![],
        height: 0.0,
    };
    let p = &view.presentation;
    let indices = visible_rows(view);
    if view.module == Some(Module::Schedule) && p.schedule_view == 1 {
        let first = month(view);
        let start = first - chrono::Duration::days(first.weekday().num_days_from_monday() as i64);
        let mut y = 0.0;
        for week in 0..6 {
            let dates: Vec<_> = (0..7)
                .map(|day| start + chrono::Duration::days((week * 7 + day) as i64))
                .collect();
            if week > 3 && dates[0].month() != first.month() {
                break;
            }
            let days: Vec<Vec<usize>> = dates
                .iter()
                .map(|date| {
                    indices
                        .iter()
                        .copied()
                        .filter(|i| view.rows[*i].meta.date == date.to_string())
                        .collect()
                })
                .collect();
            let line = spec.appearance.font_size as f32 + 8.0;
            let max = days.iter().map(Vec::len).max().unwrap_or(0);
            let height = if p.calendar_expanded {
                32.0 + line * max.max(1) as f32
            } else {
                32.0 + line * 2.0 + 22.0
            };
            for day in 0..7 {
                let rect = Rect::from_size(
                    body.left + day as f32 * body.width() / 7.0,
                    body.top + y,
                    body.width() / 7.0,
                    height,
                );
                let shown = if p.calendar_expanded {
                    days[day].len()
                } else {
                    days[day].len().min(2)
                };
                c.cells.push((
                    rect,
                    dates[day].format("%d").to_string(),
                    days[day].len() - shown,
                ));
                for (n, &row) in days[day].iter().take(shown).enumerate() {
                    c.entries.push(Entry {
                        rect: Rect::new(
                            rect.left + 3.0,
                            rect.top + 28.0 + n as f32 * line,
                            rect.right - 3.0,
                            rect.top + 28.0 + (n + 1) as f32 * line,
                        ),
                        row,
                        grid: true,
                    });
                }
            }
            y += height;
        }
        c.height = y;
        return c;
    }
    if view.module == Some(Module::Folder) && view.rows.iter().any(|r| !r.meta.group.is_empty()) {
        let cols = if p.grid { p.columns.max(1) as usize } else { 1 };
        let height = if p.grid {
            p.grid_height.max(64) as f32
        } else {
            row_height(spec, spec.appearance.show_details || p.show_modified)
        };
        let mut at = 0;
        let mut y = body.top;
        while at < indices.len() {
            let group = &view.rows[indices[at]].meta.group;
            let end = (at + 1..indices.len())
                .find(|i| view.rows[indices[*i]].meta.group != *group)
                .unwrap_or(indices.len());
            if !group.is_empty() {
                c.headings.push((
                    Rect::from_size(body.left, y, body.width(), 30.0),
                    group.clone(),
                ));
                y += 30.0;
            }
            if group.is_empty() || !view.collapsed.contains(&format!("folder-group:{group}")) {
                for (i, row) in indices[at..end].iter().enumerate() {
                    c.entries.push(Entry {
                        rect: Rect::from_size(
                            body.left + (i % cols) as f32 * body.width() / cols as f32,
                            y + (i / cols) as f32 * height,
                            body.width() / cols as f32,
                            height,
                        ),
                        row: *row,
                        grid: p.grid,
                    });
                }
                y += (end - at).div_ceil(cols) as f32 * height;
            }
            at = end;
        }
        c.height = y - body.top;
        return c;
    }
    if p.grid {
        let cols = p.columns.max(1) as usize;
        let height = (if p.grid_height == 0 {
            body.height() / p.rows.max(1) as f32
        } else {
            p.grid_height as f32
        })
        .max(row_height(spec, false) + if p.show_icons { 26.0 } else { 0.0 });
        for (i, &row) in indices.iter().enumerate() {
            c.entries.push(Entry {
                rect: Rect::from_size(
                    body.left + (i % cols) as f32 * body.width() / cols as f32,
                    body.top + (i / cols) as f32 * height,
                    body.width() / cols as f32,
                    height,
                ),
                row,
                grid: true,
            });
        }
        c.height = indices.len().div_ceil(cols) as f32 * height;
        return c;
    }
    let mut group = String::new();
    let mut y = body.top;
    for row in indices {
        let item = &view.rows[row];
        if p.show_groups && !item.meta.group.is_empty() && group != item.meta.group {
            group = item.meta.group.clone();
            let height = p.heading_size as f32 + 18.0;
            c.headings.push((
                Rect::from_size(body.left, y, body.width(), height),
                group.clone(),
            ));
            y += height;
        }
        let details =
            (spec.appearance.show_details || item.meta.always_detail) && !item.detail.is_empty();
        let height = row_height(spec, details);
        c.entries.push(Entry {
            rect: Rect::from_size(body.left, y, body.width(), height),
            row,
            grid: false,
        });
        y += height;
    }
    c.height = y - body.top;
    c
}
fn shifted(r: Rect, offset: usize) -> Rect {
    Rect::new(
        r.left,
        r.top - offset as f32,
        r.right,
        r.bottom - offset as f32,
    )
}
pub(super) fn scroll_max(spec: &Spec, view: &View, a: Rect) -> usize {
    if super::tree::enabled(view) {
        return super::tree::scroll_max(spec, view, a);
    }
    if super::shortcuts::enabled(view) {
        return super::shortcuts::scroll_max(spec, view, a);
    }
    if super::widgets::has_chat(view) {
        return super::widgets::chat_max(spec, view, a);
    }
    if super::widgets::is_studio(view) {
        return super::widgets::studio_max(spec, view, a);
    }
    if view.module == Some(Module::Pomodoro) {
        return 0;
    }
    let body = scroll_body(spec, view, a);
    (content(spec, view, body).height - body.height())
        .ceil()
        .max(0.0) as usize
}
fn chrome(spec: &Spec, view: &View, a: Rect) -> Vec<(Rect, Hit)> {
    let mut hits = vec![(Rect::from_size(a.right - 32.0, 6.0, 24.0, 24.0), Hit::Pin)];
    hits.push((
        Rect::from_size(a.right - 60.0, 6.0, 24.0, 24.0),
        Hit::Capsule,
    ));
    if spec.appearance.capsule {
        return hits;
    }
    hits.extend(super::navigation::controls(spec, view, a));
    if view.module == Some(Module::Folder) {
        let b = content_rect(spec, view, a);
        for (i, command) in [
            super::folder::Command::Up,
            super::folder::Command::Root,
            super::folder::Command::Refresh,
        ]
        .into_iter()
        .enumerate()
        {
            hits.push((
                Rect::from_size(b.right - 90.0 + i as f32 * 30.0, b.top, 28.0, 28.0),
                Hit::FolderCommand(command),
            ));
        }
    }
    if view.module == Some(Module::Schedule) && view.presentation.schedule_view == 1 {
        let b = content_rect(spec, view, a);
        hits.push((
            Rect::from_size(b.right - 62.0, b.top, 28.0, 28.0),
            Hit::Month(-1),
        ));
        hits.push((
            Rect::from_size(b.right - 30.0, b.top, 28.0, 28.0),
            Hit::Month(1),
        ));
    }
    hits
}
pub(super) fn controls(spec: &Spec, view: &View, a: Rect, offset: usize) -> Vec<(Rect, Hit)> {
    let mut hits = chrome(spec, view, a);
    if spec.appearance.capsule {
        return hits;
    }
    if super::tree::enabled(view) {
        hits.extend(super::tree::controls(spec, view, a, offset));
        return hits;
    }
    if super::shortcuts::enabled(view) {
        hits.extend(super::shortcuts::controls(spec, view, a, offset));
        return hits;
    }
    if let Some(extra) = super::widgets::controls(spec, view, a, offset) {
        hits.extend(extra);
        return hits;
    }
    let body = scroll_body(spec, view, a);
    let offset = offset.min(scroll_max(spec, view, a));
    let layout = content(spec, view, body);
    if view.module == Some(Module::Folder) {
        for (rect, name) in &layout.headings {
            let rect = shifted(*rect, offset).intersect(&body);
            if !rect.is_empty() {
                hits.push((rect, Hit::FolderGroup(format!("folder-group:{name}"))));
            }
        }
    }
    for entry in layout.entries {
        let row = &view.rows[entry.row];
        let rect = shifted(entry.rect, offset).intersect(&body);
        if rect.is_empty() {
            continue;
        }
        if row.meta.directory
            && view.presentation.expand_libraries
            && view.module != Some(Module::Folder)
        {
            hits.push((rect, Hit::Folder(row.meta.path.clone())));
            continue;
        }
        if row.checked.is_some() && view.presentation.show_checks && !view.pending {
            let check = Rect::new(
                (rect.right - 30.0).max(rect.left),
                rect.top,
                rect.right,
                rect.bottom,
            );
            hits.push((check, Hit::Row(row.id.clone(), true)));
            hits.push((
                Rect::new(rect.left, rect.top, check.left, rect.bottom),
                Hit::Row(row.id.clone(), false),
            ));
        } else {
            hits.push((rect, Hit::Row(row.id.clone(), false)));
        }
    }
    hits
}
pub(super) fn hit(s: &State, a: Rect, x: f32, y: f32) -> Option<Hit> {
    if s.spec.appearance.capsule {
        return controls(&s.spec, &s.view, a, s.offset)
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h);
    }
    if super::widgets::has_chat(&s.view) {
        if let Some(h) = super::chat::hit(&s.spec, &s.view, a, s.offset, x, y) {
            return Some(h);
        }
    }
    controls(&s.spec, &s.view, a, s.offset)
        .into_iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|(_, h)| h)
}
/// 玻璃模式下，模糊后的壁纸上覆盖着一层浅霜。
const GLASS_TINT: u32 = 0xf4f6f8;

/// 玻璃卡片会在半透明的磨砂层后模糊壁纸，而不是降低
/// 整个窗口的透明度。
pub(super) fn is_glass(spec: &Spec) -> bool {
    spec.appearance.background_color == Some(GLASS_BACKGROUND)
}

/// 透明度从 35% 到 100% 对应从清透到乳白的磨砂效果：35% 时只有模糊，
/// 默认的 85% 会在壁纸上叠加约 25% 的白色。
pub(super) fn glass_tint_alpha(spec: &Spec) -> f32 {
    let t = (spec.appearance.opacity.clamp(35, 100) - 35) as f32 / 65.0;
    0.45 * t * t
}

pub(super) fn palette(spec: &Spec) -> theme::Palette {
    let mut p = theme::configured_palette(spec.dark);
    // 加上一点前景色，避免默认卡片在壁纸上看起来像一张空白应用页面。
    p.surface = theme::mix(p.foreground, p.surface, 0.03);
    if let Some(bg) = spec.appearance.background_color {
        p.surface = bg;
        let dark = ((bg >> 16) & 255) * 299 + ((bg >> 8) & 255) * 587 + (bg & 255) * 114 < 145000;
        p.foreground = if dark { 0xf0f2f4 } else { 0x252b32 };
        p.muted = theme::mix(p.foreground, bg, 0.58);
        p.border = theme::mix(p.foreground, bg, 0.10);
        p.surface_muted = theme::mix(p.foreground, bg, 0.05);
    }
    if let Some(fg) = spec.appearance.font_color {
        p.foreground = fg;
        p.muted = theme::mix(fg, p.surface, 0.6);
    }
    p.surface_muted = theme::mix(p.foreground, p.surface, 0.06);
    p.border = theme::mix(p.foreground, p.surface, 0.12);
    p
}
fn icon(name: &str) -> Icon {
    match name {
        "home" => Icon::HOME,
        "schedule" => Icon::CALENDAR,
        "inbox" => Icon::INBOX,
        "favorites" => Icon::STAR,
        "ai" => Icon::BOT,
        "settings" => Icon::SETTINGS,
        "notifications" => Icon::BELL,
        "recent" => Icon::CLOCK,
        "folder" => Icon::FOLDER_OPEN,
        "library" | "knowledge" => Icon::BOOK_OPEN,
        "file" => Icon::FILE_TEXT,
        "quickNote" => Icon::PENCIL_LINE,
        "templates" => Icon::FILE_PLUS,
        "automations" => Icon::GIT_BRANCH,
        "desktopCards" => Icon::LAYOUT_GRID,
        "marketplace" => Icon::BLOCKS,
        _ => Icon::BOOK_OPEN,
    }
}
fn paint_icon(list: &mut DrawList, r: Rect, name: &str, size: f32, color: u32) {
    if let Some(named) = crate::ui::icons::named(name) {
        list.icon_centered(r, named, size, color);
    } else if !name.is_ascii() {
        list.text_aligned(r, name, TextStyle::Body16, color, Align::Center);
    } else {
        list.icon_centered(r, icon(name), size, color);
    }
}
fn paint_row_icon(
    list: &mut DrawList,
    view: &View,
    row: &super::Row,
    rect: Rect,
    size: f32,
    color: u32,
) {
    if view.module == Some(Module::Folder) {
        if let Some(path) = super::shortcut_icon::path(&view.workspace, &row.meta.path) {
            list.image(
                Rect::from_size(
                    rect.left + (rect.width() - size) / 2.0,
                    rect.top + (rect.height() - size) / 2.0,
                    size,
                    size,
                ),
                path,
                &row.title,
            );
            return;
        }
    }
    paint_icon(list, rect, &row.meta.icon, size, color);
}
pub(super) fn scaled(
    list: &mut DrawList,
    r: Rect,
    value: &str,
    size: f32,
    color: u32,
    align: Align,
) {
    let scale = size / 16.0;
    let start = list.cmds().len();
    list.text_aligned(
        r,
        text::ellipsize(value, TextStyle::Body16, r.width() / scale),
        TextStyle::Body16,
        color,
        align,
    );
    list.scale_text_since(start, scale);
}
pub(super) fn paint(
    list: &mut DrawList,
    spec: &Spec,
    view: &View,
    a: Rect,
    offset: usize,
    hover: Option<&Hit>,
    focused: Option<&Hit>,
) {
    let p = palette(spec);
    // 边框沿着圆角窗口轮廓绘制，因此由 DWM 负责绘制（见 `apply_frame`）。
    if is_glass(spec) {
        // 这里的窗口是透明的：DWM 会模糊壁纸，而本函数
        // 只绘制覆盖其上的颜色，所以文字保持完全不透明。
        list.rect_alpha(a, GLASS_TINT, glass_tint_alpha(spec));
        list.glass_sheen(a);
    } else {
        list.rect(a, p.surface);
    }
    for (r, h) in chrome(spec, view, a) {
        if hover == Some(&h) {
            list.rounded_rect(r, 6.0, p.surface_muted);
        }
        match &h {
            Hit::Capsule => list.icon_centered(
                r,
                if spec.appearance.capsule {
                    Icon::CHEVRON_DOWN
                } else {
                    Icon::CHEVRON_UP
                },
                14.0,
                p.muted,
            ),
            Hit::FolderCommand(command) => list.icon_centered(
                r,
                match command {
                    super::folder::Command::Up => Icon::CHEVRON_UP,
                    super::folder::Command::Root => Icon::HOME,
                    _ => Icon::ROTATE_CCW,
                },
                14.0,
                p.muted,
            ),
            Hit::Pin => {
                list.icon_centered(
                    r,
                    Icon::PIN,
                    14.0,
                    if spec.appearance.pinned {
                        p.accent
                    } else if hover == Some(&h) {
                        p.muted
                    } else {
                        theme::mix(p.muted, p.surface, 0.5)
                    },
                );
            }
            Hit::TabScroll(delta) => list.icon_centered(
                r,
                if *delta < 0 {
                    if spec.appearance.tabs_left {
                        Icon::CHEVRON_UP
                    } else {
                        Icon::CHEVRON_LEFT
                    }
                } else if spec.appearance.tabs_left {
                    Icon::CHEVRON_DOWN
                } else {
                    Icon::CHEVRON_RIGHT
                },
                12.0,
                p.muted,
            ),
            Hit::Month(delta) => list.icon_centered(
                r,
                if *delta < 0 {
                    Icon::CHEVRON_LEFT
                } else {
                    Icon::CHEVRON_RIGHT
                },
                14.0,
                p.muted,
            ),
            Hit::Page(id) => {
                if *id == view.page_id && view.tabs.len() > 1 {
                    list.rounded_rect(r, r.height().min(r.width()) / 2.0, p.surface_muted);
                }
                let label = view
                    .tabs
                    .iter()
                    .find(|(key, _)| key == id)
                    .map(|(_, label)| label.as_str())
                    .unwrap_or("");
                scaled(
                    list,
                    r,
                    label,
                    16.0,
                    if *id == view.page_id {
                        p.foreground
                    } else {
                        p.muted
                    },
                    if view.tabs.len() == 1 {
                        Align::Leading
                    } else {
                        Align::Center
                    },
                );
            }
            _ => {}
        }
        if focused == Some(&h) {
            list.rounded_border(r, 4.0, p.accent);
        }
    }
    if spec.appearance.capsule {
        scaled(
            list,
            Rect::from_size(
                a.left + 14.0,
                a.top + 4.0,
                (a.width() - 86.0).max(1.0),
                34.0,
            ),
            &format!("{} · {}", spec.title, view.rows.len()),
            spec.appearance.font_size as f32,
            p.foreground,
            Align::Leading,
        );
        return;
    }
    if spec.appearance.tabs_divider {
        let extent = navigation_extent(spec, view, a);
        if extent > 0.0 {
            if spec.appearance.tabs_left {
                list.vline(a.left + extent, a.top + 12.0, a.bottom - 12.0, p.border);
            } else {
                list.hline(a.left + 12.0, a.right - 12.0, a.top + extent, p.border);
            }
        }
    }
    let header = content_rect(spec, view, a);
    if matches!(
        view.module,
        Some(Module::Folder | Module::Weather | Module::Music | Module::Search)
    ) {
        scaled(
            list,
            Rect::from_size(
                header.left,
                header.top,
                (header.width()
                    - if view.module == Some(Module::Folder) {
                        94.0
                    } else {
                        0.0
                    })
                .max(1.0),
                28.0,
            ),
            &view.subtitle,
            12.0,
            p.muted,
            Align::Leading,
        );
    }
    if super::tree::enabled(view) {
        super::tree::paint(list, spec, view, a, offset, hover);
        return;
    }
    if super::shortcuts::enabled(view) {
        super::shortcuts::paint(list, spec, view, a, offset, hover);
        return;
    }
    if super::widgets::paint(list, spec, view, a, offset, hover) {
        return;
    }
    let body = scroll_body(spec, view, a);
    let offset = offset.min(scroll_max(spec, view, a));
    let c = content(spec, view, body);
    let calendar = view.module == Some(Module::Schedule) && view.presentation.schedule_view == 1;
    let homegrid = view.presentation.grid;
    if calendar {
        let first = month(view);
        scaled(
            list,
            Rect::from_size(header.left, header.top, header.width() - 70.0, 30.0),
            &first.format("%Y年%m月").to_string(),
            16.0,
            p.foreground,
            Align::Leading,
        );
        for (i, day) in ["一", "二", "三", "四", "五", "六", "日"]
            .iter()
            .enumerate()
        {
            let r = Rect::from_size(
                body.left + i as f32 * body.width() / 7.0,
                header.top + 34.0,
                body.width() / 7.0,
                26.0,
            );
            list.text_aligned(r, *day, TextStyle::Caption, p.muted, Align::Center);
        }
    }
    list.push_clip(body);
    for (r, label, more) in c.cells {
        let r = shifted(r, offset);
        if r.intersect(&body).is_empty() {
            continue;
        }
        list.rounded_border(r, 0.0, p.border);
        list.text(
            Rect::from_size(r.left + 6.0, r.top + 2.0, r.width() - 12.0, 24.0),
            label,
            TextStyle::Caption,
            p.muted,
        );
        if more > 0 {
            let bubble = Rect::from_size(r.right - 42.0, r.bottom - 22.0, 36.0, 18.0);
            list.rounded_rect(bubble, 9.0, p.surface_muted);
            list.text_aligned(
                bubble,
                format!("+{more}"),
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
        }
    }
    for (r, label) in c.headings {
        let label = if view.module == Some(Module::Folder) {
            let count = view.rows.iter().filter(|r| r.meta.group == label).count();
            format!(
                "{}  {label} · {count}",
                if view.collapsed.contains(&format!("folder-group:{label}")) {
                    "▸"
                } else {
                    "▾"
                }
            )
        } else {
            label
        };
        scaled(
            list,
            shifted(r, offset),
            &label,
            view.presentation.heading_size as f32,
            p.muted,
            Align::Leading,
        );
    }
    for (ordinal, entry) in c.entries.iter().enumerate() {
        let r = shifted(entry.rect, offset);
        if r.intersect(&body).is_empty() {
            continue;
        }
        if entry.grid && view.presentation.grid_lines {
            list.rounded_border(r, 0.0, p.border);
        }
        let row = &view.rows[entry.row];
        let (p, background) = item_palette(
            &p,
            view,
            &mochi_core::desktop_cards::item_key(&row.id, &row.meta.path),
        );
        if let Some(bg) = background {
            list.rounded_rect(r, 6.0, bg);
        }
        let action = if row.meta.directory
            && view.presentation.expand_libraries
            && view.module != Some(Module::Folder)
        {
            Hit::Folder(row.meta.path.clone())
        } else {
            Hit::Row(row.id.clone(), false)
        };
        if hover == Some(&action) {
            list.rounded_rect(r, 6.0, p.surface_muted);
        }
        if view.module == Some(Module::Folder) && view.folder_selected.contains(&row.id) {
            list.rounded_rect(r, 6.0, theme::mix(p.accent, p.surface, 0.16));
            list.rounded_border(r, 6.0, theme::mix(p.accent, p.surface, 0.6));
        }
        if focused == Some(&action) {
            list.rounded_border(r, 6.0, p.accent);
        }
        let mut x = r.left + 4.0 + (row.meta.depth.min(8) as f32 * 14.0);
        let mut right = r.right - 4.0;
        if row.checked.is_some() && view.presentation.show_checks {
            right -= 30.0;
            let check = Rect::from_size(
                r.right - 24.0,
                r.top + (r.height() - 16.0) / 2.0,
                16.0,
                16.0,
            );
            if row.checked == Some(true) {
                list.rounded_rect(check, 8.0, p.accent);
                list.icon_centered(check, Icon::CHECK, 11.0, 0xffffff);
            } else {
                list.rounded_border(check, 8.0, theme::mix(p.muted, p.surface, 0.7));
            }
        }
        if !entry.grid && view.presentation.show_numbers {
            list.text(
                Rect::from_size(x, r.top, 28.0, r.height()),
                format!("{}", ordinal + 1),
                TextStyle::Caption,
                p.muted,
            );
            x += 28.0;
        }
        if row.meta.directory
            && view.presentation.expand_libraries
            && view.module != Some(Module::Folder)
        {
            list.icon_centered(
                Rect::from_size(x, r.top, 16.0, r.height()),
                if view.collapsed.contains(&row.meta.path) {
                    Icon::CHEVRON_RIGHT
                } else {
                    Icon::CHEVRON_DOWN
                },
                12.0,
                p.muted,
            );
            x += 18.0;
        }
        let mut top = r.top;
        let mut height = r.height();
        if view.presentation.show_icons && !row.meta.icon.is_empty() && !calendar {
            if homegrid {
                paint_row_icon(
                    list,
                    view,
                    row,
                    Rect::from_size(
                        r.left,
                        r.top + (r.height() - 64.0).max(0.0) / 2.0,
                        r.width(),
                        28.0,
                    ),
                    20.0,
                    p.muted,
                );
                top = r.top + (r.height() - 64.0).max(0.0) / 2.0 + 34.0;
                height = 28.0;
            } else {
                paint_row_icon(
                    list,
                    view,
                    row,
                    Rect::from_size(x, r.top, 22.0, r.height()),
                    16.0,
                    p.muted,
                );
                x += 28.0;
            }
        }
        let detail = !calendar
            && (!entry.grid || row.meta.always_detail)
            && (spec.appearance.show_details || row.meta.always_detail)
            && !row.detail.is_empty();
        if detail {
            if homegrid {
                top = r.top
                    + (r.height() - 90.0).max(0.0) / 2.0
                    + if view.presentation.show_icons {
                        34.0
                    } else {
                        0.0
                    };
                height = 26.0;
            } else {
                height -= 22.0;
            }
        }
        if view.module != Some(Module::Folder) || view.presentation.show_names {
            scaled(
                list,
                Rect::new(x, top, right, top + height),
                &row.title,
                spec.appearance.font_size as f32,
                p.foreground,
                if homegrid {
                    Align::Center
                } else {
                    Align::Leading
                },
            );
        }
        if row.checked == Some(true)
            && view.module == Some(Module::Schedule)
            && view.presentation.completed_behavior
                == Some(mochi_core::desktop_cards::CompletedBehavior::Strike)
        {
            let scale = spec.appearance.font_size as f32 / 16.0;
            let title = text::ellipsize(&row.title, TextStyle::Body16, (right - x) / scale);
            let width =
                (text::measure(&title, TextStyle::Body16) * scale).min((right - x).max(0.0));
            list.hline(x, x + width, top + height / 2.0, p.foreground);
        }
        if detail {
            list.text(
                Rect::new(x, top + height, right, r.bottom),
                text::ellipsize(&row.detail, TextStyle::Caption, right - x),
                TextStyle::Caption,
                p.muted,
            );
        }
        if spec.appearance.show_separators && !entry.grid {
            list.hline(
                x,
                right,
                r.bottom - 1.0,
                theme::mix(p.foreground, p.surface, 0.07),
            );
        }
    }
    if c.entries.is_empty() && !calendar {
        list.text_aligned(
            Rect::new(body.left, body.top + 22.0, body.right, body.top + 58.0),
            if view.empty.is_empty() {
                "暂无内容"
            } else {
                &view.empty
            },
            TextStyle::Label,
            p.muted,
            Align::Center,
        );
    }
    list.pop_clip();
    if c.height > body.height() {
        let h = (body.height() * body.height() / c.height).max(18.0);
        let y = body.top + (body.height() - h) * offset as f32 / (c.height - body.height());
        list.rounded_rect(
            Rect::from_size(a.right - 5.0, y, 3.0, h),
            1.5,
            theme::mix(p.muted, p.surface, 0.4),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_calendar_week_heights_and_hidden_counts_follow_content() {
        let spec = Spec {
            id: "calendar".into(),
            title: String::new(),
            x: 0,
            y: 0,
            width: 480,
            height: 340,
            locked: false,
            dark: false,
            appearance: Default::default(),
        };
        let mut view = View {
            module: Some(Module::Schedule),
            ..Default::default()
        };
        view.presentation.schedule_view = 1;
        let first = month(&view);
        for i in 0..5 {
            view.rows.push(super::super::Row {
                id: i.to_string(),
                title: format!("安排{i}"),
                meta: mochi_core::desktop_cards::RowMeta {
                    date: first.to_string(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        let area = Rect::from_size(0.0, 0.0, 480.0, 340.0);
        let body = scroll_body(&spec, &view, area);
        let expanded = content(&spec, &view, body);
        assert_eq!(expanded.entries.len(), 5);
        assert!(expanded.cells[0].0.height() > expanded.cells[7].0.height());
        assert!(expanded.cells.iter().all(|(_, _, more)| *more == 0));
        view.presentation.calendar_expanded = false;
        let collapsed = content(&spec, &view, body);
        assert_eq!(collapsed.entries.len(), 2);
        assert_eq!(collapsed.cells[0].0.height(), collapsed.cells[7].0.height());
        assert!(collapsed.cells.iter().any(|(_, _, more)| *more == 3));
        for (r, h) in controls(&spec, &view, area, 80) {
            if matches!(h, Hit::Row(_, _)) {
                assert!(r.top >= body.top);
            }
        }
    }
}

pub(super) fn item_palette(
    base: &theme::Palette,
    view: &View,
    key: &str,
) -> (theme::Palette, Option<u32>) {
    let defaults = mochi_core::desktop_cards::ItemStyle {
        foreground: view.presentation.item_foreground,
        background: view.presentation.item_background,
    };
    let style = view
        .item_styles
        .get(key)
        .cloned()
        .unwrap_or_default()
        .over(&defaults);
    let mut p = base.clone();
    if let Some(bg) = style.background {
        p.surface = bg;
        p.foreground =
            if ((bg >> 16) & 255) * 299 + ((bg >> 8) & 255) * 587 + (bg & 255) * 114 < 145000 {
                0xf0f2f4
            } else {
                0x252b32
            };
    }
    if let Some(fg) = style.foreground {
        p.foreground = fg;
    }
    p.muted = theme::mix(p.foreground, p.surface, 0.62);
    p.surface_muted = theme::mix(p.foreground, p.surface, 0.08);
    p.border = theme::mix(p.foreground, p.surface, 0.16);
    (p, style.background)
}
