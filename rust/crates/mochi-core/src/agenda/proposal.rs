//! AI 提案：一组还没写入的变更，附带给人看的摘要。
//!
//! 线上格式沿用 `pendingScheduleDiff`（会话消息、审批卡片都认这个键）：
//!
//! ```json
//! {"id": "...", "title": "日程待办变更", "summary": "...", "status": "pending",
//!  "workspaceRoot": "...", "operations": [
//!    {"type": "create", "collection": "task", "id": "task-…", "after": {...}, "summary": "新建任务「…」"}
//!  ]}
//! ```
//!
//! 批准时以严格模式应用：记录在提案生成后被改过就拒绝，绝不覆盖用户的新修改。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::change::{ChangeSet, RecordChange};
use super::model::AgendaData;

pub const MESSAGE_KEY: &str = "pendingScheduleDiff";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalOp {
    /// `create` / `update` / `delete`
    #[serde(rename = "type")]
    pub op: String,
    pub collection: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    #[serde(default)]
    pub summary: String,
}

impl ProposalOp {
    pub fn is_delete(&self) -> bool {
        self.op == "delete"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    /// `pending` / `applied` / `rejected`
    #[serde(default = "pending")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub operations: Vec<ProposalOp>,
}

fn pending() -> String {
    "pending".into()
}

fn op_type(change: &RecordChange) -> &'static str {
    match (&change.before, &change.after) {
        (None, _) => "create",
        (Some(_), None) => "delete",
        (Some(old), Some(new)) => {
            if new.get("deleted_at").is_some() && old.get("deleted_at").is_none() {
                "delete"
            } else {
                "update"
            }
        }
    }
}

impl Proposal {
    pub fn from_change(
        change: &ChangeSet,
        before: &AgendaData,
        after: &AgendaData,
        summary: &str,
    ) -> Self {
        let lines = change.describe(before, after);
        let operations: Vec<ProposalOp> = change
            .changes
            .iter()
            .zip(lines)
            .map(|(c, line)| ProposalOp {
                op: op_type(c).into(),
                collection: c.collection.clone(),
                id: c.id.clone(),
                before: c.before.clone(),
                after: c.after.clone(),
                summary: line,
            })
            .collect();
        let summary = if summary.trim().is_empty() {
            format!("{} 项日程待办变更", operations.len())
        } else {
            summary.trim().to_owned()
        };
        Self {
            id: format!(
                "agenda-{}-{}",
                crate::jstime::now_millis(),
                crate::paths::random_base36(6)
            ),
            title: "日程待办变更".into(),
            summary,
            status: pending(),
            workspace_root: None,
            operations,
        }
    }

    pub fn has_delete(&self) -> bool {
        self.operations.iter().any(ProposalOp::is_delete)
    }

    /// 还原成可应用的变更集。
    ///
    /// 聚合卡片可能把几份基于同一版本生成的提案拼在一起，同一条记录会出现多次：
    /// 前后衔接的直接串起来；从同一版本分叉的按字段三方合并。
    pub fn to_change(&self) -> ChangeSet {
        let mut merged: Vec<RecordChange> = Vec::new();
        for op in &self.operations {
            let next = RecordChange {
                collection: op.collection.clone(),
                id: op.id.clone(),
                before: op.before.clone(),
                after: op.after.clone(),
            };
            let Some(existing) = merged
                .iter_mut()
                .find(|c| c.collection == next.collection && c.id == next.id)
            else {
                merged.push(next);
                continue;
            };
            if next.before == existing.after {
                existing.after = next.after;
            } else if let (Some(base), Some(Value::Object(ours)), Some(Value::Object(theirs))) = (
                existing.before.as_ref(),
                existing.after.as_mut(),
                next.after.as_ref(),
            ) {
                for (key, value) in theirs {
                    if base.get(key) != Some(value) {
                        ours.insert(key.clone(), value.clone());
                    }
                }
                if let Some(base) = base.as_object() {
                    for key in base.keys() {
                        if !theirs.contains_key(key) {
                            ours.remove(key);
                        }
                    }
                }
            } else {
                existing.after = next.after;
            }
        }
        merged.retain(|c| c.before != c.after);
        ChangeSet { changes: merged }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        Ok(serde_json::from_value(value.clone())?)
    }

    /// 给会话正文用的 Markdown 预览。
    pub fn preview_markdown(&self) -> String {
        let mut out = format!("**{}**\n", self.summary);
        for (index, op) in self.operations.iter().enumerate() {
            out.push_str(&format!("\n{}. {}", index + 1, op.summary));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::ops::{Clock, Editor, TaskDraft};
    use serde_json::json;

    #[test]
    fn forked_updates_merge_by_field() {
        let mut base = AgendaData::default();
        let now = crate::agenda::time::parse_stamp("2026-09-25T08:00").unwrap();
        let id = Editor::new(&mut base, Clock::fixed(now))
            .create_task(TaskDraft {
                title: "a".into(),
                ..Default::default()
            })
            .unwrap();
        let proposal = |patch: Value| {
            let mut next = base.clone();
            Editor::new(&mut next, Clock::fixed(now))
                .update(&id, patch.as_object().unwrap())
                .unwrap();
            let change = ChangeSet::diff(&base, &next);
            Proposal::from_change(&change, &base, &next, "")
        };
        let mut first = proposal(json!({"title": "b"}));
        let second = proposal(json!({"due": "2026-10-01"}));
        first.operations.extend(second.operations);
        let mut data = base.clone();
        first.to_change().apply(&mut data, true).unwrap();
        let task = data.task(&id).unwrap();
        assert_eq!(task.title, "b");
        assert_eq!(task.due.as_deref(), Some("2026-10-01"));
    }
}
