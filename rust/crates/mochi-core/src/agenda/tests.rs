//! 规则测试：独立性、逾期、多次排期、提前完成、删除与归档、单一数据来源。

use chrono::NaiveDateTime;
use serde_json::json;

use super::change::ChangeSet;
use super::model::*;
use super::ops::*;
use super::query;
use super::time;

fn at(s: &str) -> NaiveDateTime {
    time::parse_stamp(s).unwrap()
}

fn editor<'a>(data: &'a mut AgendaData, now: &str) -> Editor<'a> {
    Editor::new(data, Clock::fixed(at(now)))
}

fn task(ed: &mut Editor<'_>, title: &str) -> String {
    ed.create_task(TaskDraft {
        title: title.into(),
        ..Default::default()
    })
    .unwrap()
}

fn patch(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn short_flow_math_homework() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T08:00");
    let id = task(&mut ed, "数学作业");
    ed.update(&id, &patch(json!({"due": "2026-09-27"})))
        .unwrap();
    let a = ed
        .schedule_task(&id, "2026-09-25T19:00", "2026-09-25T20:00")
        .unwrap();
    let b = ed
        .schedule_task(&id, "2026-09-26T19:00", "2026-09-26T20:30")
        .unwrap();
    assert_ne!(a, b, "每次排期只新增一个块");
    assert_eq!(ed.data.entries.len(), 2);
    let facts = query::task_facts(ed.data, ed.data.task(&id).unwrap(), at("2026-09-25T08:00"));
    assert!(facts.scheduled);
    assert_eq!(facts.planned_minutes, 150);
    ed.set_entry_status(&a, EntryStatus::Done, None).unwrap();
    assert_eq!(
        ed.data.task(&id).unwrap().status,
        TaskStatus::Todo,
        "确认块不完成任务"
    );
    let future = ed.complete_task(&id).unwrap();
    assert_eq!(future.entry_ids, vec![b.clone()]);
    assert_eq!(
        ed.data.entry(&b).unwrap().status,
        EntryStatus::Planned,
        "完成任务默认保留未来块"
    );
}

#[test]
fn passing_time_changes_nothing() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T08:00");
    let id = task(&mut ed, "阅读");
    ed.update(
        &id,
        &patch(json!({"due": "2026-09-25", "planned_for": "2026-09-25"})),
    )
    .unwrap();
    let e = ed
        .schedule_task(&id, "2026-09-25T09:00", "2026-09-25T10:00")
        .unwrap();
    let later = at("2026-09-27T12:00");
    let slot = query::slot_by_key(&data, &e, later).unwrap();
    assert!(slot.time_passed && slot.needs_confirm);
    assert_eq!(
        slot.status,
        EntryStatus::Planned,
        "时间过了不自动标记已执行"
    );
    let t = data.task(&id).unwrap();
    assert_eq!(t.status, TaskStatus::Todo);
    assert_eq!(t.due.as_deref(), Some("2026-09-25"), "不自动顺延截止");
    assert_eq!(t.planned_for.as_deref(), Some("2026-09-25"));
    assert_eq!(data.entries.len(), 1, "不自动生成明天的块");
    assert!(query::is_overdue(t, later));
    let todos = query::day_todos(&data, later.date(), later);
    assert_eq!(todos.overdue, vec![id]);
}

#[test]
fn overdue_only_for_open_tasks() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T08:00");
    let open = task(&mut ed, "a");
    let done = task(&mut ed, "b");
    for id in [&open, &done] {
        ed.update(id, &patch(json!({"due": "2026-09-20T18:00"})))
            .unwrap();
    }
    ed.complete_task(&done).unwrap();
    let now = at("2026-09-25T08:00");
    assert!(query::is_overdue(data.task(&open).unwrap(), now));
    assert!(!query::is_overdue(data.task(&done).unwrap(), now));
}

#[test]
fn early_completion_cancels_only_future_planned_blocks() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = task(&mut ed, "论文");
    let past = ed
        .schedule_task(&id, "2026-09-24T09:00", "2026-09-24T10:00")
        .unwrap();
    let done = ed
        .schedule_task(&id, "2026-09-25T09:00", "2026-09-25T10:00")
        .unwrap();
    let future = ed
        .schedule_task(&id, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    ed.set_entry_status(&done, EntryStatus::Done, None).unwrap();
    let report = ed.complete_task(&id).unwrap();
    assert_eq!(report.entry_ids, vec![future.clone()]);
    // 即使调用方把历史块也塞进来，也只取消未来计划块。
    let n = ed.cancel_future_entries(&[past.clone(), done.clone(), future.clone()]);
    assert_eq!(n, 1);
    assert_eq!(ed.data.entry(&past).unwrap().status, EntryStatus::Planned);
    assert_eq!(ed.data.entry(&done).unwrap().status, EntryStatus::Done);
    assert_eq!(
        ed.data.entry(&future).unwrap().status,
        EntryStatus::Cancelled
    );
    ed.set_task_status(&id, TaskStatus::Todo).unwrap();
    assert_eq!(
        ed.data.entry(&future).unwrap().status,
        EntryStatus::Cancelled,
        "重新打开不恢复已取消的块"
    );
}

#[test]
fn combined_action_is_explicit() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = task(&mut ed, "报告");
    let a = ed
        .schedule_task(&id, "2026-09-25T09:00", "2026-09-25T10:00")
        .unwrap();
    let b = ed
        .schedule_task(&id, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    let report = ed.complete_entry_and_task(&a).unwrap();
    assert_eq!(ed.data.task(&id).unwrap().status, TaskStatus::Done);
    assert_eq!(ed.data.entry(&a).unwrap().status, EntryStatus::Done);
    assert_eq!(report.entry_ids, vec![b]);
}

#[test]
fn goals_and_wishes_never_auto_close() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let wish = ed
        .create_wish(WishDraft {
            title: "英语变好".into(),
            ..Default::default()
        })
        .unwrap();
    let goal = ed
        .create_goal(GoalDraft {
            title: "六级 550".into(),
            wish_id: Some(wish.clone()),
            ..Default::default()
        })
        .unwrap();
    let t = ed
        .create_task(TaskDraft {
            title: "刷真题".into(),
            goal_id: Some(goal.clone()),
            ..Default::default()
        })
        .unwrap();
    ed.complete_task(&t).unwrap();
    assert_eq!(ed.data.goal(&goal).unwrap().status, GoalStatus::Active);
    ed.set_goal_status(&goal, GoalStatus::Achieved, Some("考了 561"))
        .unwrap();
    assert_eq!(ed.data.wish(&wish).unwrap().status, WishStatus::Open);
    let chain = query::why_chain(ed.data, &t);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].kind, Kind::Wish);
}

#[test]
fn deleting_parents_unlinks_children_and_restore_relinks() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let wish = ed
        .create_wish(WishDraft {
            title: "w".into(),
            ..Default::default()
        })
        .unwrap();
    let goal = ed
        .create_goal(GoalDraft {
            title: "g".into(),
            wish_id: Some(wish.clone()),
            ..Default::default()
        })
        .unwrap();
    let t = ed
        .create_task(TaskDraft {
            title: "t".into(),
            goal_id: Some(goal.clone()),
            ..Default::default()
        })
        .unwrap();
    ed.delete(&wish, false).unwrap();
    assert!(
        ed.data.goal(&goal).unwrap().deleted_at.is_none(),
        "删愿望不删目标"
    );
    assert_eq!(ed.data.goal(&goal).unwrap().wish_id, None);
    ed.delete(&goal, false).unwrap();
    let task_after = ed.data.task(&t).unwrap();
    assert!(task_after.deleted_at.is_none(), "删目标不删任务");
    assert_eq!(task_after.goal_id, None);
    assert_eq!(task_after.title, "t");
    ed.restore(&goal).unwrap();
    assert_eq!(
        ed.data.task(&t).unwrap().goal_id.as_deref(),
        Some(goal.as_str())
    );
}

#[test]
fn deleting_a_task_keeps_history_traceable() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = task(&mut ed, "旧任务");
    let past = ed
        .schedule_task(&id, "2026-09-24T09:00", "2026-09-24T10:00")
        .unwrap();
    let future = ed
        .schedule_task(&id, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    ed.delete(&id, false).unwrap();
    assert_eq!(
        ed.data.entry(&future).unwrap().status,
        EntryStatus::Planned,
        "默认不取消"
    );
    assert!(ed.data.entry_task_deleted(ed.data.entry(&past).unwrap()));
    assert_eq!(ed.data.entry_title(ed.data.entry(&past).unwrap()), "旧任务");
    ed.purge(&id).unwrap();
    let entry = ed.data.entry(&past).unwrap();
    assert_eq!(entry.task_id, None, "清除后不留悬空引用");
    assert_eq!(entry.detached_task_title.as_deref(), Some("旧任务"));
    assert!(ed.data.entry_task_deleted(entry));
}

#[test]
fn delete_task_can_cancel_future_blocks_on_request() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = task(&mut ed, "x");
    let future = ed
        .schedule_task(&id, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    ed.delete(&id, true).unwrap();
    assert_eq!(
        ed.data.entry(&future).unwrap().status,
        EntryStatus::Cancelled
    );
}

#[test]
fn archive_does_not_cascade() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let goal = ed
        .create_goal(GoalDraft {
            title: "g".into(),
            ..Default::default()
        })
        .unwrap();
    let t = ed
        .create_task(TaskDraft {
            title: "t".into(),
            goal_id: Some(goal.clone()),
            ..Default::default()
        })
        .unwrap();
    let e = ed
        .schedule_task(&t, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    ed.set_archived(&goal, true).unwrap();
    assert!(ed.data.task(&t).unwrap().archived_at.is_none());
    assert_eq!(
        ed.data.task(&t).unwrap().goal_id.as_deref(),
        Some(goal.as_str())
    );
    assert_eq!(ed.data.entry(&e).unwrap().status, EntryStatus::Planned);
}

#[test]
fn block_title_follows_the_task() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = task(&mut ed, "初稿");
    let e = ed
        .schedule_task(&id, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    ed.update(&id, &patch(json!({"title": "终稿"}))).unwrap();
    assert_eq!(ed.data.entry_title(ed.data.entry(&e).unwrap()), "终稿");
    ed.update(&e, &patch(json!({"note": "去图书馆"}))).unwrap();
    assert!(ed.data.entry(&e).unwrap().title.is_empty());
    assert!(
        ed.update(&e, &patch(json!({"status": "done"}))).is_err(),
        "状态走专用操作"
    );
}

#[test]
fn linking_changes_only_the_relation() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let goal = ed
        .create_goal(GoalDraft {
            title: "g".into(),
            ..Default::default()
        })
        .unwrap();
    let id = task(&mut ed, "t");
    ed.update(
        &id,
        &patch(json!({"due": "2026-10-01", "priority": "high"})),
    )
    .unwrap();
    let before = ed.data.task(&id).unwrap().clone();
    ed.link(&id, Some(&goal)).unwrap();
    let after = ed.data.task(&id).unwrap();
    assert_eq!(after.goal_id.as_deref(), Some(goal.as_str()));
    assert_eq!(
        (
            after.title.as_str(),
            &after.due,
            after.status,
            after.priority
        ),
        (
            before.title.as_str(),
            &before.due,
            before.status,
            before.priority
        )
    );
    assert!(ed.link(&goal, Some(&id)).is_err(), "目标不能挂到任务下");
    // 解除时间块与任务的关联：成为沿用标题的独立日程。
    let e = ed
        .schedule_task(&id, "2026-09-26T09:00", "2026-09-26T10:00")
        .unwrap();
    ed.link(&e, None).unwrap();
    let entry = ed.data.entry(&e).unwrap();
    assert_eq!(entry.task_id, None);
    assert_eq!(entry.title, "t");
}

#[test]
fn routines_are_virtual_until_confirmed() {
    let mut data = AgendaData::default();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let r = ed
        .create_routine(RoutineDraft {
            title: "背单词".into(),
            rule: "FREQ=DAILY".into(),
            start_date: Some("2026-09-20".into()),
            time: Some("07:30".into()),
            duration_minutes: Some(20),
            ..Default::default()
        })
        .unwrap();
    let now = at("2026-09-25T12:00");
    let slots = query::day_slots(&data, now.date(), now);
    assert_eq!(slots.len(), 1);
    assert!(slots[0].is_virtual && slots[0].needs_confirm);
    let queue = query::confirm_queue(&data, now);
    assert!(queue.len() >= 5);
    let key = slots[0].key.clone();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = ed.confirm(&key, EntryStatus::Done).unwrap();
    let again = ed.confirm(&key, EntryStatus::Done).unwrap();
    assert_eq!(id, again, "同一天只落地一次");
    let stats = query::routine_stats(&data, data.routine(&r).unwrap(), now.date());
    assert_eq!(stats.streak, 1);
    let nudges = query::nudges(&data, at("2026-09-26T07:50"), at("2026-09-26T07:56"));
    assert!(nudges
        .iter()
        .any(|n| matches!(n, query::Nudge::Confirm { .. })));
}

#[test]
fn change_sets_undo_and_detect_conflicts() {
    let mut data = AgendaData::default();
    let before = data.clone();
    let mut ed = editor(&mut data, "2026-09-25T12:00");
    let id = task(&mut ed, "a");
    let change = ChangeSet::diff(&before, &data);
    assert_eq!(change.len(), 1);
    let mut undone = data.clone();
    change.inverted().apply(&mut undone, true).unwrap();
    assert!(undone.tasks.is_empty());
    // 冲突：提案生成后记录又被改过。
    let mut edited = data.clone();
    Editor::new(&mut edited, Clock::fixed(at("2026-09-25T12:05")))
        .update(&id, &patch(json!({"title": "b"})))
        .unwrap();
    assert!(change.inverted().apply(&mut edited, true).is_err());
    let lines = change.describe(&before, &data);
    assert_eq!(lines, vec!["新建任务「a」".to_owned()]);
}

#[test]
fn planner_fills_free_time_around_existing_entries() {
    let mut data = AgendaData::default();
    data.settings.work_start = "09:00".into();
    data.settings.work_end = "12:00".into();
    let mut ed = editor(&mut data, "2026-09-25T08:00");
    ed.create_entry(EntryDraft {
        title: "组会".into(),
        start: "2026-09-25T10:00".into(),
        end: "2026-09-25T11:00".into(),
        ..Default::default()
    })
    .unwrap();
    let t = ed
        .create_task(TaskDraft {
            title: "写代码".into(),
            due: Some("2026-09-25".into()),
            estimate_minutes: Some(100),
            ..Default::default()
        })
        .unwrap();
    let now = at("2026-09-25T08:00");
    let placed = super::planner::auto_place(&data, now.date(), now, None);
    assert_eq!(placed.len(), 2);
    assert!(placed.iter().all(|p| p.task_id == t));
    assert_eq!(time::stamp(placed[0].start), "2026-09-25T09:00");
    assert_eq!(time::stamp(placed[0].end), "2026-09-25T10:00");
    assert_eq!(time::stamp(placed[1].start), "2026-09-25T11:00");
}

#[test]
fn lanes_split_overlaps() {
    let spans = vec![
        (at("2026-09-25T09:00"), at("2026-09-25T10:00")),
        (at("2026-09-25T09:30"), at("2026-09-25T10:30")),
        (at("2026-09-25T11:00"), at("2026-09-25T12:00")),
    ];
    let lanes = query::assign_lanes(&spans);
    assert_eq!(lanes, vec![(0, 2), (1, 2), (0, 1)]);
}

#[test]
fn store_roundtrip_and_mutate() {
    let dir = std::env::temp_dir().join(format!("agenda-store-{}", crate::paths::random_base36(6)));
    let store = super::AgendaStore::new(&dir);
    let (id, change) = store
        .mutate(super::Source::User, |ed| {
            ed.create_task(TaskDraft {
                title: "x".into(),
                ..Default::default()
            })
        })
        .unwrap();
    assert_eq!(change.len(), 1);
    assert!(store.load().unwrap().task(&id).is_some());
    assert!(store
        .mutate(super::Source::User, |ed| ed
            .update("task-missing", &patch(json!({"title": "y"}))))
        .is_err());
    store
        .apply(super::Source::User, &change.inverted(), true)
        .unwrap();
    assert!(store.load().unwrap().tasks.is_empty());
    assert_eq!(store.history(10).len(), 2);
    std::fs::remove_dir_all(&dir).ok();
}
