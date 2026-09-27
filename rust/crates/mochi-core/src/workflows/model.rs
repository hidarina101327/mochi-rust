//! 定义工作流、节点、触发器和运行结果等核心数据结构。
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn object() -> Value {
    json!({})
}
fn yes() -> bool {
    true
}
fn stop() -> String {
    "stop".into()
}
fn timeout() -> u64 {
    60_000
}
fn format() -> String {
    "mochi.workflow".into()
}
fn version() -> u32 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    #[serde(default = "format")]
    pub format: String,
    #[serde(default = "version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default = "object")]
    pub input_schema: Value,
    #[serde(default = "object")]
    pub defaults: Value,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    #[default]
    Manual,
    Interval {
        minutes: u32,
    },
    Daily {
        time: String,
        #[serde(default)]
        utc_offset_minutes: i32,
    },
    Weekly {
        time: String,
        weekdays: Vec<u32>,
        #[serde(default)]
        utc_offset_minutes: i32,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub position: Position,
    #[serde(default = "object")]
    pub inputs: Value,
    #[serde(default = "object")]
    pub config: Value,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "stop")]
    pub on_error: String,
    #[serde(default)]
    pub retries: u8,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_each: Option<String>,
}
impl Node {
    pub fn new(id: &str, kind: &str, x: f32, y: f32) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            label: super::catalog::label(kind).into(),
            position: Position { x, y },
            inputs: super::catalog::default_inputs(kind),
            config: super::catalog::default_config(kind),
            enabled: true,
            on_error: stop(),
            retries: 0,
            timeout_ms: timeout(),
            for_each: None,
        }
    }
    pub fn mutates(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "file_write"
                | "document_append"
                | "schedule_write"
                | "base_write"
                | "script"
                | "notify"
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(
        default,
        rename = "sourceHandle",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_handle: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedWorkflow {
    pub definition: Workflow,
    pub revision: i64,
    /// 兼容旧版 API 的字段。所有已保存的工作流均可信，因此返回 true。
    pub approved: bool,
    pub enabled: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRun {
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub attempts: u32,
    pub input: Value,
    pub output: Value,
    pub error: Option<String>,
}
impl Default for NodeRun {
    fn default() -> Self {
        Self {
            status: "pending".into(),
            started_at: None,
            finished_at: None,
            attempts: 0,
            input: Value::Null,
            output: Value::Null,
            error: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub workflow_id: String,
    pub source: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub input: Value,
    pub output: Value,
    pub error: Option<String>,
    pub definition: Workflow,
    pub nodes: BTreeMap<String, NodeRun>,
}

impl Workflow {
    pub fn blank() -> Self {
        Self {
            format: format(),
            schema_version: 1,
            id: super::new_id("flow"),
            name: "新工作流".into(),
            description: String::new(),
            trigger: Trigger::Manual,
            input_schema: json!({"type":"object"}),
            defaults: object(),
            nodes: vec![
                Node::new("start", "start", 60., 100.),
                Node::new("end", "end", 620., 100.),
            ],
            edges: vec![Edge {
                id: "start_end".into(),
                source: "start".into(),
                target: "end".into(),
                source_handle: None,
            }],
        }
    }
}
