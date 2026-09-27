//! 加载并交互操作首页仪表板，包括当天日记和快捷入口。
use super::*;
use crate::ui::home;

impl App {
    /// 单帧、隔离的设计样例。不读用户工作区，也不跑首页/收藏的多截图验证。
    pub(super) fn prepare_home_design_preview(&mut self, folder: &Path) {
        let now = chrono::Local::now();
        let mut analytics = mochi_core::analytics::empty_snapshot(365, now);
        analytics.today.day.net_words = 1248;
        analytics.today.day.words_added = 1248;
        analytics.today.day.notes_edited = 6;
        analytics.today.day.ai_messages = 12;
        analytics.today.day.focus_minutes = 45;
        let recent: Vec<_> = [
            ("把零散的想法，整理成自己的知识", "知识库 / 思考与写作"),
            ("考研英语 7500 词", "知识库 / 英语 / 单词库"),
            ("产品设计周记 · 让工具回归内容", "项目 / Mochi"),
            ("Rust 学习笔记：所有权与借用", "知识库 / 技术"),
            ("九月阅读清单", "生活 / 阅读"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (title, subtitle))| home::DashboardDocument {
            path: folder.join(format!("sample-{i}.md")),
            title: title.into(),
            subtitle: subtitle.into(),
            mtime_ms: now.timestamp_millis() - (i as i64 + 1) * 3_600_000,
            favorite: i == 0,
        })
        .collect();
        self.home.set_dashboard(home::Dashboard {
            favorite_documents: vec![recent[0].clone(), recent[2].clone()],
            recent_documents: recent,
            schedule_items: [
                ("09:30", "整理阅读笔记", "done", "normal"),
                ("14:00", "梳理新版本的首页体验", "todo", "high"),
                ("17:30", "复习今日词汇", "todo", "normal"),
            ]
            .into_iter()
            .enumerate()
            .map(
                |(i, (time, title, status, priority))| home::DashboardScheduleItem {
                    id: i.to_string(),
                    title: title.into(),
                    time_label: time.into(),
                    status: status.into(),
                    priority: priority.into(),
                    kind: "task".into(),
                },
            )
            .collect(),
            inbox_items: vec![home::DashboardInboxItem {
                id: "draft".into(),
                content: "试着用自己的话解释一个刚学到的概念。".into(),
                created_at_ms: now.timestamp_millis() - 1_800_000,
            }],
            libraries: vec![home::DashboardLibrary {
                id: "knowledge".into(),
                name: "我的知识库".into(),
                type_name: "知识库".into(),
            }],
            journal: home::DashboardJournal {
                path: folder.join("journal.md"),
                date: now.format("%Y-%m-%d").to_string(),
                excerpt: "给今天留下一点值得记住的东西。".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        self.home.set_analytics(analytics);
        self.home_dirty = false;
        self.state.view = WorkspaceView::Home;
        self.state.ai_panel_open = false;
        self.focus = Focus::Main;
    }

    pub(super) fn reload_home_dashboard(&mut self) {
        if !self.shell.has_workspace() {
            self.home.set_dashboard(home::Dashboard::default());
            return;
        }
        self.reload_recent();
        self.reload_favorites();
        let ws = self.shell.workspace().unwrap();
        let document = |doc: &views::recent::Doc| home::DashboardDocument {
            path: doc.path.clone(),
            title: doc.title.clone(),
            subtitle: doc.parent.clone(),
            mtime_ms: doc.mtime_ms,
            favorite: self.shell.is_favorite(&doc.path),
        };
        let mut sessions = AiSessionService::new(&ws.root).load_index().sessions;
        sessions.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        let mut capture = CaptureService::new(&ws.root).list_items("inbox");
        capture.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let mut dashboard = home::Dashboard {
            recent_documents: self
                .views
                .recent
                .docs
                .iter()
                .take(5)
                .map(document)
                .collect(),
            favorite_documents: self
                .views
                .favorites
                .docs
                .iter()
                .take(5)
                .map(document)
                .collect(),
            inbox_items: capture
                .into_iter()
                .map(|item| home::DashboardInboxItem {
                    id: item.id,
                    content: item.content,
                    created_at_ms: item.created_at,
                })
                .collect(),
            libraries: ws
                .libraries
                .iter()
                .filter(|lib| lib.kind != navigation::HIDDEN_TYPE_ID)
                .map(|lib| home::DashboardLibrary {
                    id: lib.id.clone(),
                    name: lib.name.clone(),
                    type_name: ws
                        .library_types
                        .iter()
                        .find(|kind| kind.id == lib.kind)
                        .map(|kind| kind.name.clone())
                        .unwrap_or_else(|| "知识库".into()),
                })
                .collect(),
            ai_sessions: sessions
                .into_iter()
                .take(4)
                .map(|session| home::DashboardSession {
                    id: session.id,
                    title: session.title,
                    updated_at_ms: session.updated_at,
                    message_count: session.message_count.max(0) as usize,
                    pinned: session.pinned,
                })
                .collect(),
            ..Default::default()
        };
        if let Some(analytics) = self.home.analytics() {
            dashboard.graph = home::DashboardGraph {
                local_link_count: analytics.graph.local_link_count,
                nodes: analytics
                    .graph
                    .central_nodes
                    .iter()
                    .map(|node| home::DashboardGraphNode {
                        path: ws.root.join(&node.path),
                        title: node.title.clone(),
                        link_count: node.link_count,
                    })
                    .collect(),
            };
        }
        let today = chrono::Local::now().date_naive();
        if let Some(store) = self.agenda_store() {
            if let Ok(data) = store.load() {
                use mochi_core::agenda::{planner, query, time as at, EntryStatus, TaskStatus};
                let now = at::now();
                let slots: Vec<query::Slot> = query::day_slots(&data, today, now)
                    .into_iter()
                    .filter(|s| s.status != EntryStatus::Cancelled)
                    .collect();
                let mut items: Vec<home::DashboardScheduleItem> = slots
                    .iter()
                    .map(|s| home::DashboardScheduleItem {
                        id: s.key.clone(),
                        title: s.title.clone(),
                        time_label: if s.all_day {
                            "全天".into()
                        } else {
                            at::hm(s.start.time())
                        },
                        status: match s.status {
                            EntryStatus::Done => "done",
                            EntryStatus::Skipped => "cancelled",
                            _ if s.start <= now && now < s.end => "in_progress",
                            _ => "pending",
                        }
                        .into(),
                        kind: if s.routine_id.is_some() {
                            "重复"
                        } else {
                            "日程"
                        }
                        .into(),
                        priority: "normal".into(),
                    })
                    .collect();
                let todos = query::day_todos(&data, today, now);
                let ids = todos
                    .overdue
                    .iter()
                    .chain(&todos.doing)
                    .chain(&todos.due)
                    .chain(&todos.planned);
                let mut seen = std::collections::HashSet::new();
                for task in ids
                    .filter(|id| seen.insert(id.as_str()))
                    .filter_map(|id| data.task(id))
                {
                    items.push(home::DashboardScheduleItem {
                        id: task.id.clone(),
                        title: task.title.clone(),
                        time_label: if query::is_overdue(task, now) {
                            "已逾期".into()
                        } else {
                            "今天".into()
                        },
                        status: match task.status {
                            TaskStatus::Done => "done",
                            TaskStatus::Doing => "in_progress",
                            TaskStatus::Cancelled => "cancelled",
                            TaskStatus::Todo => "pending",
                        }
                        .into(),
                        kind: "task".into(),
                        priority: task.priority.wire().into(),
                    });
                }
                dashboard.schedule_items = items;
                let spans: Vec<_> = slots
                    .iter()
                    .filter(|s| !s.all_day)
                    .map(|s| (s.start, s.end))
                    .collect();
                dashboard.schedule_conflict_count = query::assign_lanes(&spans)
                    .iter()
                    .filter(|(_, lanes)| *lanes > 1)
                    .count();
                let (busy, capacity) = planner::day_load(&data, today, now);
                let ratio = busy as f32 / capacity.max(1) as f32;
                dashboard.schedule_risk = if ratio > 0.85 {
                    "high"
                } else if ratio > 0.6 {
                    "medium"
                } else {
                    "low"
                }
                .to_owned();
            }
        }
        if let Some(path) = self.today_journal_path() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            dashboard.journal = home::DashboardJournal {
                exists: path.is_file(),
                path,
                date: today.to_string(),
                words: mochi_core::word_count::count_words(&content),
                excerpt: content
                    .lines()
                    .filter(|line| !line.trim().is_empty() && !line.starts_with("# "))
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(140)
                    .collect(),
            };
        }
        self.home.set_dashboard(dashboard);
    }

    pub(super) fn on_home_click(&mut self, x: f32, y: f32) {
        let Some(action) = self.home.hit(self.editor_area, x, y) else {
            self.home.clear_keyboard_focus();
            return;
        };
        self.home.clear_keyboard_focus();
        self.run_home_action(action);
    }

    pub(super) fn run_home_action(&mut self, action: home::Action) {
        use home::Action;
        self.focus = Focus::Main;
        match action {
            Action::NewNote => {
                self.open_knowledge();
                self.run_command_action(CommandAction::NewFile);
            }
            Action::NewJournal | Action::OpenJournal => self.open_today_journal(),
            Action::ImportFile => {
                self.open_knowledge();
                self.run_menu_action(MenuAction::ImportFiles(PathBuf::new()));
            }
            Action::OpenAi => {
                self.state.view = WorkspaceView::MochiAi;
                self.focus = Focus::AiInput;
            }
            Action::Capture => self.show_capture(),
            Action::OpenRecentView => {
                self.state.view = WorkspaceView::Recent;
                self.reload_recent();
            }
            Action::OpenFavoritesView => self.open_favorites(),
            Action::OpenInbox => {
                self.state.view = WorkspaceView::Inbox;
                self.reload_inbox();
            }
            Action::OpenSchedule => {
                self.state.view = WorkspaceView::Schedule;
                self.reload_schedule();
                self.sched.view.query.clear();
                self.sched.view.set_view(crate::ui::agenda::View::Day);
                self.sched.view.goto_today();
            }
            Action::OpenRecent(i) | Action::OpenFavorite(i) | Action::OpenGraphNode(i) => {
                let dashboard = self.home.dashboard();
                let path = match action {
                    Action::OpenRecent(_) => {
                        dashboard.recent_documents.get(i).map(|d| d.path.clone())
                    }
                    Action::OpenFavorite(_) => {
                        dashboard.favorite_documents.get(i).map(|d| d.path.clone())
                    }
                    _ => dashboard.graph.nodes.get(i).map(|d| d.path.clone()),
                };
                if let Some(path) = path {
                    self.open_home_document(&path);
                }
            }
            Action::OpenLibrary(index) => {
                let id = self
                    .home
                    .dashboard()
                    .libraries
                    .get(index)
                    .map(|lib| lib.id.clone());
                if let Some(index) = id.and_then(|id| {
                    self.shell
                        .workspace()?
                        .libraries
                        .iter()
                        .position(|lib| lib.id == id)
                }) {
                    self.shell.select_library(index);
                    self.state.view = WorkspaceView::Editor;
                    self.invalidate_main();
                }
            }
            Action::OpenAiSession(id) => {
                self.ai_open_session(&id);
                self.state.view = WorkspaceView::MochiAi;
                self.focus = Focus::AiInput;
            }
        }
        self.sync_state();
    }

    pub(super) fn today_journal_path(&self) -> Option<PathBuf> {
        let root = &self.shell.workspace()?.root;
        let filename = format!("{}.md", chrono::Local::now().format("%Y-%m-%d"));
        let path = root.join("日记").join(&filename);
        // 更早的原生版本把日记文件夹当成一个遗留知识库。
        let migrated = root.join("知识库/日记").join(filename);
        Some(if !path.exists() && migrated.exists() {
            migrated
        } else {
            path
        })
    }

    pub(super) fn open_today_journal(&mut self) {
        let Some(path) = self.today_journal_path() else {
            return;
        };
        let result = (|| -> anyhow::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(path.parent().unwrap())?;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    write!(file, "# {}\n\n", chrono::Local::now().format("%Y-%m-%d"))?;
                    if let Some(ws) = self.shell.workspace() {
                        let _ = ws.index.index_single_file(&path);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.shell.refresh_tree();
                self.home_dirty = true;
                self.open_home_document(&path);
            }
            Err(error) => self.state.status_text = format!("无法打开今日日记：{error}"),
        }
    }

    /// 首页入口打开文件时遵循用户的标签页策略；Ctrl 覆盖为追加打开。
    /// 收藏页状态和工作区选择沿用收藏/首页入口的既有语义。
    fn open_home_document(&mut self, path: &std::path::Path) {
        if self.shell.favorites_selected() {
            self.side = SidebarState::default();
            self.outline_left_active = false;
        }
        self.shell.leave_favorites();
        let library = self.shell.workspace().and_then(|ws| {
            ws.libraries
                .iter()
                .enumerate()
                .filter(|(_, lib)| path.starts_with(&lib.path))
                .max_by_key(|(_, lib)| lib.path.len())
                .map(|(index, _)| index)
        });
        if let Some(index) = library {
            self.shell.select_library(index);
        }
        if self.open_file_from_ui(path) {
            self.state.view = WorkspaceView::Editor;
            self.focus = Focus::Main;
            self.invalidate_main();
            self.sync_state();
        } else {
            self.state.status_text = self.shell.status().to_owned();
            self.reload_favorites();
        }
    }
}

#[cfg(test)]
mod design_tests {
    use super::*;

    #[test]
    fn home_shortcuts_and_pointer_route_to_the_existing_controller_actions() {
        let root = std::env::temp_dir().join(format!(
            "mochi-home-design-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.prepare_home_design_preview(&root);
            app.editor_area = Rect::from_size(0.0, 0.0, 1200.0, 740.0);
            let page = home::layout_with_dashboard(
                app.home.analytics(),
                app.home.dashboard(),
                app.editor_area,
            );
            let first = page.next_action(None, false).unwrap();
            let rect = page.action_rect(first).unwrap();
            assert_eq!(
                app.cursor_for(rect.left + 8.0, rect.top + 8.0),
                Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND)
            );
            assert!(app.on_mouse_move(rect.left + 8.0, rect.top + 8.0));
            assert!(app.on_edit_key(0x09, false, false));
            assert_eq!(app.home.focused_action(), Some(home::Action::NewNote));
            assert!(app.on_edit_key(0x09, false, false));
            assert_eq!(app.home.focused_action(), Some(home::Action::OpenAi));
            assert!(app.on_edit_key(0x09, true, false));
            assert_eq!(app.home.focused_action(), Some(home::Action::NewNote));
            assert!(app.on_edit_key(0x1B, false, false));
            assert!(app.home.focused_action().is_none());
            app.on_edit_key(0x09, false, false);
            app.on_edit_key(0x09, false, false);
            assert!(app.on_edit_key(0x0D, false, false));
            assert_eq!(app.state.view, WorkspaceView::MochiAi);
            assert_eq!(app.focus, Focus::AiInput);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
