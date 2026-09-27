//! 按批次应用工作流的创建、更新、移动和删除操作。
use super::*;

/// 批内操作按顺序在同一个 SQLite 事务里执行。revision 指的是该步
/// 执行时的状态（一次保存会让后续步骤的 revision +1）。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowOperation {
    Save {
        definition: Workflow,
        revision: Option<i64>,
    },
    Delete {
        id: String,
        revision: i64,
    },
    SetSchedule {
        id: String,
        revision: i64,
        enabled: bool,
    },
    Move {
        id: String,
        revision: i64,
        folder_id: Option<String>,
    },
}

impl Store {
    pub fn apply_batch(&self, operations: &[WorkflowOperation]) -> Result<Vec<Value>> {
        if operations.is_empty() || operations.len() > 50 {
            return Err("每批需要 1–50 个工作流操作".into());
        }
        if serde_json::to_vec(operations).map_err(err)?.len() > 4 * 1024 * 1024 {
            return Err("工作流批量参数超过 4 MiB".into());
        }
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let mut results = Vec::new();
        for (index, operation) in operations.iter().enumerate() {
            results.push(
                Self::apply_operation(&tx, operation)
                    .map_err(|e| format!("第 {} 项失败，整批未应用：{e}", index + 1))?,
            );
        }
        tx.commit().map_err(err)?;
        Ok(results)
    }

    fn apply_operation(tx: &Connection, operation: &WorkflowOperation) -> Result<Value> {
        if let WorkflowOperation::Save {
            definition,
            revision,
        } = operation
        {
            return serde_json::to_value(Self::save_in(tx, definition, *revision)?).map_err(err);
        }
        let (id, revision) = match operation {
            WorkflowOperation::Delete { id, revision }
            | WorkflowOperation::SetSchedule { id, revision, .. }
            | WorkflowOperation::Move { id, revision, .. } => (id, *revision),
            WorkflowOperation::Save { .. } => unreachable!(),
        };
        let saved = Self::get_in(tx, id)?;
        if saved.revision != revision {
            return Err("工作流已修改，请重新读取后重试".into());
        }
        match operation {
            WorkflowOperation::Delete { .. } => {
                let changed = tx.execute("DELETE FROM workflows WHERE id=? AND NOT EXISTS(SELECT 1 FROM runs WHERE workflow_id=? AND status IN ('queued','running'))", params![id, id]).map_err(err)?;
                if changed != 1 {
                    return Err("工作流仍在运行，无法删除".into());
                }
                tx.execute("DELETE FROM workflow_locations WHERE workflow_id=?", [id])
                    .map_err(err)?;
                tx.execute("DELETE FROM slots WHERE workflow_id=?", [id])
                    .map_err(err)?;
                Ok(serde_json::json!({"id":id,"deleted":true}))
            }
            WorkflowOperation::SetSchedule { enabled, .. } => {
                if *enabled && matches!(saved.definition.trigger, Trigger::Manual) {
                    return Err("手动工作流没有定时触发器，请先修改 trigger".into());
                }
                validate(&saved.definition)?;
                if *enabled {
                    validation::validate_input(
                        &saved.definition.defaults,
                        &saved.definition.input_schema,
                    )?;
                }
                tx.execute(
                    "UPDATE workflows SET approved_hash=?,enabled=? WHERE id=?",
                    params![fingerprint(&saved.definition), enabled, id],
                )
                .map_err(err)?;
                Ok(serde_json::json!({"id":id,"revision":revision,"enabled":enabled}))
            }
            WorkflowOperation::Move { folder_id, .. } => {
                if let Some(folder) = folder_id {
                    let exists: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM workflow_folders WHERE id=?)",
                            [folder],
                            |r| r.get(0),
                        )
                        .map_err(err)?;
                    if !exists {
                        return Err("文件夹不存在".into());
                    }
                    tx.execute("INSERT INTO workflow_locations(workflow_id,folder_id) VALUES(?,?) ON CONFLICT(workflow_id) DO UPDATE SET folder_id=excluded.folder_id", params![id, folder]).map_err(err)?;
                } else {
                    tx.execute("DELETE FROM workflow_locations WHERE workflow_id=?", [id])
                        .map_err(err)?;
                }
                Ok(serde_json::json!({"id":id,"revision":revision,"folder_id":folder_id}))
            }
            WorkflowOperation::Save { .. } => unreachable!(),
        }
    }
}
