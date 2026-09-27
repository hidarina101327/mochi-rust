//! 所有写操作。每个操作只改它声明要改的字段，不做任何静默级联：
//!
//! - 日程结束不会自动标记已执行，任务不会因此完成；
//! - 确认时间块已执行不会完成任务（[`Editor::complete_entry_and_task`] 是显式的组合操作）；
//! - 任务全部完成不会达成目标，目标达成不会实现愿望；
//! - 关联 / 取消关联 / 改父级只改关系字段；
//! - 归档只改默认可见性；删除父级只解除子项关联；删除任务留墓碑。

use anyhow::{anyhow, bail, Result};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::model::*;
use super::recur;
use super::time;

/// 操作用的「现在」：本地墙钟 + UTC 元数据时间戳。测试里可以固定。
#[derive(Debug, Clone)]
pub struct Clock {
    pub now: NaiveDateTime,
    pub utc: String,
}

impl Clock {
    pub fn system() -> Self {
        Self {
            now: time::now(),
            utc: crate::jstime::now(),
        }
    }

    pub fn fixed(now: NaiveDateTime) -> Self {
        Self {
            now,
            utc: format!("{}:00.000Z", time::stamp(now)),
        }
    }

    pub fn today(&self) -> NaiveDate {
        self.now.date()
    }
}

pub fn new_id(kind: Kind) -> String {
    format!(
        "{}-{}-{}",
        kind.id_prefix(),
        crate::jstime::now_millis(),
        crate::paths::random_base36(6)
    )
}

// ---------------- 草稿 ----------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WishDraft {
    pub title: String,
    pub note: String,
    pub project_id: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GoalDraft {
    pub title: String,
    pub note: String,
    pub criteria: Vec<String>,
    pub wish_id: Option<String>,
    pub project_id: Option<String>,
    pub target_date: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TaskDraft {
    pub title: String,
    pub note: String,
    pub goal_id: Option<String>,
    pub project_id: Option<String>,
    pub priority: Option<Priority>,
    pub due: Option<String>,
    pub planned_for: Option<String>,
    pub estimate_minutes: Option<u32>,
    pub source: String,
    pub steps: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EntryDraft {
    pub title: String,
    pub note: String,
    pub task_id: Option<String>,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub location: String,
    pub color: Option<String>,
    pub project_id: Option<String>,
    pub reminders: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RoutineDraft {
    pub title: String,
    pub note: String,
    pub rule: String,
    pub start_date: Option<String>,
    pub until: Option<String>,
    pub time: Option<String>,
    pub duration_minutes: Option<u32>,
    pub goal_id: Option<String>,
    pub project_id: Option<String>,
    pub reminders: Vec<u32>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectDraft {
    pub name: String,
    pub note: String,
    pub color: Option<String>,
}

/// 完成 / 取消任务后的提示信息：它还有哪些未来的计划时间块。
/// 默认保留，只有用户明确选择时才调用 [`Editor::cancel_future_entries`]。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FutureBlocks {
    pub entry_ids: Vec<String>,
}

// ---------------- 编辑 ----------------

pub struct Editor<'a> {
    pub data: &'a mut AgendaData,
    pub clock: Clock,
}

fn clean_title(title: &str, fallback: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        if fallback.is_empty() {
            bail!("标题不能为空");
        }
        return Ok(fallback.to_owned());
    }
    Ok(title.chars().take(200).collect())
}

fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim().trim_start_matches('#').to_owned();
        if !tag.is_empty() && !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

fn steps_from(titles: &[String]) -> Vec<Step> {
    titles
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .enumerate()
        .map(|(i, t)| Step {
            id: format!("s{}{}", crate::paths::random_base36(4), i),
            title: t.to_owned(),
            done: false,
        })
        .collect()
}

fn opt_due(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => time::normalize_due(v)
            .map(Some)
            .ok_or_else(|| anyhow!("无法识别的截止时间: {v}")),
    }
}

fn opt_date(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        None => Ok(None),
        Some(v) => time::normalize_date(v)
            .map(Some)
            .ok_or_else(|| anyhow!("无法识别的日期: {v}")),
    }
}

/// 解析一个时间段；全天时对齐到零点，结束至少晚于开始。
pub fn normalize_span(start: &str, end: &str, all_day: bool) -> Result<(String, String)> {
    let start_at = time::parse_stamp(start)
        .or_else(|| time::parse_date(start).map(|d| d.and_time(NaiveTime::MIN)))
        .ok_or_else(|| anyhow!("无法识别的开始时间: {start}"))?;
    let mut end_at = time::parse_stamp(end)
        .or_else(|| {
            time::parse_date(end).map(|d| {
                // 全天日程的日期结束是包含式的：「到 25 号」= 26 号零点。
                if all_day {
                    time::add_days(d, 1).and_time(NaiveTime::MIN)
                } else {
                    d.and_time(NaiveTime::MIN)
                }
            })
        })
        .unwrap_or(start_at + chrono::Duration::minutes(60));
    let start_at = if all_day {
        start_at.date().and_time(NaiveTime::MIN)
    } else {
        start_at
    };
    if all_day && end_at.time() != NaiveTime::MIN {
        end_at = time::add_days(end_at.date(), 1).and_time(NaiveTime::MIN);
    }
    if end_at <= start_at {
        if all_day {
            end_at = time::add_days(start_at.date(), 1).and_time(NaiveTime::MIN);
        } else {
            bail!("结束时间必须晚于开始时间");
        }
    }
    Ok((time::stamp(start_at), time::stamp(end_at)))
}

impl<'a> Editor<'a> {
    pub fn new(data: &'a mut AgendaData, clock: Clock) -> Self {
        Self { data, clock }
    }

    fn stamp(&self) -> String {
        self.clock.utc.clone()
    }

    // ---------------- 校验 ----------------

    fn check_project(&self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            if self.data.live_project(id).is_none() {
                bail!("项目不存在或已删除: {id}");
            }
        }
        Ok(())
    }

    fn check_goal(&self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            if self.data.live_goal(id).is_none() {
                bail!("目标不存在或已删除: {id}");
            }
        }
        Ok(())
    }

    fn check_wish(&self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            if self.data.live_wish(id).is_none() {
                bail!("愿望不存在或已删除: {id}");
            }
        }
        Ok(())
    }

    fn check_task(&self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            if self.data.live_task(id).is_none() {
                bail!("任务不存在或已删除: {id}");
            }
        }
        Ok(())
    }

    // ---------------- 创建 ----------------

    pub fn create_wish(&mut self, draft: WishDraft) -> Result<String> {
        self.check_project(draft.project_id.as_deref())?;
        let id = new_id(Kind::Wish);
        let now = self.stamp();
        self.data.wishes.push(Wish {
            id: id.clone(),
            title: clean_title(&draft.title, "")?,
            note: draft.note.trim().to_owned(),
            project_id: draft.project_id,
            tags: clean_tags(&draft.tags),
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    pub fn create_goal(&mut self, draft: GoalDraft) -> Result<String> {
        self.check_wish(draft.wish_id.as_deref())?;
        self.check_project(draft.project_id.as_deref())?;
        let id = new_id(Kind::Goal);
        let now = self.stamp();
        self.data.goals.push(Goal {
            id: id.clone(),
            title: clean_title(&draft.title, "")?,
            note: draft.note.trim().to_owned(),
            criteria: steps_from(&draft.criteria),
            wish_id: draft.wish_id,
            project_id: draft.project_id,
            target_date: opt_date(draft.target_date.as_deref())?,
            tags: clean_tags(&draft.tags),
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    pub fn create_task(&mut self, draft: TaskDraft) -> Result<String> {
        self.check_goal(draft.goal_id.as_deref())?;
        self.check_project(draft.project_id.as_deref())?;
        let id = new_id(Kind::Task);
        let now = self.stamp();
        self.data.tasks.push(Task {
            id: id.clone(),
            title: clean_title(&draft.title, "")?,
            note: draft.note.trim().to_owned(),
            goal_id: draft.goal_id,
            project_id: draft.project_id,
            priority: draft.priority.unwrap_or_default(),
            due: opt_due(draft.due.as_deref())?,
            planned_for: opt_date(draft.planned_for.as_deref())?,
            estimate_minutes: draft.estimate_minutes.filter(|m| *m > 0),
            source: draft.source.trim().to_owned(),
            steps: steps_from(&draft.steps),
            tags: clean_tags(&draft.tags),
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    pub fn create_entry(&mut self, draft: EntryDraft) -> Result<String> {
        self.check_task(draft.task_id.as_deref())?;
        self.check_project(draft.project_id.as_deref())?;
        let (start, end) = normalize_span(&draft.start, &draft.end, draft.all_day)?;
        let title = if draft.task_id.is_some() {
            String::new()
        } else {
            clean_title(&draft.title, "")?
        };
        let id = new_id(Kind::Entry);
        let now = self.stamp();
        self.data.entries.push(Entry {
            id: id.clone(),
            title,
            note: draft.note.trim().to_owned(),
            task_id: draft.task_id,
            start,
            end,
            all_day: draft.all_day,
            location: draft.location.trim().to_owned(),
            color: draft.color,
            project_id: draft.project_id,
            reminders: draft.reminders,
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    /// 给任务新增一个时间块。每次排期都只是新增，从不覆盖已有的块。
    pub fn schedule_task(&mut self, task_id: &str, start: &str, end: &str) -> Result<String> {
        self.check_task(Some(task_id))?;
        self.create_entry(EntryDraft {
            task_id: Some(task_id.to_owned()),
            start: start.to_owned(),
            end: end.to_owned(),
            ..Default::default()
        })
    }

    pub fn create_routine(&mut self, draft: RoutineDraft) -> Result<String> {
        self.check_goal(draft.goal_id.as_deref())?;
        self.check_project(draft.project_id.as_deref())?;
        let rule = recur::normalize(&draft.rule)
            .ok_or_else(|| anyhow!("无法识别的重复规则: {}", draft.rule))?;
        let time = match draft
            .time
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            Some(t) => Some(
                time::parse_hm(t)
                    .map(time::hm)
                    .ok_or_else(|| anyhow!("无法识别的时刻: {t}"))?,
            ),
            None => None,
        };
        let id = new_id(Kind::Routine);
        let now = self.stamp();
        self.data.routines.push(Routine {
            id: id.clone(),
            title: clean_title(&draft.title, "")?,
            note: draft.note.trim().to_owned(),
            rule,
            start_date: opt_date(draft.start_date.as_deref())?
                .unwrap_or_else(|| time::date_key(self.clock.today())),
            until: opt_date(draft.until.as_deref())?,
            time,
            duration_minutes: draft.duration_minutes.filter(|m| *m > 0).unwrap_or(30),
            goal_id: draft.goal_id,
            project_id: draft.project_id,
            reminders: draft.reminders,
            color: draft.color,
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    pub fn create_project(&mut self, draft: ProjectDraft) -> Result<String> {
        let id = new_id(Kind::Project);
        let now = self.stamp();
        self.data.projects.push(Project {
            id: id.clone(),
            name: clean_title(&draft.name, "")?,
            note: draft.note.trim().to_owned(),
            color: draft.color,
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    // ---------------- 更新 ----------------

    /// 按字段白名单修改一条记录。状态、删除、归档走各自的专用操作。
    ///
    /// 值为 `null` 表示清空。只动 patch 里出现的字段。
    pub fn update(&mut self, id: &str, patch: &Map<String, Value>) -> Result<()> {
        let kind = Kind::of_id(id).ok_or_else(|| anyhow!("无法识别的记录: {id}"))?;
        let allowed: &[&str] = match kind {
            Kind::Wish => &["title", "note", "project_id", "tags"],
            Kind::Goal => &[
                "title",
                "note",
                "criteria",
                "wish_id",
                "project_id",
                "target_date",
                "result_note",
                "tags",
            ],
            Kind::Task => &[
                "title",
                "note",
                "goal_id",
                "project_id",
                "priority",
                "due",
                "planned_for",
                "estimate_minutes",
                "source",
                "steps",
                "tags",
            ],
            Kind::Entry => &[
                "title",
                "note",
                "task_id",
                "start",
                "end",
                "all_day",
                "location",
                "color",
                "project_id",
                "reminders",
                "actual_minutes",
            ],
            Kind::Routine => &[
                "title",
                "note",
                "rule",
                "start_date",
                "until",
                "time",
                "duration_minutes",
                "goal_id",
                "project_id",
                "reminders",
                "color",
            ],
            Kind::Project => &["name", "note", "color"],
        };
        let mut clean = Map::new();
        for (key, value) in patch {
            if !allowed.contains(&key.as_str()) {
                bail!("{}不能直接修改字段 `{key}`", kind.label());
            }
            clean.insert(key.clone(), self.normalize_field(kind, key, value)?);
        }
        if clean.is_empty() {
            return Ok(());
        }
        // 关系目标必须存在。
        let target = |key: &str| clean.get(key).and_then(Value::as_str).map(str::to_owned);
        self.check_goal(target("goal_id").as_deref())?;
        self.check_wish(target("wish_id").as_deref())?;
        self.check_task(target("task_id").as_deref())?;
        self.check_project(target("project_id").as_deref())?;
        let stamp = self.stamp();
        macro_rules! merge {
            ($items:expr) => {{
                let item = $items
                    .iter_mut()
                    .find(|r| r.id == id)
                    .ok_or_else(|| anyhow!("{}不存在: {id}", kind.label()))?;
                let mut value = serde_json::to_value(&*item)?;
                let object = value.as_object_mut().expect("记录序列化为对象");
                for (key, v) in &clean {
                    if v.is_null() {
                        object.remove(key);
                    } else {
                        object.insert(key.clone(), v.clone());
                    }
                }
                let mut next = serde_json::from_value(value)?;
                std::mem::swap(item, &mut next);
                item.updated_at = stamp;
            }};
        }
        match kind {
            Kind::Wish => merge!(self.data.wishes),
            Kind::Goal => merge!(self.data.goals),
            Kind::Task => merge!(self.data.tasks),
            Kind::Entry => {
                merge!(self.data.entries);
                let entry = self.data.entry_mut(id).expect("刚更新过");
                let (start, end) = normalize_span(&entry.start, &entry.end, entry.all_day)?;
                entry.start = start;
                entry.end = end;
                if entry.task_id.is_some() {
                    entry.title.clear();
                } else if entry.title.trim().is_empty() && entry.routine_id.is_none() {
                    bail!("日程标题不能为空");
                }
            }
            Kind::Routine => merge!(self.data.routines),
            Kind::Project => merge!(self.data.projects),
        }
        Ok(())
    }

    fn normalize_field(&self, kind: Kind, key: &str, value: &Value) -> Result<Value> {
        if value.is_null() {
            if matches!(
                key,
                "title" | "name" | "start" | "end" | "rule" | "start_date"
            ) {
                bail!("字段 `{key}` 不能清空");
            }
            return Ok(Value::Null);
        }
        let text = value.as_str().map(str::trim);
        Ok(match key {
            "title" | "name" => Value::String(clean_title(text.unwrap_or_default(), "")?),
            "note" | "source" | "location" | "result_note" => {
                Value::String(text.unwrap_or_default().to_owned())
            }
            "due" => opt_due(text)?.map(Value::String).unwrap_or(Value::Null),
            "planned_for" | "target_date" | "until" => {
                opt_date(text)?.map(Value::String).unwrap_or(Value::Null)
            }
            "start_date" => {
                Value::String(opt_date(text)?.ok_or_else(|| anyhow!("开始日期不能为空"))?)
            }
            "start" | "end" => {
                let raw = text.unwrap_or_default();
                Value::String(
                    time::normalize_stamp(raw)
                        .or_else(|| time::normalize_date(raw).map(|d| format!("{d}T00:00")))
                        .ok_or_else(|| anyhow!("无法识别的时间: {raw}"))?,
                )
            }
            "time" => match text.filter(|t| !t.is_empty()) {
                Some(t) => Value::String(
                    time::parse_hm(t)
                        .map(time::hm)
                        .ok_or_else(|| anyhow!("无法识别的时刻: {t}"))?,
                ),
                None => Value::Null,
            },
            "rule" => Value::String(
                recur::normalize(text.unwrap_or_default())
                    .ok_or_else(|| anyhow!("无法识别的重复规则"))?,
            ),
            "priority" => Value::String(
                Priority::from_wire(text.unwrap_or_default())
                    .ok_or_else(|| anyhow!("优先级只能是 low/normal/high/urgent"))?
                    .wire()
                    .into(),
            ),
            "tags" => {
                let tags: Vec<String> = serde_json::from_value(value.clone())?;
                serde_json::to_value(clean_tags(&tags))?
            }
            "steps" | "criteria" => {
                // 允许传字符串数组（新建步骤）或完整步骤对象（保留勾选状态）。
                let items = value
                    .as_array()
                    .ok_or_else(|| anyhow!("`{key}` 需要数组"))?;
                let steps: Vec<Step> = items
                    .iter()
                    .filter_map(|item| match item {
                        Value::String(s) if !s.trim().is_empty() => Some(Step {
                            id: format!("s{}", crate::paths::random_base36(6)),
                            title: s.trim().to_owned(),
                            done: false,
                        }),
                        Value::Object(_) => serde_json::from_value::<Step>(item.clone())
                            .ok()
                            .filter(|s| !s.title.trim().is_empty())
                            .map(|mut s| {
                                if s.id.is_empty() {
                                    s.id = format!("s{}", crate::paths::random_base36(6));
                                }
                                s
                            }),
                        _ => None,
                    })
                    .collect();
                serde_json::to_value(steps)?
            }
            "goal_id" | "wish_id" | "task_id" | "project_id" => {
                match text.filter(|t| !t.is_empty()) {
                    Some(t) => Value::String(t.to_owned()),
                    None => Value::Null,
                }
            }
            "estimate_minutes" | "duration_minutes" | "actual_minutes" => {
                let minutes = value
                    .as_u64()
                    .ok_or_else(|| anyhow!("`{key}` 需要分钟数"))?;
                if minutes == 0 {
                    if key == "duration_minutes" {
                        bail!("时长必须大于 0");
                    }
                    Value::Null
                } else {
                    Value::from(minutes.min(24 * 60 * 30))
                }
            }
            "reminders" => {
                let minutes: Vec<u32> = serde_json::from_value(value.clone())?;
                serde_json::to_value(minutes)?
            }
            "all_day" => Value::Bool(value.as_bool().unwrap_or(false)),
            "color" => match text.filter(|t| !t.is_empty()) {
                Some(t) => Value::String(t.to_owned()),
                None => Value::Null,
            },
            _ => {
                let _ = kind;
                value.clone()
            }
        })
    }

    /// 只改关系字段：子项的标题、日期、状态一概不动。`parent` 为空表示解除关联。
    pub fn link(&mut self, child: &str, parent: Option<&str>) -> Result<()> {
        let child_kind = Kind::of_id(child).ok_or_else(|| anyhow!("无法识别的记录: {child}"))?;
        let parent_kind = parent.and_then(Kind::of_id);
        let key = match (child_kind, parent_kind, parent) {
            (_, Some(Kind::Project), _) => "project_id",
            (Kind::Goal, Some(Kind::Wish) | None, _) => "wish_id",
            (Kind::Task | Kind::Routine, Some(Kind::Goal) | None, _) => "goal_id",
            (Kind::Entry, Some(Kind::Task) | None, _) => "task_id",
            (c, Some(p), _) => bail!("{}不能挂到{}下面", c.label(), p.label()),
            (c, None, _) => bail!("{}没有可解除的上级", c.label()),
        };
        let mut patch = Map::new();
        patch.insert(
            key.into(),
            parent
                .map(|p| Value::String(p.to_owned()))
                .unwrap_or(Value::Null),
        );
        if child_kind == Kind::Entry && key == "task_id" && parent.is_none() {
            // 解除关联后成为独立日程：沿用任务标题作为自己的标题，免得变成空白。
            let title = self
                .data
                .entry(child)
                .map(|e| self.data.entry_title(e))
                .unwrap_or_default();
            patch.insert(
                "title".into(),
                Value::String(if title.is_empty() {
                    "日程".into()
                } else {
                    title
                }),
            );
            let entry = self
                .data
                .entry_mut(child)
                .ok_or_else(|| anyhow!("日程不存在"))?;
            entry.detached_task_title = None;
        }
        self.update(child, &patch)
    }

    // ---------------- 状态 ----------------

    pub fn set_wish_status(&mut self, id: &str, status: WishStatus) -> Result<()> {
        let stamp = self.stamp();
        let wish = self
            .data
            .wish_mut(id)
            .ok_or_else(|| anyhow!("愿望不存在: {id}"))?;
        wish.status = status;
        wish.closed_at = (status != WishStatus::Open).then(|| stamp.clone());
        wish.updated_at = stamp;
        Ok(())
    }

    /// 目标的结果由用户判定。任务完成与否不影响这里。
    pub fn set_goal_status(
        &mut self,
        id: &str,
        status: GoalStatus,
        result_note: Option<&str>,
    ) -> Result<()> {
        let stamp = self.stamp();
        let goal = self
            .data
            .goal_mut(id)
            .ok_or_else(|| anyhow!("目标不存在: {id}"))?;
        goal.status = status;
        goal.closed_at = (!status.is_open()).then(|| stamp.clone());
        if let Some(note) = result_note {
            goal.result_note = note.trim().to_owned();
        }
        goal.updated_at = stamp;
        Ok(())
    }

    /// 改任务状态。完成或取消时返回它未来仍在计划中的时间块，由调用方询问用户是否取消；
    /// 这里不会碰任何时间块。重新打开也不会恢复之前取消的块。
    pub fn set_task_status(&mut self, id: &str, status: TaskStatus) -> Result<FutureBlocks> {
        let stamp = self.stamp();
        let task = self
            .data
            .task_mut(id)
            .ok_or_else(|| anyhow!("任务不存在: {id}"))?;
        if task.deleted_at.is_some() {
            bail!("任务已删除，请先从回收站恢复");
        }
        task.status = status;
        task.completed_at = (status == TaskStatus::Done).then(|| stamp.clone());
        task.updated_at = stamp;
        Ok(if status.is_open() {
            FutureBlocks::default()
        } else {
            self.future_blocks(id)
        })
    }

    pub fn complete_task(&mut self, id: &str) -> Result<FutureBlocks> {
        self.set_task_status(id, TaskStatus::Done)
    }

    /// 任务还没开始的未来计划块。
    pub fn future_blocks(&self, task_id: &str) -> FutureBlocks {
        let now = self.clock.now;
        FutureBlocks {
            entry_ids: self
                .data
                .entries
                .iter()
                .filter(|e| {
                    e.task_id.as_deref() == Some(task_id) && e.status == EntryStatus::Planned
                })
                .filter(|e| time::parse_stamp(&e.start).is_some_and(|s| s > now))
                .map(|e| e.id.clone())
                .collect(),
        }
    }

    /// 批量取消时间块，但只取消「未来且仍在计划中」的；历史和已处理的原样保留。
    pub fn cancel_future_entries(&mut self, ids: &[String]) -> usize {
        let now = self.clock.now;
        let stamp = self.stamp();
        let mut count = 0;
        for entry in self.data.entries.iter_mut() {
            if !ids.contains(&entry.id) || entry.status != EntryStatus::Planned {
                continue;
            }
            if time::parse_stamp(&entry.start).is_none_or(|s| s <= now) {
                continue;
            }
            entry.status = EntryStatus::Cancelled;
            entry.updated_at = stamp.clone();
            count += 1;
        }
        count
    }

    /// 标记日程自身的执行状态。与关联任务互不影响。
    pub fn set_entry_status(
        &mut self,
        id: &str,
        status: EntryStatus,
        actual_minutes: Option<u32>,
    ) -> Result<()> {
        let stamp = self.stamp();
        let entry = self
            .data
            .entry_mut(id)
            .ok_or_else(|| anyhow!("日程不存在: {id}"))?;
        entry.status = status;
        entry.confirmed_at = (status != EntryStatus::Planned).then(|| stamp.clone());
        if status == EntryStatus::Done {
            if let Some(m) = actual_minutes {
                entry.actual_minutes = Some(m).filter(|m| *m > 0);
            }
        } else {
            entry.actual_minutes = None;
        }
        entry.updated_at = stamp;
        Ok(())
    }

    /// 显式组合操作：「已执行，并完成任务」。
    pub fn complete_entry_and_task(&mut self, id: &str) -> Result<FutureBlocks> {
        self.set_entry_status(id, EntryStatus::Done, None)?;
        let task_id = self
            .data
            .entry(id)
            .and_then(|e| e.task_id.clone())
            .ok_or_else(|| anyhow!("这个日程没有关联任务"))?;
        let mut future = self.complete_task(&task_id)?;
        future.entry_ids.retain(|e| e != id);
        Ok(future)
    }

    pub fn move_entry(&mut self, id: &str, start: NaiveDateTime, end: NaiveDateTime) -> Result<()> {
        let all_day = self
            .data
            .entry(id)
            .ok_or_else(|| anyhow!("日程不存在: {id}"))?
            .all_day;
        let (start, end) = normalize_span(&time::stamp(start), &time::stamp(end), all_day)?;
        let stamp = self.stamp();
        let entry = self.data.entry_mut(id).expect("上面查过");
        entry.start = start;
        entry.end = end;
        entry.updated_at = stamp;
        Ok(())
    }

    pub fn set_routine_status(&mut self, id: &str, status: RoutineStatus) -> Result<()> {
        let stamp = self.stamp();
        let routine = self
            .data
            .routine_mut(id)
            .ok_or_else(|| anyhow!("重复安排不存在: {id}"))?;
        routine.status = status;
        routine.updated_at = stamp;
        Ok(())
    }

    /// 把重复安排某一天的虚拟发生落成真实日程（已存在则直接返回）。
    pub fn materialize(&mut self, routine_id: &str, date: NaiveDate) -> Result<String> {
        let key = time::date_key(date);
        if let Some(existing) = self.data.entries.iter().find(|e| {
            e.routine_id.as_deref() == Some(routine_id)
                && e.occurrence.as_deref() == Some(key.as_str())
        }) {
            return Ok(existing.id.clone());
        }
        let routine = self
            .data
            .live_routine(routine_id)
            .cloned()
            .ok_or_else(|| anyhow!("重复安排不存在: {routine_id}"))?;
        let (start, end, all_day) = occurrence_span(&routine, date);
        let id = new_id(Kind::Entry);
        let now = self.stamp();
        self.data.entries.push(Entry {
            id: id.clone(),
            routine_id: Some(routine_id.to_owned()),
            occurrence: Some(key),
            start: time::stamp(start),
            end: time::stamp(end),
            all_day,
            color: routine.color.clone(),
            project_id: routine.project_id.clone(),
            reminders: routine.reminders.clone(),
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        });
        Ok(id)
    }

    /// 确认某个 ID 的执行情况：真实日程直接改；虚拟发生（`rtn-…@YYYY-MM-DD`）先落地再改。
    pub fn confirm(&mut self, key: &str, status: EntryStatus) -> Result<String> {
        let id = self.resolve_entry(key)?;
        self.set_entry_status(&id, status, None)?;
        Ok(id)
    }

    /// 把条目键解析成真实日程 ID，虚拟发生会被落地。
    pub fn resolve_entry(&mut self, key: &str) -> Result<String> {
        if let Some((routine, date)) = key.split_once('@') {
            let date =
                time::parse_date(date).ok_or_else(|| anyhow!("无法识别的发生日期: {key}"))?;
            return self.materialize(routine, date);
        }
        if self.data.entry(key).is_none() {
            bail!("日程不存在: {key}");
        }
        Ok(key.to_owned())
    }

    pub fn toggle_step(&mut self, id: &str, step_id: &str) -> Result<()> {
        let stamp = self.stamp();
        let steps = match Kind::of_id(id) {
            Some(Kind::Task) => {
                &mut self
                    .data
                    .task_mut(id)
                    .ok_or_else(|| anyhow!("任务不存在"))?
                    .steps
            }
            Some(Kind::Goal) => {
                &mut self
                    .data
                    .goal_mut(id)
                    .ok_or_else(|| anyhow!("目标不存在"))?
                    .criteria
            }
            _ => bail!("只有任务步骤和目标标准可以勾选"),
        };
        let step = steps
            .iter_mut()
            .find(|s| s.id == step_id)
            .ok_or_else(|| anyhow!("步骤不存在"))?;
        step.done = !step.done;
        match Kind::of_id(id) {
            Some(Kind::Task) => self.data.task_mut(id).expect("存在").updated_at = stamp,
            _ => self.data.goal_mut(id).expect("存在").updated_at = stamp,
        }
        Ok(())
    }

    pub fn add_step(&mut self, id: &str, title: &str) -> Result<()> {
        let title = clean_title(title, "")?;
        let stamp = self.stamp();
        let step = Step {
            id: format!("s{}", crate::paths::random_base36(6)),
            title,
            done: false,
        };
        match Kind::of_id(id) {
            Some(Kind::Task) => {
                let t = self
                    .data
                    .task_mut(id)
                    .ok_or_else(|| anyhow!("任务不存在"))?;
                t.steps.push(step);
                t.updated_at = stamp;
            }
            Some(Kind::Goal) => {
                let g = self
                    .data
                    .goal_mut(id)
                    .ok_or_else(|| anyhow!("目标不存在"))?;
                g.criteria.push(step);
                g.updated_at = stamp;
            }
            _ => bail!("只有任务和目标有步骤"),
        }
        Ok(())
    }

    pub fn remove_step(&mut self, id: &str, step_id: &str) -> Result<()> {
        let stamp = self.stamp();
        match Kind::of_id(id) {
            Some(Kind::Task) => {
                let t = self
                    .data
                    .task_mut(id)
                    .ok_or_else(|| anyhow!("任务不存在"))?;
                t.steps.retain(|s| s.id != step_id);
                t.updated_at = stamp;
            }
            Some(Kind::Goal) => {
                let g = self
                    .data
                    .goal_mut(id)
                    .ok_or_else(|| anyhow!("目标不存在"))?;
                g.criteria.retain(|s| s.id != step_id);
                g.updated_at = stamp;
            }
            _ => bail!("只有任务和目标有步骤"),
        }
        Ok(())
    }

    // ---------------- 归档 / 删除 ----------------

    /// 归档只改默认可见性：不级联、不取消日程、关系与历史全部保留。
    pub fn set_archived(&mut self, id: &str, archived: bool) -> Result<()> {
        let stamp = self.stamp();
        let value = archived.then(|| stamp.clone());
        macro_rules! set {
            ($item:expr) => {{
                let item = $item.ok_or_else(|| anyhow!("记录不存在: {id}"))?;
                item.archived_at = value;
                item.updated_at = stamp;
            }};
        }
        match Kind::of_id(id) {
            Some(Kind::Wish) => set!(self.data.wish_mut(id)),
            Some(Kind::Goal) => set!(self.data.goal_mut(id)),
            Some(Kind::Task) => set!(self.data.task_mut(id)),
            Some(Kind::Routine) => set!(self.data.routine_mut(id)),
            Some(Kind::Project) => set!(self.data.project_mut(id)),
            _ => bail!("日程不能归档，可以取消或删除"),
        }
        Ok(())
    }

    /// 删除。
    ///
    /// - 日程：直接移除，只影响它自己；
    /// - 任务：墓碑，历史块仍可追溯；`cancel_future` 为真时取消未来计划块（默认不取消）；
    /// - 目标 / 愿望 / 项目 / 重复安排：墓碑 + 解除子项关联，子项继续可访问。
    pub fn delete(&mut self, id: &str, cancel_future: bool) -> Result<()> {
        let stamp = self.stamp();
        match Kind::of_id(id) {
            Some(Kind::Entry) => {
                let before = self.data.entries.len();
                self.data.entries.retain(|e| e.id != id);
                if before == self.data.entries.len() {
                    bail!("日程不存在: {id}");
                }
            }
            Some(Kind::Task) => {
                let future = self.future_blocks(id);
                let task = self
                    .data
                    .task_mut(id)
                    .ok_or_else(|| anyhow!("任务不存在: {id}"))?;
                task.deleted_at = Some(stamp.clone());
                task.updated_at = stamp;
                if cancel_future {
                    self.cancel_future_entries(&future.entry_ids);
                }
            }
            Some(Kind::Goal) => {
                let mut detached = Vec::new();
                for task in self
                    .data
                    .tasks
                    .iter_mut()
                    .filter(|t| t.goal_id.as_deref() == Some(id))
                {
                    task.goal_id = None;
                    task.updated_at = stamp.clone();
                    detached.push(task.id.clone());
                }
                for routine in self
                    .data
                    .routines
                    .iter_mut()
                    .filter(|r| r.goal_id.as_deref() == Some(id))
                {
                    routine.goal_id = None;
                    routine.updated_at = stamp.clone();
                    detached.push(routine.id.clone());
                }
                let goal = self
                    .data
                    .goal_mut(id)
                    .ok_or_else(|| anyhow!("目标不存在: {id}"))?;
                goal.deleted_at = Some(stamp.clone());
                goal.detached = detached;
                goal.updated_at = stamp;
            }
            Some(Kind::Wish) => {
                let mut detached = Vec::new();
                for goal in self
                    .data
                    .goals
                    .iter_mut()
                    .filter(|g| g.wish_id.as_deref() == Some(id))
                {
                    goal.wish_id = None;
                    goal.updated_at = stamp.clone();
                    detached.push(goal.id.clone());
                }
                let wish = self
                    .data
                    .wish_mut(id)
                    .ok_or_else(|| anyhow!("愿望不存在: {id}"))?;
                wish.deleted_at = Some(stamp.clone());
                wish.detached = detached;
                wish.updated_at = stamp;
            }
            Some(Kind::Project) => {
                let pid = Some(id.to_owned());
                macro_rules! unlink {
                    ($items:expr) => {
                        for item in $items.iter_mut().filter(|r| r.project_id == pid) {
                            item.project_id = None;
                            item.updated_at = stamp.clone();
                        }
                    };
                }
                unlink!(self.data.wishes);
                unlink!(self.data.goals);
                unlink!(self.data.tasks);
                unlink!(self.data.entries);
                unlink!(self.data.routines);
                let project = self
                    .data
                    .project_mut(id)
                    .ok_or_else(|| anyhow!("项目不存在: {id}"))?;
                project.deleted_at = Some(stamp.clone());
                project.updated_at = stamp;
            }
            Some(Kind::Routine) => {
                let routine = self
                    .data
                    .routine_mut(id)
                    .ok_or_else(|| anyhow!("重复安排不存在: {id}"))?;
                routine.deleted_at = Some(stamp.clone());
                routine.updated_at = stamp;
            }
            None => bail!("无法识别的记录: {id}"),
        }
        Ok(())
    }

    /// 从回收站恢复。之前被解除关联的子项，若仍未挂到别处则重新挂回。
    pub fn restore(&mut self, id: &str) -> Result<()> {
        let stamp = self.stamp();
        match Kind::of_id(id) {
            Some(Kind::Task) => {
                let task = self
                    .data
                    .task_mut(id)
                    .ok_or_else(|| anyhow!("任务不存在"))?;
                task.deleted_at = None;
                task.updated_at = stamp;
            }
            Some(Kind::Goal) => {
                let goal = self
                    .data
                    .goal_mut(id)
                    .ok_or_else(|| anyhow!("目标不存在"))?;
                goal.deleted_at = None;
                goal.updated_at = stamp.clone();
                let detached = std::mem::take(&mut goal.detached);
                for child in detached {
                    if let Some(t) = self.data.task_mut(&child).filter(|t| t.goal_id.is_none()) {
                        t.goal_id = Some(id.to_owned());
                        t.updated_at = stamp.clone();
                    }
                    if let Some(r) = self
                        .data
                        .routine_mut(&child)
                        .filter(|r| r.goal_id.is_none())
                    {
                        r.goal_id = Some(id.to_owned());
                        r.updated_at = stamp.clone();
                    }
                }
            }
            Some(Kind::Wish) => {
                let wish = self
                    .data
                    .wish_mut(id)
                    .ok_or_else(|| anyhow!("愿望不存在"))?;
                wish.deleted_at = None;
                wish.updated_at = stamp.clone();
                let detached = std::mem::take(&mut wish.detached);
                for child in detached {
                    if let Some(g) = self.data.goal_mut(&child).filter(|g| g.wish_id.is_none()) {
                        g.wish_id = Some(id.to_owned());
                        g.updated_at = stamp.clone();
                    }
                }
            }
            Some(Kind::Project) => {
                let p = self
                    .data
                    .project_mut(id)
                    .ok_or_else(|| anyhow!("项目不存在"))?;
                p.deleted_at = None;
                p.updated_at = stamp;
            }
            Some(Kind::Routine) => {
                let r = self
                    .data
                    .routine_mut(id)
                    .ok_or_else(|| anyhow!("重复安排不存在"))?;
                r.deleted_at = None;
                r.updated_at = stamp;
            }
            _ => bail!("这条记录不在回收站里"),
        }
        Ok(())
    }

    /// 从回收站彻底清除。清除任务时，保留下来的时间块记下原任务标题，不留悬空引用。
    pub fn purge(&mut self, id: &str) -> Result<()> {
        let in_trash = match Kind::of_id(id) {
            Some(Kind::Task) => self.data.task(id).is_some_and(|t| t.deleted_at.is_some()),
            Some(Kind::Goal) => self.data.goal(id).is_some_and(|g| g.deleted_at.is_some()),
            Some(Kind::Wish) => self.data.wish(id).is_some_and(|w| w.deleted_at.is_some()),
            Some(Kind::Project) => self
                .data
                .project(id)
                .is_some_and(|p| p.deleted_at.is_some()),
            Some(Kind::Routine) => self
                .data
                .routine(id)
                .is_some_and(|r| r.deleted_at.is_some()),
            _ => false,
        };
        if !in_trash {
            bail!("只能清除回收站里的记录");
        }
        let stamp = self.stamp();
        match Kind::of_id(id) {
            Some(Kind::Task) => {
                let title = self
                    .data
                    .task(id)
                    .map(|t| t.title.clone())
                    .unwrap_or_default();
                for entry in self
                    .data
                    .entries
                    .iter_mut()
                    .filter(|e| e.task_id.as_deref() == Some(id))
                {
                    entry.task_id = None;
                    entry.detached_task_title = Some(title.clone());
                    entry.updated_at = stamp.clone();
                }
                self.data.tasks.retain(|t| t.id != id);
            }
            Some(Kind::Goal) => self.data.goals.retain(|g| g.id != id),
            Some(Kind::Wish) => self.data.wishes.retain(|w| w.id != id),
            Some(Kind::Project) => self.data.projects.retain(|p| p.id != id),
            Some(Kind::Routine) => {
                // 已落地的发生保留为独立历史日程。
                let title = self
                    .data
                    .routine(id)
                    .map(|r| r.title.clone())
                    .unwrap_or_default();
                for entry in self
                    .data
                    .entries
                    .iter_mut()
                    .filter(|e| e.routine_id.as_deref() == Some(id))
                {
                    entry.routine_id = None;
                    if entry.title.is_empty() && entry.task_id.is_none() {
                        entry.title = title.clone();
                    }
                    entry.updated_at = stamp.clone();
                }
                self.data.routines.retain(|r| r.id != id);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn update_settings(&mut self, patch: &Map<String, Value>) -> Result<()> {
        let mut value = serde_json::to_value(&self.data.settings)?;
        let object = value.as_object_mut().expect("设置是对象");
        for (key, v) in patch {
            if !object.contains_key(key) {
                bail!("未知设置项 `{key}`");
            }
            object.insert(key.clone(), v.clone());
        }
        let settings: AgendaSettings = serde_json::from_value(value)?;
        if settings.day_start_hour >= settings.day_end_hour || settings.day_end_hour > 24 {
            bail!("时间轴范围无效");
        }
        self.data.settings = settings;
        Ok(())
    }
}

/// 重复安排在某天的时间段：有固定时刻用时刻，否则是全天。
pub fn occurrence_span(routine: &Routine, date: NaiveDate) -> (NaiveDateTime, NaiveDateTime, bool) {
    match routine.time.as_deref().and_then(time::parse_hm) {
        Some(t) => {
            let start = date.and_time(t);
            (
                start,
                start + chrono::Duration::minutes(routine.duration_minutes.max(5) as i64),
                false,
            )
        }
        None => (
            date.and_time(NaiveTime::MIN),
            time::add_days(date, 1).and_time(NaiveTime::MIN),
            true,
        ),
    }
}
