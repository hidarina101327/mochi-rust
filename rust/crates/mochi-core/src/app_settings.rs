//! 原生设置描述表；保留与原 TS 注册表同名设置键的持久化兼容。
//! 数字钳制不吸附 step；枚举忽略大小写读取、按规范拼写写回。解析失败取默认值。

use std::sync::{Arc, LazyLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings::SettingsService;

const RAW: &str = include_str!("../assets/setting-descriptors.json");

/// 设置值的三种形态。序列化成裸 JSON 标量，与 TS 的 `SettingValue` 同形。
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl SettingValue {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Bool(b) => json!(b),
            // 整数值不要打成 1000.0——设置面板和模型看到的都该是 1000
            Self::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => json!(*n as i64),
            Self::Number(n) => json!(n),
            Self::Text(s) => json!(s),
        }
    }

    /// 落盘用的字符串形式。
    pub fn to_storage(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => (*n as i64).to_string(),
            Self::Number(n) => n.to_string(),
            Self::Text(s) => s.clone(),
        }
    }
}

impl Serialize for SettingValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(s)
    }
}

impl<'de> Deserialize<'de> for SettingValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        match Value::deserialize(d)? {
            Value::Bool(b) => Ok(Self::Bool(b)),
            Value::Number(n) => n
                .as_f64()
                .map(Self::Number)
                .ok_or_else(|| D::Error::custom("设置默认值不是有限数字")),
            Value::String(s) => Ok(Self::Text(s)),
            other => Err(D::Error::custom(format!("设置值类型不支持: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingValueType {
    Boolean,
    Number,
    Enum,
    String,
    Color,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingCategory {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingDescriptor {
    pub key: String,
    pub category: String,
    pub label: String,
    pub value_type: SettingValueType,
    pub default_value: SettingValue,
    pub ui_tab: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub ui_section: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Registry {
    categories: Vec<SettingCategory>,
    descriptors: Vec<SettingDescriptor>,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    serde_json::from_str(RAW).expect("设置注册表 JSON 损坏——请检查 assets/setting-descriptors.json")
});

pub fn categories() -> &'static [SettingCategory] {
    &REGISTRY.categories
}

pub fn descriptors() -> &'static [SettingDescriptor] {
    &REGISTRY.descriptors
}

pub fn descriptor(key: &str) -> Option<&'static SettingDescriptor> {
    REGISTRY.descriptors.iter().find(|d| d.key == key)
}

pub fn category_label(id: &str) -> Option<&'static str> {
    REGISTRY
        .categories
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.label.as_str())
}

pub fn is_category(id: &str) -> bool {
    category_label(id).is_some()
}

pub fn category_ids() -> Vec<&'static str> {
    REGISTRY.categories.iter().map(|c| c.id.as_str()).collect()
}

/// 关键词检索。匹配 key、中文标签、说明、分类名与枚举取值，大小写不敏感。
/// 空查询返回全部（对齐 TS 的 `searchSettings`）。
pub fn search(query: &str) -> Vec<&'static SettingDescriptor> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return REGISTRY.descriptors.iter().collect();
    }
    REGISTRY
        .descriptors
        .iter()
        .filter(|d| {
            let mut hay = format!(
                "{} {} {} {}",
                d.key,
                d.label,
                d.description.as_deref().unwrap_or(""),
                category_label(&d.category).unwrap_or("")
            );
            if let Some(options) = &d.options {
                hay.push(' ');
                hay.push_str(&options.join(" "));
            }
            hay.to_lowercase().contains(&needle)
        })
        .collect()
}

#[derive(Debug)]
pub struct Coerced {
    pub value: SettingValue,
    /// 数字被钳到范围内时为 true——要如实回给模型，否则它以为改成了自己给的值。
    pub clamped: bool,
}

/// 把模型给的任意 JSON 值收敛成该设置项的合法值。对齐 TS 的 `coerceSettingValue`。
pub fn coerce(descriptor: &SettingDescriptor, raw: &Value) -> Result<Coerced, String> {
    let key = &descriptor.key;
    match descriptor.value_type {
        SettingValueType::Boolean => {
            let b = match raw {
                Value::Bool(b) => Some(*b),
                Value::String(s) if s == "true" || s == "1" => Some(true),
                Value::String(s) if s == "false" || s == "0" => Some(false),
                Value::Number(n) if n.as_f64() == Some(1.0) => Some(true),
                Value::Number(n) if n.as_f64() == Some(0.0) => Some(false),
                _ => None,
            }
            .ok_or_else(|| format!("{key} 需要布尔值，收到 {raw}"))?;
            Ok(Coerced {
                value: SettingValue::Bool(b),
                clamped: false,
            })
        }

        SettingValueType::Number => {
            let parsed = match raw {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.trim().parse::<f64>().ok(),
                Value::Bool(_) | Value::Null | Value::Array(_) | Value::Object(_) => None,
            }
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{key} 需要数字，收到 {raw}"))?;

            let bounded = parsed
                .min(descriptor.max.unwrap_or(f64::INFINITY))
                .max(descriptor.min.unwrap_or(f64::NEG_INFINITY));
            Ok(Coerced {
                value: SettingValue::Number(bounded),
                clamped: bounded != parsed,
            })
        }

        SettingValueType::Enum => {
            let text = raw
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| scalar_to_string(raw));
            let text = text.trim();
            let empty: Vec<String> = Vec::new();
            let options = descriptor.options.as_ref().unwrap_or(&empty);
            let matched = options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(text))
                .ok_or_else(|| format!("{key} 只接受 {}，收到 {raw}", options.join(" / ")))?;
            Ok(Coerced {
                value: SettingValue::Text(matched.clone()),
                clamped: false,
            })
        }

        SettingValueType::String | SettingValueType::Color => {
            if raw.is_null() {
                return Err(format!("{key} 需要字符串值"));
            }
            if key == "appearance.accentColor" {
                return normalize_accent(raw.as_str().unwrap_or_default())
                    .map(|value| Coerced {
                        value: SettingValue::Text(value),
                        clamped: false,
                    })
                    .ok_or_else(|| "全局主题色需要 #RRGGBB 或 #RGB 色值".into());
            }
            if descriptor.value_type == SettingValueType::Color {
                let value = raw
                    .as_str()
                    .and_then(|text| normalize_color(descriptor, text))
                    .ok_or_else(|| {
                        format!(
                            "{}需要 #RGB、#RRGGBB、rgb(r,g,b) 或 auto（跟随主题）",
                            descriptor.label
                        )
                    })?;
                return Ok(Coerced {
                    value: SettingValue::Text(value),
                    clamped: false,
                });
            }
            Ok(Coerced {
                value: SettingValue::Text(scalar_to_string(raw)),
                clamped: false,
            })
        }
    }
}

/// 对齐 JS 的 `String(raw)`：字符串取原文，其余走 JSON 表示。
fn scalar_to_string(raw: &Value) -> String {
    match raw {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 注册表的读写门面：按 key 存进应用设置文件，缺失或损坏时退回默认值。
pub struct AppSettings {
    store: Arc<SettingsService>,
}

impl AppSettings {
    pub fn new(store: Arc<SettingsService>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<SettingsService> {
        &self.store
    }

    /// 当前值。未设置过、或磁盘上的值与声明类型对不上，都退回默认值。
    pub fn read(&self, descriptor: &SettingDescriptor) -> SettingValue {
        let Some(raw) = self.store.get(&storage_key(&descriptor.key)) else {
            return descriptor.default_value.clone();
        };
        parse_stored(descriptor, &raw).unwrap_or_else(|| descriptor.default_value.clone())
    }

    pub fn write(&self, descriptor: &SettingDescriptor, value: &SettingValue) {
        self.store
            .set(&storage_key(&descriptor.key), &value.to_storage());
    }

    /// 恢复默认：**删键而不是写入默认值**——这样上游改了默认值，用户没动过的设置会跟着变。
    pub fn reset(&self, descriptor: &SettingDescriptor) {
        self.store.remove(&storage_key(&descriptor.key));
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        self.store.flush()
    }

    /// 给模型看的快照，字段与 TS 的 `SettingSnapshot` 同名同形。
    pub fn snapshot(&self, descriptor: &SettingDescriptor) -> Value {
        let value = self.read(descriptor);
        let mut out = json!({
            "key": descriptor.key,
            "category": descriptor.category,
            "categoryLabel": category_label(&descriptor.category).unwrap_or_default(),
            "label": descriptor.label,
            "type": descriptor.value_type,
            "value": value,
            "defaultValue": descriptor.default_value,
            "isDefault": value == descriptor.default_value,
            "uiTab": descriptor.ui_tab,
        });
        let obj = out.as_object_mut().expect("刚构造的就是对象");
        // 可选字段缺失时**整个键都不出现**，与 TS 的 undefined 语义一致，
        // 免得模型看到一堆 null 以为"这项没有范围限制"。
        if let Some(v) = &descriptor.description {
            obj.insert("description".into(), json!(v));
        }
        if let Some(v) = descriptor.min {
            obj.insert("min".into(), json!(v));
        }
        if let Some(v) = descriptor.max {
            obj.insert("max".into(), json!(v));
        }
        if let Some(v) = descriptor.step {
            obj.insert("step".into(), json!(v));
        }
        if let Some(v) = &descriptor.unit {
            obj.insert("unit".into(), json!(v));
        }
        if let Some(v) = &descriptor.options {
            obj.insert("options".into(), json!(v));
        }
        out
    }
}

/// 为设置键添加前缀，避免与已有的 AI 服务方配置键重名。
fn storage_key(key: &str) -> String {
    format!("app.{key}")
}

fn parse_stored(descriptor: &SettingDescriptor, raw: &str) -> Option<SettingValue> {
    if descriptor.key == "appearance.accentColor" {
        return normalize_accent(raw).map(SettingValue::Text);
    }
    match descriptor.value_type {
        SettingValueType::Boolean => match raw {
            "true" => Some(SettingValue::Bool(true)),
            "false" => Some(SettingValue::Bool(false)),
            _ => None,
        },
        SettingValueType::Number => raw
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| {
                n.max(descriptor.min.unwrap_or(f64::NEG_INFINITY))
                    .min(descriptor.max.unwrap_or(f64::INFINITY))
            })
            .map(SettingValue::Number),
        SettingValueType::Enum => descriptor
            .options
            .as_ref()?
            .iter()
            .find(|o| o.as_str() == raw)
            .map(|o| SettingValue::Text(o.clone())),
        SettingValueType::String => Some(SettingValue::Text(raw.into())),
        SettingValueType::Color => normalize_color(descriptor, raw).map(SettingValue::Text),
    }
}

fn normalize_color(descriptor: &SettingDescriptor, raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.eq_ignore_ascii_case("auto") || text.is_empty() {
        return Some("auto".into());
    }
    // 兼容既有的主题变量默认值；任意 CSS 表达式并不受原生绘制器支持。
    if matches!(&descriptor.default_value, SettingValue::Text(default) if default == text && text.starts_with("var(--"))
    {
        return Some(text.into());
    }
    if let Some(color) = normalize_accent(text) {
        return Some(color);
    }
    // 旧版允许带 alpha 的 CSS 色值，但原生绘制只使用 RGB；读取时保留原来的颜色。
    if let Some(hex) = text.strip_prefix('#') {
        if hex.len() == 8 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("#{}", hex[..6].to_ascii_lowercase()));
        }
    }
    let (body, has_alpha) = if let Some(body) = text.strip_prefix("rgba(") {
        (body, true)
    } else {
        (text.strip_prefix("rgb(")?, false)
    };
    let parts = body.strip_suffix(')')?.split(',').collect::<Vec<_>>();
    if parts.len() != if has_alpha { 4 } else { 3 } {
        return None;
    }
    if has_alpha {
        let alpha = parts[3].trim().parse::<f32>().ok()?;
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return None;
        }
    }
    let channels = parts[..3]
        .iter()
        .map(|value| value.trim().parse::<u8>().ok())
        .collect::<Option<Vec<_>>>()?;
    Some(format!(
        "#{:02x}{:02x}{:02x}",
        channels[0], channels[1], channels[2]
    ))
}

fn normalize_accent(raw: &str) -> Option<String> {
    let hex = raw.trim().strip_prefix('#')?;
    if !matches!(hex.len(), 3 | 6) || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_owned()
    };
    Some(format!("#{}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn native_sidebar_sort_modes_match_the_drag_menu_and_roundtrip() {
        let settings = app_settings();
        let descriptor = d("sidebar.sortOrder");
        assert_eq!(
            descriptor.default_value,
            SettingValue::Text("manual".into())
        );
        for mode in ["manual", "name", "name-desc"] {
            let value = coerce(descriptor, &json!(mode)).unwrap().value;
            settings.write(descriptor, &value);
            assert_eq!(settings.read(descriptor), value);
        }
    }

    fn app_settings() -> AppSettings {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("mochi-appsettings-{}-{n}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        AppSettings::new(Arc::new(SettingsService::new(Some(path))))
    }

    fn d(key: &str) -> &'static SettingDescriptor {
        descriptor(key).unwrap_or_else(|| panic!("注册表里没有 {key}"))
    }

    #[test]
    fn registry_loads_and_is_self_consistent() {
        // 设置注册表会随着产品功能增长；这里验证下限与每项结构，不能因为
        // 合法新增一项设置而让整套测试误报失败。
        assert!(descriptors().len() >= 130, "设置注册表意外缩减");
        assert!(categories().len() >= 18);
        let mut keys = std::collections::HashSet::new();
        for item in descriptors() {
            assert!(keys.insert(&item.key), "设置键重复：{}", item.key);
            assert!(
                is_category(&item.category),
                "{} 的分类不在分类表里",
                item.key
            );
            if item.value_type == SettingValueType::Enum {
                assert!(
                    item.options.as_ref().is_some_and(|o| !o.is_empty()),
                    "{} 是枚举却没有 options",
                    item.key
                );
            }
        }
    }

    /// 默认值本身必须是合法值，否则「恢复默认」会把设置写成非法状态。
    #[test]
    fn every_default_value_passes_its_own_validation() {
        for item in descriptors() {
            let coerced = coerce(item, &item.default_value.to_json())
                .unwrap_or_else(|e| panic!("{} 的默认值不合法: {e}", item.key));
            assert!(!coerced.clamped, "{} 的默认值落在 min/max 之外", item.key);
            assert_eq!(coerced.value, item.default_value, "{}", item.key);
        }
    }

    #[test]
    fn unset_settings_read_as_their_default() {
        let s = app_settings();
        let font = d("typography.fontSize");
        assert_eq!(s.read(font), font.default_value);
        assert_eq!(s.snapshot(font)["isDefault"], true);
    }

    #[test]
    fn customization_persists_and_resets_without_changing_other_preferences() {
        let settings = app_settings();
        for (key, raw) in [
            ("appearance.uiFontSize", json!(16)),
            ("appearance.dark.foreground", json!("#aBc")),
            ("dashboard.panelGap", json!(28)),
            ("schedule.hourHeight", json!(96)),
        ] {
            let item = d(key);
            let expected = coerce(item, &raw).unwrap().value;
            settings.write(item, &expected);
            settings.flush().unwrap();
            let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
                settings.store().file_path().to_path_buf(),
            ))));
            assert_eq!(reopened.read(item), expected);
            reopened.reset(item);
            reopened.flush().unwrap();
            assert_eq!(reopened.read(item), item.default_value);
            settings.store().reload().unwrap();
        }
    }

    #[test]
    fn custom_colors_accept_auto_short_hex_and_rgb_but_reject_invalid_values() {
        let item = d("appearance.dark.foreground");
        for (input, expected) in [
            ("#aBc", "#aabbcc"),
            ("rgb(1, 12, 255)", "#010cff"),
            (" AUTO ", "auto"),
            ("", "auto"),
        ] {
            assert_eq!(
                coerce(item, &json!(input)).unwrap().value,
                SettingValue::Text(expected.into())
            );
        }
        for input in [
            json!("#xx1234"),
            json!("rgb(300,0,0)"),
            json!("rgb(1,2)"),
            json!("var(--unknown)"),
            json!(123),
        ] {
            assert!(coerce(item, &input).is_err(), "{input}");
        }
        let settings = app_settings();
        settings
            .store()
            .set("app.appearance.dark.foreground", "invalid");
        assert_eq!(settings.read(item), item.default_value);
    }

    #[test]
    fn numeric_preferences_loaded_from_disk_obey_layout_bounds() {
        let settings = app_settings();
        let item = d("scrollbar.width");
        for (raw, expected) in [("-200", 3.0), ("999999", 16.0)] {
            settings.store().set("app.scrollbar.width", raw);
            assert_eq!(settings.read(item), SettingValue::Number(expected));
        }
        for raw in ["NaN", "inf"] {
            settings.store().set("app.scrollbar.width", raw);
            assert_eq!(settings.read(item), item.default_value);
        }
    }

    #[test]
    fn accent_color_validates_persists_and_falls_back_for_older_settings() {
        let s = app_settings();
        let item = d("appearance.accentColor");
        assert_eq!(s.read(item), SettingValue::Text("#22c55e".into()));
        let value = coerce(item, &json!(" #A5F ")).unwrap().value;
        assert_eq!(value, SettingValue::Text("#aa55ff".into()));
        for invalid in ["#abcd", "green", "var(--accent)", "#gggggg"] {
            assert!(coerce(item, &json!(invalid)).is_err());
        }
        s.write(item, &value);
        s.flush().unwrap();
        let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
            s.store().file_path().to_path_buf(),
        ))));
        assert_eq!(reopened.read(item), value);
        reopened.store().set("app.appearance.accentColor", "broken");
        assert_eq!(reopened.read(item), item.default_value);
    }

    #[test]
    fn favorite_parent_display_defaults_false_and_persists_through_shared_settings() {
        let s = app_settings();
        let item = d("sidebar.showFavoriteParents");
        assert_eq!(item.value_type, SettingValueType::Boolean);
        assert_eq!(item.default_value, SettingValue::Bool(false));
        assert_eq!(s.read(item), SettingValue::Bool(false));
        assert_eq!(s.snapshot(item)["isDefault"], true);

        s.write(item, &SettingValue::Bool(true));
        assert_eq!(
            s.store().get("app.sidebar.showFavoriteParents"),
            Some("true".into())
        );
        s.flush().unwrap();
        let path = s.store().file_path().to_path_buf();
        let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(path))));
        assert_eq!(reopened.read(item), SettingValue::Bool(true));
        assert_eq!(reopened.snapshot(item)["isDefault"], false);
    }

    #[test]
    fn simple_document_mode_defaults_true_and_persists_through_shared_settings() {
        let s = app_settings();
        let item = d("editor.simpleDocumentMode");
        assert_eq!(item.value_type, SettingValueType::Boolean);
        assert_eq!(item.default_value, SettingValue::Bool(true));
        assert_eq!(s.read(item), SettingValue::Bool(true));
        assert_eq!(s.snapshot(item)["isDefault"], true);

        s.write(item, &SettingValue::Bool(false));
        assert_eq!(
            s.store().get("app.editor.simpleDocumentMode"),
            Some("false".into())
        );
        s.flush().unwrap();
        let path = s.store().file_path().to_path_buf();
        let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(path))));
        assert_eq!(reopened.read(item), SettingValue::Bool(false));
        assert_eq!(reopened.snapshot(item)["isDefault"], false);
    }

    #[test]
    fn inline_completion_defaults_off_but_an_explicit_opt_in_persists() {
        let s = app_settings();
        let item = d("ai.inlineCompletionEnabled");
        assert_eq!(item.value_type, SettingValueType::Boolean);
        assert_eq!(item.default_value, SettingValue::Bool(false));
        assert_eq!(s.read(item), SettingValue::Bool(false));

        s.write(item, &SettingValue::Bool(true));
        s.flush().unwrap();
        let path = s.store().file_path().to_path_buf();
        let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(path))));
        assert_eq!(reopened.read(item), SettingValue::Bool(true));
        assert_eq!(reopened.snapshot(item)["isDefault"], false);
    }

    #[test]
    fn write_then_read_round_trips_each_type() {
        let s = app_settings();
        for (key, value) in [
            ("typography.fontSize", SettingValue::Number(20.0)),
            ("appearance.themeMode", SettingValue::Text("dark".into())),
            ("background.enabled", SettingValue::Bool(true)),
            (
                "appearance.sidebarBackground",
                SettingValue::Text("#123456".into()),
            ),
        ] {
            let item = d(key);
            s.write(item, &value);
            assert_eq!(s.read(item), value, "{key}");
            assert_eq!(s.snapshot(item)["isDefault"], false, "{key}");
        }
    }

    /// 磁盘上被手改坏的值不该让设置系统瘫掉——退回默认值即可。
    #[test]
    fn corrupt_stored_values_fall_back_to_default() {
        let s = app_settings();
        for (key, junk) in [
            ("typography.fontSize", "不是数字"),
            ("background.enabled", "yes"),
            ("appearance.themeMode", "紫色"),
        ] {
            let item = d(key);
            s.store().set(&storage_key(key), junk);
            assert_eq!(s.read(item), item.default_value, "{key}");
        }
    }

    #[test]
    fn legacy_alpha_colors_keep_their_rgb_channels_after_reload() {
        let settings = app_settings();
        let item = d("appearance.navigationBackground");
        for raw in ["#11223380", "rgba(17, 34, 51, 0.5)"] {
            settings.store().set(&storage_key(&item.key), raw);
            settings.flush().unwrap();
            let reopened = AppSettings::new(Arc::new(SettingsService::new(Some(
                settings.store().file_path().to_path_buf(),
            ))));
            assert_eq!(reopened.read(item), SettingValue::Text("#112233".into()));
            assert_eq!(
                coerce(item, &json!(raw)).unwrap().value,
                reopened.read(item)
            );
        }
        for raw in ["rgba(17,34,51,2)", "rgba(17,34,51,NaN)", "#112233xx"] {
            assert!(coerce(item, &json!(raw)).is_err());
        }
    }

    #[test]
    fn reset_removes_the_key_so_upstream_default_changes_apply() {
        let s = app_settings();
        let item = d("typography.fontSize");
        s.write(item, &SettingValue::Number(24.0));
        s.reset(item);
        assert_eq!(
            s.store().get(&storage_key(&item.key)),
            None,
            "恢复默认应删键而不是写值"
        );
        assert_eq!(s.read(item), item.default_value);
    }

    #[test]
    fn numbers_are_clamped_and_report_it() {
        let item = d("typography.fontSize");
        let (min, max) = (item.min.unwrap(), item.max.unwrap());

        let high = coerce(item, &json!(max + 100.0)).unwrap();
        assert_eq!(high.value, SettingValue::Number(max));
        assert!(high.clamped, "钳过要如实上报，否则模型以为改成了它给的值");

        let low = coerce(item, &json!(min - 100.0)).unwrap();
        assert_eq!(low.value, SettingValue::Number(min));
        assert!(low.clamped);

        let ok = coerce(item, &json!(min)).unwrap();
        assert!(!ok.clamped);
    }

    /// 模型经常把数字写成字符串。
    #[test]
    fn numeric_strings_are_accepted() {
        let item = d("typography.fontSize");
        assert_eq!(
            coerce(item, &json!(" 18 ")).unwrap().value,
            SettingValue::Number(18.0)
        );
        assert!(coerce(item, &json!("大一点")).is_err());
        assert!(coerce(item, &json!(f64::NAN.to_string())).is_err());
    }

    /// step 只是 UI 的步进提示，TS 没有吸附，这里也不能自作主张。
    #[test]
    fn values_are_not_snapped_to_step() {
        let item = descriptors()
            .iter()
            .find(|d| d.step.is_some_and(|s| s > 1.0))
            .expect("总该有个带 step 的设置项");
        let off_grid = item.min.unwrap() + 1.0;
        assert_eq!(
            coerce(item, &json!(off_grid)).unwrap().value,
            SettingValue::Number(off_grid)
        );
    }

    #[test]
    fn enums_are_case_insensitive_but_stored_canonically() {
        let item = d("appearance.themeMode");
        assert_eq!(
            coerce(item, &json!("DARK")).unwrap().value,
            SettingValue::Text("dark".into())
        );
        let err = coerce(item, &json!("紫色")).unwrap_err();
        assert!(
            err.contains("light") && err.contains("dark"),
            "错误里要列出合法取值: {err}"
        );
    }

    #[test]
    fn booleans_accept_the_shapes_models_actually_send() {
        let item = d("background.enabled");
        for raw in [json!(true), json!("true"), json!("1"), json!(1)] {
            assert_eq!(
                coerce(item, &raw).unwrap().value,
                SettingValue::Bool(true),
                "{raw}"
            );
        }
        for raw in [json!(false), json!("false"), json!("0"), json!(0)] {
            assert_eq!(
                coerce(item, &raw).unwrap().value,
                SettingValue::Bool(false),
                "{raw}"
            );
        }
        assert!(coerce(item, &json!("开")).is_err());
    }

    #[test]
    fn search_matches_key_label_description_and_category() {
        assert!(search("fontSize")
            .iter()
            .any(|d| d.key == "typography.fontSize"));
        assert!(search("字号")
            .iter()
            .any(|d| d.key.starts_with("typography.")));
        assert!(!search("深色").is_empty(), "中文说明也该能搜到");
        assert!(search("外观").iter().any(|d| d.category == "appearance"));
        assert!(search("AI 对话外观")
            .iter()
            .any(|d| d.category == "assistant"));
        assert_eq!(search("   ").len(), descriptors().len(), "空查询返回全部");
        assert!(search("绝不可能存在的关键词").is_empty());
    }

    /// 快照里可选字段缺失时整个键不出现，免得模型把 null 读成"无限制"。
    #[test]
    fn snapshot_omits_absent_optional_fields() {
        let s = app_settings();
        let color = s.snapshot(d("appearance.sidebarBackground"));
        assert!(!color.as_object().unwrap().contains_key("min"));
        assert!(!color.as_object().unwrap().contains_key("options"));
        assert_eq!(color["type"], "color");

        let num = s.snapshot(d("typography.fontSize"));
        assert!(num["min"].is_number() && num["max"].is_number());
        assert_eq!(num["categoryLabel"], "正文排版");
    }

    /// 整数不该被打成 1000.0——模型和设置面板看到的都该是 1000。
    #[test]
    fn integral_numbers_serialize_without_a_fraction() {
        assert_eq!(SettingValue::Number(1000.0).to_json().to_string(), "1000");
        assert_eq!(SettingValue::Number(1000.0).to_storage(), "1000");
        assert_eq!(SettingValue::Number(1.5).to_json().to_string(), "1.5");
    }
}
