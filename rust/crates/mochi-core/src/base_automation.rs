//! 可搬移、能力有界的视图自动化。没有脚本、没有网络访问、没有级联事件。
//! 规划是纯函数：知情同意、乐观并发、原子落盘都由调用方负责。
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::base::{self, BaseDocument, BaseFilter, BaseRecord, BaseTable, BaseView};

pub const MAX_RECORDS: usize = 200;
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const RULES_KEY: &str = "automations";
const RUNTIME_KEY: &str = "automationRuntime";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub trigger: Trigger,
    #[serde(default)]
    pub conditions: Vec<BaseFilter>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Trigger {
    Manual,
    RecordCreated,
    RecordUpdated {
        #[serde(rename = "fieldIds")]
        field_ids: Vec<String>,
    },
    EnterView,
    Interval {
        minutes: u32,
        #[serde(rename = "startAt")]
        start_at: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    UpdateRecord {
        values: BTreeMap<String, Input>,
    },
    CreateRecord {
        #[serde(rename = "tableId")]
        table_id: String,
        values: BTreeMap<String, Input>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Input {
    Literal {
        value: Value,
    },
    Field {
        #[serde(rename = "fieldId")]
        field_id: String,
    },
    RecordId,
    Now,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub schedule_slots: BTreeMap<String, u64>,
    #[serde(default)]
    pub effect_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    pub rule_id: String,
    pub table_id: String,
    pub view_id: String,
    pub at: u64,
    pub source: String,
    pub record_ids: Vec<String>,
    pub updates: usize,
    pub creates: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    rules: BTreeMap<String, RuleSnapshot>,
}
#[derive(Debug, Clone, Default)]
struct RuleSnapshot {
    signature: String,
    records: BTreeMap<String, String>,
    matching: BTreeSet<String>,
}

pub fn hash(value: &impl Serialize) -> String {
    Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn rules(view: &BaseView) -> Result<Vec<Rule>> {
    let result: Vec<Rule> = match view.extra.get(RULES_KEY) {
        Some(value) => serde_json::from_value(value.clone()).context("自动化格式无效")?,
        None => vec![],
    };
    ensure!(result.len() <= 50, "每个视图最多 50 条自动化");
    let mut ids = BTreeSet::new();
    for rule in &result {
        ensure!(ids.insert(&rule.id), "自动化 ID 重复：{}", rule.id);
    }
    Ok(result)
}

pub fn scope<'a>(
    doc: &'a BaseDocument,
    table_id: &str,
    view_id: &str,
) -> Result<(&'a BaseTable, &'a BaseView)> {
    let table = doc
        .tables
        .iter()
        .find(|t| t.id == table_id)
        .context("数据表不存在")?;
    let view = table
        .views
        .iter()
        .find(|v| v.id == view_id)
        .context("视图不存在")?;
    Ok((table, view))
}

/// 签名包含目标表结构：改掉目标列，本地的授权就随之失效。
pub fn signature(doc: &BaseDocument, table: &BaseTable, view: &BaseView, rule: &Rule) -> String {
    hash(&(
        table.id.as_str(),
        &table.fields,
        view.id.as_str(),
        &view.filters,
        rule,
        rule.actions
            .iter()
            .filter_map(|a| match a {
                Action::CreateRecord { table_id, .. } => doc
                    .tables
                    .iter()
                    .find(|t| &t.id == table_id)
                    .map(|t| (&t.id, &t.fields)),
                _ => None,
            })
            .collect::<Vec<_>>(),
    ))
}

pub fn key(table_id: &str, view_id: &str, rule_id: &str) -> String {
    // 用 JSON 数组拼键，避免用户 ID 里出现歧义分隔符。
    serde_json::to_string(&(table_id, view_id, rule_id)).unwrap()
}

pub fn validate(doc: &BaseDocument, table_id: &str, view_id: &str, rule: &Rule) -> Result<()> {
    let (table, view) = scope(doc, table_id, view_id)?;
    ensure!(
        !rule.id.trim().is_empty() && rule.id.len() <= 128,
        "自动化 ID 长度须为 1–128"
    );
    ensure!(
        !rule.name.trim().is_empty() && rule.name.chars().count() <= 100,
        "请输入不超过 100 字的名称"
    );
    ensure!(
        (1..=16).contains(&rule.actions.len()),
        "自动化须包含 1–16 个动作"
    );
    ensure!(rule.conditions.len() <= 20, "最多 20 个条件");
    for filter in rule.conditions.iter().chain(&view.filters) {
        ensure!(
            table.fields.iter().any(|f| f.id == filter.field_id),
            "条件字段不存在：{}",
            filter.field_id
        );
        ensure!(
            matches!(
                filter.operator,
                base::FilterOperator::IsEmpty | base::FilterOperator::IsNotEmpty
            ) || filter.value.is_some(),
            "条件缺少 value"
        );
    }
    match &rule.trigger {
        Trigger::RecordUpdated { field_ids } => {
            ensure!(!field_ids.is_empty(), "修改触发器需要至少一个监控字段");
            ensure!(
                field_ids
                    .iter()
                    .all(|id| table.fields.iter().any(|f| &f.id == id)),
                "监控字段不存在"
            );
        }
        Trigger::Interval { minutes, start_at } => {
            ensure!((1..=525600).contains(minutes), "间隔须为 1–525600 分钟");
            ensure!(
                *start_at <= 8_640_000_000_000_000,
                "startAt 不是有效的 UTC 毫秒时间"
            );
        }
        _ => {}
    }
    for action in &rule.actions {
        let (target, values) = match action {
            Action::UpdateRecord { values } => (table, values),
            Action::CreateRecord { table_id, values } => (
                doc.tables
                    .iter()
                    .find(|t| &t.id == table_id)
                    .context("动作目标表不存在")?,
                values,
            ),
        };
        ensure!(
            !values.is_empty() && values.len() <= 100,
            "动作须写入 1–100 个字段"
        );
        let mut literal_values = Map::new();
        for (id, input) in values {
            let field = target
                .fields
                .iter()
                .find(|f| &f.id == id)
                .with_context(|| format!("动作字段不存在：{id}"))?;
            match input {
                Input::Literal { value } => {
                    literal_values.insert(id.clone(), value.clone());
                }
                Input::Field { field_id } => {
                    ensure!(
                        table.fields.iter().any(|f| &f.id == field_id),
                        "来源字段不存在：{field_id}"
                    );
                }
                Input::Now => ensure!(
                    matches!(
                        field.field_type,
                        base::FieldType::Text | base::FieldType::DateTime
                    ),
                    "当前时间仅支持文本或日期时间字段"
                ),
                Input::RecordId => ensure!(
                    field.field_type == base::FieldType::Text,
                    "记录 ID 仅支持文本字段"
                ),
            }
        }
        // 选择类 ID、数字/区间类型、引用等校验直接复用 .mcb 的验证器。
        let mut sample = target.clone();
        sample.records = vec![BaseRecord {
            id: "automation_validation".into(),
            values: literal_values,
            ..Default::default()
        }];
        let mut candidate = doc.clone();
        candidate.tables = vec![sample];
        candidate.active_table_id = Some(target.id.clone());
        base::validate_base_document(&candidate)?;
    }
    Ok(())
}

pub fn put(doc: &mut BaseDocument, table_id: &str, view_id: &str, rule: Rule) -> Result<()> {
    validate(doc, table_id, view_id, &rule)?;
    let (_, view) = scope(doc, table_id, view_id)?;
    let mut list = rules(view)?;
    if let Some(index) = list.iter().position(|r| r.id == rule.id) {
        list[index] = rule;
    } else {
        ensure!(list.len() < 50, "每个视图最多 50 条自动化");
        list.push(rule);
    }
    let table = doc.tables.iter_mut().find(|t| t.id == table_id).unwrap();
    table
        .views
        .iter_mut()
        .find(|v| v.id == view_id)
        .unwrap()
        .extra
        .insert(RULES_KEY.into(), serde_json::to_value(list)?);
    Ok(())
}

pub fn remove(doc: &mut BaseDocument, table_id: &str, view_id: &str, rule_id: &str) -> Result<()> {
    let (_, view) = scope(doc, table_id, view_id)?;
    let mut list = rules(view)?;
    let before = list.len();
    list.retain(|r| r.id != rule_id);
    ensure!(list.len() < before, "自动化不存在");
    doc.tables
        .iter_mut()
        .find(|t| t.id == table_id)
        .unwrap()
        .views
        .iter_mut()
        .find(|v| v.id == view_id)
        .unwrap()
        .extra
        .insert(RULES_KEY.into(), serde_json::to_value(list)?);
    Ok(())
}

pub fn runtime(doc: &BaseDocument) -> Runtime {
    doc.extra
        .get(RUNTIME_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}
fn effect_hash(doc: &BaseDocument) -> String {
    hash(
        &doc.tables
            .iter()
            .map(|t| (&t.id, &t.records))
            .collect::<Vec<_>>(),
    )
}
fn save_runtime(doc: &mut BaseDocument, mut state: Runtime) {
    state.runs.drain(..state.runs.len().saturating_sub(100));
    let mut valid_slots = BTreeSet::new();
    for table in &doc.tables {
        for view in &table.views {
            for rule in rules(view).unwrap_or_default() {
                valid_slots.insert(format!(
                    "{}:{}",
                    key(&table.id, &view.id, &rule.id),
                    signature(doc, table, view, &rule)
                ));
            }
        }
    }
    state
        .schedule_slots
        .retain(|key, _| valid_slots.contains(key));
    state.effect_hash = effect_hash(doc);
    doc.extra
        .insert(RUNTIME_KEY.into(), serde_json::to_value(state).unwrap());
}

fn matching(table: &BaseTable, view: &BaseView, rule: &Rule) -> BTreeSet<String> {
    let mut scoped_view = view.clone();
    scoped_view.filters.extend(rule.conditions.clone());
    scoped_view.sorts.clear();
    base::get_view_records(table, &scoped_view, "")
        .into_iter()
        .map(|r| r.id.clone())
        .collect()
}

pub fn snapshot(doc: &BaseDocument, authorized: &BTreeSet<String>) -> Snapshot {
    let mut result = Snapshot::default();
    for table in &doc.tables {
        for view in &table.views {
            for rule in rules(view)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.enabled)
            {
                let sig = signature(doc, table, view, &rule);
                if !authorized.contains(&sig) {
                    continue;
                }
                let records = table
                    .records
                    .iter()
                    .map(|r| {
                        let value = match &rule.trigger {
                            Trigger::RecordUpdated { field_ids } => hash(
                                &field_ids
                                    .iter()
                                    .map(|id| r.values.get(id).unwrap_or(&Value::Null))
                                    .collect::<Vec<_>>(),
                            ),
                            _ => String::new(),
                        };
                        (r.id.clone(), value)
                    })
                    .collect();
                result.rules.insert(
                    key(&table.id, &view.id, &rule.id),
                    RuleSnapshot {
                        signature: sig,
                        records,
                        matching: matching(table, view, &rule),
                    },
                );
            }
        }
    }
    result
}

/// 手动/测试运行必须显式给出记录 ID，且全部落在规则的视图和过滤条件之内。
/// 所有表达式读的都是触发时的原始记录，与动作的执行顺序无关。
pub fn preview(
    doc: &BaseDocument,
    table_id: &str,
    view_id: &str,
    rule: &Rule,
    record_ids: &[String],
    now: u64,
) -> Result<(BaseDocument, Run)> {
    preview_into(doc, doc, table_id, view_id, rule, record_ids, now)
}

fn preview_into(
    doc: &BaseDocument,
    initial: &BaseDocument,
    table_id: &str,
    view_id: &str,
    rule: &Rule,
    record_ids: &[String],
    now: u64,
) -> Result<(BaseDocument, Run)> {
    validate(doc, table_id, view_id, rule)?;
    ensure!(
        !record_ids.is_empty() && record_ids.len() <= MAX_RECORDS,
        "请明确选择 1–200 条记录"
    );
    let (source, view) = scope(doc, table_id, view_id)?;
    let eligible = matching(source, view, rule);
    let unique: BTreeSet<_> = record_ids.iter().collect();
    ensure!(unique.len() == record_ids.len(), "记录 ID 重复");
    ensure!(
        record_ids.iter().all(|id| eligible.contains(id)),
        "记录不存在或不满足视图/自动化条件"
    );
    ensure!(
        record_ids.len() * rule.actions.len() <= 1000,
        "单次最多 1000 个动作；请缩小视图或记录范围"
    );
    let mut result = initial.clone();
    let mut run = Run {
        id: base::create_id("run"),
        rule_id: rule.id.clone(),
        table_id: table_id.into(),
        view_id: view_id.into(),
        at: now,
        source: "manual".into(),
        record_ids: record_ids.to_vec(),
        updates: 0,
        creates: 0,
        error: None,
    };
    for id in record_ids {
        let record = source.records.iter().find(|r| &r.id == id).unwrap();
        for action in &rule.actions {
            let (target_id, inputs) = match action {
                Action::UpdateRecord { values } => (table_id, values),
                Action::CreateRecord { table_id, values } => (table_id.as_str(), values),
            };
            let values: Map<_, _> = inputs
                .iter()
                .map(|(field, input)| {
                    let value = match input {
                        Input::Literal { value } => value.clone(),
                        Input::Field { field_id } => {
                            record.values.get(field_id).cloned().unwrap_or(Value::Null)
                        }
                        Input::RecordId => json!(id),
                        Input::Now => json!(chrono::DateTime::from_timestamp_millis(now as i64)
                            .map(|d| d
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%dT%H:%M")
                                .to_string())
                            .unwrap_or_default()),
                    };
                    (field.clone(), value)
                })
                .collect();
            let target = result
                .tables
                .iter_mut()
                .find(|t| t.id == target_id)
                .unwrap();
            match action {
                Action::UpdateRecord { .. } => {
                    let target_record = target.records.iter_mut().find(|r| &r.id == id).unwrap();
                    if values
                        .iter()
                        .any(|(k, v)| target_record.values.get(k).unwrap_or(&Value::Null) != v)
                    {
                        target_record.values.extend(values);
                        run.updates += 1;
                    }
                }
                Action::CreateRecord { .. } => {
                    target.records.push(BaseRecord {
                        id: base::create_id("record"),
                        values,
                        ..Default::default()
                    });
                    run.creates += 1;
                }
            }
        }
    }
    // 先完整校验，再返回任何变更：后面某个动作不合法就整体回滚。
    let raw = base::serialize_base_document(&result)?;
    ensure!(
        raw.len() as u64 <= MAX_FILE_BYTES,
        "执行后文件将超过 10 MiB，请缩小范围"
    );
    Ok((result, run))
}

pub fn manual_run(
    doc: &BaseDocument,
    table_id: &str,
    view_id: &str,
    rule: &Rule,
    record_ids: &[String],
    now: u64,
) -> Result<(BaseDocument, Run)> {
    let (result, run) = preview(doc, table_id, view_id, rule, record_ids, now)?;
    let result = finish_preview(result, &run)?;
    Ok((result, run))
}

/// 只落盘 UI 预览里展示的那些值，不重新生成 ID/时间。
/// 宿主仍须比对原始文件并复查脏缓冲。
pub fn finish_preview(mut planned: BaseDocument, run: &Run) -> Result<BaseDocument> {
    let mut state = runtime(&planned);
    state.runs.push(run.clone());
    save_runtime(&mut planned, state);
    ensure!(
        base::serialize_base_document(&planned)?.len() as u64 <= MAX_FILE_BYTES,
        "执行后文件超过 10 MiB"
    );
    Ok(planned)
}

/// 已保存状态之间的流转只评估一次。不补算历史、不级联、不因日程积压而刷屏。
pub fn evaluate(
    doc: &BaseDocument,
    previous: &Snapshot,
    authorized: &BTreeSet<String>,
    now: u64,
) -> Result<(BaseDocument, Snapshot)> {
    let current = snapshot(doc, authorized);
    let mut result = doc.clone();
    let mut state = runtime(doc);
    let own_effect = !state.effect_hash.is_empty() && state.effect_hash == effect_hash(doc);
    let mut ran = false;
    for table in &doc.tables {
        for view in &table.views {
            for rule in rules(view)? {
                let rule_key = key(&table.id, &view.id, &rule.id);
                let Some(next) = current.rules.get(&rule_key) else {
                    continue;
                };
                let before = previous
                    .rules
                    .get(&rule_key)
                    .filter(|old| old.signature == next.signature);
                let mut slot = None;
                let ids: Vec<String> = match &rule.trigger {
                    Trigger::Manual => vec![],
                    Trigger::Interval { minutes, start_at } if *minutes > 0 && now >= *start_at => {
                        let value = (now - start_at) / (u64::from(*minutes) * 60_000);
                        let schedule_key = format!("{}:{}", rule_key, next.signature);
                        if state
                            .schedule_slots
                            .get(&schedule_key)
                            .is_some_and(|last| *last >= value)
                        {
                            vec![]
                        } else {
                            slot = Some((schedule_key, value));
                            next.matching.iter().cloned().collect()
                        }
                    }
                    _ if own_effect || before.is_none() => vec![],
                    Trigger::RecordCreated => next
                        .matching
                        .iter()
                        .filter(|id| !before.unwrap().records.contains_key(*id))
                        .cloned()
                        .collect(),
                    Trigger::RecordUpdated { .. } => next
                        .matching
                        .iter()
                        .filter(|id| {
                            before
                                .unwrap()
                                .records
                                .get(*id)
                                .is_some_and(|old| Some(old) != next.records.get(*id))
                        })
                        .cloned()
                        .collect(),
                    Trigger::EnterView => next
                        .matching
                        .difference(&before.unwrap().matching)
                        .cloned()
                        .collect(),
                    Trigger::Interval { .. } => vec![],
                };
                if let Some((k, v)) = slot {
                    state.schedule_slots.insert(k, v);
                    ran = true;
                }
                if ids.is_empty() {
                    continue;
                }
                // 事件选择只依据 doc，有序的规则写入则逐条累积。
                // 就算前一条规则改过原始记录，动作表达式读的仍是原始值。
                let attempt = preview_into(doc, &result, &table.id, &view.id, &rule, &ids, now);
                match attempt {
                    Ok((planned, mut run)) => {
                        result = planned;
                        run.source = "automatic".into();
                        state.runs.push(run);
                    }
                    Err(error) => {
                        let mut paused = rule.clone();
                        paused.enabled = false;
                        // 就算规则本身不符合当前结构，也必须能暂停。
                        let v = result
                            .tables
                            .iter_mut()
                            .find(|t| t.id == table.id)
                            .unwrap()
                            .views
                            .iter_mut()
                            .find(|v| v.id == view.id)
                            .unwrap();
                        let mut list = rules(v)?;
                        if let Some(r) = list.iter_mut().find(|r| r.id == rule.id) {
                            *r = paused;
                        }
                        v.extra
                            .insert(RULES_KEY.into(), serde_json::to_value(list)?);
                        state.runs.push(Run {
                            id: base::create_id("run"),
                            rule_id: rule.id.clone(),
                            table_id: table.id.clone(),
                            view_id: view.id.clone(),
                            at: now,
                            source: "automatic".into(),
                            record_ids: ids.into_iter().take(MAX_RECORDS).collect(),
                            updates: 0,
                            creates: 0,
                            error: Some(error.to_string()),
                        });
                    }
                }
                ran = true;
            }
        }
    }
    if ran {
        // 整个 tick 也是原子的：写入大小受限也不能留下只应用了一半的规则。
        save_runtime(&mut result, state);
        ensure!(
            base::serialize_base_document(&result)?.len() as u64 <= MAX_FILE_BYTES,
            "自动化结果超过 10 MiB，未写入"
        );
    }
    let final_snapshot = snapshot(&result, authorized);
    Ok((result, final_snapshot))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 从重读、比对到写入，调用方都必须持有它，而不是只在序列化时用。
pub static MUTATION_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;

/// 原生自动化写入方的协作式跨进程锁，从 CAS 到替换全程持有。
/// 留下一个空的锁文件没有危害；即使崩溃后 OS 也会释放所有权。
pub struct FileLock {
    _file: std::fs::File,
}
impl FileLock {
    pub fn acquire(path: &std::path::Path) -> Result<Self> {
        let name = path.file_name().context("无效文件路径")?.to_string_lossy();
        let lock = path.with_file_name(format!(".{name}.automation-lock"));
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options
            .open(lock)
            .context("另一实例正在执行自动化，请稍后重试")?;
        #[cfg(unix)]
        {
            // flock 是建议锁，关闭/崩溃时自动释放。别用 File::try_lock，
            // 它比工作区要求的 Rust 1.88 最低版本还新。
            use std::os::fd::AsRawFd;
            unsafe extern "C" {
                fn flock(fd: i32, operation: i32) -> i32;
            }
            if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("另一实例正在执行自动化，请稍后重试");
            }
        }
        #[cfg(not(any(windows, unix)))]
        anyhow::bail!("当前平台不支持自动化文件锁");
        Ok(Self { _file: file })
    }
}
