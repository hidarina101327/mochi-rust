//! 仅在当前运行中披露能力。加载说明可能会介绍可用功能，但不会授予权限。
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::{
    agent_config::SkillDefinition, models::AiToolDefinition, permission::AiToolAction,
};
use serde_json::json;
use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
};

pub const INITIAL: &[&str] = &[
    "workspace_get_state",
    "current_document_get",
    "agent_plan",
    "skill_list",
    "skill_load",
];

pub struct SkillSession {
    pub skills: Vec<SkillDefinition>,
    eligible: Option<Vec<AiToolDefinition>>,
    active: HashSet<String>,
    pub loaded: Vec<String>,
}
impl SkillSession {
    pub fn new(skills: Vec<SkillDefinition>) -> Self {
        Self {
            skills,
            eligible: None,
            active: HashSet::new(),
            loaded: vec![],
        }
    }
    pub fn enable(&mut self, eligible: Vec<AiToolDefinition>) {
        self.active = INITIAL.iter().map(|s| (*s).into()).collect();
        self.loaded.clear();
        let external = eligible
            .iter()
            .filter(|t| super::definitions::find(&t.function.name).is_none())
            .map(|t| t.function.name.clone())
            .collect::<Vec<_>>();
        if !external.is_empty() {
            self.skills.push(SkillDefinition { id: "外部服务".into(), name: "外部服务".into(), description: Some("调用当前已连接的外部 MCP 服务".into()), aliases: vec![], tools: external, mcps: vec![], body: "只使用当前已连接并允许的服务。按照工具参数契约操作；工具成功前不要声称已经完成。".into(), source_path: String::new() });
        }
        self.eligible = Some(eligible);
    }
    pub fn definitions(&self) -> Option<Vec<AiToolDefinition>> {
        self.eligible.as_ref().map(|all| {
            all.iter()
                .filter(|t| self.active.contains(&t.function.name))
                .cloned()
                .collect()
        })
    }
    pub fn allows(&self, name: &str) -> bool {
        self.eligible.is_none() || self.active.contains(name)
    }
    fn load(&mut self, args: &ToolArgs) -> ToolOutcome {
        let id = args.str_required("id")?;
        let skill = self
            .skills
            .iter()
            .find(|s| s.id == id || s.name == id || s.aliases.iter().any(|a| a == id))
            .ok_or("找不到 Skill，请先调用 skill_list")?;
        let compiled = crate::ai::agent_config::skill_catalog::console_reference(&skill.tools);
        let body = if args.str_opt("reference") == Some("console-api.md") && compiled.is_some() {
            compiled.clone().unwrap()
        } else if let Some(reference) = args.str_opt("reference") {
            let relative = Path::new(reference);
            if relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("reference 必须是 Skill 目录内的相对路径".into());
            }
            let root = Path::new(&skill.source_path)
                .parent()
                .ok_or("此 Skill 没有参考文件")?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let path = root
                .join(relative)
                .canonicalize()
                .map_err(|e| e.to_string())?;
            if !path.starts_with(&root) {
                return Err("reference 不得越出 Skill 目录".into());
            }
            if path.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 {
                return Err("参考文件超过 128 KiB，请拆分后加载".into());
            }
            std::fs::read_to_string(path).map_err(|e| e.to_string())?
        } else {
            let mut body = skill.body.clone();
            if compiled.is_some() {
                body.push_str("\n\n控制台扩展：此能力包含当前安装包提供的模块操作接口。操作前继续调用 skill_load，使用同一 id 和 reference: console-api.md 按需读取该模块的参数契约；以上旧正文若描述为只读，以此版本契约的实际工具为准。");
            }
            body
        };
        let tools = skill
            .tools
            .iter()
            .filter(|name| {
                self.eligible
                    .as_ref()
                    .is_none_or(|all| all.iter().any(|t| &t.function.name == *name))
            })
            .cloned()
            .collect::<Vec<_>>();
        let unavailable = skill
            .tools
            .iter()
            .filter(|name| !tools.contains(name))
            .cloned()
            .collect::<Vec<_>>();
        self.active.extend(tools.iter().cloned());
        if !self.loaded.contains(&skill.name) {
            self.loaded.push(skill.name.clone());
        }
        Ok(
            json!({"id":skill.id,"name":skill.name,"instructions":body,"tools":tools,"unavailableTools":unavailable,"note":"工具将在下一次模型请求中提供；原有权限、路径范围和审批要求仍然有效。"}),
        )
    }
}

pub struct SkillToolExecutor(pub Arc<Mutex<SkillSession>>);
impl ToolExecutor for SkillToolExecutor {
    fn handles(&self, name: &str) -> bool {
        matches!(name, "skill_list" | "skill_load")
    }
    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let path = if name == "skill_load" {
            self.0
                .lock()
                .ok()
                .and_then(|s| {
                    s.skills
                        .iter()
                        .find(|s| {
                            Some(s.id.as_str()) == args.str_opt("id")
                                || Some(s.name.as_str()) == args.str_opt("id")
                                || s.aliases
                                    .iter()
                                    .any(|a| Some(a.as_str()) == args.str_opt("id"))
                        })
                        .map(|s| {
                            args.str_opt("reference")
                                .and_then(|r| {
                                    Path::new(&s.source_path)
                                        .parent()
                                        .map(|p| p.join(r).to_string_lossy().into_owned())
                                })
                                .unwrap_or_else(|| s.source_path.clone())
                        })
                })
                .filter(|p| !p.is_empty())
        } else {
            None
        };
        vec![(AiToolAction::ReadFile, path)]
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let mut session = self.0.lock().map_err(|_| "Skill 会话不可用")?;
        match name {
            "skill_list" => {
                let query = args.str_opt("query").unwrap_or("").to_lowercase();
                Ok(
                    json!({"skills":session.skills.iter().filter(|s| format!("{} {} {}",s.name,s.description.as_deref().unwrap_or(""),s.aliases.join(" ")).to_lowercase().contains(&query)).map(|s| json!({"id":s.id,"name":s.name,"description":s.description})).collect::<Vec<_>>()}),
                )
            }
            "skill_load" => session.load(args),
            _ => Err("未知 Skill 工具".into()),
        }
    }
}

pub fn definitions() -> Vec<AiToolDefinition> {
    serde_json::from_value(json!([
        {"type":"function","function":{"name":"skill_list","description":"查询可用能力目录，仅返回 Skill 名称和用途；需要时再用 skill_load 加载。","parameters":{"type":"object","properties":{"query":{"type":"string"}},"additionalProperties":false}}},
        {"type":"function","function":{"name":"skill_load","description":"按 ID 加载 Skill 的操作指引，并在下一轮提供该能力的工具。可按需读取正文引用的参考文件。","parameters":{"type":"object","properties":{"id":{"type":"string"},"reference":{"type":"string","description":"可选，Skill 目录内的参考文件相对路径"}},"required":["id"],"additionalProperties":false}}}
    ])).expect("Skill schemas")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn console_contract_is_disclosed_only_for_the_loaded_module() {
        let mut session =
            SkillSession::new(crate::ai::agent_config::skill_catalog::bundled_skills());
        session.enable(super::super::definitions::all().to_vec());
        assert_eq!(session.definitions().unwrap().len(), INITIAL.len());
        let loaded = session
            .load(&ToolArgs::from_value(json!({"id":"系统管理"})))
            .unwrap();
        assert!(loaded["instructions"]
            .as_str()
            .unwrap()
            .contains("console-api.md"));
        assert!(!loaded["instructions"]
            .as_str()
            .unwrap()
            .contains("## sync_manage"));
        assert!(session.allows("sync_manage"));
        assert!(!session.allows("exam_manage"));
        let detail = session
            .load(&ToolArgs::from_value(
                json!({"id":"系统管理","reference":"console-api.md"}),
            ))
            .unwrap();
        let body = detail["instructions"].as_str().unwrap();
        assert!(body.contains("## sync_manage"));
        assert!(!body.contains("## exam_manage"));
    }
    #[test]
    fn references_are_loaded_separately_and_cannot_escape_skill_directory() {
        let root = std::env::temp_dir().join(format!(
            "mochi-skill-ref-{}",
            crate::paths::random_base36(16)
        ));
        std::fs::create_dir_all(root.join("references")).unwrap();
        std::fs::write(root.join("references/details.md"), "reference-sentinel").unwrap();
        let skill = SkillDefinition {
            id: "sample".into(),
            name: "示例".into(),
            description: Some("description-only".into()),
            aliases: vec![],
            tools: vec!["file_read".into(), "file_write".into()],
            mcps: vec![],
            body: "body-sentinel".into(),
            source_path: root.join("SKILL.md").to_string_lossy().into_owned(),
        };
        let mut session = SkillSession::new(vec![skill]);
        session.enable(vec![super::super::definitions::find("file_read")
            .unwrap()
            .clone()]);
        assert!(session
            .load(&ToolArgs::from_value(
                json!({"id":"sample","reference":"../outside.md"})
            ))
            .is_err());
        assert!(session.definitions().unwrap().is_empty());
        let result = session
            .load(&ToolArgs::from_value(json!({"id":"sample"})))
            .unwrap();
        assert_eq!(result["instructions"], "body-sentinel");
        assert_eq!(result["unavailableTools"], json!(["file_write"]));
        let result = session
            .load(&ToolArgs::from_value(
                json!({"id":"sample","reference":"references/details.md"}),
            ))
            .unwrap();
        assert_eq!(result["instructions"], "reference-sentinel");
        assert_eq!(session.definitions().unwrap().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
