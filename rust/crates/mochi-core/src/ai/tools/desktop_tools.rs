//! 桌面工具经 UI 宿主执行，编辑就不会和它未保存的布局赛跑。
use super::{host::ToolHost, ToolArgs, ToolExecutor, ToolOutcome};
use crate::{
    ai::{models::AiToolDefinition, permission::AiToolAction},
    desktop_cards::{DesktopConfig, Module},
};
use serde_json::{json, Value};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

pub const GUIDE: &str = include_str!("../../../assets/desktop-cards/SKILL.md");
pub struct DesktopToolExecutor(pub Arc<dyn ToolHost>);
impl ToolExecutor for DesktopToolExecutor {
    fn handles(&self, name: &str) -> bool {
        name == "desktop_cards_templates"
            || (self.0.supports_desktop_cards()
                && matches!(
                    name,
                    "desktop_cards_get" | "desktop_cards_update" | "desktop_cards_batch"
                ))
    }
    fn required_actions(&self, name: &str, _: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        if matches!(name, "desktop_cards_update" | "desktop_cards_batch") {
            vec![(AiToolAction::ModifySettings, None)]
        } else {
            vec![]
        }
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if name == "desktop_cards_templates" {
            return Ok(
                json!({"templates":Module::ALL.iter().map(|m|json!({"module":m,"label":m.label(),"card":crate::desktop_cards::Card::new(m.label(),*m),"page":m.default_page(),"options":m.options().iter().map(|o|json!({"key":o.key,"label":o.label,"description":o.description})).collect::<Vec<_>>()})).collect::<Vec<_>>() }),
            );
        }
        self.0.desktop_cards(name, args.value())
    }
}
pub fn snapshot(config: &DesktopConfig) -> Value {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(config).unwrap().hash(&mut hash);
    json!({"revision":format!("{:016x}",hash.finish()),"config":config})
}
pub fn update(current: &DesktopConfig, args: &Value) -> Result<DesktopConfig, String> {
    if args["revision"] != snapshot(current)["revision"] {
        return Err("卡片已被修改，请重新读取后合并变更".into());
    }
    let raw = args.get("config").ok_or("缺少 config")?;
    if raw.to_string().len() > crate::desktop_cards::MAX_CONFIG_BYTES {
        return Err("卡片配置过大".into());
    }
    let next: DesktopConfig = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
    next.validate().map_err(|e| e.to_string())?;
    for card in &next.cards {
        for page in &card.pages {
            if let Some(old) = current
                .cards
                .iter()
                .flat_map(|c| &c.pages)
                .find(|p| p.id == page.id)
            {
                if old.module != page.module {
                    return Err("已有分页不能更换模板；请创建新分页".into());
                }
            }
        }
    }
    Ok(next)
}

/// 所有操作先在私有副本上应用；校验通过的最终布局由 UI 一次性保存。
pub fn batch(current: &DesktopConfig, args: &Value) -> Result<DesktopConfig, String> {
    if args["revision"] != snapshot(current)["revision"] {
        return Err("卡片已被修改，请重新读取后合并变更".into());
    }
    let operations = args["operations"]
        .as_array()
        .ok_or("operations 必须是数组")?;
    if operations.is_empty() || operations.len() > 64 {
        return Err("每批需要 1–64 个卡片操作".into());
    }
    if args.to_string().len() > crate::desktop_cards::MAX_CONFIG_BYTES {
        return Err("卡片批量参数过大".into());
    }
    let mut config = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let cards = config["cards"].as_array_mut().unwrap();
    for (index, operation) in operations.iter().enumerate() {
        let fields = operation.as_object().ok_or("卡片操作必须是对象")?;
        let allowed: &[&str] = match operation["op"].as_str() {
            Some("create") => &["op", "card"],
            Some("update") => &["op", "id", "changes"],
            Some("delete") => &["op", "id"],
            _ => return Err(format!("第 {} 项：未知卡片操作", index + 1)),
        };
        if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(format!("第 {} 项：包含未知卡片操作字段", index + 1));
        }
        match operation["op"].as_str() {
            Some("create") => {
                let card = operation.get("card").ok_or("create 缺少 card")?;
                let parsed: crate::desktop_cards::Card =
                    serde_json::from_value(card.clone()).map_err(|e| e.to_string())?;
                if cards.iter().any(|c| c["id"] == parsed.id) {
                    return Err(format!("第 {} 项：卡片 ID 已存在", index + 1));
                }
                cards.push(serde_json::to_value(parsed).map_err(|e| e.to_string())?);
            }
            Some("update" | "delete") => {
                let id = operation["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("缺少卡片 id")?;
                let position = cards
                    .iter()
                    .position(|c| c["id"] == id)
                    .ok_or_else(|| format!("第 {} 项：找不到卡片 {id}", index + 1))?;
                if operation["op"] == "delete" {
                    cards.remove(position);
                } else {
                    let changes = operation["changes"]
                        .as_object()
                        .ok_or("update 缺少 changes 对象")?;
                    if changes.is_empty() || changes.contains_key("id") {
                        return Err("changes 不能为空，也不能修改卡片 id".into());
                    }
                    merge_changes(&mut cards[position], &operation["changes"])?;
                }
            }
            _ => {
                return Err(format!(
                    "第 {} 项：未知卡片操作，只支持 create/update/delete",
                    index + 1
                ))
            }
        }
    }
    update(
        current,
        &json!({"revision":args["revision"],"config":config}),
    )
}

fn merge_changes(target: &mut Value, patch: &Value) -> Result<(), String> {
    if let (Some(object), Some(changes)) = (target.as_object_mut(), patch.as_object()) {
        for (key, value) in changes {
            let field = object
                .get_mut(key)
                .ok_or_else(|| format!("未知卡片字段：{key}"))?;
            merge_changes(field, value)?;
        }
    } else {
        *target = patch.clone();
    }
    Ok(())
}
pub fn definitions() -> Vec<AiToolDefinition> {
    let mut tools: Vec<AiToolDefinition> = serde_json::from_value(json!([
        {"type":"function","function":{"name":"desktop_cards_get","description":"读取当前桌面卡片布局和 revision，不包含笔记正文。","parameters":{"type":"object","properties":{},"additionalProperties":false}}},
        {"type":"function","function":{"name":"desktop_cards_templates","description":"列出桌面卡片模块、选项和新分页默认配置。","parameters":{"type":"object","properties":{},"additionalProperties":false}}},
        {"type":"function","function":{"name":"desktop_cards_update","description":"使用最近读取的 revision 原子应用完整桌面卡片配置。支持新建、修改、删除、分页、导入布局、透明度、字体和尺寸；保留未修改的卡片。已有分页不可更换模板。","parameters":{"type":"object","properties":{"revision":{"type":"string"},"config":{"type":"object","properties":{"version":{"type":"integer","enum":[1]},"cards":{"type":"array","maxItems":12,"items":{"type":"object"}}},"required":["version","cards"],"additionalProperties":false}},"required":["revision","config"],"additionalProperties":false}}}
    ])).expect("desktop tool schemas");
    tools.push(AiToolDefinition::function("desktop_cards_batch", "按 revision 批量创建、修改或删除桌面卡片。全部校验后一次保存，失败不应用任何改动。update.changes 按字段合并（appearance 可部分修改；pages 数组整体替换），保留无关卡片。先读取 get 和 templates。", json!({
        "type":"object", "properties": {
            "revision":{"type":"string"},
            "operations":{"type":"array","minItems":1,"maxItems":64,"items":{"oneOf":[
                {"type":"object","properties":{"op":{"const":"create"},"card":{"type":"object"}},"required":["op","card"],"additionalProperties":false},
                {"type":"object","properties":{"op":{"const":"update"},"id":{"type":"string"},"changes":{"type":"object"}},"required":["op","id","changes"],"additionalProperties":false},
                {"type":"object","properties":{"op":{"const":"delete"},"id":{"type":"string"}},"required":["op","id"],"additionalProperties":false}
            ]}}
        },"required":["revision","operations"],"additionalProperties":false
    })));
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_batch_merges_fields_and_preserves_unrelated_cards() {
        let mut c = DesktopConfig::default();
        let a = crate::desktop_cards::Card::new("A", Module::Home);
        let b = crate::desktop_cards::Card::new("B", Module::Schedule);
        let new = crate::desktop_cards::Card::new("C", Module::Clock);
        c.cards = vec![a.clone(), b.clone()];
        let args = json!({"revision":snapshot(&c)["revision"],"operations":[
            {"op":"update","id":a.id,"changes":{"enabled":false,"appearance":{"opacity":70}}},
            {"op":"create","card":new}
        ]});
        let next = batch(&c, &args).unwrap();
        assert_eq!(next.cards.len(), 3);
        assert!(!next.cards[0].enabled);
        assert_eq!(next.cards[0].appearance.opacity, 70);
        assert_eq!(next.cards[0].appearance.font_size, a.appearance.font_size);
        assert_eq!(next.cards[1], b);
        assert!(batch(&next, &args).is_err());
        let deleted = batch(&next, &json!({"revision":snapshot(&next)["revision"],"operations":[{"op":"delete","id":a.id},{"op":"delete","id":new.id}]})).unwrap();
        assert_eq!(deleted.cards, vec![b]);
    }
    #[test]
    fn desktop_batch_invalid_later_operation_leaves_original_unchanged() {
        let mut c = DesktopConfig::default();
        c.cards.push(crate::desktop_cards::Card::default());
        let original = c.clone();
        let id = c.cards[0].id.clone();
        for bad in [
            json!({"op":"delete","id":"missing"}),
            json!({"op":"update","id":id,"changes":{"appearance":{"opacity":0}}}),
            json!({"op":"update","id":id,"changes":{"appearance":{"opactiy":70}}}),
            json!({"op":"create","card":c.cards[0]}),
        ] {
            assert!(batch(&c, &json!({"revision":snapshot(&c)["revision"],"operations":[{"op":"update","id":id,"changes":{"title":"Changed"}},bad]})).is_err());
            assert_eq!(c, original);
        }
        let mut pages = serde_json::to_value(&c.cards[0].pages).unwrap();
        pages[0]["module"] = json!("schedule");
        assert!(batch(&c, &json!({"revision":snapshot(&c)["revision"],"operations":[{"op":"update","id":id,"changes":{"pages":pages}}]})).is_err());
    }
    #[test]
    fn desktop_tools_respect_permission_and_seed_without_overwriting() {
        use super::super::ToolRegistry;
        use crate::ai::{
            agent_config::AgentConfigService,
            models::{AiToolCall, AiToolFunction},
            permission::AiPermissionService,
        };
        struct Host(std::path::PathBuf);
        impl ToolHost for Host {
            fn workspace_root(&self) -> &std::path::Path {
                &self.0
            }
            fn supports_desktop_cards(&self) -> bool {
                true
            }
            fn desktop_cards(&self, _: &str, _: &Value) -> ToolOutcome {
                Ok(json!({"called":true}))
            }
        }
        let root =
            std::env::temp_dir().join(format!("mochi-desktop-permission-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let permissions = Arc::new(AiPermissionService::new(&root));
        let mut actions = permissions.action_permissions();
        actions.set(AiToolAction::ModifySettings, false);
        permissions.set_action_permissions(actions);
        let registry = ToolRegistry::new(permissions)
            .with(Arc::new(DesktopToolExecutor(Arc::new(Host(root.clone())))));
        for name in ["desktop_cards_update", "desktop_cards_batch"] {
            let result = registry.execute(&AiToolCall {
                id: "desktop".into(),
                kind: "function".into(),
                function: AiToolFunction {
                    name: name.into(),
                    arguments: "{}".into(),
                },
            });
            assert_eq!(serde_json::from_str::<Value>(&result).unwrap()["ok"], false);
        }
        let service = AgentConfigService::new(&root);
        service.ensure_seeds().unwrap();
        assert!(service.load_skills().iter().any(|s| s.id == "桌面卡片"));
        let skill = service.root().join("Skills/桌面卡片/SKILL.md");
        std::fs::write(&skill, "user custom guide").unwrap();
        service.ensure_seeds().unwrap();
        assert_eq!(std::fs::read_to_string(skill).unwrap(), "user custom guide");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn desktop_tools_reject_stale_revision_and_template_mutation() {
        let mut c = DesktopConfig::default();
        c.cards
            .push(crate::desktop_cards::Card::new("Test", Module::Home));
        let mut args = snapshot(&c);
        args["config"]["cards"][0]["appearance"]["opacity"] = 70.into();
        let changed = update(&c, &args).unwrap();
        assert_eq!(changed.cards[0].appearance.opacity, 70);
        assert!(update(&changed, &args).is_err());
        args["config"]["cards"][0]["pages"][0]["module"] =
            serde_json::to_value(Module::Inbox).unwrap();
        assert!(update(&c, &args).is_err());
    }
    #[test]
    fn desktop_appearance_roundtrip_and_validation() {
        let mut c = DesktopConfig::default();
        c.cards.push(crate::desktop_cards::Card::default());
        let mut old = serde_json::to_value(&c).unwrap();
        old["cards"][0]
            .as_object_mut()
            .unwrap()
            .remove("appearance");
        let restored: DesktopConfig = serde_json::from_value(old).unwrap();
        assert_eq!(restored.cards[0].appearance.opacity, 85);
        c.cards[0].appearance.opacity = 0;
        assert!(c.validate().is_err());
        c.cards[0].appearance.opacity = 85;
        c.cards[0].appearance.font_color = Some(0x1000000);
        assert!(c.validate().is_err());
    }
}
