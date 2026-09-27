//! 有限范围的结构化 .mcb 操作，与 shared/agent-tools/base-tools.ts 保持一致。
//! 文件权限和覆盖提案仍按常规 ToolRegistry 与宿主流程处理。
use crate::base_automation::MUTATION_GATE;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::host::{
    agent_path_basename, resolve_workspace_path, FileChange, PendingFileOperation, ResolveOptions,
    ToolHost,
};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::base::{self, BaseDocument, BaseRecord, BaseTable, BaseView};

const NAMES: &[&str] = &[
    "base_get_schema",
    "base_query_records",
    "base_create_record",
    "base_create_records",
    "base_update_record",
];
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

pub struct BaseToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl BaseToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    fn resolve(&self, args: &ToolArgs) -> Result<String, String> {
        let path = optional_text(args.get("path"), "path")?
            .map(str::to_owned)
            .or_else(|| self.host.active_document().map(|document| document.path))
            .ok_or("请提供 .mcb 路径，或先打开一个多维表格文档")?;
        if !path.to_ascii_lowercase().ends_with(".mcb") {
            return Err("请提供 .mcb 路径，或先打开一个多维表格文档".into());
        }
        resolve_workspace_path(
            self.host.as_ref(),
            Some(&path),
            ResolveOptions {
                fallback_to_selection: false,
                allow_mochi_dir: false,
            },
        )
    }

    fn run(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let path = self.resolve(args)?;
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err("多维表格必须是最大 10 MiB 的文件".into());
        }
        // 基于已保存的原始内容执行。未保存的界面编辑由
        // 宿主的 should_propose 和审批界面的原文检查保护。
        let original = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let mut document =
            base::parse_base_document(&original).map_err(|error| error.to_string())?;
        if name == "base_get_schema" {
            let table_id = optional_text(args.get("tableId"), "tableId")?;
            if let Some(id) = table_id {
                find_table(&document, id)?;
            }
            let tables: Vec<Value> = document.tables.iter().filter(|table| table_id.is_none_or(|id| table.id == id)).map(|table| {
                let mut value = json!({ "id":table.id, "name":table.name, "fields":table.fields, "views":table.views,
                    "recordCount":table.records.len(), "url":self.link(&path, Some(&table.id), None) });
                if let Some(id) = &table.active_view_id { value["activeViewId"] = json!(id); }
                value
            }).collect();
            let mut data = json!({"path":path,"format":document.format,"version":document.version,"tables":tables});
            if let Some(id) = &document.active_table_id {
                data["activeTableId"] = json!(id);
            }
            return bounded_result(data);
        }

        let table_id = required_text(args.get("tableId"), "tableId")?;
        let table_index = document
            .tables
            .iter()
            .position(|table| table.id == table_id)
            .ok_or_else(|| format!("数据表不存在：{table_id}"))?;
        if name == "base_query_records" {
            return self.query(&path, &document, table_index, args);
        }
        if name == "base_create_records" {
            return self.create_records(
                &path,
                &original,
                &mut document,
                table_index,
                table_id,
                args,
            );
        }

        let values =
            values_for_table(&document.tables[table_index], args.get("values"), "values")?.clone();
        let record_id;
        {
            let table = &mut document.tables[table_index];
            if name == "base_create_record" {
                record_id = optional_text(args.get("recordId"), "recordId")?
                    .map(str::to_owned)
                    .unwrap_or_else(|| base::create_id("record"));
                if let Some(existing) = table.records.iter().find(|record| record.id == record_id) {
                    if existing.values.len() == values.len()
                        && values
                            .iter()
                            .all(|(id, value)| equal_value(existing.values.get(id), Some(value)))
                    {
                        let mut data = self.location(&path, &table.id, &record_id);
                        data["applied"] = json!(true);
                        data["created"] = json!(false);
                        data["unchanged"] = json!(true);
                        return bounded_result(data);
                    }
                    return Err(format!("记录 ID 已存在且内容不同：{record_id}"));
                }
                table.records.push(BaseRecord {
                    id: record_id.clone(),
                    values: values.clone(),
                    ..Default::default()
                });
            } else if name == "base_update_record" {
                record_id = required_text(args.get("recordId"), "recordId")?.to_owned();
                let index = table
                    .records
                    .iter()
                    .position(|record| record.id == record_id)
                    .ok_or_else(|| format!("记录不存在：{record_id}"))?;
                if values.is_empty() {
                    return Err("values 至少包含一个待更新字段".into());
                }
                if args.get("expectedValues").is_some() {
                    let expected =
                        values_for_table(table, args.get("expectedValues"), "expectedValues")?;
                    for (id, value) in expected {
                        if !equal_value(table.records[index].values.get(id), Some(value)) {
                            return Err(format!(
                                "记录已发生变化，请重新读取后更新：{record_id}/{id}"
                            ));
                        }
                    }
                }
                table.records[index].values.extend(values.clone());
            } else {
                return Err(format!("未知多维表格工具：{name}"));
            }
        }
        let content =
            base::serialize_base_document(&document).map_err(|error| error.to_string())?;
        let mut data = self.location(&path, table_id, &record_id);
        if self.host.should_propose(&path) {
            let proposal = self.host.submit_proposal(PendingFileOperation {
                kind: "overwrite".into(),
                path: path.clone(),
                title: agent_path_basename(&path),
                new_path: None,
                summary: format!(
                    "{}「{}」记录 {}（{} 个字段）",
                    if name == "base_create_record" {
                        "新增"
                    } else {
                        "更新"
                    },
                    document.tables[table_index].name,
                    record_id,
                    values.len()
                ),
                content: Some(content),
                previous_content: Some(original),
                entries: vec![],
                reason: "destructive".into(),
            })?;
            if let Some(proposal) = proposal.as_object() {
                data.as_object_mut().unwrap().extend(proposal.clone());
            }
            data["applied"] = json!(false);
            return Ok(data);
        }
        if std::fs::read_to_string(&path).map_err(|error| error.to_string())? != original {
            return Err("多维表格在保存前被修改，请重新读取后重试".into());
        }
        crate::files::FileService::new()
            .write_file_safe(&path, &content)
            .map_err(|error| error.to_string())?;
        self.host
            .notify_change(FileChange::Write { path: path.clone() });
        data["applied"] = json!(true);
        data["created"] = json!(name == "base_create_record");
        bounded_result(data)
    }

    /// 根据同一份已保存的文档快照批量创建记录。先检查并规划所有
    /// 输入，再修改表格；因此重复 ID、与现有记录冲突
    /// 或单元格无效都会使
    /// 文件保持原样。最终文档只序列化一次，并且
    /// 只写入一次，或作为单个审批提案提交。
    fn create_records(
        &self,
        path: &str,
        original: &str,
        document: &mut BaseDocument,
        table_index: usize,
        table_id: &str,
        args: &ToolArgs,
    ) -> ToolOutcome {
        let raw_records = args
            .get("records")
            .and_then(Value::as_array)
            .ok_or("records 必须是 2–200 项的数组")?;
        if !(2..=200).contains(&raw_records.len()) {
            return Err("records 必须是 2–200 项的数组".into());
        }

        // 先解析并校验每个项目的结构，再规划任何修改。
        // values_for_table 会在此处捕获未知字段 ID；完整的单元格类型
        // 检查则在整个计划组装完成后，通过 serialize_base_document 执行，
        // 仍早于任何文件写入或提案提交。
        let table = &document.tables[table_index];
        let mut inputs = Vec::with_capacity(raw_records.len());
        for (index, raw) in raw_records.iter().enumerate() {
            let item = raw
                .as_object()
                .ok_or_else(|| format!("records[{index}] 必须是对象"))?;
            let record_id =
                optional_text(item.get("recordId"), &format!("records[{index}].recordId"))?
                    .map(str::to_owned);
            let values = values_for_table(
                table,
                item.get("values"),
                &format!("records[{index}].values"),
            )?
            .clone();
            inputs.push((record_id, values));
        }

        let mut used_ids = HashSet::with_capacity(inputs.len());
        let mut record_ids = Vec::with_capacity(inputs.len());
        let mut new_records = Vec::with_capacity(inputs.len());
        let mut unchanged_count = 0usize;

        for (requested_id, values) in inputs {
            let record_id = requested_id.unwrap_or_else(|| loop {
                let candidate = base::create_id("record");
                if !used_ids.contains(&candidate)
                    && !table.records.iter().any(|record| record.id == candidate)
                {
                    break candidate;
                }
            });
            if !used_ids.insert(record_id.clone()) {
                return Err(format!("records 包含重复 recordId：{record_id}"));
            }
            record_ids.push(record_id.clone());

            if let Some(existing) = table.records.iter().find(|record| record.id == record_id) {
                if existing.values.len() == values.len()
                    && values
                        .iter()
                        .all(|(id, value)| equal_value(existing.values.get(id), Some(value)))
                {
                    unchanged_count += 1;
                    continue;
                }
                return Err(format!("记录 ID 已存在且内容不同：{record_id}"));
            }
            new_records.push(BaseRecord {
                id: record_id,
                values,
                ..Default::default()
            });
        }

        let created_count = new_records.len();
        let mut data = self.batch_location(path, table_id);
        data["recordIds"] = json!(record_ids);
        data["createdCount"] = json!(created_count);
        data["unchangedCount"] = json!(unchanged_count);
        data["created"] = json!(created_count > 0);

        // 完全幂等的重试不会修改文档，因此无需提交提案或
        // 写入磁盘。仍将其报告为已应用，因为所请求的最终状态
        // 已经存在。
        if created_count == 0 {
            data["applied"] = json!(true);
            data["unchanged"] = json!(true);
            return bounded_result(data);
        }

        document.tables[table_index].records.extend(new_records);
        let content = base::serialize_base_document(document).map_err(|error| error.to_string())?;
        if self.host.should_propose(path) {
            let proposal = self.host.submit_proposal(PendingFileOperation {
                kind: "overwrite".into(),
                path: path.to_owned(),
                title: agent_path_basename(path),
                new_path: None,
                summary: format!(
                    "批量新增「{}」记录（共 {} 条，新增 {} 条，已存在 {} 条）",
                    document.tables[table_index].name,
                    created_count + unchanged_count,
                    created_count,
                    unchanged_count
                ),
                content: Some(content),
                previous_content: Some(original.to_owned()),
                entries: vec![],
                reason: "destructive".into(),
            })?;
            if let Some(proposal) = proposal.as_object() {
                data.as_object_mut().unwrap().extend(proposal.clone());
            }
            data["applied"] = json!(false);
            return bounded_result(data);
        }
        if std::fs::read_to_string(path).map_err(|error| error.to_string())? != original {
            return Err("多维表格在保存前被修改，请重新读取后重试".into());
        }
        crate::files::FileService::new()
            .write_file_safe(path, &content)
            .map_err(|error| error.to_string())?;
        self.host.notify_change(FileChange::Write {
            path: path.to_owned(),
        });
        data["applied"] = json!(true);
        bounded_result(data)
    }

    fn query(
        &self,
        path: &str,
        document: &BaseDocument,
        table_index: usize,
        args: &ToolArgs,
    ) -> ToolOutcome {
        let table = &document.tables[table_index];
        let view = optional_text(args.get("viewId"), "viewId")?
            .map(|id| {
                table
                    .views
                    .iter()
                    .find(|view| view.id == id)
                    .ok_or_else(|| format!("视图不存在：{id}"))
            })
            .transpose()?;
        let mut filters = view.map(|view| view.filters.clone()).unwrap_or_default();
        if let Some(raw) = args.get("filters") {
            bounded_array(raw, "filters", 20)?;
            let extra: Vec<base::BaseFilter> = serde_json::from_value(raw.clone())
                .map_err(|error| format!("filters 无效：{error}"))?;
            filters.extend(extra);
        }
        let sorts = if let Some(raw) = args.get("sorts") {
            bounded_array(raw, "sorts", 20)?;
            serde_json::from_value(raw.clone()).map_err(|error| format!("sorts 无效：{error}"))?
        } else {
            view.map(|view| view.sorts.clone()).unwrap_or_default()
        };
        let query = BaseView {
            id: "query".into(),
            name: "查询".into(),
            filters,
            sorts,
            ..Default::default()
        };
        let mut candidate = document.clone();
        candidate.tables[table_index].views = vec![query.clone()];
        candidate.tables[table_index].active_view_id = Some(query.id.clone());
        base::validate_base_document(&candidate).map_err(|error| error.to_string())?;
        let fields: Vec<&base::BaseField> = if let Some(raw) = args.get("fieldIds") {
            bounded_array(raw, "fieldIds", 100)?
                .iter()
                .map(|value| {
                    let id = required_text(Some(value), "fieldIds")?;
                    table
                        .fields
                        .iter()
                        .find(|field| field.id == id)
                        .ok_or_else(|| format!("字段不存在：{id}"))
                })
                .collect::<Result<_, _>>()?
        } else {
            table.fields.iter().collect()
        };
        let search = match args.get("search") {
            None => "",
            Some(Value::String(value)) => value,
            _ => return Err("search 必须是字符串".into()),
        };
        let offset = bounded_integer(args.get("offset"), "offset", 0, 0, 9_007_199_254_740_991)?;
        let limit = bounded_integer(args.get("limit"), "limit", 20, 1, 100)?;
        let matched = base::get_view_records(table, &query, search);
        let records: Vec<Value> = matched.iter().skip(offset).take(limit).map(|record| {
            let values: Map<String,Value> = fields.iter().map(|field| (field.id.clone(),record.values.get(&field.id).cloned().unwrap_or(Value::Null))).collect();
            let display: Map<String,Value> = fields.iter().map(|field| (field.id.clone(),json!(base::format_cell_value(field, record.values.get(&field.id).unwrap_or(&Value::Null))))).collect();
            json!({"id":record.id,"values":values,"displayValues":display,"url":self.link(path,Some(&table.id),Some(&record.id))})
        }).collect();
        bounded_result(
            json!({"path":path,"tableId":table.id,"total":matched.len(),"offset":offset,"limit":limit,"hasMore":offset.saturating_add(records.len())<matched.len(),"records":records}),
        )
    }

    fn link(&self, path: &str, table_id: Option<&str>, record_id: Option<&str>) -> String {
        let mut url = crate::mochi_url::build_mochi_resource_url(
            Path::new(path),
            crate::mochi_url::ResourceKind::File,
            Some(self.host.workspace_root()),
        );
        if let Some(id) = table_id {
            url.push_str(&format!("&table={}", crate::mochi_url::form_encode(id)));
        }
        if let Some(id) = record_id {
            url.push_str(&format!("&record={}", crate::mochi_url::form_encode(id)));
        }
        url
    }
    fn location(&self, path: &str, table: &str, record: &str) -> Value {
        json!({"path":path,"tableId":table,"recordId":record,"url":self.link(path,Some(table),Some(record))})
    }
    fn batch_location(&self, path: &str, table: &str) -> Value {
        json!({"path":path,"tableId":table,"url":self.link(path,Some(table),None)})
    }
}

impl ToolExecutor for BaseToolExecutor {
    fn handles(&self, name: &str) -> bool {
        NAMES.contains(&name)
    }
    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let path = self.resolve(args).ok();
        let mut actions = vec![(AiToolAction::ReadFile, path.clone())];
        if name == "base_create_record"
            || name == "base_create_records"
            || name == "base_update_record"
        {
            actions.push((AiToolAction::WriteFile, path));
        }
        actions
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let _guard = if name == "base_create_record"
            || name == "base_create_records"
            || name == "base_update_record"
        {
            Some(
                MUTATION_GATE
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            )
        } else {
            None
        };
        self.run(name, args)
    }
}

fn required_text<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{name} 必须是非空字符串"))
}
fn optional_text<'a>(value: Option<&'a Value>, name: &str) -> Result<Option<&'a str>, String> {
    value
        .map(|value| required_text(Some(value), name))
        .transpose()
}
fn bounded_array<'a>(value: &'a Value, name: &str, max: usize) -> Result<&'a Vec<Value>, String> {
    value
        .as_array()
        .filter(|value| value.len() <= max)
        .ok_or_else(|| format!("{name} 必须是最多 {max} 项的数组"))
}
fn bounded_integer(
    value: Option<&Value>,
    name: &str,
    fallback: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(fallback);
    };
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value >= min && *value <= max)
        .ok_or_else(|| format!("{name} 必须是 {min}–{max} 的整数"))
}
fn bounded_result(value: Value) -> ToolOutcome {
    if value.to_string().encode_utf16().count() > 64 * 1024 {
        return Err(
            "结果超过 64K 字符，请指定 tableId、缩小 limit 或使用 fieldIds 只读取必要字段".into(),
        );
    }
    Ok(value)
}
fn find_table<'a>(document: &'a BaseDocument, id: &str) -> Result<&'a BaseTable, String> {
    document
        .tables
        .iter()
        .find(|table| table.id == id)
        .ok_or_else(|| format!("数据表不存在：{id}"))
}
fn values_for_table<'a>(
    table: &BaseTable,
    value: Option<&'a Value>,
    name: &str,
) -> Result<&'a Map<String, Value>, String> {
    let values = value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{name} 必须是对象"))?;
    for id in values.keys() {
        if !table.fields.iter().any(|field| &field.id == id) {
            return Err(format!("{name} 包含不存在的字段 ID：{id}"));
        }
    }
    Ok(values)
}
fn equal_value(a: Option<&Value>, b: Option<&Value>) -> bool {
    a.unwrap_or(&Value::Null) == b.unwrap_or(&Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::ActiveDocument;
    use crate::ai::tools::ToolRegistry;
    use std::path::PathBuf;
    use std::sync::Mutex;

    const SOURCE: &str = include_str!("../../../../../../tests/fixtures/base-v1.mcb");
    struct Host {
        root: PathBuf,
        propose: bool,
        changes: Mutex<Vec<FileChange>>,
        proposals: Mutex<Vec<PendingFileOperation>>,
    }
    impl ToolHost for Host {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn active_document(&self) -> Option<ActiveDocument> {
            Some(ActiveDocument {
                id: "base".into(),
                title: "记录".into(),
                path: crate::paths::to_forward_slashes(
                    &self.root.join("记录.mcb").to_string_lossy(),
                ),
                is_dirty: false,
            })
        }
        fn should_propose(&self, _path: &str) -> bool {
            self.propose
        }
        fn submit_proposal(&self, operation: PendingFileOperation) -> Result<Value, String> {
            self.proposals.lock().unwrap().push(operation);
            Ok(json!({"queuedForApproval":true}))
        }
        fn notify_change(&self, change: FileChange) {
            self.changes.lock().unwrap().push(change);
        }
    }
    struct Fixture {
        root: PathBuf,
        host: Arc<Host>,
        registry: ToolRegistry,
    }
    impl Fixture {
        fn new(propose: bool) -> Self {
            let root =
                std::env::temp_dir().join(format!("mochi-base-tool-{}", base::create_id("test")));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("记录.mcb"), SOURCE).unwrap();
            let host = Arc::new(Host {
                root: root.clone(),
                propose,
                changes: Mutex::new(vec![]),
                proposals: Mutex::new(vec![]),
            });
            let registry = ToolRegistry::new(Arc::new(AiPermissionService::new(&root)))
                .with(Arc::new(BaseToolExecutor::new(host.clone())));
            Self {
                root,
                host,
                registry,
            }
        }
        fn run(&self, name: &str, args: Value) -> Value {
            serde_json::from_str(&self.registry.execute(&AiToolCall {
                id: "call".into(),
                kind: "function".into(),
                function: AiToolFunction {
                    name: name.into(),
                    arguments: args.to_string(),
                },
            }))
            .unwrap()
        }
        fn raw(&self) -> String {
            std::fs::read_to_string(self.root.join("记录.mcb")).unwrap()
        }
        fn document(&self) -> BaseDocument {
            base::parse_base_document(&self.raw()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn schema_and_filtered_page_use_stable_ids_and_relocatable_urls() {
        let f = Fixture::new(false);
        let schema = f.run("base_get_schema", json!({}));
        assert_eq!(schema["ok"], true, "{schema}");
        assert_eq!(schema["data"]["tables"][0]["recordCount"], 4);
        assert!(schema["data"]["tables"][0].get("records").is_none());
        assert!(schema["data"]["tables"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("mochi://open?path=%E8%AE%B0%E5%BD%95.mcb&table="));
        let page=f.run("base_query_records",json!({"tableId":"table_main","viewId":"view_board","filters":[{"fieldId":"tags","operator":"contains","value":"标签甲"}],"sorts":[{"fieldId":"amount","direction":"asc"}],"fieldIds":["name","group"],"limit":1}));
        assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["data"]["total"], 2);
        assert_eq!(page["data"]["hasMore"], true);
        assert_eq!(page["data"]["records"][0]["id"], "record_b");
        assert_eq!(
            page["data"]["records"][0]["displayValues"]["group"],
            "分组乙"
        );
        assert_eq!(
            page["data"]["records"][0]["values"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(f.raw(), SOURCE);
    }

    #[test]
    fn partial_update_keeps_all_other_data_and_an_idempotent_create_does_not_duplicate() {
        let f = Fixture::new(false);
        let updated=f.run("base_update_record",json!({"tableId":"table_main","recordId":"record_a","values":{"amount":12},"expectedValues":{"amount":10}}));
        assert_eq!(updated["ok"], true, "{updated}");
        let mut expected = base::parse_base_document(SOURCE).unwrap();
        expected.tables[0].records[0]
            .values
            .insert("amount".into(), json!(12));
        assert_eq!(f.document(), expected);
        assert_eq!(f.host.changes.lock().unwrap().len(), 1);
        let create =
            json!({"tableId":"table_secondary","recordId":"stable_new","values":{"name":"新条目"}});
        assert_eq!(
            f.run("base_create_record", create.clone())["data"]["created"],
            true
        );
        let once = f.raw();
        assert_eq!(
            f.run("base_create_record", create)["data"]["unchanged"],
            true
        );
        assert_eq!(f.run("base_create_record",json!({"tableId":"table_secondary","recordId":"stable_new","values":{"name":"不同"}}))["ok"],false);
        assert_eq!(f.raw(), once);
    }

    #[test]
    fn batch_create_is_atomic_and_idempotent() {
        let f = Fixture::new(false);
        let batch = json!({
            "tableId": "table_secondary",
            "records": [
                {"recordId": "batch_a", "values": {"name": "甲"}},
                {"recordId": "batch_b", "values": {"name": "乙"}},
                {"recordId": "batch_c", "values": {"name": "丙"}}
            ]
        });
        let result = f.run("base_create_records", batch.clone());
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["createdCount"], 3);
        assert_eq!(result["data"]["unchangedCount"], 0);
        assert_eq!(
            result["data"]["recordIds"],
            json!(["batch_a", "batch_b", "batch_c"])
        );
        assert!(result["data"]["url"]
            .as_str()
            .unwrap()
            .contains("table=table_secondary"));
        assert_eq!(f.document().tables[1].records.len(), 3);
        assert_eq!(
            f.host.changes.lock().unwrap().len(),
            1,
            "一个批次只能写一次"
        );

        let unchanged = f.run("base_create_records", batch);
        assert_eq!(unchanged["ok"], true, "{unchanged}");
        assert_eq!(unchanged["data"]["createdCount"], 0);
        assert_eq!(unchanged["data"]["unchangedCount"], 3);
        assert_eq!(f.document().tables[1].records.len(), 3);
        assert_eq!(
            f.host.changes.lock().unwrap().len(),
            1,
            "幂等重试不能再次写入"
        );

        let before_invalid = f.raw();
        let invalid = f.run(
            "base_create_records",
            json!({
                "tableId": "table_secondary",
                "records": [
                    {"recordId": "batch_d", "values": {"name": "丁"}},
                    {"recordId": "batch_e", "values": {"missing": true}}
                ]
            }),
        );
        assert_eq!(invalid["ok"], false, "{invalid}");
        assert_eq!(f.raw(), before_invalid, "全量校验失败时不能写入任何记录");
        assert_eq!(f.host.changes.lock().unwrap().len(), 1);

        let duplicate = f.run(
            "base_create_records",
            json!({
                "tableId": "table_secondary",
                "records": [
                    {"recordId": "batch_x", "values": {"name": "戊"}},
                    {"recordId": "batch_x", "values": {"name": "己"}}
                ]
            }),
        );
        assert_eq!(duplicate["ok"], false, "{duplicate}");
        assert_eq!(f.raw(), before_invalid, "重复 recordId 时不能写入任何记录");
    }

    #[test]
    fn batch_create_writes_54_records_as_one_file_change() {
        let f = Fixture::new(false);
        let records: Vec<Value> = (1..=54)
            .map(|index| {
                json!({
                    "recordId": format!("requested_{index:02}"),
                    "values": {"name": format!("记录 {index:02}")}
                })
            })
            .collect();

        let result = f.run(
            "base_create_records",
            json!({"tableId": "table_secondary", "records": records}),
        );

        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["createdCount"], 54);
        assert_eq!(result["data"]["recordIds"].as_array().unwrap().len(), 54);
        assert_eq!(f.document().tables[1].records.len(), 54);
        assert_eq!(
            f.host.changes.lock().unwrap().len(),
            1,
            "54 条记录必须在一次文件写入中完成"
        );
    }

    #[test]
    fn batch_create_proposal_contains_all_records_and_is_single_entry() {
        let f = Fixture::new(true);
        let path = crate::paths::to_forward_slashes(&f.root.join("记录.mcb").to_string_lossy());
        f.registry
            .permissions()
            .set_folder_permission(&path, AiPermissionLevel::Suggest)
            .unwrap();
        let result = f.run(
            "base_create_records",
            json!({
                "tableId": "table_secondary",
                "records": [
                    {"recordId": "proposal_a", "values": {"name": "甲"}},
                    {"recordId": "proposal_b", "values": {"name": "乙"}},
                    {"recordId": "proposal_c", "values": {"name": "丙"}}
                ]
            }),
        );
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["applied"], false);
        assert_eq!(result["data"]["createdCount"], 3);
        assert_eq!(f.raw(), SOURCE);
        assert!(f.host.changes.lock().unwrap().is_empty());
        let proposals = f.host.proposals.lock().unwrap();
        assert_eq!(proposals.len(), 1, "一个批次只能生成一个审批项");
        assert_eq!(proposals[0].previous_content.as_deref(), Some(SOURCE));
        let proposed = base::parse_base_document(proposals[0].content.as_deref().unwrap()).unwrap();
        let records = &proposed.tables[1].records;
        assert_eq!(records.len(), 3);
        assert_eq!(
            records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["proposal_a", "proposal_b", "proposal_c"]
        );
    }

    #[test]
    fn invalid_cells_bounds_references_and_stale_values_never_write() {
        let f = Fixture::new(false);
        for values in [
            json!({"amount":"12"}),
            json!({"group":"分组甲"}),
            json!({"progress":101}),
            json!({"missing":true}),
            json!({"time_range":["2026-09-12T12:00","2026-09-11T12:00"]}),
        ] {
            assert_eq!(
                f.run(
                    "base_update_record",
                    json!({"tableId":"table_main","recordId":"record_a","values":values})
                )["ok"],
                false
            );
        }
        for limit in [json!(0), json!(101), json!(1.5)] {
            assert_eq!(
                f.run(
                    "base_query_records",
                    json!({"tableId":"table_main","limit":limit})
                )["ok"],
                false
            );
        }
        assert_eq!(f.run("base_query_records",json!({"tableId":"table_main","filters":[{"fieldId":"missing","operator":"isEmpty"}]}))["ok"],false);
        assert_eq!(f.run("base_update_record",json!({"tableId":"table_main","recordId":"record_a","values":{"amount":12},"expectedValues":{"amount":9}}))["ok"],false);
        assert_eq!(f.raw(), SOURCE);
        assert!(f.host.changes.lock().unwrap().is_empty());
    }

    #[test]
    fn permissions_apply_before_read_and_write_and_proposals_keep_original_baseline() {
        let f = Fixture::new(true);
        let path = crate::paths::to_forward_slashes(&f.root.join("记录.mcb").to_string_lossy());
        f.registry
            .permissions()
            .set_folder_permission(&path, AiPermissionLevel::Invisible)
            .unwrap();
        assert_eq!(f.run("base_get_schema", json!({}))["ok"], false);
        f.registry
            .permissions()
            .set_folder_permission(&path, AiPermissionLevel::ReadOnly)
            .unwrap();
        assert_eq!(f.run("base_get_schema", json!({}))["ok"], true);
        let update = json!({"tableId":"table_main","recordId":"record_a","values":{"amount":12}});
        assert_eq!(f.run("base_update_record", update.clone())["ok"], false);
        assert!(f.host.proposals.lock().unwrap().is_empty());
        f.registry
            .permissions()
            .set_folder_permission(&path, AiPermissionLevel::Suggest)
            .unwrap();
        let result = f.run("base_update_record", update);
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["applied"], false);
        let proposals = f.host.proposals.lock().unwrap();
        assert_eq!(proposals[0].previous_content.as_deref(), Some(SOURCE));
        assert_eq!(
            base::parse_base_document(proposals[0].content.as_deref().unwrap())
                .unwrap()
                .tables[0]
                .records[0]
                .values["amount"],
            12
        );
        assert_eq!(f.raw(), SOURCE);
    }

    #[test]
    fn reference_updates_keep_urls_and_refuse_duplicates() {
        let f = Fixture::new(false);
        let mut document = f.document();
        document.tables[0].fields.push(base::BaseField {
            id: "refs".into(),
            name: "引用".into(),
            field_type: base::FieldType::Reference,
            ..Default::default()
        });
        std::fs::write(
            f.root.join("记录.mcb"),
            base::serialize_base_document(&document).unwrap(),
        )
        .unwrap();
        let urls = json!([
            "mochi://open?path=notes%2Fnote.md",
            "mochi://open?path=schedule&kind=task&item=task_1&label=练习",
            "mochi://open?path=记录.mcb&table=table_main&record=record_b"
        ]);
        let result = f.run(
            "base_update_record",
            json!({"tableId":"table_main","recordId":"record_a","values":{"refs":urls}}),
        );
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(f.document().tables[0].records[0].values["refs"], urls);
        let valid = f.raw();
        assert_eq!(f.run("base_update_record",json!({"tableId":"table_main","recordId":"record_a","values":{"refs":[urls[0],urls[0]]}}))["ok"],false);
        assert_eq!(f.raw(), valid);
    }
}
