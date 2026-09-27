//! 未知 mochi_docs 主题仍返回 ok:true 和主题目录，供模型改正参数。

use std::sync::Arc;

use serde_json::{json, Value};

use super::host::ToolHost;
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::product_knowledge;

const TOOL_NAMES: &[&str] = &["mochi_docs", "agent_plan"];

/// 计划步骤的合法状态。模型给别的值一律当 `pending`——
/// 状态写错不该让整个计划作废。
const PLAN_STATUSES: &[&str] = &["pending", "in_progress", "done", "skipped"];

pub struct AssistToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl AssistToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    fn topic_outline() -> Vec<Value> {
        product_knowledge::topics()
            .iter()
            .map(|t| json!({ "topic": t.id, "title": t.title, "summary": t.summary }))
            .collect()
    }

    fn docs(&self, args: &ToolArgs) -> ToolOutcome {
        if let Some(id) = args
            .str_opt("topic")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return match product_knowledge::topic(id) {
                Some(topic) => Ok(json!({
                    "topic": topic.id, "title": topic.title, "content": topic.body,
                })),
                // 返回成功响应并附有效主题目录，允许调用方修正主题后重试。
                None => Ok(json!({
                    "error": format!("未知主题: {id}"),
                    "topics": Self::topic_outline(),
                })),
            };
        }

        if let Some(query) = args
            .str_opt("query")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let matched = product_knowledge::search(query);
            if matched.is_empty() {
                return Ok(json!({
                    "message": format!("没有主题匹配「{}」，以下是全部主题目录", query.to_lowercase()),
                    "topics": Self::topic_outline(),
                }));
            }
            return Ok(json!({
                "results": matched
                    .iter()
                    .map(|t| json!({ "topic": t.id, "title": t.title, "content": t.body }))
                    .collect::<Vec<_>>(),
            }));
        }

        Ok(json!({ "topics": Self::topic_outline() }))
    }

    fn plan(&self, args: &ToolArgs) -> ToolOutcome {
        let raw_steps = match args.get("steps") {
            Some(Value::Array(items)) if !items.is_empty() => items,
            _ => return Err("steps 需要至少一个 { title } 步骤".into()),
        };

        let mut steps = Vec::with_capacity(raw_steps.len());
        for item in raw_steps {
            // 模型有时直接给字符串数组而不是对象数组，两种都收
            let (title, status) = match item {
                Value::String(title) => (title.trim(), "pending"),
                Value::Object(object) => (
                    object
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or_default(),
                    object
                        .get("status")
                        .and_then(Value::as_str)
                        .filter(|s| PLAN_STATUSES.contains(s))
                        .unwrap_or("pending"),
                ),
                _ => return Err("steps 每一项都需要 { title } 或字符串".into()),
            };
            if title.is_empty() {
                return Err("步骤标题不能为空".into());
            }
            steps.push(json!({ "title": title, "status": status }));
        }

        let note = args.str_opt("note").map(str::to_owned);
        self.host.update_plan(&steps, note.as_deref());

        Ok(json!({
            "acknowledged": true,
            "steps": steps,
            "note": note,
        }))
    }
}

impl ToolExecutor for AssistToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    /// 都不碰工作区，无需权限。
    fn required_actions(
        &self,
        _name: &str,
        _args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        Vec::new()
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "mochi_docs" => self.docs(args),
            "agent_plan" => self.plan(args),
            other => Err(format!("未知工具: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{ActionPermissions, AiPermissionService};
    use crate::ai::tools::ToolRegistry;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    #[derive(Default)]
    struct PlanHost {
        root: PathBuf,
        plans: Mutex<Vec<(Vec<Value>, Option<String>)>>,
    }

    impl ToolHost for PlanHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn update_plan(&self, steps: &[Value], note: Option<&str>) {
            self.plans
                .lock()
                .unwrap()
                .push((steps.to_vec(), note.map(str::to_owned)));
        }
    }

    struct Fixture {
        root: PathBuf,
        host: Arc<PlanHost>,
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

    fn fixture(tag: &str) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-assist-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let host = Arc::new(PlanHost {
            root: root.clone(),
            plans: Mutex::new(Vec::new()),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry =
            ToolRegistry::new(perms).with(Arc::new(AssistToolExecutor::new(host.clone())));
        Fixture {
            root,
            host,
            registry,
        }
    }

    #[test]
    fn docs_without_arguments_returns_the_outline() {
        let f = fixture("outline");
        let data = f.ok("mochi_docs", json!({}));
        let topics = data["topics"].as_array().unwrap();
        assert_eq!(topics.len(), product_knowledge::topics().len());
        assert!(topics[0]["topic"].is_string() && topics[0]["summary"].is_string());
        assert!(data["results"].is_null(), "没给参数时不该有搜索结果");
    }

    #[test]
    fn docs_by_topic_returns_the_full_body() {
        let f = fixture("topic");
        let data = f.ok("mochi_docs", json!({ "topic": "shortcuts" }));
        assert_eq!(data["topic"], "shortcuts");
        assert!(data["content"].as_str().unwrap().len() > 50);
    }

    /// 主题不存在时仍返回 ok，附上目录让模型自己改正。
    #[test]
    fn an_unknown_topic_still_succeeds_with_the_outline() {
        let f = fixture("bad-topic");
        let data = f.ok("mochi_docs", json!({ "topic": "没这个主题" }));
        assert!(data["error"].as_str().unwrap().contains("未知主题"));
        assert_eq!(
            data["topics"].as_array().unwrap().len(),
            product_knowledge::topics().len(),
            "要给出完整目录，否则模型只会重试同一个错主题"
        );
    }

    #[test]
    fn docs_by_query_returns_up_to_three_topics() {
        let f = fixture("query");
        let data = f.ok("mochi_docs", json!({ "query": "设置" }));
        let results = data["results"].as_array().unwrap();
        assert!(!results.is_empty() && results.len() <= 3);
        assert!(results[0]["content"].as_str().unwrap().len() > 50);
    }

    #[test]
    fn a_query_with_no_match_falls_back_to_the_outline() {
        let f = fixture("no-match");
        let data = f.ok("mochi_docs", json!({ "query": "绝无可能出现的关键词zzz" }));
        assert!(data["message"].as_str().unwrap().contains("没有主题匹配"));
        assert!(!data["topics"].as_array().unwrap().is_empty());
    }

    /// topic 优先于 query，与 TS 的分支顺序一致。
    #[test]
    fn topic_takes_precedence_over_query() {
        let f = fixture("both");
        let data = f.ok("mochi_docs", json!({ "topic": "editor", "query": "设置" }));
        assert_eq!(data["topic"], "editor");
    }

    #[test]
    fn plan_normalizes_steps_and_reaches_the_host() {
        let f = fixture("plan");
        let data = f.ok(
            "agent_plan",
            json!({ "steps": [
                { "title": "读取文档", "status": "done" },
                { "title": "生成摘要", "status": "in_progress" },
                { "title": "写回文件" },
            ], "note": "开工" }),
        );

        assert_eq!(data["acknowledged"], true);
        assert_eq!(data["steps"][0]["status"], "done");
        assert_eq!(data["steps"][2]["status"], "pending", "缺省状态是 pending");
        assert_eq!(data["note"], "开工");

        let plans = f.host.plans.lock().unwrap();
        assert_eq!(plans.len(), 1, "计划要送到宿主，UI 才渲染得出步骤清单");
        assert_eq!(plans[0].0.len(), 3);
        assert_eq!(plans[0].1.as_deref(), Some("开工"));
    }

    /// 模型有时直接给字符串数组。
    #[test]
    fn plain_string_steps_are_accepted() {
        let f = fixture("plan-strings");
        let data = f.ok("agent_plan", json!({ "steps": ["第一步", "第二步"] }));
        assert_eq!(data["steps"][0]["title"], "第一步");
        assert_eq!(data["steps"][0]["status"], "pending");
    }

    /// 状态写错不该让整个计划作废。
    #[test]
    fn unknown_statuses_fall_back_to_pending() {
        let f = fixture("plan-status");
        let data = f.ok(
            "agent_plan",
            json!({ "steps": [{ "title": "干活", "status": "进行中" }] }),
        );
        assert_eq!(data["steps"][0]["status"], "pending");
    }

    #[test]
    fn an_empty_plan_is_refused() {
        let f = fixture("plan-empty");
        assert!(f.err("agent_plan", json!({})).contains("steps"));
        assert!(f
            .err("agent_plan", json!({ "steps": [] }))
            .contains("steps"));
        assert!(f
            .err("agent_plan", json!({ "steps": [{ "title": "  " }] }))
            .contains("标题"));
        assert!(f
            .err("agent_plan", json!({ "steps": [123] }))
            .contains("title"));
    }

    /// 这两个工具不碰磁盘，关掉所有文件权限后照样能用。
    #[test]
    fn neither_tool_needs_any_permission() {
        let f = fixture("perm");
        let mut actions = ActionPermissions::default();
        for action in AiToolAction::ALL {
            actions.set(action, false);
        }
        f.registry.permissions().set_action_permissions(actions);

        assert_eq!(f.run("mochi_docs", json!({}))["ok"], true);
        assert_eq!(
            f.run("agent_plan", json!({ "steps": ["干活"] }))["ok"],
            true
        );
    }
}
