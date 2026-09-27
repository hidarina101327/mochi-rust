//! 管理工作流文件夹的查询、保存、移动和删除。
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowFolder {
    pub id: String,
    pub name: String,
}

impl Store {
    pub fn folders(&self) -> Result<Vec<WorkflowFolder>> {
        let db = self.db()?;
        let mut stmt = db
            .prepare("SELECT id,name FROM workflow_folders ORDER BY name COLLATE NOCASE")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(WorkflowFolder {
                    id: r.get(0)?,
                    name: r.get(1)?,
                })
            })
            .map_err(err)?
            .map(|r| r.map_err(err))
            .collect();
        rows
    }

    pub fn save_folder(&self, id: Option<&str>, name: &str) -> Result<WorkflowFolder> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err("文件夹名称需为 1–80 个字符".into());
        }
        let id = id.map(str::to_owned).unwrap_or_else(|| new_id("folder"));
        self.db()?.execute(
            "INSERT INTO workflow_folders(id,name) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name",
            params![id, name],
        ).map_err(|_| "文件夹名称已存在，或无法保存文件夹".to_string())?;
        Ok(WorkflowFolder {
            id,
            name: name.into(),
        })
    }

    pub fn move_to_folder(&self, workflow_id: &str, folder_id: Option<&str>) -> Result<()> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workflows WHERE id=?)",
                [workflow_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !exists {
            return Err("工作流不存在".into());
        }
        if let Some(folder_id) = folder_id {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM workflow_folders WHERE id=?)",
                    [folder_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !exists {
                return Err("文件夹不存在".into());
            }
            tx.execute("INSERT INTO workflow_locations(workflow_id,folder_id) VALUES(?,?) ON CONFLICT(workflow_id) DO UPDATE SET folder_id=excluded.folder_id", params![workflow_id, folder_id]).map_err(err)?;
        } else {
            tx.execute(
                "DELETE FROM workflow_locations WHERE workflow_id=?",
                [workflow_id],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    /// 删除文件夹后，其工作流回到资料库根下。
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        tx.execute("DELETE FROM workflow_locations WHERE folder_id=?", [id])
            .map_err(err)?;
        tx.execute("DELETE FROM workflow_folders WHERE id=?", [id])
            .map_err(err)?;
        tx.commit().map_err(err)
    }
}
