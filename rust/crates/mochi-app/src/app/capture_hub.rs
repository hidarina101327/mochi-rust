//! 打开快速收集面板，并处理收集条目的查看操作。
use super::*;
use crate::capture_window::{Action, Entry};

impl App {
    pub fn show_capture(&mut self) {
        if self.capture.as_ref().is_some_and(|c| c.hide_if_visible()) {
            return;
        }
        let root = self.shell.workspace().map(|ws| ws.root.clone());
        let mut pages: [Vec<Entry>; 5] = Default::default();
        if let Some(root) = &root {
            self.reload_recent();
            self.reload_favorites();
            pages[1] = self
                .views
                .recent
                .docs
                .iter()
                .map(|d| Entry {
                    title: d.title.clone(),
                    detail: d.parent.clone(),
                    action: Action::File(d.path.clone()),
                })
                .collect();
            pages[2] = self
                .views
                .favorites
                .docs
                .iter()
                .map(|d| Entry {
                    title: d.title.clone(),
                    detail: d.parent.clone(),
                    action: Action::File(d.path.clone()),
                })
                .collect();
            self.reload_schedule();
            if let Some(data) = &self.sched.view.data {
                use mochi_core::agenda::{query, time as at};
                let now = at::now();
                let today = now.date();
                let mut rows: Vec<(String, Entry)> = Vec::new();
                for slot in query::slots_between(data, today, at::add_days(today, 7), now) {
                    if slot.status != mochi_core::agenda::EntryStatus::Planned || slot.end < now {
                        continue;
                    }
                    let when = if slot.all_day {
                        at::relative_date_label(slot.start.date(), today)
                    } else {
                        format!(
                            "{} {}",
                            at::relative_date_label(slot.start.date(), today),
                            at::hm(slot.start.time())
                        )
                    };
                    rows.push((
                        at::stamp(slot.start),
                        Entry {
                            title: slot.title.clone(),
                            detail: when,
                            action: Action::Schedule(slot.key.clone()),
                        },
                    ));
                }
                for task in data
                    .tasks
                    .iter()
                    .filter(|t| query::is_visible_task(t) && t.status.is_open())
                {
                    let detail = task
                        .due
                        .as_deref()
                        .map(|d| format!("截止 {}", at::due_label(d, today)))
                        .unwrap_or_else(|| "待办".into());
                    let order = task.due.clone().unwrap_or_else(|| "9999".into());
                    rows.push((
                        order,
                        Entry {
                            title: task.title.clone(),
                            detail,
                            action: Action::Schedule(task.id.clone()),
                        },
                    ));
                }
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                pages[3] = rows.into_iter().map(|(_, e)| e).collect();
            }
            let mut sessions = AiSessionService::new(root).load_index().sessions;
            sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            pages[4] = sessions
                .into_iter()
                .map(|s| Entry {
                    title: s.title,
                    detail: format!("{} 条消息", s.message_count),
                    action: Action::Chat(s.id),
                })
                .collect();
        }
        if let Some(capture) = &self.capture {
            capture.show(root, self.state.dark, pages);
        }
    }

    pub fn open_capture_item(&mut self) {
        let Some(action) = self.capture.as_ref().and_then(|c| c.take_action()) else {
            return;
        };
        unsafe {
            let hwnd = HWND(self.hwnd_raw as *mut _);
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows::Win32::UI::WindowsAndMessaging::SW_RESTORE,
            );
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
        }
        match action {
            Action::File(path) => {
                if self.open_file_from_ui(&path) {
                    self.state.view = WorkspaceView::Editor;
                    self.sync_state();
                }
            }
            Action::Chat(id) => {
                self.state.view = WorkspaceView::MochiAi;
                self.ai_open_session(&id);
            }
            Action::Schedule(id) => {
                self.agenda_open(&id);
            }
            Action::Page(page) => match page {
                1 => self.state.view = WorkspaceView::Recent,
                2 => self.open_favorites(),
                3 => self.state.view = WorkspaceView::Schedule,
                4 => {
                    self.state.view = WorkspaceView::MochiAi;
                    self.ai_new_session();
                }
                _ => self.state.view = WorkspaceView::Inbox,
            },
        }
        self.invalidate_main();
    }
}
