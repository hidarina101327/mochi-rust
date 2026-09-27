//! 日程待办的左侧栏：此刻卡片、视图导航（带计数与待确认角标）、迷你月历。

use super::visual;
use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum SideHit {
    View(View),
    Day(NaiveDate),
    MonthPrev,
    MonthNext,
    New,
    Open(String),
}

#[derive(Debug, Clone, Default)]
pub struct SideLayout {
    pub entries: Vec<(Rect, SideHit)>,
}

impl SideLayout {
    fn push(&mut self, r: Rect, hit: SideHit) {
        self.entries.push((r, hit));
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<SideHit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h.clone())
    }
}

/// 每个视图旁边显示的数字：`(数量, 是否需要提醒)`。
fn count(data: &AgendaData, view: View, now: NaiveDateTime) -> Option<(usize, bool)> {
    let n = match view {
        View::Day => query::day_slots(data, now.date(), now)
            .iter()
            .filter(|s| s.status != EntryStatus::Cancelled)
            .count(),
        View::Confirm => {
            return Some((query::confirm_queue(data, now).len(), true)).filter(|(n, _)| *n > 0)
        }
        View::Todos => {
            let open: Vec<&core::Task> = data
                .tasks
                .iter()
                .filter(|t| query::is_visible_task(t) && t.status.is_open())
                .collect();
            let overdue = open.iter().filter(|t| query::is_overdue(t, now)).count();
            if overdue > 0 {
                return Some((overdue, true));
            }
            open.len()
        }
        View::Goals => {
            data.goals
                .iter()
                .filter(|g| g.deleted_at.is_none() && g.archived_at.is_none() && g.status.is_open())
                .count()
                + data
                    .wishes
                    .iter()
                    .filter(|w| {
                        w.deleted_at.is_none()
                            && w.archived_at.is_none()
                            && w.status == core::WishStatus::Open
                    })
                    .count()
        }
        View::Routines => data
            .routines
            .iter()
            .filter(|r| {
                r.deleted_at.is_none()
                    && r.archived_at.is_none()
                    && r.status == core::RoutineStatus::Active
            })
            .count(),
        View::Projects => data
            .projects
            .iter()
            .filter(|p| p.deleted_at.is_none() && p.archived_at.is_none())
            .count(),
        View::Trash => query::trash(data).len(),
        _ => 0,
    };
    (n > 0).then_some((n, false))
}

pub fn paint(list: &mut DrawList, area: Rect, st: &State, p: &Palette) -> SideLayout {
    let mut lay = SideLayout::default();
    let c = visual::colors(p);
    let left = area.left + 12.0;
    let right = area.right - 12.0;
    let mut y = area.top + 12.0;
    list.text_aligned(
        Rect::new(left + 4.0, y, right - 30.0, y + 26.0),
        "日程待办",
        TextStyle::Label,
        p.foreground,
        Align::Leading,
    );
    let plus = Rect::new(right - 26.0, y, right, y + 26.0);
    list.rounded_border(plus, 6.0, p.border);
    list.icon_centered(plus, Icon::PLUS, 13.0, p.foreground);
    lay.push(plus, SideHit::New);
    y += 36.0;

    let Some(data) = st.data.as_ref() else {
        list.text(
            Rect::new(left, y, right, y + 20.0),
            "正在读取…",
            TextStyle::Small,
            p.muted,
        );
        return lay;
    };

    // 此刻：正在进行 / 下一个。
    let (current, next) = query::now_and_next(data, st.now);
    let card_h = match (&current, &next) {
        (Some(_), Some(_)) => 92.0,
        (Some(_), None) => 70.0,
        (None, _) => 56.0,
    };
    let card = Rect::new(left, y, right, y + card_h);
    list.rounded_rect(card, 10.0, theme::mix(p.surface_muted, p.background, 0.4));
    list.rounded_border(card, 10.0, p.border);
    let inner_l = card.left + 12.0;
    let inner_r = card.right - 10.0;
    let mut cy = card.top + 10.0;
    match &current {
        Some(slot) => {
            let color = visual::slot_color(data, slot, &c);
            list.rounded_rect(
                Rect::new(inner_l, cy + 5.0, inner_l + 8.0, cy + 13.0),
                4.0,
                color,
            );
            list.text_aligned(
                Rect::new(inner_l + 14.0, cy, inner_r, cy + 18.0),
                text::ellipsize(&slot.title, TextStyle::Small, inner_r - inner_l - 14.0),
                TextStyle::Small,
                p.foreground,
                Align::Leading,
            );
            cy += 22.0;
            let total = slot.minutes().max(1);
            let passed = at::minutes_between(slot.start, st.now).clamp(0, total);
            visual::progress_bar(
                list,
                Rect::new(inner_l, cy + 2.0, inner_r, cy + 6.0),
                passed as f32 / total as f32,
                color,
                p,
            );
            cy += 10.0;
            list.text(
                Rect::new(inner_l, cy, inner_r, cy + 16.0),
                format!("进行中 · 还剩 {}", at::duration_label(total - passed)),
                TextStyle::Tiny,
                p.muted,
            );
            lay.push(
                Rect::new(card.left, card.top, card.right, cy + 16.0),
                SideHit::Open(slot.key.clone()),
            );
            cy += 20.0;
        }
        None => {
            list.text(
                Rect::new(inner_l, cy, inner_r, cy + 16.0),
                "此刻没有安排",
                TextStyle::Tiny,
                p.muted,
            );
            cy += 18.0;
        }
    }
    match &next {
        Some(slot) => {
            let wait = at::minutes_between(st.now, slot.start);
            let label = format!("下一个 {} · {}", at::hm(slot.start.time()), slot.title);
            list.text(
                Rect::new(inner_l, cy, inner_r, cy + 16.0),
                text::ellipsize(&label, TextStyle::Tiny, inner_r - inner_l),
                TextStyle::Tiny,
                p.foreground,
            );
            list.text(
                Rect::new(inner_l, cy + 14.0, inner_r, cy + 30.0),
                format!("{} 后开始", at::duration_label(wait)),
                TextStyle::Tiny,
                p.muted,
            );
            lay.push(
                Rect::new(card.left, cy - 2.0, card.right, cy + 30.0),
                SideHit::Open(slot.key.clone()),
            );
        }
        None if current.is_none() => {
            list.text(
                Rect::new(inner_l, cy, inner_r, cy + 16.0),
                "今天接下来是空的",
                TextStyle::Tiny,
                p.muted,
            );
        }
        None => {}
    }
    y = card.bottom + 14.0;

    let groups: [(&str, &[View]); 3] = [
        ("日程", &[View::Day, View::Month, View::Confirm]),
        (
            "事项",
            &[View::Todos, View::Goals, View::Routines, View::Projects],
        ),
        ("", &[View::Trash, View::History]),
    ];
    for (title, views) in groups {
        if title.is_empty() {
            list.hline(left + 4.0, right - 4.0, y + 4.0, p.border);
            y += 10.0;
        } else {
            list.text(
                Rect::new(left + 6.0, y, right, y + 18.0),
                title.to_owned(),
                TextStyle::Caption,
                p.muted,
            );
            y += 22.0;
        }
        for view in views {
            let r = Rect::new(left, y, right, y + 32.0);
            let active = st.view == *view && st.query.is_empty();
            if active {
                list.rounded_rect(r, 7.0, p.surface_muted);
            }
            let fg = if active {
                p.foreground
            } else {
                theme::mix(p.foreground, p.muted, 0.35)
            };
            list.icon_centered(
                Rect::new(r.left + 6.0, r.top, r.left + 28.0, r.bottom),
                view.icon(),
                15.0,
                if active { p.accent } else { p.muted },
            );
            list.text_aligned(
                Rect::new(r.left + 34.0, r.top, r.right - 44.0, r.bottom),
                view.label(),
                TextStyle::Small,
                fg,
                Align::Leading,
            );
            if let Some((n, alert)) = count(data, *view, st.now) {
                let label = n.to_string();
                let w = text::measure(&label, TextStyle::Tiny) + 12.0;
                let br = Rect::new(
                    r.right - 8.0 - w,
                    r.top + 7.0,
                    r.right - 8.0,
                    r.bottom - 7.0,
                );
                if alert {
                    let color = if *view == View::Todos {
                        c.danger
                    } else {
                        c.warning
                    };
                    list.rounded_rect(br, 9.0, color);
                    list.text_aligned(br, label, TextStyle::Tiny, 0xFFFFFF, Align::Center);
                } else {
                    list.text_aligned(br, label, TextStyle::Tiny, p.muted, Align::Center);
                }
            }
            lay.push(r, SideHit::View(*view));
            y += 34.0;
        }
        y += 6.0;
    }

    // 迷你月历：有安排的日子加一个点，点日期直接跳到那天的时间轴。
    let cal_h = 26.0 + 22.0 + 6.0 * 26.0;
    if area.bottom - y > cal_h + 40.0 {
        y += 4.0;
        let month = st.month;
        list.text_aligned(
            Rect::new(left + 6.0, y, right - 60.0, y + 22.0),
            format!("{}年{}月", month.year(), month.month()),
            TextStyle::Small,
            p.foreground,
            Align::Leading,
        );
        let prev = Rect::new(right - 52.0, y, right - 28.0, y + 22.0);
        let next = Rect::new(right - 24.0, y, right, y + 22.0);
        list.icon_centered(prev, Icon::CHEVRON_LEFT, 12.0, p.muted);
        list.icon_centered(next, Icon::CHEVRON_RIGHT, 12.0, p.muted);
        lay.push(prev, SideHit::MonthPrev);
        lay.push(next, SideHit::MonthNext);
        y += 26.0;
        let col = (right - left) / 7.0;
        for (i, name) in ["一", "二", "三", "四", "五", "六", "日"]
            .iter()
            .enumerate()
        {
            let r = Rect::new(
                left + i as f32 * col,
                y,
                left + (i + 1) as f32 * col,
                y + 20.0,
            );
            list.text_aligned(r, *name, TextStyle::Tiny, p.muted, Align::Center);
        }
        y += 22.0;
        let first = at::week_start(at::month_start(month));
        let summaries = query::range_summary(data, first, at::add_days(first, 41), st.now);
        for (index, sum) in summaries.iter().enumerate().take(42) {
            let date = at::add_days(first, index as i64);
            let row = (index / 7) as f32;
            let cl = (index % 7) as f32;
            let cell = Rect::new(
                left + cl * col,
                y + row * 26.0,
                left + (cl + 1.0) * col,
                y + (row + 1.0) * 26.0,
            );
            let dot = Rect::new(
                cell.left + (col - 22.0) / 2.0,
                cell.top + 1.0,
                cell.left + (col + 22.0) / 2.0,
                cell.top + 23.0,
            );
            let today = date == st.today();
            let chosen = date == st.date && st.view == View::Day;
            if today {
                list.rounded_rect(dot, 11.0, p.accent);
            } else if chosen {
                list.rounded_rect(dot, 11.0, p.surface_muted);
            }
            let fg = if today {
                p.accent_foreground
            } else if date.month() == month.month() {
                p.foreground
            } else {
                theme::mix(p.muted, p.background, 0.4)
            };
            list.text_aligned(
                Rect::new(dot.left, dot.top, dot.right, dot.bottom - 3.0),
                date.day().to_string(),
                TextStyle::Tiny,
                fg,
                Align::Center,
            );
            let mark = if sum.needs_confirm > 0 {
                Some(c.warning)
            } else if sum.overdue_tasks > 0 {
                Some(c.danger)
            } else if sum.slots > 0 || sum.due_tasks > 0 {
                Some(if today { p.accent_foreground } else { p.muted })
            } else {
                None
            };
            if let Some(color) = mark {
                let cx = (dot.left + dot.right) / 2.0;
                list.rounded_rect(
                    Rect::new(cx - 1.5, dot.bottom - 5.0, cx + 1.5, dot.bottom - 2.0),
                    1.5,
                    color,
                );
            }
            lay.push(cell, SideHit::Day(date));
        }
    }
    lay
}
