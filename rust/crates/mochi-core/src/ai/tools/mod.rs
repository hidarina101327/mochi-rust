//! 工具失败字段使用 error，不是 errorMessage。
//! ToolRegistry 在调用前统一检查 required_action；未声明权限的工具默认拒绝。

pub mod agenda_tools;
pub mod assist_tools;
pub mod base_automation_tools;
pub mod base_tools;
pub mod block_tools;
pub mod canvas_tools;
pub mod console_tools;
pub mod core;
pub mod definitions;
pub mod desktop_tools;
pub mod document_tools;
pub mod git_tools;
pub mod host;
pub mod memory_tools;
pub mod range_edit_tools;
pub mod registry;
pub mod script_tools;
pub mod settings_tools;
pub mod shell_tools;
pub mod skill_tools;
pub mod structured_manage;
pub mod sub_document_tools;
pub mod workflow_tools;
pub mod workspace_tools;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::agent_config::Selection;
use super::models::{AiToolCall, AiToolDefinition};
use super::permission::{AiPermissionService, AiToolAction};

/// 工具执行结果：`Ok(data)` 会被包成 `{ok:true,data}`，`Err(msg)` 包成 `{ok:false,error}`。
pub type ToolOutcome = Result<Value, String>;

/// 解析好的工具入参。模型给的是 JSON 字符串，可能是空串或非法 JSON。
pub struct ToolArgs {
    raw: Value,
}

impl ToolArgs {
    /// 解析 `function.arguments`。空串或非法 JSON 一律当空对象——
    /// 模型偶尔会漏给参数，不该因此崩掉整轮对话。
    pub fn parse(arguments: &str) -> Self {
        let trimmed = arguments.trim();
        let raw = if trimmed.is_empty() {
            json!({})
        } else {
            serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
        };
        Self { raw }
    }

    pub fn from_value(raw: Value) -> Self {
        Self { raw }
    }

    pub fn value(&self) -> &Value {
        &self.raw
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.raw.get(key)
    }

    /// 可选字符串。空串视为缺失（对齐 TS 的 `optionalString`）。
    pub fn str_opt(&self, key: &str) -> Option<&str> {
        self.raw
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// 必填字符串。缺失时给出**明确指出参数名**的错误——模型能据此自我纠正。
    pub fn str_required(&self, key: &str) -> Result<&str, String> {
        self.str_opt(key)
            .ok_or_else(|| format!("缺少必填参数 {key}"))
    }

    pub fn bool_opt(&self, key: &str) -> Option<bool> {
        self.raw.get(key).and_then(Value::as_bool)
    }

    pub fn i64_opt(&self, key: &str) -> Option<i64> {
        self.raw.get(key).and_then(|v| {
            v.as_i64()
                // 模型经常把数字写成字符串
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
    }

    /// 字符串数组。单个字符串也接受（包成一元数组），对齐 TS 的 `stringArray`。
    pub fn str_array(&self, key: &str) -> Vec<String> {
        match self.raw.get(key) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
    }
}

/// 一个工具执行器负责一组工具。
pub trait ToolExecutor: Send + Sync {
    /// 本执行器认哪些工具名。
    fn handles(&self, name: &str) -> bool;

    /// 该工具需要的权限动作与受影响的绝对路径，**可能不止一条**
    /// （`path_rename` 就要同时检查旧路径和新路径——只查一个会留下缺口）。
    ///
    /// 返回空表示纯读取的元信息类工具（如 `workspace_get_state`），不需要门禁。
    /// **实现方只需如实声明，强制由 `ToolRegistry` 完成**——这样漏写的后果是过严而非越权。
    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)>;

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome;
}

/// 按名字把工具分发给对应执行器，并在调用前统一过权限门。
pub struct ToolRegistry {
    executors: Vec<Arc<dyn ToolExecutor>>,
    permissions: Arc<AiPermissionService>,
    allowed: Option<std::collections::HashSet<String>>,
    skill_session: Option<Arc<std::sync::Mutex<skill_tools::SkillSession>>>,
}

impl ToolRegistry {
    pub fn new(permissions: Arc<AiPermissionService>) -> Self {
        Self {
            executors: Vec::new(),
            permissions,
            allowed: None,
            skill_session: None,
        }
    }

    pub fn with(mut self, executor: Arc<dyn ToolExecutor>) -> Self {
        self.executors.push(executor);
        self
    }

    pub fn register(&mut self, executor: Arc<dyn ToolExecutor>) {
        self.executors.push(executor);
    }

    pub fn install_skills(&mut self, skills: Vec<super::agent_config::SkillDefinition>) {
        let session = Arc::new(std::sync::Mutex::new(skill_tools::SkillSession::new(
            skills,
        )));
        self.register(Arc::new(skill_tools::SkillToolExecutor(session.clone())));
        self.skill_session = Some(session);
    }

    /// 资格上限在披露之前就定死；Skill 文本无法把它撑大。
    pub fn enable_progressive(&mut self, eligible: Vec<AiToolDefinition>) {
        let eligible = eligible
            .into_iter()
            .filter(|t| {
                self.allowed
                    .as_ref()
                    .is_none_or(|names| names.contains(&t.function.name))
                    && self.executors.iter().any(|e| e.handles(&t.function.name))
            })
            .collect();
        if let Some(session) = &self.skill_session {
            session.lock().expect("Skill session").enable(eligible);
        }
    }

    pub fn disclosed_tools(&self) -> Option<Vec<AiToolDefinition>> {
        self.skill_session.as_ref()?.lock().ok()?.definitions()
    }
    pub fn loaded_skills(&self) -> Vec<String> {
        self.skill_session
            .as_ref()
            .and_then(|s| s.lock().ok().map(|s| s.loaded.clone()))
            .unwrap_or_default()
    }

    /// 把本次请求宣称可用的工具名固定下来，作为执行前的又一道闸。
    /// 空列表就是刻意禁用所有工具；None 保留给老调用方。
    pub fn restrict_to(&mut self, names: impl IntoIterator<Item = String>) {
        self.allowed = Some(names.into_iter().collect());
    }

    pub fn permissions(&self) -> &Arc<AiPermissionService> {
        &self.permissions
    }

    /// 本注册表能不能执行这个工具。
    pub fn handles(&self, name: &str) -> bool {
        self.allowed
            .as_ref()
            .is_none_or(|names| names.contains(name))
            && self.executors.iter().any(|e| e.handles(name))
            && self
                .skill_session
                .as_ref()
                .is_none_or(|s| s.lock().is_ok_and(|s| s.allows(name)))
    }

    /// 认领这个工具名的执行器个数。**只该是 0 或 1**——`dispatch` 用的是
    /// `find`（先到先得），两个执行器同时认领同一个名字时后者会被静默忽略。
    /// 供组装处的测试查重用。
    pub fn executor_count_for(&self, name: &str) -> usize {
        self.executors.iter().filter(|e| e.handles(name)).count()
    }

    /// 发给模型的工具清单。**只包含本注册表真正能执行的工具**——
    /// 广告了却执行不了，模型会调用它、拿回「未知工具」、再白白花一轮去猜自己
    /// 哪里写错了，而错的并不是它。详见 `registry` 模块。
    pub fn advertise(
        &self,
        selection: &Selection,
        overrides: &BTreeMap<String, String>,
    ) -> Vec<AiToolDefinition> {
        definitions::resolve(selection, overrides)
            .into_iter()
            .filter(|t| self.handles(&t.function.name))
            .collect()
    }

    /// 执行一次工具调用，返回**要回给模型的 JSON 字符串**。
    ///
    /// 任何失败（未知工具、参数非法、权限拒绝、执行出错）都变成
    /// `{"ok":false,"error":"…"}` 而不是抛出——对齐 TS 顶层那个 catch 包装器。
    /// 模型看到错误后能自己换个方式重试，抛出去只会中断整轮对话。
    pub fn execute(&self, call: &AiToolCall) -> String {
        let name = call.function.name.as_str();
        if self
            .skill_session
            .as_ref()
            .is_some_and(|s| s.lock().map_or(true, |s| !s.allows(name)))
        {
            return serialize_outcome(Err(format!(
                "工具 {name} 尚未披露，请先用 skill_list / skill_load 加载对应能力"
            )));
        }
        if self
            .allowed
            .as_ref()
            .is_some_and(|names| !names.contains(name))
        {
            return serialize_outcome(Err(format!("当前 Agent 未启用工具 {name}")));
        }
        let args = ToolArgs::parse(&call.function.arguments);
        let outcome = self.dispatch(name, &args);
        serialize_outcome(outcome)
    }

    fn dispatch(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let Some(executor) = self.executors.iter().find(|e| e.handles(name)) else {
            return Err(format!("未知工具: {name}"));
        };

        for (action, path) in executor.required_actions(name, args) {
            self.permissions
                .assert_tool_action_allowed(action, path.as_deref())
                .map_err(|denied| denied.message)?;
        }

        executor.call(name, args)
    }
}

pub fn serialize_outcome(outcome: ToolOutcome) -> String {
    let value = match outcome {
        Ok(data) => json!({ "ok": true, "data": data }),
        Err(error) => json!({ "ok": false, "error": error }),
    };
    // 返回给模型的报文使用紧凑格式，避免缩进额外占用 Token。
    serde_json::to_string(&value)
        .unwrap_or_else(|_| r#"{"ok":false,"error":"结果序列化失败"}"#.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::AiToolFunction;
    use crate::ai::permission::AiPermissionLevel;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_ws(tag: &str) -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("mochi-toolreg-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn call(name: &str, arguments: &str) -> AiToolCall {
        AiToolCall {
            id: "c1".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    /// 一个只记录「被调用了几次」的假执行器。
    struct Fake {
        action: Option<(AiToolAction, Option<String>)>,
        calls: Mutex<usize>,
    }

    impl ToolExecutor for Fake {
        fn handles(&self, name: &str) -> bool {
            name == "fake_tool"
        }
        fn required_actions(&self, _: &str, _: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
            self.action.clone().into_iter().collect()
        }
        fn call(&self, _: &str, args: &ToolArgs) -> ToolOutcome {
            *self.calls.lock().unwrap() += 1;
            Ok(json!({ "echo": args.str_opt("msg").unwrap_or("") }))
        }
    }

    fn registry(
        action: Option<(AiToolAction, Option<String>)>,
        ws: &std::path::Path,
    ) -> (ToolRegistry, Arc<Fake>) {
        let fake = Arc::new(Fake {
            action,
            calls: Mutex::new(0),
        });
        let perms = Arc::new(AiPermissionService::new(ws));
        let reg = ToolRegistry::new(perms).with(fake.clone());
        (reg, fake)
    }

    #[test]
    fn request_allowlist_rejects_unadvertised_calls_before_dispatch() {
        let ws = temp_ws("request-scope");
        let (mut registry, fake) = registry(None, &ws);
        registry.restrict_to(Vec::<String>::new());
        assert!(!registry.handles("fake_tool"));
        let denied: Value =
            serde_json::from_str(&registry.execute(&call("fake_tool", "{}"))).unwrap();
        assert_eq!(denied["ok"], false);
        assert_eq!(*fake.calls.lock().unwrap(), 0);
        registry.restrict_to(vec!["fake_tool".into()]);
        let allowed: Value =
            serde_json::from_str(&registry.execute(&call("fake_tool", "{}"))).unwrap();
        assert_eq!(allowed["ok"], true);
        assert_eq!(*fake.calls.lock().unwrap(), 1);
        std::fs::remove_dir_all(ws).unwrap();
    }

    #[test]
    fn args_parse_tolerates_empty_and_broken_json() {
        assert!(ToolArgs::parse("").value().is_object());
        assert!(ToolArgs::parse("   ").value().is_object());
        assert!(ToolArgs::parse("{不是 JSON").value().is_object());
        assert_eq!(ToolArgs::parse(r#"{"a":1}"#).i64_opt("a"), Some(1));
    }

    #[test]
    fn args_accessors() {
        let a = ToolArgs::parse(
            r#"{"s":"x","empty":"","n":"12","b":true,"arr":["p","","q"],"one":"solo"}"#,
        );
        assert_eq!(a.str_opt("s"), Some("x"));
        assert_eq!(a.str_opt("empty"), None, "空串应视为缺失");
        assert_eq!(a.str_opt("缺失"), None);
        assert_eq!(a.i64_opt("n"), Some(12), "模型常把数字写成字符串");
        assert_eq!(a.bool_opt("b"), Some(true));
        assert_eq!(a.str_array("arr"), ["p", "q"], "空项应被剔除");
        assert_eq!(a.str_array("one"), ["solo"], "单个字符串应包成一元数组");
        assert_eq!(a.str_array("缺失"), Vec::<String>::new());
    }

    #[test]
    fn required_arg_error_names_the_parameter() {
        let err = ToolArgs::parse("{}").str_required("path").unwrap_err();
        assert!(
            err.contains("path"),
            "错误信息里要点名参数，模型才能自我纠正: {err}"
        );
    }

    #[test]
    fn success_shape_matches_ts() {
        let ws = temp_ws("ok");
        let (reg, _) = registry(None, &ws);
        assert_eq!(
            reg.execute(&call("fake_tool", r#"{"msg":"你好"}"#)),
            r#"{"ok":true,"data":{"echo":"你好"}}"#
        );
    }

    /// 键名是 error 不是 errorMessage——C# 的 agent runner 写错了。
    #[test]
    fn unknown_tool_returns_error_shape_not_panic() {
        let ws = temp_ws("unknown");
        let (reg, _) = registry(None, &ws);
        let out = reg.execute(&call("没这个工具", "{}"));
        assert!(out.starts_with(r#"{"ok":false,"error":"#), "{out}");
        assert!(out.contains("未知工具"));
        assert!(!out.contains("errorMessage"), "键名必须是 error");
    }

    /// 核心设计：执行器只声明需要什么权限，强制由 registry 完成，漏不掉。
    #[test]
    fn registry_enforces_permission_before_calling_the_tool() {
        let ws = temp_ws("gate");
        let target = ws.join("私密").join("a.md").to_string_lossy().into_owned();
        let (reg, fake) = registry(Some((AiToolAction::WriteFile, Some(target.clone()))), &ws);
        reg.permissions()
            .set_folder_permission(
                &ws.join("私密").to_string_lossy(),
                AiPermissionLevel::ReadOnly,
            )
            .unwrap();

        let out = reg.execute(&call("fake_tool", "{}"));
        assert!(
            out.contains(r#""ok":false"#),
            "只读目录下的写入应被拒: {out}"
        );
        assert!(out.contains("只读"), "{out}");
        assert_eq!(*fake.calls.lock().unwrap(), 0, "被拒的工具根本不该被执行");
    }

    #[test]
    fn permitted_action_reaches_the_tool() {
        let ws = temp_ws("gate-ok");
        let target = ws.join("公开").join("a.md").to_string_lossy().into_owned();
        let (reg, fake) = registry(Some((AiToolAction::WriteFile, Some(target))), &ws);

        let out = reg.execute(&call("fake_tool", "{}"));
        assert!(out.contains(r#""ok":true"#), "{out}");
        assert_eq!(*fake.calls.lock().unwrap(), 1);
    }

    #[test]
    fn disabled_action_blocks_even_without_a_path() {
        let ws = temp_ws("gate-action");
        let (reg, fake) = registry(Some((AiToolAction::ExecuteCommand, None)), &ws);
        // execute_command 默认关闭
        let out = reg.execute(&call("fake_tool", "{}"));
        assert!(out.contains("执行命令"), "{out}");
        assert_eq!(*fake.calls.lock().unwrap(), 0);
    }

    #[test]
    fn tools_without_a_declared_action_run_unguarded() {
        let ws = temp_ws("no-action");
        let (reg, fake) = registry(None, &ws);
        reg.execute(&call("fake_tool", "{}"));
        assert_eq!(*fake.calls.lock().unwrap(), 1, "元信息类工具不该被门禁挡住");
    }

    #[test]
    fn outcome_serialization_is_compact() {
        let out = serialize_outcome(Ok(json!({"a": 1})));
        assert!(
            !out.contains('\n'),
            "回给模型的报文不该带缩进，白吃 token: {out}"
        );
    }
}
