//! 为工作流提供读取和修改工作区对象的工具。
use super::*;
use crate::agenda::{proposal::MESSAGE_KEY, AgendaStore};
use crate::ai::tools::{agenda_tools::AgendaToolExecutor, base_tools::BaseToolExecutor};
use crate::object_reference::{ObjectKind, ObjectReference};

impl NativeHost {
    fn reference_args(&self, input: &Value) -> Result<Value> {
        let mut args = input.clone();
        if let Some(raw) = input["object"].as_str() {
            let reference = ObjectReference::parse(raw).ok_or("对象引用无效")?;
            if reference.path.is_some() {
                args["path"] = json!(self.path(input)?);
            }
            if let Some(id) = reference.table_id {
                args["tableId"] = json!(id);
            }
            if let Some(id) = reference.record_id {
                args["recordId"] = json!(id);
            }
            if let Some(id) = reference.item_id {
                args["id"] = json!(id);
            }
        }
        Ok(args)
    }
    fn tool_host(&self) -> Arc<dyn ToolHost> {
        Arc::new(Self {
            root: self.root.clone(),
            settings: self.settings.clone(),
            blocked_paths: self.blocked_paths.clone(),
            store: self.store.clone(),
        })
    }
    pub(super) fn tool(&self, node: &Node, input: &Value) -> Result<Value> {
        let args = self.reference_args(input)?;
        let name = if node.kind == "base_write" {
            match node.config["operation"].as_str().unwrap_or("create") {
                "create" => "base_create_record",
                "create_many" => "base_create_records",
                "update" => "base_update_record",
                _ => return Err("多维表格操作无效".into()),
            }
        } else {
            node.config["name"].as_str().ok_or("缺少工具名称")?
        };
        let args = ToolArgs::from_value(args);
        let base = BaseToolExecutor::new(self.tool_host());
        if base.handles(name) {
            return base.call(name, &args);
        }
        let agenda = AgendaToolExecutor::new(Arc::new(AgendaStore::new(&self.root)));
        if agenda.handles(name) {
            let _gate = crate::base_automation::MUTATION_GATE
                .lock()
                .map_err(|_| "写入锁不可用")?;
            return apply_pending(&agenda, agenda.call(name, &args)?);
        }
        let files = crate::ai::tools::core::CoreToolExecutor::new(self.tool_host());
        if files.handles(name) {
            return files.call(name, &args);
        }
        Err(format!("工作流尚未实现工具 {name}；可使用脚本节点执行"))
    }
    /// 日程待办节点：create_task / create_event / update_task / update_event。
    /// 工作流是用户显式搭建的自动化，生成的变更直接应用。
    pub(super) fn schedule(&self, node: &Node, input: &Value) -> Result<Value> {
        let args = self.reference_args(input)?;
        let mut fields = args.as_object().cloned().unwrap_or_default();
        fields.remove("object");
        let date = fields
            .remove("date")
            .and_then(|v| v.as_str().map(str::to_owned));
        if let Some(date) = &date {
            if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
                return Err("date 应为 YYYY-MM-DD".into());
            }
        }
        let operation = node.config["operation"].as_str().unwrap_or("create_task");
        let op = match operation {
            "create_task" => {
                if let Some(date) =
                    date.filter(|_| !fields.contains_key("due") && !fields.contains_key("dueAt"))
                {
                    fields.insert("due".into(), json!(date));
                }
                fields.insert("op".into(), json!("create"));
                fields.insert("kind".into(), json!("task"));
                Value::Object(fields)
            }
            "create_event" => {
                if let Some(date) = date
                    .filter(|_| !fields.contains_key("start") && !fields.contains_key("startAt"))
                {
                    fields.insert("start".into(), json!(date));
                    fields.insert("allDay".into(), json!(true));
                }
                fields.insert("op".into(), json!("create"));
                fields.insert("kind".into(), json!("entry"));
                Value::Object(fields)
            }
            "update_task" | "update_event" => {
                let id = fields.remove("id").ok_or("缺少 id")?;
                let nested = ["updates", "fields", "set"]
                    .iter()
                    .find_map(|key| fields.get(*key).filter(|v| v.is_object()).cloned());
                json!({"op": "update", "id": id, "fields": nested.unwrap_or(Value::Object(fields))})
            }
            _ => {
                return Err(
                    "日程操作应为 create_task / create_event / update_task / update_event".into(),
                )
            }
        };
        let _gate = crate::base_automation::MUTATION_GATE
            .lock()
            .map_err(|_| "写入锁不可用")?;
        let tool = AgendaToolExecutor::new(Arc::new(AgendaStore::new(&self.root)));
        let result = tool.call(
            "agenda_batch",
            &ToolArgs::from_value(json!({"operations": [op], "summary": "工作流"})),
        )?;
        apply_pending(&tool, result)
    }
    pub(super) fn object(&self, input: &Value) -> Result<Value> {
        let reference = ObjectReference::parse(input["object"].as_str().ok_or("缺少 object URL")?)
            .ok_or("对象引用无效")?;
        match reference.kind {
            ObjectKind::Document => self.file(&Node::new("read", "file_read", 0., 0.), input),
            ObjectKind::Directory => self.file(&Node::new("read", "folder_list", 0., 0.), input),
            ObjectKind::Record => {
                let args = self.reference_args(input)?;
                let path = self.path(&args)?;
                if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 10 * 1024 * 1024 {
                    return Err("多维表格超过 10 MiB".into());
                }
                let doc = crate::base::parse_base_document(
                    &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let table = doc
                    .tables
                    .iter()
                    .find(|t| Some(t.id.as_str()) == reference.table_id.as_deref())
                    .ok_or("数据表不存在")?;
                let record = table
                    .records
                    .iter()
                    .find(|r| Some(r.id.as_str()) == reference.record_id.as_deref())
                    .ok_or("记录不存在")?;
                Ok(json!({"record":record,"tableId":table.id,"path":path}))
            }
            ObjectKind::Task | ObjectKind::Event | ObjectKind::Project => {
                let data = AgendaStore::new(&self.root)
                    .load()
                    .map_err(|e| e.to_string())?;
                let id = reference.item_id.as_deref().unwrap_or_default();
                let value = match reference.kind {
                    ObjectKind::Task => data.task(id).map(serde_json::to_value),
                    ObjectKind::Event => data.entry(id).map(serde_json::to_value),
                    _ => data.project(id).map(serde_json::to_value),
                };
                value.ok_or("日程对象不存在")?.map_err(|e| e.to_string())
            }
            ObjectKind::Block => {
                let path = self.path(input)?;
                if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
                    return Err("块引用所在文档超过 4 MiB，请使用脚本分批读取".into());
                }
                let source = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let block = crate::document_blocks::read_block_from_source(
                    Path::new(&path),
                    &source,
                    reference.block_id.as_deref().ok_or("缺少 block id")?,
                )
                .map_err(|e| e.to_string())?;
                serde_json::to_value(block).map_err(|e| e.to_string())
            }
            _ => Err("该对象类型暂不支持工作流读取".into()),
        }
    }
}

/// 工作流里的日程待办变更不经审批卡片，生成即应用。
fn apply_pending(tool: &AgendaToolExecutor, result: Value) -> Result<Value> {
    match result.get(MESSAGE_KEY).filter(|p| p["status"] == "pending") {
        Some(proposal) => {
            let applied = tool.call(
                "agenda_apply_change",
                &ToolArgs::from_value(json!({ MESSAGE_KEY: proposal })),
            )?;
            let mut out = result.clone();
            out[MESSAGE_KEY] = applied[MESSAGE_KEY].clone();
            Ok(out)
        }
        None => Ok(result),
    }
}
