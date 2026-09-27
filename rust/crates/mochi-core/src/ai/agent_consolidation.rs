//! 退役冗余的内置 Agent，但不删用户文字，也不破坏菜单依赖。
use super::*;
use anyhow::{ensure, Result};
use std::collections::HashSet;
static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) const RETIRED: &[&str] = &[
    "墨池管家",
    "日程助手",
    "学习助手",
    "日报助手",
    "项目助手",
    "写作助手",
    "翻译助手",
    "联网研究助手",
    "Agent框架助手",
    "插件创作助手",
    "桌面卡片助手",
];

impl AgentConfigService {
    pub fn agent_referenced_by_menu(&self, id: &str, name: &str) -> bool {
        read_markdown_dir(&self.section("QuickActions"))
            .iter()
            .any(|doc| {
                let (data, _) = parse_frontmatter(&doc.raw);
                ["agent", "agentId", "agent_id", "agents"]
                    .iter()
                    .any(|key| {
                        fm_array(&data, key).iter().any(|value| {
                            let target = stem(value.rsplit(['/', '\\']).next().unwrap_or(value));
                            target == id || target == name
                        })
                    })
            })
    }
    pub(super) fn consolidate_agents(&self) -> Result<()> {
        let _guard = GATE
            .lock()
            .map_err(|_| anyhow::anyhow!("Agent retirement lock unavailable"))?;
        let root = self.root();
        let marker = root.join(".bundled-agents/retirement-v1.json");
        if marker.exists() {
            return Ok(());
        }
        let workspace = self.workspace_path.canonicalize()?;
        let mut archived = Vec::new();
        let mut protected = HashSet::new();
        // 停用的映射关系也要保留：菜单项以后重新打开必须还能用。
        for doc in read_markdown_dir(&self.section("QuickActions")) {
            let (data, _) = parse_frontmatter(&doc.raw);
            for key in ["agent", "agentId", "agent_id", "agents"] {
                for value in fm_array(&data, key) {
                    protected.insert(stem(value.rsplit(['/', '\\']).next().unwrap_or(&value)));
                    protected.insert(value);
                }
            }
        }
        for id in RETIRED {
            let source = root.join("Agents").join(format!("{id}.md"));
            if !source.exists() {
                continue;
            }
            let raw = std::fs::read_to_string(&source)?;
            let (data, _) = parse_frontmatter(&raw);
            let name = fm_string(&data, "name").unwrap_or_else(|| (*id).into());
            if protected.contains(*id)
                || protected.contains(&name)
                || protected.contains(&format!("{id}.md"))
            {
                continue;
            }
            ensure!(
                source.canonicalize()?.starts_with(&workspace),
                "Agent definition is outside workspace"
            );
            let archive = root.join(".bundled-agents").join("retired");
            let mut ancestor = archive.as_path();
            while !ancestor.exists() {
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("invalid archive directory"))?;
            }
            ensure!(
                ancestor.canonicalize()?.starts_with(&workspace),
                "Agent archive is outside workspace"
            );
            std::fs::create_dir_all(&archive)?;
            ensure!(
                archive.canonicalize()?.starts_with(&workspace),
                "Agent archive is outside workspace"
            );
            let target = archive.join(format!("{id}-{}.md", paths::random_base36(16)));
            // 原文件退役之前，必须先成功创建一份不会覆盖现有文件的备份。
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            file.write_all(raw.as_bytes())?;
            file.sync_all()?;
            std::fs::remove_file(&source)?;
            archived.push(*id);
        }
        let parent = marker.parent().unwrap();
        let mut ancestor = parent;
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("invalid retirement directory"))?;
        }
        ensure!(
            ancestor.canonicalize()?.starts_with(&workspace),
            "Agent retirement directory is outside workspace"
        );
        std::fs::create_dir_all(parent)?;
        ensure!(
            parent.canonicalize()?.starts_with(&workspace),
            "Agent retirement directory is outside workspace"
        );
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)?;
        file.write_all(
            serde_json::to_string_pretty(
                &serde_json::json!({"version":1,"archived":archived,"menuAgents":protected} ),
            )?
            .as_bytes(),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_preserves_menu_dependencies_and_custom_text_in_backup() {
        let root = std::env::temp_dir().join(format!(
            "mochi-agent-retirement-{}",
            paths::random_base36(16)
        ));
        let svc = AgentConfigService::new(&root);
        std::fs::create_dir_all(svc.section("Agents")).unwrap();
        std::fs::create_dir_all(svc.section("QuickActions")).unwrap();
        std::fs::write(
            svc.section("Agents").join("写作助手.md"),
            "自定义过的写作助手",
        )
        .unwrap();
        std::fs::write(svc.section("Agents").join("翻译助手.md"), "不要动").unwrap();
        std::fs::write(
            svc.section("QuickActions").join("翻译.md"),
            "---\nagent: 翻译助手\nenabled: false\n---\n翻译选区",
        )
        .unwrap();
        svc.ensure_seeds().unwrap();
        assert!(!svc.section("Agents").join("写作助手.md").exists());
        assert_eq!(
            std::fs::read_to_string(svc.section("Agents").join("翻译助手.md")).unwrap(),
            "不要动"
        );
        let backup = std::fs::read_dir(svc.root().join(".bundled-agents/retired"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            std::fs::read_to_string(backup).unwrap(),
            "自定义过的写作助手"
        );
        assert_eq!(svc.ensure_seeds().unwrap(), 0);
        // 用户有意用旧名新建的 Agent，不会每次启动都被退役。
        std::fs::write(
            svc.section("Agents").join("写作助手.md"),
            "新建自定义 Agent",
        )
        .unwrap();
        svc.ensure_seeds().unwrap();
        assert_eq!(
            std::fs::read_to_string(svc.section("Agents").join("写作助手.md")).unwrap(),
            "新建自定义 Agent"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
