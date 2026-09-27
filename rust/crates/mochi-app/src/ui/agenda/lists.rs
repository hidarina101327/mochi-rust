//! 清单类视图：待办河流、待确认、愿望与目标、重复安排、项目、回收站、操作记录、搜索。

use super::day::paint_todo_card;
use super::visual::{self, Colors};
use super::*;

/// 可滚动清单的公共骨架。
struct Scroll {
    view: Rect,
    y: f32,
    scroll: f32,
}

impl Scroll {
    fn begin(list: &mut DrawList, area: Rect, st: &State) -> Self {
        let view = Rect::new(area.left, area.top, area.right, area.bottom);
        list.push_clip(view);
        Self {
            view,
            y: view.top + 12.0 - st.scroll,
            scroll: st.scroll,
        }
    }

    fn left(&self) -> f32 {
        self.view.left + 24.0
    }

    fn right(&self) -> f32 {
        (self.view.right - 24.0).min(self.view.left + 24.0 + 860.0)
    }

    fn visible(&self, h: f32) -> bool {
        self.y + h >= self.view.top && self.y <= self.view.bottom
    }

    fn end(self, list: &mut DrawList, lay: &mut Layout) {
        list.pop_clip();
        lay.viewport = self.view;
        lay.content_height = self.y + self.scroll - self.view.top + 24.0;
    }
}

fn heading(list: &mut DrawList, s: &mut Scroll, title: &str, color: u32) {
    list.text(
        Rect::new(s.left(), s.y + 8.0, s.right(), s.y + 28.0),
        title.to_owned(),
        TextStyle::Caption,
        color,
    );
    s.y += 32.0;
}

/// 一行：图标 + 标题 + 元信息，整行可点。
#[allow(clippy::too_many_arguments)]
fn row(
    list: &mut DrawList,
    lay: &mut Layout,
    s: &mut Scroll,
    icon: Icon,
    color: u32,
    title: &str,
    meta: &str,
    hit: Hit,
    indent: f32,
    p: &Palette,
) -> Rect {
    let h = if meta.is_empty() { 34.0 } else { 46.0 }
        + crate::ui::settings_values::number("schedule.listExtraSpacing", 0.0);
    let r = Rect::new(s.left() + indent, s.y, s.right(), s.y + h - 4.0);
    if s.visible(h) {
        list.rounded_rect(r, 8.0, p.surface_muted);
        list.icon_centered(
            Rect::new(r.left + 8.0, r.top, r.left + 30.0, r.top + 30.0),
            icon,
            14.0,
            color,
        );
        list.text_aligned(
            Rect::new(r.left + 36.0, r.top + 4.0, r.right - 8.0, r.top + 26.0),
            text::ellipsize(title, TextStyle::Small, r.width() - 48.0),
            TextStyle::Small,
            p.foreground,
            Align::Leading,
        );
        if !meta.is_empty() {
            list.text_aligned(
                Rect::new(r.left + 36.0, r.top + 22.0, r.right - 8.0, r.bottom - 2.0),
                text::ellipsize(meta, TextStyle::Tiny, r.width() - 48.0),
                TextStyle::Tiny,
                p.muted,
                Align::Leading,
            );
        }
        lay.push_clipped(r, s.view, hit);
    }
    s.y += h;
    r
}

fn small_button(
    list: &mut DrawList,
    lay: &mut Layout,
    clip: Rect,
    right: f32,
    top: f32,
    label: &str,
    style: Btn,
    hit: Hit,
    p: &Palette,
) -> f32 {
    let w = button_width(label, None);
    let r = Rect::new(right - w, top, right, top + 26.0);
    if r.bottom > clip.top && r.top < clip.bottom {
        button(list, lay, r, label, None, style, hit.clone(), p);
        // 重新按裁剪区登记，避免滚出去的按钮还能点。
        lay.entries.pop();
        lay.push_clipped(r, clip, hit);
    }
    right - w - 6.0
}

// ---------- 待办河流 ----------

pub fn paint_river(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let (groups, undated) = query::deadline_river(data, st.now);
    let mut s = Scroll::begin(list, area, st);
    let today = st.today();
    if groups.is_empty() && undated.is_empty() && !st.show_done {
        empty_state(
            list,
            area,
            "没有待办",
            "在上方输入「任务：……」或「周五前 交报告」即可添加",
            p,
        );
    }
    let rail_x = s.left() + 64.0;
    for (date, ids) in &groups {
        let overdue = *date < today;
        let color = if overdue {
            c.danger
        } else if *date == today {
            c.warning
        } else {
            p.muted
        };
        let label = at::relative_date_label(*date, today);
        let group_top = s.y;
        list.text_aligned(
            Rect::new(s.left(), s.y + 2.0, rail_x - 12.0, s.y + 22.0),
            label,
            TextStyle::Small,
            color,
            Align::Trailing,
        );
        if overdue {
            list.text_aligned(
                Rect::new(s.left(), s.y + 20.0, rail_x - 12.0, s.y + 36.0),
                format!("逾期 {} 天", (today - *date).num_days()),
                TextStyle::Tiny,
                c.danger,
                Align::Trailing,
            );
        } else {
            list.text_aligned(
                Rect::new(s.left(), s.y + 20.0, rail_x - 12.0, s.y + 36.0),
                at::weekday_label(*date),
                TextStyle::Tiny,
                p.muted,
                Align::Trailing,
            );
        }
        list.rounded_rect(
            Rect::new(rail_x - 4.0, s.y + 8.0, rail_x + 4.0, s.y + 16.0),
            4.0,
            color,
        );
        for id in ids {
            let Some(task) = data.task(id) else { continue };
            let r = Rect::new(rail_x + 16.0, s.y, s.right(), s.y + 48.0);
            if s.visible(54.0) {
                paint_todo_card(list, lay, s.view, r, task, data, st, &c, p);
            }
            s.y += 54.0;
        }
        list.vline(rail_x, group_top + 18.0, s.y + 6.0, p.border);
        s.y += 10.0;
    }
    if !undated.is_empty() {
        heading(
            list,
            &mut s,
            &format!("没有截止时间 · {}", undated.len()),
            p.muted,
        );
        for id in &undated {
            let Some(task) = data.task(id) else { continue };
            let r = Rect::new(rail_x + 16.0, s.y, s.right(), s.y + 48.0);
            if s.visible(54.0) {
                paint_todo_card(list, lay, s.view, r, task, data, st, &c, p);
            }
            s.y += 54.0;
        }
    }
    if st.show_done {
        let mut done: Vec<&core::Task> = data
            .tasks
            .iter()
            .filter(|t| query::is_visible_task(t) && t.status == TaskStatus::Done)
            .collect();
        done.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));
        if !done.is_empty() {
            heading(list, &mut s, &format!("已完成 · {}", done.len()), c.success);
            for task in done.into_iter().take(60) {
                let r = Rect::new(rail_x + 16.0, s.y, s.right(), s.y + 48.0);
                if s.visible(54.0) {
                    paint_todo_card(list, lay, s.view, r, task, data, st, &c, p);
                }
                s.y += 54.0;
            }
        }
    }
    s.end(list, lay);
}

// ---------- 待确认 ----------

pub fn paint_confirm(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let queue = query::confirm_queue(data, st.now);
    if queue.is_empty() {
        empty_state(
            list,
            area,
            "没有待确认的日程",
            "时间过去的日程会出现在这里，由你确认是否真的执行了——系统从不替你判断",
            p,
        );
        lay.viewport = area;
        lay.content_height = 0.0;
        return;
    }
    let mut s = Scroll::begin(list, area, st);
    list.text(
        Rect::new(s.left(), s.y, s.right(), s.y + 22.0),
        "这些日程的时间已经过去。执行了吗？标记已执行不会自动完成关联的任务。",
        TextStyle::Small,
        p.muted,
    );
    s.y += 32.0;
    for slot in &queue {
        let h = 64.0;
        let r = Rect::new(s.left(), s.y, s.right(), s.y + h - 8.0);
        if s.visible(h) {
            let color = visual::slot_color(data, slot, &c);
            list.rounded_rect(r, 8.0, p.surface_muted);
            list.rounded_rect(
                Rect::new(r.left, r.top + 8.0, r.left + 3.0, r.bottom - 8.0),
                1.5,
                color,
            );
            lay.push_clipped(r, s.view, Hit::Open(slot.key.clone()));
            let mut title = slot.title.clone();
            if slot.task_deleted {
                title.push_str("（原任务已删除）");
            }
            let when = format!(
                "{} {}–{} · {}",
                at::relative_date_label(slot.start.date(), st.today()),
                at::hm(slot.start.time()),
                at::hm(slot.end.time()),
                at::duration_label(slot.minutes())
            );
            let kind = if slot.routine_id.is_some() {
                "重复安排"
            } else if slot.task_id.is_some() {
                "任务时间块"
            } else {
                "日程"
            };
            let buttons_w = 320.0;
            list.text_aligned(
                Rect::new(
                    r.left + 14.0,
                    r.top + 6.0,
                    r.right - buttons_w,
                    r.top + 28.0,
                ),
                text::ellipsize(&title, TextStyle::Label, r.width() - buttons_w - 20.0),
                TextStyle::Label,
                p.foreground,
                Align::Leading,
            );
            list.text_aligned(
                Rect::new(
                    r.left + 14.0,
                    r.top + 28.0,
                    r.right - buttons_w,
                    r.bottom - 4.0,
                ),
                format!("{kind} · {when}"),
                TextStyle::Caption,
                p.muted,
                Align::Leading,
            );
            let top = r.top + (r.height() - 26.0) / 2.0;
            let mut x = r.right - 10.0;
            x = small_button(
                list,
                lay,
                s.view,
                x,
                top,
                "未执行",
                Btn::Ghost,
                Hit::Confirm(slot.key.clone(), EntryStatus::Skipped),
                p,
            );
            x = small_button(
                list,
                lay,
                s.view,
                x,
                top,
                "已执行",
                Btn::Primary,
                Hit::Confirm(slot.key.clone(), EntryStatus::Done),
                p,
            );
            if slot.task_id.is_some() && slot.task_status.is_some_and(|t| t.is_open()) {
                small_button(
                    list,
                    lay,
                    s.view,
                    x,
                    top,
                    "已执行并完成任务",
                    Btn::Ghost,
                    Hit::ConfirmAndComplete(slot.key.clone()),
                    p,
                );
            }
        }
        s.y += h;
    }
    s.end(list, lay);
}

// ---------- 愿望与目标 ----------

fn live<T>(deleted: &Option<String>, archived: &Option<String>, _t: &T) -> bool {
    deleted.is_none() && archived.is_none()
}

pub fn paint_goals(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let mut s = Scroll::begin(list, area, st);
    let wishes: Vec<&core::Wish> = data
        .wishes
        .iter()
        .filter(|w| live(&w.deleted_at, &w.archived_at, w))
        .collect();
    let goals: Vec<&core::Goal> = data
        .goals
        .iter()
        .filter(|g| live(&g.deleted_at, &g.archived_at, g))
        .collect();
    if wishes.is_empty() && goals.is_empty() {
        empty_state(
            list,
            area,
            "还没有愿望或目标",
            "愿望是想要的状态，目标是可以判断达成的结果，任务是具体要做的事。它们各自独立完成，不会互相代劳。",
            p,
        );
    }
    for wish in &wishes {
        let progress = query::wish_progress(data, &wish.id, st.now);
        let meta = format!(
            "{} · {} 个目标，已达成 {} · 累计投入 {}",
            wish.status.label(),
            progress.goals_total,
            progress.goals_achieved,
            at::duration_label(progress.invested_minutes)
        );
        row(
            list,
            lay,
            &mut s,
            Icon::SPARKLES,
            c.wish,
            &wish.title,
            &meta,
            Hit::Open(wish.id.clone()),
            0.0,
            p,
        );
        for goal in goals
            .iter()
            .filter(|g| g.wish_id.as_deref() == Some(wish.id.as_str()))
        {
            paint_goal(list, lay, &mut s, goal, data, st, &c, 28.0, p);
        }
        s.y += 8.0;
    }
    let orphans: Vec<&&core::Goal> = goals
        .iter()
        .filter(|g| {
            g.wish_id
                .as_deref()
                .is_none_or(|w| data.wish(w).is_none_or(|w| w.deleted_at.is_some()))
        })
        .collect();
    if !orphans.is_empty() {
        heading(list, &mut s, "独立目标", p.muted);
        for goal in orphans {
            paint_goal(list, lay, &mut s, goal, data, st, &c, 0.0, p);
        }
    }
    s.end(list, lay);
}

#[allow(clippy::too_many_arguments)]
fn paint_goal(
    list: &mut DrawList,
    lay: &mut Layout,
    s: &mut Scroll,
    goal: &core::Goal,
    data: &AgendaData,
    st: &State,
    c: &Colors,
    indent: f32,
    p: &Palette,
) {
    let progress = query::goal_progress(data, &goal.id, st.now);
    let mut meta = vec![goal.status.label().to_owned()];
    if progress.criteria_total > 0 {
        meta.push(format!(
            "标准 {}/{}",
            progress.criteria_done, progress.criteria_total
        ));
    }
    meta.push(format!(
        "任务 {}/{}",
        progress.tasks_done, progress.tasks_total
    ));
    if progress.tasks_overdue > 0 {
        meta.push(format!("{} 项逾期", progress.tasks_overdue));
    }
    if let Some(target) = &goal.target_date {
        meta.push(format!("目标日 {}", at::due_label(target, st.today())));
    }
    if progress.invested_minutes > 0 {
        meta.push(format!(
            "投入 {}",
            at::duration_short(progress.invested_minutes)
        ));
    }
    let r = row(
        list,
        lay,
        s,
        Icon::TARGET,
        c.goal,
        &goal.title,
        &meta.join(" · "),
        Hit::Open(goal.id.clone()),
        indent,
        p,
    );
    if s.visible(0.0) || r.bottom > s.view.top {
        let bar = Rect::new(r.right - 120.0, r.top + 14.0, r.right - 12.0, r.top + 18.0);
        visual::progress_bar(list, bar, progress.ratio(), c.goal, p);
    }
    let mut tasks: Vec<&core::Task> = data
        .tasks
        .iter()
        .filter(|t| query::is_visible_task(t) && t.goal_id.as_deref() == Some(goal.id.as_str()))
        .filter(|t| t.status.is_open() || st.show_done)
        .collect();
    query::sort_tasks(&mut tasks, st.now);
    for task in tasks.iter().take(8) {
        let h = 30.0;
        let tr = Rect::new(s.left() + indent + 34.0, s.y, s.right(), s.y + h - 4.0);
        if s.visible(h) {
            let done = task.status == TaskStatus::Done;
            let check = Rect::new(tr.left, tr.top, tr.left + 22.0, tr.bottom);
            let overdue = query::is_overdue(task, st.now);
            visual::check_circle(
                list,
                check,
                done,
                if overdue { c.danger } else { p.muted },
                p,
            );
            let meta = task
                .due
                .as_deref()
                .map(|d| format!("  ·  截止 {}", at::due_label(d, st.today())))
                .unwrap_or_default();
            list.text_aligned(
                Rect::new(tr.left + 28.0, tr.top, tr.right - 8.0, tr.bottom),
                text::ellipsize(
                    &format!("{}{meta}", task.title),
                    TextStyle::Small,
                    tr.width() - 36.0,
                ),
                TextStyle::Small,
                if done {
                    p.muted
                } else if overdue {
                    c.danger
                } else {
                    p.foreground
                },
                Align::Leading,
            );
            lay.push_clipped(tr, s.view, Hit::Open(task.id.clone()));
            lay.push_clipped(check, s.view, Hit::Check(task.id.clone()));
        }
        s.y += h;
    }
    if tasks.len() > 8 {
        list.text(
            Rect::new(s.left() + indent + 62.0, s.y, s.right(), s.y + 20.0),
            format!("还有 {} 项任务", tasks.len() - 8),
            TextStyle::Tiny,
            p.muted,
        );
        s.y += 22.0;
    }
    s.y += 4.0;
}

// ---------- 重复安排 ----------

pub fn paint_routines(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let routines: Vec<&core::Routine> = data
        .routines
        .iter()
        .filter(|r| live(&r.deleted_at, &r.archived_at, r))
        .collect();
    if routines.is_empty() {
        empty_state(
            list,
            area,
            "还没有重复安排",
            "在上方输入「每天 7:00 跑步 30分钟」或「每周一 9 点 周会」",
            p,
        );
        lay.viewport = area;
        return;
    }
    let mut s = Scroll::begin(list, area, st);
    for routine in routines {
        let h = 92.0;
        let r = Rect::new(s.left(), s.y, s.right(), s.y + h - 10.0);
        if s.visible(h) {
            let active = routine.status == core::RoutineStatus::Active;
            let color = routine
                .color
                .as_deref()
                .and_then(visual::parse_color)
                .unwrap_or(c.routine);
            list.rounded_rect(r, 10.0, p.surface_muted);
            lay.push_clipped(r, s.view, Hit::Open(routine.id.clone()));
            list.icon_centered(
                Rect::new(r.left + 10.0, r.top + 8.0, r.left + 32.0, r.top + 32.0),
                Icon::REPEAT,
                15.0,
                if active { color } else { p.muted },
            );
            list.text_aligned(
                Rect::new(r.left + 40.0, r.top + 8.0, r.right - 120.0, r.top + 32.0),
                text::ellipsize(&routine.title, TextStyle::Label, r.width() - 170.0),
                TextStyle::Label,
                if active { p.foreground } else { p.muted },
                Align::Leading,
            );
            let mut when = core::recur::describe(&routine.rule);
            match &routine.time {
                Some(t) => when.push_str(&format!(" {t}")),
                None => when.push_str(" · 全天"),
            }
            if routine.duration_minutes > 0 {
                when.push_str(&format!(
                    " · {}",
                    at::duration_label(routine.duration_minutes as i64)
                ));
            }
            if let Some(goal) = routine.goal_id.as_deref() {
                when.push_str(&format!(
                    " · 为了「{}」",
                    data.title_of(goal).unwrap_or_default()
                ));
            }
            list.text_aligned(
                Rect::new(r.left + 40.0, r.top + 30.0, r.right - 120.0, r.top + 50.0),
                text::ellipsize(&when, TextStyle::Caption, r.width() - 170.0),
                TextStyle::Caption,
                p.muted,
                Align::Leading,
            );
            let stats = query::routine_stats(data, routine, st.today());
            let mut x = r.left + 40.0;
            for (_, status) in &stats.recent {
                let dot = Rect::new(x, r.top + 58.0, x + 12.0, r.top + 70.0);
                let fill = match status {
                    Some(EntryStatus::Done) => color,
                    Some(EntryStatus::Skipped) => theme::mix(c.danger, p.surface, 0.5),
                    Some(_) => p.border,
                    None => p.background,
                };
                list.rounded_rect(dot, 3.0, fill);
                if status.is_none() {
                    list.rounded_border(dot, 3.0, p.border);
                }
                x += 15.0;
            }
            list.text_aligned(
                Rect::new(x + 8.0, r.top + 54.0, r.right - 120.0, r.top + 74.0),
                format!(
                    "连续 {} 次 · 近 30 天 {}/{}",
                    stats.streak, stats.done_30, stats.expected_30
                ),
                TextStyle::Tiny,
                p.muted,
                Align::Leading,
            );
            let label = if active { "暂停" } else { "恢复" };
            small_button(
                list,
                lay,
                s.view,
                r.right - 12.0,
                r.top + 10.0,
                label,
                Btn::Ghost,
                Hit::RoutineToggle(routine.id.clone()),
                p,
            );
        }
        s.y += h;
    }
    s.end(list, lay);
}

// ---------- 项目 ----------

pub fn paint_projects(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let projects: Vec<&core::Project> = data
        .projects
        .iter()
        .filter(|x| live(&x.deleted_at, &x.archived_at, x))
        .collect();
    if projects.is_empty() {
        empty_state(
            list,
            area,
            "还没有项目",
            "项目是横向的归类：任务、日程、目标都可以归入同一个项目",
            p,
        );
        lay.viewport = area;
        return;
    }
    let mut s = Scroll::begin(list, area, st);
    for project in projects {
        let open = data
            .tasks
            .iter()
            .filter(|t| {
                query::is_visible_task(t)
                    && t.status.is_open()
                    && t.project_id.as_deref() == Some(project.id.as_str())
            })
            .count();
        let upcoming = data
            .entries
            .iter()
            .filter(|e| {
                e.status == EntryStatus::Planned
                    && e.project_id.as_deref() == Some(project.id.as_str())
            })
            .filter(|e| at::parse_stamp(&e.start).is_some_and(|t| t >= st.now))
            .count();
        let goals = data
            .goals
            .iter()
            .filter(|g| {
                g.deleted_at.is_none() && g.project_id.as_deref() == Some(project.id.as_str())
            })
            .count();
        let color = project
            .color
            .as_deref()
            .and_then(visual::parse_color)
            .unwrap_or(c.muted);
        let meta = format!("{open} 项未完成任务 · {upcoming} 个即将到来的日程 · {goals} 个目标");
        row(
            list,
            lay,
            &mut s,
            Icon::FOLDER_KANBAN,
            color,
            &project.name,
            &meta,
            Hit::Open(project.id.clone()),
            0.0,
            p,
        );
    }
    s.end(list, lay);
}

// ---------- 回收站 ----------

pub fn paint_trash(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let items = query::trash(data);
    let archived: Vec<(Kind, String, String)> = archived_items(data);
    if items.is_empty() && archived.is_empty() {
        empty_state(
            list,
            area,
            "回收站是空的",
            "删除的记录会留在这里，可以恢复",
            p,
        );
        lay.viewport = area;
        return;
    }
    let mut s = Scroll::begin(list, area, st);
    if !items.is_empty() {
        heading(list, &mut s, &format!("已删除 · {}", items.len()), p.muted);
    }
    for (kind, id, title, deleted_at) in &items {
        let meta = format!("{} · 删除于 {}", kind.label(), deleted_at.replace('T', " "));
        let r = row(
            list,
            lay,
            &mut s,
            visual::kind_icon(*kind),
            visual::kind_color(&c, *kind),
            title,
            &meta,
            Hit::Blank,
            0.0,
            p,
        );
        let top = r.top + (r.height() - 26.0) / 2.0;
        let x = small_button(
            list,
            lay,
            s.view,
            r.right - 8.0,
            top,
            "彻底删除",
            Btn::Danger,
            Hit::Purge(id.clone()),
            p,
        );
        small_button(
            list,
            lay,
            s.view,
            x,
            top,
            "恢复",
            Btn::Ghost,
            Hit::Restore(id.clone()),
            p,
        );
    }
    if !archived.is_empty() {
        heading(
            list,
            &mut s,
            &format!("已归档 · {}", archived.len()),
            p.muted,
        );
        for (kind, id, title) in &archived {
            row(
                list,
                lay,
                &mut s,
                visual::kind_icon(*kind),
                visual::kind_color(&c, *kind),
                title,
                kind.label(),
                Hit::Open(id.clone()),
                0.0,
                p,
            );
        }
    }
    s.end(list, lay);
}

fn archived_items(data: &AgendaData) -> Vec<(Kind, String, String)> {
    let mut out = Vec::new();
    for w in data
        .wishes
        .iter()
        .filter(|x| x.deleted_at.is_none() && x.archived_at.is_some())
    {
        out.push((Kind::Wish, w.id.clone(), w.title.clone()));
    }
    for g in data
        .goals
        .iter()
        .filter(|x| x.deleted_at.is_none() && x.archived_at.is_some())
    {
        out.push((Kind::Goal, g.id.clone(), g.title.clone()));
    }
    for t in data
        .tasks
        .iter()
        .filter(|x| x.deleted_at.is_none() && x.archived_at.is_some())
    {
        out.push((Kind::Task, t.id.clone(), t.title.clone()));
    }
    for r in data
        .routines
        .iter()
        .filter(|x| x.deleted_at.is_none() && x.archived_at.is_some())
    {
        out.push((Kind::Routine, r.id.clone(), r.title.clone()));
    }
    for x in data
        .projects
        .iter()
        .filter(|x| x.deleted_at.is_none() && x.archived_at.is_some())
    {
        out.push((Kind::Project, x.id.clone(), x.name.clone()));
    }
    out
}

// ---------- 操作记录 ----------

pub fn paint_history(list: &mut DrawList, area: Rect, st: &State, lay: &mut Layout, p: &Palette) {
    if st.history.is_empty() {
        empty_state(
            list,
            area,
            "还没有操作记录",
            "你、墨池 AI、桌面卡片对日程待办的每次修改都会记在这里",
            p,
        );
        lay.viewport = area;
        return;
    }
    let mut s = Scroll::begin(list, area, st);
    for entry in &st.history {
        let h = 28.0 + entry.lines.len().min(6) as f32 * 20.0 + 10.0;
        if s.visible(h) {
            list.text(
                Rect::new(s.left(), s.y + 4.0, s.right(), s.y + 24.0),
                format!(
                    "{} · {}",
                    entry
                        .at
                        .replace('T', " ")
                        .chars()
                        .take(16)
                        .collect::<String>(),
                    entry.source.label()
                ),
                TextStyle::Caption,
                p.muted,
            );
            let mut y = s.y + 26.0;
            for line in entry.lines.iter().take(6) {
                list.text(
                    Rect::new(s.left() + 14.0, y, s.right(), y + 20.0),
                    text::ellipsize(line, TextStyle::Small, s.right() - s.left() - 14.0),
                    TextStyle::Small,
                    p.foreground,
                );
                y += 20.0;
            }
            if entry.lines.len() > 6 {
                list.text(
                    Rect::new(s.left() + 14.0, y, s.right(), y + 18.0),
                    format!("……共 {} 项", entry.lines.len()),
                    TextStyle::Tiny,
                    p.muted,
                );
            }
        }
        s.y += h;
    }
    s.end(list, lay);
}

// ---------- 搜索 ----------

pub fn paint_search(
    list: &mut DrawList,
    area: Rect,
    st: &State,
    data: &AgendaData,
    lay: &mut Layout,
    p: &Palette,
) {
    let c = visual::colors(p);
    let results = query::search(data, st.query.text(), true);
    let mut s = Scroll::begin(list, area, st);
    heading(
        list,
        &mut s,
        &format!(
            "「{}」的搜索结果 · {}",
            st.query.text().trim(),
            results.len()
        ),
        p.muted,
    );
    if results.is_empty() {
        list.text(
            Rect::new(s.left(), s.y, s.right(), s.y + 22.0),
            "没有匹配的记录",
            TextStyle::Small,
            p.muted,
        );
        s.y += 30.0;
    }
    for (kind, id, title) in &results {
        let chain: Vec<String> = query::why_chain(data, id)
            .into_iter()
            .map(|n| n.title)
            .collect();
        let mut meta = kind.label().to_owned();
        if !chain.is_empty() {
            meta.push_str(" · ");
            meta.push_str(&chain.join(" › "));
        }
        if let Some(entry) = data.entry(id) {
            meta.push_str(&format!(" · {}", entry.start.replace('T', " ")));
        }
        row(
            list,
            lay,
            &mut s,
            visual::kind_icon(*kind),
            visual::kind_color(&c, *kind),
            title,
            &meta,
            Hit::Open(id.clone()),
            0.0,
            p,
        );
    }
    s.end(list, lay);
}
