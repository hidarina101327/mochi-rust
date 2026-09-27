//! 只向模型暴露当前宿主实际能执行的工具；advertise 按 handles 过滤定义表。

use std::sync::{Arc, Mutex};

use crate::agenda::AgendaStore;
use crate::app_settings::AppSettings;
use crate::git::GitService;
use crate::memory::MemoryStore;
use crate::search::SearchService;

use super::super::permission::AiPermissionService;
use super::host::ToolHost;
use super::{
    agenda_tools::AgendaToolExecutor, assist_tools::AssistToolExecutor,
    base_tools::BaseToolExecutor, canvas_tools::CanvasToolExecutor, core::CoreToolExecutor,
    document_tools::DocumentToolExecutor, git_tools::GitToolExecutor,
    memory_tools::MemoryToolExecutor, range_edit_tools::RangeEditToolExecutor,
    settings_tools::SettingsToolExecutor, shell_tools::ShellToolExecutor,
    sub_document_tools::SubDocumentToolExecutor, workspace_tools::WorkspaceToolExecutor,
    ToolRegistry,
};

/// HeadlessHost 不支持图形导出；此表供测试校验能力差异，不限制 AppHost。
pub const HOST_DEPENDENT: &[&str] = &[
    "document_export",
    "desktop_cards_get",
    "desktop_cards_update",
    "desktop_cards_batch",
    "console_state",
    "console_navigate",
    "console_tabs",
    "console_workspace",
    "inbox_manage",
    "favorites_manage",
    "recent_manage",
    "unread_manage",
    "templates_manage",
    "base_manage",
    "canvas_manage",
    "comments_manage",
    "annotations_manage",
    "calendar_navigate",
    "desktop_cards_control",
    "workflow_open",
    "ai_sessions_manage",
    "agent_definitions_manage",
    "english_manage",
    "exam_manage",
    "pomodoro_manage",
    "plugins_manage",
    "marketplace_manage",
    "mapped_folders_manage",
    "workspace_transfer",
    "sync_manage",
    "updates_manage",
    "notifications_manage",
    "home_stats",
];

/// 组装完整工具集所需的服务依赖。
///
/// 记忆功能是否可用会明确标出：可选的记忆数据库不可用时，
/// 不能因此关闭普通对话，也不能显示无法使用的记忆工具。
/// 真需要精简工具集的场景（子 Agent、后台任务）用 Agent 配置的 `Selection` 表达，
/// 那是显式的、写在 `Agents/*.md` 里的、用户看得见的。
pub struct ToolServices {
    pub host: Arc<dyn ToolHost>,
    pub permissions: Arc<AiPermissionService>,
    pub git: Arc<GitService>,
    pub agenda: Arc<AgendaStore>,
    pub settings: Arc<AppSettings>,
    pub search: Arc<SearchService>,
    pub memory: Option<Arc<Mutex<MemoryStore>>>,
}

/// 把全部执行器接进一个注册表。权限门由 `ToolRegistry` 统一强制，见 `super`。
pub fn build(services: &ToolServices) -> ToolRegistry {
    let host = &services.host;
    let mut registry = ToolRegistry::new(services.permissions.clone())
        .with(Arc::new(super::console_tools::ConsoleToolExecutor(
            host.clone(),
        )))
        .with(Arc::new(super::desktop_tools::DesktopToolExecutor(
            host.clone(),
        )))
        .with(Arc::new(super::workflow_tools::WorkflowToolExecutor::new(
            host.clone(),
        )))
        .with(Arc::new(BaseToolExecutor::new(host.clone())))
        .with(Arc::new(
            super::base_automation_tools::BaseAutomationToolExecutor::new(host.clone()),
        ))
        .with(Arc::new(CanvasToolExecutor::new(host.clone())))
        .with(Arc::new(
            CoreToolExecutor::new(host.clone()).with_search(services.search.clone()),
        ))
        .with(Arc::new(GitToolExecutor::new(
            host.clone(),
            services.git.clone(),
        )))
        .with(Arc::new(
            AgendaToolExecutor::new(services.agenda.clone())
                .with_settings(services.settings.clone()),
        ))
        .with(Arc::new(WorkspaceToolExecutor::new(host.clone())))
        .with(Arc::new(SettingsToolExecutor::new(
            services.settings.clone(),
            host.clone(),
        )))
        .with(Arc::new(DocumentToolExecutor::new(host.clone())))
        .with(Arc::new(RangeEditToolExecutor::new(host.clone())))
        .with(Arc::new(
            SubDocumentToolExecutor::new(host.clone()).with_settings(services.settings.clone()),
        ))
        .with(Arc::new(ShellToolExecutor::new(host.clone())))
        .with(Arc::new(super::script_tools::ScriptToolExecutor::new(
            host.clone(),
        )))
        .with(Arc::new(AssistToolExecutor::new(host.clone())));
    if let Some(memory) = &services.memory {
        registry.register(Arc::new(
            MemoryToolExecutor::new(memory.clone(), host.clone())
                .with_settings(services.settings.clone()),
        ));
    }
    registry.install_skills(
        crate::ai::agent_config::AgentConfigService::new(host.workspace_root())
            .load_available_skills(),
    );
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent_config::Selection;
    use crate::ai::tools::definitions;
    use crate::ai::tools::host::HeadlessHost;
    use crate::metadata_index::MetadataIndexService;
    use crate::settings::SettingsService;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
        registry: ToolRegistry,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        fixture_with_memory(tag, true)
    }
    fn fixture_with_memory(tag: &str, available: bool) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-registry-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let index = Arc::new(MetadataIndexService::new(&root));
        let services = ToolServices {
            host: Arc::new(HeadlessHost::new(&root)),
            permissions: Arc::new(AiPermissionService::new(&root)),
            git: Arc::new(GitService::new(&root)),
            agenda: Arc::new(AgendaStore::new(&root)),
            settings: Arc::new(AppSettings::new(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))),
            search: Arc::new(SearchService::new(&root, index)),
            memory: available.then(|| Arc::new(Mutex::new(MemoryStore::open(&root).unwrap()))),
        };
        Fixture {
            root,
            registry: build(&services),
        }
    }

    #[test]
    fn unavailable_memory_removes_only_memory_tools_from_advertising() {
        let f = fixture_with_memory("no-memory", false);
        let tools = f.registry.advertise(&Selection::All, &BTreeMap::new());
        assert!(tools.iter().any(|t| t.function.name == "file_read"));
        assert!(tools
            .iter()
            .any(|t| t.function.name == "workspace_get_state"));
        assert!(!tools.iter().any(|t| t.function.name.starts_with("memory_")));
        assert!(!MemoryStore::db_path(&f.root).exists());
    }

    /// 核心不变量之一：定义表里的每个工具都要有执行器，已知缺口除外。
    /// 新增工具定义却忘了接执行器，这条会指名道姓地报出来。
    #[test]
    fn every_defined_tool_has_an_executor_except_known_gaps() {
        let f = fixture("coverage");
        let missing: Vec<&str> = definitions::names()
            .into_iter()
            .filter(|n| !f.registry.handles(n))
            .collect();
        assert_eq!(
            missing, HOST_DEPENDENT,
            "定义表与执行器对不上。缺执行器的是 {missing:?}，宿主依赖工具是 {HOST_DEPENDENT:?}"
        );
    }

    /// 核心不变量之二：广告出去的每个工具都真能执行。
    #[test]
    fn advertised_names_are_all_executable() {
        let f = fixture("advertise");
        let advertised = f.registry.advertise(&Selection::All, &BTreeMap::new());
        assert!(!advertised.is_empty());
        for tool in &advertised {
            assert!(
                f.registry.handles(&tool.function.name),
                "{} 被广告给模型了，但没有执行器",
                tool.function.name
            );
        }
        assert_eq!(
            advertised.len(),
            definitions::count() - HOST_DEPENDENT.len(),
            "广告数 = 定义数 - 已知缺口数"
        );
    }

    /// 未实现的工具不能出现在发给模型的清单里——否则模型会调用它然后拿回「未知工具」。
    #[test]
    fn unimplemented_tools_are_defined_but_never_advertised() {
        let f = fixture("gap");
        for name in HOST_DEPENDENT {
            assert!(
                definitions::find(name).is_some(),
                "{name} 应该还在定义表里（上游确实有这个工具）"
            );
            assert!(
                !f.registry
                    .advertise(&Selection::All, &BTreeMap::new())
                    .iter()
                    .any(|t| &t.function.name == name),
                "{name} 没有执行器，不该广告给模型"
            );
        }
    }

    /// 即使 Agent 显式点名了未实现的工具，也不能广告出去——
    /// 用户在 `Agents/*.md` 里写的名字不该反过来创造出不存在的能力。
    #[test]
    fn an_explicit_selection_cannot_resurrect_an_unimplemented_tool() {
        let f = fixture("gap-selection");
        let picked = f.registry.advertise(
            &Selection::List(vec!["document_export".into(), "file_read".into()]),
            &BTreeMap::new(),
        );
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].function.name, "file_read");
    }

    #[test]
    fn advertise_respects_selection_and_description_overrides() {
        let f = fixture("select");
        let mut overrides = BTreeMap::new();
        overrides.insert("file_read".to_string(), "用户改写过的说明".to_string());

        let picked = f.registry.advertise(
            &Selection::List(vec!["file_read".into(), "git_commit".into()]),
            &overrides,
        );
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].function.description, "用户改写过的说明");
        assert_ne!(picked[1].function.description, "用户改写过的说明");
    }

    /// 组装顺序不能让某个执行器抢走别人的工具名（`handles` 是先到先得的）。
    #[test]
    fn no_two_executors_claim_the_same_tool() {
        let f = fixture("overlap");
        for name in definitions::names() {
            let claims = f.registry.executor_count_for(name);
            assert!(claims <= 1, "{name} 被 {claims} 个执行器同时认领了");
        }
    }

    /// 抽查每组各一个，确认组装真的把执行器接上了（而不是只有 `handles` 对）。
    #[test]
    fn one_tool_from_each_group_actually_runs() {
        let f = fixture("smoke");
        for name in [
            "folder_list",         // 核心工具
            "git_history",         // Git 工具
            "agenda_overview",     // 日程待办工具
            "workspace_get_state", // 工作区工具
            "settings_list",       // 设置工具
            "comment_list",        // 文档工具
            "memory_search",       // 记忆工具
            "mochi_docs",          // 辅助工具
        ] {
            let call = crate::ai::models::AiToolCall {
                id: "c".into(),
                kind: "function".into(),
                function: crate::ai::models::AiToolFunction {
                    name: name.into(),
                    arguments: "{}".into(),
                },
            };
            let out = f.registry.execute(&call);
            assert!(!out.contains("未知工具"), "{name} 没被接进注册表: {out}");
        }
    }

    // ---------- 与 Agent 运行时的接缝 ----------
    //
    // 上面的测试证明「注册表能执行工具」，`agent_runner` 的测试证明「循环能驱动工具」，
    // 但两者各自用的都是对方的替身。下面两条把真注册表接进真循环，证明这条缝是通的——
    // 这才是「48 个工具从此可被模型调用」这句话的凭据。

    /// 按脚本吐响应的假模型，并记录每轮收到的 transcript。
    struct ScriptedModel {
        script: std::cell::RefCell<Vec<crate::ai::models::AiCompletionResponse>>,
        seen: std::cell::RefCell<Vec<Vec<crate::ai::models::AiMessage>>>,
        tool_names: std::cell::RefCell<Vec<Vec<String>>>,
    }

    impl crate::ai::agent_runner::ModelClient for ScriptedModel {
        fn complete(
            &self,
            request: &crate::ai::models::AiCompletionRequest,
            _: &mut dyn FnMut(crate::ai::models::AiStreamChunk),
        ) -> Result<crate::ai::models::AiCompletionResponse, crate::ai::models::AiError> {
            self.seen.borrow_mut().push(request.messages.clone());
            self.tool_names.borrow_mut().push(
                request
                    .tools
                    .iter()
                    .map(|t| t.function.name.clone())
                    .collect(),
            );
            let mut script = self.script.borrow_mut();
            assert!(!script.is_empty(), "循环没按预期停下来");
            Ok(script.remove(0))
        }
    }

    fn scripted(script: Vec<crate::ai::models::AiCompletionResponse>) -> ScriptedModel {
        ScriptedModel {
            script: std::cell::RefCell::new(script),
            seen: std::cell::RefCell::new(Vec::new()),
            tool_names: std::cell::RefCell::new(Vec::new()),
        }
    }

    fn wants(name: &str, arguments: &str) -> crate::ai::models::AiCompletionResponse {
        crate::ai::models::AiCompletionResponse {
            finish_reason: "tool_calls".into(),
            tool_calls: vec![crate::ai::models::AiToolCall {
                id: "c1".into(),
                kind: "function".into(),
                function: crate::ai::models::AiToolFunction {
                    name: name.into(),
                    arguments: arguments.into(),
                },
            }],
            ..Default::default()
        }
    }

    fn agent_request(text: &str, registry: &ToolRegistry) -> crate::ai::agent_runner::AgentRequest {
        crate::ai::agent_runner::AgentRequest {
            messages: vec![crate::ai::models::AiMessage::new("user", text)],
            tools: registry.advertise(&Selection::All, &BTreeMap::new()),
            ..Default::default()
        }
    }

    /// 端到端：模型要求查产品说明书 → 真注册表执行 → 结果回到下一轮请求里。
    #[test]
    fn the_agent_loop_drives_the_real_registry() {
        let f = fixture("e2e");
        let model = scripted(vec![
            wants("mochi_docs", r#"{"topic":"shortcuts"}"#),
            crate::ai::models::AiCompletionResponse {
                content: "快捷键说明如上".into(),
                ..Default::default()
            },
        ]);

        let out = crate::ai::agent_runner::run(
            &model,
            &f.registry,
            &agent_request("墨池有哪些快捷键", &f.registry),
            |_| {},
        )
        .unwrap();

        assert_eq!(out.response.content, "快捷键说明如上");
        let tool_msg = out.transcript.iter().find(|m| m.role == "tool").unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(tool_msg.content.as_deref().unwrap()).unwrap();
        assert_eq!(payload["ok"], true, "{payload}");
        assert!(
            payload["data"]["content"].as_str().unwrap().len() > 50,
            "真的读到了说明书正文，而不是空壳"
        );
    }

    #[test]
    fn skill_loading_reveals_only_its_tools_on_the_next_model_round() {
        let mut f = fixture("progressive-rounds");
        let eligible = f.registry.advertise(&Selection::All, &BTreeMap::new());
        f.registry
            .restrict_to(eligible.iter().map(|t| t.function.name.clone()));
        f.registry.enable_progressive(eligible);
        let model = scripted(vec![
            wants("mochi_docs", r#"{"topic":"shortcuts"}"#), // 不能跳过发现阶段直接猜测工具名称。
            wants("skill_load", r#"{"id":"墨池产品知识"}"#),
            wants("mochi_docs", r#"{"topic":"shortcuts"}"#),
            crate::ai::models::AiCompletionResponse {
                content: "已查证".into(),
                ..Default::default()
            },
        ]);
        let out = crate::ai::agent_runner::run(
            &model,
            &f.registry,
            &agent_request("查快捷键", &f.registry),
            |_| {},
        )
        .unwrap();
        let requests = model.tool_names.borrow();
        assert_eq!(requests[0].len(), super::super::skill_tools::INITIAL.len());
        assert!(!requests[1].contains(&"mochi_docs".into()));
        assert!(requests[2].contains(&"mochi_docs".into()));
        assert!(!requests[2].contains(&"shell_run".into()));
        let results = out
            .transcript
            .iter()
            .filter(|m| m.role == "tool")
            .map(|m| {
                serde_json::from_str::<serde_json::Value>(m.content.as_deref().unwrap()).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(results[0]["ok"], false);
        assert_eq!(results[1]["ok"], true);
        assert_eq!(results[2]["ok"], true);
    }

    #[test]
    fn skill_loading_respects_the_agent_ceiling_host_availability_and_permissions() {
        let mut f = fixture_with_memory("progressive-ceiling", false);
        let eligible = f.registry.advertise(
            &Selection::List(vec![
                "skill_list".into(),
                "skill_load".into(),
                "file_read".into(),
            ]),
            &BTreeMap::new(),
        );
        f.registry
            .restrict_to(eligible.iter().map(|t| t.function.name.clone()));
        f.registry.enable_progressive(eligible);
        let result = f
            .registry
            .execute(&wants("skill_load", r#"{"id":"工作区文件"}"#).tool_calls[0]);
        let result: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["data"]["tools"], serde_json::json!(["file_read"]));
        assert!(!f.registry.handles("file_write"));
        assert!(!f.registry.handles("memory_write"));
        std::fs::write(f.root.join("secret.md"), "secret").unwrap();
        f.registry
            .permissions()
            .set_folder_permission(
                &f.root.to_string_lossy(),
                crate::ai::permission::AiPermissionLevel::Invisible,
            )
            .unwrap();
        let result: serde_json::Value = serde_json::from_str(
            &f.registry
                .execute(&wants("file_read", r#"{"path":"secret.md"}"#).tool_calls[0]),
        )
        .unwrap();
        assert_eq!(result["ok"], false);
    }

    /// 权限门必须在循环里也生效，且被拒时是「一条 ok:false 回给模型」而不是中断整轮——
    /// 模型据此改口告诉用户权限受限，这正是提示词里要求它做的。
    #[test]
    fn a_denied_tool_comes_back_as_an_error_result_not_an_abort() {
        let f = fixture("e2e-denied");
        let secret = f.root.join("私密");
        std::fs::create_dir_all(&secret).unwrap();
        f.registry
            .permissions()
            .set_folder_permission(
                &secret.to_string_lossy(),
                crate::ai::permission::AiPermissionLevel::ReadOnly,
            )
            .unwrap();

        let model = scripted(vec![
            wants("file_write", r#"{"path":"私密/a.md","content":"x"}"#),
            crate::ai::models::AiCompletionResponse {
                content: "该位置的 AI 权限受限".into(),
                ..Default::default()
            },
        ]);

        let out = crate::ai::agent_runner::run(
            &model,
            &f.registry,
            &agent_request("往私密目录写点东西", &f.registry),
            |_| {},
        )
        .unwrap();

        assert_eq!(
            out.response.content, "该位置的 AI 权限受限",
            "循环不该被中断"
        );
        let tool_msg = out.transcript.iter().find(|m| m.role == "tool").unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(tool_msg.content.as_deref().unwrap()).unwrap();
        assert_eq!(payload["ok"], false, "{payload}");
        assert!(
            payload["error"].as_str().unwrap().contains("只读"),
            "{payload}"
        );
        assert!(!secret.join("a.md").exists(), "被拒的写入绝不能落盘");
    }

    /// 广告给模型的工具清单就是注册表能执行的那些，一个不多一个不少。
    #[test]
    fn what_the_agent_advertises_is_exactly_what_it_can_run() {
        let f = fixture("e2e-advertise");
        let request = agent_request("随便", &f.registry);
        assert_eq!(
            request.tools.len(),
            definitions::count() - HOST_DEPENDENT.len()
        );
        for tool in &request.tools {
            let call = crate::ai::models::AiToolCall {
                id: "c".into(),
                kind: "function".into(),
                function: crate::ai::models::AiToolFunction {
                    name: tool.function.name.clone(),
                    arguments: "{}".into(),
                },
            };
            assert!(
                !f.registry.execute(&call).contains("未知工具"),
                "广告了 {} 却执行不了",
                tool.function.name
            );
        }
    }
}
