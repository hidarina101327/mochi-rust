//! 批量操作：AI 工具、控制台、桌面卡片共用的一套 JSON 操作语言。
//!
//! ```json
//! [
//!   {"op": "create", "kind": "goal", "ref": "g", "title": "六级 550"},
//!   {"op": "create", "kind": "task", "ref": "t", "title": "刷真题", "goalId": "$g", "due": "2026-10-01"},
//!   {"op": "schedule", "taskId": "$t", "start": "2026-09-26T19:00", "end": "2026-09-26T20:30"}
//! ]
//! ```
//!
//! 字段名接受 camelCase 或 snake_case。任何字符串值 `"$ref"` 会替换成同一批里
//! 先前创建的记录 id。整批在一个 [`Editor`] 上顺序执行，任何一步失败整批作废。

use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use super::model::*;
use super::ops::*;
use super::time;

/// 支持的操作名及一句话说明（写进工具描述）。
pub const OPS: &[(&str, &str)] = &[
    ("create", "kind=wish|goal|task|entry|routine|project，其余字段同记录字段；可带 ref"),
    ("update", "id + fields：按字段修改，null 清空；状态/删除/归档用专门操作"),
    ("link", "id + parent：改上级（愿望←目标←任务，任务←时间块，任意←项目）；parent=null 解除"),
    ("status", "id + status；目标可带 resultNote，时间块可带 actualMinutes"),
    ("schedule", "taskId + start + end：为任务新增一个时间块，可带 ref"),
    ("move", "id + start + end：移动日程 / 时间块"),
    ("confirm", "key + status(done|skipped)：确认过去的时间块或重复安排的某一天（key 形如 rtn-…@2026-09-25）"),
    ("complete_block", "id：时间块已执行，同时完成任务"),
    ("cancel_blocks", "ids：取消这些未来的计划时间块（历史块不受影响）"),
    ("archive", "id + archived(默认 true)：只改默认可见性"),
    ("delete", "id，任务可带 cancelFutureBlocks：删除（事项进回收站，日程直接删除）"),
    ("restore", "id：从回收站恢复"),
    ("step", "id + action(add|toggle|remove) + title/stepId：任务步骤或目标达成标准"),
    ("settings", "fields：修改日程待办设置"),
];

/// 一步的执行结果。
#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub op: String,
    pub id: Option<String>,
    /// 完成 / 删除任务后仍在未来的计划块，供调用方提示「保留还是取消」。
    pub future_blocks: Vec<String>,
}

impl StepResult {
    pub fn to_json(&self) -> Value {
        let mut value = json!({ "op": self.op });
        if let Some(id) = &self.id {
            value["id"] = json!(id);
        }
        if !self.future_blocks.is_empty() {
            value["futureBlocks"] = json!(self.future_blocks);
        }
        value
    }
}

/// `dueAt` → `due_at`。
pub fn snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// 字段别名：让 AI 用惯的旧名字也能落到新模型上。
fn canonical_field(key: &str) -> String {
    let key = snake_case(key);
    match key.as_str() {
        "description" | "notes" => "note".into(),
        "due_at" | "deadline" => "due".into(),
        "start_at" => "start".into(),
        "end_at" => "end".into(),
        "duration" | "estimate" => "estimate_minutes".into(),
        "recurrence_rule" | "recurrence" => "rule".into(),
        "planned_date" | "plan_date" => "planned_for".into(),
        _ => key,
    }
}

fn normalize_object(value: &Value, refs: &HashMap<String, String>) -> Result<Map<String, Value>> {
    let object = value.as_object().ok_or_else(|| anyhow!("需要 JSON 对象"))?;
    let mut out = Map::new();
    for (key, value) in object {
        out.insert(canonical_field(key), resolve_refs(value, refs)?);
    }
    Ok(out)
}

fn resolve_refs(value: &Value, refs: &HashMap<String, String>) -> Result<Value> {
    Ok(match value {
        Value::String(s) if s.starts_with('$') && s.len() > 1 => {
            let name = &s[1..];
            Value::String(
                refs.get(name)
                    .cloned()
                    .ok_or_else(|| anyhow!("引用 `{s}` 未在此前的 create 中定义"))?,
            )
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_refs(v, refs))
                .collect::<Result<_>>()?,
        ),
        other => other.clone(),
    })
}

fn str_field<'v>(op: &'v Map<String, Value>, key: &str) -> Result<&'v str> {
    op.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("缺少 `{key}`"))
}

fn opt_str<'v>(op: &'v Map<String, Value>, key: &str) -> Option<&'v str> {
    op.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn draft<T: serde::de::DeserializeOwned>(op: &Map<String, Value>) -> Result<T> {
    let mut fields = op.clone();
    for key in ["op", "kind", "ref", "type"] {
        fields.remove(key);
    }
    // 优先级容忍中文与旧值。
    if let Some(p) = fields.get("priority").and_then(Value::as_str) {
        let wire = match p.trim() {
            "medium" | "中" => Priority::Normal,
            "高优先级" => Priority::High,
            other => Priority::from_wire(other).unwrap_or_default(),
        }
        .wire();
        fields.insert("priority".into(), json!(wire));
    }
    serde_json::from_value(Value::Object(fields)).context("字段格式不正确")
}

/// 在同一个 [`Editor`] 上执行整批操作。
pub fn run(editor: &mut Editor<'_>, operations: &[Value]) -> Result<Vec<StepResult>> {
    let mut refs: HashMap<String, String> = HashMap::new();
    let mut results = Vec::new();
    for (index, raw) in operations.iter().enumerate() {
        let op = normalize_object(raw, &refs).with_context(|| format!("第 {} 步", index + 1))?;
        let result = run_one(editor, &op).with_context(|| {
            let name = op.get("op").and_then(Value::as_str).unwrap_or("?");
            format!("第 {} 步（{name}）失败", index + 1)
        })?;
        if let (Some(name), Some(id)) = (opt_str(&op, "ref"), &result.id) {
            refs.insert(name.trim_start_matches('$').to_owned(), id.clone());
        }
        results.push(result);
    }
    Ok(results)
}

fn run_one(ed: &mut Editor<'_>, op: &Map<String, Value>) -> Result<StepResult> {
    let name = str_field(op, "op")?.to_owned();
    let mut result = StepResult {
        op: name.clone(),
        id: None,
        future_blocks: Vec::new(),
    };
    match name.as_str() {
        "create" => {
            let kind = opt_str(op, "kind")
                .or_else(|| opt_str(op, "type"))
                .and_then(Kind::from_wire)
                .ok_or_else(|| anyhow!("create 需要 kind=wish|goal|task|entry|routine|project"))?;
            let id = match kind {
                Kind::Wish => ed.create_wish(draft(op)?)?,
                Kind::Goal => ed.create_goal(draft(op)?)?,
                Kind::Task => ed.create_task(draft(op)?)?,
                Kind::Entry => ed.create_entry(draft(op)?)?,
                Kind::Routine => ed.create_routine(draft(op)?)?,
                Kind::Project => ed.create_project(draft(op)?)?,
            };
            result.id = Some(id);
        }
        "update" => {
            let id = str_field(op, "id")?;
            let fields = op
                .get("fields")
                .or_else(|| op.get("set"))
                .or_else(|| op.get("updates"))
                .ok_or_else(|| anyhow!("update 需要 fields"))?;
            let fields = normalize_object(fields, &HashMap::new())?;
            ed.update(id, &fields)?;
            result.id = Some(id.to_owned());
        }
        "link" => {
            let id = str_field(op, "id")?;
            ed.link(
                id,
                opt_str(op, "parent").or_else(|| opt_str(op, "parent_id")),
            )?;
            result.id = Some(id.to_owned());
        }
        "status" => {
            let id = str_field(op, "id")?;
            let status = str_field(op, "status")?;
            let kind = Kind::of_id(id).ok_or_else(|| anyhow!("无法识别的记录: {id}"))?;
            let bad = || anyhow!("{}没有状态 `{status}`", kind.label());
            match kind {
                Kind::Wish => {
                    ed.set_wish_status(id, WishStatus::from_wire(status).ok_or_else(bad)?)?
                }
                Kind::Goal => ed.set_goal_status(
                    id,
                    GoalStatus::from_wire(status).ok_or_else(bad)?,
                    opt_str(op, "result_note"),
                )?,
                Kind::Task => {
                    let blocks =
                        ed.set_task_status(id, TaskStatus::from_wire(status).ok_or_else(bad)?)?;
                    if op.get("cancel_future_blocks").and_then(Value::as_bool) == Some(true) {
                        ed.cancel_future_entries(&blocks.entry_ids);
                    } else {
                        result.future_blocks = blocks.entry_ids;
                    }
                }
                Kind::Entry => ed.set_entry_status(
                    id,
                    EntryStatus::from_wire(status).ok_or_else(bad)?,
                    op.get("actual_minutes")
                        .and_then(Value::as_u64)
                        .map(|m| m as u32),
                )?,
                Kind::Routine => {
                    ed.set_routine_status(id, RoutineStatus::from_wire(status).ok_or_else(bad)?)?
                }
                Kind::Project => {
                    let status = ProjectStatus::from_wire(status).ok_or_else(bad)?;
                    let stamp = ed.clock.utc.clone();
                    let project = ed
                        .data
                        .project_mut(id)
                        .ok_or_else(|| anyhow!("项目不存在: {id}"))?;
                    project.status = status;
                    project.updated_at = stamp;
                }
            }
            result.id = Some(id.to_owned());
        }
        "schedule" => {
            let task = str_field(op, "task_id").or_else(|_| str_field(op, "id"))?;
            let start = str_field(op, "start")?;
            let end = opt_str(op, "end").unwrap_or_default();
            result.id = Some(ed.schedule_task(task, start, end)?);
        }
        "move" => {
            let key = str_field(op, "id").or_else(|_| str_field(op, "key"))?;
            let id = ed.resolve_entry(key)?;
            let start = time::parse_stamp(str_field(op, "start")?)
                .ok_or_else(|| anyhow!("无法识别的开始时间"))?;
            let end = match opt_str(op, "end") {
                Some(end) => time::parse_stamp(end).ok_or_else(|| anyhow!("无法识别的结束时间"))?,
                None => {
                    let entry = ed.data.entry(&id).ok_or_else(|| anyhow!("日程不存在"))?;
                    let minutes = super::query::entry_minutes(entry).max(15);
                    start + chrono::Duration::minutes(minutes)
                }
            };
            ed.move_entry(&id, start, end)?;
            result.id = Some(id);
        }
        "confirm" => {
            let key = str_field(op, "key").or_else(|_| str_field(op, "id"))?;
            let status = EntryStatus::from_wire(opt_str(op, "status").unwrap_or("done"))
                .ok_or_else(|| anyhow!("status 只能是 done/skipped/cancelled"))?;
            let id = ed.confirm(key, status)?;
            if let Some(minutes) = op.get("actual_minutes").and_then(Value::as_u64) {
                ed.set_entry_status(&id, status, Some(minutes as u32))?;
            }
            result.id = Some(id);
        }
        "complete_block" => {
            let key = str_field(op, "id").or_else(|_| str_field(op, "key"))?;
            let id = ed.resolve_entry(key)?;
            let blocks = ed.complete_entry_and_task(&id)?;
            if op.get("cancel_future_blocks").and_then(Value::as_bool) == Some(true) {
                ed.cancel_future_entries(&blocks.entry_ids);
            } else {
                result.future_blocks = blocks.entry_ids;
            }
            result.id = Some(id);
        }
        "cancel_blocks" => {
            let ids: Vec<String> =
                serde_json::from_value(op.get("ids").cloned().unwrap_or_default())
                    .context("cancel_blocks 需要 ids 数组")?;
            ed.cancel_future_entries(&ids);
        }
        "archive" => {
            let id = str_field(op, "id")?;
            ed.set_archived(
                id,
                op.get("archived").and_then(Value::as_bool).unwrap_or(true),
            )?;
            result.id = Some(id.to_owned());
        }
        "delete" => {
            let id = str_field(op, "id")?;
            let cancel = op
                .get("cancel_future_blocks")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            ed.delete(id, cancel)?;
            result.id = Some(id.to_owned());
        }
        "restore" => {
            let id = str_field(op, "id")?;
            ed.restore(id)?;
            result.id = Some(id.to_owned());
        }
        "step" => {
            let id = str_field(op, "id")?;
            match opt_str(op, "action").unwrap_or("add") {
                "add" => ed.add_step(id, str_field(op, "title")?)?,
                "toggle" => ed.toggle_step(id, str_field(op, "step_id")?)?,
                "remove" => ed.remove_step(id, str_field(op, "step_id")?)?,
                other => bail!("step 的 action 只能是 add/toggle/remove，而不是 {other}"),
            }
            result.id = Some(id.to_owned());
        }
        "settings" => {
            let fields = op
                .get("fields")
                .ok_or_else(|| anyhow!("settings 需要 fields"))?;
            ed.update_settings(&normalize_object(fields, &HashMap::new())?)?;
        }
        other => {
            let names: Vec<&str> = OPS.iter().map(|(n, _)| *n).collect();
            bail!("未知操作 `{other}`，可用：{}", names.join("/"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_chain_create_link_and_schedule() {
        let mut data = AgendaData::default();
        let now = time::parse_stamp("2026-09-25T08:00").unwrap();
        let mut ed = Editor::new(&mut data, Clock::fixed(now));
        let ops: Vec<Value> = serde_json::from_value(json!([
            {"op": "create", "kind": "wish", "ref": "w", "title": "身体更好"},
            {"op": "create", "kind": "goal", "ref": "g", "title": "跑完半马", "wishId": "$w", "criteria": ["21km 不停"]},
            {"op": "create", "kind": "task", "ref": "t", "title": "周末长距离", "goalId": "$g", "priority": "高", "estimateMinutes": 90},
            {"op": "schedule", "taskId": "$t", "start": "2026-09-27T07:00", "end": "2026-09-27T08:30"},
            {"op": "status", "id": "$t", "status": "done"},
        ]))
        .unwrap();
        let results = run(&mut ed, &ops).unwrap();
        assert_eq!(results.len(), 5);
        let task = data.tasks[0].clone();
        assert_eq!(task.priority, Priority::High);
        assert_eq!(task.goal_id.as_deref(), Some(data.goals[0].id.as_str()));
        assert_eq!(results[4].future_blocks, vec![data.entries[0].id.clone()]);
        assert_eq!(data.entries[0].status, EntryStatus::Planned);
    }

    #[test]
    fn a_failing_step_names_itself() {
        let mut data = AgendaData::default();
        let mut ed = Editor::new(&mut data, Clock::system());
        let ops = vec![json!({"op": "link", "id": "$nope", "parent": null})];
        let error = format!("{:#}", run(&mut ed, &ops).unwrap_err());
        assert!(error.contains("第 1 步"), "{error}");
    }
}
