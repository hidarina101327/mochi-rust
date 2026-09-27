//! 日程待办主视图。
//!
//! 两个并列模块共用一个页面：左侧导航选视图，主区是时间轴 / 月历 / 各种清单，
//! 右侧是检查器（查看、编辑、新建都在这里）。绘制是单趟的：一边画一边记录命中区，
//! 控制器（`app/agenda_page.rs`）只和 [`Hit`] 打交道。

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use mochi_core::agenda::planner::{self, Placement};
use mochi_core::agenda::time as at;
use mochi_core::agenda::{
    self as core, query, AgendaData, EntryStatus, Kind, LogEntry, Priority, TaskStatus,
};

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text;
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

pub mod day;
pub mod lists;
pub mod month;
pub mod panel;
pub mod side;
pub mod visual;

/// 主区视图。前三个是用户要求的核心视图，其余是事项与日程的辅助视图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Day,
    Month,
    Todos,
    Confirm,
    Goals,
    Routines,
    Projects,
    Trash,
    History,
}

impl View {
    pub const ALL: [View; 9] = [
        View::Day,
        View::Month,
        View::Todos,
        View::Confirm,
        View::Goals,
        View::Routines,
        View::Projects,
        View::Trash,
        View::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Day => "今日日程",
            View::Month => "月历",
            View::Todos => "待办事项",
            View::Confirm => "待确认",
            View::Goals => "愿望与目标",
            View::Routines => "重复安排",
            View::Projects => "项目",
            View::Trash => "回收站",
            View::History => "操作记录",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            View::Day => Icon::CLOCK,
            View::Month => Icon::CALENDAR_DAYS,
            View::Todos => Icon::LIST_TODO,
            View::Confirm => Icon::BELL,
            View::Goals => Icon::TARGET,
            View::Routines => Icon::REPEAT,
            View::Projects => Icon::FOLDER_KANBAN,
            View::Trash => Icon::TRASH2,
            View::History => Icon::HISTORY,
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            View::Day => "today",
            View::Month => "month",
            View::Todos => "todos",
            View::Confirm => "confirm",
            View::Goals => "goals",
            View::Routines => "routines",
            View::Projects => "projects",
            View::Trash => "trash",
            View::History => "history",
        }
    }

    pub fn from_wire(value: &str) -> Option<View> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "today" | "day" | "week" | "timeline" | "calendar" => View::Day,
            "month" => View::Month,
            "todos" | "tasks" | "todo" => View::Todos,
            "confirm" | "review" => View::Confirm,
            "goals" | "wishes" | "goal" | "wish" => View::Goals,
            "routines" | "habits" | "routine" => View::Routines,
            "projects" | "project" => View::Projects,
            "trash" | "archive" => View::Trash,
            "history" | "log" => View::History,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub undo: bool,
}

/// 时间轴上的拖拽。分钟都是相对 `date` 零点的。
#[derive(Debug, Clone, PartialEq)]
pub enum DragKind {
    /// 在空白处按下拖出一个新日程。
    Create,
    Move {
        key: String,
    },
    Resize {
        key: String,
    },
    /// 把右侧待办拖进时间轴。
    Todo {
        task_id: String,
        minutes: i64,
    },
}

#[derive(Debug, Clone)]
pub struct Drag {
    pub kind: DragKind,
    pub origin: (f32, f32),
    /// 按下点相对块开始的分钟偏移（移动时保持抓取位置）。
    pub grab_min: i64,
    pub date: NaiveDate,
    pub start: i64,
    pub end: i64,
    pub moved: bool,
    /// 指针当前在时间轴网格内（拖待办时在网格外松手等于取消）。
    pub inside: bool,
}

pub const SNAP: i64 = 15;

pub fn snap(minute: i64) -> i64 {
    ((minute as f64 / SNAP as f64).round() as i64 * SNAP).clamp(0, 24 * 60)
}

impl Drag {
    /// 指针移动到 `(x, y)` 后更新拖拽范围。
    pub fn update(&mut self, geo: &day::Geo, x: f32, y: f32) {
        if !self.moved && (x - self.origin.0).abs() < 4.0 && (y - self.origin.1).abs() < 4.0 {
            return;
        }
        self.moved = true;
        self.inside = geo.grid.contains(x, y);
        let minute = geo.minute_at(y);
        if let Some(date) = geo.date_at(x) {
            if !matches!(self.kind, DragKind::Resize { .. } | DragKind::Create) {
                self.date = date;
            }
        }
        match &self.kind {
            DragKind::Create => {
                let anchor = self.grab_min;
                let here = snap(minute);
                let (a, b) = if here >= anchor {
                    (anchor, here.max(anchor + SNAP))
                } else {
                    (here, anchor)
                };
                self.start = a;
                self.end = b.min(24 * 60);
            }
            DragKind::Move { .. } | DragKind::Todo { .. } => {
                let len = (self.end - self.start).max(SNAP);
                let start = snap(minute - self.grab_min).clamp(0, 24 * 60 - len);
                self.start = start;
                self.end = start + len;
            }
            DragKind::Resize { .. } => {
                self.end = snap(minute).max(self.start + SNAP).min(24 * 60);
            }
        }
    }

    pub fn span(&self) -> (NaiveDateTime, NaiveDateTime) {
        (
            at::at_minute(self.date, self.start),
            at::at_minute(self.date, self.end),
        )
    }
}

pub struct State {
    pub data: Option<AgendaData>,
    pub error: String,
    pub toast: Option<Toast>,
    pub view: View,
    /// 时间轴当前日期（周视图时取其所在周）。
    pub date: NaiveDate,
    /// 月历当前月（任意一天）。
    pub month: NaiveDate,
    pub week: bool,
    /// 待办事项用月历展示（否则按截止时间河流展示）。
    pub todo_month: bool,
    pub show_done: bool,
    pub scroll: f32,
    pub todo_scroll: f32,
    pub selected: Option<String>,
    pub quick: TextField,
    pub query: TextField,
    pub drag: Option<Drag>,
    pub pointer: Option<(f32, f32)>,
    pub panel: Option<panel::Panel>,
    /// 自动排入的预览；接受前不写盘。
    pub plan: Option<Vec<Placement>>,
    pub history: Vec<LogEntry>,
    pub scroll_to_now: bool,
    pub now: NaiveDateTime,
}

impl Default for State {
    fn default() -> Self {
        let now = at::now();
        Self {
            data: None,
            error: String::new(),
            toast: None,
            view: View::Day,
            date: now.date(),
            month: now.date(),
            week: false,
            todo_month: false,
            show_done: false,
            scroll: 0.0,
            todo_scroll: 0.0,
            selected: None,
            quick: TextField::new(""),
            query: TextField::new(""),
            drag: None,
            pointer: None,
            panel: None,
            plan: None,
            history: Vec::new(),
            scroll_to_now: true,
            now,
        }
    }
}

impl State {
    pub fn today(&self) -> NaiveDate {
        self.now.date()
    }

    pub fn set_view(&mut self, view: View) {
        if self.view != view {
            self.view = view;
            self.scroll = 0.0;
            self.selected = None;
            self.drag = None;
            if view == View::Day {
                self.scroll_to_now = true;
            }
        }
    }

    pub fn goto_day(&mut self, date: NaiveDate) {
        self.date = date;
        self.month = date;
        self.plan = None;
        self.set_view(View::Day);
        self.scroll_to_now = true;
    }

    pub fn goto_today(&mut self) {
        let today = self.today();
        self.date = today;
        self.month = today;
        self.plan = None;
        self.scroll_to_now = true;
    }

    /// ‹ › 翻页：时间轴按天 / 周，月历按月。
    pub fn step(&mut self, forward: bool) {
        let sign = if forward { 1 } else { -1 };
        match self.view {
            View::Day => {
                self.date = at::add_days(self.date, if self.week { 7 * sign } else { sign });
                self.plan = None;
            }
            View::Month | View::Todos => self.month = at::add_months(self.month, sign as i32),
            _ => {}
        }
    }

    /// 时间轴展示的日期范围（含）。
    pub fn days(&self) -> Vec<NaiveDate> {
        if self.week {
            let start = at::week_start(self.date);
            (0..7).map(|i| at::add_days(start, i)).collect()
        } else {
            vec![self.date]
        }
    }

    pub fn title(&self) -> String {
        match self.view {
            View::Day if self.week => {
                let start = at::week_start(self.date);
                let end = at::add_days(start, 6);
                format!(
                    "{}月{}日 – {}月{}日",
                    start.month(),
                    start.day(),
                    end.month(),
                    end.day()
                )
            }
            View::Day => {
                let rel = at::relative_date_label(self.date, self.today());
                let label = at::date_label(self.date);
                if rel.contains('月') {
                    label
                } else {
                    format!("{rel} · {label}")
                }
            }
            View::Month => format!("{}年{}月", self.month.year(), self.month.month()),
            View::Todos if self.todo_month => {
                format!("待办 · {}年{}月", self.month.year(), self.month.month())
            }
            view => view.label().to_owned(),
        }
    }
}

/// 当前获得键盘焦点的输入框。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focused {
    Quick,
    Query,
    Field(panel::Field),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Blank,
    QuickInput,
    QuickSubmit,
    Query,
    ClearQuery,
    Prev,
    Next,
    Today,
    SetWeek(bool),
    SetTodoMonth(bool),
    ToggleShowDone,
    AiPlan,
    AutoPlace,
    PlanAccept,
    PlanDiscard,
    New(Kind),
    Undo,
    DismissToast,
    /// 时间轴空白处（按下即开始拖拽新建）。
    Grid,
    Slot(String),
    SlotResize(String),
    SlotConfirm(String, EntryStatus),
    DayHead(NaiveDate),
    Todo(String),
    Check(String),
    Day(NaiveDate),
    Open(String),
    Confirm(String, EntryStatus),
    ConfirmAndComplete(String),
    ConfirmAll,
    Restore(String),
    Purge(String),
    RoutineToggle(String),
    Panel(panel::Hit),
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    /// 主区（不含顶栏与检查器）。
    pub body: Rect,
    /// 主滚动区域及其内容高度。
    pub viewport: Rect,
    pub content_height: f32,
    pub geo: Option<day::Geo>,
    pub todo_rect: Rect,
    pub todo_height: f32,
    pub panel_rect: Rect,
    pub panel_height: f32,
    pub panel_viewport: Rect,
    pub fields: Vec<(panel::Field, Rect)>,
    pub quick: Rect,
    pub query: Rect,
}

impl Layout {
    pub fn push(&mut self, rect: Rect, hit: Hit) {
        if !rect.is_empty() {
            self.entries.push((rect, hit));
        }
    }

    /// 只登记可见部分，避免滚出裁剪区的元素仍被点中。
    pub fn push_clipped(&mut self, rect: Rect, clip: Rect, hit: Hit) {
        let visible = rect.intersect(&clip);
        self.push(visible, hit);
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h.clone())
    }

    pub fn rect_of(&self, hit: &Hit) -> Option<Rect> {
        self.entries
            .iter()
            .rev()
            .find(|(_, h)| h == hit)
            .map(|(r, _)| *r)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.viewport.height()).max(0.0)
    }

    pub fn max_todo_scroll(&self) -> f32 {
        (self.todo_height - self.todo_rect.height()).max(0.0)
    }

    pub fn max_panel_scroll(&self) -> f32 {
        (self.panel_height - self.panel_viewport.height()).max(0.0)
    }

    pub fn field(&self, field: panel::Field) -> Option<Rect> {
        self.fields
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, r)| *r)
    }
}

pub const PANEL_WIDTH: f32 = 380.0;
fn panel_width() -> f32 {
    crate::ui::settings_values::number("schedule.panelWidth", PANEL_WIDTH)
}
const ROW2: f32 = 44.0;

fn row1_height() -> f32 {
    12.0 + TextStyle::Large.line_height().max(28.0) + 12.0
}

// ---------- 通用小部件 ----------

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Ghost,
    Primary,
    Toggle(bool),
    Danger,
}

pub fn button_width(label: &str, icon: Option<Icon>) -> f32 {
    let mut w = 20.0;
    if !label.is_empty() {
        w += text::measure(label, TextStyle::Small);
    }
    if icon.is_some() {
        w += if label.is_empty() { 8.0 } else { 20.0 };
    }
    w
}

/// 画一个按钮并登记命中。
#[allow(clippy::too_many_arguments)]
pub fn button(
    list: &mut DrawList,
    lay: &mut Layout,
    r: Rect,
    label: &str,
    icon: Option<Icon>,
    style: Btn,
    hit: Hit,
    p: &Palette,
) {
    let (bg, fg) = match style {
        Btn::Primary => (Some(p.accent), p.accent_foreground),
        Btn::Toggle(true) => (Some(p.surface_elevated), p.foreground),
        Btn::Toggle(false) => (None, p.muted),
        Btn::Ghost => (None, p.foreground),
        Btn::Danger => (Some(theme::mix(p.danger, p.surface, 0.14)), p.danger),
    };
    if let Some(bg) = bg {
        list.rounded_rect(r, 6.0, bg);
    } else if matches!(style, Btn::Ghost) {
        list.rounded_border(r, 6.0, p.border);
    }
    let mut x = r.left + 10.0;
    if let Some(icon) = icon {
        let ir = if label.is_empty() {
            r
        } else {
            Rect::new(x, r.top, x + 16.0, r.bottom)
        };
        list.icon_centered(ir, icon, 14.0, fg);
        x += 20.0;
    }
    if !label.is_empty() {
        list.text_aligned(
            Rect::new(x, r.top, r.right - 8.0, r.bottom),
            label,
            TextStyle::Small,
            fg,
            Align::Leading,
        );
    }
    lay.push(r, hit);
}

/// 分段切换（日 / 周 这类）。
pub fn segmented(
    list: &mut DrawList,
    lay: &mut Layout,
    right: f32,
    y: f32,
    options: &[(&str, bool, Hit)],
    p: &Palette,
) -> f32 {
    let widths: Vec<f32> = options
        .iter()
        .map(|(l, _, _)| text::measure(l, TextStyle::Small) + 20.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let outer = Rect::new(right - total, y, right, y + 28.0);
    list.rounded_rect(outer, 7.0, p.surface_muted);
    let mut x = outer.left + 2.0;
    for ((label, active, hit), w) in options.iter().zip(widths) {
        let r = Rect::new(x, y + 2.0, x + w, y + 26.0);
        if *active {
            list.rounded_rect(r, 5.0, p.background);
        }
        list.text_aligned(
            r,
            *label,
            TextStyle::Small,
            if *active { p.foreground } else { p.muted },
            Align::Center,
        );
        lay.push(r, hit.clone());
        x += w;
    }
    outer.left
}

/// 按字符折行（中文没有空格可断）。
pub fn wrap_plain(text: &str, style: TextStyle, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for ch in paragraph.chars() {
            line.push(ch);
            if text::measure(&line, style) > width && line.chars().count() > 1 {
                line.pop();
                lines.push(std::mem::take(&mut line));
                line.push(ch);
            }
        }
        lines.push(line);
    }
    lines
}

pub fn empty_state(list: &mut DrawList, r: Rect, title: &str, hint: &str, p: &Palette) {
    let mid = r.top + (r.height() / 2.0).min(160.0);
    list.text_aligned(
        Rect::new(r.left, mid - 28.0, r.right, mid - 4.0),
        title,
        TextStyle::Label,
        p.muted,
        Align::Center,
    );
    list.text_aligned(
        Rect::new(r.left + 24.0, mid, r.right - 24.0, mid + 22.0),
        hint,
        TextStyle::Small,
        p.muted,
        Align::Center,
    );
}

pub fn section_title(
    list: &mut DrawList,
    r: Rect,
    title: &str,
    count: Option<usize>,
    color: u32,
    p: &Palette,
) {
    let label = match count {
        Some(n) => format!("{title} · {n}"),
        None => title.to_owned(),
    };
    list.text(r, label, TextStyle::Caption, color);
    let _ = p;
}

// ---------- 顶栏 ----------

pub fn header_height(st: &State) -> f32 {
    row1_height() + ROW2 + if st.quick.is_empty() { 0.0 } else { 22.0 }
}

fn paint_header(
    list: &mut DrawList,
    area: Rect,
    st: &mut State,
    lay: &mut Layout,
    focus: Option<Focused>,
    p: &Palette,
) -> f32 {
    let top = area.top;
    let pad = 20.0;
    // 第一行：动作从右往左排，标题占剩下的宽度。窄时按钮只留图标。
    let compact = area.width() < 820.0;
    let y = top + 12.0;
    let mut right = area.right - pad;
    let mut place = |label: &str, icon: Option<Icon>| {
        let w = button_width(label, icon);
        let r = Rect::new(right - w, y, right, y + 28.0);
        right -= w + 8.0;
        r
    };
    let short = |label: &'static str| if compact { "" } else { label };
    match st.view {
        View::Day => {
            let (label, icon) = (short("自动排入"), Some(Icon::ZAP));
            let r = place(label, icon);
            button(list, lay, r, label, icon, Btn::Ghost, Hit::AutoPlace, p);
            let (label, icon) = (short("AI 规划  Ctrl+J"), Some(Icon::SPARKLES));
            let r = place(label, icon);
            button(list, lay, r, label, icon, Btn::Primary, Hit::AiPlan, p);
            let options = [
                ("日", !st.week, Hit::SetWeek(false)),
                ("周", st.week, Hit::SetWeek(true)),
            ];
            right = segmented(list, lay, right, y, &options, p) - 8.0;
        }
        View::Todos => {
            let label = match (compact, st.show_done) {
                (true, _) => "已完成",
                (false, true) => "隐藏已完成",
                (false, false) => "显示已完成",
            };
            let r = place(label, None);
            button(
                list,
                lay,
                r,
                label,
                None,
                Btn::Toggle(st.show_done),
                Hit::ToggleShowDone,
                p,
            );
            let options = [
                ("时间轴", !st.todo_month, Hit::SetTodoMonth(false)),
                ("月历", st.todo_month, Hit::SetTodoMonth(true)),
            ];
            right = segmented(list, lay, right, y, &options, p) - 8.0;
        }
        View::Confirm => {
            if st
                .data
                .as_ref()
                .is_some_and(|d| !query::confirm_queue(d, st.now).is_empty())
            {
                let (label, icon) = (short("全部标为已执行"), Some(Icon::CHECK));
                let r = place(label, icon);
                button(list, lay, r, label, icon, Btn::Ghost, Hit::ConfirmAll, p);
            }
        }
        View::Goals => {
            for (label, kind) in [("新目标", Kind::Goal), ("新愿望", Kind::Wish)] {
                let r = place(label, Some(Icon::PLUS));
                button(
                    list,
                    lay,
                    r,
                    label,
                    Some(Icon::PLUS),
                    Btn::Ghost,
                    Hit::New(kind),
                    p,
                );
            }
        }
        View::Routines => {
            let label = short("新重复安排");
            let r = place(label, Some(Icon::PLUS));
            button(
                list,
                lay,
                r,
                label,
                Some(Icon::PLUS),
                Btn::Ghost,
                Hit::New(Kind::Routine),
                p,
            );
        }
        View::Projects => {
            let r = place("新项目", Some(Icon::PLUS));
            button(
                list,
                lay,
                r,
                "新项目",
                Some(Icon::PLUS),
                Btn::Ghost,
                Hit::New(Kind::Project),
                p,
            );
        }
        _ => {}
    }

    let navigable =
        matches!(st.view, View::Day | View::Month) || (st.view == View::Todos && st.todo_month);
    if navigable {
        let r = Rect::new(right - 28.0, y, right, y + 28.0);
        button(
            list,
            lay,
            r,
            "",
            Some(Icon::CHEVRON_RIGHT),
            Btn::Ghost,
            Hit::Next,
            p,
        );
        right -= 32.0;
        let w = button_width("今天", None);
        let r = Rect::new(right - w, y, right, y + 28.0);
        button(list, lay, r, "今天", None, Btn::Ghost, Hit::Today, p);
        right -= w + 4.0;
        let r = Rect::new(right - 28.0, y, right, y + 28.0);
        button(
            list,
            lay,
            r,
            "",
            Some(Icon::CHEVRON_LEFT),
            Btn::Ghost,
            Hit::Prev,
            p,
        );
        right -= 36.0;
    }
    let title_w = right - (area.left + pad);
    if title_w > 40.0 {
        let title_height = TextStyle::Large.line_height().max(28.0);
        list.text(
            Rect::new(
                area.left + pad,
                top + 12.0,
                right,
                top + 12.0 + title_height,
            ),
            text::ellipsize(&st.title(), TextStyle::Large, title_w),
            TextStyle::Large,
            p.foreground,
        );
    }
    // 第二行：一句话添加 + 搜索。
    let y = top + row1_height() + 4.0;
    let query_w = 220.0_f32.min(area.width() * 0.3);
    let query = Rect::new(area.right - pad - query_w, y, area.right - pad, y + 32.0);
    let quick = Rect::new(area.left + pad, y, query.left - 10.0, y + 32.0);
    lay.quick = quick;
    lay.query = query;
    let quick_focus = focus == Some(Focused::Quick);
    list.rounded_rect(quick, 8.0, p.surface_muted);
    if quick_focus {
        list.rounded_border(quick, 8.0, p.accent);
    }
    list.icon_centered(
        Rect::new(quick.left + 6.0, quick.top, quick.left + 30.0, quick.bottom),
        Icon::PLUS,
        14.0,
        p.muted,
    );
    let field_r = Rect::new(
        quick.left + 32.0,
        quick.top,
        quick.right - 64.0,
        quick.bottom,
    );
    if st.quick.is_empty() && !quick_focus {
        list.text_aligned(
            field_r,
            text::ellipsize(
                "一句话添加：明天 9 点 写周报 1h #工作 · 每天 7:00 跑步 · 任务：读完第三章 周五",
                TextStyle::Small,
                field_r.width().max(0.0),
            ),
            TextStyle::Small,
            p.muted,
            Align::Leading,
        );
    } else {
        st.quick
            .paint(list, field_r, quick_focus, p, FieldLook::bare());
    }
    lay.push(quick, Hit::QuickInput);
    if !st.quick.is_empty() {
        let r = Rect::new(
            quick.right - 58.0,
            quick.top + 4.0,
            quick.right - 4.0,
            quick.bottom - 4.0,
        );
        button(
            list,
            lay,
            r,
            "添加",
            None,
            Btn::Primary,
            Hit::QuickSubmit,
            p,
        );
    }
    let query_focus = focus == Some(Focused::Query);
    list.rounded_rect(query, 8.0, p.surface_muted);
    if query_focus {
        list.rounded_border(query, 8.0, p.accent);
    }
    list.icon_centered(
        Rect::new(query.left + 6.0, query.top, query.left + 28.0, query.bottom),
        Icon::SEARCH,
        13.0,
        p.muted,
    );
    let qf = Rect::new(
        query.left + 30.0,
        query.top,
        query.right - 28.0,
        query.bottom,
    );
    if st.query.is_empty() && !query_focus {
        list.text_aligned(qf, "搜索全部", TextStyle::Small, p.muted, Align::Leading);
        list.text_aligned(
            Rect::new(
                query.right - 24.0,
                query.top,
                query.right - 8.0,
                query.bottom,
            ),
            "/",
            TextStyle::Small,
            p.muted,
            Align::Trailing,
        );
    } else {
        st.query.paint(list, qf, query_focus, p, FieldLook::bare());
    }
    lay.push(query, Hit::Query);
    if !st.query.is_empty() {
        let r = Rect::new(
            query.right - 26.0,
            query.top + 4.0,
            query.right - 4.0,
            query.bottom - 4.0,
        );
        list.icon_centered(r, Icon::X, 12.0, p.muted);
        lay.push(r, Hit::ClearQuery);
    }
    let mut bottom = top + row1_height() + ROW2;
    if !st.quick.is_empty() {
        let parse = core::parse::parse_quick_add(st.quick.text(), st.today());
        let summary = core::parse::summarize(&parse);
        list.text(
            Rect::new(
                area.left + pad + 32.0,
                bottom - 6.0,
                area.right - pad,
                bottom + 16.0,
            ),
            text::ellipsize(
                &format!("将创建：{summary}"),
                TextStyle::Caption,
                area.width() - 80.0,
            ),
            TextStyle::Caption,
            p.accent,
        );
        bottom += 22.0;
    }
    list.hline(area.left, area.right, bottom - 0.5, p.border);
    bottom
}

fn paint_toast(list: &mut DrawList, area: Rect, st: &State, lay: &mut Layout, p: &Palette) {
    let (message, undo, color) = if !st.error.is_empty() {
        (st.error.clone(), false, p.danger)
    } else if let Some(toast) = &st.toast {
        (toast.message.clone(), toast.undo, p.foreground)
    } else {
        return;
    };
    let max_w = (area.width() - 80.0).max(200.0);
    let msg = text::ellipsize(&message, TextStyle::Small, max_w - 110.0);
    let w = (text::measure(&msg, TextStyle::Small) + if undo { 110.0 } else { 56.0 }).min(max_w);
    let cx = area.left + area.width() / 2.0;
    let r = Rect::new(
        cx - w / 2.0,
        area.bottom - 56.0,
        cx + w / 2.0,
        area.bottom - 20.0,
    );
    list.rounded_rect(r, 10.0, p.surface_elevated);
    list.rounded_border(r, 10.0, p.border);
    list.text_aligned(
        Rect::new(r.left + 14.0, r.top, r.right - 40.0, r.bottom),
        msg,
        TextStyle::Small,
        color,
        Align::Leading,
    );
    lay.push(r, Hit::DismissToast);
    if undo {
        let ur = Rect::new(r.right - 92.0, r.top + 5.0, r.right - 34.0, r.bottom - 5.0);
        list.text_aligned(ur, "撤销", TextStyle::Small, p.accent, Align::Center);
        lay.push(ur, Hit::Undo);
    }
    let xr = Rect::new(r.right - 30.0, r.top, r.right - 6.0, r.bottom);
    list.icon_centered(xr, Icon::X, 12.0, p.muted);
    lay.push(xr, Hit::DismissToast);
}

/// 画整个主区，返回这一帧的布局。滚动越界由控制器在下一帧前夹紧。
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    st: &mut State,
    focus: Option<Focused>,
    p: &Palette,
) -> Layout {
    let mut lay = Layout::default();
    list.rect(area, p.background);
    lay.push(area, Hit::Blank);
    let header_bottom = paint_header(list, area, st, &mut lay, focus, p);
    let body = Rect::new(area.left, header_bottom, area.right, area.bottom);
    let docked = st.panel.is_some() && area.width() >= (panel_width() + 600.0).max(980.0);
    let main = if docked {
        Rect::new(body.left, body.top, body.right - panel_width(), body.bottom)
    } else {
        body
    };
    lay.body = main;
    lay.viewport = main;
    // 面板里的输入框要 `&mut` 画，先把它和数据拿出来，画完放回。
    let mut pn = st.panel.take();
    let data = st.data.take();
    let view: &State = st;
    match &data {
        None => empty_state(
            list,
            main,
            if view.error.is_empty() {
                "正在读取日程待办…"
            } else {
                "读取失败"
            },
            &view.error,
            p,
        ),
        Some(data) => {
            if !view.query.is_empty() {
                lists::paint_search(list, main, view, data, &mut lay, p);
            } else {
                match view.view {
                    View::Day => day::paint(list, main, view, data, &mut lay, pn.is_some(), p),
                    View::Month => month::paint(list, main, view, data, &mut lay, false, p),
                    View::Todos if view.todo_month => {
                        month::paint(list, main, view, data, &mut lay, true, p)
                    }
                    View::Todos => lists::paint_river(list, main, view, data, &mut lay, p),
                    View::Confirm => lists::paint_confirm(list, main, view, data, &mut lay, p),
                    View::Goals => lists::paint_goals(list, main, view, data, &mut lay, p),
                    View::Routines => lists::paint_routines(list, main, view, data, &mut lay, p),
                    View::Projects => lists::paint_projects(list, main, view, data, &mut lay, p),
                    View::Trash => lists::paint_trash(list, main, view, data, &mut lay, p),
                    View::History => lists::paint_history(list, main, view, &mut lay, p),
                }
            }
            if let Some(pn) = pn.as_mut() {
                let r = Rect::new(
                    (body.right - panel_width()).max(body.left),
                    body.top,
                    body.right,
                    body.bottom,
                );
                panel::paint(list, r, view, pn, data, &mut lay, focus, p);
            }
        }
    }
    st.data = data;
    st.panel = pn;
    paint_toast(list, main, st, &mut lay, p);
    lay
}

/// 工作日时段外的遮罩用到的 `(开始分钟, 结束分钟)`。
pub fn work_minutes(data: &AgendaData) -> (i64, i64) {
    let parse = |s: &str, d: i64| {
        at::parse_hm(s)
            .map(|t| at::minute_of_day(NaiveDate::MIN.and_time(t)) as i64)
            .unwrap_or(d)
    };
    (
        parse(&data.settings.work_start, 9 * 60),
        parse(&data.settings.work_end, 22 * 60),
    )
}

/// 任务一行的元信息：截止、优先级、估时、已排情况、步骤。
pub fn task_meta(
    data: &AgendaData,
    task: &core::Task,
    now: NaiveDateTime,
) -> Vec<(String, Option<u32>)> {
    let c_now = now;
    let facts = query::task_facts(data, task, c_now);
    let mut out = Vec::new();
    if let Some(due) = &task.due {
        out.push((format!("截止 {}", at::due_label(due, now.date())), None));
    }
    if matches!(task.priority, Priority::High | Priority::Urgent) {
        out.push((task.priority.label().to_owned(), None));
    }
    if let Some(m) = task.estimate_minutes {
        out.push((format!("预计 {}", at::duration_short(m as i64)), None));
    }
    if facts.scheduled {
        out.push((
            format!(
                "已排 {} · {}",
                facts.block_count,
                at::duration_short(facts.planned_minutes)
            ),
            None,
        ));
    }
    if facts.done_minutes > 0 {
        out.push((
            format!("已投入 {}", at::duration_short(facts.done_minutes)),
            None,
        ));
    }
    if facts.steps_total > 0 {
        out.push((
            format!("{}/{} 步", facts.steps_done, facts.steps_total),
            None,
        ));
    }
    if task.status == TaskStatus::Doing {
        out.push(("进行中".into(), None));
    }
    out
}

/// 给面板和清单用的空闲时段：下一段可放下 `minutes` 的空闲。
pub fn next_free(
    data: &AgendaData,
    date: NaiveDate,
    now: NaiveDateTime,
    minutes: i64,
) -> Option<(NaiveDateTime, NaiveDateTime)> {
    for offset in 0..14 {
        let day = at::add_days(date, offset);
        if let Some(slot) = planner::free_slots(data, day, now, None, minutes)
            .into_iter()
            .next()
        {
            return Some((slot.start, slot.start + chrono::Duration::minutes(minutes)));
        }
    }
    None
}
