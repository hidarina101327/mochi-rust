//! 带版本、可搬移的 `.mcb` 多维表格。结构与查询语义与 `shared/base.ts` 共享；
//! 解析时绝不靠丢弃数据来自救。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub type CellValue = Value;

mod presentation;
pub use presentation::{validate_view_presentation, visible_field_indices};

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    #[default]
    Text,
    Number,
    SingleSelect,
    MultiSelect,
    Date,
    DateTime,
    DateRange,
    Checkbox,
    Url,
    Progress,
    Reference,
    /// 只能指向文档和目录的引用字段。通用 `Reference` 字段可以指向任何 Mochi 对象。
    Document,
}
pub const FIELD_TYPES: [FieldType; 12] = [
    FieldType::Text,
    FieldType::Number,
    FieldType::SingleSelect,
    FieldType::MultiSelect,
    FieldType::Date,
    FieldType::DateTime,
    FieldType::DateRange,
    FieldType::Checkbox,
    FieldType::Url,
    FieldType::Progress,
    FieldType::Reference,
    FieldType::Document,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OptionColor {
    #[default]
    Gray,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Pink,
}
pub const OPTION_COLORS: [OptionColor; 8] = [
    OptionColor::Gray,
    OptionColor::Red,
    OptionColor::Orange,
    OptionColor::Yellow,
    OptionColor::Green,
    OptionColor::Blue,
    OptionColor::Purple,
    OptionColor::Pink,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewType {
    #[default]
    Grid,
    Board,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOperator {
    #[default]
    Contains,
    Equals,
    NotEquals,
    IsEmpty,
    IsNotEmpty,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BaseOption {
    pub id: String,
    pub label: String,
    pub color: OptionColor,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BaseField {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<BaseOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BaseRecord {
    pub id: String,
    pub values: Map<String, CellValue>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseSort {
    pub field_id: String,
    pub direction: SortDirection,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn deserialize_optional_value<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    // 显式的 null 是有意义的相等过滤，跟「没写这个键」不是一回事。
    Ok(Some(Value::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseFilter {
    pub field_id: String,
    pub operator: FilterOperator,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<CellValue>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseView {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub view_type: ViewType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<String>,
    pub sorts: Vec<BaseSort>,
    pub filters: Vec<BaseFilter>,
    /// 表格行高，逻辑像素。文件里没写的沿用 38px 的老默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_height: Option<f64>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub frozen_rows: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub frozen_columns: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_field_ids: Vec<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseTable {
    pub id: String,
    pub name: String,
    pub fields: Vec<BaseField>,
    pub records: Vec<BaseRecord>,
    pub views: Vec<BaseView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_view_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseDocument {
    pub format: String,
    pub version: u32,
    pub tables: Vec<BaseTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_table_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 稳定的导航目标；只有字段没有记录时指向列头。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseLocation {
    pub table_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BaseSearchEntry {
    pub text: String,
    pub location: BaseLocation,
}

/// 只索引表格里实际显示的标签，选项 ID 和 JSON 语法永远不参与索引。
pub fn base_search_entries(document: &BaseDocument) -> Vec<BaseSearchEntry> {
    let mut entries = Vec::new();
    for table in &document.tables {
        let location = BaseLocation {
            table_id: table.id.clone(),
            ..Default::default()
        };
        entries.push(BaseSearchEntry {
            text: table.name.clone(),
            location: location.clone(),
        });
        for field in &table.fields {
            entries.push(BaseSearchEntry {
                text: field.name.clone(),
                location: BaseLocation {
                    field_id: Some(field.id.clone()),
                    ..location.clone()
                },
            });
        }
        for record in &table.records {
            for field in &table.fields {
                let text =
                    format_cell_value(field, record.values.get(&field.id).unwrap_or(&Value::Null));
                if !text.is_empty() {
                    entries.push(BaseSearchEntry {
                        text,
                        location: BaseLocation {
                            record_id: Some(record.id.clone()),
                            field_id: Some(field.id.clone()),
                            ..location.clone()
                        },
                    });
                }
            }
        }
    }
    entries
}

pub fn create_id(prefix: &str) -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}_{timestamp:x}_{:x}_{:x}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}
pub fn create_base_field(field_type: FieldType, name: &str) -> BaseField {
    BaseField {
        id: create_id("field"),
        name: name.into(),
        field_type,
        ..Default::default()
    }
}
pub fn create_base_view(name: &str, view_type: ViewType) -> BaseView {
    BaseView {
        id: create_id("view"),
        name: name.into(),
        view_type,
        ..Default::default()
    }
}
pub fn create_base_table(name: &str) -> BaseTable {
    let view = create_base_view("表格视图", ViewType::Grid);
    BaseTable {
        id: create_id("table"),
        name: name.into(),
        fields: vec![create_base_field(FieldType::Text, "名称")],
        active_view_id: Some(view.id.clone()),
        views: vec![view],
        ..Default::default()
    }
}
pub fn create_base_document() -> BaseDocument {
    let table = create_base_table("数据表");
    BaseDocument {
        format: "mochi-base".into(),
        version: 1,
        active_table_id: Some(table.id.clone()),
        tables: vec![table],
        ..Default::default()
    }
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .with_context(|| format!("{path} 必须是对象"))
}
fn array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>> {
    value
        .as_array()
        .with_context(|| format!("{path} 必须是数组"))
}
fn nonempty<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("{path} 必须是非空字符串"))
}
fn add_id<'a>(value: &'a Value, ids: &mut HashSet<&'a str>, path: &str) -> Result<()> {
    if !ids.insert(nonempty(value, path)?) {
        bail!("{path} 标识符重复")
    }
    Ok(())
}
fn reference(value: &Value, ids: &HashSet<&str>, path: &str) -> Result<()> {
    if !ids.contains(nonempty(value, path)?) {
        bail!("{path} 引用不存在的标识符")
    }
    Ok(())
}
fn member(value: &Value, allowed: &[&str], path: &str) -> Result<()> {
    if !value.as_str().is_some_and(|v| allowed.contains(&v)) {
        bail!("{path} 包含不支持的值")
    }
    Ok(())
}
fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<usize>().unwrap_or(0);
    let day = value[8..].parse::<u32>().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    year >= 1
        && (1..=12).contains(&month)
        && day >= 1
        && day
            <= [
                31,
                if leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ][month - 1]
}
fn cell_shape(value: &Value, path: &str) -> Result<()> {
    if value.is_null()
        || value.is_string()
        || value.is_boolean()
        || value.as_f64().is_some_and(f64::is_finite)
        || value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
    {
        return Ok(());
    }
    bail!("{path} 不是有效的单元格值")
}
pub fn valid_date_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 16
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
        && valid_date(&value[..10])
        && value[11..13].parse::<u8>().is_ok_and(|hour| hour < 24)
        && value[14..16].parse::<u8>().is_ok_and(|minute| minute < 60)
}
pub fn valid_date_range(value: &Value) -> bool {
    value.as_array().is_some_and(|range| {
        range.len() == 2
            && range[0].as_str().is_some_and(valid_date_time)
            && range[1].as_str().is_some_and(valid_date_time)
            && range[0].as_str() <= range[1].as_str()
    })
}
/// 引用沿用现有的 Mochi URL 协议。路径穿越、写了一半的记录/日程定位符，
/// 一律不许进可搬移的 base 文件。
pub fn parse_base_reference(value: &str) -> Option<crate::mochi_url::ResourceRef> {
    use crate::mochi_url::ResourceKind;
    if value.chars().any(char::is_whitespace) || value.contains('#') {
        return None;
    }
    let query = value.split_once('?')?.1;
    let bytes = query.as_bytes();
    if bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'%'
            && !bytes
                .get(index + 1..index + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    }) {
        return None;
    }
    let mut keys = HashSet::new();
    for pair in query.split('&') {
        let key = crate::mochi_url::form_decode(pair.split_once('=').map_or(pair, |pair| pair.0));
        if ["path", "kind", "table", "record", "field", "item", "label"].contains(&key.as_str())
            && !keys.insert(key)
        {
            return None;
        }
    }
    let invalid_char =
        |character: char| character <= '\u{1f}' || character == '\u{7f}' || character == '\u{fffd}';
    let reference = crate::mochi_url::parse_mochi_resource_url(value)?;
    if reference.path.trim().is_empty()
        || reference.path.chars().any(invalid_char)
        || reference.path.split('/').any(|part| part == "..")
        || [
            &reference.table_id,
            &reference.record_id,
            &reference.field_id,
            &reference.item_id,
            &reference.label,
        ]
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty() || value.chars().any(invalid_char))
    {
        return None;
    }
    if (reference.record_id.is_some() || reference.field_id.is_some())
        && reference.table_id.is_none()
    {
        return None;
    }
    if reference.table_id.is_some()
        && (reference.kind != ResourceKind::File
            || !reference.path.to_ascii_lowercase().ends_with(".mcb"))
    {
        return None;
    }
    if if matches!(
        reference.kind,
        ResourceKind::Task | ResourceKind::Event | ResourceKind::Project | ResourceKind::AiSession
    ) {
        reference.item_id.is_none()
    } else {
        reference.item_id.is_some()
    } {
        return None;
    }
    Some(reference)
}
pub fn is_valid_base_reference(value: &str) -> bool {
    parse_base_reference(value).is_some()
        || crate::object_reference::ObjectReference::parse(value).is_some_and(|reference| {
            matches!(
                reference.kind,
                crate::object_reference::ObjectKind::AiSession
                    | crate::object_reference::ObjectKind::AiMessage
                    | crate::object_reference::ObjectKind::Block
            )
        })
}

/// 校验仅指向文档的单元格引用。文档字段接受与通用引用相同的
/// 可搬移 Mochi URL 语法，但日程记录和 AI 会话目标除外。
pub fn is_valid_document_reference(value: &str) -> bool {
    crate::object_reference::ObjectReference::parse(value)
        .is_some_and(|reference| reference.is_document())
}

pub fn format_base_reference(value: &str) -> String {
    if let Some(reference) = crate::object_reference::ObjectReference::parse(value) {
        if matches!(
            reference.kind,
            crate::object_reference::ObjectKind::AiSession
                | crate::object_reference::ObjectKind::AiMessage
                | crate::object_reference::ObjectKind::Block
        ) {
            return reference.display_label();
        }
    }
    let Some(reference) = parse_base_reference(value) else {
        return value.into();
    };
    if let Some(label) = reference.label {
        return label;
    }
    let file = reference
        .path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&reference.path);
    let name = if file.to_ascii_lowercase().ends_with(".md") {
        &file[..file.len() - 3]
    } else {
        file
    };
    if let Some(record) = reference.record_id.or(reference.item_id) {
        format!("{name} · {record}")
    } else {
        name.into()
    }
}
fn validate_cell(field: &Value, value: &Value, path: &str) -> Result<()> {
    cell_shape(value, path)?;
    if value.is_null() {
        return Ok(());
    }
    match field["type"].as_str().unwrap_or("") {
        "text" | "url" if !value.is_string() => bail!("{path} 必须是字符串或 null"),
        "date" if !value.as_str().is_some_and(valid_date) => {
            bail!("{path} 必须是有效的 YYYY-MM-DD 日期或 null")
        }
        "dateTime" if !value.as_str().is_some_and(valid_date_time) => {
            bail!("{path} 必须是有效的 YYYY-MM-DDTHH:mm 时间点或 null")
        }
        "dateRange" if !valid_date_range(value) => {
            bail!("{path} 必须包含开始、结束两个有效时间点，且开始时间不得晚于结束时间")
        }
        "number" | "progress" => {
            let number = value
                .as_f64()
                .filter(|n| n.is_finite())
                .with_context(|| format!("{path} 必须是有限数值或 null"))?;
            if field["type"] == "progress" && !(0.0..=100.0).contains(&number) {
                bail!("{path} 进度必须在 0 到 100 之间")
            }
        }
        "checkbox" if !value.is_boolean() => bail!("{path} 必须是布尔值或 null"),
        "reference" | "document" => {
            let mut seen = HashSet::new();
            for item in array(value, path)? {
                let url = item
                    .as_str()
                    .filter(|url| {
                        if field["type"] == "document" {
                            is_valid_document_reference(url)
                        } else {
                            is_valid_base_reference(url)
                        }
                    })
                    .with_context(|| format!("{path} 必须包含有效的 Mochi 引用链接"))?;
                if !seen.insert(url) {
                    bail!("{path} 引用链接重复")
                }
            }
        }
        "singleSelect" | "multiSelect" => {
            let option_ids: HashSet<&str> = field["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| o["id"].as_str())
                .collect();
            if field["type"] == "singleSelect" {
                reference(value, &option_ids, path)?;
            } else {
                let mut selected = HashSet::new();
                for (index, id) in array(value, path)?.iter().enumerate() {
                    reference(id, &option_ids, &format!("{path}[{index}]"))?;
                    if !selected.insert(id.as_str().unwrap()) {
                        bail!("{path} 选项标识符重复")
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub const BASE_DOCUMENT_VERSION: u64 = 1;

fn migrate_base_value_once(version: u64, _document: Value) -> Result<Value> {
    bail!("缺少从 v{version} 到 v{} 的迁移器", version + 1)
}

fn migrate_base_value(mut document: Value) -> Result<Value> {
    let raw_version = document["version"].as_f64().context("version 必须是整数")?;
    if !raw_version.is_finite() || raw_version.fract() != 0.0 || raw_version < 0.0 {
        bail!("version 必须是整数")
    }
    let mut version = raw_version as u64;
    if version > BASE_DOCUMENT_VERSION {
        bail!("version 来自更新版本（当前支持 {BASE_DOCUMENT_VERSION}）")
    }
    while version < BASE_DOCUMENT_VERSION {
        // 以后的迁移放在这里，一次只挪一个相邻版本。集中处理
        // 可以避免读写两端各自长出一堆散落的兼容分支。
        document = migrate_base_value_once(version, document)?;
        version += 1;
        document["version"] = Value::from(version);
    }
    document["version"] = Value::from(BASE_DOCUMENT_VERSION);
    Ok(document)
}

fn validate_value(document: &Value) -> Result<()> {
    object(document, "$")?;
    if document["format"] != "mochi-base" {
        bail!("format 必须为 mochi-base")
    }
    if document["version"].as_u64() != Some(BASE_DOCUMENT_VERSION) {
        bail!("version 不支持此版本")
    }
    let tables = array(&document["tables"], "tables")?;
    if tables.is_empty() {
        bail!("tables 至少需要一个数据表")
    }
    let mut table_ids = HashSet::new();
    for (table_index, table) in tables.iter().enumerate() {
        let path = format!("tables[{table_index}]");
        object(table, &path)?;
        add_id(&table["id"], &mut table_ids, &format!("{path}.id"))?;
        nonempty(&table["name"], &format!("{path}.name"))?;
        let fields = array(&table["fields"], &format!("{path}.fields"))?;
        if fields.is_empty() {
            bail!("{path}.fields 至少需要一个字段")
        }
        let mut field_ids = HashSet::new();
        let mut field_map = HashMap::new();
        for (field_index, field) in fields.iter().enumerate() {
            let fp = format!("{path}.fields[{field_index}]");
            object(field, &fp)?;
            add_id(&field["id"], &mut field_ids, &format!("{fp}.id"))?;
            nonempty(&field["name"], &format!("{fp}.name"))?;
            member(
                &field["type"],
                &[
                    "text",
                    "number",
                    "singleSelect",
                    "multiSelect",
                    "date",
                    "dateTime",
                    "dateRange",
                    "checkbox",
                    "url",
                    "progress",
                    "reference",
                    "document",
                ],
                &format!("{fp}.type"),
            )?;
            if let Some(width) = field.get("width") {
                if !width.as_f64().is_some_and(|n| n.is_finite() && n > 0.0) {
                    bail!("{fp}.width 必须是大于 0 的有限数值")
                }
            }
            if let Some(options) = field.get("options") {
                let mut option_ids = HashSet::new();
                for (option_index, option) in
                    array(options, &format!("{fp}.options"))?.iter().enumerate()
                {
                    let op = format!("{fp}.options[{option_index}]");
                    object(option, &op)?;
                    add_id(&option["id"], &mut option_ids, &format!("{op}.id"))?;
                    nonempty(&option["label"], &format!("{op}.label"))?;
                    member(
                        &option["color"],
                        &[
                            "gray", "red", "orange", "yellow", "green", "blue", "purple", "pink",
                        ],
                        &format!("{op}.color"),
                    )?;
                }
            }
            field_map.insert(field["id"].as_str().unwrap(), field);
        }
        let mut record_ids = HashSet::new();
        for (record_index, record) in array(&table["records"], &format!("{path}.records"))?
            .iter()
            .enumerate()
        {
            let rp = format!("{path}.records[{record_index}]");
            object(record, &rp)?;
            add_id(&record["id"], &mut record_ids, &format!("{rp}.id"))?;
            for (field_id, value) in object(&record["values"], &format!("{rp}.values"))? {
                let field = field_map
                    .get(field_id.as_str())
                    .with_context(|| format!("{rp}.values.{field_id} 引用不存在的字段"))?;
                validate_cell(field, value, &format!("{rp}.values.{field_id}"))?;
            }
        }
        let views = array(&table["views"], &format!("{path}.views"))?;
        if views.is_empty() {
            bail!("{path}.views 至少需要一个视图")
        }
        let mut view_ids = HashSet::new();
        for (view_index, view) in views.iter().enumerate() {
            let vp = format!("{path}.views[{view_index}]");
            object(view, &vp)?;
            add_id(&view["id"], &mut view_ids, &format!("{vp}.id"))?;
            nonempty(&view["name"], &format!("{vp}.name"))?;
            member(&view["type"], &["grid", "board"], &format!("{vp}.type"))?;
            validate_view_presentation(view, &vp)?;
            if let Some(group_by) = view.get("groupBy") {
                reference(group_by, &field_ids, &format!("{vp}.groupBy"))?;
                if field_map[group_by.as_str().unwrap()]["type"] != "singleSelect" {
                    bail!("{vp}.groupBy 分组字段必须为单选字段")
                }
            }
            for (index, sort) in array(&view["sorts"], &format!("{vp}.sorts"))?
                .iter()
                .enumerate()
            {
                let sp = format!("{vp}.sorts[{index}]");
                object(sort, &sp)?;
                reference(&sort["fieldId"], &field_ids, &format!("{sp}.fieldId"))?;
                member(
                    &sort["direction"],
                    &["asc", "desc"],
                    &format!("{sp}.direction"),
                )?;
            }
            for (index, filter) in array(&view["filters"], &format!("{vp}.filters"))?
                .iter()
                .enumerate()
            {
                let fp = format!("{vp}.filters[{index}]");
                object(filter, &fp)?;
                reference(&filter["fieldId"], &field_ids, &format!("{fp}.fieldId"))?;
                member(
                    &filter["operator"],
                    &["contains", "equals", "notEquals", "isEmpty", "isNotEmpty"],
                    &format!("{fp}.operator"),
                )?;
                if filter["operator"] != "isEmpty"
                    && filter["operator"] != "isNotEmpty"
                    && filter.get("value").is_none()
                {
                    bail!("{fp}.value 缺少筛选值")
                }
                if let Some(value) = filter.get("value") {
                    cell_shape(value, &format!("{fp}.value"))?;
                }
            }
        }
        if let Some(id) = table.get("activeViewId") {
            reference(id, &view_ids, &format!("{path}.activeViewId"))?;
        }
    }
    if let Some(id) = document.get("activeTableId") {
        reference(id, &table_ids, "activeTableId")?;
    }
    Ok(())
}

pub fn parse_base_document(text: &str) -> Result<BaseDocument> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .context("多维表格格式无效：无法解析 JSON")?;
    let value = migrate_base_value(value).context("多维表格格式无效")?;
    validate_value(&value).context("多维表格格式无效")?;
    serde_json::from_value(value).context("多维表格格式无效")
}
pub fn validate_base_document(document: &BaseDocument) -> Result<()> {
    validate_value(&serde_json::to_value(document)?).context("多维表格格式无效")
}
pub fn serialize_base_document(document: &BaseDocument) -> Result<String> {
    validate_base_document(document)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(document)?))
}

pub fn is_cell_empty(value: &CellValue) -> bool {
    value.is_null() || value.as_str() == Some("") || value.as_array().is_some_and(Vec::is_empty)
}
fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map(number_text).unwrap_or_default(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => a.iter().map(scalar_text).collect::<Vec<_>>().join(","),
        _ => String::new(),
    }
}
// 指数阈值与 JavaScript 对齐，这样超大/极小数值在过滤和搜索时行为一致，
// 且不用引入额外依赖。
fn number_text(number: f64) -> String {
    if number == 0.0 {
        return "0".into();
    }
    let text = number.to_string();
    let magnitude = number.abs();
    if (0.000001..1e21).contains(&magnitude) {
        return text;
    }
    let (sign, digits) = text
        .strip_prefix('-')
        .map(|s| ("-", s))
        .unwrap_or(("", &text));
    let (significant, exponent) = if let Some(fraction) = digits.strip_prefix("0.") {
        let leading_zeroes = fraction.bytes().take_while(|b| *b == b'0').count();
        (&fraction[leading_zeroes..], -(leading_zeroes as i32) - 1)
    } else {
        (digits.trim_end_matches('0'), digits.len() as i32 - 1)
    };
    let mantissa = if significant.len() <= 1 {
        significant.to_owned()
    } else {
        format!("{}.{}", &significant[..1], &significant[1..])
    };
    format!(
        "{sign}{mantissa}e{}{exponent}",
        if exponent >= 0 { "+" } else { "" }
    )
}
pub fn format_cell_value(field: &BaseField, value: &CellValue) -> String {
    if is_cell_empty(value) {
        return String::new();
    }
    match field.field_type {
        FieldType::SingleSelect => field
            .options
            .iter()
            .find(|o| Some(o.id.as_str()) == value.as_str())
            .map(|o| o.label.clone())
            .unwrap_or_default(),
        FieldType::MultiSelect => value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| {
                field
                    .options
                    .iter()
                    .find(|o| Some(o.id.as_str()) == id.as_str())
                    .map(|o| o.label.as_str())
            })
            .collect::<Vec<_>>()
            .join(", "),
        FieldType::Progress => format!("{}%", scalar_text(value)),
        FieldType::Reference | FieldType::Document => value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(format_base_reference)
            .collect::<Vec<_>>()
            .join(", "),
        FieldType::DateTime => scalar_text(value).replace('T', " "),
        FieldType::DateRange => value
            .as_array()
            .into_iter()
            .flatten()
            .map(|point| scalar_text(point).replace('T', " "))
            .collect::<Vec<_>>()
            .join(" 至 "),
        _ => scalar_text(value),
    }
}

/// 归一化用户输入的本地日期/时间。存储值保持不带时区、精确到分钟的 ISO 字符串；
/// 非法值和时间戳绝不靠猜。
pub fn normalize_date_time_input(text: &str) -> Option<String> {
    normalize_time_endpoint(text, false)
}
fn normalize_time_endpoint(text: &str, end_of_day: bool) -> Option<String> {
    let text = text.trim();
    if valid_date(text) {
        return Some(format!(
            "{text}T{}",
            if end_of_day { "23:59" } else { "00:00" }
        ));
    }
    let mut point = text.to_owned();
    if point.get(10..11) == Some(" ") {
        point.replace_range(10..11, "T");
    }
    valid_date_time(&point).then_some(point)
}
/// 编辑器接受与字段转换相同的显示分隔符；而解析器只认文件里
/// 存储的规范二元素数组。
pub fn normalize_date_range_input(text: &str) -> Option<CellValue> {
    let text = text.trim();
    let parts = text
        .split(['至', '~', '～', '→'])
        .map(str::trim)
        .collect::<Vec<_>>();
    let (start, end) = match parts.as_slice() {
        [one] => {
            let start = normalize_time_endpoint(one, false)?;
            let end = normalize_time_endpoint(one, true)?;
            (start, end)
        }
        [start, end] => (
            normalize_time_endpoint(start, false)?,
            normalize_time_endpoint(end, true)?,
        ),
        _ => return None,
    };
    (start <= end).then(|| serde_json::json!([start, end]))
}
fn select_type(kind: FieldType) -> bool {
    matches!(kind, FieldType::SingleSelect | FieldType::MultiSelect)
}
fn option_for_label(field: &mut BaseField, label: &str) -> String {
    if let Some(option) = field.options.iter().find(|option| option.label == label) {
        return option.id.clone();
    }
    let id = create_id("option");
    field.options.push(BaseOption {
        id: id.clone(),
        label: label.into(),
        color: OPTION_COLORS[field.options.len() % OPTION_COLORS.len()],
        ..Default::default()
    });
    id
}
fn convert_cell(old: &BaseField, next: &mut BaseField, value: &CellValue) -> CellValue {
    if is_cell_empty(value) {
        return Value::Null;
    }
    if select_type(next.field_type) {
        let selected = if select_type(old.field_type) {
            let values = if let Some(values) = value.as_array() {
                values.clone()
            } else {
                vec![value.clone()]
            };
            values
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .filter(|id| next.options.iter().any(|option| &option.id == id))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if !selected.is_empty() {
            if next.field_type == FieldType::MultiSelect {
                return serde_json::json!(selected);
            }
            if selected.len() == 1 {
                return serde_json::json!(selected[0]);
            }
        }
        let label = format_cell_value(old, value);
        if label.trim().is_empty() {
            return Value::Null;
        }
        let id = option_for_label(next, &label);
        return if next.field_type == FieldType::SingleSelect {
            serde_json::json!(id)
        } else {
            serde_json::json!([id])
        };
    }
    let display = format_cell_value(old, value);
    match next.field_type {
        FieldType::Text | FieldType::Url => Value::String(display),
        FieldType::Number | FieldType::Progress => {
            let number = if let Some(number) = value.as_f64() {
                Some(number)
            } else if let Some(value) = value.as_bool() {
                Some(if value { 1.0 } else { 0.0 })
            } else {
                display
                    .trim()
                    .strip_suffix('%')
                    .unwrap_or(display.trim())
                    .trim()
                    .parse::<f64>()
                    .ok()
            };
            number
                .filter(|number| number.is_finite())
                .map(|number| {
                    serde_json::json!(if next.field_type == FieldType::Progress {
                        number.clamp(0.0, 100.0)
                    } else {
                        number
                    })
                })
                .unwrap_or(Value::Null)
        }
        FieldType::Checkbox => {
            let checked = if let Some(checked) = value.as_bool() {
                Some(checked)
            } else if let Some(number) = value.as_f64() {
                Some(number != 0.0)
            } else {
                match display.trim().to_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" | "是" | "已勾选" => Some(true),
                    "false" | "0" | "no" | "off" | "否" | "未勾选" => Some(false),
                    _ => None,
                }
            };
            checked.map(Value::Bool).unwrap_or(Value::Null)
        }
        FieldType::Date | FieldType::DateTime => {
            let source = if old.field_type == FieldType::DateRange {
                value[0].as_str().unwrap_or("").to_owned()
            } else {
                display
            };
            let point = normalize_date_time_input(&source).or_else(|| {
                normalize_date_range_input(&source)
                    .and_then(|range| range[0].as_str().map(str::to_owned))
            });
            point
                .map(|point| {
                    Value::String(if next.field_type == FieldType::Date {
                        point[..10].to_owned()
                    } else {
                        point
                    })
                })
                .unwrap_or(Value::Null)
        }
        FieldType::DateRange => normalize_date_range_input(&display).unwrap_or(Value::Null),
        FieldType::Reference | FieldType::Document => {
            let document_only = next.field_type == FieldType::Document;
            let mut references = Vec::new();
            let mut add = |url: &str| {
                let valid = if document_only {
                    is_valid_document_reference(url)
                } else {
                    is_valid_base_reference(url)
                };
                if valid && !references.iter().any(|item| item == url) {
                    references.push(url.to_owned());
                }
            };
            if let Some(url) = value.as_str() {
                add(url);
            }
            if let Some(values) = value.as_array() {
                for url in values.iter().filter_map(Value::as_str) {
                    add(url);
                }
            }
            if references.is_empty() {
                Value::Null
            } else {
                serde_json::json!(references)
            }
        }
        FieldType::SingleSelect | FieldType::MultiSelect => unreachable!(),
    }
}
/// 转换既有字段，保留 ID、标签、元数据以及所有能转换的值。
/// 返回因转换而被清空的非空单元格数。改类型是原子的：
/// 不合法的候选绝不会接触到线上表格。
pub fn convert_base_field_type(
    table: &mut BaseTable,
    field_id: &str,
    kind: FieldType,
) -> Result<usize> {
    let index = table
        .fields
        .iter()
        .position(|field| field.id == field_id)
        .context("字段不存在")?;
    if table.fields[index].field_type == kind {
        return Ok(0);
    }
    let mut candidate = table.clone();
    let old = candidate.fields[index].clone();
    let mut next = old.clone();
    next.field_type = kind;
    if !select_type(kind) || !select_type(old.field_type) {
        next.options.clear();
    }
    let old_shape = serde_json::to_value(&old)?;
    let mut cleared = 0;
    for record in &mut candidate.records {
        let had_value = record.values.contains_key(field_id);
        let previous = record.values.get(field_id).unwrap_or(&Value::Null);
        let value = if validate_cell(&old_shape, previous, "value").is_ok() {
            convert_cell(&old, &mut next, previous)
        } else {
            Value::Null
        };
        if !is_cell_empty(previous) && value.is_null() {
            cleared += 1;
        }
        if had_value || !value.is_null() {
            record.values.insert(field_id.into(), value);
        }
    }
    candidate.fields[index] = next;
    for view in &mut candidate.views {
        view.filters.retain(|filter| filter.field_id != field_id);
        view.sorts.retain(|sort| sort.field_id != field_id);
        if kind != FieldType::SingleSelect && view.group_by.as_deref() == Some(field_id) {
            view.group_by = None;
        }
    }
    validate_base_document(&BaseDocument {
        format: "mochi-base".into(),
        version: 1,
        tables: vec![candidate.clone()],
        ..Default::default()
    })?;
    *table = candidate;
    Ok(cleared)
}
fn equals_cell(field: &BaseField, left: &Value, right: &Value) -> bool {
    if is_cell_empty(left) && is_cell_empty(right) {
        return true;
    }
    if field.field_type == FieldType::DateRange {
        return left == right;
    }
    if let Some(a) = left.as_array() {
        if right.is_string() {
            return a.contains(right);
        }
        if let Some(b) = right.as_array() {
            return a.len() == b.len() && a.iter().all(|v| b.contains(v));
        }
    }
    if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
        return a == b;
    }
    left == right
}

/// 过滤条件之间是 AND 关系；排序相同时保持源顺序；空单元格排在最后。
pub fn get_view_records<'a>(
    table: &'a BaseTable,
    view: &BaseView,
    search: &str,
) -> Vec<&'a BaseRecord> {
    let field_map: HashMap<&str, &BaseField> = table
        .fields
        .iter()
        .map(|field| (field.id.as_str(), field))
        .collect();
    let query = search.trim().to_lowercase();
    let null = Value::Null;
    let mut records: Vec<_> = table
        .records
        .iter()
        .filter(|record| {
            if !query.is_empty()
                && !table.fields.iter().any(|field| {
                    format_cell_value(field, record.values.get(&field.id).unwrap_or(&null))
                        .to_lowercase()
                        .contains(&query)
                })
            {
                return false;
            }
            view.filters.iter().all(|filter| {
                let Some(field) = field_map.get(filter.field_id.as_str()) else {
                    return false;
                };
                let value = record.values.get(&field.id).unwrap_or(&null);
                let expected = filter.value.as_ref().unwrap_or(&null);
                match filter.operator {
                    FilterOperator::IsEmpty => is_cell_empty(value),
                    FilterOperator::IsNotEmpty => !is_cell_empty(value),
                    FilterOperator::Equals => equals_cell(field, value, expected),
                    FilterOperator::NotEquals => !equals_cell(field, value, expected),
                    FilterOperator::Contains => format_cell_value(field, value)
                        .to_lowercase()
                        .contains(&scalar_text(expected).to_lowercase()),
                }
            })
        })
        .collect();
    records.sort_by(|left, right| {
        for sort in &view.sorts {
            let Some(field) = field_map.get(sort.field_id.as_str()) else {
                continue;
            };
            let a = left.values.get(&field.id).unwrap_or(&null);
            let b = right.values.get(&field.id).unwrap_or(&null);
            let a_empty = is_cell_empty(a);
            let b_empty = is_cell_empty(b);
            if a_empty != b_empty {
                return a_empty.cmp(&b_empty);
            }
            if a_empty && b_empty {
                continue;
            }
            let order = if field.field_type == FieldType::DateRange {
                a[0].as_str()
                    .cmp(&b[0].as_str())
                    .then(a[1].as_str().cmp(&b[1].as_str()))
            } else if let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) {
                a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
            } else if let (Some(a), Some(b)) = (a.as_bool(), b.as_bool()) {
                a.cmp(&b)
            } else {
                format_cell_value(field, a).cmp(&format_cell_value(field, b))
            };
            if !order.is_eq() {
                return if sort.direction == SortDirection::Desc {
                    order.reverse()
                } else {
                    order
                };
            }
        }
        std::cmp::Ordering::Equal
    });
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base_references_validate_format_convert_and_search_display_labels() {
        let note = "mochi://open?path=%E7%AC%94%E8%AE%B0%2F%E5%A4%8D%E4%B9%A0.md";
        let block = "mochi://block?path=%E7%AC%94%E8%AE%B0%2F%E5%A4%8D%E4%B9%A0.md&id=block_00000000-0000-4000-8000-000000000001&label=%E5%A4%8D%E4%B9%A0%E5%9D%97";
        let record = "mochi://open?path=study.mcb&table=t&record=r&field=f";
        let task="mochi://open?path=schedule&kind=task&item=task-1&label=%E5%A4%8D%E4%B9%A0%E8%AE%A1%E5%88%92";
        assert_eq!(format_base_reference(note), "复习");
        assert_eq!(format_base_reference(record), "study.mcb · r");
        assert_eq!(format_base_reference(task), "复习计划");
        for url in [
            note,
            block,
            record,
            task,
            "mochi://open?path=C%3A%2Foutside.pdf",
            "mochi://open?path=study.mcb&table=t&field=f",
            "mochi://open?path=notes&kind=directory",
        ] {
            assert!(is_valid_base_reference(url), "{url}");
        }
        for url in [
            "mochi://open?path=../secret",
            "mochi://open?path=%2E%2E%2Fsecret",
            "mochi://open?path=x&path=y",
            "mochi://open?path=%GG",
            "mochi://open?path=%FF",
            "mochi://open?path=x%00",
            "mochi://open?path=note.md&table=t",
            "mochi://open?path=study.mcb&record=r",
            "mochi://open?path=schedule&kind=task",
            "mochi://open?path=x&kind=directory&item=i",
            "mochi://open?path=x#heading",
            " mochi://open?path=x",
            "mochi://open?path=x&label=",
        ] {
            assert!(!is_valid_base_reference(url), "accepted {url}");
        }
        let mut document = create_base_document();
        let table = &mut document.tables[0];
        let field = create_base_field(FieldType::Reference, "学习材料");
        let field_id = field.id.clone();
        table.fields.push(field);
        let mut row = BaseRecord {
            id: "row".into(),
            ..Default::default()
        };
        row.values
            .insert(field_id.clone(), json!([note, task, block]));
        table.records.push(row);
        assert_eq!(
            get_view_records(table, &table.views[0], "复习计划").len(),
            1
        );
        assert_eq!(get_view_records(table, &table.views[0], "task-1").len(), 0);
        assert_eq!(
            format_cell_value(&table.fields[1], &table.records[0].values[&field_id]),
            "复习, 复习计划, 复习块"
        );
        assert_eq!(
            parse_base_document(&serialize_base_document(&document).unwrap()).unwrap(),
            document
        );
        document.tables[0].records[0]
            .values
            .insert(field_id.clone(), json!([note, note]));
        assert!(serialize_base_document(&document).is_err());
        document.tables[0].records[0]
            .values
            .insert(field_id.clone(), json!([note]));
        assert_eq!(
            convert_base_field_type(&mut document.tables[0], &field_id, FieldType::Text).unwrap(),
            0
        );
        assert_eq!(
            document.tables[0].records[0].values[&field_id],
            json!("复习")
        );
        document.tables[0].records[0]
            .values
            .insert(field_id.clone(), json!(note));
        assert_eq!(
            convert_base_field_type(&mut document.tables[0], &field_id, FieldType::Reference)
                .unwrap(),
            0
        );
        assert_eq!(
            document.tables[0].records[0].values[&field_id],
            json!([note])
        );
    }

    #[test]
    fn document_field_only_accepts_document_objects_and_round_trips() {
        let note = "mochi://open?path=notes%2Fstudy.md";
        let block = "mochi://block?path=notes%2Fstudy.md&id=block_00000000-0000-4000-8000-000000000001&label=%E5%AE%9E%E9%AA%8C%E5%9D%97";
        let task = "mochi://open?path=schedule&kind=task&item=task-1";
        assert!(is_valid_document_reference(note));
        assert!(is_valid_document_reference(block));
        assert_eq!(format_base_reference(block), "实验块");
        assert!(!is_valid_document_reference(task));
        let mut document = create_base_document();
        let field = create_base_field(FieldType::Document, "文档");
        let field_id = field.id.clone();
        document.tables[0].fields.push(field);
        document.tables[0].records.push(BaseRecord {
            id: "document-row".into(),
            values: [(field_id.clone(), json!([note, block]))]
                .into_iter()
                .collect(),
            ..Default::default()
        });
        let raw = serialize_base_document(&document).unwrap();
        assert_eq!(parse_base_document(&raw).unwrap(), document);
        document.tables[0].records[0]
            .values
            .insert(field_id.clone(), json!([task]));
        assert!(serialize_base_document(&document).is_err());
    }

    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/base-v1.mcb");
    fn fixture() -> BaseDocument {
        parse_base_document(FIXTURE).unwrap()
    }
    fn ids(table: &BaseTable, view: &BaseView, search: &str) -> Vec<String> {
        get_view_records(table, view, search)
            .iter()
            .map(|r| r.id.clone())
            .collect()
    }

    #[test]
    fn base_time_cells_validate_calendar_minute_precision_and_order() {
        let mut document = create_base_document();
        let table = &mut document.tables[0];
        table
            .fields
            .push(create_base_field(FieldType::DateTime, "时间点"));
        table
            .fields
            .push(create_base_field(FieldType::DateRange, "时间段"));
        let point = table.fields[1].id.clone();
        let range = table.fields[2].id.clone();
        let mut record = BaseRecord {
            id: create_id("record"),
            ..Default::default()
        };
        record
            .values
            .insert(point.clone(), json!("2000-02-29T00:00"));
        record.values.insert(
            range.clone(),
            json!(["2026-09-12T09:00", "2026-09-12T18:30"]),
        );
        table.records.push(record);
        let raw = serialize_base_document(&document).unwrap();
        assert_eq!(parse_base_document(&raw).unwrap(), document);
        let original = serde_json::to_value(&document).unwrap();
        for invalid in [
            "0000-01-01T00:00",
            "1900-02-29T12:00",
            "2026-02-30T00:00",
            "2026-09-12T24:00",
            "2026-09-12T23:60",
            "2026-09-12",
            "2026-09-12 09:00",
            "2026-09-12T09:00:00",
            "2026-09-12T09:00Z",
            "2026-09-12T09:00+08:00",
            "2026-9-12T09:00",
            "123456789中01:00",
            "2026-09-中T09:00",
            "2026-09-12T中:00",
            "éééééT09:00",
            "2026-00中T09:00",
        ] {
            let mut value = original.clone();
            value["tables"][0]["records"][0]["values"][&point] = json!(invalid);
            assert!(
                parse_base_document(&value.to_string()).is_err(),
                "{invalid}"
            );
        }
        for invalid in [
            json!([]),
            json!(["2026-09-12T09:00"]),
            json!(["2026-09-12T09:00", "2026-09-12T18:00", "2026-09-13T00:00"]),
            json!(["2026-09-12", "2026-09-13"]),
            json!(["2026-09-12T18:00", "2026-09-12T09:00"]),
            json!("2026-09-12T09:00"),
            json!(["2026-09-12T09:00", null]),
        ] {
            let mut value = original.clone();
            value["tables"][0]["records"][0]["values"][&range] = invalid.clone();
            assert!(
                parse_base_document(&value.to_string()).is_err(),
                "{invalid}"
            );
        }
    }
    #[test]
    fn base_time_ranges_format_sort_and_compare_as_ordered_endpoints() {
        let mut table = create_base_table("时间数据");
        let field = create_base_field(FieldType::DateRange, "时间段");
        let id = field.id.clone();
        table.fields.push(field);
        for (index, value) in [
            json!(["2026-09-12T09:00", "2026-09-12T12:00"]),
            json!(["2026-09-12T08:00", "2026-09-12T20:00"]),
            json!(["2026-09-12T09:00", "2026-09-12T10:00"]),
        ]
        .into_iter()
        .enumerate()
        {
            let mut record = BaseRecord {
                id: format!("r{index}"),
                ..Default::default()
            };
            record.values.insert(id.clone(), value);
            table.records.push(record);
        }
        let mut view = table.views[0].clone();
        view.sorts.push(BaseSort {
            field_id: id.clone(),
            direction: SortDirection::Asc,
            ..Default::default()
        });
        assert_eq!(ids(&table, &view, ""), vec!["r1", "r2", "r0"]);
        assert_eq!(
            format_cell_value(&table.fields[1], &table.records[0].values[&id]),
            "2026-09-12 09:00 至 2026-09-12 12:00"
        );
        for (value, expected) in [
            (json!(["2026-09-12T09:00", "2026-09-12T12:00"]), vec!["r0"]),
            (json!(["2026-09-12T12:00", "2026-09-12T09:00"]), vec![]),
            (json!("2026-09-12T09:00"), vec![]),
        ] {
            view.filters = vec![BaseFilter {
                field_id: id.clone(),
                operator: FilterOperator::Equals,
                value: Some(value),
                ..Default::default()
            }];
            assert_eq!(ids(&table, &view, ""), expected);
        }
        view.filters = vec![];
        assert_eq!(ids(&table, &view, "09:00 至"), vec!["r2", "r0"]);
    }
    #[test]
    fn base_existing_field_conversion_preserves_labels_extensions_and_cleans_views() {
        let mut document = fixture();
        let table = &mut document.tables[0];
        let id = table.fields[3].id.clone();
        let old = table.fields[3].clone();
        let labels = table
            .records
            .iter()
            .map(|record| format_cell_value(&old, record.values.get(&id).unwrap_or(&Value::Null)))
            .collect::<Vec<_>>();
        convert_base_field_type(table, &id, FieldType::SingleSelect).unwrap();
        let field = table.fields[3].clone();
        for (record, label) in table.records.iter().zip(&labels) {
            assert_eq!(
                &format_cell_value(&field, record.values.get(&id).unwrap_or(&Value::Null),),
                label
            );
        }
        assert!(old
            .options
            .iter()
            .all(|option| field.options.contains(option)));
        assert_eq!(old.extra, field.extra);
        convert_base_field_type(table, &id, FieldType::Text).unwrap();
        assert!(table.fields[3].options.is_empty());
        for (record, label) in table.records.iter().zip(&labels) {
            assert_eq!(
                &format_cell_value(
                    &table.fields[3],
                    record.values.get(&id).unwrap_or(&Value::Null),
                ),
                label
            );
        }
        table.views[0].sorts.push(BaseSort {
            field_id: id.clone(),
            ..Default::default()
        });
        table.views[0].filters.push(BaseFilter {
            field_id: id.clone(),
            operator: FilterOperator::Contains,
            value: Some(json!("x")),
            ..Default::default()
        });
        convert_base_field_type(table, &id, FieldType::MultiSelect).unwrap();
        assert!(table.views[0].sorts.iter().all(|sort| sort.field_id != id));
        assert!(table.views[0]
            .filters
            .iter()
            .all(|filter| filter.field_id != id));
        assert!(serialize_base_document(&document).is_ok());
    }
    #[test]
    fn base_field_conversion_normalizes_times_numbers_and_failed_values() {
        fn converted(text: &str, target: FieldType) -> Value {
            let mut table = create_base_table("数据表");
            let id = table.fields[0].id.clone();
            let mut record = BaseRecord {
                id: "record".into(),
                ..Default::default()
            };
            record.values.insert(id.clone(), json!(text));
            table.records.push(record);
            convert_base_field_type(&mut table, &id, target).unwrap();
            table.records[0].values[&id].clone()
        }
        assert_eq!(
            converted("2026-09-12", FieldType::DateTime),
            json!("2026-09-12T00:00")
        );
        assert_eq!(
            converted("2026-09-12", FieldType::DateRange),
            json!(["2026-09-12T00:00", "2026-09-12T23:59"])
        );
        assert_eq!(
            converted("2026-09-12 09:00 至 2026-09-12 10:30", FieldType::DateRange),
            json!(["2026-09-12T09:00", "2026-09-12T10:30"])
        );
        assert_eq!(
            converted("2026-09-12T09:00 → 2026-09-13T10:30", FieldType::Date),
            json!("2026-09-12")
        );
        assert_eq!(
            converted("2026-09-12T11:00 ~ 2026-09-12T10:00", FieldType::DateRange),
            Value::Null
        );
        assert_eq!(converted("175%", FieldType::Progress), json!(100.0));
        assert_eq!(converted("-20", FieldType::Progress), json!(0.0));
        assert_eq!(converted("1.2e3", FieldType::Number), json!(1200.0));
        assert_eq!(converted("NaN", FieldType::Number), Value::Null);
        assert_eq!(converted("  ", FieldType::SingleSelect), Value::Null);
        assert_eq!(converted("未勾选", FieldType::Checkbox), json!(false));
        let mut table = create_base_table("数据表");
        let id = table.fields[0].id.clone();
        let mut record = BaseRecord {
            id: "record".into(),
            ..Default::default()
        };
        record.values.insert(id.clone(), json!("not a date"));
        table.records.push(record);
        assert_eq!(
            convert_base_field_type(&mut table, &id, FieldType::DateTime).unwrap(),
            1
        );
        assert!(table.records[0].values[&id].is_null());
    }

    #[test]
    fn base_field_conversion_preserves_missing_and_explicit_null_values() {
        let mut table = create_base_table("数据表");
        let id = table.fields[0].id.clone();
        for (index, value) in [
            None,
            Some(Value::Null),
            Some(json!("2026-09-12")),
            Some(json!("invalid")),
        ]
        .into_iter()
        .enumerate()
        {
            let mut record = BaseRecord {
                id: format!("record_{index}"),
                ..Default::default()
            };
            if let Some(value) = value {
                record.values.insert(id.clone(), value);
            }
            table.records.push(record);
        }
        assert_eq!(
            convert_base_field_type(&mut table, &id, FieldType::DateTime).unwrap(),
            1
        );
        assert!(!table.records[0].values.contains_key(&id));
        assert_eq!(table.records[1].values.get(&id), Some(&Value::Null));
        assert_eq!(table.records[2].values[&id], json!("2026-09-12T00:00"));
        assert_eq!(table.records[3].values.get(&id), Some(&Value::Null));
    }

    #[test]
    fn base_empty_generic_document_and_unique_ids() {
        let document = create_base_document();
        assert_eq!(
            parse_base_document(&serialize_base_document(&document).unwrap()).unwrap(),
            document
        );
        assert_eq!(document.tables.len(), 1);
        assert!(document.tables[0].records.is_empty());
        assert_eq!(document.tables[0].fields.len(), 1);
        assert_eq!(document.tables[0].fields[0].field_type, FieldType::Text);
        assert_ne!(create_base_table("数据表").id, document.tables[0].id);
        assert!(create_base_field(FieldType::SingleSelect, "单选")
            .options
            .is_empty());
        assert!(parse_base_document(
            &serialize_base_document(&BaseDocument {
                tables: vec![BaseTable {
                    views: vec![create_base_view("看板", ViewType::Board)],
                    active_view_id: None,
                    ..document.tables[0].clone()
                }],
                ..document
            })
            .unwrap()
        )
        .is_ok());
    }

    #[test]
    fn base_fixture_roundtrips_all_fields_extensions_and_null_filter() {
        let document = fixture();
        assert_eq!(document.tables[0].fields.len(), 10);
        assert_eq!(document.tables[0].fields[8].field_type, FieldType::DateTime);
        assert_eq!(
            document.tables[0].fields[9].field_type,
            FieldType::DateRange
        );
        assert_eq!(
            parse_base_document(&serialize_base_document(&document).unwrap()).unwrap(),
            document
        );
        let original: Value = serde_json::from_str(FIXTURE).unwrap();
        let saved = serde_json::to_value(&document).unwrap();
        for path in [
            "/extension",
            "/tables/0/extension",
            "/tables/0/fields/0/extension",
            "/tables/0/fields/2/options/0/extension",
            "/tables/0/records/0/extension",
            "/tables/0/views/0/extension",
            "/tables/0/views/0/sorts/0/extension",
            "/tables/0/views/1/filters/0/extension",
        ] {
            assert_eq!(
                saved.pointer(path),
                original.pointer(path),
                "extension {path}"
            );
        }
        assert_eq!(
            document.tables[0].views[2].filters[0].value,
            Some(Value::Null)
        );
        assert_eq!(
            parse_base_document(&format!("\u{feff}{FIXTURE}")).unwrap(),
            document
        );
        assert!(
            parse_base_document(&FIXTURE.replace("\"version\": 1", "\"version\": 1e0")).is_ok()
        );
    }

    #[test]
    fn base_rejects_malformed_unsupported_and_invalid_references() {
        for (path, value) in [
            ("/format", json!("other")),
            ("/version", json!(2)),
            ("/tables", json!([])),
            ("/activeTableId", json!("missing")),
            ("/tables/0/fields", json!([])),
            ("/tables/0/fields/0/type", json!("unknown")),
            ("/tables/0/fields/0/width", json!(0)),
            ("/tables/0/fields/0/width", Value::Null),
            ("/tables/0/fields/2/options/0/color", json!("cyan")),
            ("/tables/0/records/0/values/name", json!(1)),
            ("/tables/0/records/0/values/amount", json!("2")),
            ("/tables/0/records/0/values/group", json!("missing")),
            ("/tables/0/records/0/values/tags", json!("tag_a")),
            ("/tables/0/records/0/values/tags", json!(["tag_a", "tag_a"])),
            ("/tables/0/records/0/values/tags", json!(["missing"])),
            ("/tables/0/records/0/values/day", json!("2025-02-29")),
            ("/tables/0/records/0/values/day", json!("1900-02-29")),
            ("/tables/0/records/0/values/day", json!("0000-01-01")),
            ("/tables/0/records/0/values/day", json!("2026-13-01")),
            ("/tables/0/records/0/values/checked", json!("false")),
            ("/tables/0/records/0/values/progress", json!(101)),
            ("/tables/0/records/0/values/progress", json!(-1)),
            ("/tables/0/views", json!([])),
            ("/tables/0/views/0/type", json!("calendar")),
            ("/tables/0/views/1/groupBy", json!("missing")),
            ("/tables/0/views/1/groupBy", json!("name")),
            ("/tables/0/views/0/sorts/0/fieldId", json!("missing")),
            ("/tables/0/views/0/sorts/0/direction", json!("up")),
            ("/tables/0/views/1/filters/0/fieldId", json!("missing")),
            ("/tables/0/views/1/filters/0/operator", json!("above")),
            ("/tables/0/views/1/filters/0/value", json!({})),
            ("/tables/0/activeViewId", json!("missing")),
        ] {
            let mut raw: Value = serde_json::from_str(FIXTURE).unwrap();
            *raw.pointer_mut(path).unwrap() = value;
            assert!(
                parse_base_document(&raw.to_string()).is_err(),
                "accepted invalid path {path}"
            );
        }
        for path in [
            "/tables",
            "/tables/0/fields",
            "/tables/0/records",
            "/tables/0/views",
            "/tables/0/fields/2/options",
        ] {
            let mut raw: Value = serde_json::from_str(FIXTURE).unwrap();
            let array = raw.pointer_mut(path).unwrap().as_array_mut().unwrap();
            array.push(array[0].clone());
            assert!(
                parse_base_document(&raw.to_string()).is_err(),
                "accepted duplicate in {path}"
            );
        }
        let mut raw: Value = serde_json::from_str(FIXTURE).unwrap();
        raw["tables"][0]["records"][0]["values"]["missing"] = json!("x");
        assert!(parse_base_document(&raw.to_string()).is_err());
        let mut raw: Value = serde_json::from_str(FIXTURE).unwrap();
        raw["tables"][0]["views"][1]["filters"][0]
            .as_object_mut()
            .unwrap()
            .remove("value");
        assert!(parse_base_document(&raw.to_string()).is_err());
        assert!(parse_base_document("").is_err());
        assert!(parse_base_document("{}").is_err());
        assert!(
            parse_base_document(&FIXTURE.replace("\"amount\": 10", "\"amount\": 1e999")).is_err()
        );
    }

    #[test]
    fn base_queries_sort_numerically_with_stable_ties_and_empty_last() {
        let document = fixture();
        let table = &document.tables[0];
        let mut view = table.views[0].clone();
        assert_eq!(
            ids(table, &view, ""),
            ["record_b", "record_c", "record_a", "record_empty"]
        );
        view.sorts[0].direction = SortDirection::Desc;
        assert_eq!(
            ids(table, &view, ""),
            ["record_a", "record_b", "record_c", "record_empty"]
        );
        assert_eq!(
            table
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["record_a", "record_b", "record_c", "record_empty"]
        );
    }

    #[test]
    fn base_queries_sort_unicode_scalars_independently_of_locale() {
        let mut document = fixture();
        let table = &mut document.tables[0];
        table.records[0]
            .values
            .insert("name".into(), json!("\u{10000}"));
        table.records[1]
            .values
            .insert("name".into(), json!("\u{e000}"));
        table.records[2].values.insert("name".into(), json!("A"));
        table.views[0].sorts[0].field_id = "name".into();
        assert_eq!(
            ids(table, &table.views[0], ""),
            ["record_c", "record_b", "record_a", "record_empty"]
        );
    }

    #[test]
    fn base_queries_search_labels_and_format_zero_false_and_tags() {
        let document = fixture();
        let table = &document.tables[0];
        assert_eq!(
            ids(table, &table.views[0], "ALPHA"),
            ["record_c", "record_a"]
        );
        assert_eq!(
            ids(table, &table.views[0], "标签甲"),
            ["record_b", "record_a"]
        );
        assert_eq!(
            format_cell_value(&table.fields[2], &json!("option_a")),
            "分组甲"
        );
        assert_eq!(
            format_cell_value(&table.fields[3], &json!(["tag_b", "tag_a"])),
            "标签乙, 标签甲"
        );
        assert_eq!(format_cell_value(&table.fields[5], &json!(false)), "false");
        assert_eq!(format_cell_value(&table.fields[7], &json!(0)), "0%");
        assert!(!is_cell_empty(&json!(false)));
        assert!(!is_cell_empty(&json!(0)));
        for (value, text) in [
            (1e21, "1e+21"),
            (1.25e22, "1.25e+22"),
            (1e-7, "1e-7"),
            (-1.25e-8, "-1.25e-8"),
            (1e-6, "0.000001"),
            (-0.0, "0"),
        ] {
            assert_eq!(format_cell_value(&table.fields[1], &json!(value)), text);
        }
    }

    #[test]
    fn base_queries_typed_filters_match_typescript() {
        let document = fixture();
        let table = &document.tables[0];
        for (field, operator, value, expected) in [
            (
                "tags",
                FilterOperator::Equals,
                json!("tag_a"),
                vec!["record_a", "record_b"],
            ),
            (
                "tags",
                FilterOperator::Equals,
                json!(["tag_a", "tag_b"]),
                vec!["record_b"],
            ),
            (
                "group",
                FilterOperator::NotEquals,
                json!("option_a"),
                vec!["record_b", "record_c", "record_empty"],
            ),
            (
                "name",
                FilterOperator::Contains,
                json!("ALPHA"),
                vec!["record_a", "record_c"],
            ),
            (
                "group",
                FilterOperator::Contains,
                json!("分组甲"),
                vec!["record_a"],
            ),
            (
                "checked",
                FilterOperator::Equals,
                json!(false),
                vec!["record_a"],
            ),
            (
                "progress",
                FilterOperator::Equals,
                json!(0),
                vec!["record_c"],
            ),
            (
                "day",
                FilterOperator::Equals,
                Value::Null,
                vec!["record_c", "record_empty"],
            ),
            (
                "checked",
                FilterOperator::IsEmpty,
                Value::Null,
                vec!["record_empty"],
            ),
            (
                "tags",
                FilterOperator::IsEmpty,
                Value::Null,
                vec!["record_c", "record_empty"],
            ),
            (
                "progress",
                FilterOperator::IsNotEmpty,
                Value::Null,
                vec!["record_a", "record_b", "record_c"],
            ),
        ] {
            let view = BaseView {
                filters: vec![BaseFilter {
                    field_id: field.into(),
                    operator,
                    value: Some(value),
                    ..Default::default()
                }],
                sorts: vec![],
                ..table.views[0].clone()
            };
            assert_eq!(
                ids(table, &view, ""),
                expected,
                "filter {field} {operator:?}"
            );
        }
        let view = BaseView {
            filters: vec![
                BaseFilter {
                    field_id: "checked".into(),
                    operator: FilterOperator::Equals,
                    value: Some(json!(true)),
                    ..Default::default()
                },
                BaseFilter {
                    field_id: "progress".into(),
                    operator: FilterOperator::IsNotEmpty,
                    ..Default::default()
                },
            ],
            ..table.views[0].clone()
        };
        assert_eq!(ids(table, &view, "alpha"), ["record_c"]);
    }
}
