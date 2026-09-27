//! 月历：日程月历（每格显示当天的日程）与待办月历（每格显示当天截止 / 计划的待办）。
//! 点格子进入那一天的时间轴。

use std::collections::HashMap;

use super::visual;
use super::*;

pub fn paint(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    todos: bool,
    p: &Palette,
) {
    let c = visual::colors(p);
    let first = at::week_start(at::month_start(st.month));
    let last = at::add_days(first, 41);
    let inner = Rect::new(
        area.left + 16.0,
        area.top + 8.0,
        area.right - 16.0,
        area.bottom - 12.0,
    );
    let head_h = 26.0;
    let col_w = inner.width() / 7.0;
    for (i, name) in ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
        .iter()
        .enumerate()
    {
        let r = Rect::new(
            inner.left + i as f32 * col_w,
            inner.top,
            inner.left + (i + 1) as f32 * col_w,
            inner.top + head_h,
        );
        list.text_aligned(r, *name, TextStyle::Caption, p.muted, Align::Center);
    }
    let grid_top = inner.top + head_h;
    let row_h = ((inner.bottom - grid_top) / 6.0).max(64.0);
    lay.viewport = Rect::new(inner.left, grid_top, inner.right, inner.bottom);
    lay.content_height = row_h * 6.0;
    list.push_clip(lay.viewport);
    let top = grid_top - st.scroll;

    let summaries: HashMap<String, query::DaySummary> = if todos {
        HashMap::new()
    } else {
        query::range_summary(data, first, last, st.now)
            .into_iter()
            .map(|s| (s.date.clone(), s))
            .collect()
    };
    let mut tasks_by_day: HashMap<NaiveDate, Vec<(&core::Task, bool)>> = HashMap::new();
    if todos {
        for task in data.tasks.iter().filter(|t| query::is_visible_task(t)) {
            let shown = task.status.is_open() || (st.show_done && task.status == TaskStatus::Done);
            if !shown {
                continue;
            }
            if let Some(due) = task.due.as_deref().and_then(at::Due::parse) {
                tasks_by_day
                    .entry(due.date())
                    .or_default()
                    .push((task, true));
            } else if let Some(planned) = task.planned_for.as_deref().and_then(at::parse_date) {
                tasks_by_day.entry(planned).or_default().push((task, false));
            }
        }
        for items in tasks_by_day.values_mut() {
            items.sort_by(|a, b| {
                b.0.priority
                    .rank()
                    .cmp(&a.0.priority.rank())
                    .then(a.0.title.cmp(&b.0.title))
            });
        }
    }

    for index in 0..42 {
        let date = at::add_days(first, index);
        let row = (index / 7) as f32;
        let col = (index % 7) as f32;
        let cell = Rect::new(
            inner.left + col * col_w,
            top + row * row_h,
            inner.left + (col + 1.0) * col_w,
            top + (row + 1.0) * row_h,
        );
        let in_month = date.month() == st.month.month();
        let today = date == st.today();
        let hovered = st.pointer.is_some_and(|(x, y)| cell.contains(x, y));
        let bg = if hovered {
            p.surface_muted
        } else if in_month {
            p.background
        } else {
            theme::mix(p.surface_muted, p.background, 0.4)
        };
        list.rect(
            Rect::new(
                cell.left + 0.5,
                cell.top + 0.5,
                cell.right - 0.5,
                cell.bottom - 0.5,
            ),
            bg,
        );
        list.hline(cell.left, cell.right, cell.top, p.border);
        list.vline(cell.left, cell.top, cell.bottom, p.border);
        lay.push_clipped(cell, lay.viewport, Hit::Day(date));

        let num = Rect::new(
            cell.left + 6.0,
            cell.top + 4.0,
            cell.left + 30.0,
            cell.top + 26.0,
        );
        if today {
            list.rounded_rect(num, 11.0, p.accent);
        }
        list.text_aligned(
            num,
            date.day().to_string(),
            TextStyle::Small,
            if today {
                p.accent_foreground
            } else if in_month {
                p.foreground
            } else {
                p.muted
            },
            Align::Center,
        );
        let line_h = 19.0;
        let mut y = cell.top + 30.0;
        let capacity = (((cell.bottom - 4.0 - y) / line_h).floor() as usize).max(1);
        if todos {
            let items = tasks_by_day.get(&date).map(Vec::as_slice).unwrap_or(&[]);
            let overdue = items
                .iter()
                .filter(|(t, _)| query::is_overdue(t, st.now))
                .count();
            if overdue > 0 {
                visual::badge(
                    list,
                    cell.right - 60.0,
                    cell.top + 5.0,
                    &format!("{overdue} 逾期"),
                    c.danger,
                    p,
                );
            }
            let shown = if items.len() > capacity {
                capacity - 1
            } else {
                items.len()
            };
            for (task, is_due) in items.iter().take(shown) {
                let r = Rect::new(cell.left + 4.0, y, cell.right - 4.0, y + line_h - 2.0);
                let done = task.status == TaskStatus::Done;
                let color = if query::is_overdue(task, st.now) {
                    c.danger
                } else if *is_due {
                    c.task
                } else {
                    p.muted
                };
                visual::check_circle(
                    list,
                    Rect::new(r.left, r.top, r.left + 14.0, r.bottom),
                    done,
                    color,
                    p,
                );
                let label = if *is_due {
                    task.title.clone()
                } else {
                    format!("计划 · {}", task.title)
                };
                list.text_aligned(
                    Rect::new(r.left + 18.0, r.top, r.right, r.bottom),
                    text::ellipsize(&label, TextStyle::Tiny, r.width() - 20.0),
                    TextStyle::Tiny,
                    if done { p.muted } else { p.foreground },
                    Align::Leading,
                );
                lay.push_clipped(r, lay.viewport, Hit::Todo(task.id.clone()));
                y += line_h;
            }
            if items.len() > shown {
                list.text(
                    Rect::new(cell.left + 8.0, y, cell.right - 4.0, y + line_h),
                    format!("还有 {} 项", items.len() - shown),
                    TextStyle::Tiny,
                    p.muted,
                );
            }
        } else if let Some(sum) = summaries.get(&at::date_key(date)) {
            if sum.needs_confirm > 0 {
                visual::badge(
                    list,
                    cell.right - 58.0,
                    cell.top + 5.0,
                    &format!("{} 待确认", sum.needs_confirm),
                    c.warning,
                    p,
                );
            } else if sum.busy_minutes > 0 {
                list.text_aligned(
                    Rect::new(cell.left, cell.top + 5.0, cell.right - 8.0, cell.top + 23.0),
                    at::duration_short(sum.busy_minutes),
                    TextStyle::Tiny,
                    p.muted,
                    Align::Trailing,
                );
            }
            let total = sum.slots;
            let shown = if total > capacity {
                capacity - 1
            } else {
                total
            }
            .min(sum.titles.len());
            for (title, status, color) in sum.titles.iter().take(shown) {
                let r = Rect::new(cell.left + 4.0, y, cell.right - 4.0, y + line_h - 2.0);
                let tint = color
                    .as_deref()
                    .and_then(visual::parse_color)
                    .unwrap_or(c.entry);
                let planned = *status == EntryStatus::Planned;
                list.rounded_rect_alpha(r, 4.0, tint, if planned { 0.16 } else { 0.06 });
                list.rounded_rect(
                    Rect::new(r.left, r.top + 3.0, r.left + 2.5, r.bottom - 3.0),
                    1.0,
                    tint,
                );
                let label = text::ellipsize(title, TextStyle::Tiny, r.width() - 12.0);
                list.text_aligned(
                    Rect::new(r.left + 7.0, r.top, r.right - 2.0, r.bottom),
                    label.clone(),
                    TextStyle::Tiny,
                    if planned { p.foreground } else { p.muted },
                    Align::Leading,
                );
                if *status == EntryStatus::Skipped {
                    list.hline(
                        r.left + 7.0,
                        r.left + 7.0 + text::measure(&label, TextStyle::Tiny),
                        r.top + 9.0,
                        p.muted,
                    );
                }
                y += line_h;
            }
            if total > shown {
                list.text(
                    Rect::new(cell.left + 8.0, y, cell.right - 4.0, y + line_h),
                    format!("还有 {} 项", total - shown),
                    TextStyle::Tiny,
                    p.muted,
                );
            }
            if sum.due_tasks > 0 && y + line_h < cell.bottom {
                let label = if sum.overdue_tasks > 0 {
                    format!("{} 项待办截止 · {} 逾期", sum.due_tasks, sum.overdue_tasks)
                } else {
                    format!("{} 项待办截止", sum.due_tasks)
                };
                list.text(
                    Rect::new(
                        cell.left + 8.0,
                        cell.bottom - 20.0,
                        cell.right - 4.0,
                        cell.bottom - 2.0,
                    ),
                    text::ellipsize(&label, TextStyle::Tiny, cell.width() - 12.0),
                    TextStyle::Tiny,
                    if sum.overdue_tasks > 0 {
                        c.danger
                    } else {
                        c.task
                    },
                );
            }
        }
    }
    list.pop_clip();
}
