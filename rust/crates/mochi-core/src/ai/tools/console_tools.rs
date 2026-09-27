//! 控制台操作由应用 UI 宿主串行执行。
use super::{host::ToolHost, ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::{models::AiToolDefinition, permission::AiToolAction};
use serde_json::json;
use std::sync::Arc;

pub const SPECS: &[(&str, &str, &str)] = &[
    (
        "console_state",
        "读取页面、标签、活动文档、分屏与弹窗状态",
        "get",
    ),
    ("console_navigate", "切换控制台模块", "open"),
    (
        "console_tabs",
        "打开、定位、聚焦、关闭文档标签和管理分屏",
        "open,focus,close,split,unsplit,focus_pane,locate",
    ),
    (
        "console_workspace",
        "查询和切换工作区；切换会结束当前工作区的 Agent 运行",
        "get,switch",
    ),
    (
        "inbox_manage",
        "查询、新建、编辑、删除收集条目，归档为笔记、转任务和批量处理",
        "list,create,update,delete,archive,to_task,batch",
    ),
    (
        "favorites_manage",
        "查询、添加、移除和排序收藏",
        "list,add,remove,reorder",
    ),
    (
        "recent_manage",
        "查询、移除或清空最近文档历史",
        "list,remove,clear",
    ),
    (
        "unread_manage",
        "查询、标记未读和按版本确认已读",
        "list,mark,read",
    ),
    (
        "templates_manage",
        "读取、新建、更新、删除、实例化模板和管理分组",
        "list,get,create,update,delete,instantiate,group_create,group_rename,group_delete,move",
    ),
    (
        "base_manage",
        "批量更新/删除记录和管理表、字段、视图结构",
        "get,batch",
    ),
    (
        "canvas_manage",
        "修改/删除卡片和图形，调整视口和批量布局",
        "get,batch",
    ),
    (
        "comments_manage",
        "创建、回复、更新、解决、重开和删除文档评论",
        "list,create,reply,update,resolve,reopen,delete",
    ),
    (
        "annotations_manage",
        "读取、新建、修改、删除 PDF 批注并定位页面",
        "list,create,update,delete,locate",
    ),
    (
        "calendar_navigate",
        "打开日程待办并定位日期、任务或日程",
        "open",
    ),
    (
        "desktop_cards_control",
        "显示、隐藏、聚焦卡片或打开卡片管理与编辑器",
        "open,edit,show,hide,focus",
    ),
    ("workflow_open", "打开工作流管理或定位工作流编辑器", "open"),
    (
        "ai_sessions_manage",
        "查询、创建、更新、删除会话与项目分组，导出会话",
        "list,get,create,update,delete,project_save,project_delete,export",
    ),
    (
        "agent_definitions_manage",
        "查询、校验、修改 Agent/Skill/MCP/Tool 定义，预览并应用随包更新",
        "list,get,validate,save,delete,updates,apply_update",
    ),
    (
        "english_manage",
        "查询词库、复习和学习统计，维护词条、文章与句子，执行评分",
        "get,add_word,import_words,remove_dictionary,add_article,add_sentence,grade,dictation",
    ),
    (
        "exam_manage",
        "读取和编辑试卷，答题、评分、重置并定位考试",
        "get,update,answer,submit,reset,open",
    ),
    (
        "pomodoro_manage",
        "查询、开始、暂停、继续和重置番茄钟及历史",
        "get,start,pause,resume,reset,history",
    ),
    (
        "plugins_manage",
        "查询、安装、启用、停用、卸载和打包原生插件",
        "list,install,enable,disable,uninstall,package,status",
    ),
    (
        "marketplace_manage",
        "加载市场目录、下载并安装包，查询后台操作结果",
        "list,refresh,install,status",
    ),
    (
        "mapped_folders_manage",
        "读取、新增、修改和删除映射目录配置",
        "list,add,update,delete",
    ),
    (
        "workspace_transfer",
        "导出工作区压缩包或检查、导入压缩包",
        "export,inspect,import,status",
    ),
    (
        "sync_manage",
        "查看同步状态和冲突，执行同步、拉取或推送并解决冲突",
        "status,fetch,pull,push,resolve",
    ),
    (
        "updates_manage",
        "查询应用更新状态、检查和下载安装更新",
        "get,check,download,install,status",
    ),
    (
        "notifications_manage",
        "查询通知、标记已读和删除通知",
        "list,read,delete,clear",
    ),
    ("home_stats", "查询首页统计与活动趋势", "get"),
];

pub fn read_only(name: &str, action: &str) -> bool {
    matches!(
        action,
        "get" | "list" | "status" | "history" | "validate" | "updates" | "preview" | "inspect"
    ) || name == "home_stats"
}

pub struct ConsoleToolExecutor(pub Arc<dyn ToolHost>);
impl ToolExecutor for ConsoleToolExecutor {
    fn handles(&self, name: &str) -> bool {
        self.0.supports_console() && SPECS.iter().any(|s| s.0 == name)
    }
    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let mut actions = vec![(AiToolAction::ReadFile, None)];
        let action = args.str_opt("action").unwrap_or("get");
        if !read_only(name, action) {
            actions.push((AiToolAction::ModifySettings, None));
            let ui_only = matches!(
                name,
                "console_navigate" | "console_tabs" | "calendar_navigate" | "workflow_open"
            ) || (name == "desktop_cards_control"
                && matches!(action, "open" | "edit" | "focus"))
                || (name == "exam_manage" && action == "open");
            if !ui_only {
                actions.push((AiToolAction::WriteFile, None));
            }
        }
        if matches!(
            name,
            "sync_manage" | "marketplace_manage" | "updates_manage"
        ) && !matches!(action, "get" | "status" | "install")
            || (name == "marketplace_manage" && action == "install")
        {
            actions.push((AiToolAction::NetworkAccess, None));
        }
        if name == "console_navigate" && args.value()["data"]["module"] == "marketplace" {
            actions.push((AiToolAction::NetworkAccess, None));
        }
        // 具体的目标路径和每条操作的写入/提案闸门，也在 UI 线程上、
        // 每条操作即将应用之前逐个复查。
        if matches!(
            action,
            "delete" | "uninstall" | "clear" | "group_delete" | "project_delete"
        ) || args.value()["operations"].as_array().is_some_and(|ops| {
            ops.iter().any(|o| {
                o["op"]
                    .as_str()
                    .or(o["type"].as_str())
                    .or(o["action"].as_str())
                    .is_some_and(|s| s.contains("delete"))
            })
        }) {
            actions.push((AiToolAction::DeleteFile, None));
        }
        if (name == "sync_manage" && !(action == "status" && args.str_opt("id").is_some()))
            || (name == "updates_manage" && action == "install")
        {
            actions.push((AiToolAction::ExecuteCommand, None));
        }
        actions
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if !args.value().is_object() {
            return Err("参数必须是对象".into());
        }
        let spec = SPECS.iter().find(|s| s.0 == name).ok_or("未知控制台工具")?;
        let action = args.str_opt("action").unwrap_or("get");
        if !spec.2.split(',').any(|a| a == action) {
            return Err(format!("action 应为 {}", spec.2));
        }
        if args.value().to_string().len() > 4 * 1024 * 1024 {
            return Err("单次请求超过 4 MiB".into());
        }
        self.0.console(name, args.value())
    }
}

pub fn definitions() -> Vec<AiToolDefinition> {
    SPECS.iter().map(|(name, description, actions)| serde_json::from_value(json!({"type":"function","function":{
        "name":name,"description":format!("{description}。参数按对应 Skill 的操作契约提供；变更后回读。"),
        "parameters":{"type":"object","properties":{
            "action":{"type":"string","enum":actions.split(',').collect::<Vec<_>>()},
            "id":{"type":"string","description":"读取结果中的稳定 ID"},
            "path":{"type":"string","description":"工作区内的目标路径；工作区切换和导入使用明确的绝对路径"},
            "revision":{"type":"string","description":"get 返回的版本，用于防止覆盖并发编辑"},
            "data":{"type":"object","description":"对应 Skill 中该 action 的参数"},
            "operations":{"type":"array","minItems":1,"maxItems":64,"items":{"type":"object"},"description":"按 Skill 契约提供的批量操作；返回逐项结果或原子提交结果"}
        },"required":["action"],"additionalProperties":false}
    }})).expect("console schema")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_deletions_and_network_actions_retain_permission_gates() {
        let e = ConsoleToolExecutor(Arc::new(super::super::host::HeadlessHost::new(".")));
        for op in [
            json!({"op":"record_delete"}),
            json!({"action":"delete"}),
            json!({"type":"delete"}),
        ] {
            let gates = e.required_actions(
                "base_manage",
                &ToolArgs::from_value(json!({"action":"batch","operations":[op]})),
            );
            assert!(gates.iter().any(|(a, _)| *a == AiToolAction::DeleteFile));
        }
        let gates = e.required_actions(
            "marketplace_manage",
            &ToolArgs::from_value(json!({"action":"list"})),
        );
        assert!(gates.iter().any(|(a, _)| *a == AiToolAction::NetworkAccess));
        let gates = e.required_actions(
            "updates_manage",
            &ToolArgs::from_value(json!({"action":"install"})),
        );
        assert!(gates
            .iter()
            .any(|(a, _)| *a == AiToolAction::ExecuteCommand));
        assert!(e
            .call(
                "base_manage",
                &ToolArgs::from_value(json!({"action":"misspelled"}))
            )
            .unwrap_err()
            .contains("action"));
    }
}
