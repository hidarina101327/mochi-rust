//! 排程助手：找空闲时段、把待办自动排进某一天。结果只是提案，由调用方决定是否应用。

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::Serialize;

use super::model::*;
use super::query::{self, is_overdue, sort_tasks};
use super::time;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FreeSlot {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

impl FreeSlot {
    pub fn minutes(&self) -> i64 {
        time::minutes_between(self.start, self.end)
    }
}

fn work_window(data: &AgendaData, date: NaiveDate) -> (NaiveDateTime, NaiveDateTime) {
    let start = time::parse_hm(&data.settings.work_start)
        .unwrap_or(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
    let end = time::parse_hm(&data.settings.work_end)
        .unwrap_or(NaiveTime::from_hms_opt(22, 0, 0).unwrap());
    (date.and_time(start), date.and_time(end))
}

/// 某天 `[from, to)` 内没被占用的时段（不早于现在，至少 `min_minutes`）。
pub fn free_slots(
    data: &AgendaData,
    date: NaiveDate,
    now: NaiveDateTime,
    window: Option<(NaiveDateTime, NaiveDateTime)>,
    min_minutes: i64,
) -> Vec<FreeSlot> {
    let (mut from, to) = window.unwrap_or_else(|| work_window(data, date));
    if from < now {
        // 从现在起，并对齐到下一个 5 分钟。
        from = now;
        let rem = time::minute_of_day(from) % 5;
        if rem != 0 {
            from += chrono::Duration::minutes((5 - rem) as i64);
        }
    }
    if from >= to {
        return Vec::new();
    }
    let mut busy: Vec<(NaiveDateTime, NaiveDateTime)> = query::day_slots(data, date, now)
        .into_iter()
        .filter(|s| !s.all_day && matches!(s.status, EntryStatus::Planned | EntryStatus::Done))
        .map(|s| (s.start, s.end))
        .collect();
    busy.sort();
    let mut out = Vec::new();
    let mut cursor = from;
    for (start, end) in busy {
        if end <= cursor {
            continue;
        }
        if start > cursor {
            let slot_end = start.min(to);
            if time::minutes_between(cursor, slot_end) >= min_minutes {
                out.push(FreeSlot {
                    start: cursor,
                    end: slot_end,
                });
            }
        }
        cursor = cursor.max(end);
        if cursor >= to {
            break;
        }
    }
    if time::minutes_between(cursor, to) >= min_minutes {
        out.push(FreeSlot {
            start: cursor,
            end: to,
        });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Placement {
    pub task_id: String,
    pub title: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub reason: String,
}

const MAX_BLOCK: i64 = 90;
const MIN_BLOCK: i64 = 25;
const BREAK: i64 = 10;

/// 该排进这一天的候选任务：逾期、当天 / 三天内截止、计划当天、进行中；
/// 已经在这一天有计划块的跳过。可以用 `only` 限定范围。
pub fn candidates(
    data: &AgendaData,
    date: NaiveDate,
    now: NaiveDateTime,
    only: Option<&[String]>,
) -> Vec<String> {
    let key = time::date_key(date);
    let horizon = time::add_days(date, 3);
    let mut tasks: Vec<&Task> = data
        .tasks
        .iter()
        .filter(|t| query::is_visible_task(t) && t.status.is_open())
        .filter(|t| match only {
            Some(ids) => ids.contains(&t.id),
            None => {
                let due = t
                    .due
                    .as_deref()
                    .and_then(time::Due::parse)
                    .map(|d| d.date());
                is_overdue(t, now)
                    || due.is_some_and(|d| d <= horizon)
                    || t.planned_for.as_deref() == Some(key.as_str())
                    || t.status == TaskStatus::Doing
            }
        })
        .filter(|t| {
            !data.entries.iter().any(|e| {
                e.task_id.as_deref() == Some(t.id.as_str())
                    && e.status == EntryStatus::Planned
                    && e.start.starts_with(&key)
            })
        })
        .collect();
    sort_tasks(&mut tasks, now);
    tasks.into_iter().map(|t| t.id.clone()).collect()
}

/// 贪心地把候选任务放进空闲时段。每个任务需要的时长 = 预计 − 已执行 − 未来已排，
/// 没有预计就用默认块长；长任务拆成不超过 90 分钟的块，块之间留 10 分钟。
pub fn auto_place(
    data: &AgendaData,
    date: NaiveDate,
    now: NaiveDateTime,
    only: Option<&[String]>,
) -> Vec<Placement> {
    let mut slots = free_slots(data, date, now, None, MIN_BLOCK);
    let mut out = Vec::new();
    for task_id in candidates(data, date, now, only) {
        let Some(task) = data.live_task(&task_id) else {
            continue;
        };
        let facts = query::task_facts(data, task, now);
        let mut need = match task.estimate_minutes {
            Some(estimate) => (estimate as i64 - facts.done_minutes - facts.planned_minutes).max(0),
            None => data.settings.default_block_minutes as i64,
        };
        if need == 0 {
            continue;
        }
        let reason = if is_overdue(task, now) {
            "已逾期".to_owned()
        } else if let Some(due) = &task.due {
            format!("截止 {}", time::due_label(due, now.date()))
        } else if task.planned_for.is_some() {
            "计划今天做".to_owned()
        } else {
            "进行中".to_owned()
        };
        while need > 0 {
            let Some(index) = slots
                .iter()
                .position(|s| s.minutes() >= MIN_BLOCK.min(need))
            else {
                break;
            };
            let slot = &mut slots[index];
            let length = need.min(MAX_BLOCK).min(slot.minutes());
            let start = slot.start;
            let end = start + chrono::Duration::minutes(length);
            out.push(Placement {
                task_id: task.id.clone(),
                title: task.title.clone(),
                start,
                end,
                reason: reason.clone(),
            });
            need -= length;
            slot.start = end + chrono::Duration::minutes(BREAK);
            if slot.start >= slot.end {
                slots.remove(index);
            }
        }
    }
    out
}

/// 一天的负荷：已安排分钟 / 可工作分钟。
pub fn day_load(data: &AgendaData, date: NaiveDate, now: NaiveDateTime) -> (i64, i64) {
    let (from, to) = work_window(data, date);
    let capacity = time::minutes_between(from, to).max(1);
    let busy: i64 = query::day_slots(data, date, now)
        .iter()
        .filter(|s| {
            !s.all_day && s.status != EntryStatus::Cancelled && s.status != EntryStatus::Skipped
        })
        .map(|s| time::minutes_between(s.start.max(from), s.end.min(to)).max(0))
        .sum();
    (busy, capacity)
}
