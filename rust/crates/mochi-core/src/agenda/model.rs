//! 日程待办的数据模型。
//!
//! 两个并列模块共用一份数据：
//! - **事项**：愿望 [`Wish`]、目标 [`Goal`]、任务 [`Task`]，回答「想实现什么 / 要做什么 / 进展如何」；
//! - **日程**：[`Entry`]（独立日程或任务的时间块）与 [`Routine`]（重复安排），回答「什么时候做」。
//!
//! 父级关系（`wish_id` / `goal_id`）表达「为了什么」，`Entry::task_id` 表达「安排到什么时候」。
//! 日程从不复制任务正文：关联任务的时间块标题永远取任务标题。
//!
//! 时间约定：安排类时间（`Entry::start` / `Task::due` 等）是**本地墙钟**字符串
//! `YYYY-MM-DD` 或 `YYYY-MM-DDTHH:MM`；元数据时间戳（`created_at` 等）是 UTC ISO。

use serde::{Deserialize, Serialize};

fn is_false(v: &bool) -> bool {
    !*v
}

macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $wire:literal : $label:literal),+ $(,)? } default $default:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $wire)] $variant),+
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$default
            }
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$(Self::$variant),+];

            pub fn wire(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }

            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$variant => $label),+
                }
            }

            /// 接受线上值或中文标签。
            pub fn from_wire(value: &str) -> Option<Self> {
                match value.trim() {
                    $($wire | $label => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

wire_enum!(
    /// 愿望的生命周期。愿望允许模糊，不强制期限或标准。
    WishStatus { Open = "open": "心愿中", Realized = "realized": "已实现", Dropped = "dropped": "已放下" } default Open
);

wire_enum!(
    /// 目标的生命周期。「达成」只能由用户显式判定，任务全部完成不会自动达成。
    GoalStatus {
        Active = "active": "进行中",
        Paused = "paused": "暂停",
        Achieved = "achieved": "已达成",
        Abandoned = "abandoned": "已放弃",
    } default Active
);

wire_enum!(
    /// 任务的生命周期。
    TaskStatus { Todo = "todo": "待做", Doing = "doing": "进行中", Done = "done": "已完成", Cancelled = "cancelled": "已取消" } default Todo
);

wire_enum!(
    /// 时间块 / 日程自身的执行状态，与关联任务的完成状态互相独立。
    EntryStatus {
        Planned = "planned": "计划中",
        Done = "done": "已执行",
        Skipped = "skipped": "未执行",
        Cancelled = "cancelled": "已取消",
    } default Planned
);

wire_enum!(
    Priority { Low = "low": "低", Normal = "normal": "普通", High = "high": "高", Urgent = "urgent": "紧急" } default Normal
);

wire_enum!(
    RoutineStatus { Active = "active": "进行中", Paused = "paused": "已暂停" } default Active
);

wire_enum!(
    ProjectStatus { Active = "active": "进行中", Done = "done": "已完成" } default Active
);

impl TaskStatus {
    /// 还需要执行（参与逾期判断、出现在待办列表）。
    pub fn is_open(self) -> bool {
        matches!(self, Self::Todo | Self::Doing)
    }
}

impl GoalStatus {
    pub fn is_open(self) -> bool {
        matches!(self, Self::Active | Self::Paused)
    }
}

impl Priority {
    pub fn rank(self) -> u8 {
        match self {
            Self::Urgent => 3,
            Self::High => 2,
            Self::Normal => 1,
            Self::Low => 0,
        }
    }
}

/// 记录种类。ID 前缀与种类一一对应，便于从引用里反推种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Wish,
    Goal,
    Task,
    Entry,
    Routine,
    Project,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Wish,
        Kind::Goal,
        Kind::Task,
        Kind::Entry,
        Kind::Routine,
        Kind::Project,
    ];

    pub fn wire(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Goal => "goal",
            Self::Task => "task",
            Self::Entry => "entry",
            Self::Routine => "routine",
            Self::Project => "project",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Wish => "愿望",
            Self::Goal => "目标",
            Self::Task => "任务",
            Self::Entry => "日程",
            Self::Routine => "重复",
            Self::Project => "项目",
        }
    }

    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Wish => "wish",
            Self::Goal => "goal",
            Self::Task => "task",
            Self::Entry => "ent",
            Self::Routine => "rtn",
            Self::Project => "proj",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "wish" | "愿望" => Self::Wish,
            "goal" | "目标" => Self::Goal,
            "task" | "任务" | "todo" => Self::Task,
            "entry" | "event" | "block" | "日程" => Self::Entry,
            "routine" | "habit" | "重复" => Self::Routine,
            "project" | "项目" => Self::Project,
            _ => return None,
        })
    }

    pub fn of_id(id: &str) -> Option<Self> {
        let prefix = id.split('-').next()?;
        Self::ALL
            .into_iter()
            .find(|kind| kind.id_prefix() == prefix)
    }
}

/// 任务清单里的一步。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "is_false")]
    pub done: bool,
}

/// 愿望：可以很模糊，不强制期限、标准或子项。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wish {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub status: WishStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    /// 删除时被解除关联的目标，恢复时据此重新挂回（仅当它们仍未挂到别处）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub detached: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 目标：决定追求的结果，有自己的达成标准、状态和结果判定。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Goal {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// 达成标准（可多条，逐条判断）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<Step>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wish_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub status: GoalStatus,
    /// 期望达成日期 `YYYY-MM-DD`，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<String>,
    /// 结果复盘：达成 / 放弃时写下的判断。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub result_note: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub detached: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 任务：具体行动。可以来自目标也可以独立存在；不强制截止，不强制一天完成。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub status: TaskStatus,
    pub priority: Priority,
    /// 截止：`YYYY-MM-DD` 或 `YYYY-MM-DDTHH:MM`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// 打算在哪天做（`YYYY-MM-DD`），不是时间块，只是意向。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate_minutes: Option<u32>,
    /// 来源说明（例如「老师布置」），只是附加信息。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    /// 删除是墓碑：历史时间块仍能追溯到它，回收站可恢复。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 日程：某个日期或时间段的事件 / 执行安排。
///
/// 关联任务时是该任务的一个时间块；否则是独立日程。
/// 由重复安排生成时带 `routine_id` + `occurrence`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    /// 独立日程的标题；关联任务时忽略（标题跟随任务）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// 时间块自己的备注（不是任务正文）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// 任务被彻底清除后留下的标题快照，保证历史日程不悬空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached_task_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    /// 重复安排的第几天（`YYYY-MM-DD`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<String>,
    /// `YYYY-MM-DDTHH:MM`
    pub start: String,
    /// `YYYY-MM-DDTHH:MM`，严格晚于 `start`。
    pub end: String,
    #[serde(skip_serializing_if = "is_false")]
    pub all_day: bool,
    pub status: EntryStatus,
    /// 实际投入分钟（确认执行时可填，不填按计划时长计）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_minutes: Option<u32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub location: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// 提前多少分钟提醒。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 重复安排：每天 / 每周 / 每月发生的事。未被处理的发生是虚拟的，
/// 被确认、移动或编辑时才落成一条 [`Entry`]。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routine {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// RRULE 子集：`FREQ=DAILY|WEEKLY|MONTHLY;INTERVAL;BYDAY;BYMONTHDAY`。
    pub rule: String,
    /// 从哪天开始（`YYYY-MM-DD`）。
    pub start_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// 固定时刻 `HH:MM`；为空表示当天任意时间（全天打卡）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    pub duration_minutes: u32,
    /// 为哪个目标服务（「为了什么」）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub status: RoutineStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub status: ProjectStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgendaSettings {
    /// 时间轴显示范围（小时）。
    pub day_start_hour: u32,
    pub day_end_hour: u32,
    /// 自动排入使用的可工作时段 `HH:MM`。
    pub work_start: String,
    pub work_end: String,
    pub default_block_minutes: u32,
    /// 待确认队列回看多少天。
    pub confirm_lookback_days: u32,
    /// 结束后多少分钟发出「确认是否执行」提醒。
    pub confirm_after_minutes: u32,
}

impl Default for AgendaSettings {
    fn default() -> Self {
        Self {
            day_start_hour: 6,
            day_end_hour: 24,
            work_start: "09:00".into(),
            work_end: "22:00".into(),
            default_block_minutes: 60,
            confirm_lookback_days: 14,
            confirm_after_minutes: 5,
        }
    }
}

pub const DATA_VERSION: u32 = 1;

/// 整个日程待办的数据快照，落盘为一个原子写入的 JSON 文件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgendaData {
    pub version: u32,
    pub wishes: Vec<Wish>,
    pub goals: Vec<Goal>,
    pub tasks: Vec<Task>,
    pub entries: Vec<Entry>,
    pub routines: Vec<Routine>,
    pub projects: Vec<Project>,
    pub settings: AgendaSettings,
}

impl Default for AgendaData {
    fn default() -> Self {
        Self {
            version: DATA_VERSION,
            wishes: Vec::new(),
            goals: Vec::new(),
            tasks: Vec::new(),
            entries: Vec::new(),
            routines: Vec::new(),
            projects: Vec::new(),
            settings: AgendaSettings::default(),
        }
    }
}

impl AgendaData {
    pub fn wish(&self, id: &str) -> Option<&Wish> {
        self.wishes.iter().find(|w| w.id == id)
    }
    pub fn goal(&self, id: &str) -> Option<&Goal> {
        self.goals.iter().find(|g| g.id == id)
    }
    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn routine(&self, id: &str) -> Option<&Routine> {
        self.routines.iter().find(|r| r.id == id)
    }
    pub fn project(&self, id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn wish_mut(&mut self, id: &str) -> Option<&mut Wish> {
        self.wishes.iter_mut().find(|w| w.id == id)
    }
    pub fn goal_mut(&mut self, id: &str) -> Option<&mut Goal> {
        self.goals.iter_mut().find(|g| g.id == id)
    }
    pub fn task_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }
    pub fn entry_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }
    pub fn routine_mut(&mut self, id: &str) -> Option<&mut Routine> {
        self.routines.iter_mut().find(|r| r.id == id)
    }
    pub fn project_mut(&mut self, id: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == id)
    }

    /// 活着的（未删除）任务。
    pub fn live_task(&self, id: &str) -> Option<&Task> {
        self.task(id).filter(|t| t.deleted_at.is_none())
    }
    pub fn live_goal(&self, id: &str) -> Option<&Goal> {
        self.goal(id).filter(|g| g.deleted_at.is_none())
    }
    pub fn live_wish(&self, id: &str) -> Option<&Wish> {
        self.wish(id).filter(|w| w.deleted_at.is_none())
    }
    pub fn live_project(&self, id: &str) -> Option<&Project> {
        self.project(id).filter(|p| p.deleted_at.is_none())
    }
    pub fn live_routine(&self, id: &str) -> Option<&Routine> {
        self.routine(id).filter(|r| r.deleted_at.is_none())
    }

    /// 任意记录的显示标题。
    pub fn title_of(&self, id: &str) -> Option<String> {
        match Kind::of_id(id)? {
            Kind::Wish => self.wish(id).map(|w| w.title.clone()),
            Kind::Goal => self.goal(id).map(|g| g.title.clone()),
            Kind::Task => self.task(id).map(|t| t.title.clone()),
            Kind::Entry => self.entry(id).map(|e| self.entry_title(e)),
            Kind::Routine => self.routine(id).map(|r| r.title.clone()),
            Kind::Project => self.project(id).map(|p| p.name.clone()),
        }
    }

    /// 时间块的显示标题：关联任务时跟随任务，任务已删除时明确标出。
    pub fn entry_title(&self, entry: &Entry) -> String {
        if let Some(task) = entry.task_id.as_deref().and_then(|id| self.task(id)) {
            return task.title.clone();
        }
        if let Some(title) = &entry.detached_task_title {
            return title.clone();
        }
        if entry.title.is_empty() {
            if let Some(routine) = entry.routine_id.as_deref().and_then(|id| self.routine(id)) {
                return routine.title.clone();
            }
        }
        entry.title.clone()
    }

    /// 时间块关联的原任务是否已被删除（墓碑或已清除）。
    pub fn entry_task_deleted(&self, entry: &Entry) -> bool {
        entry.detached_task_title.is_some()
            || entry
                .task_id
                .as_deref()
                .and_then(|id| self.task(id))
                .is_some_and(|t| t.deleted_at.is_some())
    }

    /// 记录是否存在（包括回收站里的）。
    pub fn exists(&self, id: &str) -> bool {
        match Kind::of_id(id) {
            Some(Kind::Wish) => self.wish(id).is_some(),
            Some(Kind::Goal) => self.goal(id).is_some(),
            Some(Kind::Task) => self.task(id).is_some(),
            Some(Kind::Entry) => self.entry(id).is_some(),
            Some(Kind::Routine) => self.routine(id).is_some(),
            Some(Kind::Project) => self.project(id).is_some(),
            None => false,
        }
    }
}
