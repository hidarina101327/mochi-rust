//! 整理控制台中可用的 Agent 列表。
use super::*;
impl App {
    pub(super) fn console_agents(&mut self, name: &str, action: &str, a: &Value) -> CResult<Value> {
        let root = self.console_root()?;
        let d = data(a);
        let writing = !mochi_core::ai::tools::console_tools::read_only(name, action);
        if name == "agent_definitions_manage" {
            use mochi_core::ai::agent_config::{parse_frontmatter, AgentConfigService};
            let svc = AgentConfigService::new(&root);
            self.console_scope(&svc.root().to_string_lossy(), writing)?;
            if action == "list" {
                return Ok(
                    json!({"agents":svc.load_agents().iter().map(|v|json!({"id":v.id,"name":v.name,"path":v.source_path})).collect::<Vec<_>>(),"skills":svc.load_available_skills().iter().map(|v|json!({"id":v.id,"name":v.name,"path":v.source_path,"tools":v.tools})).collect::<Vec<_>>(),"mcps":svc.load_mcp_servers().iter().map(|v|json!({"id":v.id,"name":v.name,"path":v.source_path,"enabled":v.enabled})).collect::<Vec<_>>(),"tools":svc.load_prompt_tools().iter().map(|v|json!({"id":v.id,"path":v.source_path,"enabled":v.enabled})).collect::<Vec<_>>() }),
                );
            }
            if matches!(action, "updates" | "apply_update") {
                let updates = svc.agent_updates(true)?;
                if action == "updates" {
                    return Ok(
                        json!({"updates":updates.iter().map(|u|Ok(json!({"id":u.relative_path,"revision":u.revision,"currentRevision":mochi_core::ai::agent_config::updates::revision(&u.current),"diff":u.diff()?,"upstreamDiff":u.upstream_diff()?}))).collect::<CResult<Vec<_>>>()?}),
                    );
                }
                ensure!(
                    !self.agent.source_editor.as_ref().is_some_and(|e| e.dirty),
                    "Agent 定义有未保存编辑"
                );
                let u = updates
                    .iter()
                    .find(|u| Some(u.relative_path.as_str()) == a["id"].as_str())
                    .context("更新不存在")?;
                ensure!(
                    a["revision"] == u.revision
                        && d["currentRevision"]
                            == mochi_core::ai::agent_config::updates::revision(&u.current),
                    "预览后定义已变化，请重新预览"
                );
                let backup = svc.apply_agent_update(u)?;
                self.reload_agent_config();
                self.reload_ai_agents();
                return Ok(json!({"applied":true,"backup":backup}));
            }
            let path = self.console_target(a, writing)?;
            let relative = path
                .strip_prefix(svc.root())
                .context("定义必须位于 Agent配置 中")?;
            let section = relative
                .components()
                .next()
                .context("缺少定义类型")?
                .as_os_str()
                .to_string_lossy();
            ensure!(
                matches!(
                    section.as_ref(),
                    "Agents" | "Skills" | "Tools" | "MCPs" | "QuickActions"
                ),
                "不支持此配置路径"
            );
            ensure!(
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md")),
                "定义必须是 Markdown"
            );
            let depth = relative.components().count();
            let expected = match section.as_ref() {
                "Skills" => Some("SKILL.md"),
                "Tools" => Some("TOOL.md"),
                "MCPs" => Some("MCP.md"),
                _ => None,
            };
            ensure!(if let Some(file)=expected{depth==3&&path.file_name().is_some_and(|n|n.to_string_lossy().eq_ignore_ascii_case(file))}else{depth==2},"定义路径不符合目录契约：Agents/名称.md、Skills/名称/SKILL.md、Tools/名称/TOOL.md、MCPs/名称/MCP.md 或 QuickActions/名称.md");
            if action == "get" {
                return Ok(versioned(json!(std::fs::read_to_string(&path)?)));
            }
            ensure!(
                !self.agent.source_editor.as_ref().is_some_and(|e| e.dirty),
                "定义编辑器有未保存内容"
            );
            if action == "delete" {
                ensure!(
                    path.file_name()
                        .is_none_or(|n| !n.to_string_lossy().eq_ignore_ascii_case("通用助手.md")),
                    "不能删除通用助手"
                );
                if section == "Agents" {
                    let raw = std::fs::read_to_string(&path)?;
                    let (fm, _) = parse_frontmatter(&raw);
                    let id = path.file_stem().unwrap().to_string_lossy();
                    let name = match fm.iter().find(|(k, _)| k == "name").map(|(_, v)| v) {
                        Some(mochi_core::ai::agent_config::FmValue::Scalar(s)) => s.as_str(),
                        _ => id.as_ref(),
                    };
                    ensure!(
                        !svc.agent_referenced_by_menu(&id, name),
                        "此 Agent 被右键菜单引用，不能删除"
                    );
                }
                check_revision(a, &json!(std::fs::read_to_string(&path)?))?;
                std::fs::remove_file(&path)?;
            } else {
                let content = text(d, "content")?;
                let (fm, body) = parse_frontmatter(content);
                ensure!(
                    !fm.is_empty() && (section == "MCPs" || !body.trim().is_empty()),
                    "定义需要 frontmatter 和正文"
                );
                let scalar = |key: &str| {
                    fm.iter()
                        .find(|(k, _)| k == key)
                        .and_then(|(_, v)| match v {
                            mochi_core::ai::agent_config::FmValue::Scalar(s) => Some(s.as_str()),
                            _ => None,
                        })
                };
                for (key, allowed) in [
                    ("enabled", &["true", "false"][..]),
                    (
                        "skillRouting",
                        &["none", "explicit", "light", "progressive"][..],
                    ),
                    ("executionMode", &["reactive", "plan-and-execute"][..]),
                ] {
                    if let Some(v) = scalar(key) {
                        ensure!(allowed.contains(&v), "{key} 取值无效");
                    }
                }
                if section == "MCPs" {
                    match scalar("transport").unwrap_or("stdio") {
                        "stdio" => ensure!(
                            scalar("command").is_some_and(|s| !s.trim().is_empty()),
                            "stdio MCP 缺少 command"
                        ),
                        "http" | "sse" => ensure!(
                            scalar("url").is_some_and(
                                |s| s.starts_with("https://") || s.starts_with("http://")
                            ),
                            "MCP 缺少 HTTP URL"
                        ),
                        _ => bail!("MCP transport 无效"),
                    }
                }
                let warnings = fm
                    .iter()
                    .filter(|(k, _)| k == "tools")
                    .flat_map(|(_, v)| match v {
                        mochi_core::ai::agent_config::FmValue::List(v) => v.clone(),
                        mochi_core::ai::agent_config::FmValue::Scalar(v) => vec![v.clone()],
                    })
                    .filter(|t| {
                        section != "MCPs"
                            && t != "*"
                            && mochi_core::ai::tools::definitions::find(t).is_none()
                    })
                    .collect::<Vec<_>>();
                if action == "validate" {
                    return Ok(json!({"valid":warnings.is_empty(),"unregisteredTools":warnings}));
                }
                ensure!(action == "save", "未知定义操作");
                ensure!(
                    warnings.is_empty(),
                    "包含未注册工具：{}",
                    warnings.join(", ")
                );
                if path.exists() {
                    check_revision(a, &json!(std::fs::read_to_string(&path)?))?;
                }
                mochi_core::files::FileService::new().write_file_safe(&path, content)?;
            }
            self.reload_agent_config();
            self.reload_ai_agents();
            return Ok(json!({"saved":true,"path":path,"nextRequestUsesChanges":true}));
        }
        use mochi_core::ai::session::*;
        let svc = AiSessionService::new(&root);
        self.console_scope(&svc.base_path().to_string_lossy(), writing)?;
        let mut index = svc.load_index();
        let now = mochi_core::jstime::now_millis();
        if let Some(id) = a["id"].as_str() {
            ensure!(mochi_core::ai::locator::safe_session_id(id), "无效 ID");
        }
        match action {
            "list" => return Ok(versioned(json!(index))),
            "get" | "export" => {
                let c = svc.load_session(text(a, "id")?).context("会话不存在")?;
                if action == "get" {
                    return Ok(json!(c));
                }
                let path = self.console_target(a, true)?;
                ensure!(!path.exists(), "导出路径已经存在");
                let body = c
                    .messages
                    .iter()
                    .map(|m| format!("## {}\n\n{}\n", m.role(), m.content()))
                    .collect::<Vec<_>>()
                    .join("\n");
                mochi_core::files::FileService::new().write_file_safe(&path, &body)?;
                return Ok(json!({"path":path}));
            }
            "create" => {
                let id = new_conversation_id();
                let title = text(d, "title")?.to_owned();
                svc.save_session(&AiConversation {
                    id: id.clone(),
                    title: title.clone(),
                    created_at: now,
                    updated_at: now,
                    ..Default::default()
                })?;
                index.sessions.push(AiSessionMeta {
                    id,
                    title,
                    created_at: now,
                    updated_at: now,
                    ..Default::default()
                });
            }
            "update" => {
                check_revision(a, &json!(index))?;
                let id = text(a, "id")?;
                if let Some(p) = d["projectId"].as_str() {
                    ensure!(index.projects.iter().any(|v| v.id == p), "项目不存在");
                }
                let m = index
                    .sessions
                    .iter_mut()
                    .find(|m| m.id == id)
                    .context("会话不存在")?;
                if let Some(title) = d["title"].as_str() {
                    ensure!(!title.trim().is_empty(), "标题不能为空");
                    let mut c = svc.load_session(id).context("会话文件不存在")?;
                    c.title = title.into();
                    c.updated_at = now;
                    svc.save_session(&c)?;
                    m.title = title.into();
                    if let Some(active) = self.ai.panel.active.as_mut().filter(|c| c.id == id) {
                        active.title = title.into();
                    }
                }
                if let Some(v) = d["pinned"].as_bool() {
                    m.pinned = v;
                }
                if d.get("projectId").is_some() {
                    m.project_id = d["projectId"].as_str().map(str::to_owned);
                }
                if let Some(v) = d["order"].as_i64() {
                    m.order = v;
                }
                m.updated_at = now;
            }
            "delete" => {
                check_revision(a, &json!(index))?;
                let id = text(a, "id")?;
                ensure!(
                    self.ai.panel.active.as_ref().is_none_or(|c| c.id != id),
                    "不能删除当前正在运行的会话"
                );
                ensure!(index.sessions.iter().any(|c| c.id == id), "会话不存在");
                svc.delete_session(id)?;
                index.sessions.retain(|c| c.id != id);
            }
            "project_save" => {
                check_revision(a, &json!(index))?;
                let id = a["id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(new_project_id);
                if let Some(p) = index.projects.iter_mut().find(|p| p.id == id) {
                    p.name = text(d, "name")?.into();
                    p.updated_at = now;
                    if let Some(order) = d["order"].as_i64() {
                        p.order = order;
                    }
                } else {
                    index.projects.push(AiProject {
                        id,
                        name: text(d, "name")?.into(),
                        created_at: now,
                        updated_at: now,
                        ..Default::default()
                    });
                }
            }
            "project_delete" => {
                check_revision(a, &json!(index))?;
                let id = text(a, "id")?;
                ensure!(index.projects.iter().any(|p| p.id == id), "项目不存在");
                index.projects.retain(|p| p.id != id);
                for s in &mut index.sessions {
                    if s.project_id.as_deref() == Some(id) {
                        s.project_id = None;
                    }
                }
            }
            _ => bail!("未知会话操作"),
        }
        svc.save_index(&index)?;
        self.ai.panel.sessions = index.sessions.clone();
        self.ai.workspace.projects = index.projects.clone();
        Ok(versioned(json!(index)))
    }
}
