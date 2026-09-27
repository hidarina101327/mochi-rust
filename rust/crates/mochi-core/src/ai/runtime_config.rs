//! 按请求选择 Agent/Skill，与 aiPrompts.ts 对齐。读取这份配置
//! 绝不改工作区文件，也不授予任何运行时权限。
use super::agent_config::{
    AgentConfigService, AgentDefinition, McpServerDefinition, Selection, SkillDefinition,
};
use std::{collections::HashSet, path::Path};

const BASE_BATCH_GUIDANCE: &str = "# 多维表格批量写入\n\
- 用户要求新增 2 条及以上记录时，必须调用一次 base_create_records，把全部 records 放在同一批次中；不要先用 base_create_record 测试一条，也不要拆成多个单条写入。\n\
- 为每条记录提供稳定、唯一的 recordId，便于安全重试。批量工具会整体校验，并只写入一次或生成一个审批项。";

pub struct RuntimeConfig {
    pub progressive: bool,
    pub agent_id: Option<String>,
    pub agent_name: String,
    pub system_prompt: String,
    pub tools: Selection,
    pub mcps: Vec<McpServerDefinition>,
    pub skills: Vec<String>,
    pub model: Option<String>,
    pub max_turns: Option<usize>,
}

pub fn load(root: &Path, id: Option<&str>, request: &str) -> RuntimeConfig {
    let service = AgentConfigService::new(root);
    let agent = service.find_agent(id);
    if agent.as_ref().is_none_or(|a| {
        a.id == super::agent_config::GENERAL_ASSISTANT_ID || a.skill_routing == "progressive"
    }) {
        let skills = service.load_available_skills();
        let catalog = skills
            .iter()
            .map(|s| {
                format!(
                    "- {}：{}",
                    s.id,
                    s.description.as_deref().unwrap_or("按需加载查看说明")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mcps = service
            .load_mcp_servers()
            .into_iter()
            .filter(|m| {
                m.enabled
                    && !m.is_builtin()
                    && agent.as_ref().is_none_or(|a| {
                        a.id == super::agent_config::GENERAL_ASSISTANT_ID
                            || a.mcps.contains(&m.id)
                            || a.mcps.contains(&m.name)
                    })
            })
            .collect();
        let planning = if agent
            .as_ref()
            .is_some_and(|a| a.execution_mode == "plan-and-execute")
        {
            "\n执行模式：先调用 agent_plan 列出步骤，完成每步后更新计划；简单单步请求可直接处理。"
        } else {
            ""
        };
        return RuntimeConfig {
            progressive: true,
            agent_id: agent.as_ref().map(|a| a.id.clone()),
            agent_name: agent.as_ref().map_or_else(|| "通用助手".into(), |a| a.name.clone()),
            system_prompt: format!("{}\n\n# 按需使用能力\n你可以通过 Skills 管理墨池与工作区。以下目录仅表示有这些能力，不表示工具已经加载。首次只有基本上下文、计划和 Skill 工具；当用户提问涉及某项能力或任务需要时，调用 skill_load，读取返回的指引后再使用下一轮提供的工具。需要更多细节时只加载相关参考文件；跨领域任务可以依次加载多个 Skill。不要一次加载全部能力。Skill 不改变权限或审批边界，未执行成功不声称完成。后续用户消息仍可按需重新加载。\n\n# 能力目录\n{}{}", agent.as_ref().map_or("你是墨池通用助手，根据用户目标查证并完成工作。", |a| a.system_prompt.trim()), catalog, planning),
            tools: agent.as_ref().map_or_else(|| super::agent_config::normalize_agent_tools(super::agent_config::GENERAL_ASSISTANT_ID, vec![]), |a| a.tools.clone()),
            skills: vec![],
            mcps,
            model: agent.as_ref().and_then(|a| a.model.clone()).filter(|m| !m.trim().is_empty()),
            max_turns: agent.as_ref().and_then(|a| a.max_turns).map(|n| n.clamp(1,120) as usize),
        };
    }
    let agent = agent.expect("general fallback handled above");
    let available_skills = service.load_skills();
    let skills = select_skills(&available_skills, &agent, request);
    let mut prompt = format!(
        "{}\n\n{}",
        agent.system_prompt.trim(),
        crate::product_knowledge::brief()
    );
    if agent.execution_mode == "plan-and-execute" {
        prompt.push_str("\n\n# 执行模式：先计划后执行\n- 接到任务后，先调用 agent_plan 列出 3-7 步计划（第一步 in_progress，其余 pending），再开始执行。\n- 每完成一步立即调用 agent_plan 更新全量步骤状态（done / in_progress / skipped）。\n- 调整计划后再继续；简单单步请求可直接回答。");
    }
    if !skills.is_empty() {
        prompt.push_str("\n\n# Active Skills\nSkills are execution protocols selected by the active Agent. Follow their input contract, workflow, tool orchestration, output format and constraints. Skills do not grant permissions or add unadvertised tools.\n");
        for skill in &skills {
            prompt.push_str(&format!("\n## Skill: {}\n- id: {}\n- description: {}\n- aliases: {}\n- tools: {}\n- mcps: {}\n{}\n",skill.name,skill.id,skill.description.as_deref().unwrap_or(""),skill.aliases.join(", "),skill.tools.join(", "),skill.mcps.join(", "),skill.body));
        }
    }
    if agent.tools.contains("base_create_records") {
        prompt.push_str("\n\n");
        prompt.push_str(BASE_BATCH_GUIDANCE);
    }
    let mut tools = agent.tools.clone();
    // 老工作区可能存有一份定制过的、只允许新建文件的 Skill 快照。保留原样，
    // 工具对外宣称的仍是当前的批量语义。
    if agent.tools.contains("workflow_batch") {
        prompt.push_str("\n\n# 工作流批量管理\n先用 workflow_list / workflow_get 读取 ID、定义和 revision。多项创建、修改、删除、定时启停、移动优先一次 workflow_batch；整批失败不应用。save 后 revision 加 1。只在用户要求执行时调用 workflow_run；queued 不是执行成功。\n");
    }
    if agent.tools.contains("desktop_cards_batch") {
        prompt.push_str("\n\n# 桌面卡片批量管理\n先用 desktop_cards_get 读取 config 和 revision，新建前用 desktop_cards_templates。多张卡片的创建、字段修改、删除优先一次 desktop_cards_batch；changes 合并字段、pages 整体替换，保留无关内容。成功返回新 revision；冲突或超时先重读，不盲目重放。\n");
    }
    if agent.tools.contains("script_run") {
        prompt.push_str("\n\n");
        prompt.push_str(crate::script::GUIDE);
    }
    if agent.tools.contains("base_automation_put") {
        prompt.push_str("\n\n# 多维表格自动化\n");
        prompt.push_str(super::tools::base_automation_tools::GUIDE);
    }
    if agent.execution_mode == "plan-and-execute" {
        if let Selection::List(names) = &mut tools {
            if !names.iter().any(|n| n == "agent_plan") {
                names.push("agent_plan".into());
            }
        }
    }
    let mcps = service
        .load_mcp_servers()
        .into_iter()
        .filter(|m| {
            m.enabled
                && !m.is_builtin()
                && (agent.mcps.contains(&m.id) || agent.mcps.contains(&m.name))
        })
        .collect();
    RuntimeConfig {
        progressive: false,
        agent_id: Some(agent.id),
        agent_name: agent.name,
        system_prompt: prompt,
        tools,
        mcps,
        skills: skills.iter().map(|s| s.name.clone()).collect(),
        model: agent.model.filter(|m| !m.trim().is_empty()),
        max_turns: agent.max_turns.map(|n| n.clamp(1, 120) as usize),
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| {
            if "`*_#[]()>~{}|/\\:：,，.。;；!?！？\"'“”‘’".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn find_skill<'a>(skills: &'a [SkillDefinition], name: &str) -> Option<&'a SkillDefinition> {
    let query = normalize(name);
    skills.iter().find(|s| {
        normalize(&s.id) == query
            || normalize(&s.name) == query
            || s.aliases.iter().any(|a| normalize(a) == query)
    })
}
fn score(skill: &SkillDefinition, request: &str) -> usize {
    let query = normalize(request);
    if query.is_empty() {
        return 0;
    }
    let routing = normalize(&format!(
        "{} {} {} {}",
        skill.id,
        skill.name,
        skill.description.as_deref().unwrap_or(""),
        skill.aliases.join(" ")
    ));
    let aliases = std::iter::once(&skill.id)
        .chain(std::iter::once(&skill.name))
        .chain(skill.aliases.iter())
        .map(|a| normalize(a))
        .filter(|a| !a.is_empty() && query.contains(a))
        .count()
        * 12;
    let latin = |s: &str| {
        s.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .filter(|w| w.len() >= 2 && w.starts_with(|c: char| c.is_ascii_alphanumeric()))
            .map(str::to_owned)
            .collect::<HashSet<_>>()
    };
    let overlap = latin(&query).intersection(&latin(&routing)).count().min(8);
    let cjk = routing
        .chars()
        .filter(|c| matches!(*c, '\u{3400}'..='\u{9fff}'))
        .collect::<Vec<_>>();
    let bigrams = cjk
        .windows(2)
        .map(|w| w.iter().collect::<String>())
        .collect::<HashSet<_>>();
    aliases
        + overlap
        + bigrams
            .iter()
            .filter(|s| query.contains(s.as_str()))
            .count()
            .min(8)
}
pub fn select_skills<'a>(
    skills: &'a [SkillDefinition],
    agent: &AgentDefinition,
    request: &str,
) -> Vec<&'a SkillDefinition> {
    if agent.skill_routing == "none" {
        return vec![];
    }
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for name in &agent.skills {
        if let Some(skill) = find_skill(skills, name) {
            if seen.insert(skill.id.clone()) {
                selected.push(skill);
            }
        }
    }
    if agent.skill_routing == "light" && !request.trim().is_empty() {
        let candidates =
            if agent.routable_skills.is_all() || agent.routable_skills.as_list().is_empty() {
                skills.iter().collect::<Vec<_>>()
            } else {
                agent
                    .routable_skills
                    .as_list()
                    .iter()
                    .filter_map(|n| find_skill(skills, n))
                    .collect()
            };
        let mut ranked = candidates
            .into_iter()
            .filter(|s| {
                !seen.contains(&s.id) && s.description.as_ref().is_some_and(|d| !d.is_empty())
            })
            .map(|s| (score(s, request), s))
            .filter(|(n, _)| *n >= 4)
            .collect::<Vec<_>>();
        ranked.sort_by(|(a, x), (b, y)| {
            b.cmp(a)
                .then_with(|| crate::files::locale_compare(&x.name, &y.name))
        });
        for (_, skill) in ranked.into_iter().take(3) {
            if seen.insert(skill.id.clone()) {
                selected.push(skill);
            }
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mochi-agent-runtime-{}",
                crate::paths::random_base36(12)
            ));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn write(&self, relative: &str, body: &str) {
            let path = self.0.join("Agent配置").join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn runtime_loading_is_readonly_and_without_agent_uses_builtin_tools() {
        let f = Fixture::new();
        let plan = load(&f.0, None, "hello");
        assert!(plan.progressive);
        assert!(plan.tools.contains("skill_load"));
        assert!(plan.mcps.is_empty());
        assert!(!plan.system_prompt.contains(BASE_BATCH_GUIDANCE));
        assert!(!f.0.join("Agent配置").exists());
    }
    #[test]
    fn shared_seeds_are_create_only_and_provide_real_default_agents() {
        let f = Fixture::new();
        f.write(
            "Agents/通用助手.md",
            "---\nname: 我的助手\n---\n用户自定义正文",
        );
        let service = AgentConfigService::new(&f.0);
        let seeds: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("../../assets/agent-seeds.json")).unwrap();
        let native_tool_names = super::super::tools::definitions::names()
            .into_iter()
            .collect::<Vec<_>>();
        let mut expected = seeds
            .keys()
            .filter(|path| !path.starts_with("Agents/") || *path == "Agents/通用助手.md")
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        expected.extend(
            native_tool_names
                .iter()
                .map(|name| format!("Tools/{name}/TOOL.md")),
        );
        expected.extend(
            super::super::agent_config::skill_catalog::bundled_skills()
                .iter()
                .map(|s| format!("Skills/{}/SKILL.md", s.id)),
        );
        // 用户自有的 Agent 保留。重叠的种子路径只计一次。
        assert_eq!(service.ensure_seeds().unwrap(), expected.len() - 1);
        for name in native_tool_names {
            assert!(f
                .0
                .join(format!("Agent配置/Tools/{name}/TOOL.md"))
                .is_file());
        }
        assert_eq!(service.ensure_seeds().unwrap(), 0);
        let general = service.find_agent(None).unwrap();
        assert_eq!(general.name, "我的助手");
        assert_eq!(general.system_prompt, "用户自定义正文");
        assert_eq!(service.load_agents().len(), 1);
        assert!(!service.load_skills().is_empty());
    }

    #[test]
    fn seeded_general_assistant_keeps_agenda_tools_without_external_internal_mcp() {
        let f = Fixture::new();
        let service = AgentConfigService::new(&f.0);
        service.ensure_seeds().unwrap();
        let plan = load(&f.0, None, "查看今天的日程");
        assert!(plan.tools.contains("agenda_overview"));
        assert!(plan.tools.contains("agenda_batch"));
        assert!(plan.tools.contains("script_environment"));
        assert!(plan.tools.contains("script_run"));
        assert!(!plan.system_prompt.contains(crate::script::GUIDE));
        assert!(plan.mcps.iter().all(|mcp| !mcp.is_builtin()));
        assert!(!plan.mcps.iter().any(|mcp| mcp.id == "schedule-local"));
    }

    #[test]
    fn explicit_agent_changes_prompt_tools_skills_model_and_iteration_limit() {
        let f = Fixture::new();
        f.write("Agents/只读.md","---\nname: 只读助手\ntools: [file_read]\nskills: [步骤别名]\nskillRouting: explicit\nmcps: [safe]\nmodel: test-model\nmaxTurns: 3\nexecutionMode: plan-and-execute\n---\n仅做审阅。");
        f.write(
            "Skills/步骤/SKILL.md",
            "---\nname: 步骤\naliases: [步骤别名]\ndescription: 审阅流程\n---\n先阅读，再列问题。",
        );
        f.write(
            "MCPs/safe/MCP.md",
            "---\nname: safe\ntransport: stdio\ncommand: example\nenabled: false\n---\n",
        );
        let plan = load(&f.0, Some("只读"), "审阅");
        assert_eq!(plan.agent_name, "只读助手");
        assert!(plan.tools.contains("file_read"));
        assert!(plan.tools.contains("agent_plan"));
        assert!(!plan.tools.contains("file_write"));
        assert!(!plan.tools.contains("script_run"));
        assert!(!plan.system_prompt.contains(crate::script::GUIDE));
        assert!(plan.system_prompt.contains("先阅读，再列问题"));
        assert_eq!(plan.skills, ["步骤"]);
        assert!(plan.mcps.is_empty());
        assert_eq!(plan.model.as_deref(), Some("test-model"));
        assert_eq!(plan.max_turns, Some(3));
    }
    #[test]
    fn light_routing_is_bounded_and_none_disables_explicit_skills() {
        let f = Fixture::new();
        f.write(
            "Agents/路由.md",
            "---\nskillRouting: light\nroutableSkills: ['*']\n---\n助手",
        );
        for name in ["主题甲", "主题乙", "主题丙", "主题丁"] {
            f.write(
                &format!("Skills/{name}/SKILL.md"),
                &format!("---\nname: {name}\ndescription: 匹配主题\n---\n{name}流程"),
            );
        }
        let plan = load(&f.0, Some("路由"), "主题甲 主题乙 主题丙 主题丁");
        assert_eq!(plan.skills.len(), 3);
        f.write(
            "Agents/路由.md",
            "---\nskillRouting: none\nskills: [主题甲]\n---\n助手",
        );
        assert!(load(&f.0, Some("路由"), "主题甲").skills.is_empty());
    }

    #[test]
    fn mcb_skill_routes_for_existing_general_agents_and_preserves_custom_allowlists() {
        let f = Fixture::new();
        f.write("Agents/通用助手.md", "---\nname: 通用助手\ntools: [file_read]\nskillRouting: light\nroutableSkills: [日报格式]\n---\n已有配置");
        f.write("Agents/自定义.md", "---\nname: 自定义\ntools: [file_read]\nskillRouting: light\nroutableSkills: [日报格式]\n---\n自定义配置");
        AgentConfigService::new(&f.0).ensure_seeds().unwrap();
        let plan = load(
            &f.0,
            Some("通用助手"),
            "把这条的进度改为 50\n当前文档：.mcb 多维表格",
        );
        assert!(plan.skills.is_empty());
        assert!(plan.system_prompt.contains("多维表格"));
        assert!(!plan.system_prompt.contains("先用 base_get_schema"));
        assert!(plan.tools.contains("base_update_record"));
        assert!(plan.tools.contains("base_create_records"));
        assert!(!plan.system_prompt.contains(BASE_BATCH_GUIDANCE));
        let custom = load(&f.0, Some("自定义"), ".mcb 多维表格");
        assert!(!custom.skills.iter().any(|skill| skill == "多维表格"));
        assert!(!custom.tools.contains("base_update_record"));
        assert!(std::fs::read_to_string(
            AgentConfigService::new(&f.0)
                .root()
                .join("Agents/通用助手.md")
        )
        .unwrap()
        .contains("已有配置"));
    }

    #[test]
    fn management_skills_route_for_legacy_general_agents_without_overwriting_custom_definitions() {
        let f = Fixture::new();
        let legacy = "---\nname: 通用助手\ntools: [file_read]\nroutableSkills: [日报格式]\n---\n用户自己的提示词";
        f.write("Agents/通用助手.md", legacy);
        f.write(
            "Agents/自定义.md",
            "---\ntools: [file_read]\nskillRouting: light\nroutableSkills: [日报格式]\n---\n自定义",
        );
        let service = AgentConfigService::new(&f.0);
        service.ensure_seeds().unwrap();
        let plan = load(
            &f.0,
            Some("通用助手"),
            "批量暂停工作流，并修改桌面卡片的透明度",
        );
        assert!(plan.skills.is_empty());
        assert!(plan.system_prompt.contains("工作流"));
        assert!(plan.system_prompt.contains("桌面卡片"));
        assert!(plan.tools.contains("workflow_batch"));
        assert!(plan.tools.contains("desktop_cards_batch"));
        for skill in service
            .load_skills()
            .iter()
            .filter(|s| ["工作流", "桌面卡片"].contains(&s.id.as_str()))
        {
            assert!(!skill.tools.is_empty());
            for tool in &skill.tools {
                assert!(
                    super::super::tools::definitions::find(tool).is_some(),
                    "{tool}"
                );
                assert!(service
                    .root()
                    .join(format!("Tools/{tool}/TOOL.md"))
                    .is_file());
            }
        }
        let custom = load(&f.0, Some("自定义"), "工作流 桌面卡片");
        assert!(!custom.tools.contains("workflow_batch"));
        assert!(!custom.tools.contains("desktop_cards_batch"));
        assert!(custom.skills.is_empty());
        assert_eq!(
            std::fs::read_to_string(service.root().join("Agents/通用助手.md")).unwrap(),
            legacy
        );
        f.write(
            "Skills/桌面卡片/SKILL.md",
            "---\nname: desktop-cards\ndescription: 桌面卡片\n---\n用户保留的旧说明",
        );
        service.ensure_seeds().unwrap();
        let legacy_skill = load(&f.0, Some("通用助手"), "桌面卡片");
        assert!(!legacy_skill.system_prompt.contains("用户保留的旧说明"));
        assert!(service.load_available_skills().iter().any(
            |s| s.body == "用户保留的旧说明" && s.tools.contains(&"desktop_cards_batch".into())
        ));
        assert!(!legacy_skill
            .system_prompt
            .contains("优先一次 desktop_cards_batch"));
    }
}
