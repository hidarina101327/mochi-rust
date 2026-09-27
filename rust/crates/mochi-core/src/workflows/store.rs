//! 使用 SQLite 持久化工作流、运行记录和文件夹数据。
use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
mod folders;
pub use folders::WorkflowFolder;
mod batch;
pub use batch::WorkflowOperation;

#[derive(Clone)]
pub struct Store {
    path: PathBuf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub workflow_id: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub node_count: usize,
    pub revision: i64,
    pub approved: bool,
    pub enabled: bool,
    pub updated_at: i64,
    #[serde(default)]
    pub folder_id: Option<String>,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        let dir = root.join(".mochi/workflows");
        std::fs::create_dir_all(&dir).map_err(err)?;
        let store = Self {
            path: dir.join("workflows.sqlite3"),
        };
        let db = store.db()?;
        db.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS workflows(id TEXT PRIMARY KEY, definition TEXT NOT NULL, revision INTEGER NOT NULL,
                approved_hash TEXT, enabled INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS runs(id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL, status TEXT NOT NULL,
                source TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER, heartbeat INTEGER NOT NULL,
                cancel INTEGER NOT NULL DEFAULT 0, payload TEXT NOT NULL);
            CREATE UNIQUE INDEX IF NOT EXISTS one_active_flow ON runs(workflow_id) WHERE status IN ('queued','running');
            CREATE INDEX IF NOT EXISTS run_history ON runs(workflow_id,started_at DESC);
            CREATE TABLE IF NOT EXISTS slots(workflow_id TEXT PRIMARY KEY, slot TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS notices(id TEXT PRIMARY KEY, title TEXT NOT NULL, message TEXT NOT NULL, consumed INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS editor_draft(id INTEGER PRIMARY KEY CHECK(id=1),payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS dirty_editors(owner INTEGER PRIMARY KEY,heartbeat INTEGER NOT NULL,paths TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS workflow_folders(id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS workflow_locations(workflow_id TEXT PRIMARY KEY, folder_id TEXT NOT NULL);") .map_err(err)?;
        Ok(store)
    }
    fn db(&self) -> Result<Connection> {
        let db = Connection::open(&self.path).map_err(err)?;
        db.busy_timeout(Duration::from_secs(3)).map_err(err)?;
        Ok(db)
    }
    pub fn list(&self) -> Result<Vec<SavedWorkflow>> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT definition,revision,approved_hash,enabled,updated_at FROM workflows ORDER BY updated_at DESC").map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })
            .map_err(err)?;
        rows.map(|r| {
            let (text, revision, _hash, enabled, updated_at) = r.map_err(err)?;
            let definition: Workflow = serde_json::from_str(&text).map_err(err)?;
            let approved = true; // 兼容字段：已保存的工作流均可信任脚本。
            Ok(SavedWorkflow {
                definition,
                revision,
                approved,
                enabled: enabled && approved,
                updated_at,
            })
        })
        .collect()
    }
    pub fn get(&self, id: &str) -> Result<SavedWorkflow> {
        Self::get_in(&self.db()?, id)
    }
    fn get_in(db: &Connection, id: &str) -> Result<SavedWorkflow> {
        let (text,revision,_hash,enabled,updated_at):(String,i64,Option<String>,bool,i64)=db.query_row(
            "SELECT definition,revision,approved_hash,enabled,updated_at FROM workflows WHERE id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(err)?;
        let definition: Workflow = serde_json::from_str(&text).map_err(err)?;
        let approved = true;
        Ok(SavedWorkflow {
            definition,
            revision,
            approved,
            enabled: enabled && approved,
            updated_at,
        })
    }
    pub fn summaries(&self) -> Result<Vec<WorkflowSummary>> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT id,json_extract(definition,'$.name'),json_extract(definition,'$.description'),json_array_length(definition,'$.nodes'),revision,1,enabled,updated_at,(SELECT folder_id FROM workflow_locations WHERE workflow_id=workflows.id) FROM workflows ORDER BY updated_at DESC").map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(WorkflowSummary {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    node_count: r.get::<_, i64>(3)? as usize,
                    revision: r.get(4)?,
                    approved: r.get(5)?,
                    enabled: r.get(6)?,
                    updated_at: r.get(7)?,
                    folder_id: r.get(8)?,
                })
            })
            .map_err(err)?
            .map(|r| r.map_err(err))
            .collect();
        rows
    }

    pub fn save(&self, flow: &Workflow, expected: Option<i64>) -> Result<SavedWorkflow> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let saved = Self::save_in(&tx, flow, expected)?;
        tx.commit().map_err(err)?;
        Ok(saved)
    }
    fn save_in(tx: &Connection, flow: &Workflow, expected: Option<i64>) -> Result<SavedWorkflow> {
        validate(flow)?;
        let old: Option<(i64, String)> = tx
            .query_row(
                "SELECT revision,definition FROM workflows WHERE id=?",
                [&flow.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(err)?;
        if old.as_ref().map(|o| o.0) != expected {
            return Err("工作流已被其他编辑者修改，请重新加载后保存".into());
        }
        if old.is_none()
            && tx
                .query_row("SELECT count(*) FROM workflows", [], |r| r.get::<_, i64>(0))
                .map_err(err)?
                >= 200
        {
            return Err("工作流数量已达 200".into());
        }
        let rev = expected.unwrap_or(0) + 1;
        let hash = fingerprint(flow);
        let trigger_changed = old
            .as_ref()
            .and_then(|o| serde_json::from_str::<Workflow>(&o.1).ok())
            .is_some_and(|previous| previous.trigger != flow.trigger);
        tx.execute("INSERT INTO workflows(id,definition,revision,approved_hash,enabled,updated_at) VALUES (?1,?2,?3,?5,0,?4)
            ON CONFLICT(id) DO UPDATE SET definition=excluded.definition,revision=excluded.revision,updated_at=excluded.updated_at,
            approved_hash=excluded.approved_hash, enabled=CASE WHEN ?6 THEN 0 ELSE enabled END",
            params![flow.id,serde_json::to_string(flow).map_err(err)?,rev,now(),hash,matches!(flow.trigger, Trigger::Manual)]).map_err(err)?;
        if trigger_changed {
            tx.execute("DELETE FROM slots WHERE workflow_id=?", [&flow.id])
                .map_err(err)?;
        }
        Self::get_in(tx, &flow.id)
    }
    pub fn import(&self, text: &str) -> Result<SavedWorkflow> {
        if text.len() > 524288 {
            return Err("导入 JSON 超过 512 KiB".into());
        }
        let mut flow: Workflow = serde_json::from_str(text).map_err(err)?;
        flow.id = new_id("flow");
        self.save(&flow, None)
    }
    // 兼容较早的原生调用方。审批不再决定能否执行工作流。
    pub fn approve(&self, id: &str, revision: i64, enabled: bool) -> Result<()> {
        self.set_schedule(id, revision, enabled)
    }
    pub fn set_schedule(&self, id: &str, revision: i64, enabled: bool) -> Result<()> {
        let saved = self.get(id)?;
        let enabled = enabled && !matches!(saved.definition.trigger, Trigger::Manual);
        if saved.revision != revision {
            return Err("工作流已修改，请重新检查".into());
        }
        validate(&saved.definition)?;
        if enabled {
            validation::validate_input(&saved.definition.defaults, &saved.definition.input_schema)?;
        }
        let changed = self
            .db()?
            .execute(
                "UPDATE workflows SET approved_hash=?,enabled=? WHERE id=? AND revision=?",
                params![fingerprint(&saved.definition), enabled, id, revision],
            )
            .map_err(err)?;
        if changed != 1 {
            return Err("工作流已修改，请重新检查".into());
        }
        Ok(())
    }
    pub fn pause(&self, id: &str) -> Result<()> {
        self.db()?
            .execute("UPDATE workflows SET enabled=0 WHERE id=?", [id])
            .map_err(err)?;
        Ok(())
    }
    pub fn set_dirty_paths(&self, paths: &[PathBuf]) -> Result<()> {
        let db = self.db()?;
        if paths.is_empty() {
            db.execute(
                "DELETE FROM dirty_editors WHERE owner=?",
                [std::process::id()],
            )
            .map_err(err)?;
        } else {
            db.execute("INSERT INTO dirty_editors VALUES(?,?,?) ON CONFLICT(owner) DO UPDATE SET heartbeat=excluded.heartbeat,paths=excluded.paths",params![std::process::id(),now(),serde_json::to_string(paths).map_err(err)?]).map_err(err)?;
        }
        Ok(())
    }
    pub fn dirty_paths(&self) -> Result<Vec<PathBuf>> {
        let db = self.db()?;
        let mut stmt = db
            .prepare("SELECT paths FROM dirty_editors WHERE heartbeat>?")
            .map_err(err)?;
        let rows = stmt
            .query_map([now() - 120_000], |r| r.get::<_, String>(0))
            .map_err(err)?;
        let mut paths = Vec::new();
        for row in rows {
            paths.extend(serde_json::from_str::<Vec<PathBuf>>(&row.map_err(err)?).map_err(err)?);
        }
        Ok(paths)
    }
    pub fn save_editor_draft(&self, value: Option<&Value>) -> Result<()> {
        let db = self.db()?;
        if let Some(value) = value {
            let text = value.to_string();
            if text.len() > 2 * 1024 * 1024 {
                return Err("编辑草稿超过 2 MiB".into());
            }
            db.execute("INSERT INTO editor_draft VALUES(1,?) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",[text]).map_err(err)?;
        } else {
            db.execute("DELETE FROM editor_draft", []).map_err(err)?;
        }
        Ok(())
    }
    pub fn editor_draft(&self) -> Result<Option<Value>> {
        let text: Option<String> = self
            .db()?
            .query_row("SELECT payload FROM editor_draft WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        text.map(|s| serde_json::from_str(&s).map_err(err))
            .transpose()
    }
    pub fn delete(&self, id: &str, revision: i64) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction().map_err(err)?;
        let changed=tx.execute("DELETE FROM workflows WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM runs WHERE workflow_id=? AND status IN ('queued','running'))",params![id,revision,id]).map_err(err)?;
        if changed != 1 {
            return Err("工作流已修改或仍在运行，无法删除".into());
        }
        tx.execute("DELETE FROM workflow_locations WHERE workflow_id=?", [id])
            .map_err(err)?;
        tx.commit().map_err(err)
    }
    pub fn enqueue(
        &self,
        id: &str,
        input: Value,
        source: &str,
        slot: Option<&str>,
    ) -> Result<Option<Run>> {
        self.recover_stale()?;
        let saved = self.get(id)?;
        if slot.is_some() && !saved.enabled {
            return Err("定时运行已暂停".into());
        }
        let mut merged = saved.definition.defaults.clone();
        for (k, v) in input.as_object().ok_or("运行输入必须为 JSON 对象")? {
            merged[k] = v.clone();
        }
        validation::validate_input(&merged, &saved.definition.input_schema)?;
        if merged.to_string().len() > 1_048_576 {
            return Err("运行输入超过 1 MiB".into());
        }
        let run = Run {
            id: new_id("run"),
            workflow_id: id.into(),
            source: source.into(),
            status: "queued".into(),
            started_at: now(),
            finished_at: None,
            input: merged,
            output: Value::Null,
            error: None,
            nodes: saved
                .definition
                .nodes
                .iter()
                .map(|n| (n.id.clone(), NodeRun::default()))
                .collect(),
            definition: saved.definition,
        };
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let current: Option<(i64, bool)> = tx
            .query_row(
                "SELECT revision,enabled FROM workflows WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(err)?;
        if !current
            .is_some_and(|(rev, enabled)| rev == saved.revision && (slot.is_none() || enabled))
        {
            return Err("工作流版本或定时状态已变化，请重试".into());
        }
        if let Some(slot) = slot {
            let old: Option<String> = tx
                .query_row("SELECT slot FROM slots WHERE workflow_id=?", [id], |r| {
                    r.get(0)
                })
                .optional()
                .map_err(err)?;
            if old.as_deref() == Some(slot) {
                return Ok(None);
            }
        }
        if tx
            .query_row(
                "SELECT count(*) FROM runs WHERE workflow_id=? AND status IN ('queued','running')",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .map_err(err)?
            > 0
        {
            return Ok(None);
        }
        tx.execute("INSERT INTO runs(id,workflow_id,status,source,started_at,heartbeat,payload) VALUES(?,?,'queued',?,?,?,?)",params![run.id,id,source,run.started_at,now(),serde_json::to_string(&run).map_err(err)?]).map_err(err)?;
        if let Some(slot) = slot {
            tx.execute("INSERT INTO slots VALUES(?,?) ON CONFLICT(workflow_id) DO UPDATE SET slot=excluded.slot",params![id,slot]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(Some(run))
    }
    pub fn claim(&self) -> Result<Option<Run>> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT id,payload FROM runs WHERE status='queued' ORDER BY started_at LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(err)?;
        let Some((id, payload)) = row else {
            return Ok(None);
        };
        let mut run: Run = serde_json::from_str(&payload).map_err(err)?;
        run.status = "running".into();
        tx.execute(
            "UPDATE runs SET status='running',heartbeat=?,payload=? WHERE id=?",
            params![now(), serde_json::to_string(&run).map_err(err)?, id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(Some(run))
    }
    pub fn save_run(&self, run: &Run) -> Result<()> {
        let text = serde_json::to_string(run).map_err(err)?;
        if text.len() > 8 * 1024 * 1024 {
            return Err("运行记录超过 8 MiB，请拆分批次".into());
        }
        let db = self.db()?;
        db.execute(
            "UPDATE runs SET status=?,finished_at=?,heartbeat=?,payload=? WHERE id=?",
            params![run.status, run.finished_at, now(), text, run.id],
        )
        .map_err(err)?;
        if run.finished_at.is_some() {
            db.execute("DELETE FROM runs WHERE workflow_id=?1 AND status NOT IN ('queued','running') AND id NOT IN (SELECT id FROM runs WHERE workflow_id=?1 ORDER BY started_at DESC LIMIT 50)",[&run.workflow_id]).map_err(err)?;
        }
        Ok(())
    }
    pub fn history(&self, id: &str) -> Result<Vec<RunSummary>> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT id,workflow_id,status,started_at,finished_at,source FROM runs WHERE workflow_id=? ORDER BY started_at DESC LIMIT 50").map_err(err)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok(RunSummary {
                    id: r.get(0)?,
                    workflow_id: r.get(1)?,
                    status: r.get(2)?,
                    started_at: r.get(3)?,
                    finished_at: r.get(4)?,
                    source: r.get(5)?,
                })
            })
            .map_err(err)?
            .map(|r| r.map_err(err))
            .collect();
        rows
    }
    pub fn run(&self, id: &str) -> Result<Run> {
        let s: String = self
            .db()?
            .query_row("SELECT payload FROM runs WHERE id=?", [id], |r| r.get(0))
            .map_err(err)?;
        serde_json::from_str(&s).map_err(err)
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        self.db()?
            .execute(
                "UPDATE runs SET cancel=1 WHERE id=? AND status IN ('queued','running')",
                [id],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn cancelled(&self, id: &str) -> bool {
        self.db()
            .and_then(|db| {
                db.query_row("SELECT cancel FROM runs WHERE id=?", [id], |r| {
                    r.get::<_, bool>(0)
                })
                .map_err(err)
            })
            .unwrap_or(true)
    }
    pub fn heartbeat(&self, id: &str) -> Result<()> {
        self.db()?
            .execute(
                "UPDATE runs SET heartbeat=? WHERE id=? AND status='running'",
                params![now(), id],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn recover_stale(&self) -> Result<()> {
        let db = self.db()?;
        let mut stmt = db
            .prepare("SELECT payload FROM runs WHERE status='running' AND heartbeat<?")
            .map_err(err)?;
        let rows: Vec<String> = stmt
            .query_map([now() - 120_000], |r| r.get(0))
            .map_err(err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(err)?;
        for text in rows {
            let mut r: Run = serde_json::from_str(&text).map_err(err)?;
            r.status = "interrupted".into();
            r.finished_at = Some(now());
            r.error = Some("执行进程中断；副作用节点不会自动重放".into());
            for n in r.nodes.values_mut().filter(|n| n.status == "running") {
                n.status = "interrupted".into();
                n.finished_at = r.finished_at;
            }
            self.save_run(&r)?;
        }
        Ok(())
    }
    pub fn notify(&self, id: &str, title: &str, message: &str) -> Result<()> {
        self.db()?
            .execute(
                "INSERT OR IGNORE INTO notices(id,title,message) VALUES(?,?,?)",
                params![id, title, message],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn take_notices(&self) -> Result<Vec<(String, String)>> {
        let mut db = self.db()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let rows = {
            let mut stmt = tx
                .prepare("SELECT title,message FROM notices WHERE consumed=0 LIMIT 50")
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            rows
        };
        tx.execute("DELETE FROM notices WHERE rowid IN (SELECT rowid FROM notices WHERE consumed=0 LIMIT 50)",[]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(rows)
    }
}
