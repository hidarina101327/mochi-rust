//! Agent 创建工具与界面共用同一个规划器。持久启用只能通过界面完成。
use super::{
    host::{
        agent_path_basename, resolve_workspace_path, FileChange, PendingFileOperation,
        ResolveOptions, ToolHost,
    },
    ToolArgs, ToolExecutor, ToolOutcome,
};
use crate::ai::permission::AiToolAction;
use crate::{base, base_automation as automation};
use serde_json::{json, Value};
use std::sync::Arc;

const NAMES: &[&str] = &[
    "base_automation_list",
    "base_automation_put",
    "base_automation_delete",
    "base_automation_test",
    "base_automation_run",
];
#[cfg(test)]
mod tests;
pub struct BaseAutomationToolExecutor {
    host: Arc<dyn ToolHost>,
}
impl BaseAutomationToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }
    fn resolve(&self, args: &ToolArgs) -> Result<String, String> {
        let path = args
            .str_opt("path")
            .map(str::to_owned)
            .or_else(|| self.host.active_document().map(|d| d.path))
            .ok_or("请指定 .mcb 文件")?;
        if !path.to_ascii_lowercase().ends_with(".mcb") {
            return Err("仅支持 .mcb 文件".into());
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
    fn run(&self, name: &str, args: &ToolArgs) -> anyhow::Result<Value> {
        let path = self.resolve(args).map_err(anyhow::Error::msg)?;
        let meta = std::fs::metadata(&path)?;
        anyhow::ensure!(
            meta.is_file() && meta.len() <= automation::MAX_FILE_BYTES,
            "文件超过 10 MiB"
        );
        let original = std::fs::read_to_string(&path)?;
        let mut document = base::parse_base_document(&original)?;
        if name == "base_automation_list" {
            let mut views = vec![];
            for table in &document.tables {
                if args.str_opt("tableId").is_some_and(|id| table.id != id) {
                    continue;
                }
                for view in &table.views {
                    if args.str_opt("viewId").is_some_and(|id| view.id != id) {
                        continue;
                    }
                    let rules: Vec<_> = automation::rules(view)?
                        .into_iter()
                        .filter(|r| args.str_opt("ruleId").is_none_or(|id| r.id == id))
                        .collect();
                    views.push(json!({"tableId":table.id,"viewId":view.id,"viewName":view.name,"filters":view.filters,"rules":rules}));
                }
            }
            let mut state = automation::runtime(&document);
            state.runs.retain(|r| {
                args.str_opt("tableId").is_none_or(|id| r.table_id == id)
                    && args.str_opt("viewId").is_none_or(|id| r.view_id == id)
                    && args.str_opt("ruleId").is_none_or(|id| r.rule_id == id)
            });
            let result = json!({"path":path,"views":views,"runs":state.runs,"guide":GUIDE});
            anyhow::ensure!(
                result.to_string().len() <= 64 * 1024,
                "规则结果超过 64 KiB，请指定 tableId、viewId 或 ruleId 缩小范围"
            );
            return Ok(result);
        }
        let table_id = args.str_required("tableId").map_err(anyhow::Error::msg)?;
        let view_id = args.str_required("viewId").map_err(anyhow::Error::msg)?;
        let (_, view) = automation::scope(&document, table_id, view_id)?;
        let mut report = json!({});
        if name == "base_automation_delete" {
            automation::remove(
                &mut document,
                table_id,
                view_id,
                args.str_required("ruleId").map_err(anyhow::Error::msg)?,
            )?;
        } else {
            anyhow::ensure!(
                !(args.get("rule").is_some() && args.get("ruleId").is_some()),
                "rule 与 ruleId 请只提供一个"
            );
            let mut rule: automation::Rule = if let Some(value) = args.get("rule") {
                serde_json::from_value(value.clone())?
            } else {
                let id = args.str_required("ruleId").map_err(anyhow::Error::msg)?;
                automation::rules(view)?
                    .into_iter()
                    .find(|r| r.id == id)
                    .ok_or_else(|| anyhow::anyhow!("自动化不存在"))?
            };
            if name == "base_automation_put" {
                anyhow::ensure!(args.get("rule").is_some(), "put 需要完整 rule 对象");
                // 不能通过 Agent 或导入的文档授予持续执行权限。
                rule.enabled = false;
                automation::put(&mut document, table_id, view_id, rule.clone())?;
                report = json!({"rule":rule,"activation":"draft; 用户在视图 > 自动化中确认启用"});
            } else {
                let ids: Vec<String> = serde_json::from_value(
                    args.get("recordIds")
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("需要明确指定 recordIds"))?,
                )?;
                let (planned, run) = if name == "base_automation_test" {
                    automation::preview(
                        &document,
                        table_id,
                        view_id,
                        &rule,
                        &ids,
                        automation::now_ms(),
                    )?
                } else {
                    automation::manual_run(
                        &document,
                        table_id,
                        view_id,
                        &rule,
                        &ids,
                        automation::now_ms(),
                    )?
                };
                let mut changes = vec![];
                for table in &planned.tables {
                    let old = document.tables.iter().find(|t| t.id == table.id).unwrap();
                    let index: std::collections::BTreeMap<_, _> =
                        old.records.iter().map(|r| (&r.id, r)).collect();
                    for record in &table.records {
                        if index
                            .get(&record.id)
                            .is_none_or(|r| r.values != record.values)
                        {
                            if changes.len() < 20 {
                                changes.push(json!({"tableId":table.id,"recordId":record.id,"before":index.get(&record.id).map(|r| &r.values),"after":record.values}));
                            }
                        }
                    }
                }
                report = json!({"run":run,"previewChanges":changes,"previewLimit":20,"dryRun":name == "base_automation_test"});
                if report.to_string().len() > 64 * 1024 {
                    report.as_object_mut().unwrap().remove("previewChanges");
                    report["previewOmitted"] =
                        json!("字段内容过大，仅返回数量；可用 base_query_records 按字段查看");
                }
                if name == "base_automation_test" {
                    return Ok(report);
                }
                document = planned;
            }
        }
        let content = base::serialize_base_document(&document)?;
        anyhow::ensure!(
            content.len() as u64 <= automation::MAX_FILE_BYTES,
            "文件超过 10 MiB"
        );
        if report.to_string().len() > 64 * 1024 {
            let id = report["rule"]["id"].clone();
            report = json!({"ruleId":id,"enabled":false,"detailOmitted":true,"activation":"draft; 用户在视图 > 自动化中确认启用"});
        }
        if self.host.should_propose(&path) {
            let proposal = self
                .host
                .submit_proposal(PendingFileOperation {
                    kind: "overwrite".into(),
                    path: path.clone(),
                    title: agent_path_basename(&path),
                    new_path: None,
                    content: Some(content),
                    previous_content: Some(original),
                    summary: format!("多维表格自动化：{name}"),
                    entries: vec![],
                    reason: "destructive".into(),
                })
                .map_err(anyhow::Error::msg)?;
            if let Some(object) = proposal.as_object() {
                report.as_object_mut().unwrap().extend(object.clone());
            }
            report["applied"] = json!(false);
        } else {
            let _file_lock = automation::FileLock::acquire(std::path::Path::new(&path))?;
            anyhow::ensure!(
                std::fs::read_to_string(&path)? == original,
                "文件已改变，请重新读取并测试"
            );
            crate::files::FileService::new().write_file_safe(&path, &content)?;
            self.host
                .notify_change(FileChange::Write { path: path.clone() });
            report["applied"] = json!(true);
        }
        report["path"] = json!(path);
        Ok(report)
    }
}
impl ToolExecutor for BaseAutomationToolExecutor {
    fn handles(&self, name: &str) -> bool {
        NAMES.contains(&name)
    }
    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let path = self.resolve(args).ok();
        let mut actions = vec![(AiToolAction::ReadFile, path.clone())];
        if !matches!(name, "base_automation_list" | "base_automation_test") {
            actions.push((AiToolAction::WriteFile, path));
        }
        actions
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if !self.handles(name) {
            return Err("未知自动化工具".into());
        }
        let _guard = automation::MUTATION_GATE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.run(name, args).map_err(|e| e.to_string())
    }
}

pub const GUIDE: &str = "先用 base_get_schema 获取表/视图/字段的稳定 ID，再 base_automation_list 读取已有规则。规则位于视图内，视图筛选与 conditions 为 AND。触发器 manual、recordCreated、recordUpdated{fieldIds}、enterView（首次满足条件）、interval{minutes,startAt UTC毫秒}。动作 updateRecord{values}、createRecord{tableId,values}；values 每个字段使用 {type:literal,value:原生JSON值} / {type:field,fieldId} / {type:now} / {type:recordId}，下拉字段使用选项ID。base_automation_test 提供 rule 或 ruleId 与明确的 recordIds，只预览不写入；put 保存完整规则并强制为草稿，用户必须在界面确认持续启用。run 仅执行一次且受写入审批约束，不启用后台执行。持续运行要求本地应用及工作区打开；无历史回填，不级联触发；每次最多200条记录/1000个动作，失败整体回滚。不能用 file_write 绕过用户启用或审批。";
