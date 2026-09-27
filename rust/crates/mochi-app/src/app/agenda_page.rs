//! 日程待办页的控制器：读写数据、撤销重做、点击、拖拽、键盘、一句话添加、自动排入。
//!
//! 所有修改都走 [`App::agenda_mutate`]：在写锁内读最新数据、改、写回，同时把精确的变更集
//! 压进撤销栈。界面只和 `ui::agenda::Hit` 打交道。
use super::*;
use crate::ui::agenda::{self, day, panel, side, DragKind, Focused, Hit, Toast, View};
use mochi_core::agenda::{
    parse, planner, query, time as at, AgendaStore, Editor, EntryDraft, EntryStatus, GoalDraft,
    Kind, RoutineDraft, RoutineStatus, Source, TaskDraft, TaskStatus, WishDraft,
};

const UNDO_LIMIT: usize = 100;

impl App {
    pub(super) fn agenda_store(&self) -> Option<AgendaStore> {
        self.shell.workspace().map(|ws| AgendaStore::new(&ws.root))
    }

    /// 重读数据（打开页面、每次改动之后、磁盘被别处改过时）。
    pub(super) fn reload_schedule(&mut self) {
        let Some(store) = self.agenda_store() else {
            self.sched.view.data = None;
            return;
        };
        self.agenda_bind_workspace(&store);
        self.sched.view.now = at::now();
        match store.load() {
            Ok(data) => {
                self.sched.stamp = Some(store.stamp());
                self.sched.view.data = Some(data);
                if self.sched.view.view == View::History {
                    self.sched.view.history = store.history(200);
                }
                self.agenda_sync_panel();
            }
            Err(error) => self.sched.view.error = error.to_string(),
        }
    }

    /// 换了工作区就丢掉上一个工作区的撤销栈和页面状态。
    fn agenda_bind_workspace(&mut self, store: &AgendaStore) {
        if self.sched.workspace.as_deref() != Some(store.workspace()) {
            self.sched.workspace = Some(store.workspace().to_path_buf());
            self.sched.undo.clear();
            self.sched.redo.clear();
            self.sched.view = agenda::State::default();
        }
    }

    pub(super) fn agenda_push_undo(&mut self, change: mochi_core::agenda::ChangeSet) {
        if change.is_empty() {
            return;
        }
        if let Some(store) = self.agenda_store() {
            self.agenda_bind_workspace(&store);
        }
        self.sched.undo.push(change);
        if self.sched.undo.len() > UNDO_LIMIT {
            self.sched.undo.remove(0);
        }
        self.sched.redo.clear();
    }

    /// 磁盘上的数据被 AI、桌面卡片或另一个窗口改过就重读。
    pub(super) fn agenda_refresh_if_stale(&mut self) {
        let Some(store) = self.agenda_store() else {
            return;
        };
        if self.sched.view.data.is_none()
            || self.sched.workspace.as_deref() != Some(store.workspace())
            || self.sched.stamp != Some(store.stamp())
        {
            self.reload_schedule();
        }
    }

    /// 面板展示的记录在别处被改了：面板没有未保存的编辑时跟着刷新。
    fn agenda_sync_panel(&mut self) {
        let st = &mut self.sched.view;
        let (Some(data), Some(pn)) = (st.data.as_ref(), st.panel.as_ref()) else {
            return;
        };
        let Some(id) = pn.record_id().map(str::to_owned) else {
            return;
        };
        if pn.dirty() || pn.focus.is_some() {
            return;
        }
        match panel::Panel::open(data, &id) {
            Some(mut fresh) => {
                fresh.scroll = pn.scroll;
                fresh.prompt = pn.prompt.clone();
                st.panel = Some(fresh);
            }
            None => st.panel = None,
        }
    }

    /// 执行一次修改并记入撤销栈。失败时展示错误并返回 `None`。
    pub(super) fn agenda_mutate<R>(
        &mut self,
        toast: Option<String>,
        f: impl FnOnce(&mut Editor<'_>) -> anyhow::Result<R>,
    ) -> Option<R> {
        let store = self.agenda_store()?;
        match store.mutate(Source::User, f) {
            Ok((result, change)) => {
                self.sched.view.error.clear();
                self.agenda_push_undo(change);
                self.sched.view.toast = toast.map(|message| Toast {
                    message,
                    undo: true,
                });
                self.reload_schedule();
                Some(result)
            }
            Err(error) => {
                self.sched.view.toast = None;
                self.sched.view.error = error.to_string();
                None
            }
        }
    }

    pub(super) fn schedule_undo(&mut self) -> bool {
        let Some(change) = self.sched.undo.pop() else {
            self.sched.view.toast = Some(Toast {
                message: "没有可以撤销的操作".into(),
                undo: false,
            });
            return true;
        };
        let Some(store) = self.agenda_store() else {
            return false;
        };
        match store.apply(Source::User, &change.inverted(), true) {
            Ok(_) => {
                self.sched.view.error.clear();
                self.sched.view.toast = Some(Toast {
                    message: "已撤销 · Ctrl+Y 重做".into(),
                    undo: false,
                });
                self.sched.redo.push(change);
            }
            Err(error) => {
                self.sched.view.toast = None;
                self.sched.view.error = format!("无法撤销：相关记录之后又被改过（{error}）");
            }
        }
        self.reload_schedule();
        true
    }

    pub(super) fn schedule_redo(&mut self) -> bool {
        let Some(change) = self.sched.redo.pop() else {
            return true;
        };
        let Some(store) = self.agenda_store() else {
            return false;
        };
        match store.apply(Source::User, &change, true) {
            Ok(_) => {
                self.sched.view.error.clear();
                self.sched.view.toast = Some(Toast {
                    message: "已重做".into(),
                    undo: false,
                });
                self.sched.undo.push(change);
            }
            Err(error) => {
                self.sched.view.toast = None;
                self.sched.view.error = format!("无法重做：{error}");
            }
        }
        self.reload_schedule();
        true
    }

    // ---------- 绘制 ----------

    pub(super) fn agenda_focused(&self) -> Option<Focused> {
        match self.focus {
            Focus::ScheduleQuick => Some(Focused::Quick),
            Focus::ScheduleQuery => Some(Focused::Query),
            Focus::ScheduleForm => self
                .sched
                .view
                .panel
                .as_ref()
                .and_then(|p| p.focus)
                .map(Focused::Field),
            _ => None,
        }
    }

    pub(super) fn paint_schedule(&mut self, area: Rect, p: &Palette) {
        self.agenda_refresh_if_stale();
        let focus = self.agenda_focused();
        let st = &mut self.sched.view;
        st.now = at::now();
        if st.scroll_to_now && st.view == View::Day && st.query.is_empty() {
            st.scroll_to_now = false;
            let target = match st.data.as_ref() {
                Some(_) if st.days().contains(&st.today()) => {
                    at::minute_of_day(st.now) as i64 - 120
                }
                Some(data) => agenda::work_minutes(data).0 - 30,
                None => 8 * 60,
            };
            st.scroll = (target.max(0) as f32 / 60.0 * day::hour_height()).max(0.0);
        }
        let mark = self.list.len();
        let mut lay = agenda::paint(&mut self.list, area, st, focus, p);
        // 越界的滚动夹紧后重画一次，避免底部露出空白。
        let mut again = false;
        if st.scroll > lay.max_scroll() {
            st.scroll = lay.max_scroll();
            again = true;
        }
        if st.todo_scroll > lay.max_todo_scroll() {
            st.todo_scroll = lay.max_todo_scroll();
            again = true;
        }
        if let Some(pn) = st.panel.as_mut() {
            if pn.scroll > lay.max_panel_scroll() {
                pn.scroll = lay.max_panel_scroll();
                again = true;
            }
        }
        if again {
            self.list.truncate(mark);
            lay = agenda::paint(&mut self.list, area, st, focus, p);
        }
        self.sched.layout = lay;
    }

    pub(super) fn paint_schedule_sidebar(&mut self, area: Rect, p: &Palette) {
        self.agenda_refresh_if_stale();
        self.sched.side_layout = side::paint(&mut self.list, area, &self.sched.view, p);
    }

    pub(super) fn on_schedule_sidebar_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.sched.side_layout.hit(x, y) else {
            return;
        };
        self.agenda_commit_panel();
        if matches!(
            self.focus,
            Focus::ScheduleQuick | Focus::ScheduleQuery | Focus::ScheduleForm
        ) {
            self.agenda_blur();
        }
        let st = &mut self.sched.view;
        match hit {
            side::SideHit::View(view) => {
                st.query.clear();
                if view == View::Day && st.view == View::Day {
                    st.goto_today();
                }
                st.set_view(view);
                if view == View::History {
                    self.sched.view.history = self
                        .agenda_store()
                        .map(|s| s.history(200))
                        .unwrap_or_default();
                }
            }
            side::SideHit::Day(date) => {
                st.query.clear();
                st.goto_day(date);
            }
            side::SideHit::MonthPrev => st.month = at::add_months(st.month, -1),
            side::SideHit::MonthNext => st.month = at::add_months(st.month, 1),
            side::SideHit::New => self.agenda_new(Kind::Task, None, None),
            side::SideHit::Open(key) => self.agenda_open_in_place(&key),
        }
    }

    // ---------- 打开、新建 ----------

    /// 在当前视图里打开检查器（不切视图）。
    pub(super) fn agenda_open_in_place(&mut self, key: &str) {
        self.agenda_commit_panel();
        let st = &mut self.sched.view;
        let Some(data) = st.data.as_ref() else { return };
        match panel::Panel::open(data, key) {
            Some(pn) => {
                st.selected = Some(key.to_owned());
                st.panel = Some(pn);
            }
            None => st.error = "这条记录已经不存在了".into(),
        }
    }

    /// 从别处（首页、链接、收集面板、Agent）跳到某条记录：切到合适的视图再打开。
    pub(super) fn agenda_open(&mut self, key: &str) -> bool {
        self.state.view = WorkspaceView::Schedule;
        self.reload_schedule();
        let now = at::now();
        let st = &mut self.sched.view;
        let Some(data) = st.data.as_ref() else {
            return false;
        };
        st.query.clear();
        let head = key.split('@').next().unwrap_or(key);
        match Kind::of_id(head) {
            Some(Kind::Entry) | Some(Kind::Routine)
                if key.contains('@') || head.starts_with("ent") =>
            {
                if let Some(slot) = query::slot_by_key(data, key, now) {
                    st.goto_day(slot.start.date());
                }
            }
            Some(Kind::Task) => st.set_view(View::Todos),
            Some(Kind::Goal) | Some(Kind::Wish) => st.set_view(View::Goals),
            Some(Kind::Routine) => st.set_view(View::Routines),
            Some(Kind::Project) => st.set_view(View::Projects),
            _ => {}
        }
        let found = self
            .sched
            .view
            .data
            .as_ref()
            .and_then(|d| panel::Panel::open(d, key));
        let ok = found.is_some();
        if let Some(pn) = found {
            self.sched.view.selected = Some(key.to_owned());
            self.sched.view.panel = Some(pn);
        } else {
            self.state.status_text = "引用的日程待办已被删除".into();
        }
        self.focus = Focus::Main;
        self.sync_state();
        ok
    }

    /// 打开新建面板。
    pub(super) fn agenda_new(
        &mut self,
        kind: Kind,
        span: Option<(chrono::NaiveDateTime, chrono::NaiveDateTime)>,
        parent: Option<String>,
    ) {
        self.agenda_commit_panel();
        let st = &mut self.sched.view;
        let date = if st.view == View::Day {
            st.date
        } else {
            st.today()
        };
        let span = span.or_else(|| {
            let data = st.data.as_ref()?;
            agenda::next_free(data, date, st.now, 60).filter(|(start, _)| start.date() == date)
        });
        st.panel = Some(panel::Panel::new_record(kind, date, span, parent));
        self.agenda_focus_field(panel::Field::Title);
    }

    pub(super) fn agenda_focus_field(&mut self, field: panel::Field) {
        if let Some(pn) = self.sched.view.panel.as_mut() {
            if pn.has(field) {
                pn.focus = Some(field);
                if let Some(f) = pn.field_mut(field) {
                    f.select_all();
                }
                self.focus = Focus::ScheduleForm;
            }
        }
    }

    pub(super) fn agenda_blur(&mut self) {
        if let Some(pn) = self.sched.view.panel.as_mut() {
            pn.focus = None;
        }
        self.focus = Focus::Main;
    }

    // ---------- 主区点击 ----------

    pub(super) fn on_schedule_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.sched.layout.hit(x, y) else {
            return;
        };
        let same_field = matches!(
            (&hit, self.sched.view.panel.as_ref().and_then(|p| p.focus)),
            (Hit::Panel(panel::Hit::Field(a)), Some(b)) if *a == b
        );
        if self.focus == Focus::ScheduleForm && !same_field {
            self.agenda_commit_panel();
        }
        let keeps_focus = matches!(
            (&hit, self.focus),
            (Hit::QuickInput | Hit::QuickSubmit, Focus::ScheduleQuick)
                | (Hit::Query | Hit::ClearQuery, Focus::ScheduleQuery)
                | (Hit::Panel(panel::Hit::Field(_)), Focus::ScheduleForm)
        );
        if !keeps_focus
            && matches!(
                self.focus,
                Focus::ScheduleQuick | Focus::ScheduleQuery | Focus::ScheduleForm
            )
        {
            self.agenda_blur();
        }
        let now = at::now();
        self.sched.view.now = now;
        match hit {
            Hit::Blank => self.sched.view.selected = None,
            Hit::QuickInput => {
                self.focus = Focus::ScheduleQuick;
                let r = self.sched.layout.quick;
                self.sched.view.quick.click(x - r.left - 32.0, false);
            }
            Hit::QuickSubmit => self.submit_quick_add(),
            Hit::Query => {
                self.focus = Focus::ScheduleQuery;
                let r = self.sched.layout.query;
                self.sched.view.query.click(x - r.left - 30.0, false);
            }
            Hit::ClearQuery => {
                self.sched.view.query.clear();
                self.sched.view.scroll = 0.0;
            }
            Hit::Prev => self.sched.view.step(false),
            Hit::Next => self.sched.view.step(true),
            Hit::Today => self.sched.view.goto_today(),
            Hit::SetWeek(week) => {
                self.sched.view.week = week;
                self.sched.view.scroll_to_now = true;
            }
            Hit::SetTodoMonth(month) => {
                self.sched.view.todo_month = month;
                self.sched.view.scroll = 0.0;
            }
            Hit::ToggleShowDone => self.sched.view.show_done = !self.sched.view.show_done,
            Hit::AiPlan => self.agenda_ai_plan(),
            Hit::AutoPlace => self.agenda_auto_place(),
            Hit::PlanAccept => self.agenda_plan_accept(),
            Hit::PlanDiscard => self.sched.view.plan = None,
            Hit::New(kind) => self.agenda_new(kind, None, None),
            Hit::Undo => {
                self.schedule_undo();
            }
            Hit::DismissToast => {
                self.sched.view.toast = None;
                self.sched.view.error.clear();
            }
            Hit::Grid => self.agenda_begin_drag(DragKind::Create, x, y, None),
            Hit::Slot(key) => {
                self.sched.view.selected = Some(key.clone());
                let all_day = self
                    .sched
                    .view
                    .data
                    .as_ref()
                    .and_then(|d| query::slot_by_key(d, &key, now))
                    .is_none_or(|s| s.all_day);
                if all_day || self.sched.layout.geo.is_none() {
                    self.agenda_open_in_place(&key);
                } else {
                    self.agenda_begin_drag(DragKind::Move { key }, x, y, None);
                }
            }
            Hit::SlotResize(key) => self.agenda_begin_drag(DragKind::Resize { key }, x, y, None),
            Hit::SlotConfirm(key, status) | Hit::Confirm(key, status) => {
                self.agenda_confirm(&key, status)
            }
            Hit::DayHead(date) => {
                self.sched.view.week = false;
                self.sched.view.goto_day(date);
            }
            Hit::Todo(id) => {
                self.sched.view.selected = Some(id.clone());
                if self.sched.view.view == View::Day
                    && self.sched.layout.geo.is_some()
                    && self.sched.view.query.is_empty()
                {
                    let minutes = self.agenda_block_minutes(&id);
                    self.agenda_begin_drag(
                        DragKind::Todo {
                            task_id: id,
                            minutes,
                        },
                        x,
                        y,
                        Some(minutes),
                    );
                } else {
                    self.agenda_open_in_place(&id);
                }
            }
            Hit::Check(id) => self.agenda_toggle_task(&id),
            Hit::Day(date) => self.sched.view.goto_day(date),
            Hit::Open(id) => self.agenda_open_in_place(&id),
            Hit::ConfirmAndComplete(key) => self.agenda_complete_block_and_task(&key),
            Hit::ConfirmAll => {
                let keys: Vec<String> = self
                    .sched
                    .view
                    .data
                    .as_ref()
                    .map(|d| {
                        query::confirm_queue(d, now)
                            .into_iter()
                            .map(|s| s.key)
                            .collect()
                    })
                    .unwrap_or_default();
                let n = keys.len();
                self.agenda_mutate(
                    Some(format!("{n} 个日程已标为已执行 · 关联任务的状态没有改变")),
                    |ed| {
                        for key in &keys {
                            ed.confirm(key, EntryStatus::Done)?;
                        }
                        Ok(())
                    },
                );
            }
            Hit::Restore(id) => {
                self.agenda_mutate(Some("已恢复".into()), |ed| ed.restore(&id));
            }
            Hit::Purge(id) => {
                self.agenda_mutate(Some("已永久删除".into()), |ed| ed.purge(&id));
            }
            Hit::RoutineToggle(id) => {
                let active = self
                    .sched
                    .view
                    .data
                    .as_ref()
                    .and_then(|d| d.routine(&id))
                    .is_some_and(|r| r.status == RoutineStatus::Active);
                let (status, label) = if active {
                    (RoutineStatus::Paused, "已暂停，之后不再生成")
                } else {
                    (RoutineStatus::Active, "已恢复")
                };
                self.agenda_mutate(Some(label.into()), |ed| ed.set_routine_status(&id, status));
            }
            Hit::Panel(h) => self.agenda_panel_hit(h, x, y),
        }
    }

    /// 给任务排一块时间用多长：剩余估时，否则默认块长。
    fn agenda_block_minutes(&self, task_id: &str) -> i64 {
        let Some(data) = self.sched.view.data.as_ref() else {
            return 60;
        };
        let default = data.settings.default_block_minutes.max(15) as i64;
        let Some(task) = data.task(task_id) else {
            return default;
        };
        let facts = query::task_facts(data, task, self.sched.view.now);
        match task.estimate_minutes {
            Some(total) => {
                let left = total as i64 - facts.done_minutes - facts.planned_minutes;
                if left >= 15 {
                    left.min(180)
                } else {
                    default
                }
            }
            None => default,
        }
    }

    // ---------- 拖拽 ----------

    fn agenda_begin_drag(&mut self, kind: DragKind, x: f32, y: f32, length: Option<i64>) {
        let Some(geo) = self.sched.layout.geo.clone() else {
            return;
        };
        let now = self.sched.view.now;
        let date = geo.date_at(x).unwrap_or(self.sched.view.date);
        let minute = geo.minute_at(y);
        let (start, end, grab) = match &kind {
            DragKind::Create => {
                let m = agenda::snap(minute - agenda::SNAP / 2);
                (m, m + agenda::SNAP, m)
            }
            DragKind::Move { key } | DragKind::Resize { key } => {
                let Some(slot) = self
                    .sched
                    .view
                    .data
                    .as_ref()
                    .and_then(|d| query::slot_by_key(d, key, now))
                else {
                    return;
                };
                let s = at::minute_of_day(slot.start) as i64;
                let e = (s + slot.minutes()).min(24 * 60);
                (s, e, minute - s)
            }
            DragKind::Todo { .. } => {
                let len = length.unwrap_or(60);
                (0, len, (len / 4).min(15))
            }
        };
        let date = match &kind {
            DragKind::Move { key } | DragKind::Resize { key } => self
                .sched
                .view
                .data
                .as_ref()
                .and_then(|d| query::slot_by_key(d, key, now))
                .map_or(date, |s| s.start.date()),
            _ => date,
        };
        self.sched.view.drag = Some(agenda::Drag {
            kind,
            origin: (x, y),
            grab_min: grab,
            date,
            start,
            end,
            moved: false,
            inside: true,
        });
        self.drag = Some(Drag {
            target: DragTarget::ScheduleBlock,
            grab_offset: 0.0,
        });
    }

    pub(super) fn schedule_drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some(geo) = self.sched.layout.geo.clone() else {
            return false;
        };
        let max = self.sched.layout.max_scroll();
        let st = &mut self.sched.view;
        let Some(drag) = st.drag.as_mut() else {
            return false;
        };
        let before = (drag.date, drag.start, drag.end, drag.moved, drag.inside);
        drag.update(&geo, x, y);
        let changed = before != (drag.date, drag.start, drag.end, drag.moved, drag.inside);
        if drag.moved && drag.inside {
            if y > geo.grid.bottom - 24.0 {
                st.scroll = (st.scroll + 12.0).min(max);
            } else if y < geo.grid.top + 24.0 {
                st.scroll = (st.scroll - 12.0).max(0.0);
            }
        }
        changed
    }

    pub(super) fn finish_schedule_drag(&mut self) {
        let Some(drag) = self.sched.view.drag.take() else {
            return;
        };
        let now = self.sched.view.now;
        let (start, end) = drag.span();
        match drag.kind {
            DragKind::Create => {
                if drag.moved && drag.inside {
                    self.agenda_new(Kind::Entry, Some((start, end)), None);
                } else if !drag.moved {
                    if self.sched.view.panel.is_some() {
                        self.sched.view.panel = None;
                    } else {
                        let len = self
                            .sched
                            .view
                            .data
                            .as_ref()
                            .map_or(60, |d| d.settings.default_block_minutes.max(15) as i64);
                        let s = at::at_minute(drag.date, drag.start);
                        self.agenda_new(
                            Kind::Entry,
                            Some((s, s + chrono::Duration::minutes(len))),
                            None,
                        );
                    }
                }
            }
            DragKind::Move { key } | DragKind::Resize { key } => {
                if !drag.moved {
                    self.agenda_open_in_place(&key);
                    return;
                }
                if !drag.inside {
                    return;
                }
                let unchanged = self
                    .sched
                    .view
                    .data
                    .as_ref()
                    .and_then(|d| query::slot_by_key(d, &key, now))
                    .is_some_and(|s| s.start == start && s.end == end);
                if unchanged {
                    return;
                }
                let label = format!(
                    "已改到 {} {}–{}",
                    at::date_label(start.date()),
                    at::hm(start.time()),
                    at::hm(end.time())
                );
                if let Some(id) = self.agenda_mutate(Some(label), |ed| {
                    let id = ed.resolve_entry(&key)?;
                    ed.move_entry(&id, start, end)?;
                    Ok(id)
                }) {
                    self.sched.view.selected = Some(id);
                }
            }
            DragKind::Todo { task_id, .. } => {
                if !drag.moved {
                    self.agenda_open_in_place(&task_id);
                    return;
                }
                if !drag.inside {
                    return;
                }
                self.agenda_schedule_task(&task_id, start, end);
            }
        }
    }

    /// 给任务新增一个时间块（从不改动已有的块和任务本身）。
    pub(super) fn agenda_schedule_task(
        &mut self,
        task_id: &str,
        start: chrono::NaiveDateTime,
        end: chrono::NaiveDateTime,
    ) -> Option<String> {
        let label = format!(
            "已安排到 {} {}–{} · 任务的截止和状态不变",
            at::relative_date_label(start.date(), at::today()),
            at::hm(start.time()),
            at::hm(end.time())
        );
        let id = self.agenda_mutate(Some(label), |ed| {
            ed.schedule_task(task_id, &at::stamp(start), &at::stamp(end))
        })?;
        self.sched.view.selected = Some(id.clone());
        Some(id)
    }

    // ---------- 常用动作 ----------

    pub(super) fn agenda_confirm(&mut self, key: &str, status: EntryStatus) {
        let message = match status {
            EntryStatus::Done => "已标为已执行 · 任务本身没有被完成",
            EntryStatus::Skipped => "已标为没做",
            EntryStatus::Cancelled => "已取消这段安排",
            EntryStatus::Planned => "已改回计划中",
        };
        if let Some(id) = self.agenda_mutate(Some(message.into()), |ed| ed.confirm(key, status)) {
            if self.sched.view.panel.as_ref().and_then(|p| p.record_id()) == Some(key) {
                self.agenda_open_in_place(&id);
            }
        }
    }

    /// 勾选 / 取消勾选任务。完成时如有未来的计划块，询问是否一并取消（默认保留）。
    pub(super) fn agenda_toggle_task(&mut self, id: &str) {
        let Some(task) = self.sched.view.data.as_ref().and_then(|d| d.task(id)) else {
            return;
        };
        let title = task.title.clone();
        if task.status.is_open() {
            let result = self.agenda_mutate(Some(format!("已完成「{title}」")), |ed| {
                ed.complete_task(id)
            });
            if let Some(future) = result {
                self.agenda_ask_future(id, future.entry_ids, "任务已完成");
            }
        } else {
            self.agenda_mutate(
                Some(format!("「{title}」重新打开 · 之前取消的安排不会恢复")),
                |ed| ed.set_task_status(id, TaskStatus::Todo).map(|_| ()),
            );
        }
    }

    pub(super) fn agenda_complete_block_and_task(&mut self, key: &str) {
        let result = self.agenda_mutate(Some("已执行，任务也已完成".into()), |ed| {
            let id = ed.resolve_entry(key)?;
            let future = ed.complete_entry_and_task(&id)?;
            let task = ed.data.entry(&id).and_then(|e| e.task_id.clone());
            Ok((id, task, future))
        });
        if let Some((id, task, future)) = result {
            if self.sched.view.panel.as_ref().and_then(|p| p.record_id()) == Some(key) {
                self.agenda_open_in_place(&id);
            }
            if let Some(task) = task {
                self.agenda_ask_future(&task, future.entry_ids, "任务已完成");
            }
        }
    }

    /// 任务还有未来的计划块时，在任务面板里问一句。什么都不选 = 保留。
    pub(super) fn agenda_ask_future(
        &mut self,
        task_id: &str,
        entry_ids: Vec<String>,
        reason: &str,
    ) {
        if entry_ids.is_empty() {
            return;
        }
        let open_other =
            self.sched.view.panel.as_ref().and_then(|p| p.record_id()) != Some(task_id);
        if open_other {
            self.agenda_open_in_place(task_id);
        }
        if let Some(pn) = self.sched.view.panel.as_mut() {
            pn.prompt = Some(panel::FuturePrompt {
                task_id: task_id.to_owned(),
                entry_ids,
                reason: reason.to_owned(),
            });
        }
    }

    pub(super) fn agenda_delete(&mut self, id: &str) {
        let Some(data) = self.sched.view.data.as_ref() else {
            return;
        };
        let title = data.title_of(id).unwrap_or_default();
        let future = match Kind::of_id(id) {
            Some(Kind::Task) => {
                let now = self.sched.view.now;
                query::task_blocks(data, id)
                    .into_iter()
                    .filter(|e| {
                        e.status == EntryStatus::Planned
                            && at::parse_stamp(&e.start).is_some_and(|s| s > now)
                    })
                    .map(|e| e.id.clone())
                    .collect()
            }
            _ => Vec::new(),
        };
        let note = match Kind::of_id(id) {
            Some(Kind::Wish) => " · 它下面的目标都还在",
            Some(Kind::Goal) => " · 它下面的任务都还在",
            Some(Kind::Task) => " · 已安排的时间块仍保留",
            _ => "",
        };
        if self
            .agenda_mutate(Some(format!("已删除「{title}」{note}")), |ed| {
                ed.delete(id, false)
            })
            .is_some()
        {
            if Kind::of_id(id) == Some(Kind::Task) && !future.is_empty() {
                self.agenda_ask_future(id, future, "任务已删除");
            } else if self.sched.view.panel.as_ref().and_then(|p| p.record_id()) == Some(id) {
                self.sched.view.panel = None;
            }
            self.sched.view.selected = None;
        }
    }

    // ---------- 自动排入、AI ----------

    fn agenda_auto_place(&mut self) {
        let st = &mut self.sched.view;
        let Some(data) = st.data.as_ref() else { return };
        let plan = planner::auto_place(data, st.date, st.now, None);
        if plan.is_empty() {
            st.toast = Some(Toast {
                message: "这一天没有需要排入的待办，或已经没有空闲时间了".into(),
                undo: false,
            });
            st.plan = None;
        } else {
            st.plan = Some(plan);
            st.scroll_to_now = true;
        }
    }

    fn agenda_plan_accept(&mut self) {
        let Some(plan) = self.sched.view.plan.take() else {
            return;
        };
        let n = plan.len();
        self.agenda_mutate(Some(format!("已排入 {n} 个时间块")), |ed| {
            for p in &plan {
                ed.schedule_task(&p.task_id, &at::stamp(p.start), &at::stamp(p.end))?;
            }
            Ok(())
        });
    }

    /// Ctrl+J：把「帮我安排这一天」的请求填进墨池 AI，让用户补充后发送。
    pub(super) fn agenda_ai_plan(&mut self) {
        let st = &self.sched.view;
        let date = if st.view == View::Day {
            st.date
        } else {
            st.today()
        };
        let rel = at::relative_date_label(date, st.today());
        let idea = st.quick.text().trim().to_owned();
        let mut prompt = format!("帮我设计{rel}（{}）的日程。", at::date_label(date));
        if !idea.is_empty() {
            prompt.push_str(&format!("我的想法：{idea}。"));
        }
        prompt.push_str("先看这一天已有的安排、快到期和逾期的待办，结合空闲时间排出时间块，说明理由；生成待我确认的变更，不要直接改。");
        self.ai.panel.input.set_text(&prompt);
        if self.ai.float.is_none() {
            self.state.ai_panel_open = true;
            self.set_right_panel(RightPanel::Assistant);
        }
        self.focus = Focus::AiInput;
        self.invalidate_main();
    }

    // ---------- 一句话添加 ----------

    pub(super) fn submit_quick_add(&mut self) {
        let text = self.sched.view.quick.text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let today = at::today();
        let base = if self.sched.view.view == View::Day {
            self.sched.view.date
        } else {
            today
        };
        let q = parse::parse_quick_add(&text, today);
        if q.title.trim().is_empty() {
            self.sched.view.error = "写上要做什么".into();
            return;
        }
        let kind = q.kind();
        let time = q
            .time
            .and_then(|(h, m)| chrono::NaiveTime::from_hms_opt(h, m, 0));
        let date = q.date.unwrap_or(base);
        let default_len = self
            .sched
            .view
            .data
            .as_ref()
            .map_or(60, |d| d.settings.default_block_minutes.max(15) as i64);
        let len = q.duration_minutes.map_or(default_len, |m| m.max(5) as i64);
        let title = q.title.clone();
        let tags = q.tags.clone();
        let result = self.agenda_mutate(
            Some(format!("已添加{}「{title}」", kind.label())),
            |ed| match kind {
                Kind::Wish => ed.create_wish(WishDraft {
                    title: title.clone(),
                    tags: tags.clone(),
                    ..Default::default()
                }),
                Kind::Goal => ed.create_goal(GoalDraft {
                    title: title.clone(),
                    tags: tags.clone(),
                    ..Default::default()
                }),
                Kind::Entry => {
                    let (start, end, all_day) = match time {
                        Some(t) => {
                            let s = date.and_time(t);
                            (
                                at::stamp(s),
                                at::stamp(s + chrono::Duration::minutes(len)),
                                false,
                            )
                        }
                        None => (at::date_key(date), at::date_key(date), true),
                    };
                    ed.create_entry(EntryDraft {
                        title: title.clone(),
                        start,
                        end,
                        all_day,
                        ..Default::default()
                    })
                }
                Kind::Routine => ed.create_routine(RoutineDraft {
                    title: title.clone(),
                    rule: q
                        .recurrence_rule
                        .clone()
                        .unwrap_or_else(|| "FREQ=DAILY".into()),
                    start_date: Some(at::date_key(q.date.unwrap_or(today))),
                    time: time.map(at::hm),
                    duration_minutes: q.duration_minutes.map(|m| m.max(5) as u32),
                    ..Default::default()
                }),
                _ => ed.create_task(TaskDraft {
                    title: title.clone(),
                    priority: q.priority(),
                    due: q.date.map(|d| match time {
                        Some(t) => at::stamp(d.and_time(t)),
                        None => at::date_key(d),
                    }),
                    estimate_minutes: q.duration_minutes.map(|m| m.max(5) as u32),
                    tags: tags.clone(),
                    ..Default::default()
                }),
            },
        );
        if let Some(id) = result {
            self.sched.view.quick.clear();
            self.sched.view.selected = Some(id);
            if kind == Kind::Entry && time.is_some() && self.sched.view.view == View::Day {
                self.sched.view.goto_day(date);
            }
        }
    }

    // ---------- 键盘 ----------

    /// 焦点在页面主区时的单键命令。
    pub(super) fn schedule_key(&mut self, key: u16, shift: bool) -> bool {
        let selected = self.sched.view.selected.clone();
        match key {
            0x1B => {
                let st = &mut self.sched.view;
                if st.drag.take().is_some() {
                    self.drag = None;
                } else if st.panel.as_ref().is_some_and(|p| p.prompt.is_some()) {
                    if let Some(pn) = st.panel.as_mut() {
                        pn.prompt = None;
                    }
                } else if st.panel.is_some() {
                    self.agenda_commit_panel();
                    self.sched.view.panel = None;
                } else if st.plan.is_some() {
                    st.plan = None;
                } else if st.toast.is_some() || !st.error.is_empty() {
                    st.toast = None;
                    st.error.clear();
                } else if !st.query.is_empty() {
                    st.query.clear();
                } else if st.selected.is_some() {
                    st.selected = None;
                } else {
                    return false;
                }
            }
            // N 新建（一句话添加）；Shift+N 打开新建面板
            0x4E if shift => self.agenda_new(Kind::Task, None, None),
            0x4E => {
                self.focus = Focus::ScheduleQuick;
                self.sched.view.quick.select_all();
            }
            0xBF => {
                self.focus = Focus::ScheduleQuery;
                self.sched.view.query.select_all();
            }
            0x54 => self.sched.view.goto_today(),
            0x25 | 0xDB => self.sched.view.step(false),
            0x27 | 0xDD => self.sched.view.step(true),
            0x44 => {
                self.sched.view.week = false;
                self.sched.view.set_view(View::Day);
            }
            0x57 => {
                self.sched.view.week = true;
                self.sched.view.set_view(View::Day);
                self.sched.view.scroll_to_now = true;
            }
            0x4D => self.sched.view.set_view(View::Month),
            0x31..=0x39 => {
                let view = View::ALL[(key - 0x31) as usize];
                self.sched.view.query.clear();
                self.sched.view.set_view(view);
                if view == View::History {
                    self.sched.view.history = self
                        .agenda_store()
                        .map(|s| s.history(200))
                        .unwrap_or_default();
                }
            }
            0x50 => self.agenda_auto_place(),
            0x0D | 0x45 => match selected {
                Some(id) => self.agenda_open_in_place(&id),
                None => return false,
            },
            0x20 | 0x58 => match selected {
                Some(id) if Kind::of_id(&id) == Some(Kind::Task) => self.agenda_toggle_task(&id),
                Some(key) => self.agenda_confirm(&key, EntryStatus::Done),
                None => return false,
            },
            0x2E | 0x08 => match selected {
                Some(id) if !id.contains('@') => self.agenda_delete(&id),
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    /// 页面上的输入框收到按键。返回是否消费。
    pub(super) fn agenda_field_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        match self.focus {
            Focus::ScheduleQuick => match self.sched.view.quick.key(key, shift, ctrl) {
                FieldKey::Submit => {
                    self.submit_quick_add();
                    true
                }
                FieldKey::Cancel => {
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Edited => true,
                FieldKey::Ignored => false,
            },
            Focus::ScheduleQuery => match self.sched.view.query.key(key, shift, ctrl) {
                FieldKey::Cancel => {
                    self.sched.view.query.clear();
                    self.focus = Focus::Main;
                    true
                }
                FieldKey::Submit => true,
                FieldKey::Edited => {
                    self.sched.view.scroll = 0.0;
                    true
                }
                FieldKey::Ignored => false,
            },
            Focus::ScheduleForm => self.agenda_panel_key(key, shift, ctrl),
            _ => false,
        }
    }

    pub(super) fn agenda_char(&mut self, ch: char) -> bool {
        match self.focus {
            Focus::ScheduleQuick => self.sched.view.quick.char(ch),
            Focus::ScheduleQuery => {
                if self.sched.view.query.char(ch) {
                    self.sched.view.scroll = 0.0;
                    return true;
                }
                false
            }
            Focus::ScheduleForm => self.agenda_field_mut().is_some_and(|f| f.char(ch)),
            _ => false,
        }
    }

    pub(super) fn agenda_field_mut(&mut self) -> Option<&mut crate::ui::widgets::TextField> {
        match self.focus {
            Focus::ScheduleQuick => Some(&mut self.sched.view.quick),
            Focus::ScheduleQuery => Some(&mut self.sched.view.query),
            Focus::ScheduleForm => {
                let pn = self.sched.view.panel.as_mut()?;
                let field = pn.focus?;
                pn.field_mut(field)
            }
            _ => None,
        }
    }

    pub(super) fn agenda_multiline_focused(&self) -> bool {
        self.focus == Focus::ScheduleForm
            && self
                .sched
                .view
                .panel
                .as_ref()
                .and_then(|p| p.focus)
                .is_some_and(|f| f.multiline())
    }

    // ---------- 滚轮、悬停、光标 ----------

    pub(super) fn agenda_wheel(&mut self, x: f32, y: f32, step: f32) {
        let lay = &self.sched.layout;
        let st = &mut self.sched.view;
        if let Some(pn) = st
            .panel
            .as_mut()
            .filter(|_| lay.panel_viewport.contains(x, y))
        {
            pn.scroll = (pn.scroll - step).clamp(0.0, lay.max_panel_scroll());
        } else if lay.todo_rect.contains(x, y) {
            st.todo_scroll = (st.todo_scroll - step).clamp(0.0, lay.max_todo_scroll());
        } else {
            st.scroll = (st.scroll - step).clamp(0.0, lay.max_scroll());
        }
    }

    pub(super) fn agenda_hover(&mut self, x: f32, y: f32) -> bool {
        let before = self.sched.view.pointer;
        self.sched.view.pointer = Some((x, y));
        let hit_now = self.sched.layout.hit(x, y);
        let hit_before = before.and_then(|(bx, by)| self.sched.layout.hit(bx, by));
        hit_now != hit_before
    }

    pub(super) fn agenda_hand_cursor(&self, x: f32, y: f32) -> bool {
        self.sched.layout.hit(x, y).is_some_and(|h| {
            !matches!(
                h,
                Hit::Blank
                    | Hit::Grid
                    | Hit::QuickInput
                    | Hit::Query
                    | Hit::Panel(panel::Hit::Field(_))
            )
        }) || self.sched.side_layout.hit(x, y).is_some()
    }
}
