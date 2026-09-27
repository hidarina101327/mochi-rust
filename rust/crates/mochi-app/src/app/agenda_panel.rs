//! 检查器面板的控制器：保存字段、新建、状态切换、上级 / 项目选择、后续安排的询问。
use super::*;
use crate::ui::agenda::panel::{self, Field, PickSlot, Target};
use crate::ui::agenda::{Hit, Toast, View};
use mochi_core::agenda::{
    batch, query, time as at, GoalStatus, Kind, ProjectStatus, RoutineStatus, TaskStatus,
    WishStatus,
};
use serde_json::{json, Map};

impl App {
    /// 把面板里改过的字段写回。新建面板不自动保存（要点「创建」或回车）。
    pub(super) fn agenda_commit_panel(&mut self) {
        let today = at::today();
        let st = &mut self.sched.view;
        let (Some(pn), Some(data)) = (st.panel.as_mut(), st.data.as_ref()) else {
            return;
        };
        if pn.is_new() || !pn.dirty() {
            return;
        }
        let Some(id) = pn.record_id().map(str::to_owned) else {
            return;
        };
        let patch = match pn.changes(data, today) {
            Ok(patch) => patch,
            Err(error) => {
                pn.error = error;
                return;
            }
        };
        pn.error.clear();
        if patch.is_empty() {
            pn.mark_saved();
            return;
        }
        let saved = self.agenda_mutate(Some("已保存".into()), |ed| {
            let id = if id.contains('@') {
                ed.resolve_entry(&id)?
            } else {
                id.clone()
            };
            ed.update(&id, &patch)?;
            Ok(id)
        });
        if let Some(real) = saved {
            if let Some(pn) = self.sched.view.panel.as_mut() {
                pn.mark_saved();
                if pn.record_id() != Some(real.as_str()) {
                    pn.target = Target::Record(real);
                }
            }
            self.agenda_refresh_panel();
        }
    }

    /// 没有焦点字段时按最新数据重建面板（把「明天 14:00」这类输入规范化显示）。
    fn agenda_refresh_panel(&mut self) {
        let st = &mut self.sched.view;
        let (Some(pn), Some(data)) = (st.panel.as_ref(), st.data.as_ref()) else {
            return;
        };
        if pn.focus.is_some() || pn.dirty() {
            return;
        }
        let Some(id) = pn.record_id() else { return };
        if let Some(mut fresh) = panel::Panel::open(data, id) {
            fresh.scroll = pn.scroll;
            fresh.prompt = pn.prompt.clone();
            st.panel = Some(fresh);
        }
    }

    fn agenda_panel_create(&mut self) {
        let today = at::today();
        let Some(pn) = self.sched.view.panel.as_mut() else {
            return;
        };
        let op = match pn.create_op(today) {
            Ok(op) => op,
            Err(error) => {
                pn.error = error;
                return;
            }
        };
        let kind = pn.kind();
        let results = self.agenda_mutate(Some(format!("已创建{}", kind.label())), |ed| {
            batch::run(ed, &[op])
        });
        let Some(id) = results
            .and_then(|r| r.into_iter().next())
            .and_then(|r| r.id)
        else {
            return;
        };
        self.agenda_blur();
        self.sched.view.panel = None;
        self.agenda_open_in_place(&id);
        if kind == Kind::Entry {
            if let Some(date) = self
                .sched
                .view
                .data
                .as_ref()
                .and_then(|d| d.entry(&id))
                .and_then(|e| at::parse_stamp(&e.start))
                .map(|s| s.date())
            {
                if self.sched.view.view == View::Day && !self.sched.view.days().contains(&date) {
                    self.sched.view.date = date;
                }
            }
        }
    }

    /// 面板字段里的按键。Tab 切换字段；回车保存（多行字段是 Ctrl+回车）；Esc 放弃当前字段的修改。
    pub(super) fn agenda_panel_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        let Some(field) = self.sched.view.panel.as_ref().and_then(|p| p.focus) else {
            return false;
        };
        if key == 0x09 && !ctrl {
            let is_new = self.sched.view.panel.as_ref().is_some_and(|p| p.is_new());
            if !is_new {
                self.agenda_commit_panel();
            }
            let Some(pn) = self.sched.view.panel.as_mut() else {
                return true;
            };
            let order = pn.tab_order();
            if let Some(i) = order.iter().position(|f| *f == field) {
                let next = if shift {
                    (i + order.len() - 1) % order.len()
                } else {
                    (i + 1) % order.len()
                };
                self.agenda_focus_field(order[next]);
            }
            return true;
        }
        let rect = self.sched.layout.field(field).unwrap_or(Rect::ZERO);
        let Some(pn) = self.sched.view.panel.as_mut() else {
            return false;
        };
        let Some(f) = pn.field_mut(field) else {
            return false;
        };
        let result = if field.multiline() {
            f.multiline_key(rect, key, shift, ctrl)
        } else {
            f.key(key, shift, ctrl)
        };
        match result {
            FieldKey::Submit if field.multiline() && !ctrl => {}
            FieldKey::Submit => {
                if field == Field::NewStep {
                    self.agenda_add_step();
                } else if pn.is_new() {
                    self.agenda_panel_create();
                } else {
                    self.agenda_blur();
                    self.agenda_commit_panel();
                }
            }
            FieldKey::Cancel => {
                if let Some(text) = pn
                    .loaded
                    .iter()
                    .find(|(f, _)| *f == field)
                    .map(|(_, t)| t.clone())
                {
                    if let Some(f) = pn.field_mut(field) {
                        f.set_text(&text);
                    }
                }
                self.agenda_blur();
            }
            _ => {}
        }
        !matches!(result, FieldKey::Ignored)
    }

    fn agenda_add_step(&mut self) {
        let Some(pn) = self.sched.view.panel.as_mut() else {
            return;
        };
        let Some(id) = pn.record_id().map(str::to_owned) else {
            return;
        };
        let text = pn
            .field_mut(Field::NewStep)
            .map(|f| f.text().trim().to_owned())
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        if self
            .agenda_mutate(None, |ed| ed.add_step(&id, &text))
            .is_some()
        {
            if let Some(f) = self
                .sched
                .view
                .panel
                .as_mut()
                .and_then(|p| p.field_mut(Field::NewStep))
            {
                f.clear();
            }
        }
    }

    pub(super) fn agenda_panel_hit(&mut self, hit: panel::Hit, x: f32, y: f32) {
        let Some(pn) = self.sched.view.panel.as_ref() else {
            return;
        };
        let id = pn.record_id().map(str::to_owned);
        let is_new = pn.is_new();
        let key = id.clone().unwrap_or_default();
        match hit {
            panel::Hit::Close => {
                self.agenda_blur();
                self.sched.view.panel = None;
            }
            panel::Hit::Create => self.agenda_panel_create(),
            panel::Hit::Field(field) => {
                let Some(rect) = self.sched.layout.field(field) else {
                    return;
                };
                let Some(pn) = self.sched.view.panel.as_mut() else {
                    return;
                };
                pn.focus = Some(field);
                self.focus = Focus::ScheduleForm;
                if let Some(f) = pn.field_mut(field) {
                    if field.multiline() {
                        f.multiline_click(rect, x, y);
                    } else {
                        f.click(x - rect.left - 10.0, false);
                    }
                }
            }
            panel::Hit::Open(target) => self.agenda_open_in_place(&target),
            panel::Hit::NewKind(kind) => {
                let Some(pn) = self.sched.view.panel.as_ref() else {
                    return;
                };
                let title = pn
                    .fields
                    .iter()
                    .find(|(f, _)| *f == Field::Title)
                    .map(|(_, t)| t.text().to_owned());
                let date = if self.sched.view.view == View::Day {
                    self.sched.view.date
                } else {
                    at::today()
                };
                let mut fresh = panel::Panel::new_record(kind, date, None, None);
                if let (Some(title), Some(f)) = (title, fresh.field_mut(Field::Title)) {
                    f.set_text(&title);
                }
                self.sched.view.panel = Some(fresh);
                self.agenda_focus_field(Field::Title);
            }
            panel::Hit::TaskStatus(status) => {
                let label = match status {
                    TaskStatus::Done => "任务已完成 · 目标不会因此自动达成",
                    TaskStatus::Cancelled => "任务已取消",
                    TaskStatus::Doing => "任务进行中",
                    TaskStatus::Todo => "任务回到待办 · 之前取消的安排不会恢复",
                };
                if let Some(future) =
                    self.agenda_mutate(Some(label.into()), |ed| ed.set_task_status(&key, status))
                {
                    if matches!(status, TaskStatus::Done | TaskStatus::Cancelled) {
                        let reason = if status == TaskStatus::Done {
                            "任务已完成"
                        } else {
                            "任务已取消"
                        };
                        self.agenda_ask_future(&key, future.entry_ids, reason);
                    }
                }
            }
            panel::Hit::GoalStatus(status) => {
                let label = match status {
                    GoalStatus::Achieved => "目标已达成 · 愿望不会因此自动实现",
                    GoalStatus::Abandoned => "目标已放弃 · 它下面的任务都还在",
                    GoalStatus::Paused => "目标已暂停",
                    GoalStatus::Active => "目标进行中",
                };
                self.agenda_mutate(Some(label.into()), |ed| {
                    ed.set_goal_status(&key, status, None)
                });
            }
            panel::Hit::WishStatus(status) => {
                let label = match status {
                    WishStatus::Realized => "愿望已实现",
                    WishStatus::Dropped => "愿望已放下",
                    WishStatus::Open => "愿望重新打开",
                };
                self.agenda_mutate(Some(label.into()), |ed| ed.set_wish_status(&key, status));
            }
            panel::Hit::EntryStatus(status) => self.agenda_confirm(&key, status),
            panel::Hit::ProjectStatus(status) => {
                let label = if status == ProjectStatus::Done {
                    "项目已完成"
                } else {
                    "项目进行中"
                };
                self.agenda_mutate(Some(label.into()), |ed| {
                    batch::run(
                        ed,
                        &[json!({"op": "status", "id": key, "status": status.wire()})],
                    )
                    .map(|_| ())
                });
            }
            panel::Hit::RoutineActive(active) => {
                let status = if active {
                    RoutineStatus::Active
                } else {
                    RoutineStatus::Paused
                };
                let label = if active {
                    "重复安排已恢复"
                } else {
                    "已暂停，之后不再生成"
                };
                self.agenda_mutate(Some(label.into()), |ed| ed.set_routine_status(&key, status));
            }
            panel::Hit::Priority(priority) => {
                if is_new {
                    if let Some(pn) = self.sched.view.panel.as_mut() {
                        pn.picks.priority = priority;
                    }
                } else {
                    self.agenda_patch(&key, "priority", json!(priority.wire()));
                }
            }
            panel::Hit::AllDay(all_day) => {
                if let Some(pn) = self.sched.view.panel.as_mut() {
                    pn.picks.all_day = all_day;
                }
            }
            panel::Hit::Color(color) => {
                if is_new {
                    if let Some(pn) = self.sched.view.panel.as_mut() {
                        pn.picks.color = color;
                    }
                } else {
                    self.agenda_patch(
                        &key,
                        "color",
                        color.map_or(serde_json::Value::Null, |c| json!(c)),
                    );
                }
            }
            panel::Hit::Pick(slot) => self.agenda_open_picker(slot, x, y),
            panel::Hit::Unlink(slot) => self.agenda_pick(slot, None),
            panel::Hit::Step(step) => {
                self.agenda_mutate(None, |ed| ed.toggle_step(&key, &step));
            }
            panel::Hit::StepRemove(step) => {
                self.agenda_mutate(None, |ed| ed.remove_step(&key, &step));
            }
            panel::Hit::ScheduleTask => self.agenda_schedule_next(&key),
            panel::Hit::CompleteBlockAndTask => self.agenda_complete_block_and_task(&key),
            panel::Hit::NewChild(kind) => {
                let parent_kind = Kind::of_id(&key);
                let date = if self.sched.view.view == View::Day {
                    self.sched.view.date
                } else {
                    at::today()
                };
                self.agenda_commit_panel();
                let mut fresh = match parent_kind {
                    Some(Kind::Project) => {
                        let mut p = panel::Panel::new_record(kind, date, None, None);
                        p.picks.project = Some(key.clone());
                        p
                    }
                    _ => panel::Panel::new_record(kind, date, None, Some(key.clone())),
                };
                fresh.focus = None;
                self.sched.view.panel = Some(fresh);
                self.agenda_focus_field(Field::Title);
            }
            panel::Hit::RulePreset(rule) => {
                if let Some(f) = self
                    .sched
                    .view
                    .panel
                    .as_mut()
                    .and_then(|p| p.field_mut(Field::Rule))
                {
                    f.set_text(rule);
                }
                if !is_new {
                    self.agenda_commit_panel();
                }
            }
            panel::Hit::Archive(archived) => {
                let label = if archived {
                    "已归档 · 只是不再显示，关联都保留"
                } else {
                    "已取消归档"
                };
                self.agenda_mutate(Some(label.into()), |ed| ed.set_archived(&key, archived));
            }
            panel::Hit::Delete => self.agenda_delete(&key),
            panel::Hit::Restore => {
                self.agenda_mutate(Some("已恢复".into()), |ed| ed.restore(&key));
            }
            panel::Hit::KeepFuture => {
                if let Some(pn) = self.sched.view.panel.as_mut() {
                    pn.prompt = None;
                }
                self.sched.view.toast = Some(Toast {
                    message: "后续安排都保留着".into(),
                    undo: false,
                });
            }
            panel::Hit::CancelFuture => {
                let Some(prompt) = self.sched.view.panel.as_mut().and_then(|p| p.prompt.take())
                else {
                    return;
                };
                let n = prompt.entry_ids.len();
                self.agenda_mutate(
                    Some(format!("已取消 {n} 个未开始的时间块")),
                    |ed| Ok(ed.cancel_future_entries(&prompt.entry_ids)),
                );
            }
        }
    }

    fn agenda_patch(&mut self, id: &str, key: &str, value: serde_json::Value) {
        let mut patch = Map::new();
        patch.insert(key.into(), value);
        self.agenda_mutate(None, |ed| ed.update(id, &patch));
    }

    /// 「再安排一次」：从当前查看的日子起找下一段放得下的空闲，新增一个时间块。
    fn agenda_schedule_next(&mut self, task_id: &str) {
        let minutes = {
            let Some(data) = self.sched.view.data.as_ref() else {
                return;
            };
            let default = data.settings.default_block_minutes.max(15) as i64;
            data.task(task_id)
                .map(|t| {
                    let facts = query::task_facts(data, t, self.sched.view.now);
                    match t.estimate_minutes {
                        Some(total)
                            if total as i64 - facts.done_minutes - facts.planned_minutes >= 15 =>
                        {
                            (total as i64 - facts.done_minutes - facts.planned_minutes).min(120)
                        }
                        _ => default,
                    }
                })
                .unwrap_or(default)
        };
        let st = &self.sched.view;
        let from = st.date.max(st.today());
        let Some(data) = st.data.as_ref() else { return };
        let Some((start, end)) = crate::ui::agenda::next_free(data, from, st.now, minutes) else {
            self.sched.view.error =
                "接下来两周都找不到放得下的空闲时间，可以直接拖到时间轴上".into();
            return;
        };
        if self.agenda_schedule_task(task_id, start, end).is_some() {
            self.sched.view.date = start.date();
            self.sched.view.month = start.date();
            if self.sched.view.view == View::Day {
                self.sched.view.scroll_to_now = true;
            }
            self.agenda_refresh_panel();
        }
    }

    // ---------- 上级 / 项目选择 ----------

    fn agenda_open_picker(&mut self, slot: PickSlot, x: f32, y: f32) {
        let st = &self.sched.view;
        let (Some(pn), Some(data)) = (st.panel.as_ref(), st.data.as_ref()) else {
            return;
        };
        let kind = pn.kind();
        let mut items: Vec<MenuItem<MenuAction>> = Vec::new();
        let pick = |label: String, id: Option<String>| {
            MenuItem::new(label, MenuAction::AgendaPick(slot, id))
        };
        match (slot, kind) {
            (PickSlot::Project, _) => {
                items.push(pick("不归入项目".into(), None));
                for p in data
                    .projects
                    .iter()
                    .filter(|p| p.deleted_at.is_none() && p.archived_at.is_none())
                {
                    items.push(pick(p.name.clone(), Some(p.id.clone())));
                }
            }
            (PickSlot::Parent, Kind::Task | Kind::Routine) => {
                items.push(pick("不属于任何目标".into(), None));
                for g in data.goals.iter().filter(|g| {
                    g.deleted_at.is_none() && g.archived_at.is_none() && g.status.is_open()
                }) {
                    items.push(pick(format!("目标 · {}", g.title), Some(g.id.clone())));
                }
            }
            (PickSlot::Parent, Kind::Goal) => {
                items.push(pick("不属于任何愿望".into(), None));
                for w in data
                    .wishes
                    .iter()
                    .filter(|w| w.deleted_at.is_none() && w.archived_at.is_none())
                {
                    items.push(pick(format!("愿望 · {}", w.title), Some(w.id.clone())));
                }
            }
            (PickSlot::Parent, Kind::Entry) => {
                items.push(pick("独立日程（不关联任务）".into(), None));
                let now = st.now;
                let mut tasks: Vec<&mochi_core::agenda::Task> = data
                    .tasks
                    .iter()
                    .filter(|t| query::is_visible_task(t) && t.status.is_open())
                    .collect();
                query::sort_tasks(&mut tasks, now);
                for t in tasks.into_iter().take(60) {
                    items.push(pick(format!("任务 · {}", t.title), Some(t.id.clone())));
                }
            }
            _ => return,
        }
        let anchor = self
            .sched
            .layout
            .rect_of(&Hit::Panel(panel::Hit::Pick(slot)))
            .unwrap_or_else(|| Rect::new(x, y, x, y));
        let viewport = self.renderer.viewport();
        let mut menu = Menu::open_anchored(items, anchor, viewport);
        let width = anchor
            .width()
            .max(220.0)
            .min((viewport.width() - 16.0).max(1.0));
        let left = anchor.left.clamp(
            viewport.left + 8.0,
            (viewport.right - width - 8.0).max(viewport.left + 8.0),
        );
        menu.rect = Rect::from_size(left, menu.rect.top, width, menu.rect.height());
        self.menu = Some(menu);
    }

    /// 选择器选中后：新建面板只记下；已有记录立即建立 / 解除关联（不改子项的标题、日期、状态）。
    pub(super) fn agenda_pick(&mut self, slot: PickSlot, value: Option<String>) {
        let Some(pn) = self.sched.view.panel.as_mut() else {
            return;
        };
        if pn.is_new() {
            match slot {
                PickSlot::Parent => pn.picks.parent = value,
                PickSlot::Project => pn.picks.project = value,
            }
            return;
        }
        let Some(id) = pn.record_id().map(str::to_owned) else {
            return;
        };
        match slot {
            PickSlot::Parent => {
                let label = if value.is_some() {
                    "已关联 · 标题、日期和状态都没变"
                } else {
                    "已解除关联"
                };
                self.agenda_mutate(Some(label.into()), |ed| {
                    let id = if id.contains('@') {
                        ed.resolve_entry(&id)?
                    } else {
                        id.clone()
                    };
                    ed.link(&id, value.as_deref())
                });
            }
            PickSlot::Project => {
                self.agenda_patch(
                    &id,
                    "project_id",
                    value.map_or(serde_json::Value::Null, |v| json!(v)),
                );
            }
        }
        self.agenda_refresh_panel();
    }
}
