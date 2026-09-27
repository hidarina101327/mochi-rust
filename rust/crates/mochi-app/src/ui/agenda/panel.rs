//! 检查器：查看、编辑、新建都在主区右侧这一栏里完成。
//!
//! 编辑是就地的：字段失焦、回车或关闭面板时只提交改过的字段；状态、优先级、
//! 上级这些选择项点一下就生效。新建时先在这里填好，再点「创建」。
//!
//! 面板只描述「改了什么」（[`Panel::changes`] / [`Panel::create_op`]），真正写盘由控制器
//! 通过同一套批量操作语言完成。

use chrono::NaiveTime;
use serde_json::{json, Map, Value};

use super::visual::{self, Colors};
use super::*;
use mochi_core::agenda::{GoalStatus, ProjectStatus, RoutineStatus, WishStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Title,
    Note,
    Due,
    Planned,
    Estimate,
    Source,
    Tags,
    Start,
    End,
    Location,
    Actual,
    TargetDate,
    ResultNote,
    Rule,
    Time,
    Duration,
    StartDate,
    Until,
    NewStep,
}

impl Field {
    pub fn multiline(self) -> bool {
        matches!(self, Field::Note | Field::ResultNote)
    }

    fn label(self) -> &'static str {
        match self {
            Field::Title => "标题",
            Field::Note => "备注",
            Field::Due => "截止",
            Field::Planned => "计划日",
            Field::Estimate => "预计",
            Field::Source => "来源",
            Field::Tags => "标签",
            Field::Start => "开始",
            Field::End => "结束",
            Field::Location => "地点",
            Field::Actual => "实际投入",
            Field::TargetDate => "期望日期",
            Field::ResultNote => "结果复盘",
            Field::Rule => "重复",
            Field::Time => "时刻",
            Field::Duration => "时长",
            Field::StartDate => "起始日",
            Field::Until => "截止日",
            Field::NewStep => "",
        }
    }

    fn placeholder(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Field::Title, Kind::Wish) => "想实现什么？可以很模糊",
            (Field::Title, Kind::Goal) => "决定追求的结果，例如：六级 600 分",
            (Field::Title, Kind::Task) => "具体要做的事",
            (Field::Title, Kind::Entry) => "这段时间做什么",
            (Field::Title, Kind::Routine) => "每天 / 每周要做的事",
            (Field::Title, Kind::Project) => "项目名称",
            (Field::Note, _) => "补充说明…",
            (Field::Due, _) => "不设截止也可以 · 例：周五 18:00",
            (Field::Planned, _) => "打算哪天做 · 例：明天",
            (Field::Estimate, _) | (Field::Duration, _) | (Field::Actual, _) => "分钟",
            (Field::Source, _) => "例：老师布置 · 只是附加信息",
            (Field::Tags, _) => "空格分隔",
            (Field::Start, _) => "例：明天 14:00",
            (Field::End, _) => "例：15:30",
            (Field::Location, _) => "可选",
            (Field::TargetDate, _) => "可选 · 例：12月20日",
            (Field::ResultNote, _) => "达成或放弃时，写下你的判断",
            (Field::Rule, _) => "每天 / 工作日 / 每周一三 / 每月15号",
            (Field::Time, _) => "留空 = 全天打卡",
            (Field::StartDate, _) => "默认今天",
            (Field::Until, _) => "可选",
            (Field::NewStep, Kind::Goal) => "添加一条达成标准，回车确认",
            (Field::NewStep, _) => "添加一个步骤，回车确认",
        }
    }
}

/// 面板正在展示什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 已有记录（或重复安排的虚拟发生 `rtn-…@日期`）。
    Record(String),
    New(Kind),
}

/// 菜单选择器改的是哪一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickSlot {
    Parent,
    Project,
}

/// 新建时还没落盘的选择项。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picks {
    /// 任务 / 重复安排 → 目标；目标 → 愿望；日程 → 任务。
    pub parent: Option<String>,
    pub project: Option<String>,
    pub priority: Priority,
    pub all_day: bool,
    pub color: Option<String>,
}

/// 需要用户明确选择的后续问题。默认选项永远是「什么都不改」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturePrompt {
    pub task_id: String,
    pub entry_ids: Vec<String>,
    pub reason: String,
}

pub struct Panel {
    pub target: Target,
    pub fields: Vec<(Field, TextField)>,
    /// 字段载入时的文本；提交时只写与它不同的字段。
    pub loaded: Vec<(Field, String)>,
    pub focus: Option<Field>,
    pub scroll: f32,
    pub error: String,
    pub picks: Picks,
    pub prompt: Option<FuturePrompt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    Close,
    Create,
    Field(Field),
    Open(String),
    NewKind(Kind),
    TaskStatus(TaskStatus),
    GoalStatus(GoalStatus),
    WishStatus(WishStatus),
    EntryStatus(EntryStatus),
    ProjectStatus(ProjectStatus),
    RoutineActive(bool),
    Priority(Priority),
    AllDay(bool),
    Color(Option<String>),
    Pick(PickSlot),
    Unlink(PickSlot),
    Step(String),
    StepRemove(String),
    /// 给任务再安排一个时间块（每次都是新增）。
    ScheduleTask,
    /// 时间块：已执行并完成任务（显式组合操作）。
    CompleteBlockAndTask,
    /// 以当前记录为上级新建子项。
    NewChild(Kind),
    RulePreset(&'static str),
    Archive(bool),
    Delete,
    Restore,
    KeepFuture,
    CancelFuture,
}

// ---------- 字段文本的读写 ----------

fn show_stamp(value: &str) -> String {
    value.replace('T', " ")
}

fn show_minutes(value: Option<u32>) -> String {
    value.map(|m| m.to_string()).unwrap_or_default()
}

/// 全天日程的结束在数据里是次日零点，展示成包含式的最后一天。
fn show_end(entry: &core::Entry) -> String {
    if entry.all_day {
        at::parse_stamp(&entry.end)
            .map(|e| at::date_key(at::add_days(e.date(), -1)))
            .unwrap_or_default()
    } else {
        show_stamp(&entry.end)
    }
}

/// 宽松的时间输入：`2026-09-28 14:00`、`9/28`、`明天 下午3点`、`14:30`（沿用 `base` 的日期）。
pub fn parse_when(
    text: &str,
    base: NaiveDate,
    today: NaiveDate,
) -> Option<(NaiveDate, Option<NaiveTime>)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stamp) = at::parse_stamp(text) {
        return Some((stamp.date(), Some(stamp.time())));
    }
    if text.len() <= 10 {
        if let Some(date) = at::parse_date(text) {
            return Some((date, None));
        }
    }
    if let Some(t) = at::parse_hm(text) {
        return Some((base, Some(t)));
    }
    let parse = core::parse::parse_quick_add(text, today);
    let time = parse
        .time
        .and_then(|(h, m)| NaiveTime::from_hms_opt(h, m, 0));
    match (parse.date, time) {
        (None, None) => None,
        (date, time) => Some((date.unwrap_or(base), time)),
    }
}

fn when_stamp(
    text: &str,
    base: NaiveDate,
    today: NaiveDate,
    default_time: NaiveTime,
) -> Option<String> {
    parse_when(text, base, today).map(|(d, t)| at::stamp(d.and_time(t.unwrap_or(default_time))))
}

fn when_due(text: &str, today: NaiveDate) -> Option<String> {
    parse_when(text, today, today).map(|(d, t)| match t {
        Some(t) => at::stamp(d.and_time(t)),
        None => at::date_key(d),
    })
}

fn when_date(text: &str, today: NaiveDate) -> Option<String> {
    parse_when(text, today, today).map(|(d, _)| at::date_key(d))
}

fn parse_rule(text: &str) -> Option<String> {
    let text = text.trim();
    core::recur::normalize(text).or_else(|| {
        core::parse::parse_quick_add(text, at::today())
            .recurrence_rule
            .and_then(|r| core::recur::normalize(&r))
    })
}

fn tags_of(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == '，')
        .map(|t| t.trim().trim_start_matches('#').to_owned())
        .filter(|t| !t.is_empty())
        .collect()
}

fn minutes_of(text: &str) -> Result<Option<u32>, String> {
    let text = text
        .trim()
        .trim_end_matches("分钟")
        .trim_end_matches('m')
        .trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Some(hours) = text.strip_suffix('h').or_else(|| text.strip_suffix("小时")) {
        return hours
            .trim()
            .parse::<f32>()
            .map(|h| Some((h * 60.0).round() as u32))
            .map_err(|_| format!("「{text}」不是时长"));
    }
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| format!("「{text}」不是分钟数"))
}

impl Panel {
    fn with(target: Target, fields: Vec<(Field, String)>, picks: Picks) -> Self {
        let kind = match &target {
            Target::New(k) => *k,
            Target::Record(id) => {
                Kind::of_id(id.split('@').next().unwrap_or(id)).unwrap_or(Kind::Entry)
            }
        };
        Self {
            target,
            loaded: fields.clone(),
            fields: fields
                .into_iter()
                .map(|(f, text)| {
                    let mut field = TextField::new(f.placeholder(kind)).with_text(&text);
                    field.style = if f == Field::Title {
                        TextStyle::Body16
                    } else {
                        TextStyle::Small
                    };
                    (f, field)
                })
                .collect(),
            focus: None,
            scroll: 0.0,
            error: String::new(),
            picks,
            prompt: None,
        }
    }

    /// 新建面板。`date` / `span` 用来预填时间，`parent` 预填上级。
    pub fn new_record(
        kind: Kind,
        date: NaiveDate,
        span: Option<(NaiveDateTime, NaiveDateTime)>,
        parent: Option<String>,
    ) -> Self {
        let mut fields = vec![(Field::Title, String::new())];
        match kind {
            Kind::Task => fields.extend([
                (Field::Due, String::new()),
                (Field::Planned, String::new()),
                (Field::Estimate, String::new()),
                (Field::Source, String::new()),
                (Field::Tags, String::new()),
                (Field::Note, String::new()),
            ]),
            Kind::Entry => {
                let (start, end) = span.unwrap_or_else(|| {
                    let start = date.and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
                    (start, start + chrono::Duration::minutes(60))
                });
                fields.extend([
                    (Field::Start, show_stamp(&at::stamp(start))),
                    (Field::End, show_stamp(&at::stamp(end))),
                    (Field::Location, String::new()),
                    (Field::Note, String::new()),
                ]);
            }
            Kind::Goal => fields.extend([
                (Field::TargetDate, String::new()),
                (Field::Note, String::new()),
            ]),
            Kind::Wish => fields.push((Field::Note, String::new())),
            Kind::Routine => fields.extend([
                (Field::Rule, "每天".into()),
                (Field::Time, String::new()),
                (Field::Duration, "30".into()),
                (Field::StartDate, at::date_key(date)),
                (Field::Until, String::new()),
                (Field::Note, String::new()),
            ]),
            Kind::Project => fields.push((Field::Note, String::new())),
        }
        let mut panel = Self::with(
            Target::New(kind),
            fields,
            Picks {
                parent,
                ..Default::default()
            },
        );
        panel.focus = Some(Field::Title);
        panel
    }

    /// 打开一条已有记录。虚拟发生没有可编辑字段。
    pub fn open(data: &AgendaData, key: &str) -> Option<Self> {
        let target = Target::Record(key.to_owned());
        if key.contains('@') {
            return Some(Self::with(target, Vec::new(), Picks::default()));
        }
        let fields = match Kind::of_id(key)? {
            Kind::Task => {
                let t = data.task(key)?;
                vec![
                    (Field::Title, t.title.clone()),
                    (
                        Field::Due,
                        t.due.as_deref().map(show_stamp).unwrap_or_default(),
                    ),
                    (Field::Planned, t.planned_for.clone().unwrap_or_default()),
                    (Field::Estimate, show_minutes(t.estimate_minutes)),
                    (Field::Source, t.source.clone()),
                    (Field::Tags, t.tags.join(" ")),
                    (Field::NewStep, String::new()),
                    (Field::Note, t.note.clone()),
                ]
            }
            Kind::Entry => {
                let e = data.entry(key)?;
                let mut fields = Vec::new();
                if e.task_id.is_none() {
                    fields.push((Field::Title, e.title.clone()));
                }
                let start = if e.all_day {
                    e.start.get(..10).unwrap_or(&e.start).to_owned()
                } else {
                    show_stamp(&e.start)
                };
                fields.extend([
                    (Field::Start, start),
                    (Field::End, show_end(e)),
                    (Field::Location, e.location.clone()),
                ]);
                if e.status == EntryStatus::Done {
                    fields.push((Field::Actual, show_minutes(e.actual_minutes)));
                }
                fields.push((Field::Note, e.note.clone()));
                fields
            }
            Kind::Goal => {
                let g = data.goal(key)?;
                vec![
                    (Field::Title, g.title.clone()),
                    (Field::TargetDate, g.target_date.clone().unwrap_or_default()),
                    (Field::Tags, g.tags.join(" ")),
                    (Field::NewStep, String::new()),
                    (Field::Note, g.note.clone()),
                    (Field::ResultNote, g.result_note.clone()),
                ]
            }
            Kind::Wish => {
                let w = data.wish(key)?;
                vec![
                    (Field::Title, w.title.clone()),
                    (Field::Tags, w.tags.join(" ")),
                    (Field::Note, w.note.clone()),
                ]
            }
            Kind::Routine => {
                let r = data.routine(key)?;
                vec![
                    (Field::Title, r.title.clone()),
                    (Field::Rule, core::recur::describe(&r.rule)),
                    (Field::Time, r.time.clone().unwrap_or_default()),
                    (Field::Duration, r.duration_minutes.to_string()),
                    (Field::StartDate, r.start_date.clone()),
                    (Field::Until, r.until.clone().unwrap_or_default()),
                    (Field::Note, r.note.clone()),
                ]
            }
            Kind::Project => {
                let p = data.project(key)?;
                vec![
                    (Field::Title, p.name.clone()),
                    (Field::Note, p.note.clone()),
                ]
            }
        };
        Some(Self::with(target, fields, Picks::default()))
    }

    pub fn kind(&self) -> Kind {
        match &self.target {
            Target::New(kind) => *kind,
            Target::Record(id) => {
                Kind::of_id(id.split('@').next().unwrap_or(id)).unwrap_or(Kind::Entry)
            }
        }
    }

    pub fn record_id(&self) -> Option<&str> {
        match &self.target {
            Target::Record(id) => Some(id),
            Target::New(_) => None,
        }
    }

    pub fn is_new(&self) -> bool {
        matches!(self.target, Target::New(_))
    }

    pub fn field(&self, field: Field) -> Option<&TextField> {
        self.fields
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, t)| t)
    }

    pub fn field_mut(&mut self, field: Field) -> Option<&mut TextField> {
        self.fields
            .iter_mut()
            .find(|(f, _)| *f == field)
            .map(|(_, t)| t)
    }

    pub fn has(&self, field: Field) -> bool {
        self.field(field).is_some()
    }

    fn text(&self, field: Field) -> &str {
        self.field(field).map(TextField::text).unwrap_or("")
    }

    fn changed(&self, field: Field) -> bool {
        let loaded = self
            .loaded
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, t)| t.as_str());
        self.field(field).is_some_and(|t| Some(t.text()) != loaded)
    }

    pub fn dirty(&self) -> bool {
        !self.is_new()
            && self
                .fields
                .iter()
                .any(|(f, _)| *f != Field::NewStep && self.changed(*f))
    }

    /// 提交后以当前文本为新的基准。
    pub fn mark_saved(&mut self) {
        self.loaded = self
            .fields
            .iter()
            .filter(|(f, _)| *f != Field::NewStep)
            .map(|(f, t)| (*f, t.text().to_owned()))
            .collect();
    }

    /// 已有记录改过的字段 → `update` 的 patch。
    pub fn changes(
        &self,
        data: &AgendaData,
        today: NaiveDate,
    ) -> Result<Map<String, Value>, String> {
        let mut patch = Map::new();
        let Some(id) = self.record_id() else {
            return Ok(patch);
        };
        let kind = self.kind();
        let base = data
            .entry(id)
            .and_then(|e| at::parse_stamp(&e.start))
            .map(|s| s.date())
            .unwrap_or(today);
        for (field, _) in &self.fields {
            if *field == Field::NewStep || !self.changed(*field) {
                continue;
            }
            let text = self.text(*field).trim();
            let key = match (*field, kind) {
                (Field::Title, Kind::Project) => "name",
                (Field::Title, _) => "title",
                (Field::Note, _) => "note",
                (Field::Due, _) => "due",
                (Field::Planned, _) => "planned_for",
                (Field::Estimate, _) => "estimate_minutes",
                (Field::Source, _) => "source",
                (Field::Tags, _) => "tags",
                (Field::Start, _) => "start",
                (Field::End, _) => "end",
                (Field::Location, _) => "location",
                (Field::Actual, _) => "actual_minutes",
                (Field::TargetDate, _) => "target_date",
                (Field::ResultNote, _) => "result_note",
                (Field::Rule, _) => "rule",
                (Field::Time, _) => "time",
                (Field::Duration, _) => "duration_minutes",
                (Field::StartDate, _) => "start_date",
                (Field::Until, _) => "until",
                (Field::NewStep, _) => continue,
            };
            let value = match *field {
                Field::Title => {
                    if text.is_empty() {
                        return Err("标题不能为空".into());
                    }
                    json!(text)
                }
                Field::Note | Field::Source | Field::Location | Field::ResultNote => json!(text),
                Field::Tags => json!(tags_of(text)),
                Field::Estimate | Field::Duration | Field::Actual => match minutes_of(text)? {
                    Some(m) => json!(m),
                    None if *field == Field::Duration => return Err("时长不能为空".into()),
                    None => json!(0),
                },
                Field::Due => match text {
                    "" => Value::Null,
                    t => json!(when_due(t, today).ok_or_else(|| format!("看不懂截止时间「{t}」"))?),
                },
                Field::Planned | Field::TargetDate | Field::Until => match text {
                    "" => Value::Null,
                    t => json!(when_date(t, today).ok_or_else(|| format!("看不懂日期「{t}」"))?),
                },
                Field::StartDate => {
                    json!(when_date(text, today).ok_or_else(|| format!("看不懂日期「{text}」"))?)
                }
                Field::Start | Field::End => {
                    let entry = data.entry(id).ok_or("日程已被删除")?;
                    if entry.all_day {
                        let date = when_date(text, today)
                            .ok_or_else(|| format!("看不懂日期「{text}」"))?;
                        if *field == Field::End {
                            let d = at::parse_date(&date)
                                .map(|d| at::add_days(d, 1))
                                .ok_or("日期无效")?;
                            json!(format!("{}T00:00", at::date_key(d)))
                        } else {
                            json!(format!("{date}T00:00"))
                        }
                    } else {
                        let fallback = at::parse_stamp(if *field == Field::Start {
                            &entry.start
                        } else {
                            &entry.end
                        })
                        .map(|s| s.time())
                        .unwrap_or(NaiveTime::MIN);
                        json!(when_stamp(text, base, today, fallback)
                            .ok_or_else(|| format!("看不懂时间「{text}」"))?)
                    }
                }
                Field::Time => match text {
                    "" => Value::Null,
                    t => json!(at::parse_hm(t)
                        .map(at::hm)
                        .or_else(|| parse_when(t, today, today).and_then(|(_, t)| t).map(at::hm))
                        .ok_or_else(|| format!("看不懂时刻「{t}」"))?),
                },
                Field::Rule => {
                    json!(parse_rule(text).ok_or_else(|| format!("看不懂重复规则「{text}」"))?)
                }
                Field::NewStep => continue,
            };
            patch.insert(key.into(), value);
        }
        // 移动开始时保持时长：只改开始没改结束时，结束跟着平移。
        if kind == Kind::Entry && patch.contains_key("start") && !patch.contains_key("end") {
            if let (Some(entry), Some(new_start)) = (
                data.entry(id),
                patch
                    .get("start")
                    .and_then(Value::as_str)
                    .and_then(at::parse_stamp),
            ) {
                if let (Some(s), Some(e)) =
                    (at::parse_stamp(&entry.start), at::parse_stamp(&entry.end))
                {
                    patch.insert("end".into(), json!(at::stamp(new_start + (e - s))));
                }
            }
        }
        Ok(patch)
    }

    /// 新建面板 → 一条 `create` 操作。
    pub fn create_op(&self, today: NaiveDate) -> Result<Value, String> {
        let kind = self.kind();
        let title = self.text(Field::Title).trim().to_owned();
        if title.is_empty() {
            return Err(format!("先写下{}的标题", kind.label()));
        }
        let mut op = json!({"op": "create", "kind": kind.wire()});
        let o = op.as_object_mut().expect("对象");
        o.insert(
            if kind == Kind::Project {
                "name"
            } else {
                "title"
            }
            .into(),
            json!(title),
        );
        let note = self.text(Field::Note).trim();
        if !note.is_empty() {
            o.insert("note".into(), json!(note));
        }
        if let Some(project) = &self.picks.project {
            o.insert("project_id".into(), json!(project));
        }
        if let Some(color) = &self.picks.color {
            o.insert("color".into(), json!(color));
        }
        let tags = tags_of(self.text(Field::Tags));
        if !tags.is_empty() {
            o.insert("tags".into(), json!(tags));
        }
        match kind {
            Kind::Task => {
                if let Some(goal) = &self.picks.parent {
                    o.insert("goal_id".into(), json!(goal));
                }
                o.insert("priority".into(), json!(self.picks.priority.wire()));
                let due = self.text(Field::Due).trim();
                if !due.is_empty() {
                    o.insert(
                        "due".into(),
                        json!(when_due(due, today)
                            .ok_or_else(|| format!("看不懂截止时间「{due}」"))?),
                    );
                }
                let planned = self.text(Field::Planned).trim();
                if !planned.is_empty() {
                    o.insert(
                        "planned_for".into(),
                        json!(when_date(planned, today)
                            .ok_or_else(|| format!("看不懂日期「{planned}」"))?),
                    );
                }
                if let Some(m) = minutes_of(self.text(Field::Estimate))? {
                    o.insert("estimate_minutes".into(), json!(m));
                }
                let source = self.text(Field::Source).trim();
                if !source.is_empty() {
                    o.insert("source".into(), json!(source));
                }
            }
            Kind::Entry => {
                let all_day = self.picks.all_day;
                let start_text = self.text(Field::Start);
                let (start_date, start_time) = parse_when(start_text, today, today)
                    .ok_or_else(|| format!("看不懂开始时间「{start_text}」"))?;
                let start = start_date
                    .and_time(start_time.unwrap_or(NaiveTime::from_hms_opt(9, 0, 0).unwrap()));
                let end_text = self.text(Field::End);
                let end = match parse_when(end_text, start_date, today) {
                    Some((d, Some(t))) => d.and_time(t),
                    Some((d, None)) if all_day => at::add_days(d, 1).and_time(NaiveTime::MIN),
                    Some((d, None)) => d.and_time(start.time()) + chrono::Duration::minutes(60),
                    None => start + chrono::Duration::minutes(60),
                };
                if all_day {
                    o.insert("start".into(), json!(at::date_key(start_date)));
                    o.insert(
                        "end".into(),
                        json!(at::date_key(at::add_days(end.date(), -1).max(start_date))),
                    );
                } else {
                    o.insert("start".into(), json!(at::stamp(start)));
                    o.insert("end".into(), json!(at::stamp(end)));
                }
                o.insert("all_day".into(), json!(all_day));
                if let Some(task) = &self.picks.parent {
                    o.insert("task_id".into(), json!(task));
                }
                let location = self.text(Field::Location).trim();
                if !location.is_empty() {
                    o.insert("location".into(), json!(location));
                }
            }
            Kind::Goal => {
                if let Some(wish) = &self.picks.parent {
                    o.insert("wish_id".into(), json!(wish));
                }
                let target = self.text(Field::TargetDate).trim();
                if !target.is_empty() {
                    o.insert(
                        "target_date".into(),
                        json!(when_date(target, today)
                            .ok_or_else(|| format!("看不懂日期「{target}」"))?),
                    );
                }
            }
            Kind::Routine => {
                if let Some(goal) = &self.picks.parent {
                    o.insert("goal_id".into(), json!(goal));
                }
                let rule = self.text(Field::Rule);
                o.insert(
                    "rule".into(),
                    json!(parse_rule(rule).ok_or_else(|| format!("看不懂重复规则「{rule}」"))?),
                );
                let time = self.text(Field::Time).trim();
                if !time.is_empty() {
                    let t = at::parse_hm(time)
                        .or_else(|| parse_when(time, today, today).and_then(|(_, t)| t))
                        .ok_or_else(|| format!("看不懂时刻「{time}」"))?;
                    o.insert("time".into(), json!(at::hm(t)));
                }
                if let Some(m) = minutes_of(self.text(Field::Duration))? {
                    o.insert("duration_minutes".into(), json!(m));
                }
                let start = self.text(Field::StartDate).trim();
                if !start.is_empty() {
                    o.insert(
                        "start_date".into(),
                        json!(when_date(start, today).ok_or("起始日无效")?),
                    );
                }
                let until = self.text(Field::Until).trim();
                if !until.is_empty() {
                    o.insert(
                        "until".into(),
                        json!(when_date(until, today).ok_or("截止日无效")?),
                    );
                }
            }
            Kind::Wish | Kind::Project => {}
        }
        Ok(op)
    }

    /// 可以用 Tab 切换的字段顺序。
    pub fn tab_order(&self) -> Vec<Field> {
        self.fields.iter().map(|(f, _)| *f).collect()
    }
}

// ---------- 绘制 ----------

const PAD: f32 = 18.0;
const LABEL_W: f32 = 72.0;
const ROW_H: f32 = 32.0;

struct Painter<'a> {
    list: &'a mut DrawList,
    lay: &'a mut Layout,
    clip: Rect,
    left: f32,
    right: f32,
    y: f32,
    p: &'a Palette,
    c: Colors,
}

impl Painter<'_> {
    fn push(&mut self, r: Rect, hit: Hit) {
        self.lay.push_clipped(r, self.clip, super::Hit::Panel(hit));
    }

    fn visible(&self, h: f32) -> bool {
        self.y + h >= self.clip.top && self.y <= self.clip.bottom
    }

    fn gap(&mut self, h: f32) {
        self.y += h;
    }

    fn section(&mut self, title: &str) {
        self.gap(10.0);
        self.list.text(
            Rect::new(self.left, self.y, self.right, self.y + 18.0),
            title.to_owned(),
            TextStyle::Caption,
            self.p.muted,
        );
        self.y += 22.0;
    }

    fn chip(&mut self, x: f32, label: &str, active: bool, color: u32, hit: Hit) -> f32 {
        let w = text::measure(label, TextStyle::Small) + 18.0;
        let r = Rect::new(x, self.y, x + w, self.y + 26.0);
        if active {
            self.list
                .rounded_rect(r, 13.0, theme::mix(color, self.p.background, 0.18));
            self.list.rounded_border(r, 13.0, color);
        } else {
            self.list.rounded_border(r, 13.0, self.p.border);
        }
        self.list.text_aligned(
            r,
            label,
            TextStyle::Small,
            if active { color } else { self.p.muted },
            Align::Center,
        );
        self.push(r, hit);
        x + w + 6.0
    }

    /// 一行可换行的选择片。
    fn chips(&mut self, items: Vec<(String, bool, u32, Hit)>) {
        let mut x = self.left;
        for (label, active, color, hit) in items {
            let w = text::measure(&label, TextStyle::Small) + 18.0;
            if x + w > self.right && x > self.left {
                x = self.left;
                self.y += 32.0;
            }
            x = self.chip(x, &label, active, color, hit);
        }
        self.y += 34.0;
    }

    fn action(&mut self, x: f32, label: &str, icon: Option<Icon>, style: Btn, hit: Hit) -> f32 {
        let w = button_width(label, icon);
        let r = Rect::new(x, self.y, x + w, self.y + 28.0);
        if self.visible(28.0) {
            button(
                self.list,
                self.lay,
                r,
                label,
                icon,
                style,
                super::Hit::Panel(hit.clone()),
                self.p,
            );
            self.lay.entries.pop();
            self.push(r, hit);
        }
        x + w + 6.0
    }

    fn link_row(&mut self, icon: Icon, color: u32, title: &str, meta: &str, hit: Hit) {
        let h = if meta.is_empty() { 30.0 } else { 42.0 };
        let r = Rect::new(self.left, self.y, self.right, self.y + h - 4.0);
        if self.visible(h) {
            self.list.rounded_rect(r, 7.0, self.p.surface_muted);
            self.list.icon_centered(
                Rect::new(r.left + 6.0, r.top, r.left + 26.0, r.top + 26.0),
                icon,
                13.0,
                color,
            );
            self.list.text_aligned(
                Rect::new(r.left + 30.0, r.top + 3.0, r.right - 8.0, r.top + 23.0),
                text::ellipsize(title, TextStyle::Small, r.width() - 40.0),
                TextStyle::Small,
                self.p.foreground,
                Align::Leading,
            );
            if !meta.is_empty() {
                self.list.text_aligned(
                    Rect::new(r.left + 30.0, r.top + 20.0, r.right - 8.0, r.bottom - 2.0),
                    text::ellipsize(meta, TextStyle::Tiny, r.width() - 40.0),
                    TextStyle::Tiny,
                    self.p.muted,
                    Align::Leading,
                );
            }
            self.push(r, hit);
        }
        self.y += h;
    }

    fn note(&mut self, text: &str, color: u32) {
        let lines = wrap_plain(text, TextStyle::Tiny, self.right - self.left);
        for line in lines {
            self.list.text(
                Rect::new(self.left, self.y, self.right, self.y + 16.0),
                line,
                TextStyle::Tiny,
                color,
            );
            self.y += 16.0;
        }
        self.y += 4.0;
    }
}

fn paint_field(
    pt: &mut Painter<'_>,
    pn: &mut Panel,
    field: Field,
    focus: Option<Focused>,
    label: Option<&str>,
) {
    let focused = focus == Some(Focused::Field(field));
    let multiline = field.multiline();
    let left = match label {
        Some(_) => pt.left + LABEL_W,
        None => pt.left,
    };
    let Some(tf) = pn.field_mut(field) else {
        return;
    };
    let h = if multiline {
        tf.multiline_height(pt.right - left).clamp(64.0, 220.0)
    } else if field == Field::Title {
        38.0
    } else {
        ROW_H
    };
    let r = Rect::new(left, pt.y, pt.right, pt.y + h);
    if let Some(label) = label {
        pt.list.text_aligned(
            Rect::new(pt.left, pt.y, left - 8.0, pt.y + ROW_H),
            label,
            TextStyle::Small,
            pt.p.muted,
            Align::Leading,
        );
    }
    if pt.visible(h) {
        if multiline {
            let look = FieldLook {
                radius: 6.0,
                background: Some(pt.p.surface_muted),
                border: Some(if focused {
                    pt.p.accent
                } else {
                    pt.p.surface_muted
                }),
                focus_ring: Some(pt.p.accent),
                padding_left: 10.0,
                padding_right: 10.0,
                leading_icon: None,
            };
            tf.paint_multiline_with_look(pt.list, r, focused, pt.p, look);
        } else {
            let hovered_bg = if focused {
                pt.p.background
            } else {
                theme::mix(pt.p.surface_muted, pt.p.background, 0.6)
            };
            let look = FieldLook {
                radius: 6.0,
                background: Some(if field == Field::Title {
                    pt.p.background
                } else {
                    hovered_bg
                }),
                border: Some(if focused {
                    pt.p.accent
                } else if field == Field::Title {
                    pt.p.background
                } else {
                    pt.p.border
                }),
                focus_ring: Some(pt.p.accent),
                padding_left: if field == Field::Title { 4.0 } else { 10.0 },
                padding_right: 8.0,
                leading_icon: None,
            };
            tf.paint(pt.list, r, focused, pt.p, look);
        }
        let visible = r.intersect(&pt.clip);
        if !visible.is_empty() {
            pt.lay.fields.push((field, r));
            pt.lay.push(visible, super::Hit::Panel(Hit::Field(field)));
        }
    }
    pt.y += h + 6.0;
}

fn header(pt: &mut Painter<'_>, kind: Kind, title: &str, is_new: bool) {
    let color = visual::kind_color(&pt.c, kind);
    let r = Rect::new(pt.left, pt.y, pt.right, pt.y + 28.0);
    pt.list.rounded_rect(
        Rect::new(r.left, r.top + 2.0, r.left + 24.0, r.top + 26.0),
        6.0,
        theme::mix(color, pt.p.background, 0.16),
    );
    pt.list.icon_centered(
        Rect::new(r.left, r.top + 2.0, r.left + 24.0, r.top + 26.0),
        visual::kind_icon(kind),
        13.0,
        color,
    );
    pt.list.text_aligned(
        Rect::new(r.left + 32.0, r.top, r.right - 36.0, r.bottom),
        if is_new {
            format!("新建{}", kind.label())
        } else {
            title.to_owned()
        },
        TextStyle::Small,
        color,
        Align::Leading,
    );
    let close = Rect::new(r.right - 26.0, r.top + 1.0, r.right, r.bottom - 1.0);
    pt.list.icon_centered(close, Icon::X, 13.0, pt.p.muted);
    pt.push(close, Hit::Close);
    pt.y += 36.0;
}

/// 「为了什么」：愿望 › 目标 › 任务，每一节都能点开。
fn chain(pt: &mut Painter<'_>, data: &AgendaData, id: &str) {
    let nodes = query::why_chain(data, id);
    if nodes.is_empty() {
        return;
    }
    let mut x = pt.left;
    pt.list.text_aligned(
        Rect::new(x, pt.y, x + 40.0, pt.y + 22.0),
        "为了",
        TextStyle::Tiny,
        pt.p.muted,
        Align::Leading,
    );
    x += 28.0;
    for (i, node) in nodes.iter().enumerate() {
        if i > 0 {
            pt.list.icon_centered(
                Rect::new(x, pt.y, x + 12.0, pt.y + 22.0),
                Icon::CHEVRON_RIGHT,
                10.0,
                pt.p.muted,
            );
            x += 14.0;
        }
        let color = visual::kind_color(&pt.c, node.kind);
        let label = text::ellipsize(
            &node.title,
            TextStyle::Tiny,
            ((pt.right - x) - 24.0).clamp(40.0, 150.0),
        );
        let w = text::measure(&label, TextStyle::Tiny) + 26.0;
        let r = Rect::new(x, pt.y + 1.0, x + w, pt.y + 21.0);
        pt.list
            .rounded_rect(r, 10.0, theme::mix(color, pt.p.background, 0.12));
        pt.list.icon_centered(
            Rect::new(r.left + 3.0, r.top, r.left + 17.0, r.bottom),
            visual::kind_icon(node.kind),
            10.0,
            color,
        );
        pt.list.text_aligned(
            Rect::new(r.left + 17.0, r.top, r.right - 4.0, r.bottom),
            label,
            TextStyle::Tiny,
            color,
            Align::Leading,
        );
        pt.push(r, Hit::Open(node.id.clone()));
        x += w + 2.0;
        if x > pt.right - 40.0 {
            break;
        }
    }
    pt.y += 28.0;
}

fn picker(
    pt: &mut Painter<'_>,
    label: &str,
    value: Option<(Kind, String)>,
    slot: PickSlot,
    empty: &str,
) {
    pt.list.text_aligned(
        Rect::new(pt.left, pt.y, pt.left + LABEL_W - 8.0, pt.y + ROW_H),
        label,
        TextStyle::Small,
        pt.p.muted,
        Align::Leading,
    );
    let r = Rect::new(pt.left + LABEL_W, pt.y, pt.right, pt.y + ROW_H);
    let has = value.is_some();
    pt.list.rounded_border(r, 6.0, pt.p.border);
    let mut x = r.left + 10.0;
    if let Some((kind, title)) = &value {
        let color = visual::kind_color(&pt.c, *kind);
        pt.list.icon_centered(
            Rect::new(x, r.top, x + 14.0, r.bottom),
            visual::kind_icon(*kind),
            12.0,
            color,
        );
        x += 20.0;
        pt.list.text_aligned(
            Rect::new(x, r.top, r.right - 30.0, r.bottom),
            text::ellipsize(title, TextStyle::Small, r.right - 36.0 - x),
            TextStyle::Small,
            pt.p.foreground,
            Align::Leading,
        );
    } else {
        pt.list.text_aligned(
            Rect::new(x, r.top, r.right - 30.0, r.bottom),
            empty,
            TextStyle::Small,
            pt.p.muted,
            Align::Leading,
        );
    }
    pt.push(r, Hit::Pick(slot));
    if has {
        let x = Rect::new(r.right - 26.0, r.top + 4.0, r.right - 4.0, r.bottom - 4.0);
        pt.list.icon_centered(x, Icon::X, 11.0, pt.p.muted);
        pt.push(x, Hit::Unlink(slot));
    } else {
        pt.list.icon_centered(
            Rect::new(r.right - 24.0, r.top, r.right - 6.0, r.bottom),
            Icon::CHEVRON_DOWN,
            11.0,
            pt.p.muted,
        );
    }
    pt.y += ROW_H + 6.0;
}

fn titled(data: &AgendaData, id: Option<&str>) -> Option<(Kind, String)> {
    let id = id?;
    let kind = Kind::of_id(id)?;
    let mut title = data.title_of(id)?;
    let deleted = match kind {
        Kind::Task => data.live_task(id).is_none(),
        Kind::Goal => data.live_goal(id).is_none(),
        Kind::Wish => data.live_wish(id).is_none(),
        Kind::Project => data.live_project(id).is_none(),
        _ => false,
    };
    if deleted {
        title.push_str("（已删除）");
    }
    Some((kind, title))
}

fn parent_label(kind: Kind) -> Option<(&'static str, &'static str)> {
    match kind {
        Kind::Task | Kind::Routine => Some(("目标", "为哪个目标服务？可以不选")),
        Kind::Goal => Some(("愿望", "来自哪个愿望？可以不选")),
        Kind::Entry => Some(("任务", "独立日程 · 可以关联到任务")),
        _ => None,
    }
}

fn swatches(pt: &mut Painter<'_>, current: Option<&str>) {
    pt.list.text_aligned(
        Rect::new(pt.left, pt.y, pt.left + LABEL_W - 8.0, pt.y + 24.0),
        "颜色",
        TextStyle::Small,
        pt.p.muted,
        Align::Leading,
    );
    let mut x = pt.left + LABEL_W;
    let auto = Rect::new(x, pt.y + 2.0, x + 20.0, pt.y + 22.0);
    pt.list.rounded_border(
        auto,
        10.0,
        if current.is_none() {
            pt.p.foreground
        } else {
            pt.p.border
        },
    );
    pt.list
        .text_aligned(auto, "A", TextStyle::Tiny, pt.p.muted, Align::Center);
    pt.push(auto, Hit::Color(None));
    x += 26.0;
    for (hex, _) in visual::SWATCHES {
        let Some(color) = visual::parse_color(hex) else {
            continue;
        };
        let r = Rect::new(x, pt.y + 2.0, x + 20.0, pt.y + 22.0);
        pt.list.rounded_rect(r, 10.0, color);
        if current.is_some_and(|c| c.eq_ignore_ascii_case(hex)) {
            pt.list.icon_centered(r, Icon::CHECK, 11.0, 0xFFFFFF);
        }
        pt.push(r, Hit::Color(Some((*hex).to_owned())));
        x += 26.0;
    }
    pt.y += 32.0;
}

fn steps(
    pt: &mut Painter<'_>,
    pn: &mut Panel,
    focus: Option<Focused>,
    items: &[core::Step],
    goal: bool,
) {
    pt.section(if goal { "达成标准" } else { "步骤" });
    for step in items {
        let r = Rect::new(pt.left, pt.y, pt.right, pt.y + 26.0);
        if pt.visible(26.0) {
            let check = Rect::new(r.left, r.top, r.left + 20.0, r.bottom);
            visual::check_circle(
                pt.list,
                check,
                step.done,
                if step.done { pt.c.success } else { pt.p.muted },
                pt.p,
            );
            let shown = text::ellipsize(&step.title, TextStyle::Small, r.width() - 56.0);
            pt.list.text_aligned(
                Rect::new(r.left + 26.0, r.top, r.right - 26.0, r.bottom),
                shown.clone(),
                TextStyle::Small,
                if step.done {
                    pt.p.muted
                } else {
                    pt.p.foreground
                },
                Align::Leading,
            );
            if step.done {
                let w = text::measure(&shown, TextStyle::Small);
                pt.list
                    .hline(r.left + 26.0, r.left + 26.0 + w, r.top + 13.0, pt.p.muted);
            }
            pt.push(
                Rect::new(r.left, r.top, r.right - 28.0, r.bottom),
                Hit::Step(step.id.clone()),
            );
            let x = Rect::new(r.right - 22.0, r.top + 3.0, r.right, r.bottom - 3.0);
            pt.list.icon_centered(x, Icon::X, 10.0, pt.p.muted);
            pt.push(x, Hit::StepRemove(step.id.clone()));
        }
        pt.y += 28.0;
    }
    paint_field(pt, pn, Field::NewStep, focus, None);
    if goal && !items.is_empty() {
        pt.note(
            "标准全部勾上也不会自动判定达成——结果由你在上方状态里决定。",
            pt.p.muted,
        );
    }
}

fn footer(pt: &mut Painter<'_>, archived: Option<bool>, deletable: bool) {
    pt.gap(14.0);
    pt.list.hline(pt.left, pt.right, pt.y, pt.p.border);
    pt.gap(12.0);
    let mut x = pt.left;
    if let Some(archived) = archived {
        let (label, icon) = if archived {
            ("取消归档", Icon::ROTATE_CCW)
        } else {
            ("归档", Icon::ARCHIVE)
        };
        x = pt.action(x, label, Some(icon), Btn::Ghost, Hit::Archive(!archived));
    }
    if deletable {
        pt.action(x, "删除", Some(Icon::TRASH2), Btn::Danger, Hit::Delete);
    }
    pt.y += 36.0;
}

fn future_prompt(pt: &mut Painter<'_>, prompt: &FuturePrompt) {
    let h = 108.0;
    let r = Rect::new(pt.left - 6.0, pt.y, pt.right + 6.0, pt.y + h);
    pt.list
        .rounded_rect(r, 10.0, theme::mix(pt.c.warning, pt.p.background, 0.12));
    pt.list.rounded_border(r, 10.0, pt.c.warning);
    pt.list.text(
        Rect::new(r.left + 12.0, r.top + 10.0, r.right - 12.0, r.top + 30.0),
        prompt.reason.clone(),
        TextStyle::Small,
        pt.p.foreground,
    );
    pt.list.text(
        Rect::new(r.left + 12.0, r.top + 32.0, r.right - 12.0, r.top + 50.0),
        format!(
            "它还有 {} 个未来的计划时间块。默认保留，需要时再取消。",
            prompt.entry_ids.len()
        ),
        TextStyle::Tiny,
        pt.p.muted,
    );
    pt.y += 62.0;
    let x = pt.left + 6.0;
    let x = pt.action(x, "保留后续安排", None, Btn::Primary, Hit::KeepFuture);
    pt.action(
        x,
        &format!("取消这 {} 个时间块", prompt.entry_ids.len()),
        None,
        Btn::Ghost,
        Hit::CancelFuture,
    );
    pt.y = r.bottom + 12.0;
}

/// 画检查器。`r` 是整栏。
#[allow(clippy::too_many_arguments)]
pub fn paint(
    list: &mut DrawList,
    r: Rect,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    lay: &mut Layout,
    focus: Option<Focused>,
    p: &Palette,
) {
    list.rect(r, p.background);
    list.vline(r.left, r.top, r.bottom, p.border);
    lay.push(r, super::Hit::Blank);
    let clip = Rect::new(r.left + 1.0, r.top, r.right, r.bottom);
    lay.panel_rect = r;
    lay.panel_viewport = clip;
    list.push_clip(clip);
    let mut pt = Painter {
        list,
        lay,
        clip,
        left: r.left + PAD,
        right: r.right - PAD,
        y: r.top + 14.0 - pn.scroll,
        p,
        c: visual::colors(p),
    };
    let start = pt.y;
    let kind = pn.kind();
    match pn.target.clone() {
        Target::New(kind) => paint_new(&mut pt, st, pn, data, kind, focus),
        Target::Record(key) => {
            if key.contains('@') {
                paint_occurrence(&mut pt, st, data, &key);
            } else {
                match kind {
                    Kind::Task => paint_task(&mut pt, st, pn, data, &key, focus),
                    Kind::Entry => paint_entry(&mut pt, st, pn, data, &key, focus),
                    Kind::Goal => paint_goal(&mut pt, st, pn, data, &key, focus),
                    Kind::Wish => paint_wish(&mut pt, st, pn, data, &key, focus),
                    Kind::Routine => paint_routine(&mut pt, st, pn, data, &key, focus),
                    Kind::Project => paint_project(&mut pt, st, pn, data, &key, focus),
                }
            }
        }
    }
    if !pn.error.is_empty() {
        pt.gap(6.0);
        let color = pt.c.danger;
        let error = pn.error.clone();
        pt.note(&error, color);
    }
    let height = pt.y - start + 40.0;
    list.pop_clip();
    lay.panel_height = height;
}

fn missing(pt: &mut Painter<'_>) {
    pt.note("这条记录已不存在。", pt.p.muted);
}

fn paint_new(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    kind: Kind,
    focus: Option<Focused>,
) {
    header(pt, kind, "", true);
    // 可以在这里换种类：用户必须能直接创建任何一种，不必从愿望开始。
    let kinds = [
        Kind::Task,
        Kind::Entry,
        Kind::Goal,
        Kind::Wish,
        Kind::Routine,
        Kind::Project,
    ];
    let items = kinds
        .iter()
        .map(|k| {
            (
                k.label().to_owned(),
                *k == kind,
                visual::kind_color(&pt.c, *k),
                Hit::NewKind(*k),
            )
        })
        .collect();
    pt.chips(items);
    paint_field(pt, pn, Field::Title, focus, None);
    match kind {
        Kind::Task => {
            let items = Priority::ALL
                .iter()
                .map(|x| {
                    let color = visual::priority_color(&pt.c, *x).unwrap_or(pt.c.task);
                    (
                        format!("{}优先", x.label()),
                        pn.picks.priority == *x,
                        color,
                        Hit::Priority(*x),
                    )
                })
                .collect();
            pt.chips(items);
            for f in [
                Field::Due,
                Field::Planned,
                Field::Estimate,
                Field::Source,
                Field::Tags,
            ] {
                paint_field(pt, pn, f, focus, Some(f.label()));
            }
        }
        Kind::Entry => {
            pt.chips(vec![
                (
                    "定时".into(),
                    !pn.picks.all_day,
                    pt.c.entry,
                    Hit::AllDay(false),
                ),
                (
                    "全天".into(),
                    pn.picks.all_day,
                    pt.c.entry,
                    Hit::AllDay(true),
                ),
            ]);
            for f in [Field::Start, Field::End, Field::Location] {
                paint_field(pt, pn, f, focus, Some(f.label()));
            }
            swatches(pt, pn.picks.color.as_deref());
        }
        Kind::Goal => paint_field(
            pt,
            pn,
            Field::TargetDate,
            focus,
            Some(Field::TargetDate.label()),
        ),
        Kind::Routine => {
            rule_presets(pt, pn.text(Field::Rule));
            for f in [
                Field::Rule,
                Field::Time,
                Field::Duration,
                Field::StartDate,
                Field::Until,
            ] {
                paint_field(pt, pn, f, focus, Some(f.label()));
            }
            swatches(pt, pn.picks.color.as_deref());
        }
        Kind::Wish | Kind::Project => {}
    }
    if let Some((label, empty)) = parent_label(kind) {
        picker(
            pt,
            label,
            titled(data, pn.picks.parent.as_deref()),
            PickSlot::Parent,
            empty,
        );
    }
    if kind != Kind::Project {
        picker(
            pt,
            "项目",
            titled(data, pn.picks.project.as_deref()),
            PickSlot::Project,
            "不归入项目",
        );
    }
    paint_field(pt, pn, Field::Note, focus, None);
    pt.gap(6.0);
    let x = pt.action(
        pt.left,
        &format!("创建{}", kind.label()),
        Some(Icon::PLUS),
        Btn::Primary,
        Hit::Create,
    );
    pt.list.text_aligned(
        Rect::new(x + 4.0, pt.y, pt.right, pt.y + 28.0),
        "回车创建 · Esc 关闭",
        TextStyle::Tiny,
        pt.p.muted,
        Align::Leading,
    );
    pt.y += 40.0;
    let hint = match kind {
        Kind::Wish => "愿望可以很模糊，也可以只是一个还没决定行动的想法。不需要期限、标准或目标。",
        Kind::Goal => "目标是决定追求的结果，有自己的状态和结果判断。可以不挂在任何愿望下。",
        Kind::Task => "任务不强制截止，也不必一天做完。之后可以把它拖进时间轴安排多次。",
        Kind::Entry => "日程是某段时间的安排。关联任务后它就是该任务的一个时间块，标题跟随任务。",
        Kind::Routine => "每天 / 每周重复的事。时间过去后会请你确认是否执行，系统不会替你判断。",
        Kind::Project => "项目是横向归类：任务、日程、目标都可以归入。",
    };
    pt.note(hint, pt.p.muted);
    let _ = st;
}

fn rule_presets(pt: &mut Painter<'_>, current: &str) {
    let normalized = parse_rule(current);
    let items = core::recur::PRESETS
        .iter()
        .map(|(rule, label)| {
            (
                (*label).to_owned(),
                normalized.as_deref() == core::recur::normalize(rule).as_deref(),
                pt.c.routine,
                Hit::RulePreset(label),
            )
        })
        .collect();
    pt.chips(items);
}

fn paint_task(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    id: &str,
    focus: Option<Focused>,
) {
    let Some(task) = data.task(id) else {
        return missing(pt);
    };
    header(pt, Kind::Task, "任务", false);
    chain(pt, data, id);
    if task.deleted_at.is_some() {
        pt.note(
            "这个任务已删除，历史时间块仍保留。恢复后才能继续编辑。",
            pt.c.danger,
        );
        let x = pt.left;
        pt.action(
            x,
            "从回收站恢复",
            Some(Icon::ROTATE_CCW),
            Btn::Primary,
            Hit::Restore,
        );
        pt.y += 40.0;
        return;
    }
    paint_field(pt, pn, Field::Title, focus, None);
    let facts = query::task_facts(data, task, st.now);
    // 计算出的展示信息，与生命周期分开。
    let mut badges: Vec<(String, u32)> = Vec::new();
    if facts.overdue {
        badges.push(("逾期".into(), pt.c.danger));
    }
    if facts.scheduled {
        badges.push(("已排期".into(), pt.c.task));
    }
    if facts.unconfirmed > 0 {
        badges.push((
            format!("{} 个时间块待确认", facts.unconfirmed),
            pt.c.warning,
        ));
    }
    if task.archived_at.is_some() {
        badges.push(("已归档".into(), pt.p.muted));
    }
    if !badges.is_empty() {
        let mut x = pt.left;
        for (label, color) in badges {
            x += visual::badge(pt.list, x, pt.y, &label, color, pt.p);
        }
        pt.y += 26.0;
    }
    let items = TaskStatus::ALL
        .iter()
        .map(|s| {
            let color = match s {
                TaskStatus::Done => pt.c.success,
                TaskStatus::Cancelled => pt.p.muted,
                _ => pt.c.task,
            };
            (
                s.label().to_owned(),
                task.status == *s,
                color,
                Hit::TaskStatus(*s),
            )
        })
        .collect();
    pt.chips(items);
    if let Some(prompt) = pn.prompt.clone() {
        future_prompt(pt, &prompt);
    }
    let items = Priority::ALL
        .iter()
        .map(|x| {
            let color = visual::priority_color(&pt.c, *x).unwrap_or(pt.c.task);
            (
                format!("{}优先", x.label()),
                task.priority == *x,
                color,
                Hit::Priority(*x),
            )
        })
        .collect();
    pt.chips(items);
    for f in [Field::Due, Field::Planned, Field::Estimate] {
        paint_field(pt, pn, f, focus, Some(f.label()));
    }
    picker(
        pt,
        "目标",
        titled(data, task.goal_id.as_deref()),
        PickSlot::Parent,
        "不属于任何目标 · 可以独立存在",
    );
    picker(
        pt,
        "项目",
        titled(data, task.project_id.as_deref()),
        PickSlot::Project,
        "不归入项目",
    );
    for f in [Field::Source, Field::Tags] {
        paint_field(pt, pn, f, focus, Some(f.label()));
    }

    // 时间块：同一个任务可以排很多次，每次都只是新增安排。
    let blocks = query::task_blocks(data, id);
    let head = if blocks.is_empty() {
        "时间安排".to_owned()
    } else {
        format!(
            "时间安排 · {} 次 · 已投入 {} · 待进行 {}",
            blocks.len(),
            at::duration_short(facts.done_minutes),
            at::duration_short(facts.planned_minutes)
        )
    };
    pt.section(&head);
    for entry in &blocks {
        let (Some(s), Some(e)) = (at::parse_stamp(&entry.start), at::parse_stamp(&entry.end))
        else {
            continue;
        };
        let status_color = visual::entry_status_color(&pt.c, entry.status);
        let mut meta = entry.status.label().to_owned();
        if entry.status == EntryStatus::Planned && e <= st.now {
            meta = "待确认".into();
        }
        if !entry.note.is_empty() {
            meta.push_str(" · ");
            meta.push_str(&entry.note);
        }
        let title = format!(
            "{} {}–{} · {}",
            at::relative_date_label(s.date(), st.today()),
            at::hm(s.time()),
            at::hm(e.time()),
            at::duration_short(at::minutes_between(s, e))
        );
        pt.link_row(
            Icon::CLOCK,
            status_color,
            &title,
            &meta,
            Hit::Open(entry.id.clone()),
        );
    }
    let x = pt.action(
        pt.left,
        "再安排一次",
        Some(Icon::PLUS),
        Btn::Ghost,
        Hit::ScheduleTask,
    );
    pt.list.text_aligned(
        Rect::new(x, pt.y, pt.right, pt.y + 28.0),
        "或把任务拖进时间轴",
        TextStyle::Tiny,
        pt.p.muted,
        Align::Leading,
    );
    pt.y += 36.0;
    let steps_list = task.steps.clone();
    steps(pt, pn, focus, &steps_list, false);
    pt.section("备注");
    paint_field(pt, pn, Field::Note, focus, None);
    if let Some(done) = task.completed_at.as_deref() {
        pt.note(
            &format!(
                "完成于 {}",
                done.replace('T', " ").chars().take(16).collect::<String>()
            ),
            pt.p.muted,
        );
    }
    footer(pt, Some(task.archived_at.is_some()), true);
}

fn paint_entry(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    id: &str,
    focus: Option<Focused>,
) {
    let Some(entry) = data.entry(id) else {
        return missing(pt);
    };
    let linked = entry.task_id.is_some() || entry.detached_task_title.is_some();
    header(
        pt,
        Kind::Entry,
        if linked {
            "任务时间块"
        } else if entry.routine_id.is_some() {
            "重复安排的一次"
        } else {
            "日程"
        },
        false,
    );
    chain(pt, data, id);
    if linked {
        let title = data.entry_title(entry);
        let r = Rect::new(pt.left, pt.y, pt.right, pt.y + 34.0);
        pt.list.text_aligned(
            Rect::new(r.left + 4.0, r.top, r.right - 70.0, r.bottom),
            text::ellipsize(&title, TextStyle::Body16, r.width() - 80.0),
            TextStyle::Body16,
            pt.p.foreground,
            Align::Leading,
        );
        if let Some(task) = entry.task_id.as_deref().and_then(|t| data.live_task(t)) {
            let open = Rect::new(r.right - 64.0, r.top + 5.0, r.right, r.bottom - 5.0);
            pt.list.text_aligned(
                open,
                "打开任务 ›",
                TextStyle::Small,
                pt.c.task,
                Align::Trailing,
            );
            pt.push(open, Hit::Open(task.id.clone()));
        }
        pt.y += 38.0;
        if data.entry_task_deleted(entry) {
            pt.note("原任务已删除。这条时间块作为历史记录保留。", pt.c.danger);
        } else {
            pt.note("标题跟随任务；这里的备注只属于这一次安排。", pt.p.muted);
        }
    } else {
        paint_field(pt, pn, Field::Title, focus, None);
    }
    // 执行状态只描述这一次安排。
    let passed = at::parse_stamp(&entry.end).is_some_and(|e| e <= st.now);
    if passed && entry.status == EntryStatus::Planned {
        pt.note("时间已过，执行了吗？系统不会自动替你标记。", pt.c.warning);
    }
    let items = EntryStatus::ALL
        .iter()
        .map(|s| {
            (
                s.label().to_owned(),
                entry.status == *s,
                visual::entry_status_color(&pt.c, *s),
                Hit::EntryStatus(*s),
            )
        })
        .collect();
    pt.chips(items);
    if let Some(task) = entry.task_id.as_deref().and_then(|t| data.live_task(t)) {
        if task.status.is_open() {
            let x = pt.left;
            pt.action(
                x,
                "已执行，并完成任务",
                Some(Icon::CHECK),
                Btn::Ghost,
                Hit::CompleteBlockAndTask,
            );
            pt.y += 34.0;
            pt.note(
                "只标「已执行」不会完成任务：任务可能还需要下一次。",
                pt.p.muted,
            );
        }
    }
    if let Some(prompt) = pn.prompt.clone() {
        future_prompt(pt, &prompt);
    }
    for f in [Field::Start, Field::End] {
        paint_field(
            pt,
            pn,
            f,
            focus,
            Some(if entry.all_day {
                if f == Field::Start {
                    "开始日"
                } else {
                    "结束日"
                }
            } else {
                f.label()
            }),
        );
    }
    let minutes = query::entry_minutes(entry);
    if minutes > 0 {
        pt.note(&format!("共 {}", at::duration_label(minutes)), pt.p.muted);
    }
    for f in [Field::Location, Field::Actual] {
        if pn.has(f) {
            paint_field(pt, pn, f, focus, Some(f.label()));
        }
    }
    if entry.routine_id.is_none() && !data.entry_task_deleted(entry) {
        picker(
            pt,
            "任务",
            titled(data, entry.task_id.as_deref()),
            PickSlot::Parent,
            "独立日程 · 可以关联到任务",
        );
    }
    if entry.task_id.is_none() {
        picker(
            pt,
            "项目",
            titled(data, entry.project_id.as_deref()),
            PickSlot::Project,
            "不归入项目",
        );
        swatches(pt, entry.color.as_deref());
    }
    if let Some(routine) = entry
        .routine_id
        .as_deref()
        .and_then(|r| data.live_routine(r))
    {
        pt.link_row(
            Icon::REPEAT,
            pt.c.routine,
            &routine.title,
            &core::recur::describe(&routine.rule),
            Hit::Open(routine.id.clone()),
        );
    }
    pt.section("这次的备注");
    paint_field(pt, pn, Field::Note, focus, None);
    footer(pt, None, true);
    pt.note("删除只影响这一次安排，不会删除或完成关联任务。", pt.p.muted);
}

fn paint_occurrence(pt: &mut Painter<'_>, st: &State, data: &AgendaData, key: &str) {
    let Some(slot) = query::slot_by_key(data, key, st.now) else {
        return missing(pt);
    };
    header(pt, Kind::Routine, "重复安排的一次", false);
    chain(pt, data, key.split('@').next().unwrap_or(key));
    pt.list.text(
        Rect::new(pt.left + 4.0, pt.y, pt.right, pt.y + 26.0),
        text::ellipsize(&slot.title, TextStyle::Body16, pt.right - pt.left),
        TextStyle::Body16,
        pt.p.foreground,
    );
    pt.y += 32.0;
    let when = if slot.all_day {
        format!("{} · 全天", at::date_label(slot.start.date()))
    } else {
        format!(
            "{} {}–{}",
            at::date_label(slot.start.date()),
            at::hm(slot.start.time()),
            at::hm(slot.end.time())
        )
    };
    pt.note(&when, pt.p.muted);
    if slot.time_passed {
        pt.note("时间已过，执行了吗？", pt.c.warning);
    }
    let items = [
        EntryStatus::Done,
        EntryStatus::Skipped,
        EntryStatus::Cancelled,
    ]
    .iter()
    .map(|s| {
        (
            s.label().to_owned(),
            false,
            visual::entry_status_color(&pt.c, *s),
            Hit::EntryStatus(*s),
        )
    })
    .collect();
    pt.chips(items);
    pt.note(
        "标记或修改这一天时，它才会成为一条独立记录；其余日子不受影响。",
        pt.p.muted,
    );
    if let Some(routine) = slot
        .routine_id
        .as_deref()
        .and_then(|r| data.live_routine(r))
    {
        pt.section("来自");
        pt.link_row(
            Icon::REPEAT,
            pt.c.routine,
            &routine.title,
            &core::recur::describe(&routine.rule),
            Hit::Open(routine.id.clone()),
        );
    }
}

fn paint_goal(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    id: &str,
    focus: Option<Focused>,
) {
    let Some(goal) = data.goal(id) else {
        return missing(pt);
    };
    header(pt, Kind::Goal, "目标", false);
    chain(pt, data, id);
    if goal.deleted_at.is_some() {
        pt.note(
            "这个目标已删除，原下属任务已解除关联并继续独立存在。",
            pt.c.danger,
        );
        pt.action(
            pt.left,
            "从回收站恢复",
            Some(Icon::ROTATE_CCW),
            Btn::Primary,
            Hit::Restore,
        );
        pt.y += 40.0;
        return;
    }
    paint_field(pt, pn, Field::Title, focus, None);
    let items = GoalStatus::ALL
        .iter()
        .map(|s| {
            let color = match s {
                GoalStatus::Achieved => pt.c.success,
                GoalStatus::Abandoned => pt.p.muted,
                _ => pt.c.goal,
            };
            (
                s.label().to_owned(),
                goal.status == *s,
                color,
                Hit::GoalStatus(*s),
            )
        })
        .collect();
    pt.chips(items);
    let progress = query::goal_progress(data, id, st.now);
    let bar = Rect::new(pt.left, pt.y + 4.0, pt.right, pt.y + 10.0);
    visual::progress_bar(pt.list, bar, progress.ratio(), pt.c.goal, pt.p);
    pt.y += 16.0;
    let mut facts = vec![format!(
        "任务 {}/{}",
        progress.tasks_done, progress.tasks_total
    )];
    if progress.criteria_total > 0 {
        facts.push(format!(
            "标准 {}/{}",
            progress.criteria_done, progress.criteria_total
        ));
    }
    if progress.tasks_overdue > 0 {
        facts.push(format!("{} 项逾期", progress.tasks_overdue));
    }
    facts.push(format!(
        "已投入 {}",
        at::duration_short(progress.invested_minutes)
    ));
    if progress.planned_minutes > 0 {
        facts.push(format!(
            "已排 {}",
            at::duration_short(progress.planned_minutes)
        ));
    }
    pt.note(&facts.join(" · "), pt.p.muted);
    if progress.tasks_total > 0 && progress.tasks_open == 0 && goal.status.is_open() {
        pt.note("任务都完成了。结果达到了吗？达成与否由你判定。", pt.c.goal);
    }
    paint_field(
        pt,
        pn,
        Field::TargetDate,
        focus,
        Some(Field::TargetDate.label()),
    );
    picker(
        pt,
        "愿望",
        titled(data, goal.wish_id.as_deref()),
        PickSlot::Parent,
        "不来自愿望 · 可以独立存在",
    );
    picker(
        pt,
        "项目",
        titled(data, goal.project_id.as_deref()),
        PickSlot::Project,
        "不归入项目",
    );
    paint_field(pt, pn, Field::Tags, focus, Some(Field::Tags.label()));
    let criteria = goal.criteria.clone();
    steps(pt, pn, focus, &criteria, true);

    pt.section("拆出的任务");
    let mut tasks: Vec<&core::Task> = data
        .tasks
        .iter()
        .filter(|t| t.deleted_at.is_none() && t.goal_id.as_deref() == Some(id))
        .collect();
    query::sort_tasks(&mut tasks, st.now);
    for task in tasks.iter().take(30) {
        let meta: Vec<String> = task_meta(data, task, st.now)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        let color = if task.status == TaskStatus::Done {
            pt.c.success
        } else {
            pt.c.task
        };
        let label = format!(
            "{}{}",
            if task.status == TaskStatus::Done {
                "✓ "
            } else {
                ""
            },
            task.title
        );
        pt.link_row(
            visual::kind_icon(Kind::Task),
            color,
            &label,
            &meta.join(" · "),
            Hit::Open(task.id.clone()),
        );
    }
    let routines: Vec<&core::Routine> = data
        .routines
        .iter()
        .filter(|r| r.deleted_at.is_none() && r.goal_id.as_deref() == Some(id))
        .collect();
    for routine in routines {
        pt.link_row(
            Icon::REPEAT,
            pt.c.routine,
            &routine.title,
            &core::recur::describe(&routine.rule),
            Hit::Open(routine.id.clone()),
        );
    }
    let x = pt.action(
        pt.left,
        "拆一个任务",
        Some(Icon::PLUS),
        Btn::Ghost,
        Hit::NewChild(Kind::Task),
    );
    pt.action(
        x,
        "加一个重复安排",
        Some(Icon::REPEAT),
        Btn::Ghost,
        Hit::NewChild(Kind::Routine),
    );
    pt.y += 36.0;
    pt.section("备注");
    paint_field(pt, pn, Field::Note, focus, None);
    pt.section("结果复盘");
    paint_field(pt, pn, Field::ResultNote, focus, None);
    footer(pt, Some(goal.archived_at.is_some()), true);
    pt.note(
        "删除目标不会删除任务：它们会解除归属，继续独立存在。",
        pt.p.muted,
    );
}

fn paint_wish(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    id: &str,
    focus: Option<Focused>,
) {
    let Some(wish) = data.wish(id) else {
        return missing(pt);
    };
    header(pt, Kind::Wish, "愿望", false);
    if wish.deleted_at.is_some() {
        pt.note(
            "这个愿望已删除，原目标已解除关联并继续独立存在。",
            pt.c.danger,
        );
        pt.action(
            pt.left,
            "从回收站恢复",
            Some(Icon::ROTATE_CCW),
            Btn::Primary,
            Hit::Restore,
        );
        pt.y += 40.0;
        return;
    }
    paint_field(pt, pn, Field::Title, focus, None);
    let items = WishStatus::ALL
        .iter()
        .map(|s| {
            let color = if *s == WishStatus::Realized {
                pt.c.success
            } else {
                pt.c.wish
            };
            (
                s.label().to_owned(),
                wish.status == *s,
                color,
                Hit::WishStatus(*s),
            )
        })
        .collect();
    pt.chips(items);
    let progress = query::wish_progress(data, id, st.now);
    if progress.goals_total > 0 {
        pt.note(
            &format!(
                "{} 个目标，已达成 {} · 累计投入 {}",
                progress.goals_total,
                progress.goals_achieved,
                at::duration_label(progress.invested_minutes)
            ),
            pt.p.muted,
        );
    }
    picker(
        pt,
        "项目",
        titled(data, wish.project_id.as_deref()),
        PickSlot::Project,
        "不归入项目",
    );
    paint_field(pt, pn, Field::Tags, focus, Some(Field::Tags.label()));
    pt.section("为它设立的目标");
    for goal in data
        .goals
        .iter()
        .filter(|g| g.deleted_at.is_none() && g.wish_id.as_deref() == Some(id))
    {
        let p = query::goal_progress(data, &goal.id, st.now);
        let meta = format!(
            "{} · 任务 {}/{}",
            goal.status.label(),
            p.tasks_done,
            p.tasks_total
        );
        pt.link_row(
            Icon::TARGET,
            pt.c.goal,
            &goal.title,
            &meta,
            Hit::Open(goal.id.clone()),
        );
    }
    pt.action(
        pt.left,
        "设立一个目标",
        Some(Icon::PLUS),
        Btn::Ghost,
        Hit::NewChild(Kind::Goal),
    );
    pt.y += 36.0;
    pt.section("备注");
    paint_field(pt, pn, Field::Note, focus, None);
    footer(pt, Some(wish.archived_at.is_some()), true);
}

fn paint_routine(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    id: &str,
    focus: Option<Focused>,
) {
    let Some(routine) = data.routine(id) else {
        return missing(pt);
    };
    header(pt, Kind::Routine, "重复安排", false);
    chain(pt, data, id);
    if routine.deleted_at.is_some() {
        pt.note(
            "这个重复安排已删除。已确认过的日子仍保留在历史里。",
            pt.c.danger,
        );
        pt.action(
            pt.left,
            "从回收站恢复",
            Some(Icon::ROTATE_CCW),
            Btn::Primary,
            Hit::Restore,
        );
        pt.y += 40.0;
        return;
    }
    paint_field(pt, pn, Field::Title, focus, None);
    let active = routine.status == RoutineStatus::Active;
    pt.chips(vec![
        (
            "进行中".into(),
            active,
            pt.c.routine,
            Hit::RoutineActive(true),
        ),
        (
            "已暂停".into(),
            !active,
            pt.p.muted,
            Hit::RoutineActive(false),
        ),
    ]);
    let stats = query::routine_stats(data, routine, st.today());
    let mut x = pt.left;
    for (_, status) in &stats.recent {
        let dot = Rect::new(x, pt.y, x + 14.0, pt.y + 14.0);
        let fill = match status {
            Some(EntryStatus::Done) => pt.c.routine,
            Some(EntryStatus::Skipped) => theme::mix(pt.c.danger, pt.p.surface, 0.5),
            Some(_) => pt.p.border,
            None => pt.p.background,
        };
        pt.list.rounded_rect(dot, 3.0, fill);
        if status.is_none() {
            pt.list.rounded_border(dot, 3.0, pt.p.border);
        }
        x += 17.0;
    }
    pt.y += 20.0;
    pt.note(
        &format!(
            "连续 {} 次 · 近 30 天完成 {}/{} · 空格是还没确认的日子",
            stats.streak, stats.done_30, stats.expected_30
        ),
        pt.p.muted,
    );
    rule_presets(pt, pn.text(Field::Rule));
    for f in [
        Field::Rule,
        Field::Time,
        Field::Duration,
        Field::StartDate,
        Field::Until,
    ] {
        paint_field(pt, pn, f, focus, Some(f.label()));
    }
    picker(
        pt,
        "目标",
        titled(data, routine.goal_id.as_deref()),
        PickSlot::Parent,
        "为哪个目标服务？可以不选",
    );
    picker(
        pt,
        "项目",
        titled(data, routine.project_id.as_deref()),
        PickSlot::Project,
        "不归入项目",
    );
    swatches(pt, routine.color.as_deref());
    pt.section("备注");
    paint_field(pt, pn, Field::Note, focus, None);
    footer(pt, Some(routine.archived_at.is_some()), true);
}

fn paint_project(
    pt: &mut Painter<'_>,
    st: &State,
    pn: &mut Panel,
    data: &AgendaData,
    id: &str,
    focus: Option<Focused>,
) {
    let Some(project) = data.project(id) else {
        return missing(pt);
    };
    header(pt, Kind::Project, "项目", false);
    if project.deleted_at.is_some() {
        pt.note("这个项目已删除，其中的记录已解除归类。", pt.c.danger);
        pt.action(
            pt.left,
            "从回收站恢复",
            Some(Icon::ROTATE_CCW),
            Btn::Primary,
            Hit::Restore,
        );
        pt.y += 40.0;
        return;
    }
    paint_field(pt, pn, Field::Title, focus, None);
    let items = ProjectStatus::ALL
        .iter()
        .map(|s| {
            (
                s.label().to_owned(),
                project.status == *s,
                pt.c.muted,
                Hit::ProjectStatus(*s),
            )
        })
        .collect();
    pt.chips(items);
    swatches(pt, project.color.as_deref());
    let pid = Some(id);
    pt.section("目标");
    for goal in data
        .goals
        .iter()
        .filter(|g| g.deleted_at.is_none() && g.project_id.as_deref() == pid)
    {
        pt.link_row(
            Icon::TARGET,
            pt.c.goal,
            &goal.title,
            goal.status.label(),
            Hit::Open(goal.id.clone()),
        );
    }
    pt.section("未完成的任务");
    let mut tasks: Vec<&core::Task> = data
        .tasks
        .iter()
        .filter(|t| {
            query::is_visible_task(t) && t.status.is_open() && t.project_id.as_deref() == pid
        })
        .collect();
    query::sort_tasks(&mut tasks, st.now);
    for task in tasks.iter().take(30) {
        let meta: Vec<String> = task_meta(data, task, st.now)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        pt.link_row(
            visual::kind_icon(Kind::Task),
            pt.c.task,
            &task.title,
            &meta.join(" · "),
            Hit::Open(task.id.clone()),
        );
    }
    pt.action(
        pt.left,
        "新建任务",
        Some(Icon::PLUS),
        Btn::Ghost,
        Hit::NewChild(Kind::Task),
    );
    pt.y += 36.0;
    pt.section("备注");
    paint_field(pt, pn, Field::Note, focus, None);
    footer(pt, Some(project.archived_at.is_some()), true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_core::agenda::{Clock, Editor, EntryDraft, TaskDraft};

    fn day(s: &str) -> NaiveDate {
        at::parse_date(s).unwrap()
    }

    #[test]
    fn loose_times_accept_natural_language_and_bare_clock() {
        let today = day("2026-09-26");
        assert_eq!(
            parse_when("2026-09-28 14:00", today, today),
            Some((day("2026-09-28"), NaiveTime::from_hms_opt(14, 0, 0)))
        );
        assert_eq!(
            parse_when("15:30", day("2026-10-01"), today),
            Some((day("2026-10-01"), NaiveTime::from_hms_opt(15, 30, 0)))
        );
        assert_eq!(
            parse_when("明天 下午3点", today, today),
            Some((day("2026-09-27"), NaiveTime::from_hms_opt(15, 0, 0)))
        );
        assert_eq!(when_due("周五", today).as_deref(), Some("2026-10-02"));
        assert_eq!(parse_when("随便", today, today), None);
        assert_eq!(minutes_of("1.5h"), Ok(Some(90)));
        assert_eq!(
            parse_rule("每周一三").as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,WE")
        );
    }

    #[test]
    fn edits_only_send_changed_fields_and_moving_start_keeps_length() {
        let mut data = AgendaData::default();
        let now = at::parse_stamp("2026-09-26T08:00").unwrap();
        let mut ed = Editor::new(&mut data, Clock::fixed(now));
        let task = ed
            .create_task(TaskDraft {
                title: "数学作业".into(),
                ..Default::default()
            })
            .unwrap();
        let entry = ed
            .create_entry(EntryDraft {
                task_id: Some(task.clone()),
                start: "2026-09-26T19:00".into(),
                end: "2026-09-26T20:30".into(),
                ..Default::default()
            })
            .unwrap();
        let mut panel = Panel::open(&data, &task).unwrap();
        assert!(panel.changes(&data, now.date()).unwrap().is_empty());
        panel.field_mut(Field::Due).unwrap().set_text("明天 18:00");
        let patch = panel.changes(&data, now.date()).unwrap();
        assert_eq!(patch.len(), 1);
        assert_eq!(patch["due"], json!("2026-09-27T18:00"));

        let mut panel = Panel::open(&data, &entry).unwrap();
        assert!(!panel.has(Field::Title), "关联任务的时间块不维护自己的标题");
        panel.field_mut(Field::Start).unwrap().set_text("20:00");
        let patch = panel.changes(&data, now.date()).unwrap();
        assert_eq!(patch["start"], json!("2026-09-26T20:00"));
        assert_eq!(patch["end"], json!("2026-09-26T21:30"));
    }

    #[test]
    fn new_entry_from_a_dragged_span_becomes_a_create_op() {
        let today = day("2026-09-26");
        let start = at::parse_stamp("2026-09-26T14:00").unwrap();
        let mut panel = Panel::new_record(
            Kind::Entry,
            today,
            Some((start, start + chrono::Duration::minutes(30))),
            None,
        );
        assert!(panel.create_op(today).is_err());
        panel.field_mut(Field::Title).unwrap().set_text("开会");
        let op = panel.create_op(today).unwrap();
        assert_eq!(op["start"], json!("2026-09-26T14:00"));
        assert_eq!(op["end"], json!("2026-09-26T14:30"));
        assert_eq!(op["kind"], json!("entry"));
    }
}
