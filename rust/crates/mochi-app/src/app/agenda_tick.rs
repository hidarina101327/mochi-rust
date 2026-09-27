//! 定时检查：日程开始提醒、结束后请确认是否执行、任务到截止时间。
use super::*;
use crate::ui::notifications::Category;
use chrono::{Duration, NaiveDateTime};
use mochi_core::agenda::query::{self, Nudge};
use mochi_core::agenda::time as at;

/// 通知的标题和正文。
fn describe(nudge: &Nudge) -> (String, String) {
    match nudge {
        Nudge::Starting {
            title,
            start,
            minutes_before,
            ..
        } => (
            title.clone(),
            if *minutes_before == 0 {
                format!("现在开始 · {}", at::hm(start.time()))
            } else {
                format!("{minutes_before} 分钟后开始 · {}", at::hm(start.time()))
            },
        ),
        Nudge::Confirm { title, end, .. } => (
            format!("做了吗？「{title}」"),
            format!("{} 已结束，到日程待办里确认是否执行", at::hm(end.time())),
        ),
        Nudge::Due { title, .. } => (
            format!("到截止时间了：「{title}」"),
            "任务状态不会自动改变".into(),
        ),
    }
}

impl App {
    pub(super) fn schedule_tick(&mut self) {
        let Some(root) = self.shell.workspace().map(|w| w.root.clone()) else {
            return;
        };
        if self.sched.reminder_workspace.as_ref() != Some(&root) {
            self.sched.reminder_workspace = Some(root);
            self.sched.reminder_cursor = None;
            self.sched.delivered.clear();
        }
        self.agenda_refresh_if_stale();
        let now = at::now();
        self.sched.view.now = now;
        let Some(data) = self.sched.view.data.as_ref() else {
            return;
        };
        let cursor = self
            .sched
            .reminder_cursor
            .unwrap_or(now - Duration::minutes(1));
        let catch_up = now - Duration::minutes(15);
        let mut out: Vec<(String, String)> = Vec::new();
        // 休眠期间错过的提醒合并成一条，既不悄悄丢掉，也不连弹一串。
        if cursor < catch_up {
            let from: NaiveDateTime = cursor.max(now - Duration::hours(24));
            let missed: Vec<Nudge> = query::nudges(data, from, catch_up)
                .into_iter()
                .filter(|n| !self.sched.delivered.contains(&n.key()))
                .collect();
            if !missed.is_empty() {
                let titles = missed
                    .iter()
                    .take(3)
                    .map(|n| format!("「{}」", describe(n).0))
                    .collect::<Vec<_>>()
                    .join("、");
                out.push((format!("错过了 {} 条日程待办提醒", missed.len()), titles));
                for n in &missed {
                    self.sched.delivered.insert(n.key());
                }
            }
        }
        for n in query::nudges(data, cursor.max(catch_up), now) {
            if self.sched.delivered.insert(n.key()) {
                out.push(describe(&n));
            }
        }
        self.sched.reminder_cursor = Some(now);
        if self.sched.delivered.len() > 4096 {
            self.sched.delivered.clear();
        }
        for (title, message) in out {
            self.publish_notification(Category::Schedule, &title, &message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::agenda::{AgendaData, Clock, Editor, TaskDraft};

    #[test]
    fn task_blocks_ask_for_confirmation_after_they_end() {
        let now = at::parse_stamp("2026-09-21T10:00").unwrap();
        let mut data = AgendaData::default();
        let mut ed = Editor::new(&mut data, Clock::fixed(now));
        let task = ed
            .create_task(TaskDraft {
                title: "数学作业".into(),
                ..Default::default()
            })
            .unwrap();
        ed.schedule_task(&task, "2026-09-21T08:00", "2026-09-21T09:00")
            .unwrap();
        let wait = data.settings.confirm_after_minutes as i64;
        let end = at::parse_stamp("2026-09-21T09:00").unwrap() + Duration::minutes(wait);
        let found = query::nudges(&data, end - Duration::seconds(15), end);
        assert_eq!(found.len(), 1);
        let (title, _) = describe(&found[0]);
        assert!(title.contains("数学作业"));
    }
}
