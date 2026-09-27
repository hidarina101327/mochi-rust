//! 记录级变更集：撤销 / 重做、AI 提案审阅、操作日志都基于它。
//!
//! 一条变更记下某条记录修改前后的完整 JSON。应用时可以先核对「修改前」是否仍与
//! 当前数据一致，不一致就是冲突（例如 AI 提案生成后用户又改了同一条记录）。

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::*;

pub const SETTINGS: &str = "settings";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordChange {
    /// `wish` / `goal` / `task` / `entry` / `routine` / `project` / `settings`
    pub collection: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

impl RecordChange {
    pub fn kind(&self) -> Option<Kind> {
        Kind::from_wire(&self.collection)
    }

    pub fn is_create(&self) -> bool {
        self.before.is_none() && self.after.is_some()
    }

    pub fn is_remove(&self) -> bool {
        self.before.is_some() && self.after.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    pub changes: Vec<RecordChange>,
}

fn records<T: Serialize>(items: &[T], id: impl Fn(&T) -> &str) -> Vec<(String, Value)> {
    items
        .iter()
        .map(|item| {
            (
                id(item).to_owned(),
                serde_json::to_value(item).unwrap_or(Value::Null),
            )
        })
        .collect()
}

fn collections(data: &AgendaData) -> Vec<(&'static str, Vec<(String, Value)>)> {
    vec![
        ("wish", records(&data.wishes, |r| &r.id)),
        ("goal", records(&data.goals, |r| &r.id)),
        ("task", records(&data.tasks, |r| &r.id)),
        ("entry", records(&data.entries, |r| &r.id)),
        ("routine", records(&data.routines, |r| &r.id)),
        ("project", records(&data.projects, |r| &r.id)),
        (
            SETTINGS,
            vec![(
                SETTINGS.to_owned(),
                serde_json::to_value(&data.settings).unwrap_or(Value::Null),
            )],
        ),
    ]
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// 比较两个快照得出变更。
    pub fn diff(before: &AgendaData, after: &AgendaData) -> Self {
        let mut changes = Vec::new();
        for ((name, old), (_, new)) in collections(before).into_iter().zip(collections(after)) {
            for (id, value) in &old {
                match new.iter().find(|(nid, _)| nid == id) {
                    Some((_, nv)) if nv == value => {}
                    Some((_, nv)) => changes.push(RecordChange {
                        collection: name.into(),
                        id: id.clone(),
                        before: Some(value.clone()),
                        after: Some(nv.clone()),
                    }),
                    None => changes.push(RecordChange {
                        collection: name.into(),
                        id: id.clone(),
                        before: Some(value.clone()),
                        after: None,
                    }),
                }
            }
            for (id, value) in &new {
                if !old.iter().any(|(oid, _)| oid == id) {
                    changes.push(RecordChange {
                        collection: name.into(),
                        id: id.clone(),
                        before: None,
                        after: Some(value.clone()),
                    });
                }
            }
        }
        Self { changes }
    }

    pub fn inverted(&self) -> Self {
        Self {
            changes: self
                .changes
                .iter()
                .rev()
                .map(|c| RecordChange {
                    collection: c.collection.clone(),
                    id: c.id.clone(),
                    before: c.after.clone(),
                    after: c.before.clone(),
                })
                .collect(),
        }
    }

    /// 应用到数据上。`strict` 时先核对每条记录当前值等于 `before`，任何一条不符就整体放弃。
    pub fn apply(&self, data: &mut AgendaData, strict: bool) -> Result<()> {
        if strict {
            let current = collections(data);
            for change in &self.changes {
                let now = current
                    .iter()
                    .find(|(name, _)| *name == change.collection)
                    .and_then(|(_, items)| items.iter().find(|(id, _)| *id == change.id))
                    .map(|(_, v)| v);
                if now != change.before.as_ref() {
                    let label = data
                        .title_of(&change.id)
                        .unwrap_or_else(|| change.id.clone());
                    bail!("「{label}」在此期间已被修改，无法应用这组变更");
                }
            }
        }
        let mut next = data.clone();
        for change in &self.changes {
            apply_one(&mut next, change)?;
        }
        *data = next;
        Ok(())
    }

    /// 人类可读的变更说明，一条变更一行。
    pub fn describe(&self, before: &AgendaData, after: &AgendaData) -> Vec<String> {
        self.changes
            .iter()
            .map(|c| describe_one(c, before, after))
            .collect()
    }

    /// 合并连续的变更（例如拖动中的多次移动），保留最早的 before 和最新的 after。
    pub fn merge(mut self, later: ChangeSet) -> Self {
        for change in later.changes {
            if let Some(existing) = self
                .changes
                .iter_mut()
                .find(|c| c.collection == change.collection && c.id == change.id)
            {
                existing.after = change.after;
            } else {
                self.changes.push(change);
            }
        }
        self.changes.retain(|c| c.before != c.after);
        self
    }

    pub fn touches(&self, id: &str) -> bool {
        self.changes.iter().any(|c| c.id == id)
    }
}

fn put<T: serde::de::DeserializeOwned>(
    items: &mut Vec<T>,
    id: &str,
    value: Option<&Value>,
    id_of: impl Fn(&T) -> &str,
) -> Result<()> {
    let position = items.iter().position(|item| id_of(item) == id);
    match (value, position) {
        (None, Some(index)) => {
            items.remove(index);
        }
        (None, None) => {}
        (Some(value), Some(index)) => items[index] = serde_json::from_value(value.clone())?,
        (Some(value), None) => items.push(serde_json::from_value(value.clone())?),
    }
    Ok(())
}

fn apply_one(data: &mut AgendaData, change: &RecordChange) -> Result<()> {
    let after = change.after.as_ref();
    match change.collection.as_str() {
        "wish" => put(&mut data.wishes, &change.id, after, |r| &r.id),
        "goal" => put(&mut data.goals, &change.id, after, |r| &r.id),
        "task" => put(&mut data.tasks, &change.id, after, |r| &r.id),
        "entry" => put(&mut data.entries, &change.id, after, |r| &r.id),
        "routine" => put(&mut data.routines, &change.id, after, |r| &r.id),
        "project" => put(&mut data.projects, &change.id, after, |r| &r.id),
        SETTINGS => {
            if let Some(value) = after {
                data.settings = serde_json::from_value(value.clone())?;
            }
            Ok(())
        }
        other => bail!("未知的数据集合: {other}"),
    }
}

fn title_in(value: &Value) -> String {
    ["title", "name"]
        .iter()
        .find_map(|k| value.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned()
}

fn field_label(key: &str) -> &str {
    match key {
        "title" | "name" => "标题",
        "note" => "备注",
        "status" => "状态",
        "priority" => "优先级",
        "due" => "截止",
        "planned_for" => "计划日",
        "estimate_minutes" => "预计时长",
        "start" | "end" => "时间",
        "goal_id" => "所属目标",
        "wish_id" => "所属愿望",
        "task_id" => "关联任务",
        "project_id" => "项目",
        "archived_at" => "归档",
        "deleted_at" => "删除",
        "steps" => "步骤",
        "criteria" => "达成标准",
        "rule" => "重复规则",
        "time" => "时刻",
        "tags" => "标签",
        "result_note" => "结果复盘",
        _ => key,
    }
}

fn describe_one(change: &RecordChange, before: &AgendaData, after: &AgendaData) -> String {
    if change.collection == SETTINGS {
        return "修改日程待办设置".into();
    }
    let kind_label = change.kind().map(Kind::label).unwrap_or("记录");
    let title = |data: &AgendaData, value: Option<&Value>| {
        data.title_of(&change.id)
            .filter(|t| !t.is_empty())
            .or_else(|| value.map(title_in))
            .unwrap_or_default()
    };
    match (&change.before, &change.after) {
        (None, Some(v)) => {
            let mut line = format!("新建{kind_label}「{}」", title(after, Some(v)));
            if let (Some(start), Some(end)) = (
                v.get("start").and_then(Value::as_str),
                v.get("end").and_then(Value::as_str),
            ) {
                line.push_str(&format!(
                    " {}–{}",
                    start.replace('T', " "),
                    &end[end.len().saturating_sub(5)..]
                ));
            }
            line
        }
        (Some(v), None) => format!("移除{kind_label}「{}」", title(before, Some(v))),
        (Some(old), Some(new)) => {
            let name = title(after, Some(new));
            if new.get("deleted_at").is_some() && old.get("deleted_at").is_none() {
                return format!("删除{kind_label}「{name}」（可在回收站恢复）");
            }
            if new.get("status") != old.get("status") {
                let status = new.get("status").and_then(Value::as_str).unwrap_or("");
                let label = status_label(change.kind(), status);
                return format!("{kind_label}「{name}」→ {label}");
            }
            let mut fields: Vec<&str> = Vec::new();
            if let (Some(a), Some(b)) = (old.as_object(), new.as_object()) {
                for key in a.keys().chain(b.keys()) {
                    if matches!(key.as_str(), "updated_at" | "created_at") {
                        continue;
                    }
                    let label = field_label(key);
                    if a.get(key) != b.get(key) && !fields.contains(&label) {
                        fields.push(label);
                    }
                }
            }
            if fields.is_empty() {
                format!("更新{kind_label}「{name}」")
            } else {
                format!("修改{kind_label}「{name}」的{}", fields.join("、"))
            }
        }
        (None, None) => String::new(),
    }
}

pub fn status_label(kind: Option<Kind>, status: &str) -> &'static str {
    match kind {
        Some(Kind::Wish) => WishStatus::from_wire(status).map(WishStatus::label),
        Some(Kind::Goal) => GoalStatus::from_wire(status).map(GoalStatus::label),
        Some(Kind::Task) => TaskStatus::from_wire(status).map(TaskStatus::label),
        Some(Kind::Entry) => EntryStatus::from_wire(status).map(EntryStatus::label),
        Some(Kind::Routine) => RoutineStatus::from_wire(status).map(RoutineStatus::label),
        Some(Kind::Project) => ProjectStatus::from_wire(status).map(ProjectStatus::label),
        None => None,
    }
    .unwrap_or("未知状态")
}
