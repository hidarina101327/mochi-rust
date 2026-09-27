//! 整批校验成功后再应用设置；发生钳制必须返回 clamped。

use std::sync::Arc;

use serde_json::{json, Value};

use super::host::ToolHost;
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::app_settings::{self, AppSettings, SettingDescriptor, SettingValue};

const TOOL_NAMES: &[&str] = &[
    "settings_list",
    "settings_get",
    "settings_update",
    "settings_reset",
    "settings_open_ui",
];

const DEFAULT_UI_TAB: &str = "general";

pub struct SettingsToolExecutor {
    settings: Arc<AppSettings>,
    host: Arc<dyn ToolHost>,
}

impl SettingsToolExecutor {
    pub fn new(settings: Arc<AppSettings>, host: Arc<dyn ToolHost>) -> Self {
        Self { settings, host }
    }
    fn snapshot(&self, descriptor: &SettingDescriptor) -> Value {
        let mut value = self.settings.snapshot(descriptor);
        if let Some(fixed) = self.host.fixed_setting(&descriptor.key) {
            value["value"] = json!(fixed);
            value["defaultValue"] = json!(fixed);
            value["isDefault"] = true.into();
            value["readOnly"] = true.into();
        }
        value
    }

    /// 按 key 取描述符。找不到时**顺手给出相近的 key**——模型据此自我纠正，
    /// 比单纯报错省一轮往返。
    fn resolve(&self, key: &str) -> Result<&'static SettingDescriptor, String> {
        let key = key.trim();
        if let Some(found) = app_settings::descriptor(key) {
            return Ok(found);
        }
        let guesses: Vec<&str> = app_settings::search(key)
            .into_iter()
            .take(5)
            .map(|d| d.key.as_str())
            .collect();
        let hint = if guesses.is_empty() {
            String::new()
        } else {
            format!("；相近的 key：{}", guesses.join(", "))
        };
        Err(format!(
            "未知设置 key: {key}{hint}。先用 settings_list 查询可用设置。"
        ))
    }

    fn resolve_all(&self, keys: &[String]) -> Result<Vec<&'static SettingDescriptor>, String> {
        keys.iter().map(|k| self.resolve(k)).collect()
    }

    fn require_category<'a>(&self, id: &'a str) -> Result<&'a str, String> {
        if app_settings::is_category(id) {
            Ok(id)
        } else {
            Err(format!(
                "未知分类: {id}。可用分类：{}",
                app_settings::category_ids().join(", ")
            ))
        }
    }

    fn applied_entry(
        &self,
        descriptor: &SettingDescriptor,
        previous: SettingValue,
        clamped: bool,
    ) -> Value {
        json!({
            "key": descriptor.key,
            "label": descriptor.label,
            "previousValue": previous,
            // 回读而不是回显入参：钳制之后的真实值才是用户看到的
            "value": self.settings.read(descriptor),
            "clamped": clamped,
        })
    }

    fn list(&self, args: &ToolArgs) -> ToolOutcome {
        let mut matched: Vec<&'static SettingDescriptor> =
            app_settings::descriptors().iter().collect();

        if let Some(category) = args
            .str_opt("category")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let category = self.require_category(category)?;
            matched.retain(|d| d.category == category);
        }
        if let Some(query) = args
            .str_opt("query")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let hits: Vec<&str> = app_settings::search(query)
                .into_iter()
                .map(|d| d.key.as_str())
                .collect();
            matched.retain(|d| hits.contains(&d.key.as_str()));
        }

        // 分类的出现顺序 = 在注册表里首次出现的顺序，对齐 TS 那个 Map 的插入序
        let mut order: Vec<&str> = Vec::new();
        let mut buckets: Vec<Vec<Value>> = Vec::new();
        for descriptor in &matched {
            let idx = match order.iter().position(|id| *id == descriptor.category) {
                Some(i) => i,
                None => {
                    order.push(&descriptor.category);
                    buckets.push(Vec::new());
                    buckets.len() - 1
                }
            };
            buckets[idx].push(self.snapshot(descriptor));
        }

        Ok(json!({
            "total": matched.len(),
            "categories": order
                .iter()
                .zip(buckets)
                .map(|(id, settings)| json!({
                    "id": id,
                    "label": app_settings::category_label(id).unwrap_or_default(),
                    "settings": settings,
                }))
                .collect::<Vec<_>>(),
        }))
    }

    fn get(&self, args: &ToolArgs) -> ToolOutcome {
        let keys = require_string_array(args, "keys")?;
        if keys.is_empty() {
            return Err("keys 不能为空".into());
        }
        let descriptors = self.resolve_all(&keys)?;
        Ok(json!({
            "settings": descriptors
                .iter()
                .map(|d| self.snapshot(d))
                .collect::<Vec<_>>(),
        }))
    }

    fn update(&self, args: &ToolArgs) -> ToolOutcome {
        let raw_changes = match args.get("changes") {
            Some(Value::Array(items)) if !items.is_empty() => items,
            _ => return Err("changes 需要至少一项 { key, value }".into()),
        };

        // 第一遍只校验，一项不合法就整批放弃——半套生效的设置比不改更糟
        let mut prepared = Vec::with_capacity(raw_changes.len());
        for item in raw_changes {
            let Some(object) = item.as_object() else {
                return Err("changes 每一项都需要 { key, value }".into());
            };
            let key = object
                .get("key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("changes 项缺少 key")?;
            let descriptor = self.resolve(key)?;
            if self.host.fixed_setting(&descriptor.key).is_some() {
                return Err("此设置由原生编辑器架构固定，不能通过工具修改".into());
            }
            let coerced =
                app_settings::coerce(descriptor, object.get("value").unwrap_or(&Value::Null))?;
            if (descriptor.key == "ai.editApplyMode" && coerced.value.to_storage() == "auto")
                || (descriptor.key == "ai.requirePermissionPrompt"
                    && coerced.value.to_storage() == "false")
            {
                return Err(
                    "降低 AI 审批要求必须由用户在设置界面确认，不能由 AI 工具自行关闭".into(),
                );
            }
            prepared.push((descriptor, coerced));
        }

        let applied: Vec<Value> = prepared
            .into_iter()
            .map(|(descriptor, coerced)| {
                let previous = self.settings.read(descriptor);
                self.settings.write(descriptor, &coerced.value);
                self.applied_entry(descriptor, previous, coerced.clamped)
            })
            .collect();

        self.settings
            .flush()
            .map_err(|e| format!("设置写盘失败: {e}"))?;
        Ok(json!({
            "applied": applied,
            "summary": format!("已修改 {} 项设置", applied.len()),
        }))
    }

    fn reset(&self, args: &ToolArgs) -> ToolOutcome {
        let mut targets: Vec<&'static SettingDescriptor> = match args.get("keys") {
            Some(Value::Null) | None => Vec::new(),
            Some(_) => self.resolve_all(&require_string_array(args, "keys")?)?,
        };

        if let Some(category) = args
            .str_opt("category")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let category = self.require_category(category)?;
            targets.extend(
                app_settings::descriptors()
                    .iter()
                    .filter(|d| d.category == category),
            );
        }
        if targets.is_empty() {
            return Err("需要提供 keys 或 category".into());
        }

        let mut seen: Vec<&str> = Vec::new();
        let mut applied = Vec::new();
        for descriptor in targets {
            if self.host.fixed_setting(&descriptor.key).is_some() {
                continue;
            }
            if seen.contains(&descriptor.key.as_str()) {
                continue;
            }
            seen.push(&descriptor.key);
            let previous = self.settings.read(descriptor);
            // 已经是默认值的不计入 applied——「改了 0 项」和「改了 5 项」对用户是两回事
            if previous == descriptor.default_value {
                continue;
            }
            self.settings.reset(descriptor);
            applied.push(self.applied_entry(descriptor, previous, false));
        }

        self.settings
            .flush()
            .map_err(|e| format!("设置写盘失败: {e}"))?;
        let summary = if applied.is_empty() {
            "目标设置均已是默认值，无需修改".to_owned()
        } else {
            format!("已恢复 {} 项设置的默认值", applied.len())
        };
        Ok(json!({ "applied": applied, "summary": summary }))
    }

    fn open_ui(&self, args: &ToolArgs) -> ToolOutcome {
        let tab = args
            .str_opt("tab")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_UI_TAB);
        self.host.open_settings_ui(tab)?;
        Ok(json!({ "opened": true, "tab": tab }))
    }
}

impl ToolExecutor for SettingsToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    fn required_actions(
        &self,
        name: &str,
        _args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        match name {
            // 设置不是工作区文件，路径级权限无从谈起；只有写入受动作级门禁约束
            "settings_update" | "settings_reset" => vec![(AiToolAction::ModifySettings, None)],
            _ => Vec::new(),
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "settings_list" => self.list(args),
            "settings_get" => self.get(args),
            "settings_update" => self.update(args),
            "settings_reset" => self.reset(args),
            "settings_open_ui" => self.open_ui(args),
            other => Err(format!("未知的设置工具: {other}")),
        }
    }
}

/// 严格的字符串数组：混进非字符串就报错，而不是像 `ToolArgs::str_array` 那样静默丢掉。
/// 设置项是精确操作，「悄悄少改一项」比报错更糟。
fn require_string_array(args: &ToolArgs, field: &str) -> Result<Vec<String>, String> {
    match args.get(field) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{field} 需要字符串数组"))
            })
            .collect(),
        _ => Err(format!("{field} 需要字符串数组")),
    }
}

/// 便于测试断言的辅助：把 `applied` 数组按 key 索引。
#[cfg(test)]
fn index_applied(data: &Value) -> serde_json::Map<String, Value> {
    data["applied"]
        .as_array()
        .expect("applied 应是数组")
        .iter()
        .map(|item| (item["key"].as_str().unwrap().to_owned(), item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionService, AiToolAction};
    use crate::ai::tools::host::HeadlessHost;
    use crate::ai::tools::ToolRegistry;
    use crate::settings::SettingsService;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct UiHost {
        root: PathBuf,
        opened: Mutex<Vec<String>>,
    }

    impl ToolHost for UiHost {
        fn workspace_root(&self) -> &std::path::Path {
            &self.root
        }
        fn open_settings_ui(&self, tab: &str) -> Result<(), String> {
            self.opened.lock().unwrap().push(tab.to_owned());
            Ok(())
        }
    }

    struct Fixture {
        root: PathBuf,
        settings_file: PathBuf,
        settings: Arc<AppSettings>,
        host: Arc<UiHost>,
        registry: ToolRegistry,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = std::fs::remove_file(&self.settings_file);
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

        fn value_of(&self, key: &str) -> SettingValue {
            self.settings.read(app_settings::descriptor(key).unwrap())
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let stem = format!("mochi-settingstool-{}-{tag}-{n}", std::process::id());
        let root = std::env::temp_dir().join(&stem);
        let settings_file = std::env::temp_dir().join(format!("{stem}.json"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&settings_file);
        std::fs::create_dir_all(&root).unwrap();

        let settings = Arc::new(AppSettings::new(Arc::new(SettingsService::new(Some(
            settings_file.clone(),
        )))));
        let host = Arc::new(UiHost {
            root: root.clone(),
            opened: Mutex::new(Vec::new()),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry = ToolRegistry::new(perms).with(Arc::new(SettingsToolExecutor::new(
            settings.clone(),
            host.clone(),
        )));
        Fixture {
            root,
            settings_file,
            settings,
            host,
            registry,
        }
    }

    /// 「修改设置」动作默认就是允许的（只有「执行命令」默认关闭），
    /// 这里显式设置一遍，免得默认值将来变了测试却还在依赖它。
    fn allow_settings(f: &Fixture) {
        set_settings_permission(f, true);
    }

    fn set_settings_permission(f: &Fixture, allowed: bool) {
        let mut actions = f.registry.permissions().action_permissions();
        actions.set(AiToolAction::ModifySettings, allowed);
        f.registry.permissions().set_action_permissions(actions);
    }

    #[test]
    fn list_returns_everything_grouped_by_category() {
        let f = fixture("list");
        let data = f.ok("settings_list", json!({}));
        assert_eq!(data["total"], app_settings::descriptors().len());

        let cats = data["categories"].as_array().unwrap();
        assert_eq!(cats[0]["id"], "general", "分类顺序应是注册表里的首次出现序");
        assert!(cats
            .iter()
            .all(|c| c["label"].as_str().is_some_and(|l| !l.is_empty())));

        let total: usize = cats
            .iter()
            .map(|c| c["settings"].as_array().unwrap().len())
            .sum();
        assert_eq!(total, app_settings::descriptors().len(), "不该有设置项掉队");
    }

    #[test]
    fn list_filters_by_category_and_query() {
        let f = fixture("list-filter");
        let by_cat = f.ok("settings_list", json!({ "category": "appearance" }));
        let cats = by_cat["categories"].as_array().unwrap();
        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0]["id"], "appearance");

        let by_query = f.ok("settings_list", json!({ "query": "字号" }));
        assert!(by_query["total"].as_u64().unwrap() > 0);
        assert!(by_query["total"].as_u64().unwrap() < app_settings::descriptors().len() as u64);

        // 两个条件是与关系
        let both = f.ok(
            "settings_list",
            json!({ "category": "appearance", "query": "字号" }),
        );
        assert!(both["total"].as_u64().unwrap() <= by_query["total"].as_u64().unwrap());
    }

    #[test]
    fn unknown_category_lists_the_valid_ones() {
        let f = fixture("bad-cat");
        let err = f.err("settings_list", json!({ "category": "没这个分类" }));
        assert!(
            err.contains("general") && err.contains("appearance"),
            "{err}"
        );
    }

    #[test]
    fn get_returns_snapshots_with_defaults() {
        let f = fixture("get");
        let data = f.ok(
            "settings_get",
            json!({ "keys": ["typography.fontSize", "appearance.themeMode"] }),
        );
        let items = data["settings"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["key"], "typography.fontSize");
        assert_eq!(items[0]["isDefault"], true);
        assert_eq!(items[1]["value"], "light");
    }

    /// 找不到 key 时给出相近候选，模型能少走一轮。
    #[test]
    fn unknown_key_suggests_near_misses() {
        let f = fixture("get-miss");
        let err = f.err("settings_get", json!({ "keys": ["typography.fontsize "] }));
        assert!(err.contains("未知设置 key"), "{err}");
        assert!(err.contains("typography.fontSize"), "应给出相近 key: {err}");
    }

    #[test]
    fn get_rejects_a_non_string_array() {
        let f = fixture("get-shape");
        assert!(f
            .err("settings_get", json!({ "keys": "typography.fontSize" }))
            .contains("字符串数组"));
        assert!(f
            .err("settings_get", json!({ "keys": [1, 2] }))
            .contains("字符串数组"));
        assert!(f
            .err("settings_get", json!({ "keys": [] }))
            .contains("不能为空"));
    }

    #[test]
    fn update_applies_and_reports_old_to_new() {
        let f = fixture("update");
        allow_settings(&f);
        let data = f.ok(
            "settings_update",
            json!({ "changes": [
                { "key": "typography.fontSize", "value": 20 },
                { "key": "appearance.themeMode", "value": "dark" },
            ]}),
        );
        assert_eq!(data["summary"], "已修改 2 项设置");

        let applied = index_applied(&data);
        assert_eq!(applied["typography.fontSize"]["value"], 20);
        assert_eq!(applied["typography.fontSize"]["clamped"], false);
        assert!(applied["typography.fontSize"]["label"].as_str().is_some());
        assert_eq!(applied["appearance.themeMode"]["previousValue"], "light");
        assert_eq!(applied["appearance.themeMode"]["value"], "dark");

        assert_eq!(
            f.value_of("typography.fontSize"),
            SettingValue::Number(20.0)
        );
        assert_eq!(
            f.value_of("appearance.themeMode"),
            SettingValue::Text("dark".into())
        );
    }

    #[test]
    fn update_reports_clamping_instead_of_lying() {
        let f = fixture("update-clamp");
        allow_settings(&f);
        let max = app_settings::descriptor("typography.fontSize")
            .unwrap()
            .max
            .unwrap();
        let data = f.ok(
            "settings_update",
            json!({ "changes": [{ "key": "typography.fontSize", "value": max + 999.0 }] }),
        );
        let applied = index_applied(&data);
        assert_eq!(applied["typography.fontSize"]["clamped"], true);
        assert_eq!(applied["typography.fontSize"]["value"], max);
    }

    /// 一项非法就整批不改——不能留下半套生效的中间态。
    #[test]
    fn update_is_all_or_nothing() {
        let f = fixture("update-atomic");
        allow_settings(&f);
        let err = f.err(
            "settings_update",
            json!({ "changes": [
                { "key": "typography.fontSize", "value": 20 },
                { "key": "appearance.themeMode", "value": "紫色" },
            ]}),
        );
        assert!(err.contains("只接受"), "{err}");
        assert_eq!(
            f.value_of("typography.fontSize"),
            app_settings::descriptor("typography.fontSize")
                .unwrap()
                .default_value,
            "同批里的合法项也不该落地"
        );
    }

    #[test]
    fn update_rejects_malformed_change_lists() {
        let f = fixture("update-shape");
        allow_settings(&f);
        assert!(f.err("settings_update", json!({})).contains("至少一项"));
        assert!(f
            .err("settings_update", json!({ "changes": [] }))
            .contains("至少一项"));
        assert!(f
            .err("settings_update", json!({ "changes": ["fontSize=20"] }))
            .contains("{ key, value }"));
        assert!(f
            .err("settings_update", json!({ "changes": [{ "value": 20 }] }))
            .contains("缺少 key"));
    }

    /// 关掉「修改设置」动作后，工具在执行前就该被 registry 挡下。
    #[test]
    fn permission_gate_blocks_writes_but_not_reads() {
        let f = fixture("perm");
        set_settings_permission(&f, false);

        assert_eq!(
            f.run("settings_list", json!({}))["ok"],
            true,
            "读取不该被挡"
        );
        let err = f.err(
            "settings_update",
            json!({ "changes": [{ "key": "typography.fontSize", "value": 20 }] }),
        );
        assert!(err.contains("设置"), "{err}");
        assert_eq!(
            f.value_of("typography.fontSize"),
            app_settings::descriptor("typography.fontSize")
                .unwrap()
                .default_value
        );
        assert!(f
            .err("settings_reset", json!({ "category": "typography" }))
            .contains("设置"));
    }

    #[test]
    fn tools_cannot_disable_their_own_approval_requirements() {
        let f = fixture("approval-boundary");
        allow_settings(&f);
        assert!(f.err("settings_update",json!({"changes":[{"key":"typography.fontSize","value":20},{"key":"ai.editApplyMode","value":"auto"}]})).contains("用户"));
        assert_eq!(
            f.value_of("typography.fontSize"),
            app_settings::descriptor("typography.fontSize")
                .unwrap()
                .default_value
        );
        assert!(f
            .err(
                "settings_update",
                json!({"changes":[{"key":"ai.requirePermissionPrompt","value":false}]})
            )
            .contains("用户"));
    }

    #[test]
    fn reset_restores_defaults_by_key_and_by_category() {
        let f = fixture("reset");
        allow_settings(&f);
        f.ok(
            "settings_update",
            json!({ "changes": [
                { "key": "typography.fontSize", "value": 22 },
                { "key": "appearance.themeMode", "value": "dark" },
            ]}),
        );

        let by_key = f.ok("settings_reset", json!({ "keys": ["typography.fontSize"] }));
        assert_eq!(by_key["applied"].as_array().unwrap().len(), 1);
        assert_eq!(
            f.value_of("typography.fontSize"),
            app_settings::descriptor("typography.fontSize")
                .unwrap()
                .default_value
        );
        assert_eq!(
            f.value_of("appearance.themeMode"),
            SettingValue::Text("dark".into()),
            "没点名的不该被动"
        );

        let by_cat = f.ok("settings_reset", json!({ "category": "appearance" }));
        assert_eq!(by_cat["applied"].as_array().unwrap().len(), 1);
        assert_eq!(
            f.value_of("appearance.themeMode"),
            SettingValue::Text("light".into())
        );
    }

    /// 已是默认值的项不计入 applied——「改了 0 项」和「改了 5 项」对用户是两回事。
    #[test]
    fn reset_on_untouched_settings_reports_no_changes() {
        let f = fixture("reset-noop");
        allow_settings(&f);
        let data = f.ok("settings_reset", json!({ "category": "typography" }));
        assert!(data["applied"].as_array().unwrap().is_empty());
        assert!(
            data["summary"].as_str().unwrap().contains("无需修改"),
            "{data}"
        );
    }

    #[test]
    fn reset_deduplicates_overlapping_keys_and_category() {
        let f = fixture("reset-dedup");
        allow_settings(&f);
        f.ok(
            "settings_update",
            json!({ "changes": [{ "key": "appearance.themeMode", "value": "dark" }] }),
        );

        let data = f.ok(
            "settings_reset",
            json!({ "keys": ["appearance.themeMode"], "category": "appearance" }),
        );
        assert_eq!(
            data["applied"].as_array().unwrap().len(),
            1,
            "同一项不该出现两次"
        );
    }

    #[test]
    fn reset_without_targets_fails() {
        let f = fixture("reset-empty");
        allow_settings(&f);
        assert!(f
            .err("settings_reset", json!({}))
            .contains("keys 或 category"));
        assert!(f
            .err("settings_reset", json!({ "keys": [] }))
            .contains("keys 或 category"));
    }

    #[test]
    fn open_ui_defaults_to_general_and_reaches_the_host() {
        let f = fixture("open-ui");
        assert_eq!(f.ok("settings_open_ui", json!({}))["tab"], "general");
        assert_eq!(
            f.ok("settings_open_ui", json!({ "tab": "ai" }))["tab"],
            "ai"
        );
        assert_eq!(*f.host.opened.lock().unwrap(), ["general", "ai"]);
    }

    /// 无窗口宿主必须报告打开设置失败。
    #[test]
    fn open_ui_fails_honestly_without_a_window() {
        let root =
            std::env::temp_dir().join(format!("mochi-settings-headless-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let host = Arc::new(HeadlessHost::new(&root));
        assert!(host.open_settings_ui("general").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 改完必须落盘：进程重启后设置还在。
    #[test]
    fn updates_survive_a_restart() {
        let f = fixture("persist");
        allow_settings(&f);
        f.ok(
            "settings_update",
            json!({ "changes": [{ "key": "typography.fontSize", "value": 21 }] }),
        );

        let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
            f.settings_file.clone(),
        ))));
        assert_eq!(
            reopened.read(app_settings::descriptor("typography.fontSize").unwrap()),
            SettingValue::Number(21.0)
        );
    }
}
