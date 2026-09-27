//! 只读查询与计算展示信息。「逾期」「已排期」「时间已过」「待确认」都在这里实时算出，
//! 从不写回生命周期状态。

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime};
use serde::Serialize;

use super::model::*;
use super::ops::occurrence_span;
use super::recur;
use super::time;

/// 任务的计算信息。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TaskFacts {
    /// 仍需执行且已过截止。
    pub overdue: bool,
    /// 还有未来的计划块。
    pub scheduled: bool,
    pub block_count: usize,
    /// 未来计划中的分钟数。
    pub planned_minutes: i64,
    /// 已执行的分钟数（按实际填写，否则按计划时长）。
    pub done_minutes: i64,
    /// 已过时间但还没确认的块数。
    pub unconfirmed: usize,
    pub next_block: Option<String>,
    pub last_done: Option<String>,
    pub steps_done: usize,
    pub steps_total: usize,
}

fn span(entry: &Entry) -> Option<(NaiveDateTime, NaiveDateTime)> {
    Some((
        time::parse_stamp(&entry.start)?,
        time::parse_stamp(&entry.end)?,
    ))
}

pub fn entry_minutes(entry: &Entry) -> i64 {
    if entry.all_day {
        return 0;
    }
    span(entry)
        .map(|(a, b)| time::minutes_between(a, b))
        .unwrap_or(0)
}

pub fn executed_minutes(entry: &Entry) -> i64 {
    if entry.status != EntryStatus::Done {
        return 0;
    }
    entry
        .actual_minutes
        .map(i64::from)
        .unwrap_or_else(|| entry_minutes(entry))
}

pub fn is_overdue(task: &Task, now: NaiveDateTime) -> bool {
    task.deleted_at.is_none()
        && task.status.is_open()
        && task
            .due
            .as_deref()
            .and_then(time::Due::parse)
            .is_some_and(|d| d.deadline() <= now)
}

pub fn task_facts(data: &AgendaData, task: &Task, now: NaiveDateTime) -> TaskFacts {
    let mut facts = TaskFacts {
        overdue: is_overdue(task, now),
        steps_total: task.steps.len(),
        steps_done: task.steps.iter().filter(|s| s.done).count(),
        ..Default::default()
    };
    let mut next: Option<NaiveDateTime> = None;
    let mut last: Option<NaiveDateTime> = None;
    for entry in data
        .entries
        .iter()
        .filter(|e| e.task_id.as_deref() == Some(task.id.as_str()))
    {
        if entry.status == EntryStatus::Cancelled {
            continue;
        }
        facts.block_count += 1;
        let Some((start, end)) = span(entry) else {
            continue;
        };
        match entry.status {
            EntryStatus::Planned if end > now => {
                facts.planned_minutes += time::minutes_between(start.max(now), end);
                if start > now || end > now {
                    facts.scheduled = true;
                    if next.is_none_or(|n| start < n) {
                        next = Some(start);
                    }
                }
            }
            EntryStatus::Planned => facts.unconfirmed += 1,
            EntryStatus::Done => {
                facts.done_minutes += executed_minutes(entry);
                if last.is_none_or(|l| end > l) {
                    last = Some(end);
                }
            }
            _ => {}
        }
    }
    facts.next_block = next.map(time::stamp);
    facts.last_done = last.map(time::stamp);
    facts
}

/// 任务的全部时间块（含已取消），按开始时间排序。
pub fn task_blocks<'a>(data: &'a AgendaData, task_id: &str) -> Vec<&'a Entry> {
    let mut blocks: Vec<&Entry> = data
        .entries
        .iter()
        .filter(|e| e.task_id.as_deref() == Some(task_id))
        .collect();
    blocks.sort_by(|a, b| a.start.cmp(&b.start));
    blocks
}

/// 正在进行的日程与今天接下来的第一个日程（都不含全天和已取消）。
pub fn now_and_next(data: &AgendaData, now: NaiveDateTime) -> (Option<Slot>, Option<Slot>) {
    let slots: Vec<Slot> = day_slots(data, now.date(), now)
        .into_iter()
        .filter(|s| !s.all_day && s.status != EntryStatus::Cancelled)
        .collect();
    let current = slots.iter().find(|s| s.is_now(now)).cloned();
    let next = slots.into_iter().find(|s| s.start > now);
    (current, next)
}

/// 一条「为了什么」链上的节点。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChainNode {
    pub kind: Kind,
    pub id: String,
    pub title: String,
}

/// 从某条记录往上追溯：时间块 → 任务 → 目标 → 愿望。返回自顶向下的顺序（不含自身）。
pub fn why_chain(data: &AgendaData, id: &str) -> Vec<ChainNode> {
    let mut chain = Vec::new();
    let mut goal_id: Option<String> = None;
    match Kind::of_id(id) {
        Some(Kind::Entry) => {
            if let Some(entry) = data.entry(id) {
                if let Some(task) = entry.task_id.as_deref().and_then(|t| data.live_task(t)) {
                    chain.push(ChainNode {
                        kind: Kind::Task,
                        id: task.id.clone(),
                        title: task.title.clone(),
                    });
                    goal_id = task.goal_id.clone();
                } else if let Some(routine) = entry
                    .routine_id
                    .as_deref()
                    .and_then(|r| data.live_routine(r))
                {
                    goal_id = routine.goal_id.clone();
                }
            }
        }
        Some(Kind::Task) => goal_id = data.task(id).and_then(|t| t.goal_id.clone()),
        Some(Kind::Routine) => goal_id = data.routine(id).and_then(|r| r.goal_id.clone()),
        Some(Kind::Goal) => {
            if let Some(wish) = data
                .goal(id)
                .and_then(|g| g.wish_id.as_deref())
                .and_then(|w| data.live_wish(w))
            {
                return vec![ChainNode {
                    kind: Kind::Wish,
                    id: wish.id.clone(),
                    title: wish.title.clone(),
                }];
            }
            return Vec::new();
        }
        _ => return Vec::new(),
    }
    if let Some(goal) = goal_id.as_deref().and_then(|g| data.live_goal(g)) {
        chain.insert(
            0,
            ChainNode {
                kind: Kind::Goal,
                id: goal.id.clone(),
                title: goal.title.clone(),
            },
        );
        if let Some(wish) = goal.wish_id.as_deref().and_then(|w| data.live_wish(w)) {
            chain.insert(
                0,
                ChainNode {
                    kind: Kind::Wish,
                    id: wish.id.clone(),
                    title: wish.title.clone(),
                },
            );
        }
    }
    chain
}

/// 时间轴上的一项：真实日程，或重复安排尚未落地的虚拟发生。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Slot {
    /// 真实日程是日程 ID；虚拟发生是 `rtn-…@YYYY-MM-DD`。
    pub key: String,
    pub entry_id: Option<String>,
    pub task_id: Option<String>,
    pub routine_id: Option<String>,
    pub title: String,
    pub note: String,
    pub location: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub status: EntryStatus,
    pub color: Option<String>,
    pub project_id: Option<String>,
    /// 结束时间已过（计算值）。
    pub time_passed: bool,
    /// 时间已过且仍在计划中：等待用户确认是否执行。
    pub needs_confirm: bool,
    pub task_status: Option<TaskStatus>,
    pub task_deleted: bool,
    pub is_virtual: bool,
}

impl Slot {
    pub fn minutes(&self) -> i64 {
        time::minutes_between(self.start, self.end)
    }

    pub fn is_now(&self, now: NaiveDateTime) -> bool {
        self.start <= now && now < self.end
    }
}

fn slot_of(data: &AgendaData, entry: &Entry, now: NaiveDateTime) -> Option<Slot> {
    let (start, end) = span(entry)?;
    let task = entry.task_id.as_deref().and_then(|t| data.task(t));
    let time_passed = end <= now;
    Some(Slot {
        key: entry.id.clone(),
        entry_id: Some(entry.id.clone()),
        task_id: entry.task_id.clone(),
        routine_id: entry.routine_id.clone(),
        title: data.entry_title(entry),
        note: entry.note.clone(),
        location: entry.location.clone(),
        start,
        end,
        all_day: entry.all_day,
        status: entry.status,
        color: entry.color.clone(),
        project_id: entry
            .project_id
            .clone()
            .or_else(|| task.and_then(|t| t.project_id.clone())),
        time_passed,
        needs_confirm: time_passed && entry.status == EntryStatus::Planned,
        task_status: task.map(|t| t.status),
        task_deleted: data.entry_task_deleted(entry),
        is_virtual: false,
    })
}

/// 重复安排在 `[from, to]` 内未落地的虚拟发生。
fn virtual_slots(
    data: &AgendaData,
    from: NaiveDate,
    to: NaiveDate,
    now: NaiveDateTime,
) -> Vec<Slot> {
    let mut out = Vec::new();
    for routine in &data.routines {
        if routine.deleted_at.is_some()
            || routine.archived_at.is_some()
            || routine.status != RoutineStatus::Active
        {
            continue;
        }
        let (Some(rule), Some(anchor)) = (
            recur::parse(&routine.rule),
            time::parse_date(&routine.start_date),
        ) else {
            continue;
        };
        let until = routine.until.as_deref().and_then(time::parse_date);
        for date in recur::dates_between(&rule, anchor, from, to) {
            if until.is_some_and(|u| date > u) {
                break;
            }
            let key = time::date_key(date);
            let materialized = data.entries.iter().any(|e| {
                e.routine_id.as_deref() == Some(routine.id.as_str())
                    && e.occurrence.as_deref() == Some(key.as_str())
            });
            if materialized {
                continue;
            }
            let (start, end, all_day) = occurrence_span(routine, date);
            let time_passed = end <= now;
            out.push(Slot {
                key: format!("{}@{key}", routine.id),
                entry_id: None,
                task_id: None,
                routine_id: Some(routine.id.clone()),
                title: routine.title.clone(),
                note: routine.note.clone(),
                location: String::new(),
                start,
                end,
                all_day,
                status: EntryStatus::Planned,
                color: routine.color.clone(),
                project_id: routine.project_id.clone(),
                time_passed,
                needs_confirm: time_passed,
                task_status: None,
                task_deleted: false,
                is_virtual: true,
            });
        }
    }
    out
}

/// `[from, to]` 闭区间内与之相交的所有日程（含虚拟发生），按开始时间排序。
pub fn slots_between(
    data: &AgendaData,
    from: NaiveDate,
    to: NaiveDate,
    now: NaiveDateTime,
) -> Vec<Slot> {
    let range_start = from.and_time(chrono::NaiveTime::MIN);
    let range_end = time::add_days(to, 1).and_time(chrono::NaiveTime::MIN);
    let mut out: Vec<Slot> = data
        .entries
        .iter()
        .filter_map(|e| slot_of(data, e, now))
        .filter(|s| s.start < range_end && s.end > range_start)
        .collect();
    out.extend(virtual_slots(data, from, to, now));
    out.sort_by(|a, b| {
        b.all_day
            .cmp(&a.all_day)
            .then(a.start.cmp(&b.start))
            .then(a.end.cmp(&b.end))
            .then(a.key.cmp(&b.key))
    });
    out
}

pub fn day_slots(data: &AgendaData, date: NaiveDate, now: NaiveDateTime) -> Vec<Slot> {
    slots_between(data, date, date, now)
}

pub fn slot_by_key(data: &AgendaData, key: &str, now: NaiveDateTime) -> Option<Slot> {
    if let Some((routine, date)) = key.split_once('@') {
        let date = time::parse_date(date)?;
        return virtual_slots(data, date, date, now)
            .into_iter()
            .find(|s| s.routine_id.as_deref() == Some(routine))
            .or_else(|| {
                let occurrence = time::date_key(date);
                data.entries
                    .iter()
                    .find(|e| {
                        e.routine_id.as_deref() == Some(routine)
                            && e.occurrence.as_deref() == Some(occurrence.as_str())
                    })
                    .and_then(|e| slot_of(data, e, now))
            });
    }
    data.entry(key).and_then(|e| slot_of(data, e, now))
}

/// 待确认队列：回看窗口内时间已过、仍在计划中的日程。
pub fn confirm_queue(data: &AgendaData, now: NaiveDateTime) -> Vec<Slot> {
    let from = time::add_days(
        now.date(),
        -(data.settings.confirm_lookback_days.max(1) as i64),
    );
    let mut out: Vec<Slot> = slots_between(data, from, now.date(), now)
        .into_iter()
        .filter(|s| s.needs_confirm && !s.task_deleted)
        .collect();
    out.sort_by(|a, b| b.end.cmp(&a.end));
    out
}

pub fn is_visible_task(task: &Task) -> bool {
    task.deleted_at.is_none() && task.archived_at.is_none()
}

/// 某一天右侧栏的待办分组。同一个任务只出现在最靠前的那组。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DayTodos {
    pub overdue: Vec<String>,
    pub due: Vec<String>,
    pub planned: Vec<String>,
    pub scheduled: Vec<String>,
    pub doing: Vec<String>,
    /// 当天完成的任务。
    pub done: Vec<String>,
}

impl DayTodos {
    pub fn is_empty(&self) -> bool {
        self.overdue.is_empty()
            && self.due.is_empty()
            && self.planned.is_empty()
            && self.scheduled.is_empty()
            && self.doing.is_empty()
            && self.done.is_empty()
    }
}

pub fn day_todos(data: &AgendaData, date: NaiveDate, now: NaiveDateTime) -> DayTodos {
    let key = time::date_key(date);
    let mut out = DayTodos::default();
    let is_today = date == now.date();
    let scheduled_today: Vec<&str> = data
        .entries
        .iter()
        .filter(|e| e.status != EntryStatus::Cancelled && e.start.starts_with(&key))
        .filter_map(|e| e.task_id.as_deref())
        .collect();
    let mut tasks: Vec<&Task> = data.tasks.iter().filter(|t| is_visible_task(t)).collect();
    sort_tasks(&mut tasks, now);
    for task in tasks {
        let due_date = task
            .due
            .as_deref()
            .and_then(time::Due::parse)
            .map(|d| d.date());
        if !task.status.is_open() {
            let done_today = task.status == TaskStatus::Done
                && task
                    .completed_at
                    .as_deref()
                    .and_then(time::parse_stamp)
                    .is_some_and(|t| t.date() == date);
            if done_today {
                out.done.push(task.id.clone());
            }
            continue;
        }
        let id = task.id.clone();
        if is_today && is_overdue(task, now) && due_date.is_some_and(|d| d < date) {
            out.overdue.push(id);
        } else if due_date == Some(date) {
            out.due.push(id);
        } else if task.planned_for.as_deref() == Some(key.as_str()) {
            out.planned.push(id);
        } else if scheduled_today.contains(&task.id.as_str()) {
            out.scheduled.push(id);
        } else if is_today && task.status == TaskStatus::Doing {
            out.doing.push(id);
        }
    }
    out
}

/// 待办排序：逾期 → 截止近 → 优先级高 → 新建早。
pub fn sort_tasks(tasks: &mut [&Task], now: NaiveDateTime) {
    tasks.sort_by(|a, b| {
        let due = |t: &Task| {
            t.due
                .as_deref()
                .and_then(time::Due::parse)
                .map(|d| d.deadline())
        };
        is_overdue(b, now)
            .cmp(&is_overdue(a, now))
            .then_with(|| match (due(a), due(b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then(b.priority.rank().cmp(&a.priority.rank()))
            .then(a.created_at.cmp(&b.created_at))
    });
}

/// 月历一格的摘要。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DaySummary {
    pub date: String,
    pub slots: usize,
    pub all_day: usize,
    /// 当天被安排掉的分钟数（不含全天和已取消）。
    pub busy_minutes: i64,
    pub done: usize,
    pub needs_confirm: usize,
    pub due_tasks: usize,
    pub overdue_tasks: usize,
    /// 前几条标题，供格子里展示。
    pub titles: Vec<(String, EntryStatus, Option<String>)>,
}

pub fn range_summary(
    data: &AgendaData,
    from: NaiveDate,
    to: NaiveDate,
    now: NaiveDateTime,
) -> Vec<DaySummary> {
    let mut by_day: HashMap<NaiveDate, DaySummary> = HashMap::new();
    let mut day = from;
    while day <= to {
        by_day.insert(
            day,
            DaySummary {
                date: time::date_key(day),
                ..Default::default()
            },
        );
        day = time::add_days(day, 1);
    }
    for slot in slots_between(data, from, to, now) {
        if slot.status == EntryStatus::Cancelled {
            continue;
        }
        let mut day = slot.start.date().max(from);
        let last = if slot.end.time() == chrono::NaiveTime::MIN {
            time::add_days(slot.end.date(), -1)
        } else {
            slot.end.date()
        };
        while day <= last.min(to) {
            if let Some(summary) = by_day.get_mut(&day) {
                summary.slots += 1;
                if slot.all_day {
                    summary.all_day += 1;
                } else {
                    let day_start = day.and_time(chrono::NaiveTime::MIN);
                    let day_end = time::add_days(day, 1).and_time(chrono::NaiveTime::MIN);
                    summary.busy_minutes +=
                        time::minutes_between(slot.start.max(day_start), slot.end.min(day_end));
                }
                if slot.status == EntryStatus::Done {
                    summary.done += 1;
                }
                if slot.needs_confirm {
                    summary.needs_confirm += 1;
                }
                if summary.titles.len() < 6 {
                    summary
                        .titles
                        .push((slot.title.clone(), slot.status, slot.color.clone()));
                }
            }
            day = time::add_days(day, 1);
        }
    }
    for task in data
        .tasks
        .iter()
        .filter(|t| is_visible_task(t) && t.status.is_open())
    {
        let Some(due) = task.due.as_deref().and_then(time::Due::parse) else {
            continue;
        };
        if let Some(summary) = by_day.get_mut(&due.date()) {
            summary.due_tasks += 1;
            if is_overdue(task, now) {
                summary.overdue_tasks += 1;
            }
        }
    }
    let mut out: Vec<DaySummary> = by_day.into_values().collect();
    out.sort_by(|a, b| a.date.cmp(&b.date));
    out
}

/// 目标的计算进度。只用于展示，绝不据此改目标状态。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GoalProgress {
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub tasks_open: usize,
    pub tasks_overdue: usize,
    pub criteria_total: usize,
    pub criteria_done: usize,
    pub routines: usize,
    /// 累计投入（已执行的时间块 + 重复安排）。
    pub invested_minutes: i64,
    /// 未来已排的分钟。
    pub planned_minutes: i64,
}

impl GoalProgress {
    /// 0..=1，优先按达成标准，其次按任务。
    pub fn ratio(&self) -> f32 {
        if self.criteria_total > 0 {
            self.criteria_done as f32 / self.criteria_total as f32
        } else if self.tasks_total > 0 {
            self.tasks_done as f32 / self.tasks_total as f32
        } else {
            0.0
        }
    }
}

pub fn goal_progress(data: &AgendaData, goal_id: &str, now: NaiveDateTime) -> GoalProgress {
    let mut p = GoalProgress::default();
    if let Some(goal) = data.goal(goal_id) {
        p.criteria_total = goal.criteria.len();
        p.criteria_done = goal.criteria.iter().filter(|c| c.done).count();
    }
    for task in data
        .tasks
        .iter()
        .filter(|t| t.deleted_at.is_none() && t.goal_id.as_deref() == Some(goal_id))
    {
        if task.status == TaskStatus::Cancelled {
            continue;
        }
        p.tasks_total += 1;
        match task.status {
            TaskStatus::Done => p.tasks_done += 1,
            _ => p.tasks_open += 1,
        }
        if is_overdue(task, now) {
            p.tasks_overdue += 1;
        }
        let facts = task_facts(data, task, now);
        p.invested_minutes += facts.done_minutes;
        p.planned_minutes += facts.planned_minutes;
    }
    for routine in data
        .routines
        .iter()
        .filter(|r| r.deleted_at.is_none() && r.goal_id.as_deref() == Some(goal_id))
    {
        p.routines += 1;
        p.invested_minutes += data
            .entries
            .iter()
            .filter(|e| e.routine_id.as_deref() == Some(routine.id.as_str()))
            .map(|e| {
                if e.all_day && e.status == EntryStatus::Done {
                    e.actual_minutes
                        .map(i64::from)
                        .unwrap_or(routine.duration_minutes as i64)
                } else {
                    executed_minutes(e)
                }
            })
            .sum::<i64>();
    }
    p
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct WishProgress {
    pub goals_total: usize,
    pub goals_achieved: usize,
    pub goals_active: usize,
    pub invested_minutes: i64,
}

pub fn wish_progress(data: &AgendaData, wish_id: &str, now: NaiveDateTime) -> WishProgress {
    let mut p = WishProgress::default();
    for goal in data
        .goals
        .iter()
        .filter(|g| g.deleted_at.is_none() && g.wish_id.as_deref() == Some(wish_id))
    {
        p.goals_total += 1;
        match goal.status {
            GoalStatus::Achieved => p.goals_achieved += 1,
            GoalStatus::Active => p.goals_active += 1,
            _ => {}
        }
        p.invested_minutes += goal_progress(data, &goal.id, now).invested_minutes;
    }
    p
}

/// 重复安排的执行记录。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RoutineStats {
    /// 截至昨天（今天已完成则含今天）连续完成的次数。
    pub streak: usize,
    /// 最近 30 天应发生次数与完成次数。
    pub expected_30: usize,
    pub done_30: usize,
    /// 最近 14 天每次发生的状态（旧 → 新），用于小热力条。
    pub recent: Vec<(String, Option<EntryStatus>)>,
}

pub fn routine_stats(data: &AgendaData, routine: &Routine, today: NaiveDate) -> RoutineStats {
    let mut stats = RoutineStats::default();
    let (Some(rule), Some(anchor)) = (
        recur::parse(&routine.rule),
        time::parse_date(&routine.start_date),
    ) else {
        return stats;
    };
    let status_on = |date: NaiveDate| {
        let key = time::date_key(date);
        data.entries
            .iter()
            .find(|e| {
                e.routine_id.as_deref() == Some(routine.id.as_str())
                    && e.occurrence.as_deref() == Some(key.as_str())
            })
            .map(|e| e.status)
    };
    let dates = recur::dates_between(&rule, anchor, time::add_days(today, -60), today);
    for date in dates.iter().rev() {
        match status_on(*date) {
            Some(EntryStatus::Done) => stats.streak += 1,
            _ if *date == today => {}
            _ => break,
        }
    }
    for date in dates.iter().filter(|d| **d > time::add_days(today, -30)) {
        stats.expected_30 += 1;
        if status_on(*date) == Some(EntryStatus::Done) {
            stats.done_30 += 1;
        }
    }
    stats.recent = dates
        .iter()
        .filter(|d| **d > time::add_days(today, -14))
        .map(|d| (time::date_key(*d), status_on(*d)))
        .collect();
    stats
}

/// 截止时间河流：按截止日分组的未完成任务，外加一组「没有截止」。
pub fn deadline_river(
    data: &AgendaData,
    now: NaiveDateTime,
) -> (Vec<(NaiveDate, Vec<String>)>, Vec<String>) {
    let mut dated: HashMap<NaiveDate, Vec<&Task>> = HashMap::new();
    let mut undated: Vec<&Task> = Vec::new();
    for task in data
        .tasks
        .iter()
        .filter(|t| is_visible_task(t) && t.status.is_open())
    {
        match task.due.as_deref().and_then(time::Due::parse) {
            Some(due) => dated.entry(due.date()).or_default().push(task),
            None => undated.push(task),
        }
    }
    let mut groups: Vec<(NaiveDate, Vec<String>)> = dated
        .into_iter()
        .map(|(date, mut tasks)| {
            sort_tasks(&mut tasks, now);
            (date, tasks.into_iter().map(|t| t.id.clone()).collect())
        })
        .collect();
    groups.sort_by_key(|(d, _)| *d);
    sort_tasks(&mut undated, now);
    (groups, undated.into_iter().map(|t| t.id.clone()).collect())
}

/// 搜索标题 / 备注 / 标签。
pub fn search(data: &AgendaData, query: &str, include_hidden: bool) -> Vec<(Kind, String, String)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let hit = |title: &str, note: &str, tags: &[String]| {
        title.to_lowercase().contains(&q)
            || note.to_lowercase().contains(&q)
            || tags.iter().any(|t| t.to_lowercase().contains(&q))
    };
    let mut out = Vec::new();
    for w in &data.wishes {
        if (include_hidden || (w.deleted_at.is_none() && w.archived_at.is_none()))
            && hit(&w.title, &w.note, &w.tags)
        {
            out.push((Kind::Wish, w.id.clone(), w.title.clone()));
        }
    }
    for g in &data.goals {
        if (include_hidden || (g.deleted_at.is_none() && g.archived_at.is_none()))
            && hit(&g.title, &g.note, &g.tags)
        {
            out.push((Kind::Goal, g.id.clone(), g.title.clone()));
        }
    }
    for t in &data.tasks {
        if (include_hidden || is_visible_task(t)) && hit(&t.title, &t.note, &t.tags) {
            out.push((Kind::Task, t.id.clone(), t.title.clone()));
        }
    }
    for r in &data.routines {
        if (include_hidden || r.deleted_at.is_none()) && hit(&r.title, &r.note, &[]) {
            out.push((Kind::Routine, r.id.clone(), r.title.clone()));
        }
    }
    for e in data.entries.iter().filter(|e| e.task_id.is_none()) {
        if hit(&e.title, &e.note, &[]) {
            out.push((Kind::Entry, e.id.clone(), data.entry_title(e)));
        }
    }
    for p in &data.projects {
        if (include_hidden || p.deleted_at.is_none()) && hit(&p.name, &p.note, &[]) {
            out.push((Kind::Project, p.id.clone(), p.name.clone()));
        }
    }
    out
}

/// 回收站里的记录（种类、ID、标题、删除时间）。
pub fn trash(data: &AgendaData) -> Vec<(Kind, String, String, String)> {
    let mut out = Vec::new();
    for w in &data.wishes {
        if let Some(at) = &w.deleted_at {
            out.push((Kind::Wish, w.id.clone(), w.title.clone(), at.clone()));
        }
    }
    for g in &data.goals {
        if let Some(at) = &g.deleted_at {
            out.push((Kind::Goal, g.id.clone(), g.title.clone(), at.clone()));
        }
    }
    for t in &data.tasks {
        if let Some(at) = &t.deleted_at {
            out.push((Kind::Task, t.id.clone(), t.title.clone(), at.clone()));
        }
    }
    for r in &data.routines {
        if let Some(at) = &r.deleted_at {
            out.push((Kind::Routine, r.id.clone(), r.title.clone(), at.clone()));
        }
    }
    for p in &data.projects {
        if let Some(at) = &p.deleted_at {
            out.push((Kind::Project, p.id.clone(), p.name.clone(), at.clone()));
        }
    }
    out.sort_by(|a, b| b.3.cmp(&a.3));
    out
}

/// 提醒：`(from, to]` 内应该发出的通知。
#[derive(Debug, Clone, PartialEq)]
pub enum Nudge {
    /// 日程即将开始。
    Starting {
        key: String,
        title: String,
        start: NaiveDateTime,
        minutes_before: u32,
    },
    /// 日程已结束，请确认是否执行。
    Confirm {
        key: String,
        title: String,
        end: NaiveDateTime,
    },
    /// 任务到截止时间。
    Due { task_id: String, title: String },
}

impl Nudge {
    pub fn key(&self) -> String {
        match self {
            Self::Starting {
                key,
                minutes_before,
                ..
            } => format!("start:{key}:{minutes_before}"),
            Self::Confirm { key, .. } => format!("confirm:{key}"),
            Self::Due { task_id, .. } => format!("due:{task_id}"),
        }
    }
}

pub fn nudges(data: &AgendaData, from: NaiveDateTime, to: NaiveDateTime) -> Vec<Nudge> {
    let mut out = Vec::new();
    let confirm_delay = chrono::Duration::minutes(data.settings.confirm_after_minutes as i64);
    let routine_reminders = |slot: &Slot| -> Vec<u32> {
        match (&slot.entry_id, &slot.routine_id) {
            (Some(id), _) => data
                .entry(id)
                .map(|e| e.reminders.clone())
                .unwrap_or_default(),
            (None, Some(r)) => data
                .routine(r)
                .map(|r| r.reminders.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    };
    for slot in slots_between(
        data,
        time::add_days(from.date(), -1),
        time::add_days(to.date(), 1),
        to,
    ) {
        if slot.status != EntryStatus::Planned || slot.task_deleted {
            continue;
        }
        if !slot.all_day {
            for minutes in routine_reminders(&slot) {
                let at = slot.start - chrono::Duration::minutes(minutes as i64);
                if at > from && at <= to {
                    out.push(Nudge::Starting {
                        key: slot.key.clone(),
                        title: slot.title.clone(),
                        start: slot.start,
                        minutes_before: minutes,
                    });
                }
            }
        }
        // 重复安排和任务块结束后都请用户确认；独立日程通常是会议之类，不打扰。
        if slot.routine_id.is_some() || slot.task_id.is_some() {
            let at = slot.end + confirm_delay;
            if at > from && at <= to {
                out.push(Nudge::Confirm {
                    key: slot.key.clone(),
                    title: slot.title.clone(),
                    end: slot.end,
                });
            }
        }
    }
    for task in data
        .tasks
        .iter()
        .filter(|t| is_visible_task(t) && t.status.is_open())
    {
        if let Some(time::Due::At(at)) = task.due.as_deref().and_then(time::Due::parse) {
            if at > from && at <= to {
                out.push(Nudge::Due {
                    task_id: task.id.clone(),
                    title: task.title.clone(),
                });
            }
        }
    }
    out
}

/// 按时间重叠给日程分泳道：返回每项的 (泳道, 该重叠簇的泳道数)。
pub fn assign_lanes(spans: &[(NaiveDateTime, NaiveDateTime)]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|a, b| {
        spans[*a]
            .0
            .cmp(&spans[*b].0)
            .then(spans[*b].1.cmp(&spans[*a].1))
    });
    let mut result = vec![(0usize, 1usize); spans.len()];
    let mut cluster: Vec<usize> = Vec::new();
    let mut lane_ends: Vec<NaiveDateTime> = Vec::new();
    let mut cluster_end: Option<NaiveDateTime> = None;
    let flush = |cluster: &mut Vec<usize>, lanes: usize, result: &mut Vec<(usize, usize)>| {
        for index in cluster.drain(..) {
            result[index].1 = lanes.max(1);
        }
    };
    for index in order {
        let (start, end) = spans[index];
        if cluster_end.is_some_and(|e| start >= e) {
            flush(&mut cluster, lane_ends.len(), &mut result);
            lane_ends.clear();
            cluster_end = None;
        }
        let lane = match lane_ends.iter().position(|e| *e <= start) {
            Some(lane) => {
                lane_ends[lane] = end;
                lane
            }
            None => {
                lane_ends.push(end);
                lane_ends.len() - 1
            }
        };
        result[index].0 = lane;
        cluster.push(index);
        cluster_end = Some(cluster_end.map_or(end, |e| e.max(end)));
    }
    flush(&mut cluster, lane_ends.len(), &mut result);
    result
}
