//! 桌面卡片的有界只读数据提供器。

use super::{
    page_key, validate_relative_path, Action, Module, Page, PageSnapshot, Row, Snapshot,
    MAX_ROWS_PER_PAGE,
};
use crate::agenda::{query, time as agenda_time, AgendaData, AgendaStore};
use crate::ai::session::AiSessionIndex;
use crate::capture::{CaptureFile, CaptureItem};
use crate::domain::Library;
use chrono::Local;
use rusqlite::{params, Connection, OpenFlags};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct SnapshotLimits {
    pub max_cards: usize,
    pub max_rows: usize,
    pub max_read_bytes: usize,
    pub max_dir_entries: usize,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_cards: super::MAX_CARDS,
            max_rows: usize::MAX,
            max_read_bytes: 16 * 1024 * 1024,
            max_dir_entries: usize::MAX,
        }
    }
}

pub struct SnapshotBuilder {
    workspace: PathBuf,
    limits: SnapshotLimits,
    read_state: Mutex<ReadState>,
}

#[derive(Default)]
struct ReadState {
    bytes: usize,
    dir_entries: usize,
    folder_entries: usize,
    remaining_folder_pages: usize,
    cache: HashMap<PathBuf, Option<Vec<u8>>>,
    errors: Vec<String>,
}

impl SnapshotBuilder {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            limits: SnapshotLimits::default(),
            read_state: Mutex::new(ReadState::default()),
        }
    }

    pub fn with_limits(workspace: impl AsRef<Path>, limits: SnapshotLimits) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
            limits,
            read_state: Mutex::new(ReadState::default()),
        }
    }

    pub fn build(&self, config: &super::DesktopConfig) -> anyhow::Result<Snapshot> {
        if let Ok(mut state) = self.read_state.lock() {
            state.bytes = 0;
            state.dir_entries = 0;
            state.folder_entries = 0;
            state.remaining_folder_pages = 0;
            state.cache.clear();
            state.errors.clear();
        }
        let mut snapshot = Snapshot {
            generated_at: crate::jstime::now(),
            pages: std::collections::BTreeMap::new(),
        };
        config.validate()?;
        let enabled_cards = config
            .cards
            .iter()
            .filter(|card| card.enabled)
            .take(self.limits.max_cards.min(super::MAX_CARDS));
        let folder_pages = enabled_cards
            .clone()
            .filter_map(|card| card.active_page())
            .filter(|page| page.module == Module::Folder && !page.folder.path.is_empty())
            .count();
        if let Ok(mut state) = self.read_state.lock() {
            state.remaining_folder_pages = folder_pages;
        }
        for card in enabled_cards {
            let Some(page) = card.active_page() else {
                continue;
            };
            let data = self.build_page(page);
            snapshot.pages.insert(page_key(&card.id, &page.id), data);
            for node in &page.studio.nodes {
                if node.kind == super::studio::Kind::Data {
                    if let Some(module) = Module::from_wire(&node.target).filter(|m| {
                        !matches!(
                            m,
                            Module::Custom | Module::Clock | Module::Shortcuts | Module::Folder
                        )
                    }) {
                        let mut source = Page::new(module);
                        source.limit = 0;
                        let data = self.build_page(&source);
                        snapshot.pages.insert(
                            format!("{}:node:{}", page_key(&card.id, &page.id), node.id),
                            data,
                        );
                    }
                }
            }
        }
        Ok(snapshot)
    }

    fn build_page(&self, page: &Page) -> PageSnapshot {
        if page.module == Module::Folder {
            return self.folder_snapshot(page);
        }
        if page.options.is_empty() {
            let mut snapshot = PageSnapshot::empty(page);
            snapshot.empty_message = "未选择展示内容".into();
            return snapshot;
        }
        if matches!(page.module, Module::Automations | Module::Pomodoro) {
            let Some((read_limit, entry_limit)) = self.source_limits(page) else {
                let mut snapshot = PageSnapshot::empty(page);
                snapshot.empty_message = "读取失败：本次刷新来源预算已用尽".into();
                return snapshot;
            };
            let mut snapshot = match super::extra_provider::extra_snapshot(
                &self.workspace,
                page,
                read_limit,
                entry_limit,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    let mut snapshot = PageSnapshot::empty(page);
                    snapshot.empty_message = format!("读取失败：{error}");
                    snapshot
                }
            };
            snapshot.rows.truncate(if page.limit == 0 {
                self.limits.max_rows
            } else {
                page.limit.min(self.limits.max_rows)
            });
            if snapshot.rows.is_empty() && snapshot.empty_message.is_empty() {
                snapshot.empty_message = format!("{}暂无可展示内容", page.title);
            }
            return snapshot;
        }
        if page.source.is_some()
            && !matches!(
                page.module,
                Module::Knowledge | Module::Base | Module::Canvas | Module::Exam | Module::Document
            )
        {
            let Some((source_read_limit, source_entry_limit)) = self.source_limits(page) else {
                let mut snapshot = PageSnapshot::empty(page);
                snapshot.empty_message = "读取失败：本次刷新来源预算已用尽".into();
                return snapshot;
            };
            let mut snapshot = match super::source_provider::source_snapshot(
                &self.workspace,
                page,
                source_read_limit,
                source_entry_limit,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    let mut snapshot = PageSnapshot::empty(page);
                    snapshot.empty_message = format!("读取失败：{error}");
                    snapshot
                }
            };
            snapshot.rows.truncate(if page.limit == 0 {
                self.limits.max_rows
            } else {
                page.limit.min(self.limits.max_rows)
            });
            if snapshot.rows.is_empty() && snapshot.empty_message.is_empty() {
                snapshot.empty_message = format!("{}暂无可展示内容", page.title);
            }
            return snapshot;
        }
        let mut snapshot = PageSnapshot::empty(page);
        let error_start = self
            .read_state
            .lock()
            .map(|state| state.errors.len())
            .unwrap_or(0);
        let mut rows = self.module_rows(page);
        self.decorate_rows(page, &mut rows);
        rows.truncate(if page.limit == 0 {
            self.limits.max_rows
        } else {
            page.limit.min(self.limits.max_rows)
        });
        snapshot.rows = rows;
        let errors = self
            .read_state
            .lock()
            .map(|state| {
                state
                    .errors
                    .iter()
                    .skip(error_start)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !errors.is_empty() {
            if snapshot.rows.is_empty() {
                snapshot.empty_message = format!("读取失败：{}", errors.join("；"));
            } else {
                snapshot.subtitle = format!("{}（部分数据读取失败）", snapshot.subtitle);
            }
        }
        if snapshot.rows.is_empty() {
            if errors.is_empty() {
                snapshot.empty_message = format!("{}暂无可展示内容", page.title);
            }
        }
        snapshot
    }

    fn folder_snapshot(&self, page: &Page) -> PageSnapshot {
        let reserved = self.reserve_folder_entries(page);
        let result = super::folder::snapshot_with_scan(
            page,
            reserved,
            self.limits.max_rows.min(super::folder::MAX_FOLDER_ROWS),
        );
        let (snapshot, consumed) = match result {
            Ok(result) => (result.snapshot, result.entries_scanned),
            Err(error) => {
                let mut snapshot = PageSnapshot::empty(page);
                snapshot.subtitle = if page.folder.path.is_empty() {
                    "文件夹：未选择".into()
                } else {
                    format!("文件夹：{}", page.folder.path)
                };
                snapshot.subtitle = format!("读取失败：{error} · {}", snapshot.subtitle);
                snapshot.empty_message = format!("读取失败：{error}");
                (snapshot, 0)
            }
        };
        self.release_folder_reservation(reserved, consumed);
        snapshot
    }

    /// Folder pages share the ordinary directory-entry budget and a hard 4096-entry
    /// budget across the whole refresh, so duplicating pages cannot repeat an unbounded scan.
    fn reserve_folder_entries(&self, page: &Page) -> usize {
        if page.folder.path.is_empty() {
            return 0;
        }
        let Ok(mut state) = self.read_state.lock() else {
            return 0;
        };
        let folder_remaining =
            super::folder::MAX_FOLDER_ENTRIES.saturating_sub(state.folder_entries);
        let shared_remaining = self
            .limits
            .max_dir_entries
            .saturating_sub(state.dir_entries);
        let remaining_pages = state.remaining_folder_pages.max(1);
        state.remaining_folder_pages = state.remaining_folder_pages.saturating_sub(1);
        let fair_share = folder_remaining
            .min(shared_remaining)
            .div_ceil(remaining_pages);
        let reserved = fair_share.min(super::folder::MAX_FOLDER_ENTRIES);
        state.folder_entries = state.folder_entries.saturating_add(reserved);
        state.dir_entries = state.dir_entries.saturating_add(reserved);
        reserved
    }

    fn release_folder_reservation(&self, reserved: usize, consumed: usize) {
        let unused = reserved.saturating_sub(consumed.min(reserved));
        if let Ok(mut state) = self.read_state.lock() {
            state.folder_entries = state.folder_entries.saturating_sub(unused);
            state.dir_entries = state.dir_entries.saturating_sub(unused);
        }
    }

    /// 调来源提供方之前，先按最坏情况预留来源开销。
    /// 来源页刻意用自己的解析器；正因为在这里预留，多张指向大型
    /// 文档/资料库的卡片才不会让一次刷新突破共享的字节/目录预算。
    fn source_limits(&self, page: &Page) -> Option<(usize, usize)> {
        let total_bytes = self
            .limits
            .max_read_bytes
            .saturating_mul(super::MAX_CARDS)
            .min(192 * 1024 * 1024);
        let total_dirs = self
            .limits
            .max_dir_entries
            .min(32)
            .saturating_mul(32 * super::MAX_CARDS)
            .min(12288);
        let needs_directory_budget = matches!(page.module, Module::Knowledge | Module::Exam);
        let requested_read = if page.module == Module::Knowledge {
            1
        } else {
            self.limits.max_read_bytes
        };
        let requested_entries = if needs_directory_budget {
            self.limits.max_dir_entries.min(32)
        } else {
            self.limits.max_dir_entries
        };
        let (read_limit, entry_limit) = if let Ok(state) = self.read_state.lock() {
            let remaining_bytes = total_bytes.saturating_sub(state.bytes);
            let read_limit = requested_read.min(remaining_bytes);
            if read_limit == 0 {
                return None;
            }
            if needs_directory_budget {
                let remaining_dirs = total_dirs.saturating_sub(state.dir_entries);
                let entry_limit = requested_entries.min(remaining_dirs / 32);
                if entry_limit == 0 {
                    return None;
                }
                (read_limit, entry_limit)
            } else {
                (read_limit, requested_entries)
            }
        } else {
            return None;
        };
        let reserved_dirs = if needs_directory_budget {
            entry_limit.saturating_mul(32)
        } else {
            0
        };
        if let Ok(mut state) = self.read_state.lock() {
            // 规划锁释放后再复查一次。现在并没有并发构建其他页面，
            // 但这一步会维护好不变量，避免以后其他工作线程复用 builder 时出错。
            if state.bytes.saturating_add(read_limit) > total_bytes
                || state.dir_entries.saturating_add(reserved_dirs) > total_dirs
            {
                return None;
            }
            state.bytes = state.bytes.saturating_add(read_limit);
            state.dir_entries = state.dir_entries.saturating_add(reserved_dirs);
            Some((read_limit, entry_limit))
        } else {
            None
        }
    }

    fn module_rows(&self, page: &Page) -> Vec<Row> {
        match page.module {
            Module::Home => self.home_rows(page),
            Module::Schedule => self.schedule_rows(page),
            Module::Inbox => self.inbox_rows(page),
            Module::QuickNote => self.quick_note_rows(page),
            Module::Recent => self.recent_rows(page),
            Module::Favorites => self.favorite_rows(page),
            Module::Knowledge => self.knowledge_rows(page),
            // 在后台工作线程中，没有明确来源的文档页没有稳定的
            // 「当前文档」身份。这里放一行通用行看起来像能点开点什么，
            // 实际却什么也开不了；想要文档卡片就先配置来源。
            Module::Document => self.collection_rows(page),
            Module::Base => self.collection_rows(page),
            Module::Canvas => self.collection_rows(page),
            Module::QuickNav => self.quick_nav_rows(page),
            Module::Folder | Module::Weather | Module::Music | Module::Search => Vec::new(),
            Module::English => self.english_rows(page),
            // 这些模块在到达这段通用分发之前已被 extra_provider 接管；
            // 将来的代码路径若复用这个助手，兜底也保持无效。
            Module::Exam => self.collection_rows(page),
            Module::Ai => self.ai_rows(page),
            Module::Templates => self.template_rows(page),
            Module::Automations | Module::Pomodoro => Vec::new(),
            Module::AgentConfig => self.agent_rows(page),
            Module::Custom | Module::Clock | Module::Shortcuts => Vec::new(),
        }
    }

    fn home_rows(&self, page: &Page) -> Vec<Row> {
        Module::Home
            .options()
            .into_iter()
            .filter(|entry| page.selected(&entry.key))
            .map(|entry| Row {
                id: format!("entry:{}", entry.key),
                title: entry.label,
                detail: String::new(),
                action: Some(Action::OpenModule(entry.key.clone())),
                checked: None,
                meta: super::RowMeta {
                    icon: entry.key,
                    ..Default::default()
                },
            })
            .collect()
    }

    fn agenda_data(&self) -> AgendaData {
        let store = AgendaStore::new(&self.workspace);
        let path = store.data_path();
        if path.is_file() {
            return self.read_json_path::<AgendaData>(&path).unwrap_or_default();
        }
        // 尚未迁移的旧版日程由存储层一次性迁移。
        store.load().unwrap_or_default()
    }

    /// 日程待办卡片：任务与时间块共用一个列表，勾选即切换完成 / 已执行。
    fn schedule_rows(&self, page: &Page) -> Vec<Row> {
        let data = self.agenda_data();
        let now = agenda_time::now();
        let today = now.date();
        let horizon = agenda_time::add_days(today, 7);
        let hide_done = page.completed_behavior() == super::CompletedBehavior::Hide;
        let mut objects: Vec<(Value, bool, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let want_tasks = ["tasks", "today", "overdue", "upcoming"]
            .iter()
            .any(|key| page.selected(key));
        if want_tasks {
            for task in data.tasks.iter().filter(|t| query::is_visible_task(t)) {
                let due_day = task
                    .due
                    .as_deref()
                    .and_then(agenda_time::Due::parse)
                    .map(|d| d.date());
                let planned = task
                    .planned_for
                    .as_deref()
                    .and_then(agenda_time::parse_date);
                let day = planned.or(due_day);
                let overdue = query::is_overdue(task, now);
                let include = page.selected("tasks")
                    || (page.selected("overdue") && overdue)
                    || (page.selected("today") && (overdue || day == Some(today)))
                    || (page.selected("upcoming")
                        && day.is_some_and(|d| d >= today && d <= horizon));
                let done = !task.status.is_open();
                if !include || (done && hide_done) || !seen.insert(task.id.clone()) {
                    continue;
                }
                let object = serde_json::json!({
                    "id": task.id, "title": task.title, "kind": "task",
                    "status": task.status.wire(), "priority": task.priority.wire(),
                    "note": task.note, "due": task.due, "projectId": task.project_id,
                });
                let date = day.map(agenda_time::date_key).unwrap_or_default();
                objects.push((object, done, date));
            }
        }
        if page.selected("events") || page.selected("today") || page.selected("upcoming") {
            let to = if page.selected("events") || page.selected("upcoming") {
                horizon
            } else {
                today
            };
            for slot in query::slots_between(&data, today, to, now) {
                let done = slot.status != crate::agenda::EntryStatus::Planned;
                if (done && hide_done) || !seen.insert(slot.key.clone()) {
                    continue;
                }
                let object = serde_json::json!({
                    "id": slot.key, "title": slot.title, "kind": "entry",
                    "status": slot.status.wire(), "note": slot.note,
                    "start": agenda_time::stamp(slot.start), "projectId": slot.project_id,
                });
                objects.push((object, done, agenda_time::date_key(slot.start.date())));
            }
        }
        let mut rows = Vec::new();
        for (object, done, date) in objects {
            let value = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or("");
            let groups: Vec<String> = if page.presentation.schedule_view == 2 {
                page.presentation
                    .groups
                    .iter()
                    .filter(|g| super::schedule_rules::matches(&g.rule, &object, &date, today))
                    .map(|g| g.name.clone())
                    .collect()
            } else {
                vec![if date.is_empty() {
                    "未设置日期".into()
                } else {
                    date.clone()
                }]
            };
            for (group_index, group) in groups.into_iter().enumerate() {
                let lead = if value("kind") == "entry" {
                    value("start").get(11..16).unwrap_or("全天").to_owned()
                } else {
                    match value("priority") {
                        "urgent" => "紧急",
                        "high" => "高优先级",
                        "low" => "低优先级",
                        _ => "一般",
                    }
                    .to_owned()
                };
                let note = value("note");
                rows.push(Row {
                    id: format!("{}:{group_index}", value("id")),
                    title: value("title").into(),
                    detail: if note.is_empty() {
                        lead
                    } else {
                        format!("{lead} · {note}")
                    },
                    action: Some(Action::ToggleTask {
                        id: value("id").into(),
                        checked: !done,
                    }),
                    checked: Some(done),
                    meta: super::RowMeta {
                        date: date.clone(),
                        group,
                        ..Default::default()
                    },
                });
            }
        }
        if page.presentation.schedule_view == 2 {
            rows.sort_by_key(|r| {
                page.presentation
                    .groups
                    .iter()
                    .position(|g| g.name == r.meta.group)
                    .unwrap_or(usize::MAX)
            });
        } else {
            rows.sort_by(|a, b| a.meta.date.cmp(&b.meta.date));
        }
        rows
    }

    fn inbox_rows(&self, page: &Page) -> Vec<Row> {
        if !page.selected("pending") && !page.selected("latest") {
            return Vec::new();
        }
        let file = self.read_json_path::<CaptureFile>(&self.workspace.join("收件箱/items.json"));
        let Some(file) = file else { return Vec::new() };
        let mut items = file.items;
        items.retain(|item| {
            (item.status == "inbox" && page.selected("pending"))
                || (item.status != "inbox" && page.selected("latest"))
        });
        items.sort_by_key(|item| (item.status != "inbox", Reverse(item.updated_at)));
        items
            .into_iter()
            .map(|item| {
                let group = if item.status == "inbox" {
                    "未归档"
                } else {
                    "已归档"
                };
                let mut row = capture_row(item);
                row.meta.group = group.into();
                row
            })
            .collect()
    }

    fn quick_note_rows(&self, page: &Page) -> Vec<Row> {
        let mut rows = Vec::new();
        if page.selected("capture") {
            rows.push(Row {
                id: "capture".into(),
                title: "新建速记".into(),
                detail: "记录想法、任务或待办".into(),
                action: Some(Action::Capture),
                checked: None,
                meta: Default::default(),
            });
        }
        if page.selected("latest") {
            let file =
                self.read_json_path::<CaptureFile>(&self.workspace.join("收件箱/items.json"));
            if let Some(file) = file {
                let mut items = file.items;
                items.sort_by_key(|item| Reverse(item.updated_at));
                rows.extend(items.into_iter().map(capture_row));
            }
        }
        rows
    }

    fn recent_rows(&self, page: &Page) -> Vec<Row> {
        if !page.selected("documents") {
            return Vec::new();
        }
        for relative in [".mochi/recent.json", ".mochi/recent-files.json"] {
            if let Some(rows) = self.value_rows(relative) {
                if !rows.is_empty() {
                    return rows;
                }
            }
        }
        let rows = self.activity_recent_rows();
        if rows.is_empty() {
            self.workspace_rows()
        } else {
            rows
        }
    }

    /// 活动 JSONL 是追加式的，会越积越大。只读当前/最近一个月的
    /// 有界尾部，再对文件打开记录去重。
    fn activity_recent_rows(&self) -> Vec<Row> {
        let dir = self.workspace.join(".mochi/activity");
        let mut files = self
            .read_dir_entries(&dir)
            .into_iter()
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        files.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        files.reverse();
        let mut rows = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for path in files.into_iter() {
            for event in self.read_activity_tail(&path) {
                if !matches!(
                    event.kind,
                    crate::analytics::events::ActivityEventType::FileOpen
                        | crate::analytics::events::ActivityEventType::FileSave
                        | crate::analytics::events::ActivityEventType::FileCreate
                ) {
                    continue;
                }
                let Some(relative) = event.relative_path else {
                    continue;
                };
                if !validate_relative_path(&relative, "活动文件路径").is_ok()
                    || !seen.insert(relative.clone())
                {
                    continue;
                }
                let title = Path::new(&relative)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                rows.push(Row {
                    id: format!("recent:{relative}"),
                    title,
                    detail: format!("最近使用 · {}", event.timestamp),
                    action: Some(Action::OpenFile(relative)),
                    checked: None,
                    meta: Default::default(),
                });
            }
            if rows.len() >= self.limits.max_rows {
                break;
            }
        }
        rows
    }

    fn read_activity_tail(&self, path: &Path) -> Vec<crate::analytics::events::ActivityEvent> {
        let Ok(mut file) = File::open(path) else {
            return Vec::new();
        };
        let Ok(length) = file.metadata().map(|metadata| metadata.len()) else {
            return Vec::new();
        };
        let start = length.saturating_sub(self.limits.max_read_bytes as u64);
        let read_length = length.saturating_sub(start) as usize;
        if !self.reserve_read_budget(read_length, path) {
            return Vec::new();
        }
        if file.seek(SeekFrom::Start(start)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines = text.lines().collect::<Vec<_>>();
        let lines = if start > 0 {
            lines.into_iter().skip(1).collect::<Vec<_>>()
        } else {
            lines
        };
        lines
            .into_iter()
            .rev()
            .filter_map(|line| serde_json::from_str(line).ok())
            .take(self.limits.max_rows)
            .collect()
    }

    fn favorite_rows(&self, page: &Page) -> Vec<Row> {
        if page.module == Module::Favorites && !page.selected("documents") {
            return Vec::new();
        }
        let store = crate::paths::mochi_dir(&self.workspace).join("favorites.json");
        let Ok(metadata) = fs::metadata(&store) else {
            return Vec::new();
        };
        if metadata.len() > self.limits.max_read_bytes as u64 {
            return Vec::new();
        }
        let Ok(favorites) = crate::favorites::Favorites::load(&self.workspace) else {
            return Vec::new();
        };
        favorites
            .paths()
            .into_iter()
            .filter_map(|path| {
                let relative = self.relative_path(&path)?;
                let title = path.file_name()?.to_string_lossy().into_owned();
                Some(Row {
                    id: format!("favorite:{relative}"),
                    title,
                    detail: relative.clone(),
                    action: Some(Action::OpenFile(relative)),
                    checked: None,
                    meta: Default::default(),
                })
            })
            .collect()
    }

    fn knowledge_rows(&self, page: &Page) -> Vec<Row> {
        let mut rows = Vec::new();
        for library in self.read_vec::<Library>(".mochi/libraries.json") {
            let Some(path) = self.library_path(&library.path) else {
                continue;
            };
            let Some(relative) = self.relative_path(&path) else {
                continue;
            };
            rows.push(Row {
                id: format!("library:{relative}"),
                title: library.name,
                detail: String::new(),
                action: Some(Action::OpenFile(relative.clone())),
                checked: None,
                meta: super::RowMeta {
                    path: relative,
                    directory: true,
                    icon: library
                        .icon
                        .filter(|icon| !icon.is_empty())
                        .unwrap_or_else(|| "library".into()),
                    ..Default::default()
                },
            });
        }
        if rows.is_empty() {
            let root = page
                .source
                .as_deref()
                .and_then(|p| super::safe_source_path(&self.workspace, p).ok())
                .unwrap_or_else(|| self.workspace.join("知识库"));
            self.library_tree(
                &root,
                0,
                &mut rows,
                &mut std::collections::HashSet::new(),
                false,
            );
        }
        rows
    }

    fn library_tree(
        &self,
        root: &Path,
        depth: usize,
        rows: &mut Vec<Row>,
        _seen: &mut std::collections::HashSet<PathBuf>,
        _recursive: bool,
    ) {
        for node in crate::files::FileService::new()
            .build_file_tree_shallow(root)
            .unwrap_or_default()
        {
            let path = Path::new(&node.path);
            let Some(relative) = self.relative_path(path) else {
                continue;
            };
            if super::safe_source_path(&self.workspace, &relative).is_err() {
                continue;
            }
            let directory = node.is_directory();
            rows.push(Row {
                id: format!("tree:{relative}"),
                title: node.name,
                action: Some(Action::OpenFile(relative.clone())),
                meta: super::RowMeta {
                    path: relative,
                    depth,
                    directory,
                    icon: if directory { "folder" } else { "file" }.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        }
    }

    fn decorate_rows(&self, page: &Page, rows: &mut [Row]) {
        for row in rows {
            if let Some(Action::OpenFile(relative)) = &row.action {
                row.meta.path = relative.clone();
                if let Ok(path) = super::safe_source_path(&self.workspace, relative) {
                    row.meta.directory = path.is_dir();
                    if row.meta.icon.is_empty() {
                        row.meta.icon = if row.meta.directory { "folder" } else { "file" }.into();
                    }
                    if page.module == Module::Recent
                        || page.presentation.show_modified
                        || page.selected("modifiedAt")
                    {
                        row.detail =
                            if page.selected("modifiedAt") || page.presentation.show_modified {
                                fs::metadata(&path)
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .map(|time| {
                                        chrono::DateTime::<Local>::from(time)
                                            .format("%Y-%m-%d %H:%M")
                                            .to_string()
                                    })
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                        row.meta.always_detail =
                            page.selected("modifiedAt") || page.presentation.show_modified;
                    }
                }
                if !page.presentation.show_extensions && !row.meta.directory {
                    row.title = Path::new(&row.title)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| row.title.clone());
                }
            }
        }
    }

    fn quick_nav_rows(&self, page: &Page) -> Vec<Row> {
        if !page.selected("shortcuts") && !page.selected("recent") {
            return Vec::new();
        }
        let Some(model) = self.read_json_path::<crate::quick_navigation::Model>(
            &self
                .workspace
                .join(".mochi/extensions-data/quick-navigation/default/data.json"),
        ) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let action = || Some(Action::OpenModule(Module::QuickNav.wire_name().into()));
        if page.selected("shortcuts") {
            let mut items = model.items.clone();
            items.sort_by_key(|item| (item.order, item.name.clone()));
            for item in items {
                if !seen.insert(item.id.clone()) {
                    continue;
                }
                rows.push(quick_nav_row(item, action()));
            }
        }
        if page.selected("recent") {
            let mut items = model
                .items
                .into_iter()
                .filter(|item| item.last_opened_at.is_some())
                .collect::<Vec<_>>();
            items.sort_by_key(|item| Reverse(item.last_opened_at.clone()));
            for item in items {
                if !seen.insert(item.id.clone()) {
                    continue;
                }
                rows.push(quick_nav_row(item, action()));
            }
        }
        rows
    }

    fn english_rows(&self, page: &Page) -> Vec<Row> {
        let service = crate::english_lab::Service::new(&self.workspace);
        let path = service.path();
        if !path.is_file() {
            return Vec::new();
        }
        let Some(connection) = self.open_english_read_only(path) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if page.selected("stats") {
            if let Some((words, due, accuracy)) = self.english_dashboard(&connection) {
                rows.push(module_entry(
                    Module::English,
                    "学习进度",
                    &format!(
                        "词汇 {}，待复习 {}，今日正确率 {:.0}%",
                        words,
                        due,
                        accuracy * 100.0
                    ),
                ));
            }
        }
        let mut words = Vec::new();
        if page.selected("review") {
            words.extend(self.english_due_words(&connection, page.limit));
        }
        if page.selected("recent") {
            let mut seen = words
                .iter()
                .map(|word| word.id)
                .collect::<std::collections::HashSet<_>>();
            for word in self.english_recent_words(&connection, page.limit) {
                if seen.insert(word.id) {
                    words.push(word);
                }
            }
        }
        rows.extend(words.into_iter().take(page.limit).map(|word| Row {
            id: format!("word:{}", word.id),
            title: word.word,
            detail: word.meaning,
            action: Some(Action::OpenModule(Module::English.wire_name().into())),
            checked: None,
            meta: Default::default(),
        }));
        rows
    }

    /// 直接打开英语数据库，不走 `Service::open`——那是一条会改状态
    /// 的应用路径（建目录、配 WAL、跑迁移）。卡片必须保持只读，
    /// 不能因为在桌面上可见就顺手建库。
    fn open_english_read_only(&self, path: &Path) -> Option<Connection> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.record_read_error(format!("英语数据库无法读取（{}）", error));
                return None;
            }
        };
        if metadata.len() > self.limits.max_read_bytes as u64 {
            self.record_read_error(format!(
                "英语数据库超过单文件 {} 字节读取上限",
                self.limits.max_read_bytes
            ));
            return None;
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
            if let Ok(sidecar_metadata) = fs::metadata(&sidecar) {
                if sidecar_metadata.len() > self.limits.max_read_bytes as u64 {
                    self.record_read_error(format!(
                        "英语数据库辅助文件超过读取上限：{}",
                        sidecar.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    return None;
                }
            }
        }
        if !self.reserve_read_budget(metadata.len() as usize, path) {
            return None;
        }
        let connection = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(connection) => connection,
            Err(error) => {
                self.record_read_error(format!("英语数据库无法只读打开（{}）", error));
                return None;
            }
        };
        if let Err(error) = connection.busy_timeout(std::time::Duration::from_millis(250)) {
            self.record_read_error(format!("英语数据库读取超时设置失败（{}）", error));
            return None;
        }
        Some(connection)
    }

    fn english_dashboard(&self, connection: &Connection) -> Option<(u64, u64, f64)> {
        let now = crate::jstime::now();
        let day = Local::now().format("%Y-%m-%d").to_string();
        let result = connection.query_row(
            "SELECT
                (SELECT COUNT(*) FROM words),
                (SELECT COUNT(*) FROM word_states WHERE suspended=0 AND due IS NOT NULL AND due<=?1),
                COALESCE((SELECT AVG(is_correct) FROM test_records WHERE date(create_time)=?2), 0)",
            params![now, day],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?.max(0) as u64,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, f64>(2)?.clamp(0.0, 1.0),
                ))
            },
        );
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.record_read_error(format!("英语学习统计读取失败（{}）", error));
                None
            }
        }
    }

    fn english_due_words(
        &self,
        connection: &Connection,
        limit: usize,
    ) -> Vec<crate::english_lab::Word> {
        let now = crate::jstime::now();
        let mut statement = match connection.prepare(
            "SELECT w.id,w.word,COALESCE(w.us_phonetic,w.phonetic,''),
                COALESCE((SELECT chinese FROM meanings m WHERE m.word_id=w.id ORDER BY m.id LIMIT 1),''),
                COALESCE(s.state,'new'),s.due,COALESCE(s.mastery,0)
             FROM words w LEFT JOIN word_states s ON s.word_id=w.id
             WHERE COALESCE(s.suspended,0)=0 AND (s.due IS NULL OR s.due<=?1)
             ORDER BY CASE WHEN s.due IS NULL THEN 0 ELSE 1 END,s.due
             LIMIT ?2",
        ) {
            Ok(statement) => statement,
            Err(error) => {
                self.record_read_error(format!("英语待复习词查询失败（{}）", error));
                return Vec::new();
            }
        };
        let rows = match statement.query_map(
            params![now, limit.min(MAX_ROWS_PER_PAGE) as i64],
            english_word_from_row,
        ) {
            Ok(rows) => rows,
            Err(error) => {
                self.record_read_error(format!("英语待复习词读取失败（{}）", error));
                return Vec::new();
            }
        };
        self.collect_english_words(rows, "英语待复习词")
    }

    fn english_recent_words(
        &self,
        connection: &Connection,
        limit: usize,
    ) -> Vec<crate::english_lab::Word> {
        let mut statement = match connection.prepare(
            "SELECT w.id,w.word,COALESCE(w.us_phonetic,w.phonetic,''),
                COALESCE((SELECT chinese FROM meanings m WHERE m.word_id=w.id ORDER BY m.id LIMIT 1),''),
                COALESCE(s.state,'new'),s.due,COALESCE(s.mastery,0)
             FROM words w LEFT JOIN word_states s ON s.word_id=w.id
             ORDER BY w.id DESC LIMIT ?1",
        ) {
            Ok(statement) => statement,
            Err(error) => {
                self.record_read_error(format!("英语最近词查询失败（{}）", error));
                return Vec::new();
            }
        };
        let rows = match statement.query_map(
            params![limit.min(MAX_ROWS_PER_PAGE) as i64],
            english_word_from_row,
        ) {
            Ok(rows) => rows,
            Err(error) => {
                self.record_read_error(format!("英语最近词读取失败（{}）", error));
                return Vec::new();
            }
        };
        self.collect_english_words(rows, "英语最近词")
    }

    fn collect_english_words<F>(
        &self,
        rows: rusqlite::MappedRows<'_, F>,
        label: &str,
    ) -> Vec<crate::english_lab::Word>
    where
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<crate::english_lab::Word>,
    {
        rows.filter_map(|row| match row {
            Ok(word) => Some(word),
            Err(error) => {
                self.record_read_error(format!("{label}读取失败（{}）", error));
                None
            }
        })
        .collect()
    }

    fn ai_rows(&self, page: &Page) -> Vec<Row> {
        let mut rows = Vec::new();
        if page.selected("sessions") {
            if let Some(index) = self.read_json_path::<AiSessionIndex>(
                &self.workspace.join(".mochi/ai-sessions/index.json"),
            ) {
                let mut sessions = index.sessions;
                sessions.sort_by_key(|session| Reverse(session.updated_at));
                rows.extend(sessions.into_iter().map(|session| Row {
                    id: format!("ai:{}", session.id),
                    title: session.title,
                    detail: format!("{} 条消息", session.message_count),
                    action: Some(Action::OpenModule(Module::Ai.wire_name().into())),
                    checked: None,
                    meta: Default::default(),
                }));
            }
        }
        if page.selected("inbox") {
            let pending = self.pending_agent_operations();
            if pending > 0 {
                rows.push(module_entry(
                    Module::Inbox,
                    "待处理操作",
                    &format!("{} 项操作等待确认", pending),
                ));
            }
        }
        rows
    }

    fn template_rows(&self, _page: &Page) -> Vec<Row> {
        if !_page.selected("templates") {
            return Vec::new();
        }
        let root = self.workspace.join(".mochi/templates");
        self.directory_rows(&root, Module::Templates)
    }

    fn pending_agent_operations(&self) -> usize {
        let Some(value) = self.read_value(crate::ai::agent_inbox::INBOX_REL_PATH) else {
            return 0;
        };
        let Value::Array(entries) = value else {
            return 0;
        };
        entries
            .iter()
            .filter(|entry| {
                entry
                    .get("operation")
                    .and_then(|operation| operation.get("status"))
                    .and_then(Value::as_str)
                    == Some("pending")
            })
            .count()
    }

    fn agent_rows(&self, page: &Page) -> Vec<Row> {
        let mut rows = Vec::new();
        let root = self.workspace.join("Agent配置");
        for (section, option) in [
            ("Agents", "agents"),
            ("Skills", "skills"),
            ("QuickActions", "quickActions"),
            ("MCPs", "mcps"),
        ] {
            if page.selected(option) {
                rows.extend(self.directory_rows(&root.join(section), Module::AgentConfig));
            }
        }
        rows
    }

    fn collection_rows(&self, page: &Page) -> Vec<Row> {
        match super::collection::rows(
            &self.workspace,
            page,
            self.limits.max_dir_entries,
            self.limits.max_rows,
        ) {
            Ok(rows) => rows,
            Err(error) => {
                if let Ok(mut state) = self.read_state.lock() {
                    state.errors.push(error.to_string());
                }
                vec![]
            }
        }
    }

    fn directory_rows(&self, path: &Path, _module: Module) -> Vec<Row> {
        self.read_dir_entries(path)
            .into_iter()
            .filter_map(|entry| {
                let relative = self.relative_path(&entry.path())?;
                Some(Row {
                    id: format!("file:{relative}"),
                    title: entry.file_name().to_string_lossy().into_owned(),
                    detail: relative.clone(),
                    action: Some(Action::OpenFile(relative)),
                    checked: None,
                    meta: Default::default(),
                })
            })
            .collect()
    }

    fn workspace_rows(&self) -> Vec<Row> {
        self.content_files()
            .into_iter()
            .filter_map(|path| {
                let relative = self.relative_path(&path)?;
                Some(Row {
                    id: format!("file:{relative}"),
                    title: path.file_name()?.to_string_lossy().into_owned(),
                    detail: relative.clone(),
                    action: Some(Action::OpenFile(relative)),
                    checked: None,
                    meta: Default::default(),
                })
            })
            .collect()
    }

    /// 常规工作区布局把文档存在各知识库实例之下。只枚举根和一层
    /// 资料库；文件树保持惰性加载，卡片绝不递归扫描工作区。
    fn content_files(&self) -> Vec<PathBuf> {
        let mut files = self
            .workspace_entries()
            .into_iter()
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        let mut roots = self
            .workspace_entries()
            .into_iter()
            .filter(|path| {
                matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("知识库" | "日记")
                )
            })
            .collect::<Vec<_>>();
        for root in roots.drain(..) {
            for library in self.read_dir_entries(&root) {
                if library.path().is_file() {
                    files.push(library.path());
                    continue;
                }
                files.extend(
                    self.read_dir_entries(&library.path())
                        .into_iter()
                        .map(|entry| entry.path())
                        .filter(|path| path.is_file()),
                );
                if files.len() >= self.limits.max_dir_entries {
                    break;
                }
            }
        }
        files.sort_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|time| time.elapsed().ok())
        });
        files.reverse();
        files.truncate(self.limits.max_dir_entries);
        files
    }

    fn workspace_entries(&self) -> Vec<PathBuf> {
        self.read_dir_entries(&self.workspace)
            .into_iter()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                (name != ".mochi" && !name.starts_with('.')).then_some(entry.path())
            })
            .collect()
    }

    fn library_path(&self, raw: &str) -> Option<PathBuf> {
        let root = self.workspace.canonicalize().ok()?;
        let configured = Path::new(raw);
        let candidate = if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            root.join(configured)
        };
        let canonical = candidate.canonicalize().ok()?;
        crate::paths::path_is_within(&root, &canonical).then_some(canonical)
    }

    fn relative_path(&self, path: &Path) -> Option<String> {
        let root = self.workspace.canonicalize().ok()?;
        let canonical = path.canonicalize().ok()?;
        if !crate::paths::path_is_within(&root, &canonical) {
            return None;
        }
        let relative = canonical.strip_prefix(root).ok()?;
        let value = relative.to_string_lossy().replace('\\', "/");
        (!value.is_empty() && validate_relative_path(&value, "工作区路径").is_ok()).then_some(value)
    }

    fn read_vec<T: DeserializeOwned>(&self, relative: &str) -> Vec<T> {
        self.read_json_path::<Vec<T>>(&self.workspace.join(relative))
            .unwrap_or_default()
    }

    fn read_value(&self, relative: &str) -> Option<Value> {
        self.read_json_path::<Value>(&self.workspace.join(relative))
    }

    /// 只枚举目录的一小段有界前缀。每次刷新的预算由所有卡片共享，
    /// 同一个目录来源就算复制到多张卡上，也不可能把一次刷新变成
    /// 无界的文件系统遍历。
    fn read_dir_entries(&self, path: &Path) -> Vec<fs::DirEntry> {
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Vec::new();
            }
            Err(error) => {
                self.record_read_error(format!(
                    "{} 无法读取目录（{}）",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    error
                ));
                return Vec::new();
            }
        };
        let mut output = Vec::new();
        for result in entries {
            if output.len() >= self.limits.max_dir_entries || !self.reserve_dir_entry(path) {
                break;
            }
            match result {
                Ok(entry) => output.push(entry),
                Err(error) => self.record_read_error(format!(
                    "{} 目录项读取失败（{}）",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    error
                )),
            }
        }
        output
    }

    fn reserve_dir_entry(&self, path: &Path) -> bool {
        let total_limit = self
            .limits
            .max_dir_entries
            .saturating_mul(32 * super::MAX_CARDS);
        let allowed = if let Ok(mut state) = self.read_state.lock() {
            if state.dir_entries >= total_limit {
                false
            } else {
                state.dir_entries = state.dir_entries.saturating_add(1);
                true
            }
        } else {
            false
        };
        if !allowed {
            self.record_read_error(format!(
                "本次刷新目录读取预算已用尽，跳过 {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            return false;
        }
        true
    }

    fn read_json_path<T: DeserializeOwned>(&self, path: &Path) -> Option<T> {
        let bytes = self.read_bytes(path)?;
        match serde_json::from_slice(&bytes) {
            Ok(value) => Some(value),
            Err(error) => {
                self.record_read_error(format!(
                    "{} 格式错误（{}）",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    error
                ));
                None
            }
        }
    }

    fn read_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        if let Ok(state) = self.read_state.lock() {
            if let Some(bytes) = state.cache.get(path) {
                return bytes.clone();
            }
        }
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Ok(mut state) = self.read_state.lock() {
                    state.cache.insert(path.to_path_buf(), None);
                }
                return None;
            }
            Err(error) => {
                self.record_read_error(format!(
                    "{} 无法读取（{}）",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    error
                ));
                return None;
            }
        };
        let max = self.limits.max_read_bytes;
        let length = metadata.len();
        if length > max as u64 {
            self.record_read_error(format!(
                "{} 超过单文件 {} 字节读取上限",
                path.file_name().unwrap_or_default().to_string_lossy(),
                max
            ));
            if let Ok(mut state) = self.read_state.lock() {
                state.cache.insert(path.to_path_buf(), None);
            }
            return None;
        }
        if !self.reserve_read_budget(length as usize, path) {
            if let Ok(mut state) = self.read_state.lock() {
                state.cache.insert(path.to_path_buf(), None);
            }
            return None;
        }
        let file = File::open(path).ok()?;
        let mut bytes = Vec::with_capacity(length as usize);
        if file.take(max as u64 + 1).read_to_end(&mut bytes).is_err() {
            self.record_read_error(format!(
                "{} 读取失败",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            return None;
        }
        if bytes.len() > max {
            self.record_read_error(format!(
                "{} 超过单文件读取上限",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            if let Ok(mut state) = self.read_state.lock() {
                state.cache.insert(path.to_path_buf(), None);
            }
            return None;
        }
        if let Ok(mut state) = self.read_state.lock() {
            state.cache.insert(path.to_path_buf(), Some(bytes.clone()));
        }
        Some(bytes)
    }

    fn reserve_read_budget(&self, amount: usize, path: &Path) -> bool {
        let total_limit = self
            .limits
            .max_read_bytes
            .saturating_mul(super::MAX_CARDS)
            .min(192 * 1024 * 1024);
        let allowed = if let Ok(mut state) = self.read_state.lock() {
            if state.bytes.saturating_add(amount) > total_limit {
                false
            } else {
                state.bytes = state.bytes.saturating_add(amount);
                true
            }
        } else {
            false
        };
        if !allowed {
            self.record_read_error(format!(
                "本次刷新读取预算已用尽，跳过 {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            return false;
        }
        true
    }

    fn record_read_error(&self, message: String) {
        if let Ok(mut state) = self.read_state.lock() {
            if state.errors.len() < 8 && !state.errors.iter().any(|item| item == &message) {
                state.errors.push(message);
            }
        }
    }

    fn value_rows(&self, relative: &str) -> Option<Vec<Row>> {
        Some(
            value_rows_from_value(self.read_value(relative)?)
                .into_iter()
                .filter_map(|mut row| {
                    let Some(Action::OpenFile(raw)) = &row.action else {
                        return None;
                    };
                    let path = self.library_path(raw)?;
                    row.action = Some(Action::OpenFile(self.relative_path(&path)?));
                    Some(row)
                })
                .collect(),
        )
    }
}

pub fn build_snapshot(workspace: &Path, config: &super::DesktopConfig) -> anyhow::Result<Snapshot> {
    SnapshotBuilder::new(workspace).build(config)
}

pub fn build_snapshot_with_limits(
    workspace: &Path,
    config: &super::DesktopConfig,
    limits: SnapshotLimits,
) -> anyhow::Result<Snapshot> {
    SnapshotBuilder::with_limits(workspace, limits).build(config)
}

fn english_word_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::english_lab::Word> {
    Ok(crate::english_lab::Word {
        id: row.get(0)?,
        word: row.get(1)?,
        phonetic: row.get(2)?,
        meaning: row.get(3)?,
        state: row.get(4)?,
        due: row.get(5)?,
        mastery: row.get(6)?,
    })
}

fn quick_nav_row(item: crate::quick_navigation::Item, action: Option<Action>) -> Row {
    let title = if item.name.trim().is_empty() {
        item.target.clone()
    } else {
        item.name
    };
    let detail = if item.note.trim().is_empty() {
        item.target
    } else {
        format!("{} · {}", item.target, item.note)
    };
    Row {
        id: format!("quick-nav:{}", item.id),
        title: clip_provider_text(&title, 96),
        detail: clip_provider_text(&detail, 220),
        action,
        checked: None,
        meta: Default::default(),
    }
}

fn clip_provider_text(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let clipped = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn module_entry(module: Module, title: &str, detail: &str) -> Row {
    Row {
        id: format!("module:{}:{}", module.wire_name(), title),
        title: title.into(),
        detail: detail.into(),
        action: Some(Action::OpenModule(module.wire_name().into())),
        checked: None,
        meta: Default::default(),
    }
}

fn capture_row(item: CaptureItem) -> Row {
    let detail = item
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Row {
        id: item.id,
        title: detail.chars().take(64).collect(),
        detail,
        action: Some(Action::OpenModule(Module::Inbox.wire_name().into())),
        checked: None,
        meta: Default::default(),
    }
}

fn value_rows_from_value(value: Value) -> Vec<Row> {
    let values = match value {
        Value::Array(values) => values,
        Value::Object(mut object) => {
            for key in ["items", "entries", "files", "sessions", "data"] {
                if let Some(Value::Array(values)) = object.remove(key) {
                    return values.into_iter().filter_map(value_to_row).collect();
                }
            }
            return Vec::new();
        }
        _ => return Vec::new(),
    };
    values.into_iter().filter_map(value_to_row).collect()
}

fn value_to_row(value: Value) -> Option<Row> {
    let value = if let Value::String(path) = value {
        serde_json::json!({"path":path})
    } else {
        value
    };
    let object = value.as_object()?;
    let raw_path = ["path", "filePath", "relativePath"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))?;
    let title = ["title", "name", "label", "path", "id"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())?
        .to_owned();
    let detail = ["detail", "description", "note", "target", "updatedAt"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(&title)
        .to_owned();
    Some(Row {
        id,
        title,
        detail,
        action: Some(Action::OpenFile(raw_path.into())),
        checked: object.get("checked").and_then(Value::as_bool),
        meta: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_cards::{Card, DesktopConfig, Interaction};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn every_card_at_the_product_limit_gets_a_source_budget() {
        let root = workspace("maximum-sources");
        fs::write(root.join("note.md"), "# 每张卡片都应显示\n").unwrap();
        let mut config = DesktopConfig::default();
        for i in 0..super::super::MAX_CARDS {
            let mut card = Card::new(format!("卡片 {i}"), Module::Document);
            card.pages[0].source = Some("note.md".into());
            card.pages[0].options = vec!["title".into()];
            config.cards.push(card);
        }
        let snapshot = build_snapshot(&root, &config).unwrap();
        assert_eq!(snapshot.pages.len(), super::super::MAX_CARDS);
        assert!(
            snapshot.pages.values().all(|p| !p.rows.is_empty()),
            "{snapshot:?}"
        );
        fs::create_dir_all(root.join("library")).unwrap();
        fs::write(root.join("library/note.md"), "# 内容\n").unwrap();
        for card in &mut config.cards {
            card.pages[0].module = Module::Knowledge;
            card.pages[0].source = Some("library".into());
            card.pages[0].options = vec!["recent".into()];
        }
        let snapshot = build_snapshot(&root, &config).unwrap();
        assert!(
            snapshot.pages.values().all(|p| !p.rows.is_empty()),
            "{snapshot:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 写入几条今天截止的待办任务。
    pub(super) fn seed_tasks(root: &Path, titles: &[&str]) -> Vec<String> {
        let today = crate::agenda::time::date_key(crate::agenda::time::today());
        crate::agenda::AgendaStore::new(root)
            .mutate(crate::agenda::Source::User, |ed| {
                titles
                    .iter()
                    .map(|title| {
                        ed.create_task(crate::agenda::TaskDraft {
                            title: (*title).into(),
                            due: Some(today.clone()),
                            ..Default::default()
                        })
                    })
                    .collect()
            })
            .unwrap()
            .0
    }

    fn workspace(tag: &str) -> PathBuf {
        let number = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mochi-desktop-provider-{tag}-{number}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn empty_inbox_provider_is_bounded_and_empty() {
        let root = workspace("empty");
        let mut config = DesktopConfig::new();
        config.cards.push(Card::new("收件箱", Module::Inbox));
        let snapshot = build_snapshot(&root, &config).unwrap();
        assert_eq!(snapshot.pages.len(), 1);
        assert!(snapshot.pages.values().next().unwrap().rows.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn schedule_and_inbox_use_real_workspace_files() {
        let root = workspace("fixtures");
        seed_tasks(&root, &["读书"]);
        fs::create_dir_all(root.join("收件箱")).unwrap();
        fs::write(
            root.join("收件箱/items.json"),
            r#"{"version":1,"items":[{"id":"cap-1","content":"收集资料","createdAt":1,"updatedAt":2,"source":"app","status":"inbox","archivedPath":null,"archivedAt":null}]}"#,
        )
        .unwrap();
        let mut config = DesktopConfig::new();
        config.cards.push(Card::new("日程", Module::Schedule));
        config.cards.push(Card::new("收件箱", Module::Inbox));
        let snapshot = build_snapshot(&root, &config).unwrap();
        let schedule = snapshot
            .pages
            .values()
            .find(|page| page.title.starts_with("日程"))
            .unwrap();
        assert_eq!(schedule.rows[0].title, "读书");
        assert!(matches!(
            &schedule.rows[0].action,
            Some(Action::ToggleTask { id, checked: true }) if id.starts_with("task-")
        ));
        let inbox = snapshot
            .pages
            .values()
            .find(|page| page.title == "收件箱")
            .unwrap();
        assert_eq!(inbox.rows[0].title, "收集资料");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn page_interaction_can_force_open_only_and_limits_are_hard() {
        let root = workspace("caps");
        let ids = seed_tasks(&root, &["一", "二"]);
        let mut config = DesktopConfig::new();
        let mut card = Card::new("日程", Module::Schedule);
        card.pages[0].interaction = Interaction::OpenOnly;
        config.cards.push(card);
        config.cards.push(Card::new("收件箱", Module::Inbox));
        let snapshot = build_snapshot_with_limits(
            &root,
            &config,
            SnapshotLimits {
                max_cards: 1,
                max_rows: 1,
                max_read_bytes: 1024,
                max_dir_entries: 8,
            },
        )
        .unwrap();
        assert_eq!(snapshot.pages.len(), 1);
        let page = snapshot.pages.values().next().unwrap();
        assert_eq!(page.rows.len(), 1);
        assert!(matches!(
            &page.rows[0].action,
            Some(Action::ToggleTask { id, checked: true }) if ids.contains(id)
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_rows_are_metadata_only_and_path_traversal_is_rejected() {
        let root = workspace("source");
        fs::write(root.join("linked.md"), "very large note body is not read").unwrap();
        let mut config = DesktopConfig::new();
        let mut card = Card::new("来源", Module::Document);
        card.pages[0].source = Some("linked.md".into());
        config.cards.push(card);
        let snapshot = build_snapshot(&root, &config).unwrap();
        let row = &snapshot.pages.values().next().unwrap().rows[0];
        assert_eq!(row.action, Some(Action::OpenFile("linked.md".into())));
        assert!(crate::desktop_cards::safe_source_path(&root, "../outside").is_err());
        let _ = fs::remove_dir_all(root);
    }
}
#[cfg(test)]
mod presentation_tests {
    use super::*;
    fn workspace(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mochi-desktop-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn desktop_new_templates_and_unlimited_documents_have_real_metadata() {
        let root = workspace("presentation");
        fs::create_dir_all(root.join(".mochi")).unwrap();
        fs::create_dir_all(root.join("知识库/A/目录")).unwrap();
        fs::write(root.join("知识库/A/目录/文件.md"), "text").unwrap();
        let files: Vec<_> = (0..30)
            .map(|i| {
                let path = format!("doc-{i}.md");
                fs::write(root.join(&path), "content").unwrap();
                serde_json::json!({"path":path,"title":format!("doc-{i}.md")})
            })
            .collect();
        fs::write(
            root.join(".mochi/recent.json"),
            serde_json::to_vec(&files).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(".mochi/libraries.json"),
            r#"[{"id":"a","name":"库 A","type":"knowledge","path":"知识库/A","icon":"BookOpen"}]"#,
        )
        .unwrap();
        let builder = SnapshotBuilder::new(&root);
        let home = builder.build_page(&Page::new(Module::Home));
        assert_eq!(home.rows.len(), 14);
        assert!(home
            .rows
            .iter()
            .all(|row| matches!(row.action, Some(Action::OpenModule(_))) && row.detail.is_empty()));
        let mut page = Page::new(Module::Recent);
        let recent = builder.build_page(&page);
        assert_eq!(recent.rows.len(), 30);
        assert!(recent.rows.iter().all(|row| !row.title.ends_with(".md")
            && row.meta.always_detail
            && !row.detail.is_empty()));
        page.limit = 1;
        page.presentation.show_extensions = true;
        let limited = builder.build_page(&page);
        assert_eq!(limited.rows.len(), 1);
        assert!(limited.rows[0].title.ends_with(".md"));
        let mut library = Page::new(Module::Knowledge);
        assert_eq!(builder.build_page(&library).rows.len(), 1);
        library.presentation.expand_libraries = true;
        let tree = builder.build_page(&library);
        assert_eq!(
            tree.rows.len(),
            1,
            "snapshot must not enumerate collapsed descendants"
        );
        assert_eq!(tree.rows[0].meta.icon, "BookOpen");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completed_behavior_persists_and_follows_schedule_toggles() {
        use super::super::CompletedBehavior::{Hide, Keep, Strike};
        let root = workspace("completed-behavior");
        let today = crate::agenda::time::date_key(crate::agenda::time::today());
        let task = crate::agenda::AgendaStore::new(&root)
            .mutate(crate::agenda::Source::User, |ed| {
                let task = ed.create_task(crate::agenda::TaskDraft {
                    title: "任务".into(),
                    ..Default::default()
                })?;
                let entry = ed.create_entry(crate::agenda::EntryDraft {
                    title: "事件".into(),
                    start: format!("{today}T00:00"),
                    end: format!("{today}T00:30"),
                    ..Default::default()
                })?;
                ed.set_entry_status(&entry, crate::agenda::EntryStatus::Done, None)?;
                Ok(task)
            })
            .unwrap()
            .0;
        let builder = SnapshotBuilder::new(&root);
        let mut page = Page::new(Module::Schedule);
        assert_eq!(builder.build_page(&page).rows.len(), 1);
        page.options.push("completed".into());
        assert_eq!(page.completed_behavior(), Keep);
        assert_eq!(builder.build_page(&page).rows.len(), 2);
        for behavior in [Hide, Strike, Keep] {
            page.presentation.completed_behavior = Some(behavior);
            let saved = serde_json::to_string(&page).unwrap();
            let restored: Page = serde_json::from_str(&saved).unwrap();
            assert_eq!(restored.completed_behavior(), behavior);
            super::super::toggle_schedule_item(&root, &task, true).unwrap();
            let rows = SnapshotBuilder::new(&root).build_page(&restored).rows;
            assert_eq!(rows.len(), if behavior == Hide { 0 } else { 2 });
            assert!(rows.iter().all(|r| r.checked == Some(true)));
            super::super::toggle_schedule_item(&root, &task, false).unwrap();
            let rows = SnapshotBuilder::new(&root).build_page(&restored).rows;
            assert!(rows
                .iter()
                .any(|r| r.title == "任务" && r.checked == Some(false)));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_schedule_dates_and_overlapping_custom_groups() {
        let root = workspace("schedule-presentation");
        crate::agenda::AgendaStore::new(&root)
            .mutate(crate::agenda::Source::User, |ed| {
                ed.create_task(crate::agenda::TaskDraft {
                    title: "整理".into(),
                    priority: Some(crate::agenda::Priority::Urgent),
                    due: Some("2026-09-20T10:00".into()),
                    ..Default::default()
                })?;
                let done = ed.create_task(crate::agenda::TaskDraft {
                    title: "完成".into(),
                    ..Default::default()
                })?;
                ed.set_task_status(&done, crate::agenda::TaskStatus::Done)?;
                Ok(())
            })
            .unwrap();
        let builder = SnapshotBuilder::new(&root);
        let mut page = Page::new(Module::Schedule);
        let list = builder.build_page(&page);
        assert_eq!(list.rows.len(), 1);
        assert_eq!(list.rows[0].meta.date, "2026-09-20");
        assert!(!list.rows[0].detail.contains("2026-09-20"));
        page.presentation.schedule_view = 2;
        page.presentation.groups = vec![
            super::super::ScheduleGroup {
                name: "紧急".into(),
                rule: "优先级=紧急".into(),
            },
            super::super::ScheduleGroup {
                name: "本周".into(),
                rule: "日期<2026-09-21".into(),
            },
        ];
        let list = builder.build_page(&page);
        assert_eq!(list.rows.len(), 2);
        assert_ne!(list.rows[0].id, list.rows[1].id);
        assert_eq!(list.rows[0].meta.group, "紧急");
        assert_eq!(list.rows[1].meta.group, "本周");
        let _ = fs::remove_dir_all(root);
    }
}
