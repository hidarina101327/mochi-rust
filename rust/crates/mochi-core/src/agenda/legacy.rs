//! 从旧版 `schedule/*.json` 一次性迁移到新的日程待办数据。
//!
//! 旧模型把愿望、任务、习惯、日程混在两张表里靠 `itemType` 区分；这里按新概念拆开：
//! - `itemType` 为 someday/goal/note/memo → 愿望
//! - `itemType` 为 habit 或带重复规则的日程 → 重复安排（打卡记录落成已执行的日程）
//! - 其余任务 → 任务；任务上的固定时间 → 该任务的一个时间块
//! - 其余日程 → 独立日程
//!
//! 旧的任务可以直接挂在愿望下；新模型里任务只挂目标，所以为这样的愿望补一个同名目标。
//! 旧文件原样保留，不做删除。

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use super::model::*;
use super::recur;
use super::time::{self, Due};

fn read_array(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn time_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get("time")
        .and_then(|t| t.get(key))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn tags(v: &Value) -> Vec<String> {
    v.get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn priority(v: &Value) -> Priority {
    match s(v, "priority") {
        Some("low") => Priority::Low,
        Some("high") => Priority::High,
        Some("urgent") => Priority::Urgent,
        _ => Priority::Normal,
    }
}

fn is_wish(v: &Value) -> bool {
    matches!(
        s(v, "itemType").map(str::to_ascii_lowercase).as_deref(),
        Some("someday" | "goal" | "note" | "memo")
    )
}

fn is_habit(v: &Value) -> bool {
    s(v, "itemType").is_some_and(|t| t.eq_ignore_ascii_case("habit"))
}

fn created(v: &Value) -> String {
    s(v, "createdAt")
        .map(str::to_owned)
        .unwrap_or_else(crate::jstime::now)
}

fn updated(v: &Value) -> String {
    s(v, "updatedAt")
        .map(str::to_owned)
        .unwrap_or_else(|| created(v))
}

fn new_id(kind: Kind, old: Option<&str>) -> String {
    // 保留旧 ID 的唯一后缀，方便排查；前缀换成新种类。
    let suffix = old
        .and_then(|id| id.split_once('-').map(|(_, rest)| rest.to_owned()))
        .unwrap_or_else(|| crate::paths::random_base36(8));
    format!("{}-{suffix}", kind.id_prefix())
}

pub fn has_legacy(schedule_dir: &Path) -> bool {
    ["tasks.json", "events.json", "projects.json"]
        .iter()
        .any(|f| schedule_dir.join(f).is_file())
}

pub fn migrate(schedule_dir: &Path) -> AgendaData {
    let events = read_array(&schedule_dir.join("events.json"));
    let tasks = read_array(&schedule_dir.join("tasks.json"));
    let projects = read_array(&schedule_dir.join("projects.json"));
    let mut data = AgendaData::default();
    let mut project_ids: HashMap<String, String> = HashMap::new();
    let mut wish_ids: HashMap<String, String> = HashMap::new();

    for p in &projects {
        let Some(old) = s(p, "id") else { continue };
        let id = new_id(Kind::Project, Some(old));
        project_ids.insert(old.to_owned(), id.clone());
        data.projects.push(Project {
            id,
            name: s(p, "name").unwrap_or("未命名项目").to_owned(),
            note: s(p, "description").unwrap_or_default().to_owned(),
            color: s(p, "color").map(str::to_owned),
            status: if s(p, "status") == Some("done") {
                ProjectStatus::Done
            } else {
                ProjectStatus::Active
            },
            archived_at: (s(p, "status") == Some("archived")).then(|| updated(p)),
            deleted_at: None,
            created_at: created(p),
            updated_at: updated(p),
        });
    }
    let project_of = |v: &Value| s(v, "projectId").and_then(|p| project_ids.get(p).cloned());

    // 先建愿望，任务和日程才能找到父级。
    for item in events.iter().chain(tasks.iter()).filter(|v| is_wish(v)) {
        let Some(old) = s(item, "id") else { continue };
        let id = new_id(Kind::Wish, Some(old));
        wish_ids.insert(old.to_owned(), id.clone());
        let status = s(item, "status").unwrap_or_default();
        data.wishes.push(Wish {
            id,
            title: s(item, "title").unwrap_or("未命名愿望").to_owned(),
            note: s(item, "description").unwrap_or_default().to_owned(),
            status: if status == "done" {
                WishStatus::Realized
            } else if status == "cancelled" {
                WishStatus::Dropped
            } else {
                WishStatus::Open
            },
            project_id: project_of(item),
            tags: tags(item),
            archived_at: (status == "archived").then(|| updated(item)),
            created_at: created(item),
            updated_at: updated(item),
            ..Default::default()
        });
    }

    let mut goal_for_wish: HashMap<String, String> = HashMap::new();
    let mut goal_for = |data: &mut AgendaData, wish_id: &str| -> String {
        if let Some(id) = goal_for_wish.get(wish_id) {
            return id.clone();
        }
        let wish = data.wish(wish_id).cloned().unwrap_or_default();
        let id = new_id(Kind::Goal, None);
        data.goals.push(Goal {
            id: id.clone(),
            title: wish.title.clone(),
            wish_id: Some(wish_id.to_owned()),
            project_id: wish.project_id.clone(),
            created_at: wish.created_at.clone(),
            updated_at: wish.updated_at.clone(),
            ..Default::default()
        });
        goal_for_wish.insert(wish_id.to_owned(), id.clone());
        id
    };

    for item in tasks.iter().chain(events.iter()) {
        if is_wish(item) {
            continue;
        }
        let is_event = s(item, "kind") == Some("event");
        let rule = s(item, "recurrenceRule").and_then(recur::normalize);
        let parent_wish = s(item, "parentId").and_then(|p| wish_ids.get(p).cloned());
        let start = time_field(item, "startAt").and_then(time::parse_stamp);
        let end = time_field(item, "endAt").and_then(time::parse_stamp);
        let all_day = item.get("allDay").and_then(Value::as_bool).unwrap_or(false);
        let status = s(item, "status").unwrap_or_default();
        let archived = (status == "archived").then(|| updated(item));

        if is_habit(item) || rule.is_some() {
            let id = new_id(Kind::Routine, s(item, "id"));
            let start_date = start
                .map(|t| t.date())
                .or_else(|| {
                    s(item, "createdAt")
                        .and_then(time::parse_stamp)
                        .map(|t| t.date())
                })
                .unwrap_or_else(time::today);
            let duration = match (start, end) {
                (Some(a), Some(b)) if b > a => time::minutes_between(a, b) as u32,
                _ => 30,
            };
            let goal_id = parent_wish.as_deref().map(|w| goal_for(&mut data, w));
            data.routines.push(Routine {
                id: id.clone(),
                title: s(item, "title").unwrap_or("重复事项").to_owned(),
                note: s(item, "description").unwrap_or_default().to_owned(),
                rule: rule.unwrap_or_else(|| "FREQ=DAILY".into()),
                start_date: time::date_key(start_date),
                time: start
                    .filter(|_| !all_day && is_event)
                    .map(|t| time::hm(t.time())),
                duration_minutes: duration,
                goal_id,
                project_id: project_of(item),
                archived_at: archived,
                created_at: created(item),
                updated_at: updated(item),
                ..Default::default()
            });
            let done_dates = item
                .get("completedDates")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for date in done_dates.iter().filter_map(Value::as_str) {
                let Some(day) = time::parse_date(date) else {
                    continue;
                };
                data.entries.push(Entry {
                    id: new_id(Kind::Entry, None),
                    routine_id: Some(id.clone()),
                    occurrence: Some(time::date_key(day)),
                    start: time::stamp(day.and_time(chrono::NaiveTime::MIN)),
                    end: time::stamp(time::add_days(day, 1).and_time(chrono::NaiveTime::MIN)),
                    all_day: true,
                    status: EntryStatus::Done,
                    created_at: created(item),
                    updated_at: updated(item),
                    ..Default::default()
                });
            }
            continue;
        }

        let entry_status = match status {
            "done" => EntryStatus::Done,
            "cancelled" => EntryStatus::Cancelled,
            _ => EntryStatus::Planned,
        };
        let block = start.map(|a| {
            let b = end
                .filter(|b| *b > a)
                .unwrap_or_else(|| a + chrono::Duration::minutes(if all_day { 1440 } else { 60 }));
            (a, b)
        });

        if is_event {
            let Some((a, b)) = block else { continue };
            data.entries.push(Entry {
                id: new_id(Kind::Entry, s(item, "id")),
                title: s(item, "title").unwrap_or("未命名日程").to_owned(),
                note: s(item, "description").unwrap_or_default().to_owned(),
                start: time::stamp(a),
                end: time::stamp(b),
                all_day,
                status: entry_status,
                project_id: project_of(item),
                created_at: created(item),
                updated_at: updated(item),
                ..Default::default()
            });
            continue;
        }

        let task_id = new_id(Kind::Task, s(item, "id"));
        let task_status = match status {
            "done" => TaskStatus::Done,
            "cancelled" => TaskStatus::Cancelled,
            "doing" | "in_progress" | "active" => TaskStatus::Doing,
            _ => TaskStatus::Todo,
        };
        let goal_id = parent_wish.as_deref().map(|w| goal_for(&mut data, w));
        let estimate = item
            .get("cost")
            .and_then(|c| c.get("durationMinutes"))
            .or_else(|| item.get("time").and_then(|t| t.get("durationMinutes")))
            .and_then(Value::as_u64)
            .map(|m| m as u32);
        data.tasks.push(Task {
            id: task_id.clone(),
            title: s(item, "title").unwrap_or("未命名任务").to_owned(),
            note: s(item, "description").unwrap_or_default().to_owned(),
            goal_id,
            project_id: project_of(item),
            status: task_status,
            priority: priority(item),
            due: time_field(item, "dueAt")
                .or_else(|| time_field(item, "latestEnd"))
                .and_then(|d| {
                    if d.len() <= 10 {
                        time::normalize_due(d)
                    } else {
                        time::parse_stamp(d).map(|t| Due::At(t).wire())
                    }
                }),
            estimate_minutes: estimate,
            tags: tags(item),
            completed_at: (task_status == TaskStatus::Done).then(|| updated(item)),
            archived_at: archived,
            created_at: created(item),
            updated_at: updated(item),
            ..Default::default()
        });
        if let Some((a, b)) = block {
            data.entries.push(Entry {
                id: new_id(Kind::Entry, None),
                task_id: Some(task_id),
                start: time::stamp(a),
                end: time::stamp(b),
                all_day,
                status: entry_status,
                created_at: created(item),
                updated_at: updated(item),
                ..Default::default()
            });
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_legacy_items_into_new_concepts() {
        let dir =
            std::env::temp_dir().join(format!("agenda-legacy-{}", crate::paths::random_base36(6)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tasks.json"),
            r#"[
              {"id":"task-1-a","kind":"task","title":"去冰岛","itemType":"someday","status":"draft","tags":[],"priority":"medium","time":{"kind":"intent"}},
              {"id":"task-2-b","kind":"task","title":"攒钱","parentId":"task-1-a","status":"todo","tags":[],"priority":"high","time":{"kind":"intent","dueAt":"2026-10-01"}},
              {"id":"task-3-c","kind":"task","title":"背单词","itemType":"habit","status":"todo","tags":[],"priority":"medium","time":{"kind":"intent"},"completedDates":["2026-09-20"]}
            ]"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("events.json"),
            r#"[{"id":"event-4-d","kind":"event","title":"组会","status":"scheduled","tags":[],"priority":"medium","time":{"kind":"fixed","startAt":"2026-09-25T10:00","endAt":"2026-09-25T11:00"}}]"#,
        )
        .unwrap();
        let data = migrate(&dir);
        assert_eq!(data.wishes.len(), 1);
        assert_eq!(data.goals.len(), 1, "挂在愿望下的任务需要一个桥接目标");
        assert_eq!(
            data.goals[0].wish_id.as_deref(),
            Some(data.wishes[0].id.as_str())
        );
        let task = &data.tasks[0];
        assert_eq!(task.goal_id.as_deref(), Some(data.goals[0].id.as_str()));
        assert_eq!(task.due.as_deref(), Some("2026-10-01"));
        assert_eq!(task.priority, Priority::High);
        assert_eq!(data.routines.len(), 1);
        assert_eq!(data.entries.len(), 2);
        assert!(data
            .entries
            .iter()
            .any(|e| e.title == "组会" && e.start == "2026-09-25T10:00"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
