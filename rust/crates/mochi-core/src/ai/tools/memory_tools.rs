//! 记忆读写同样受文件权限约束：search → read_file，write → write_file。

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::host::ToolHost;
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::memory::{MemoryKind, MemoryScope, MemorySearchOptions, MemoryStore, MemoryWriteInput};

const TOOL_NAMES: &[&str] = &["memory_search", "memory_write"];

pub struct MemoryToolExecutor {
    /// `rusqlite::Connection` 是 `Send` 但不是 `Sync`，而工具执行器要求 `Sync`。
    store: Arc<Mutex<MemoryStore>>,
    host: Arc<dyn ToolHost>,
    settings: Option<Arc<crate::app_settings::AppSettings>>,
}

impl MemoryToolExecutor {
    pub fn new(store: Arc<Mutex<MemoryStore>>, host: Arc<dyn ToolHost>) -> Self {
        Self {
            store,
            host,
            settings: None,
        }
    }
    pub fn with_settings(mut self, settings: Arc<crate::app_settings::AppSettings>) -> Self {
        self.settings = Some(settings);
        self
    }
    fn writes_enabled(&self) -> bool {
        self.settings
            .as_ref()
            .and_then(|s| {
                crate::app_settings::descriptor("ai.memoryAutoExtract")
                    .map(|d| matches!(s.read(d), crate::app_settings::SettingValue::Bool(true)))
            })
            .unwrap_or(true)
    }
}

impl ToolExecutor for MemoryToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name) && (name != "memory_write" || self.writes_enabled())
    }

    fn required_actions(
        &self,
        name: &str,
        _args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        // 记忆库在 .mochi 下，不属于任何用户目录，所以只做动作级检查、不带路径
        match name {
            "memory_write" => vec![(AiToolAction::WriteFile, None)],
            _ => vec![(AiToolAction::ReadFile, None)],
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if name == "memory_write" && !self.writes_enabled() {
            return Err("AI 自动记忆已关闭，未写入长期记忆".into());
        }
        let store = self.store.lock().map_err(|_| "记忆库锁已中毒".to_owned())?;

        match name {
            "memory_search" => {
                let records = store
                    .search(&MemorySearchOptions {
                        query: args.str_opt("query").map(str::to_owned),
                        kind: None,
                        scope: args.str_opt("scope").map(MemoryScope::parse),
                        scope_ref: args.str_opt("scopeRef").map(str::to_owned),
                        limit: args.i64_opt("limit"),
                    })
                    .map_err(|e| format!("检索记忆失败: {e}"))?;
                Ok(json!({ "count": records.len(), "memories": records }))
            }

            "memory_write" => {
                let title = args.str_required("title")?.trim().to_owned();
                let content = args
                    .str_opt("content")
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if title.is_empty() {
                    return Err("记忆标题不能为空".into());
                }
                if content.is_empty() {
                    return Err("记忆内容不能为空".into());
                }

                let record = store
                    .write(MemoryWriteInput {
                        kind: Some(
                            args.str_opt("kind")
                                .map_or(MemoryKind::Fact, MemoryKind::parse),
                        ),
                        scope: Some(
                            args.str_opt("scope")
                                .map_or(MemoryScope::Global, MemoryScope::parse),
                        ),
                        scope_ref: args.str_opt("scopeRef").map(str::to_owned),
                        title,
                        content,
                        tags: args.str_array("tags"),
                        session_id: self.host.active_session_id(),
                        upsert: None,
                    })
                    .map_err(|e| format!("写入记忆失败: {e}"))?;
                Ok(json!({ "memory": record }))
            }

            other => Err(format!("未知的记忆工具: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{ActionPermissions, AiPermissionService};
    use crate::ai::tools::ToolRegistry;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct SessionHost {
        root: PathBuf,
        session: Option<String>,
    }

    impl ToolHost for SessionHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn active_session_id(&self) -> Option<String> {
            self.session.clone()
        }
    }

    struct Fixture {
        root: PathBuf,
        store: Arc<Mutex<MemoryStore>>,
        registry: ToolRegistry,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Fixture {
        fn run(&self, name: &str, args: Value) -> Value {
            let call = AiToolCall {
                id: "c".into(),
                kind: "function".into(),
                function: AiToolFunction {
                    name: name.into(),
                    arguments: args.to_string(),
                },
            };
            serde_json::from_str(&self.registry.execute(&call)).unwrap()
        }
        fn ok(&self, name: &str, args: Value) -> Value {
            let out = self.run(name, args);
            assert_eq!(out["ok"], true, "{out}");
            out["data"].clone()
        }
        fn err(&self, name: &str, args: Value) -> String {
            let out = self.run(name, args);
            assert_eq!(out["ok"], false, "{out}");
            out["error"].as_str().unwrap().to_owned()
        }
    }

    fn fixture(tag: &str, session: Option<&str>) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-memtool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let store = Arc::new(Mutex::new(MemoryStore::open(&root).unwrap()));
        let host = Arc::new(SessionHost {
            root: root.clone(),
            session: session.map(str::to_owned),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry =
            ToolRegistry::new(perms).with(Arc::new(MemoryToolExecutor::new(store.clone(), host)));
        Fixture {
            root,
            store,
            registry,
        }
    }

    #[test]
    fn automatic_memory_setting_is_checked_during_the_run() {
        let f = fixture("memory-setting", None);
        let settings = Arc::new(crate::app_settings::AppSettings::new(Arc::new(
            crate::settings::SettingsService::new(Some(f.root.join("settings.json"))),
        )));
        let executor = MemoryToolExecutor::new(
            f.store.clone(),
            Arc::new(SessionHost {
                root: f.root.clone(),
                session: None,
            }),
        )
        .with_settings(settings.clone());
        let descriptor = crate::app_settings::descriptor("ai.memoryAutoExtract").unwrap();
        assert!(executor.handles("memory_write"));
        settings.write(descriptor, &crate::app_settings::SettingValue::Bool(false));
        assert!(!executor.handles("memory_write"));
        assert!(executor.handles("memory_search"));
        settings.write(descriptor, &crate::app_settings::SettingValue::Bool(true));
        assert!(executor.handles("memory_write"));
    }

    #[test]
    fn write_then_search_round_trips_through_the_tools() {
        let f = fixture("roundtrip", Some("sess-9"));
        let written = f.ok(
            "memory_write",
            json!({ "title": "构建命令", "content": "用 pnpm run electron:dev 启动",
                    "tags": ["构建", "命令"] }),
        );
        assert_eq!(written["memory"]["title"], "构建命令");
        assert_eq!(written["memory"]["kind"], "fact", "缺省类型是 fact");
        assert_eq!(written["memory"]["scope"], "global");
        assert_eq!(
            written["memory"]["sessionId"], "sess-9",
            "应带上当前会话 id"
        );
        assert_eq!(written["memory"]["tags"], json!(["构建", "命令"]));

        let found = f.ok("memory_search", json!({ "query": "pnpm" }));
        assert_eq!(found["count"], 1);
        assert_eq!(found["memories"][0]["title"], "构建命令");
    }

    /// 中文检索要能命中——这是本地用户的主要用法。
    #[test]
    fn chinese_queries_find_their_memory() {
        let f = fixture("chinese", None);
        f.ok(
            "memory_write",
            json!({ "title": "回答风格", "content": "希望回答尽量简洁" }),
        );
        let found = f.ok("memory_search", json!({ "query": "简洁" }));
        assert_eq!(found["count"], 1, "{found}");
    }

    #[test]
    fn scope_and_kind_are_passed_through() {
        let f = fixture("scope", None);
        let written = f.ok(
            "memory_write",
            json!({ "title": "这个库", "content": "存的是面试题", "kind": "pinned",
                    "scope": "library", "scopeRef": "D:/ws/知识库" }),
        );
        assert_eq!(written["memory"]["kind"], "pinned");
        assert_eq!(written["memory"]["scope"], "library");
        assert_eq!(written["memory"]["scopeRef"], "D:/ws/知识库");

        let scoped = f.ok(
            "memory_search",
            json!({ "scope": "library", "scopeRef": "D:/ws/知识库" }),
        );
        assert_eq!(scoped["count"], 1);
        assert_eq!(
            f.ok("memory_search", json!({ "scope": "document" }))["count"],
            0
        );
    }

    /// 模型自创的类型不该让整轮对话失败，退回 fact 即可。
    #[test]
    fn unknown_kinds_fall_back_to_fact() {
        let f = fixture("badkind", None);
        let written = f.ok(
            "memory_write",
            json!({ "title": "t", "content": "c", "kind": "灵光一闪" }),
        );
        assert_eq!(written["memory"]["kind"], "fact");
    }

    #[test]
    fn empty_title_or_content_is_refused() {
        let f = fixture("empty", None);
        assert!(f
            .err("memory_write", json!({ "content": "只有正文" }))
            .contains("title"));
        assert!(f
            .err("memory_write", json!({ "title": "   ", "content": "x" }))
            .contains("标题"));
        assert!(f
            .err("memory_write", json!({ "title": "t", "content": "   " }))
            .contains("内容"));
        assert!(f
            .err("memory_write", json!({ "title": "t" }))
            .contains("内容"));
    }

    /// 同标题覆盖更新，避免记忆越堆越重复。
    #[test]
    fn writing_the_same_title_twice_updates_in_place() {
        let f = fixture("upsert", None);
        let first = f.ok(
            "memory_write",
            json!({ "title": "偏好", "content": "简洁" }),
        );
        let second = f.ok(
            "memory_write",
            json!({ "title": "偏好", "content": "详细" }),
        );
        assert_eq!(first["memory"]["id"], second["memory"]["id"]);
        assert_eq!(f.store.lock().unwrap().list(None).unwrap().len(), 1);
    }

    #[test]
    fn search_without_a_query_returns_recent_memories() {
        let f = fixture("recent", None);
        for i in 0..3 {
            f.ok(
                "memory_write",
                json!({ "title": format!("记忆{i}"), "content": "x" }),
            );
        }
        let found = f.ok("memory_search", json!({}));
        assert_eq!(found["count"], 3);

        let limited = f.ok("memory_search", json!({ "limit": 2 }));
        assert_eq!(limited["count"], 2);
    }

    /// 关掉「写入文件」后不该还能往磁盘写记忆——这是与 TS 的有意偏离。
    #[test]
    fn disabling_file_writes_also_blocks_memory_writes() {
        let f = fixture("perm", None);
        let mut actions = ActionPermissions::default();
        actions.set(AiToolAction::WriteFile, false);
        f.registry.permissions().set_action_permissions(actions);

        assert!(!f
            .err("memory_write", json!({ "title": "t", "content": "c" }))
            .is_empty());
        assert_eq!(f.store.lock().unwrap().list(None).unwrap().len(), 0);
        // 只读检索不受影响
        assert_eq!(f.ok("memory_search", json!({}))["count"], 0);
    }
}
