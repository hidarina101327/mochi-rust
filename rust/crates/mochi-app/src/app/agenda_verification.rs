//! 日程待办的快照场景（真实感的示例数据）与端到端行为测试。
use super::*;
use crate::ui::agenda::View;
use mochi_core::agenda::{
    time as at, AgendaStore, EntryDraft, GoalDraft, Kind, ProjectDraft, RoutineDraft, Source,
    TaskDraft, WishDraft,
};

/// 在工作区写入一套示例数据，返回 `(数学作业任务, 今天的独立日程)`。
pub(super) fn seed_agenda(store: &AgendaStore) -> anyhow::Result<(String, String)> {
    let today = at::today();
    let day = |offset: i64, hm: &str| format!("{}T{hm}", at::date_key(at::add_days(today, offset)));
    let (result, _) = store.mutate(Source::System, |ed| {
        let project = ed.create_project(ProjectDraft {
            name: "期末冲刺".into(),
            color: Some("#8B5CF6".into()),
            ..Default::default()
        })?;
        let wish = ed.create_wish(WishDraft {
            title: "成为能独立做项目的开发者".into(),
            note: "不是为了分数，是想做出自己真正会用的东西。".into(),
            ..Default::default()
        })?;
        let math = ed.create_goal(GoalDraft {
            title: "本学期拿下高数 A".into(),
            wish_id: Some(wish.clone()),
            project_id: Some(project.clone()),
            criteria: vec!["期末 ≥ 90 分".into(), "每周做完两套卷".into()],
            target_date: Some(at::date_key(at::add_days(today, 60))),
            ..Default::default()
        })?;
        let books = ed.create_goal(GoalDraft {
            title: "三个月读完 6 本书".into(),
            wish_id: Some(wish),
            ..Default::default()
        })?;
        let homework = ed.create_task(TaskDraft {
            title: "数学作业：第三章习题".into(),
            goal_id: Some(math.clone()),
            project_id: Some(project.clone()),
            priority: Some(mochi_core::agenda::Priority::High),
            due: Some(day(1, "18:00")),
            estimate_minutes: Some(120),
            source: "周二课上布置".into(),
            steps: vec![
                "3.1 节 1–10 题".into(),
                "3.2 节 1–8 题".into(),
                "对答案订正".into(),
            ],
            ..Default::default()
        })?;
        let first_step = ed
            .data
            .task(&homework)
            .map(|t| t.steps[0].id.clone())
            .unwrap_or_default();
        ed.toggle_step(&homework, &first_step)?;
        ed.create_task(TaskDraft {
            title: "整理错题本".into(),
            goal_id: Some(math.clone()),
            due: Some(at::date_key(today)),
            estimate_minutes: Some(45),
            ..Default::default()
        })?;
        ed.create_task(TaskDraft {
            title: "读《原则》第 4 章".into(),
            goal_id: Some(books.clone()),
            planned_for: Some(at::date_key(today)),
            estimate_minutes: Some(60),
            ..Default::default()
        })?;
        ed.create_task(TaskDraft {
            title: "交物理实验报告".into(),
            priority: Some(mochi_core::agenda::Priority::Urgent),
            due: Some(day(-1, "23:59")),
            ..Default::default()
        })?;
        ed.create_task(TaskDraft {
            title: "给导师回邮件".into(),
            ..Default::default()
        })?;
        let past = ed.schedule_task(&homework, &day(0, "08:00"), &day(0, "09:30"))?;
        ed.set_entry_status(&past, mochi_core::agenda::EntryStatus::Done, None)?;
        ed.schedule_task(&homework, &day(1, "19:00"), &day(1, "20:00"))?;
        let meeting = ed.create_entry(EntryDraft {
            title: "组会".into(),
            start: day(0, "14:00"),
            end: day(0, "15:00"),
            location: "B305".into(),
            project_id: Some(project),
            ..Default::default()
        })?;
        ed.create_entry(EntryDraft {
            title: "健身房".into(),
            start: day(0, "18:30"),
            end: day(0, "19:30"),
            color: Some("#10B981".into()),
            ..Default::default()
        })?;
        ed.create_entry(EntryDraft {
            title: "妈妈生日".into(),
            start: at::date_key(today),
            end: at::date_key(today),
            all_day: true,
            ..Default::default()
        })?;
        ed.create_routine(RoutineDraft {
            title: "晨跑 3 公里".into(),
            rule: "FREQ=DAILY".into(),
            start_date: Some(at::date_key(at::add_days(today, -10))),
            time: Some("07:00".into()),
            duration_minutes: Some(30),
            reminders: vec![10],
            ..Default::default()
        })?;
        ed.create_routine(RoutineDraft {
            title: "背单词".into(),
            rule: "FREQ=DAILY".into(),
            start_date: Some(at::date_key(at::add_days(today, -10))),
            time: Some("21:30".into()),
            duration_minutes: Some(20),
            goal_id: Some(math),
            ..Default::default()
        })?;
        ed.create_routine(RoutineDraft {
            title: "写周报".into(),
            rule: "FREQ=WEEKLY;BYDAY=FR".into(),
            start_date: Some(at::date_key(at::add_days(today, -30))),
            time: Some("17:00".into()),
            duration_minutes: Some(30),
            ..Default::default()
        })?;
        Ok((homework, meeting))
    })?;
    Ok(result)
}

impl App {
    /// `schedule[-视图][-面板][-narrow]` 场景。
    pub(super) fn prepare_schedule_snapshot(&mut self, scenario: &str) -> anyhow::Result<()> {
        self.state.view = WorkspaceView::Schedule;
        let store = self
            .agenda_store()
            .ok_or_else(|| anyhow::anyhow!("没有工作区"))?;
        let (homework, meeting) = seed_agenda(&store)?;
        self.reload_schedule();
        let st = &mut self.sched.view;
        let view = [
            ("week", View::Day),
            ("month", View::Month),
            ("todos", View::Todos),
            ("confirm", View::Confirm),
            ("goals", View::Goals),
            ("routines", View::Routines),
            ("projects", View::Projects),
            ("trash", View::Trash),
            ("history", View::History),
        ]
        .into_iter()
        .find(|(k, _)| scenario.contains(k))
        .map_or(View::Day, |(_, v)| v);
        st.set_view(view);
        st.week = scenario.contains("week");
        st.todo_month = scenario.contains("todos-month");
        if view == View::History {
            st.history = store.history(50);
        }
        if scenario.contains("panel-task") {
            self.agenda_open_in_place(&homework);
        } else if scenario.contains("panel-entry") {
            self.agenda_open_in_place(&meeting);
        } else if scenario.contains("panel-goal") {
            let goal = self
                .sched
                .view
                .data
                .as_ref()
                .and_then(|d| d.goals.first())
                .map(|g| g.id.clone());
            if let Some(goal) = goal {
                self.agenda_open_in_place(&goal);
            }
        } else if scenario.contains("new") {
            let kind = if scenario.contains("new-task") {
                Kind::Task
            } else {
                Kind::Entry
            };
            self.agenda_new(kind, None, None);
        }
        if scenario.contains("plan") {
            let data = self.sched.view.data.as_ref().unwrap();
            self.sched.view.plan = Some(mochi_core::agenda::planner::auto_place(
                data,
                self.sched.view.date,
                at::now(),
                None,
            ));
        }
        if scenario.contains("quick") {
            self.focus = Focus::ScheduleQuick;
            self.sched.view.quick.set_text("明天 9 点 写周报 1h #工作");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::agenda::{self, panel};
    use mochi_core::agenda::{EntryStatus, TaskStatus};

    fn test_app(tag: &str) -> (App, PathBuf) {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-agenda-{tag}-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            root.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&root, || {}).unwrap();
        app.state.view = WorkspaceView::Schedule;
        app.reload_schedule();
        (app, root)
    }

    fn layout(app: &mut App) {
        let palette = crate::ui::theme::configured_palette(false);
        let mut list = DrawList::default();
        app.sched.layout = agenda::paint(
            &mut list,
            Rect::new(0.0, 0.0, 1400.0, 900.0),
            &mut app.sched.view,
            None,
            &palette,
        );
    }

    fn center(r: Rect) -> (f32, f32) {
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    #[test]
    fn dragging_a_block_moves_it_on_disk_and_undo_restores_it() {
        let (mut app, root) = test_app("drag");
        let date = at::add_days(at::today(), 1);
        let start = date.and_hms_opt(10, 0, 0).unwrap();
        let id = app
            .agenda_mutate(None, |ed| {
                ed.create_entry(EntryDraft {
                    title: "拖拽验证".into(),
                    start: at::stamp(start),
                    end: at::stamp(start + chrono::Duration::hours(1)),
                    ..Default::default()
                })
            })
            .unwrap();
        app.sched.view.goto_day(date);
        app.sched.view.scroll = 8.0 * agenda::day::HOUR_H;
        layout(&mut app);
        let block = app
            .sched
            .layout
            .rect_of(&agenda::Hit::Slot(id.clone()))
            .unwrap();
        let (x, _) = center(block);
        let y = block.top + 12.0;
        app.on_schedule_click(x, y);
        assert!(app.sched.view.drag.is_some());
        app.schedule_drag_to(x, y + agenda::day::HOUR_H);
        app.finish_schedule_drag();
        let store = app.agenda_store().unwrap();
        let moved = store.load().unwrap().entry(&id).unwrap().start.clone();
        assert_eq!(
            at::parse_stamp(&moved),
            Some(start + chrono::Duration::hours(1))
        );
        app.schedule_undo();
        let restored = store.load().unwrap().entry(&id).unwrap().start.clone();
        assert_eq!(at::parse_stamp(&restored), Some(start));
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dragging_a_todo_into_the_timeline_only_adds_a_block() {
        let (mut app, root) = test_app("todo-drag");
        let today = at::today();
        let task = app
            .agenda_mutate(None, |ed| {
                ed.create_task(TaskDraft {
                    title: "数学作业".into(),
                    due: Some(at::date_key(today)),
                    estimate_minutes: Some(60),
                    ..Default::default()
                })
            })
            .unwrap();
        app.sched.view.scroll = 8.0 * agenda::day::HOUR_H;
        layout(&mut app);
        let card = app
            .sched
            .layout
            .rect_of(&agenda::Hit::Todo(task.clone()))
            .unwrap();
        let geo = app.sched.layout.geo.clone().unwrap();
        let (x, y) = center(card);
        app.on_schedule_click(x, y);
        let target_x = (geo.days[0].1 + geo.days[0].2) / 2.0;
        app.schedule_drag_to(target_x, geo.y_of(15 * 60));
        app.finish_schedule_drag();
        let data = app.agenda_store().unwrap().load().unwrap();
        let blocks = mochi_core::agenda::query::task_blocks(&data, &task);
        assert_eq!(blocks.len(), 1);
        let t = data.task(&task).unwrap();
        assert_eq!(t.status, TaskStatus::Todo, "排时间不改任务状态");
        assert_eq!(
            t.due.as_deref(),
            Some(at::date_key(today).as_str()),
            "排时间不改截止"
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn completing_a_task_asks_about_future_blocks_and_keeps_them_by_default() {
        let (mut app, root) = test_app("complete");
        let tomorrow = at::add_days(at::today(), 1);
        let s = tomorrow.and_hms_opt(19, 0, 0).unwrap();
        let task = app
            .agenda_mutate(None, |ed| {
                let t = ed.create_task(TaskDraft {
                    title: "数学作业".into(),
                    ..Default::default()
                })?;
                ed.schedule_task(
                    &t,
                    &at::stamp(s),
                    &at::stamp(s + chrono::Duration::hours(1)),
                )?;
                Ok(t)
            })
            .unwrap();
        app.agenda_toggle_task(&task);
        let prompt = app
            .sched
            .view
            .panel
            .as_ref()
            .and_then(|p| p.prompt.clone())
            .expect("应该询问");
        assert_eq!(prompt.entry_ids.len(), 1);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert_eq!(
            data.entry(&prompt.entry_ids[0]).unwrap().status,
            EntryStatus::Planned,
            "默认保留"
        );
        app.agenda_panel_hit(panel::Hit::CancelFuture, 0.0, 0.0);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert_eq!(
            data.entry(&prompt.entry_ids[0]).unwrap().status,
            EntryStatus::Cancelled
        );
        // 重新打开任务不会恢复已取消的块。
        app.agenda_toggle_task(&task);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert!(data.task(&task).unwrap().status.is_open());
        assert_eq!(
            data.entry(&prompt.entry_ids[0]).unwrap().status,
            EntryStatus::Cancelled
        );
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirming_a_block_does_not_complete_its_task() {
        let (mut app, root) = test_app("confirm");
        let s = at::now() - chrono::Duration::hours(3);
        let (task, entry) = app
            .agenda_mutate(None, |ed| {
                let t = ed.create_task(TaskDraft {
                    title: "写报告".into(),
                    ..Default::default()
                })?;
                let e = ed.schedule_task(
                    &t,
                    &at::stamp(s),
                    &at::stamp(s + chrono::Duration::hours(1)),
                )?;
                Ok((t, e))
            })
            .unwrap();
        app.agenda_confirm(&entry, EntryStatus::Done);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert_eq!(data.entry(&entry).unwrap().status, EntryStatus::Done);
        assert!(data.task(&task).unwrap().status.is_open());
        app.agenda_complete_block_and_task(&entry);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert_eq!(data.task(&task).unwrap().status, TaskStatus::Done);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deleting_a_task_keeps_its_blocks_and_asks_about_future_ones() {
        let (mut app, root) = test_app("delete");
        let s = at::add_days(at::today(), 2).and_hms_opt(9, 0, 0).unwrap();
        let (task, entry) = app
            .agenda_mutate(None, |ed| {
                let t = ed.create_task(TaskDraft {
                    title: "旧任务".into(),
                    ..Default::default()
                })?;
                let e = ed.schedule_task(
                    &t,
                    &at::stamp(s),
                    &at::stamp(s + chrono::Duration::hours(1)),
                )?;
                Ok((t, e))
            })
            .unwrap();
        app.agenda_delete(&task);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert!(
            data.task(&task).unwrap().deleted_at.is_some(),
            "删除保留可追溯记录"
        );
        assert_eq!(data.entry(&entry).unwrap().status, EntryStatus::Planned);
        assert!(data.entry_task_deleted(data.entry(&entry).unwrap()));
        assert!(app
            .sched
            .view
            .panel
            .as_ref()
            .and_then(|p| p.prompt.as_ref())
            .is_some());
        app.agenda_panel_hit(panel::Hit::KeepFuture, 0.0, 0.0);
        let data = app.agenda_store().unwrap().load().unwrap();
        assert_eq!(data.entry(&entry).unwrap().status, EntryStatus::Planned);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quick_add_understands_entries_and_tasks() {
        let (mut app, root) = test_app("quick");
        app.sched.view.quick.set_text("明天 9 点 写周报 1h #工作");
        app.submit_quick_add();
        app.sched.view.quick.set_text("任务：读完第三章 周五");
        app.submit_quick_add();
        let data = app.agenda_store().unwrap().load().unwrap();
        let entry = data
            .entries
            .iter()
            .find(|e| e.title == "写周报")
            .expect("日程");
        let tomorrow = at::add_days(at::today(), 1);
        assert_eq!(entry.start, format!("{}T09:00", at::date_key(tomorrow)));
        assert_eq!(entry.end, format!("{}T10:00", at::date_key(tomorrow)));
        let task = data
            .tasks
            .iter()
            .find(|t| t.title == "读完第三章")
            .expect("任务");
        assert!(task.due.is_some());
        assert!(app.sched.view.quick.is_empty());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_undo_reports_the_conflict_without_overwriting() {
        let (mut app, root) = test_app("undo-conflict");
        let id = app
            .agenda_mutate(None, |ed| {
                ed.create_wish(WishDraft {
                    title: "最初标题".into(),
                    ..Default::default()
                })
            })
            .unwrap();
        let store = app.agenda_store().unwrap();
        store
            .mutate(Source::Ai, |ed| {
                let mut patch = serde_json::Map::new();
                patch.insert("title".into(), serde_json::json!("外部修改"));
                ed.update(&id, &patch)
            })
            .unwrap();
        app.schedule_undo();
        assert!(!app.sched.view.error.is_empty());
        assert_eq!(store.load().unwrap().wish(&id).unwrap().title, "外部修改");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn panel_edits_save_only_changed_fields() {
        let (mut app, root) = test_app("panel");
        let (homework, _) = seed_agenda(&app.agenda_store().unwrap()).unwrap();
        app.reload_schedule();
        app.agenda_open_in_place(&homework);
        {
            let pn = app.sched.view.panel.as_mut().unwrap();
            pn.field_mut(panel::Field::Title)
                .unwrap()
                .set_text("数学作业：第三、四章");
        }
        app.agenda_commit_panel();
        let data = app.agenda_store().unwrap().load().unwrap();
        let t = data.task(&homework).unwrap();
        assert_eq!(t.title, "数学作业：第三、四章");
        assert_eq!(t.estimate_minutes, Some(120));
        assert!(!app.sched.view.panel.as_ref().unwrap().dirty());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
