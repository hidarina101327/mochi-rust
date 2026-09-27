//! 今日日程：单日 / 一周时间轴，外加右侧当天待办。
//!
//! 时间轴上所有东西都能直接操作：空白处拖出新日程，拖动块改时间，拖底边改时长，
//! 把右侧待办拖进来就排出一个时间块。已过时间但还没确认的块带琥珀描边和 ✓ ✗。

use super::visual::{self, Colors};
use super::*;

pub const HOUR_H: f32 = 52.0;
pub fn hour_height() -> f32 {
    crate::ui::settings_values::number("schedule.hourHeight", HOUR_H)
}
pub const GUTTER: f32 = 56.0;
pub fn time_gutter() -> f32 {
    crate::ui::settings_values::number("schedule.timeGutter", GUTTER)
}
pub const TODO_W: f32 = 290.0;
pub fn todo_width() -> f32 {
    crate::ui::settings_values::number("schedule.todoWidth", TODO_W)
}

#[derive(Debug, Clone)]
pub struct Geo {
    pub grid: Rect,
    /// 零点所在的 y（已减去滚动）。
    pub top: f32,
    pub hour_h: f32,
    pub days: Vec<(NaiveDate, f32, f32)>,
}

impl Geo {
    pub fn minute_at(&self, y: f32) -> i64 {
        (((y - self.top) / self.hour_h) * 60.0).clamp(0.0, 24.0 * 60.0) as i64
    }

    pub fn y_of(&self, minute: i64) -> f32 {
        self.top + minute as f32 / 60.0 * self.hour_h
    }

    pub fn date_at(&self, x: f32) -> Option<NaiveDate> {
        self.days
            .iter()
            .find(|(_, l, r)| x >= *l && x < *r)
            .map(|(d, _, _)| *d)
            .or_else(|| {
                let first = self.days.first()?;
                let last = self.days.last()?;
                Some(if x < first.1 { first.0 } else { last.0 })
            })
    }

    pub fn column(&self, date: NaiveDate) -> Option<(f32, f32)> {
        self.days
            .iter()
            .find(|(d, _, _)| *d == date)
            .map(|(_, l, r)| (*l, *r))
    }
}

pub fn clock(minute: i64) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

pub fn paint(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    panel_open: bool,
    p: &Palette,
) {
    let c = visual::colors(p);
    // 新建/编辑面板和当天待办共用右侧这一栏：面板开着就由它占位。
    let show_todos = !panel_open && !st.week && area.width() >= 720.0;
    let main = if show_todos {
        Rect::new(area.left, area.top, area.right - todo_width(), area.bottom)
    } else {
        area
    };
    let days = st.days();
    let mut y = main.top;

    // 概览条：今天被排掉多少、还剩多少、有几个待确认。
    if !st.week {
        y = paint_summary(list, main, y, st, data, &c, p);
    }
    if let Some(plan) = &st.plan {
        y = paint_plan_banner(list, lay, main, y, plan, p);
    }

    let grid_left = main.left + time_gutter();
    let col_w = ((main.right - 12.0 - grid_left) / days.len() as f32).max(40.0);
    let columns: Vec<(NaiveDate, f32, f32)> = days
        .iter()
        .enumerate()
        .map(|(i, d)| {
            (
                *d,
                grid_left + i as f32 * col_w,
                grid_left + (i + 1) as f32 * col_w,
            )
        })
        .collect();

    if st.week {
        let head = Rect::new(main.left, y, main.right, y + 44.0);
        for (date, l, r) in &columns {
            let cell = Rect::new(*l, head.top, *r, head.bottom);
            let today = *date == st.today();
            let chosen = *date == st.date;
            if chosen {
                list.rounded_rect(
                    Rect::new(
                        cell.left + 2.0,
                        cell.top + 4.0,
                        cell.right - 2.0,
                        cell.bottom - 4.0,
                    ),
                    6.0,
                    p.surface_muted,
                );
            }
            list.text_aligned(
                Rect::new(cell.left, cell.top + 4.0, cell.right, cell.top + 22.0),
                at::weekday_label(*date),
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
            list.text_aligned(
                Rect::new(cell.left, cell.top + 20.0, cell.right, cell.bottom - 2.0),
                date.day().to_string(),
                TextStyle::Label,
                if today { p.accent } else { p.foreground },
                Align::Center,
            );
            lay.push(cell, Hit::DayHead(*date));
        }
        y = head.bottom;
    }

    let from = *days.first().unwrap_or(&st.date);
    let to = *days.last().unwrap_or(&st.date);
    let slots: Vec<query::Slot> = query::slots_between(data, from, to, st.now)
        .into_iter()
        .filter(|s| s.status != EntryStatus::Cancelled)
        .collect();

    y = paint_all_day(list, lay, main, y, &columns, &slots, st, data, &c, p);
    list.hline(main.left, main.right, y, p.border);

    let grid = Rect::new(main.left, y, main.right, main.bottom);
    let content = 24.0 * hour_height() + 16.0;
    lay.viewport = grid;
    lay.content_height = content;
    let geo = Geo {
        grid,
        top: grid.top + 8.0 - st.scroll,
        hour_h: hour_height(),
        days: columns.clone(),
    };
    lay.push(grid, Hit::Grid);
    list.push_clip(grid);

    // 工作时段外加一层淡底。
    let (work_from, work_to) = work_minutes(data);
    for (_, l, r) in &columns {
        let shade = theme::mix(p.surface_muted, p.background, 0.5);
        let a = geo.y_of(0);
        let b = geo.y_of(work_from);
        list.rect(Rect::new(*l, a, *r, b), shade);
        let a = geo.y_of(work_to);
        let b = geo.y_of(24 * 60);
        list.rect(Rect::new(*l, a, *r, b), shade);
    }
    for hour in 0..=24 {
        let hy = geo.y_of(hour * 60);
        list.hline(grid_left, main.right - 12.0, hy, p.border);
        if hour < 24 {
            list.text_aligned(
                Rect::new(main.left, hy - 8.0, grid_left - 8.0, hy + 8.0),
                format!("{hour:02}:00"),
                TextStyle::Tiny,
                p.muted,
                Align::Trailing,
            );
        }
    }
    for (_, l, _) in &columns {
        list.vline(*l, geo.y_of(0), geo.y_of(24 * 60), p.border);
    }

    let dragging_key = st
        .drag
        .as_ref()
        .filter(|d| d.moved)
        .and_then(|d| match &d.kind {
            DragKind::Move { key } | DragKind::Resize { key } => Some(key.clone()),
            _ => None,
        });

    for (date, left, right) in &columns {
        let day_start = date.and_time(chrono::NaiveTime::MIN);
        let day_end = at::add_days(*date, 1).and_time(chrono::NaiveTime::MIN);
        let mut todays: Vec<&query::Slot> = slots
            .iter()
            .filter(|s| !s.all_day && s.start < day_end && s.end > day_start)
            .collect();
        todays.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
        let spans: Vec<(NaiveDateTime, NaiveDateTime)> = todays
            .iter()
            .map(|s| (s.start.max(day_start), s.end.min(day_end)))
            .collect();
        let lanes = query::assign_lanes(&spans);
        for ((slot, (s, e)), (lane, count)) in todays.iter().zip(&spans).zip(lanes) {
            if dragging_key.as_deref() == Some(slot.key.as_str()) {
                continue;
            }
            let w = (right - left - 6.0) / count.max(1) as f32;
            let top = geo.y_of(at::minutes_between(day_start, *s));
            let bottom = geo.y_of(at::minutes_between(day_start, *e)).max(top + 18.0);
            let r = Rect::new(
                left + 2.0 + lane as f32 * w,
                top + 1.0,
                left + 2.0 + (lane + 1) as f32 * w - 2.0,
                bottom - 1.0,
            );
            paint_slot(list, lay, grid, r, slot, data, st, &c, p);
        }
    }

    // 自动排入预览。
    if let Some(plan) = &st.plan {
        for placement in plan {
            let Some((l, r)) = geo.column(placement.start.date()) else {
                continue;
            };
            let top = geo.y_of(at::minute_of_day(placement.start) as i64);
            let bottom = geo.y_of(at::minute_of_day(placement.end) as i64);
            let rect = Rect::new(l + 3.0, top + 1.0, r - 4.0, bottom - 1.0);
            list.rounded_rect_alpha(rect, 6.0, p.accent, 0.12);
            list.rounded_border(rect, 6.0, p.accent);
            list.text(
                Rect::new(
                    rect.left + 8.0,
                    rect.top + 2.0,
                    rect.right - 4.0,
                    rect.top + 20.0,
                ),
                text::ellipsize(
                    &format!("建议 · {}", placement.title),
                    TextStyle::Small,
                    rect.width() - 12.0,
                ),
                TextStyle::Small,
                p.accent,
            );
            if rect.height() > 38.0 {
                list.text(
                    Rect::new(
                        rect.left + 8.0,
                        rect.top + 20.0,
                        rect.right - 4.0,
                        rect.top + 36.0,
                    ),
                    text::ellipsize(&placement.reason, TextStyle::Tiny, rect.width() - 12.0),
                    TextStyle::Tiny,
                    p.muted,
                );
            }
        }
    }

    // 拖拽中的幽灵块。
    if let Some(drag) = st
        .drag
        .as_ref()
        .filter(|d| d.moved || matches!(d.kind, DragKind::Create))
    {
        if drag.moved && drag.inside {
            if let Some((l, r)) = geo.column(drag.date) {
                let rect = Rect::new(
                    l + 3.0,
                    geo.y_of(drag.start) + 1.0,
                    r - 4.0,
                    geo.y_of(drag.end) - 1.0,
                );
                list.rounded_rect_alpha(rect, 6.0, p.accent, 0.22);
                list.rounded_border(rect, 6.0, p.accent);
                let label = match &drag.kind {
                    DragKind::Create => "新日程".to_owned(),
                    DragKind::Todo { task_id, .. } => data.title_of(task_id).unwrap_or_default(),
                    DragKind::Move { key } | DragKind::Resize { key } => {
                        query::slot_by_key(data, key, st.now)
                            .map(|s| s.title)
                            .unwrap_or_default()
                    }
                };
                list.text(
                    Rect::new(
                        rect.left + 8.0,
                        rect.top + 2.0,
                        rect.right - 4.0,
                        rect.top + 20.0,
                    ),
                    text::ellipsize(&label, TextStyle::Small, rect.width() - 12.0),
                    TextStyle::Small,
                    p.foreground,
                );
                list.text(
                    Rect::new(
                        rect.left + 8.0,
                        rect.top + 18.0,
                        rect.right - 4.0,
                        rect.top + 34.0,
                    ),
                    format!(
                        "{} – {} · {}",
                        clock(drag.start),
                        clock(drag.end),
                        at::duration_label(drag.end - drag.start)
                    ),
                    TextStyle::Tiny,
                    p.muted,
                );
            }
        }
    }

    // 现在线。
    if let Some((l, r)) = geo.column(st.today()) {
        let ny = geo.y_of(at::minute_of_day(st.now) as i64);
        list.rect(Rect::new(l, ny - 1.0, r, ny + 1.0), c.now);
        list.rounded_rect(Rect::new(l - 4.0, ny - 4.0, l + 4.0, ny + 4.0), 4.0, c.now);
        list.text_aligned(
            Rect::new(main.left, ny - 8.0, grid_left - 8.0, ny + 8.0),
            at::hm(st.now.time()),
            TextStyle::Tiny,
            c.now,
            Align::Trailing,
        );
    }
    list.pop_clip();
    lay.geo = Some(geo);

    if show_todos {
        paint_todos(
            list,
            lay,
            Rect::new(main.right, area.top, area.right, area.bottom),
            st,
            data,
            &c,
            p,
        );
    }
}

fn paint_summary(
    list: &mut DrawList,
    main: Rect,
    y: f32,
    st: &State,
    data: &AgendaData,
    c: &Colors,
    p: &Palette,
) -> f32 {
    let (busy, capacity) = planner::day_load(data, st.date, st.now);
    let free = planner::free_slots(data, st.date, st.now, None, 15)
        .iter()
        .map(|s| s.minutes())
        .sum::<i64>();
    let confirm = query::confirm_queue(data, st.now).len();
    let row = Rect::new(main.left + 20.0, y + 8.0, main.right - 20.0, y + 30.0);
    let mut parts = vec![format!("已安排 {}", at::duration_label(busy))];
    if st.date >= st.today() {
        parts.push(format!("还有 {} 空闲", at::duration_label(free)));
    }
    let mut notice = String::new();
    if confirm > 0 {
        notice = format!("{confirm} 项待确认（左侧「待确认」）");
        if text::measure(&notice, TextStyle::Small) > row.width() * 0.4 {
            notice = format!("{confirm} 待确认");
        }
    }
    let notice_w = if notice.is_empty() {
        0.0
    } else {
        text::measure(&notice, TextStyle::Small) + 16.0
    };
    let label = text::ellipsize(
        &parts.join(" · "),
        TextStyle::Small,
        (row.width() - notice_w).max(0.0),
    );
    list.text_aligned(row, &label, TextStyle::Small, p.muted, Align::Leading);
    let bar_left = row.left + text::measure(&label, TextStyle::Small) + 14.0;
    let bar = Rect::new(
        bar_left,
        row.top + 8.0,
        (bar_left + 160.0).min(row.right - notice_w),
        row.top + 13.0,
    );
    if bar.width() > 40.0 {
        let ratio = busy as f32 / capacity.max(1) as f32;
        let color = if ratio > 0.9 {
            c.danger
        } else if ratio > 0.7 {
            c.warning
        } else {
            c.success
        };
        visual::progress_bar(list, bar, ratio, color, p);
    }
    if !notice.is_empty() {
        list.text_aligned(row, notice, TextStyle::Small, c.warning, Align::Trailing);
    }
    y + 36.0
}

fn paint_plan_banner(
    list: &mut DrawList,
    lay: &mut Layout,
    main: Rect,
    y: f32,
    plan: &[Placement],
    p: &Palette,
) -> f32 {
    let r = Rect::new(main.left + 16.0, y + 4.0, main.right - 16.0, y + 40.0);
    list.rounded_rect(r, 8.0, theme::mix(p.accent, p.background, 0.1));
    let msg = if plan.is_empty() {
        "没有需要排入的待办，或今天已经没有合适的空闲时段".to_owned()
    } else {
        let minutes: i64 = plan
            .iter()
            .map(|x| at::minutes_between(x.start, x.end))
            .sum();
        format!(
            "自动排入预览：{} 个时间块，共 {} · 虚线为建议位置",
            plan.len(),
            at::duration_label(minutes)
        )
    };
    list.icon_centered(
        Rect::new(r.left + 8.0, r.top, r.left + 28.0, r.bottom),
        Icon::ZAP,
        14.0,
        p.accent,
    );
    list.text_aligned(
        Rect::new(r.left + 32.0, r.top, r.right - 180.0, r.bottom),
        text::ellipsize(&msg, TextStyle::Small, r.width() - 220.0),
        TextStyle::Small,
        p.foreground,
        Align::Leading,
    );
    let mut x = r.right - 8.0;
    let w = button_width("放弃", None);
    button(
        list,
        lay,
        Rect::new(x - w, r.top + 5.0, x, r.bottom - 5.0),
        "放弃",
        None,
        Btn::Ghost,
        Hit::PlanDiscard,
        p,
    );
    x -= w + 6.0;
    if !plan.is_empty() {
        let w = button_width("全部接受", None);
        button(
            list,
            lay,
            Rect::new(x - w, r.top + 5.0, x, r.bottom - 5.0),
            "全部接受",
            None,
            Btn::Primary,
            Hit::PlanAccept,
            p,
        );
    }
    y + 46.0
}

#[allow(clippy::too_many_arguments)]
fn paint_all_day(
    list: &mut DrawList,
    lay: &mut Layout,
    main: Rect,
    y: f32,
    columns: &[(NaiveDate, f32, f32)],
    slots: &[query::Slot],
    st: &State,
    data: &AgendaData,
    c: &Colors,
    p: &Palette,
) -> f32 {
    let per_day: Vec<Vec<&query::Slot>> = columns
        .iter()
        .map(|(date, _, _)| {
            let start = date.and_time(chrono::NaiveTime::MIN);
            let end = at::add_days(*date, 1).and_time(chrono::NaiveTime::MIN);
            slots
                .iter()
                .filter(|s| s.all_day && s.start < end && s.end > start)
                .collect()
        })
        .collect();
    let rows = per_day.iter().map(Vec::len).max().unwrap_or(0).min(3);
    if rows == 0 {
        return y;
    }
    let h = rows as f32 * 24.0 + 8.0;
    list.text_aligned(
        Rect::new(
            main.left,
            y + 4.0,
            main.left + time_gutter() - 8.0,
            y + 24.0,
        ),
        "全天",
        TextStyle::Tiny,
        p.muted,
        Align::Trailing,
    );
    for ((_, l, r), items) in columns.iter().zip(&per_day) {
        for (i, slot) in items.iter().enumerate() {
            let top = y + 4.0 + i as f32 * 24.0;
            let rect = Rect::new(l + 3.0, top, r - 4.0, top + 21.0);
            if i == 2 && items.len() > 3 {
                list.text_aligned(
                    rect,
                    format!("还有 {} 项", items.len() - 2),
                    TextStyle::Tiny,
                    p.muted,
                    Align::Leading,
                );
                lay.push(rect, Hit::DayHead(slot.start.date()));
                break;
            }
            let color = visual::slot_color(data, slot, c);
            let alpha = if slot.status == EntryStatus::Planned {
                0.2
            } else {
                0.08
            };
            list.rounded_rect_alpha(rect, 5.0, color, alpha);
            if st.selected.as_deref() == Some(slot.key.as_str()) {
                list.rounded_border(rect, 5.0, color);
            }
            let fg = if slot.status == EntryStatus::Planned {
                p.foreground
            } else {
                p.muted
            };
            list.text_aligned(
                Rect::new(rect.left + 8.0, rect.top, rect.right - 4.0, rect.bottom),
                text::ellipsize(&slot.title, TextStyle::Small, rect.width() - 12.0),
                TextStyle::Small,
                fg,
                Align::Leading,
            );
            lay.push(rect, Hit::Slot(slot.key.clone()));
        }
    }
    y + h
}

#[allow(clippy::too_many_arguments)]
fn paint_slot(
    list: &mut DrawList,
    lay: &mut Layout,
    clip: Rect,
    r: Rect,
    slot: &query::Slot,
    data: &AgendaData,
    st: &State,
    c: &Colors,
    p: &Palette,
) {
    let color = visual::slot_color(data, slot, c);
    let (alpha, fg) = match slot.status {
        EntryStatus::Planned => (0.2, p.foreground),
        EntryStatus::Done => (0.1, p.muted),
        _ => (0.06, p.muted),
    };
    list.rounded_rect(r, 6.0, p.background);
    list.rounded_rect_alpha(r, 6.0, color, alpha);
    list.rounded_rect(
        Rect::new(r.left, r.top + 2.0, r.left + 3.0, r.bottom - 2.0),
        1.5,
        color,
    );
    let selected = st.selected.as_deref() == Some(slot.key.as_str());
    if slot.needs_confirm {
        list.rounded_border(r, 6.0, c.warning);
    } else if selected || slot.is_now(st.now) {
        list.rounded_border(r, 6.0, color);
    }
    let mut title = slot.title.clone();
    if slot.task_deleted {
        title.push_str("（原任务已删除）");
    }
    let mut left = r.left + 8.0;
    if slot.routine_id.is_some() {
        list.icon_centered(
            Rect::new(left, r.top + 2.0, left + 14.0, r.top + 20.0),
            Icon::REPEAT,
            11.0,
            color,
        );
        left += 16.0;
    } else if slot.task_id.is_some() {
        let done = slot.task_status == Some(TaskStatus::Done);
        visual::check_circle(
            list,
            Rect::new(left, r.top + 3.0, left + 13.0, r.top + 19.0),
            done,
            color,
            p,
        );
        left += 17.0;
    }
    let buttons = if slot.needs_confirm && r.width() > 90.0 {
        48.0
    } else {
        0.0
    };
    let title_w = (r.right - 4.0 - buttons - left).max(0.0);
    let shown = text::ellipsize(&title, TextStyle::Small, title_w);
    let tr = Rect::new(
        left,
        r.top + 1.0,
        r.right - 4.0 - buttons,
        (r.top + 21.0).min(r.bottom),
    );
    list.text(tr, shown.clone(), TextStyle::Small, fg);
    if slot.status == EntryStatus::Skipped {
        let w = text::measure(&shown, TextStyle::Small);
        list.hline(left, left + w, tr.top + 10.0, p.muted);
    }
    if slot.status == EntryStatus::Done {
        list.icon_centered(
            Rect::new(r.right - 20.0, r.top + 2.0, r.right - 4.0, r.top + 20.0),
            Icon::CHECK,
            12.0,
            c.success,
        );
    }
    if r.height() >= 36.0 {
        let mut meta = format!(
            "{}–{} · {}",
            at::hm(slot.start.time()),
            at::hm(slot.end.time()),
            at::duration_short(slot.minutes())
        );
        if !slot.location.is_empty() {
            meta.push_str(" · ");
            meta.push_str(&slot.location);
        }
        if slot.needs_confirm {
            meta = format!("待确认 · {meta}");
        }
        list.text(
            Rect::new(
                r.left + 8.0,
                r.top + 19.0,
                r.right - 4.0,
                (r.top + 35.0).min(r.bottom),
            ),
            text::ellipsize(&meta, TextStyle::Tiny, r.width() - 12.0),
            TextStyle::Tiny,
            if slot.needs_confirm {
                c.warning
            } else {
                p.muted
            },
        );
    }
    lay.push_clipped(r, clip, Hit::Slot(slot.key.clone()));
    if !slot.all_day && r.height() > 14.0 {
        let handle = Rect::new(r.left, r.bottom - 5.0, r.right, r.bottom + 1.0);
        lay.push_clipped(handle, clip, Hit::SlotResize(slot.key.clone()));
    }
    if buttons > 0.0 {
        let b1 = Rect::new(r.right - 48.0, r.top + 3.0, r.right - 26.0, r.top + 21.0);
        let b2 = Rect::new(r.right - 24.0, r.top + 3.0, r.right - 2.0, r.top + 21.0);
        list.rounded_rect(b1, 5.0, theme::mix(c.success, p.background, 0.2));
        list.icon_centered(b1, Icon::CHECK, 11.0, c.success);
        list.rounded_rect(b2, 5.0, theme::mix(c.danger, p.background, 0.16));
        list.icon_centered(b2, Icon::X, 11.0, c.danger);
        lay.push_clipped(
            b1,
            clip,
            Hit::SlotConfirm(slot.key.clone(), EntryStatus::Done),
        );
        lay.push_clipped(
            b2,
            clip,
            Hit::SlotConfirm(slot.key.clone(), EntryStatus::Skipped),
        );
    }
}

/// 右侧当天待办。按分组依次排列，同一任务只出现一次。
pub fn todo_groups(
    data: &AgendaData,
    date: NaiveDate,
    now: NaiveDateTime,
) -> Vec<(&'static str, Vec<String>)> {
    let todos = query::day_todos(data, date, now);
    let mut listed: Vec<String> = Vec::new();
    let mut groups = Vec::new();
    for (label, ids) in [
        ("逾期", todos.overdue),
        ("今天截止", todos.due),
        ("计划今天做", todos.planned),
        ("已排时间", todos.scheduled),
        ("进行中", todos.doing),
    ] {
        if !ids.is_empty() {
            listed.extend(ids.iter().cloned());
            groups.push((label, ids));
        }
    }
    if date >= now.date() {
        let mut backlog: Vec<&core::Task> = data
            .tasks
            .iter()
            .filter(|t| {
                query::is_visible_task(t)
                    && t.status.is_open()
                    && t.due.is_none()
                    && t.planned_for.is_none()
            })
            .filter(|t| !listed.contains(&t.id) && !query::task_facts(data, t, now).scheduled)
            .collect();
        query::sort_tasks(&mut backlog, now);
        let ids: Vec<String> = backlog.into_iter().take(12).map(|t| t.id.clone()).collect();
        if !ids.is_empty() {
            groups.push(("待安排", ids));
        }
    }
    if !todos.done.is_empty() {
        groups.push(("今天完成", todos.done));
    }
    groups
}

const CARD_H: f32 = 54.0;
pub fn todo_card_height() -> f32 {
    crate::ui::settings_values::number("schedule.todoCardHeight", CARD_H)
}

#[allow(clippy::too_many_arguments)]
fn paint_todos(
    list: &mut DrawList,
    lay: &mut Layout,
    area: Rect,
    st: &State,
    data: &AgendaData,
    c: &Colors,
    p: &Palette,
) {
    list.rect(area, theme::mix(p.surface_muted, p.background, 0.45));
    list.vline(area.left, area.top, area.bottom, p.border);
    let head = Rect::new(
        area.left + 14.0,
        area.top + 8.0,
        area.right - 12.0,
        area.top + 36.0,
    );
    let label = if st.date == st.today() {
        "今天的待办".to_owned()
    } else {
        format!("{}的待办", at::relative_date_label(st.date, st.today()))
    };
    list.text_aligned(head, label, TextStyle::Label, p.foreground, Align::Leading);
    let w = button_width("新待办", Some(Icon::PLUS));
    button(
        list,
        lay,
        Rect::new(head.right - w, head.top, head.right, head.bottom),
        "新待办",
        Some(Icon::PLUS),
        Btn::Ghost,
        Hit::New(Kind::Task),
        p,
    );
    let view = Rect::new(area.left + 1.0, head.bottom + 6.0, area.right, area.bottom);
    lay.push(view, Hit::Blank);
    let groups = todo_groups(data, st.date, st.now);
    list.push_clip(view);
    let mut y = view.top - st.todo_scroll;
    if groups.is_empty() {
        list.text_aligned(
            Rect::new(view.left + 14.0, y + 20.0, view.right - 14.0, y + 44.0),
            "今天没有待办，轻松一点。",
            TextStyle::Small,
            p.muted,
            Align::Leading,
        );
        y += 60.0;
    }
    for (label, ids) in &groups {
        let color = match *label {
            "逾期" => c.danger,
            "今天截止" => c.warning,
            _ => p.muted,
        };
        list.text(
            Rect::new(view.left + 14.0, y + 6.0, view.right - 12.0, y + 24.0),
            format!("{label} · {}", ids.len()),
            TextStyle::Caption,
            color,
        );
        y += 28.0;
        for id in ids {
            let Some(task) = data.task(id) else { continue };
            let r = Rect::new(
                view.left + 10.0,
                y,
                view.right - 10.0,
                y + todo_card_height() - 6.0,
            );
            paint_todo_card(list, lay, view, r, task, data, st, c, p);
            y += todo_card_height();
        }
        y += 4.0;
    }
    list.text(
        Rect::new(view.left + 14.0, y + 6.0, view.right - 12.0, y + 24.0),
        "把待办拖到左侧时间轴，就能排出时间块",
        TextStyle::Tiny,
        p.muted,
    );
    y += 32.0;
    list.pop_clip();
    lay.todo_rect = view;
    lay.todo_height = y + st.todo_scroll - view.top;
}

#[allow(clippy::too_many_arguments)]
pub fn paint_todo_card(
    list: &mut DrawList,
    lay: &mut Layout,
    clip: Rect,
    r: Rect,
    task: &core::Task,
    data: &AgendaData,
    st: &State,
    c: &Colors,
    p: &Palette,
) {
    let done = task.status == TaskStatus::Done;
    let selected = st.selected.as_deref() == Some(task.id.as_str());
    list.rounded_rect(r, 8.0, p.background);
    list.rounded_border(r, 8.0, if selected { p.accent } else { p.border });
    if let Some(color) = visual::priority_color(c, task.priority) {
        list.rounded_rect(
            Rect::new(r.left, r.top + 8.0, r.left + 3.0, r.bottom - 8.0),
            1.5,
            color,
        );
    }
    let check = Rect::new(r.left + 6.0, r.top + 4.0, r.left + 30.0, r.top + 26.0);
    visual::check_circle(list, check, done, if done { c.success } else { p.muted }, p);
    let title_r = Rect::new(r.left + 32.0, r.top + 4.0, r.right - 8.0, r.top + 26.0);
    let shown = text::ellipsize(&task.title, TextStyle::Small, title_r.width());
    list.text_aligned(
        title_r,
        shown.clone(),
        TextStyle::Small,
        if done { p.muted } else { p.foreground },
        Align::Leading,
    );
    if done {
        list.hline(
            title_r.left,
            title_r.left + text::measure(&shown, TextStyle::Small),
            title_r.top + 11.0,
            p.muted,
        );
    }
    let overdue = query::is_overdue(task, st.now);
    let meta: Vec<String> = task_meta(data, task, st.now)
        .into_iter()
        .map(|(s, _)| s)
        .collect();
    let meta = if meta.is_empty() {
        task.goal_id
            .as_deref()
            .map(|g| format!("为了「{}」", data.title_of(g).unwrap_or_default()))
            .unwrap_or_else(|| "未排时间".into())
    } else {
        meta.join(" · ")
    };
    list.text_aligned(
        Rect::new(r.left + 32.0, r.top + 25.0, r.right - 8.0, r.bottom - 3.0),
        text::ellipsize(&meta, TextStyle::Tiny, r.width() - 42.0),
        TextStyle::Tiny,
        if overdue { c.danger } else { p.muted },
        Align::Leading,
    );
    lay.push_clipped(r, clip, Hit::Todo(task.id.clone()));
    lay.push_clipped(check, clip, Hit::Check(task.id.clone()));
}
