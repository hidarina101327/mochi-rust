//! Schema 由 extract-tool-definitions.js 提取，再合并 native-tool-definitions.json；不要手抄定义。

use std::collections::BTreeMap;
use std::sync::LazyLock;

use super::super::models::AiToolDefinition;

const RAW: &str = include_str!("../../../assets/tool-definitions.json");
const CANVAS_RAW: &str = r#"[
 {"type":"function","function":{"name":"canvas_get","description":"读取 .mcanvas 画布的文字、笔迹、卡片、内容 revision 和建议绘图区域。画画或写字前先读取，再使用 canvas_draw 直接绘制。path 省略时使用本轮用户打开的画布。","parameters":{"type":"object","properties":{"path":{"type":"string","description":"画布路径，可省略"}},"additionalProperties":false}}},
 {"type":"function","function":{"name":"canvas_add_cards","description":"向已有画布添加文档、Base 记录、日程或 PDF 批注的 Mochi 对象 URL。会校验引用和画布格式后保存。","parameters":{"type":"object","properties":{"path":{"type":"string"},"references":{"type":"array","items":{"type":"string"},"minItems":1}},"required":["path","references"],"additionalProperties":false}}},
 {"type":"function","function":{"name":"canvas_move_card","description":"移动画布中的一个卡片到指定坐标。","parameters":{"type":"object","properties":{"path":{"type":"string"},"cardId":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"}},"required":["path","cardId","x","y"],"additionalProperties":false}}}
]"#;
const RETIRED_INLINE_BLOCK_TOOLS: &[&str] = &[
    "document_blocks_list",
    "document_block_read",
    "document_block_propose_edit",
];

static ALL: LazyLock<Vec<AiToolDefinition>> = LazyLock::new(|| {
    let mut tools = serde_json::from_str::<Vec<AiToolDefinition>>(RAW)
        .expect("内置工具定义 JSON 损坏——assets/tool-definitions.json 需重新生成")
        .into_iter()
        // 生成的资源中保留旧的数据结构，是为了兼容较早的安装版本；
        // Mochi 现行的工具目录和 Agent 可选能力里绝不含它们。
        .filter(|tool| !RETIRED_INLINE_BLOCK_TOOLS.contains(&tool.function.name.as_str()))
        .collect::<Vec<_>>();
    tools.extend(
        serde_json::from_str::<Vec<AiToolDefinition>>(CANVAS_RAW).expect("内置画布工具定义损坏"),
    );
    tools.extend(
        serde_json::from_str::<Vec<AiToolDefinition>>(include_str!(
            "../../../assets/canvas-drawing-tools.json"
        ))
        .expect("内置绘图工具定义损坏"),
    );
    tools.extend(
        serde_json::from_str::<Vec<AiToolDefinition>>(include_str!(
            "../../../assets/base-automation-tools.json"
        ))
        .expect("自动化工具定义损坏"),
    );
    tools.extend(
        serde_json::from_str::<Vec<AiToolDefinition>>(include_str!(
            "../../../assets/script-tools.json"
        ))
        .expect("脚本工具定义损坏"),
    );
    tools.extend(super::workflow_tools::definitions());
    tools.extend(super::desktop_tools::definitions());
    tools.extend(super::skill_tools::definitions());
    tools.extend(super::console_tools::definitions());
    tools
});

/// 全部内置工具定义，顺序与 TS 的 `mochiTools` 一致（模型看到的顺序会影响选择倾向）。
pub fn all() -> &'static [AiToolDefinition] {
    &ALL
}

pub fn count() -> usize {
    ALL.len()
}

pub fn find(name: &str) -> Option<&'static AiToolDefinition> {
    ALL.iter().find(|t| t.function.name == name)
}

pub fn names() -> Vec<&'static str> {
    ALL.iter().map(|t| t.function.name.as_str()).collect()
}

/// 按 Agent 的工具选择过滤，并套用用户在 `Tools/<name>/TOOL.md` 里改写的描述。
///
/// 选择清单里出现的未知工具名会被忽略——Agent 文档是用户可编辑的，
/// 写错一个名字不该让整个工具集失效。
pub fn resolve(
    selection: &super::super::agent_config::Selection,
    overrides: &BTreeMap<String, String>,
) -> Vec<AiToolDefinition> {
    ALL.iter()
        .filter(|t| selection.contains(&t.function.name))
        .map(|t| {
            let mut tool = t.clone();
            if let Some(desc) = overrides.get(&tool.function.name) {
                tool.function.description = desc.clone();
            }
            tool
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent_config::Selection;

    #[test]
    fn embedded_definitions_parse() {
        assert_eq!(
            count(),
            77 + crate::ai::tools::console_tools::SPECS.len(),
            "工具数变了——同步 rust/crates/mochi-core/assets/tool-definitions.json"
        );
        assert!(ALL.iter().all(|t| t.kind == "function"));
        assert!(ALL.iter().all(|t| !t.function.name.is_empty()));
        assert!(ALL.iter().all(|t| !t.function.description.is_empty()));
    }

    #[test]
    fn every_tool_has_an_object_schema() {
        for tool in all() {
            let p = &tool.function.parameters;
            assert_eq!(
                p.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "{} 的 parameters 不是 object",
                tool.function.name
            );
            assert!(
                p.get("properties").is_some(),
                "{} 缺 properties",
                tool.function.name
            );
        }
    }

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in names() {
            assert!(seen.insert(name), "工具名重复: {name}");
        }
    }

    /// C# 版缺的那些能力，这里必须都在。
    #[test]
    fn covers_the_capabilities_csharp_missed() {
        for name in [
            "git_commit",
            "git_history",
            "git_file_diff",
            "git_restore_file",
            "document_export",
            "document_range_propose_edit",
            "path_rename",
            "file_delete",
            "folder_delete",
            "kb_create",
            "kb_select",
            "agenda_overview",
            "agenda_list",
            "agenda_get",
            "agenda_find_free_time",
            "agenda_history",
            "agenda_batch",
            "agenda_plan_day",
            "agenda_apply_change",
            "agent_plan",
            "mochi_docs",
            "shell_run",
            "settings_reset",
            "settings_open_ui",
        ] {
            assert!(find(name).is_some(), "缺工具 {name}");
        }
    }

    /// C# 自创的名字，真实工具集里没有——不该出现。
    #[test]
    fn does_not_contain_the_invented_csharp_tool() {
        assert!(
            find("schedule_delete").is_none(),
            "schedule_delete 是 C# 自创的，真实工具集里没有"
        );
    }

    #[test]
    fn resolve_filters_by_selection() {
        let picked = resolve(
            &Selection::List(vec!["file_read".into(), "不存在的工具".into()]),
            &BTreeMap::new(),
        );
        assert_eq!(picked.len(), 1, "未知工具名应被忽略而不是报错");
        assert_eq!(picked[0].function.name, "file_read");

        assert_eq!(resolve(&Selection::All, &BTreeMap::new()).len(), count());
    }

    #[test]
    fn resolve_applies_description_overrides() {
        let mut overrides = BTreeMap::new();
        overrides.insert("file_read".to_string(), "用户改写过的说明".to_string());
        let picked = resolve(&Selection::List(vec!["file_read".into()]), &overrides);
        assert_eq!(picked[0].function.description, "用户改写过的说明");
        // 未被覆盖的保持原样
        let untouched = resolve(&Selection::List(vec!["file_write".into()]), &overrides);
        assert_ne!(untouched[0].function.description, "用户改写过的说明");
    }
}
