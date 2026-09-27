//! 日程待办的 AI 工具。
//!
//! 读工具直接返回数据和计算信息；写工具在内存里演算出变更，生成 `pendingScheduleDiff`
//! 提案，由用户批准后以严格模式应用（记录在此期间被改过就拒绝）。
//! 自动执行模式下直接应用，但含删除或超过 5 条记录变更的提案仍要人工确认。

use std::sync::Arc;

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::{json, Value};

use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::agenda::{
    batch, planner, proposal::MESSAGE_KEY, query, time, AgendaData, AgendaStore, ChangeSet, Clock,
    Editor, Kind, Proposal, Source,
};
use crate::ai::permission::AiToolAction;
use crate::app_settings::{self, AppSettings};

const TOOL_NAMES: &[&str] = &[
    "agenda_overview",
    "agenda_list",
    "agenda_get",
    "agenda_find_free_time",
    "agenda_history",
    "agenda_batch",
    "agenda_plan_day",
    "agenda_apply_change",
];

const READ_ONLY: &[&str] = &[
    "agenda_overview",
    "agenda_list",
    "agenda_get",
    "agenda_find_free_time",
    "agenda_history",
];

/// 自动执行模式下可直接应用的最大记录变更数。
const AUTO_APPLY_MAX_CHANGES: usize = 5;

pub struct AgendaToolExecutor {
    store: Arc<AgendaStore>,
    settings: Option<Arc<AppSettings>>,
    /// 「现在」由调用方注入，测试才能确定性地运行。
    now: fn() -> NaiveDateTime,
}

impl AgendaToolExecutor {
    pub fn new(store: Arc<AgendaStore>) -> Self {
        Self {
            store,
            settings: None,
            now: time::now,
        }
    }

    pub fn with_settings(mut self, settings: Arc<AppSettings>) -> Self {
        self.settings = Some(settings);
        self
    }

    #[cfg(test)]
    fn with_now(mut self, now: fn() -> NaiveDateTime) -> Self {
        self.now = now;
        self
    }

    fn load(&self) -> Result<AgendaData, String> {
        self.store
            .load()
            .map_err(|e| format!("读取日程待办失败: {e:#}"))
    }

    fn automatic(&self) -> bool {
        self.settings.as_ref().is_some_and(|settings| {
            app_settings::descriptor("ai.editApplyMode")
                .is_some_and(|descriptor| settings.read(descriptor).to_storage() == "auto")
        })
    }

    /// 在副本上演算，得到提案；自动模式下满足条件就直接应用。
    fn propose(
        &self,
        summary: &str,
        f: impl FnOnce(&mut Editor<'_>) -> anyhow::Result<Value>,
    ) -> ToolOutcome {
        let before = self.load()?;
        let mut after = before.clone();
        let detail = {
            let mut editor = Editor::new(
                &mut after,
                Clock {
                    now: (self.now)(),
                    utc: crate::jstime::now(),
                },
            );
            f(&mut editor).map_err(|e| format!("{e:#}"))?
        };
        let change = ChangeSet::diff(&before, &after);
        if change.is_empty() {
            return Ok(
                json!({ "unchanged": true, "detail": detail, "message": "没有产生任何变更" }),
            );
        }
        let mut proposal = Proposal::from_change(&change, &before, &after, summary);
        proposal.workspace_root = Some(self.store.workspace().to_string_lossy().into_owned());
        let automatic = self.automatic();
        let needs_review = proposal.has_delete() || change.len() > AUTO_APPLY_MAX_CHANGES;
        if automatic && !needs_review {
            self.store
                .apply(Source::Ai, &change, true)
                .map_err(|e| format!("{e:#}"))?;
            proposal.status = "applied".into();
        }
        let mut out = json!({
            MESSAGE_KEY: proposal.to_value(),
            "changes": proposal.operations.iter().map(|op| op.summary.clone()).collect::<Vec<_>>(),
            "detail": detail,
        });
        if proposal.status == "pending" {
            if automatic {
                out["requiresApproval"] =
                    json!("包含删除或超过 5 条记录的修改，自动执行模式下仍需用户确认；批准前不要声称已写入");
            } else {
                out["note"] =
                    json!("已生成待批准的变更卡片；用户批准前数据不会改变，不要声称已写入");
            }
        }
        Ok(out)
    }
}

fn date_arg(args: &ToolArgs, key: &str, fallback: NaiveDate) -> Result<NaiveDate, String> {
    match args.str_opt(key) {
        None => Ok(fallback),
        Some(raw) => time::parse_date(raw)
            .or_else(|| time::parse_stamp(raw).map(|t| t.date()))
            .or_else(|| match raw.trim() {
                "今天" | "today" => Some(fallback),
                "明天" | "tomorrow" => Some(time::add_days(fallback, 1)),
                "后天" => Some(time::add_days(fallback, 2)),
                "昨天" | "yesterday" => Some(time::add_days(fallback, -1)),
                _ => None,
            })
            .ok_or_else(|| format!("无法识别的日期 {key}: {raw}")),
    }
}

fn slot_json(slot: &query::Slot) -> Value {
    let mut value = json!({
        "key": slot.key,
        "title": slot.title,
        "start": time::stamp(slot.start),
        "end": time::stamp(slot.end),
        "status": slot.status.wire(),
    });
    let object = value.as_object_mut().expect("对象");
    if slot.all_day {
        object.insert("allDay".into(), json!(true));
    }
    if let Some(task) = &slot.task_id {
        object.insert("taskId".into(), json!(task));
        if let Some(status) = slot.task_status {
            object.insert("taskStatus".into(), json!(status.wire()));
        }
    }
    if slot.task_deleted {
        object.insert("taskDeleted".into(), json!(true));
    }
    if let Some(routine) = &slot.routine_id {
        object.insert("routineId".into(), json!(routine));
    }
    if slot.is_virtual {
        object.insert("virtual".into(), json!(true));
    }
    if slot.needs_confirm {
        object.insert("needsConfirm".into(), json!(true));
    }
    if !slot.note.is_empty() {
        object.insert("note".into(), json!(slot.note));
    }
    if !slot.location.is_empty() {
        object.insert("location".into(), json!(slot.location));
    }
    value
}

fn task_brief(data: &AgendaData, id: &str, now: NaiveDateTime) -> Value {
    let Some(task) = data.task(id) else {
        return json!({ "id": id });
    };
    let facts = query::task_facts(data, task, now);
    let mut value = json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.wire(),
        "priority": task.priority.wire(),
    });
    let object = value.as_object_mut().expect("对象");
    if let Some(due) = &task.due {
        object.insert("due".into(), json!(due));
    }
    if let Some(day) = &task.planned_for {
        object.insert("plannedFor".into(), json!(day));
    }
    if let Some(estimate) = task.estimate_minutes {
        object.insert("estimateMinutes".into(), json!(estimate));
    }
    if let Some(goal) = &task.goal_id {
        object.insert("goalId".into(), json!(goal));
    }
    if facts.overdue {
        object.insert("overdue".into(), json!(true));
    }
    object.insert("scheduled".into(), json!(facts.scheduled));
    if facts.planned_minutes > 0 {
        object.insert("plannedMinutes".into(), json!(facts.planned_minutes));
    }
    if facts.done_minutes > 0 {
        object.insert("doneMinutes".into(), json!(facts.done_minutes));
    }
    if facts.steps_total > 0 {
        object.insert(
            "steps".into(),
            json!(format!("{}/{}", facts.steps_done, facts.steps_total)),
        );
    }
    value
}

/// 记录全文 + 计算信息。
fn record_json(data: &AgendaData, id: &str, now: NaiveDateTime) -> Option<Value> {
    let kind = Kind::of_id(id)?;
    let mut value = match kind {
        Kind::Wish => serde_json::to_value(data.wish(id)?).ok()?,
        Kind::Goal => serde_json::to_value(data.goal(id)?).ok()?,
        Kind::Task => serde_json::to_value(data.task(id)?).ok()?,
        Kind::Entry => serde_json::to_value(data.entry(id)?).ok()?,
        Kind::Routine => serde_json::to_value(data.routine(id)?).ok()?,
        Kind::Project => serde_json::to_value(data.project(id)?).ok()?,
    };
    let object = value.as_object_mut()?;
    object.insert("kind".into(), json!(kind.wire()));
    match kind {
        Kind::Task => {
            let task = data.task(id)?;
            object.insert(
                "facts".into(),
                serde_json::to_value(query::task_facts(data, task, now)).ok()?,
            );
        }
        Kind::Goal => {
            let progress = query::goal_progress(data, id, now);
            object.insert("progress".into(), serde_json::to_value(&progress).ok()?);
            object.insert(
                "progressRatio".into(),
                json!((progress.ratio() * 100.0).round() / 100.0),
            );
        }
        Kind::Wish => {
            object.insert(
                "progress".into(),
                serde_json::to_value(query::wish_progress(data, id, now)).ok()?,
            );
        }
        Kind::Routine => {
            let routine = data.routine(id)?;
            object.insert(
                "describe".into(),
                json!(crate::agenda::recur::describe(&routine.rule)),
            );
            object.insert(
                "stats".into(),
                serde_json::to_value(query::routine_stats(data, routine, now.date())).ok()?,
            );
        }
        Kind::Entry => {
            let entry = data.entry(id)?;
            object.insert("displayTitle".into(), json!(data.entry_title(entry)));
            if data.entry_task_deleted(entry) {
                object.insert("taskDeleted".into(), json!(true));
            }
        }
        Kind::Project => {}
    }
    Some(value)
}

fn batch_detail(results: &[batch::StepResult]) -> Value {
    json!({ "steps": results.iter().map(batch::StepResult::to_json).collect::<Vec<_>>() })
}

impl ToolExecutor for AgendaToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    fn required_actions(
        &self,
        name: &str,
        _args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        // 日程待办数据不在知识库路径下，只看动作开关。写工具走 write_file：
        // 提案本身不落盘，但批准和自动执行会。
        let action = if READ_ONLY.contains(&name) {
            AiToolAction::ReadFile
        } else {
            AiToolAction::WriteFile
        };
        vec![(action, None)]
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let now = (self.now)();
        let today = now.date();
        match name {
            "agenda_overview" => {
                let data = self.load()?;
                let from = date_arg(args, "date", today)?;
                let days = args.i64_opt("days").unwrap_or(1).clamp(1, 14);
                let mut day_values = Vec::new();
                for offset in 0..days {
                    let date = time::add_days(from, offset);
                    let slots: Vec<Value> = query::day_slots(&data, date, now)
                        .iter()
                        .map(slot_json)
                        .collect();
                    let todos = query::day_todos(&data, date, now);
                    let brief = |ids: &[String]| {
                        ids.iter()
                            .map(|id| task_brief(&data, id, now))
                            .collect::<Vec<_>>()
                    };
                    let (busy, capacity) = planner::day_load(&data, date, now);
                    day_values.push(json!({
                        "date": time::date_key(date),
                        "label": time::relative_date_label(date, today),
                        "slots": slots,
                        "todos": {
                            "overdue": brief(&todos.overdue),
                            "due": brief(&todos.due),
                            "planned": brief(&todos.planned),
                            "scheduled": brief(&todos.scheduled),
                            "doing": brief(&todos.doing),
                            "doneToday": brief(&todos.done),
                        },
                        "load": { "busyMinutes": busy, "workMinutes": capacity },
                    }));
                }
                let confirm: Vec<Value> = query::confirm_queue(&data, now)
                    .iter()
                    .take(30)
                    .map(slot_json)
                    .collect();
                let goals: Vec<Value> = data
                    .goals
                    .iter()
                    .filter(|g| g.deleted_at.is_none() && g.archived_at.is_none() && g.status.is_open())
                    .map(|g| {
                        let progress = query::goal_progress(&data, &g.id, now);
                        json!({
                            "id": g.id, "title": g.title, "status": g.status.wire(),
                            "wishId": g.wish_id, "targetDate": g.target_date,
                            "tasksDone": progress.tasks_done, "tasksTotal": progress.tasks_total,
                            "criteriaDone": progress.criteria_done, "criteriaTotal": progress.criteria_total,
                        })
                    })
                    .collect();
                let wishes: Vec<Value> = data
                    .wishes
                    .iter()
                    .filter(|w| w.deleted_at.is_none() && w.archived_at.is_none())
                    .map(|w| json!({ "id": w.id, "title": w.title, "status": w.status.wire() }))
                    .collect();
                let routines: Vec<Value> = data
                    .routines
                    .iter()
                    .filter(|r| r.deleted_at.is_none() && r.archived_at.is_none())
                    .map(|r| {
                        let stats = query::routine_stats(&data, r, today);
                        json!({
                            "id": r.id, "title": r.title, "rule": crate::agenda::recur::describe(&r.rule),
                            "time": r.time, "status": r.status.wire(), "streak": stats.streak,
                            "done30": stats.done_30, "expected30": stats.expected_30,
                        })
                    })
                    .collect();
                let unscheduled: Vec<Value> = data
                    .tasks
                    .iter()
                    .filter(|t| query::is_visible_task(t) && t.status.is_open())
                    .filter(|t| !query::task_facts(&data, t, now).scheduled)
                    .take(40)
                    .map(|t| task_brief(&data, &t.id, now))
                    .collect();
                Ok(json!({
                    "now": time::stamp(now),
                    "weekday": time::weekday_label(today),
                    "days": day_values,
                    "awaitingConfirmation": confirm,
                    "unscheduledOpenTasks": unscheduled,
                    "activeGoals": goals,
                    "wishes": wishes,
                    "routines": routines,
                    "projects": data.projects.iter().filter(|p| p.deleted_at.is_none()).map(|p| json!({"id": p.id, "name": p.name, "status": p.status.wire()})).collect::<Vec<_>>(),
                    "settings": data.settings,
                }))
            }

            "agenda_list" => {
                let data = self.load()?;
                let include_hidden = args.bool_opt("includeHidden").unwrap_or(false);
                let limit = args.i64_opt("limit").unwrap_or(100).clamp(1, 500) as usize;
                let kind = args
                    .str_opt("kind")
                    .map(|k| Kind::from_wire(k).ok_or_else(|| format!("未知的 kind: {k}")))
                    .transpose()?;
                if kind == Some(Kind::Entry) || args.str_opt("from").is_some() {
                    let from = date_arg(args, "from", today)?;
                    let to = date_arg(args, "to", time::add_days(from, 6))?;
                    if (to - from).num_days() > 92 {
                        return Err("日程查询跨度不能超过 92 天".into());
                    }
                    let slots: Vec<Value> = query::slots_between(&data, from, to, now)
                        .iter()
                        .filter(|s| {
                            args.str_opt("status")
                                .is_none_or(|st| s.status.wire() == st)
                        })
                        .filter(|s| {
                            args.str_opt("taskId")
                                .is_none_or(|t| s.task_id.as_deref() == Some(t))
                        })
                        .take(limit)
                        .map(slot_json)
                        .collect();
                    return Ok(
                        json!({ "from": time::date_key(from), "to": time::date_key(to), "slots": slots }),
                    );
                }
                let ids: Vec<String> = match args.str_opt("query") {
                    Some(q) => query::search(&data, q, include_hidden)
                        .into_iter()
                        .filter(|(k, _, _)| kind.is_none_or(|kind| *k == kind))
                        .map(|(_, id, _)| id)
                        .collect(),
                    None => {
                        let visible = |deleted: &Option<String>, archived: &Option<String>| {
                            include_hidden || (deleted.is_none() && archived.is_none())
                        };
                        let mut ids = Vec::new();
                        let want = |k: Kind| kind.is_none_or(|kind| kind == k);
                        if want(Kind::Wish) {
                            ids.extend(
                                data.wishes
                                    .iter()
                                    .filter(|r| visible(&r.deleted_at, &r.archived_at))
                                    .map(|r| r.id.clone()),
                            );
                        }
                        if want(Kind::Goal) {
                            ids.extend(
                                data.goals
                                    .iter()
                                    .filter(|r| visible(&r.deleted_at, &r.archived_at))
                                    .filter(|r| {
                                        args.str_opt("wishId")
                                            .is_none_or(|w| r.wish_id.as_deref() == Some(w))
                                    })
                                    .map(|r| r.id.clone()),
                            );
                        }
                        if want(Kind::Task) {
                            let mut tasks: Vec<_> = data
                                .tasks
                                .iter()
                                .filter(|r| visible(&r.deleted_at, &r.archived_at))
                                .filter(|r| {
                                    args.str_opt("goalId")
                                        .is_none_or(|g| r.goal_id.as_deref() == Some(g))
                                })
                                .collect();
                            query::sort_tasks(&mut tasks, now);
                            ids.extend(tasks.into_iter().map(|r| r.id.clone()));
                        }
                        if want(Kind::Routine) {
                            ids.extend(
                                data.routines
                                    .iter()
                                    .filter(|r| visible(&r.deleted_at, &r.archived_at))
                                    .map(|r| r.id.clone()),
                            );
                        }
                        if want(Kind::Project) {
                            ids.extend(
                                data.projects
                                    .iter()
                                    .filter(|r| visible(&r.deleted_at, &r.archived_at))
                                    .map(|r| r.id.clone()),
                            );
                        }
                        ids
                    }
                };
                let status = args.str_opt("status");
                let project = args.str_opt("projectId");
                let items: Vec<Value> = ids
                    .iter()
                    .filter_map(|id| record_json(&data, id, now))
                    .filter(|v| status.is_none_or(|s| v["status"] == s))
                    .filter(|v| project.is_none_or(|p| v["project_id"] == p))
                    .take(limit)
                    .collect();
                Ok(json!({ "count": items.len(), "items": items }))
            }

            "agenda_get" => {
                let data = self.load()?;
                let id = args.str_required("id")?;
                let (id, virtual_slot) = if id.contains('@') {
                    let slot =
                        query::slot_by_key(&data, id, now).ok_or_else(|| format!("找不到 {id}"))?;
                    match slot.entry_id.clone() {
                        Some(entry) => (entry, None),
                        None => (
                            slot.routine_id.clone().unwrap_or_default(),
                            Some(slot_json(&slot)),
                        ),
                    }
                } else {
                    (id.to_owned(), None)
                };
                let mut record =
                    record_json(&data, &id, now).ok_or_else(|| format!("记录不存在: {id}"))?;
                if let Some(slot) = virtual_slot {
                    record["occurrence"] = slot;
                }
                let chain: Vec<Value> = query::why_chain(&data, &id)
                    .into_iter()
                    .map(|n| json!({ "kind": n.kind.wire(), "id": n.id, "title": n.title }))
                    .collect();
                let mut children = Vec::new();
                match Kind::of_id(&id) {
                    Some(Kind::Wish) => children.extend(
                        data.goals.iter().filter(|g| g.deleted_at.is_none() && g.wish_id.as_deref() == Some(&id)).map(|g| json!({"kind": "goal", "id": g.id, "title": g.title, "status": g.status.wire()})),
                    ),
                    Some(Kind::Goal) => {
                        children.extend(data.tasks.iter().filter(|t| t.deleted_at.is_none() && t.goal_id.as_deref() == Some(&id)).map(|t| task_brief(&data, &t.id, now)));
                        children.extend(data.routines.iter().filter(|r| r.deleted_at.is_none() && r.goal_id.as_deref() == Some(&id)).map(|r| json!({"kind": "routine", "id": r.id, "title": r.title})));
                    }
                    _ => {}
                }
                let blocks: Vec<Value> = if Kind::of_id(&id) == Some(Kind::Task) {
                    let mut entries: Vec<_> = data
                        .entries
                        .iter()
                        .filter(|e| e.task_id.as_deref() == Some(&id))
                        .collect();
                    entries.sort_by(|a, b| a.start.cmp(&b.start));
                    entries
                        .iter()
                        .map(|e| json!({"id": e.id, "start": e.start, "end": e.end, "status": e.status.wire(), "actualMinutes": e.actual_minutes}))
                        .collect()
                } else {
                    Vec::new()
                };
                Ok(
                    json!({ "record": record, "why": chain, "children": children, "blocks": blocks }),
                )
            }

            "agenda_find_free_time" => {
                let data = self.load()?;
                let date = date_arg(args, "date", today)?;
                let min = args.i64_opt("minMinutes").unwrap_or(30).clamp(5, 24 * 60);
                let window = match (args.str_opt("from"), args.str_opt("to")) {
                    (Some(from), Some(to)) => {
                        let at = |raw: &str| {
                            time::parse_hm(raw)
                                .map(|t| date.and_time(t))
                                .or_else(|| time::parse_stamp(raw))
                                .ok_or_else(|| format!("无法识别的时间: {raw}"))
                        };
                        Some((
                            at(from)?,
                            if to == "24:00" {
                                time::add_days(date, 1).and_time(chrono::NaiveTime::MIN)
                            } else {
                                at(to)?
                            },
                        ))
                    }
                    _ => None,
                };
                let free: Vec<Value> = planner::free_slots(&data, date, now, window, min)
                    .iter()
                    .map(|s| json!({ "start": time::stamp(s.start), "end": time::stamp(s.end), "minutes": s.minutes() }))
                    .collect();
                let suggestions: Vec<Value> = planner::auto_place(&data, date, now, None)
                    .iter()
                    .map(|p| json!({ "taskId": p.task_id, "title": p.title, "start": time::stamp(p.start), "end": time::stamp(p.end), "reason": p.reason }))
                    .collect();
                let (busy, capacity) = planner::day_load(&data, date, now);
                Ok(json!({
                    "date": time::date_key(date),
                    "free": free,
                    "load": { "busyMinutes": busy, "workMinutes": capacity },
                    "suggestedPlacements": suggestions,
                }))
            }

            "agenda_history" => {
                let limit = args.i64_opt("limit").unwrap_or(20).clamp(1, 200) as usize;
                Ok(json!({ "entries": self.store.history(limit) }))
            }

            "agenda_batch" => {
                let operations = args
                    .get("operations")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| "缺少必填参数 operations（数组）".to_string())?;
                if operations.is_empty() {
                    return Err("operations 不能为空".into());
                }
                if operations.len() > 200 {
                    return Err("一批最多 200 个操作".into());
                }
                let summary = args.str_opt("summary").unwrap_or_default().to_owned();
                self.propose(&summary, |editor| {
                    let results = batch::run(editor, &operations)?;
                    Ok(batch_detail(&results))
                })
            }

            "agenda_plan_day" => {
                let data = self.load()?;
                let date = date_arg(args, "date", today)?;
                let only = args.str_array("taskIds");
                let only = (!only.is_empty()).then_some(only);
                let placements = planner::auto_place(&data, date, now, only.as_deref());
                if placements.is_empty() {
                    return Ok(
                        json!({ "unchanged": true, "message": "这一天没有需要排入的任务，或已经没有足够的空闲时段" }),
                    );
                }
                let operations: Vec<Value> = placements
                    .iter()
                    .map(|p| json!({ "op": "schedule", "taskId": p.task_id, "start": time::stamp(p.start), "end": time::stamp(p.end) }))
                    .collect();
                let summary = args
                    .str_opt("summary")
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "为{}排入 {} 个时间块",
                            time::relative_date_label(date, today),
                            placements.len()
                        )
                    });
                let reasons: Vec<Value> = placements
                    .iter()
                    .map(|p| json!({ "title": p.title, "start": time::stamp(p.start), "end": time::stamp(p.end), "reason": p.reason }))
                    .collect();
                self.propose(&summary, |editor| {
                    let results = batch::run(editor, &operations)?;
                    let mut detail = batch_detail(&results);
                    detail["placements"] = json!(reasons);
                    Ok(detail)
                })
            }

            "agenda_apply_change" => {
                let raw = args
                    .get(MESSAGE_KEY)
                    .or_else(|| args.get("proposal"))
                    .cloned()
                    .ok_or_else(|| format!("缺少必填参数 {MESSAGE_KEY}"))?;
                let mut proposal =
                    Proposal::from_value(&raw).map_err(|e| format!("提案格式不合法: {e}"))?;
                if proposal.status != "pending" {
                    return Err(format!("此提案状态为 {}，不能再次应用", proposal.status));
                }
                self.store
                    .apply(Source::Ai, &proposal.to_change(), true)
                    .map_err(|e| format!("{e:#}"))?;
                proposal.status = "applied".into();
                Ok(json!({ MESSAGE_KEY: proposal.to_value() }))
            }

            other => Err(format!("未知的日程待办工具: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_now() -> NaiveDateTime {
        time::parse_stamp("2026-09-25T08:00").unwrap()
    }

    fn fixture() -> (std::path::PathBuf, AgendaToolExecutor, Arc<AgendaStore>) {
        let root = std::env::temp_dir().join(format!(
            "mochi-agenda-tools-{}",
            crate::paths::random_base36(10)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(AgendaStore::new(&root));
        (
            root,
            AgendaToolExecutor::new(store.clone()).with_now(fixed_now),
            store,
        )
    }

    fn call(executor: &AgendaToolExecutor, name: &str, args: Value) -> Value {
        executor.call(name, &ToolArgs::from_value(args)).unwrap()
    }

    #[test]
    fn batch_proposes_then_apply_writes_once() {
        let (root, executor, store) = fixture();
        let out = call(
            &executor,
            "agenda_batch",
            json!({"summary": "英语计划", "operations": [
                {"op": "create", "kind": "goal", "ref": "g", "title": "六级 550"},
                {"op": "create", "kind": "task", "ref": "t", "title": "真题一套", "goalId": "$g", "due": "2026-09-26", "estimateMinutes": 120},
                {"op": "schedule", "taskId": "$t", "start": "2026-09-25T19:00", "end": "2026-09-25T21:00"},
            ]}),
        );
        assert_eq!(out[MESSAGE_KEY]["status"], "pending");
        assert!(store.load().unwrap().tasks.is_empty(), "未批准前不写入");
        let applied = call(
            &executor,
            "agenda_apply_change",
            json!({ MESSAGE_KEY: out[MESSAGE_KEY] }),
        );
        assert_eq!(applied[MESSAGE_KEY]["status"], "applied");
        let data = store.load().unwrap();
        assert_eq!(
            (data.goals.len(), data.tasks.len(), data.entries.len()),
            (1, 1, 1)
        );
        // 同一份提案重放会因记录已存在而冲突。
        assert!(executor
            .call(
                "agenda_apply_change",
                &ToolArgs::from_value(json!({ MESSAGE_KEY: out[MESSAGE_KEY] }))
            )
            .is_err());
        let overview = call(&executor, "agenda_overview", json!({}));
        assert_eq!(overview["days"][0]["slots"][0]["title"], "真题一套");
        let task_id = data.tasks[0].id.clone();
        let got = call(&executor, "agenda_get", json!({ "id": task_id }));
        assert_eq!(got["why"][0]["title"], "六级 550");
        assert_eq!(got["blocks"].as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn plan_day_places_due_tasks_into_free_time() {
        let (root, executor, store) = fixture();
        store
            .mutate(Source::User, |ed| {
                ed.create_task(crate::agenda::TaskDraft {
                    title: "写报告".into(),
                    due: Some("2026-09-25".into()),
                    estimate_minutes: Some(60),
                    ..Default::default()
                })
            })
            .unwrap();
        let out = call(&executor, "agenda_plan_day", json!({}));
        let ops = out[MESSAGE_KEY]["operations"].as_array().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["collection"], "entry");
        assert_eq!(out["detail"]["placements"][0]["start"], "2026-09-25T09:00");
        std::fs::remove_dir_all(root).ok();
    }
}
