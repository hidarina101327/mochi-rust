//! 定义工作流工具接口，并将 AI 的工具请求交给工作流执行器。
use super::{host::ToolHost, ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::{models::AiToolDefinition, permission::AiToolAction};
use crate::workflows::{self, Store, Workflow, WorkflowOperation};
use serde_json::json;
use std::sync::Arc;

pub const GUIDE: &str = include_str!("../../../assets/workflows/SKILL.md");

#[cfg(test)]
#[path = "workflow_tools_tests.rs"]
mod tests;

pub struct WorkflowToolExecutor {
    host: Arc<dyn ToolHost>,
}
impl WorkflowToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }
}
pub fn definitions() -> Vec<AiToolDefinition> {
    let id = json!({"type":"string","description":"工作流 ID"});
    [
        ("workflow_catalog","读取工作流 JSON 格式、节点目录与参数示例。创建前先调用。",json!({}),vec![]),
        ("workflow_list","列出当前工作区全局自动化工作流、版本和定时状态。",json!({}),vec![]),
        ("workflow_get","读取完整工作流 JSON，包含画布位置、节点、连线及修订号；可直接导出。id 或 ids 二选一，ids 支持一次读取最多 50 个。",json!({"id":id,"ids":{"type":"array","minItems":1,"maxItems":50,"items":{"type":"string"}}}),vec![]),
        ("workflow_save","创建或更新工作流。definition 使用 mochi.workflow v1。更新必传读取到的 revision，保存后按可信脚本运行；编辑不会关闭已启用的定时。",json!({"definition":{"type":"object"},"revision":{"type":"integer"}}),vec!["definition"]),
        ("workflow_import","导入 mochi.workflow v1 JSON 为新的可信工作流，不修改来源工作流。",json!({"json":{"type":"string"}}),vec!["json"]),
        ("workflow_validate","验证工作流 DAG、变量引用、节点、定时参数；不执行或写入。",json!({"definition":{"type":"object"}}),vec!["definition"]),
        ("workflow_run","将可信工作流直接加入执行队列，返回 runId；主程序或 workflow worker 运行时执行。不需要额外授权。",json!({"id":id,"input":{"type":"object"}}),vec!["id"]),
        ("workflow_history","读取最近 50 次执行或指定 runId 的节点输入/输出/错误/耗时与定义快照。",json!({"id":id,"runId":{"type":"string"}}),vec![]),
        ("workflow_cancel","请求停止某次运行，保留已完成节点的效果与记录，不自动回滚。",json!({"runId":{"type":"string"}}),vec!["runId"]),
        ("workflow_delete","删除一个工作流定义，需最新 revision；运行中拒绝删除，历史记录保留。",json!({"id":id,"revision":{"type":"integer"}}),vec!["id","revision"]),
        ("workflow_set_schedule","按最新 revision 启用或暂停定时。手动触发的工作流不能启用定时；暂停不取消已有运行。",json!({"id":id,"revision":{"type":"integer"},"enabled":{"type":"boolean"}}),vec!["id","revision","enabled"]),
        ("workflow_folders","管理工作流文件夹：list 列出；save 创建或重命名（name 必填，重命名传 id）；delete 删除文件夹但保留其中工作流。移动工作流用 workflow_batch 的 move 操作。",json!({"action":{"type":"string","enum":["list","save","delete"]},"id":{"type":"string"},"name":{"type":"string"}}),vec!["action"]),
        ("workflow_batch","在一个事务内按顺序批量创建、修改、删除、启停定时或移动工作流。1–50 项；任一失败全部回滚。save 新建省略 revision，更新必传；save 使 revision 加 1，后续同 ID 操作用新 revision。不会运行工作流。",json!({"operations":{"type":"array","minItems":1,"maxItems":50,"items":{"oneOf":[
            {"type":"object","properties":{"op":{"const":"save"},"definition":{"type":"object"},"revision":{"type":"integer"}},"required":["op","definition"],"additionalProperties":false},
            {"type":"object","properties":{"op":{"const":"delete"},"id":{"type":"string"},"revision":{"type":"integer"}},"required":["op","id","revision"],"additionalProperties":false},
            {"type":"object","properties":{"op":{"const":"set_schedule"},"id":{"type":"string"},"revision":{"type":"integer"},"enabled":{"type":"boolean"}},"required":["op","id","revision","enabled"],"additionalProperties":false},
            {"type":"object","properties":{"op":{"const":"move"},"id":{"type":"string"},"revision":{"type":"integer"},"folder_id":{"type":["string","null"]}},"required":["op","id","revision","folder_id"],"additionalProperties":false}
        ]}}}),vec!["operations"]),
    ].into_iter().map(|(name,description,properties,required)|AiToolDefinition::function(name,description,json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}))).collect()
}
impl ToolExecutor for WorkflowToolExecutor {
    fn handles(&self, name: &str) -> bool {
        matches!(
            name,
            "workflow_catalog"
                | "workflow_list"
                | "workflow_get"
                | "workflow_save"
                | "workflow_import"
                | "workflow_validate"
                | "workflow_run"
                | "workflow_history"
                | "workflow_cancel"
                | "workflow_delete"
                | "workflow_set_schedule"
                | "workflow_batch"
                | "workflow_folders"
        )
    }
    fn required_actions(&self, _: &str, _: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        // 工作流操作属于明确授权的自动化，不是 AI 对文件的直接编辑。
        Vec::new()
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if name == "workflow_catalog" {
            return Ok(workflows::catalog::describe());
        }
        if name == "workflow_validate" {
            let flow: Workflow =
                serde_json::from_value(args.get("definition").cloned().ok_or("缺少 definition")?)
                    .map_err(|e| e.to_string())?;
            return Ok(json!({"valid":true,"order":workflows::validate(&flow)?}));
        }
        let store = Store::open(self.host.workspace_root())?;
        match name {
            "workflow_list" => {
                Ok(json!({"workflows":store.summaries()?,"folders":store.folders()?}))
            }
            "workflow_get" => {
                if args.get("ids").is_some() {
                    if args.get("id").is_some() {
                        return Err("id 和 ids 只能提供一个".into());
                    }
                    let ids: Vec<String> = serde_json::from_value(args.get("ids").unwrap().clone())
                        .map_err(|e| e.to_string())?;
                    if ids.is_empty() || ids.len() > 50 {
                        return Err("ids 需要 1–50 个工作流 ID".into());
                    }
                    let flows = ids
                        .iter()
                        .map(|id| store.get(id))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(json!({"workflows":flows}))
                } else {
                    Ok(json!(store.get(args.str_required("id")?)?))
                }
            }
            "workflow_batch" => {
                let operations: Vec<WorkflowOperation> = serde_json::from_value(
                    args.get("operations").cloned().ok_or("缺少 operations")?,
                )
                .map_err(|e| e.to_string())?;
                Ok(json!({"applied":true,"results":store.apply_batch(&operations)?}))
            }
            "workflow_delete" | "workflow_set_schedule" => {
                let id = args.str_required("id")?.to_owned();
                let revision = args.i64_opt("revision").ok_or("缺少 revision")?;
                let op = if name == "workflow_delete" {
                    WorkflowOperation::Delete { id, revision }
                } else {
                    WorkflowOperation::SetSchedule {
                        id,
                        revision,
                        enabled: args.bool_opt("enabled").ok_or("缺少 enabled")?,
                    }
                };
                Ok(store.apply_batch(&[op])?.remove(0))
            }
            "workflow_folders" => match args.str_required("action")? {
                "list" => Ok(json!({"folders":store.folders()?})),
                "save" => Ok(json!(
                    store.save_folder(args.str_opt("id"), args.str_required("name")?)?
                )),
                "delete" => {
                    store.delete_folder(args.str_required("id")?)?;
                    Ok(json!({"deleted":true}))
                }
                _ => Err("未知文件夹操作".into()),
            },
            "workflow_save" => {
                let flow: Workflow = serde_json::from_value(
                    args.get("definition").cloned().ok_or("缺少 definition")?,
                )
                .map_err(|e| e.to_string())?;
                Ok(json!(store.save(&flow, args.i64_opt("revision"))?))
            }
            "workflow_import" => Ok(json!(store.import(args.str_required("json")?)?)),
            "workflow_run" => {
                let id = args.str_required("id")?;
                let run = store
                    .enqueue(
                        id,
                        args.get("input").cloned().unwrap_or(json!({})),
                        "agent",
                        None,
                    )?
                    .ok_or("此工作流已有运行中的任务")?;
                Ok(json!({"runId":run.id,"status":"queued"}))
            }
            "workflow_history" => {
                if let Some(run) = args.str_opt("runId") {
                    Ok(json!(store.run(run)?))
                } else {
                    Ok(json!({"runs":store.history(args.str_required("id")?)?}))
                }
            }
            "workflow_cancel" => {
                store.cancel(args.str_required("runId")?)?;
                Ok(json!({"cancellationRequested":true}))
            }
            _ => Err("未知工作流工具".into()),
        }
    }
}
