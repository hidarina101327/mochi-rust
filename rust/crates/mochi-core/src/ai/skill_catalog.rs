//! 内置能力元数据。正文只经 skill_load 返回。
use super::SkillDefinition;

pub fn bundled_skills() -> Vec<SkillDefinition> {
    let groups: &[(&str, &str, &[&str], &str)] = &[
        ("工作区文件", "查找、读取、创建、移动和删除工作区文件与文件夹", &["file_", "folder_", "content_search", "path_rename"], "先查询目标并读取当前内容。使用文件工具处理普通文件；局部文档修改使用文档编辑能力，多维表格和画布使用各自的工具。删除前明确目标，不扩大用户要求的范围。"),
        ("知识库", "列出、创建和切换知识库", &["kb_"], "先 kb_list 获取知识库名称与路径，再按用户目标创建或切换。"),
        ("文档编辑", "编辑文档片段、创建子文档、导出文档", &["document_range_", "subdocument_", "document_export", "file_read"], "读取目标内容后，用 document_range_propose_edit 携带行范围和原文哈希提交局部修改。待批准不等于已写入。子文档使用 subdocument_create；导出使用 document_export，并回报实际返回的文件路径。"),
        ("评论与批注", "读取和管理文档评论、回复、解决状态与 PDF 批注", &["comment_", "annotation_", "comments_manage", "annotations_manage"], "通过 comment_list 和 annotation_list 读取已有内容。管理评论与批注前，继续加载 console-api.md 获取参数契约。"),
        ("版本历史", "查询 Git 历史、比较差异、提交和恢复文件", &["git_"], "恢复或提交前先查看历史和差异，确认准确的文件与版本；只处理用户指定范围。提交成功以返回的提交 ID 为准。"),
        ("长期记忆", "搜索和保存用户明确表达的长期偏好", &["memory_"], "先搜索已有记忆以避免重复。仅保存用户明确表达的稳定信息，不保存凭据或推断敏感信息；遵循当前自动记忆开关，未成功写入不声称已记住。"),
        ("脚本执行", "查询脚本运行环境并执行自动化脚本或命令", &["script_", "shell_"], crate::script::GUIDE),
        ("联网研究规范", "联网搜索、核验网页来源和整理引用", &["web_search", "http_request"], "先搜索候选来源，再读取关键页面核验。区分来源事实与推断，回答中给出支持结论的链接。网络与命令权限由工具执行层检查。"),
        ("日程规划规范", "查看和管理日程待办：日程、时间块、任务、目标、愿望、重复安排与项目", &["agenda_"], "先 agenda_overview 了解安排，再用 agenda_list / agenda_get 取稳定 ID；所有修改用一次 agenda_batch 提交，给任务排时间用 agenda_plan_day。时间过去、时间块执行、任务全部完成都不会自动推进上层状态，只按用户明确说明操作。完成或删除任务返回 futureBlocks 时询问是否取消，默认保留。根据工具返回区分待批准变更与已应用变更，不自行调用内部审批接口。"),
        ("多维表格", "读取表结构、查询记录、创建与修改表格记录", &["base_get_", "base_query_", "base_create_", "base_update_", "base_manage"], "先用 base_get_schema 获取稳定表、字段和选项 ID，再查询记录。新增两条及以上记录必须一次 base_create_records，提供稳定唯一 recordId；不要用单条工具逐条试写。"),
        ("多维表格自动化", "编写、预览和单次运行多维表格自动化规则", &["base_automation_"], super::super::tools::base_automation_tools::GUIDE),
        ("画布", "读取画布、添加和移动对象卡片、绘图与写字", &["canvas_"], "先 canvas_get 读取当前内容与 revision，避让已有对象。绘图用 canvas_draw 的平滑路径与 texts，一次提交完整作品，不用脚本覆盖画布。保留无关对象，成功后再报告结果。"),
        ("工作流", "读取、修改、批量管理和运行工作流及定时任务", &["workflow_"], super::super::tools::workflow_tools::GUIDE),
        ("桌面卡片", "获取、修改与批量管理桌面卡片和页面配置", &["desktop_cards_"], super::super::tools::desktop_tools::GUIDE),
        ("墨池产品知识", "查询墨池功能、数据格式与使用方式", &["mochi_docs"], "先用 mochi_docs 查询对应功能说明，以实际说明和工具结果回答。"),
        ("墨池设置控制", "查询、修改、重置应用设置及打开设置界面", &["settings_"], "先 settings_list / settings_get 获取准确键名和取值，再更新指定设置；修改后回读验证。"),
        ("插件创作", "设计与编写墨池原生 v2 插件", &["file_read", "file_write", "folder_list", "mochi_docs"], "查询插件说明后生成 manifestVersion: 2 的 manifest.json、声明式 ui.json 与必要资源。最小化声明 permissions，不使用 Electron API 或任意 JavaScript 桥接。只写用户指定工程目录。安装使用系统管理 Skill 中的插件工具。"),
        ("控制台导航", "查看页面和标签、切换模块/工作区、分屏和定位日历", &["console_", "calendar_navigate"], "先读取 console_state；具体操作先加载 console-api.md。"),
        ("收集与内容管理", "管理收集箱、收藏、最近文档、未读与模板分组", &["inbox_manage", "favorites_manage", "recent_manage", "unread_manage", "templates_manage"], "先加载 console-api.md，再读取目标模块；批量变更检查逐项结果。"),
        ("AI配置与会话", "管理会话、Agent/Skills/MCP 定义、工具分配及安装包更新差异", &["ai_sessions_manage", "agent_definitions_manage"], "先加载 console-api.md。更新定义先读取正文和差异，保留用户自定义内容与菜单依赖。"),
        ("学习与专注", "驱动英语学习、试卷答题提交与番茄钟", &["english_manage", "exam_manage", "pomodoro_manage"], "先加载 console-api.md。答题使用真实题目 ID，提交后读取评分；计时与学习操作使用应用的同一状态。"),
        ("系统管理", "管理插件、市场、映射目录、工作区导入导出、Git同步和应用更新", &["plugins_manage", "marketplace_manage", "mapped_folders_manage", "workspace_transfer", "sync_manage", "updates_manage"], "先加载 console-api.md。后台任务返回 ID 后继续查询状态，仅 completed 表示已完成。"),
        ("通知与首页", "查询通知、标记已读、清理通知和读取首页统计", &["notifications_manage", "home_stats"], "先加载 console-api.md，再查询实际数据。"),
    ];
    groups
        .iter()
        .map(|(id, description, prefixes, body)| SkillDefinition {
            id: (*id).into(),
            name: (*id).into(),
            description: Some((*description).into()),
            aliases: vec![],
            tools: super::super::tools::definitions::names()
                .into_iter()
                .filter(|name| {
                    *name != "agenda_apply_change"
                        && prefixes.iter().any(|prefix| name.starts_with(prefix))
                })
                .map(str::to_owned)
                .collect(),
            mcps: vec![],
            body: (*body).into(),
            source_path: String::new(),
        })
        .collect()
}

pub fn document(skill: &SkillDefinition) -> String {
    format!(
        "---\nname: {}\ndescription: {}\ntools: [{}]\n---\n\n{}\n",
        skill.name,
        skill.description.as_deref().unwrap_or(""),
        skill.tools.join(", "),
        skill.body
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_console_action_has_a_scoped_reference() {
        for (name, _, actions) in crate::ai::tools::console_tools::SPECS {
            let guide = super::console_reference(&[name.to_string()]).unwrap();
            assert!(guide.contains(&format!("## {name}\n")));
            assert_eq!(guide.matches("\n## ").count(), 1);
            for action in actions.split(',') {
                assert!(guide.contains(action), "{name} missing {action}");
            }
        }
    }
    #[test]
    fn every_general_capability_has_a_discovery_route() {
        let skills = super::bundled_skills();
        for name in crate::ai::tools::definitions::names() {
            if name == "agenda_apply_change"
                || crate::ai::tools::skill_tools::INITIAL.contains(&name)
            {
                continue;
            }
            assert!(
                skills.iter().any(|s| s.tools.iter().any(|t| t == name)),
                "undiscoverable tool: {name}"
            );
        }
    }
}

/// 编译参考文档随可执行文件一起带版本。工作区里已有的指令文件
/// 绝不为了「公开最新 API 细节」而被覆盖。
pub fn console_reference(tools: &[String]) -> Option<String> {
    let source =
        include_str!("../../assets/console-skills/references/console-api.md").replace("\r\n", "\n");
    let mut parts = source.split("\n## ");
    let header = parts.next().unwrap_or_default();
    let sections = parts
        .filter(|part| {
            tools
                .iter()
                .any(|name| part.lines().next() == Some(name.as_str()))
        })
        .collect::<Vec<_>>();
    (!sections.is_empty()).then(|| format!("{header}\n## {}", sections.join("\n## ")))
}
