//! 原生侧的设置名只是现有 Electron 设置桶之上的视图。
//! 共享偏好只有一份持久化值，绝不维护第二份镜像。
use super::file::Values;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

pub(super) const VERSION_KEY: &str = "__mochi_shared_settings_version";
pub(super) const SOURCE_KEY: &str = "__mochi_shared_settings_source";
const AI: &str = "mochi-ai";
const PROVIDERS: &[&str] = &["state", "providers"];
const ACTIONS: [&str; 6] = [
    "read_file",
    "write_file",
    "delete_file",
    "execute_command",
    "network_access",
    "modify_settings",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mapping {
    native_key: String,
    store: String,
    path: Vec<String>,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Shortcut {
    action: String,
    default_key: String,
}

static MAPPINGS: LazyLock<Vec<Mapping>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../shared/settings-map.json"))
        .expect("invalid shared settings map")
});

static SHORTCUTS: LazyLock<Vec<Shortcut>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../shared/shortcut-actions.json"))
        .expect("invalid shared shortcuts map")
});

#[derive(Clone)]
pub(super) struct Change {
    pub key: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

fn mapping(key: &str) -> Option<&'static Mapping> {
    MAPPINGS.iter().find(|entry| entry.native_key == key)
}

fn provider_id(key: &str) -> Option<&str> {
    key.strip_prefix("ai.providers.")?.strip_suffix(".apiKey")
}

pub(super) fn is_alias(key: &str) -> bool {
    mapping(key).is_some()
        || provider_id(key).is_some()
        || key.starts_with("ai.provider.")
        || key.starts_with("pdf.last-page:")
        || matches!(
            key,
            "ai.providers" | "ai.actionPermissions" | "search.history" | "keyboard.shortcuts"
        )
}

fn allow_legacy(values: &Values) -> bool {
    !values
        .get(SOURCE_KEY)
        .is_some_and(|source| source.starts_with("electron"))
}

fn bucket(values: &Values, name: &str) -> Result<Option<Value>> {
    values
        .get(name)
        .map(|raw| {
            serde_json::from_str(raw).with_context(|| format!("设置桶 {name} 无法解析，原值已保留"))
        })
        .transpose()
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |item, segment| item.get(*segment))
}

fn bucket_field(values: &Values, name: &str, path: &[&str]) -> Result<Option<Value>> {
    Ok(bucket(values, name)?
        .as_ref()
        .and_then(|value| field(value, path))
        .cloned())
}

fn metadata(providers: &Value) -> Value {
    Value::Array(
        providers
            .as_array()
            .into_iter()
            .flatten()
            .map(|provider| {
                let mut metadata = provider.as_object().cloned().unwrap_or_default();
                metadata.retain(|key, _| {
                    ["id", "name", "baseUrl", "model", "stream", "protocol"].contains(&key.as_str())
                });
                Value::Object(metadata)
            })
            .collect(),
    )
}

fn as_storage(value: &Value, kind: &str) -> Option<String> {
    if value.is_null() {
        return None;
    }
    match kind {
        "string" => value.as_str().map(str::to_owned),
        "boolean" => value.as_bool().map(|value| value.to_string()),
        "number" => value.as_number().map(ToString::to_string),
        "json" => Some(value.to_string()),
        _ => None,
    }
}

fn from_storage(raw: &str, kind: &str) -> Result<Value> {
    match kind {
        "string" => Ok(Value::String(raw.into())),
        "boolean" => match raw {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => bail!("共享设置需要布尔值"),
        },
        "number" => {
            let value: Value = serde_json::from_str(raw).context("共享设置需要数字")?;
            if !value.is_number() {
                bail!("共享设置需要数字");
            }
            Ok(value)
        }
        "json" => serde_json::from_str(raw).context("共享设置的 JSON 值无效"),
        _ => bail!("未知共享设置类型"),
    }
}

pub(super) fn get(values: &Values, key: &str) -> Result<Option<String>> {
    let projected = if let Some(entry) = mapping(key) {
        if entry.path.is_empty() {
            values.get(&entry.store).cloned()
        } else {
            let path: Vec<_> = entry.path.iter().map(String::as_str).collect();
            bucket_field(values, &entry.store, &path)?
                .as_ref()
                .and_then(|value| as_storage(value, &entry.kind))
        }
    } else if key == "ai.providers" {
        bucket_field(values, AI, PROVIDERS)?.map(|providers| metadata(&providers).to_string())
    } else if let Some(id) = provider_id(key) {
        let providers = bucket_field(values, AI, PROVIDERS)?;
        let secret = providers
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|providers| {
                providers
                    .iter()
                    .find(|p| p.get("id").and_then(Value::as_str) == Some(id))
            })
            .and_then(|provider| provider.get("apiKey"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        // 已删除的提供商绝不能从旧配置里把 key 捞回来。
        if providers.is_some() {
            return Ok(secret);
        }
        secret
    } else if key == "ai.actionPermissions" {
        bucket_field(values, AI, &["state", "permissions"])?.map(|permissions| {
            ACTIONS
                .iter()
                .map(|action| {
                    let allowed = permissions
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|permission| {
                            permission.get("action").and_then(Value::as_str) == Some(*action)
                        })
                        .and_then(|permission| permission.get("allowed"))
                        .and_then(Value::as_bool)
                        .unwrap_or(*action != "execute_command");
                    format!("{action}={}", u8::from(allowed))
                })
                .collect::<Vec<_>>()
                .join(",")
        })
    } else if key == "search.history" {
        bucket_field(values, "mochi-search", &["state", "searchHistory"])?.map(|history| {
            Value::Array(
                history
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|item| {
                        item.get("query")
                            .and_then(Value::as_str)
                            .map(|query| json!(query))
                    })
                    .collect(),
            )
            .to_string()
        })
    } else if key == "keyboard.shortcuts" {
        let mut native: Map<String, Value> = values
            .get(key)
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        // 旧键下面只剩原生专属的绑定。
        native.retain(|key, _| !SHORTCUTS.iter().any(|entry| entry.default_key == *key));
        if let Some(custom) = bucket(values, "mochi:custom-shortcuts")? {
            for shortcut in SHORTCUTS.iter() {
                if let Some(value) = custom.get(&shortcut.action) {
                    native.insert(shortcut.default_key.clone(), value.clone());
                }
            }
            Some(Value::Object(native).to_string())
        } else {
            None
        }
    } else if let Some(path) = key.strip_prefix("pdf.last-page:") {
        bucket_field(values, "mochi:pdf-last-pages", &[path])?
            .as_ref()
            .and_then(|value| as_storage(value, "number"))
    } else {
        return Ok(values.get(key).cloned());
    };
    Ok(projected.or_else(|| {
        allow_legacy(values)
            .then(|| values.get(key).cloned())
            .flatten()
    }))
}

fn fresh_bucket(name: &str) -> Value {
    let version = match name {
        "mochi-ai" => 3,
        "workspace-storage" => 1,
        _ => 0,
    };
    json!({"state": {}, "version": version})
}

fn field_mut<'a>(value: &'a mut Value, path: &[&str]) -> Result<&'a mut Value> {
    let mut cursor = value;
    for segment in path {
        if cursor.is_null() {
            *cursor = Value::Object(Map::new());
        }
        let object = cursor
            .as_object_mut()
            .context("共享设置的对象结构无效，原值已保留")?;
        cursor = object.entry((*segment).to_owned()).or_insert(Value::Null);
    }
    Ok(cursor)
}

fn write_field(
    values: &mut Values,
    name: &str,
    path: &[&str],
    before: Option<&Value>,
    after: Option<&Value>,
) -> Result<()> {
    let mut root = bucket(values, name)?.unwrap_or_else(|| {
        if path.first() == Some(&"state") {
            fresh_bucket(name)
        } else {
            json!({})
        }
    });
    if let Some((leaf, parent)) = path.split_last() {
        let object = field_mut(&mut root, parent)?;
        if object.is_null() {
            *object = json!({});
        }
        let object = object.as_object_mut().context("共享设置字段的父对象无效")?;
        let current = object.get(*leaf);
        match merge_value(current, before, after) {
            Some(value) => {
                object.insert((*leaf).into(), value);
            }
            None => {
                object.remove(*leaf);
            }
        }
    } else if let Some(value) = merge_value(Some(&root), before, after) {
        root = value;
    } else {
        values.remove(name);
        return Ok(());
    }
    values.insert(name.into(), root.to_string());
    Ok(())
}

/// 把写入方的实际修改应用到最新值上。对象叶子节点和提供商 ID 各自独立，
/// 过期的窗口因此盖不掉别人已做的修改。
pub(super) fn merge_value(
    current: Option<&Value>,
    before: Option<&Value>,
    after: Option<&Value>,
) -> Option<Value> {
    if before == after {
        return current.cloned();
    }
    let after = after?;
    if let Some(after) = after.as_object() {
        let before = before.and_then(Value::as_object);
        let mut merged = current
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let keys: BTreeSet<_> = before
            .into_iter()
            .flat_map(|old| old.keys())
            .chain(after.keys())
            .collect();
        for key in keys {
            match merge_value(
                merged.get(key),
                before.and_then(|old| old.get(key)),
                after.get(key),
            ) {
                Some(value) => {
                    merged.insert(key.clone(), value);
                }
                None => {
                    merged.remove(key);
                }
            }
        }
        return Some(Value::Object(merged));
    }
    if let Some(after_items) = after.as_array() {
        let before_items = before.and_then(Value::as_array);
        if unique_ids(after_items) && before_items.is_none_or(|items| unique_ids(items)) {
            let current_items = current
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let old: BTreeMap<_, _> = before_items
                .into_iter()
                .flatten()
                .filter_map(|item| Some((item.get("id")?.as_str()?, item)))
                .collect();
            let next: BTreeMap<_, _> = after_items
                .iter()
                .filter_map(|item| Some((item.get("id")?.as_str()?, item)))
                .collect();
            let mut output = Vec::new();
            let mut included = BTreeSet::new();
            for item in current_items {
                let Some(id) = item.get("id").and_then(Value::as_str).map(str::to_owned) else {
                    output.push(item);
                    continue;
                };
                if old.contains_key(id.as_str()) && !next.contains_key(id.as_str()) {
                    continue;
                }
                let merged = match next.get(id.as_str()) {
                    Some(new) => merge_value(Some(&item), old.get(id.as_str()).copied(), Some(new))
                        .unwrap_or(item),
                    None => item,
                };
                included.insert(id);
                output.push(merged);
            }
            for item in after_items {
                let id = item.get("id").and_then(Value::as_str).unwrap();
                if !included.contains(id) {
                    // 字段级修改不能把别人删掉的条目救活。
                    if !old.contains_key(id) {
                        output.push(item.clone());
                    }
                }
            }
            return Some(Value::Array(output));
        }
    }
    Some(after.clone())
}

fn unique_ids(items: &[Value]) -> bool {
    let mut ids = BTreeSet::new();
    items.iter().all(|item| {
        item.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && ids.insert(id))
    })
}

fn parse_optional(raw: Option<&str>, kind: &str) -> Result<Option<Value>> {
    raw.map(|value| from_storage(value, kind)).transpose()
}

pub(super) fn apply(values: &mut Values, change: &Change) -> Result<()> {
    let key = change.key.as_str();
    if let Some(entry) = mapping(key) {
        if entry.path.is_empty() && entry.kind == "string" {
            match &change.after {
                Some(value) => {
                    values.insert(entry.store.clone(), value.clone());
                }
                None => {
                    values.remove(&entry.store);
                }
            }
        } else {
            let before = parse_optional(change.before.as_deref(), &entry.kind)?;
            let after = parse_optional(change.after.as_deref(), &entry.kind)?;
            let path: Vec<_> = entry.path.iter().map(String::as_str).collect();
            write_field(values, &entry.store, &path, before.as_ref(), after.as_ref())?;
        }
        values.remove(key);
    } else if key == "ai.providers" {
        let before = parse_optional(change.before.as_deref(), "json")?;
        let after = parse_optional(change.after.as_deref(), "json")?;
        if after.as_ref().is_some_and(|value| !value.is_array()) {
            bail!("提供商设置必须是数组");
        }
        write_field(values, AI, PROVIDERS, before.as_ref(), after.as_ref())?;
        values.remove(key);
    } else if let Some(id) = provider_id(key) {
        let mut providers = bucket_field(values, AI, PROVIDERS)?.unwrap_or_else(|| json!([]));
        let providers_array = providers.as_array_mut().context("提供商设置必须是数组")?;
        let index = providers_array
            .iter()
            .position(|provider| provider.get("id").and_then(Value::as_str) == Some(id));
        if index.is_none() && (change.before.is_some() || change.after.is_none()) {
            values.remove(key);
            return Ok(());
        }
        if let Some(value) = &change.after {
            let index = index.unwrap_or_else(|| {
                providers_array.push(json!({"id": id}));
                providers_array.len() - 1
            });
            providers_array[index]
                .as_object_mut()
                .context("提供商设置对象无效")?
                .insert("apiKey".into(), json!(value));
        } else if let Some(index) = index {
            providers_array[index]
                .as_object_mut()
                .context("提供商设置对象无效")?
                .remove("apiKey");
        }
        // providers 值刚从事务里读出来，无需再合并。
        let current = bucket_field(values, AI, PROVIDERS)?;
        write_field(values, AI, PROVIDERS, current.as_ref(), Some(&providers))?;
        values.remove(key);
    } else if key == "ai.actionPermissions" {
        apply_permissions(values, change)?;
    } else if key == "search.history" {
        apply_search_history(values, change)?;
    } else if key == "keyboard.shortcuts" {
        apply_shortcuts(values, change)?;
    } else if let Some(path) = key.strip_prefix("pdf.last-page:") {
        let before = parse_optional(change.before.as_deref(), "number")?;
        let after = parse_optional(change.after.as_deref(), "number")?;
        write_field(
            values,
            "mochi:pdf-last-pages",
            &[path],
            before.as_ref(),
            after.as_ref(),
        )?;
        values.remove(key);
    } else {
        match &change.after {
            Some(value) => {
                // 原生自有的 JSON 值同样按独立叶子合并。
                let before = change
                    .before
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok());
                let after = serde_json::from_str::<Value>(value).ok();
                let current = values
                    .get(key)
                    .and_then(|raw| serde_json::from_str(raw).ok());
                let merged = match after {
                    Some(after) if after.is_object() || after.is_array() => {
                        merge_value(current.as_ref(), before.as_ref(), Some(&after))
                            .map(|value| value.to_string())
                    }
                    _ => None,
                };
                values.insert(key.into(), merged.unwrap_or_else(|| value.clone()));
            }
            None => {
                values.remove(key);
            }
        }
    }
    Ok(())
}

fn action_values(raw: Option<&str>) -> BTreeMap<&str, bool> {
    raw.into_iter()
        .flat_map(|raw| raw.split(','))
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            let value = match value.trim() {
                "1" | "true" => true,
                "0" | "false" => false,
                _ => return None,
            };
            Some((key.trim(), value))
        })
        .collect()
}

fn apply_permissions(values: &mut Values, change: &Change) -> Result<()> {
    let before = action_values(change.before.as_deref());
    let after = action_values(change.after.as_deref());
    let original = bucket_field(values, AI, &["state", "permissions"])?;
    let mut permissions = original.clone().unwrap_or_else(|| json!([]));
    let items = permissions
        .as_array_mut()
        .context("AI 权限设置必须是数组")?;
    for action in ACTIONS {
        let old = before.get(action).copied();
        let next = after.get(action).copied();
        if old == next {
            continue;
        }
        let allowed = next.unwrap_or(action != "execute_command");
        if let Some(item) = items
            .iter_mut()
            .find(|item| item.get("action").and_then(Value::as_str) == Some(action))
        {
            item.as_object_mut()
                .context("AI 权限设置对象无效")?
                .insert("allowed".into(), json!(allowed));
        } else {
            items.push(json!({"action": action, "allowed": allowed, "requiresConfirmation": action == "execute_command"}));
        }
    }
    write_field(
        values,
        AI,
        &["state", "permissions"],
        original.as_ref(),
        Some(&permissions),
    )?;
    values.remove(&change.key);
    Ok(())
}

fn apply_search_history(values: &mut Values, change: &Change) -> Result<()> {
    let before: Vec<String> = change
        .before
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let after: Vec<String> = change
        .after
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let original = bucket_field(values, "mochi-search", &["state", "searchHistory"])?;
    let current = original.as_ref().and_then(Value::as_array);
    let mut items: Vec<Value> = after
        .iter()
        .map(|query| {
            current
                .into_iter()
                .flatten()
                .find(|item| item.get("query").and_then(Value::as_str) == Some(query))
                .cloned()
                .unwrap_or_else(|| json!({"query": query, "timestamp": super::file::now_millis()}))
        })
        .collect();
    for item in current.into_iter().flatten() {
        let Some(query) = item.get("query").and_then(Value::as_str) else {
            continue;
        };
        if !before.iter().any(|old| old == query) && !after.iter().any(|next| next == query) {
            items.push(item.clone());
        }
    }
    write_field(
        values,
        "mochi-search",
        &["state", "searchHistory"],
        original.as_ref(),
        Some(&Value::Array(items)),
    )?;
    values.remove(&change.key);
    Ok(())
}

fn apply_shortcuts(values: &mut Values, change: &Change) -> Result<()> {
    let before: BTreeMap<String, String> = change
        .before
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    let after: BTreeMap<String, String> = change
        .after
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    for shortcut in SHORTCUTS.iter() {
        let before = before.get(&shortcut.default_key).map(|value| json!(value));
        let after = after.get(&shortcut.default_key).map(|value| json!(value));
        if before != after {
            write_field(
                values,
                "mochi:custom-shortcuts",
                &[&shortcut.action],
                before.as_ref(),
                after.as_ref(),
            )?;
        }
    }
    let exclusive: BTreeMap<_, _> = after
        .into_iter()
        .filter(|(key, _)| {
            !SHORTCUTS
                .iter()
                .any(|shortcut| shortcut.default_key == *key)
        })
        .collect();
    if exclusive.is_empty() {
        values.remove(&change.key);
    } else {
        values.insert(change.key.clone(), serde_json::to_string(&exclusive)?);
    }
    Ok(())
}

pub(super) fn normalize_providers(values: &mut Values) -> Result<()> {
    let existing = bucket_field(values, AI, PROVIDERS)?;
    let mut providers = existing.clone().or_else(|| {
        values
            .get("ai.providers")
            .and_then(|raw| serde_json::from_str(raw).ok())
    });
    if providers.is_none() {
        if let (Some(base), Some(model)) = (
            values.get("ai.provider.baseUrl"),
            values.get("ai.provider.model"),
        ) {
            if !base.trim().is_empty() && !model.trim().is_empty() {
                providers = Some(json!([{
                    "id": "default", "name": values.get("ai.provider.name").map(String::as_str).unwrap_or("默认"),
                    "baseUrl": base, "model": model, "stream": true,
                    "apiKey": values.get("ai.provider.apiKey").map(String::as_str).unwrap_or("")
                }]));
            }
        }
    }
    let Some(mut providers) = providers else {
        return normalize_orphaned_secrets(values);
    };
    let mut migrated_ids = BTreeSet::new();
    for provider in providers.as_array_mut().context("提供商设置必须是数组")? {
        let Some(object) = provider.as_object_mut() else {
            bail!("提供商设置对象无效")
        };
        if let Some(id) = object.get("id").and_then(Value::as_str) {
            migrated_ids.insert(id.to_owned());
        }
        if existing.is_none() {
            if let Some(id) = object.get("id").and_then(Value::as_str) {
                if let Some(secret) = values.get(&format!("ai.providers.{id}.apiKey")) {
                    object.insert("apiKey".into(), json!(secret));
                }
            }
        }
        if let Some(secret) = object.get("apiKey").and_then(Value::as_str) {
            if !secret.is_empty() && !super::is_encrypted(secret) {
                object.insert("apiKey".into(), json!(super::protect(secret)?));
            }
        }
    }
    write_field(values, AI, PROVIDERS, existing.as_ref(), Some(&providers))?;
    values.retain(|key, _| {
        key != "ai.providers"
            && !provider_id(key).is_some_and(|id| migrated_ids.contains(id))
            && ![
                "ai.provider.name",
                "ai.provider.baseUrl",
                "ai.provider.model",
                "ai.provider.apiKey",
            ]
            .contains(&key.as_str())
    });
    normalize_orphaned_secrets(values)
}

fn normalize_orphaned_secrets(values: &mut Values) -> Result<()> {
    for (key, value) in values.iter_mut() {
        if (provider_id(key).is_some() || key == "ai.provider.apiKey")
            && !value.is_empty()
            && !super::is_encrypted(value)
        {
            *value = super::protect(value)?;
        }
    }
    Ok(())
}

pub(super) fn keys(values: &Values) -> Vec<String> {
    let mut keys: BTreeSet<_> = values.keys().cloned().collect();
    for entry in MAPPINGS.iter() {
        if get(values, &entry.native_key).ok().flatten().is_some() {
            keys.insert(entry.native_key.clone());
        }
    }
    for key in [
        "ai.providers",
        "ai.actionPermissions",
        "search.history",
        "keyboard.shortcuts",
    ] {
        if get(values, key).ok().flatten().is_some() {
            keys.insert(key.into());
        }
    }
    if let Ok(Some(Value::Array(providers))) = bucket_field(values, AI, PROVIDERS) {
        for provider in providers {
            if let Some(id) = provider.get("id").and_then(Value::as_str) {
                keys.insert(format!("ai.providers.{id}.apiKey"));
            }
        }
    }
    keys.into_iter().collect()
}

pub(super) fn migrate_native(native: &Values) -> Result<Values> {
    let mut migrated = native.clone();
    // 删别名之前，先把提供商元数据和它的密钥一起物化出来。
    normalize_providers(&mut migrated)?;
    for (key, value) in native {
        if is_alias(key)
            && key != "ai.providers"
            && provider_id(key).is_none()
            && !key.starts_with("ai.provider.")
        {
            let value = if key == "background.imagePath" && std::path::Path::new(value).is_file() {
                super::background_image_data_url(std::path::Path::new(value))?
            } else {
                value.clone()
            };
            apply(
                &mut migrated,
                &Change {
                    key: key.clone(),
                    before: None,
                    after: Some(value),
                },
            )?;
        }
    }
    if bucket_field(&migrated, AI, &["state", "currentProviderId"])?.is_none() {
        if let Some(id) = bucket_field(&migrated, AI, PROVIDERS)?
            .and_then(|providers| providers.get(0)?.get("id").cloned())
        {
            write_field(
                &mut migrated,
                AI,
                &["state", "currentProviderId"],
                None,
                Some(&id),
            )?;
        }
    }
    Ok(migrated)
}
