//! 请求前检查上下文预算：先压缩旧工具结果，仍超限则停止。
//! agent_plan 照常派发；实时更新走 ToolHost，执行轨迹保存结果快照。

use std::time::Instant;

use serde_json::{json, Map, Value};

use super::models::{
    estimate_tokens, AiCompletionRequest, AiCompletionResponse, AiError, AiMessage, AiStreamChunk,
    AiToolCall, AiToolDefinition,
};
use super::service::AiService;

/// 内置系统提示词保存在 assets/agent-system-prompt.txt——
/// 里面的 LaTeX 转义示例（`\frac` 是正面示例、`\\frac` 是反面示例）手抄必错。
pub const SYSTEM_PROMPT: &str = include_str!("../../assets/agent-system-prompt.txt");

/// 上游默认的上下文窗口。取值来自 `ai-agent.ts` 的 `DEFAULT_CONTEXT_WINDOW_TOKENS`。
pub const DEFAULT_CONTEXT_WINDOW_TOKENS: i64 = 1_048_565;

/// 默认最大工具轮次。
pub const DEFAULT_MAX_TOOL_ITERATIONS: usize = 120;

/// 没给 `max_tokens` 时，预算计算里假定的输出预留量。
const ASSUMED_MAX_TOKENS: i64 = 2_000;

/// 预算下限：再怎么小的上下文窗口也不能把预算压到没法干活。
const MIN_CONTEXT_BUDGET: i64 = 32_000;

/// 短于这个长度的工具结果不值得压缩（UTF-16 长度，对齐 TS 的 `.length`）。
const COMPACT_THRESHOLD: usize = 1024;

/// 压缩时保留的「引用型」字段：模型靠它们指回完整数据，而不必把数据本身留在上下文里。
const COMPACT_KEEP_KEYS: &[&str] = &[
    "scanId",
    "draftId",
    "previewId",
    "revision",
    "path",
    "mode",
    "candidateCount",
    "itemCount",
    "returned",
    "total",
    "hasMore",
    "nextOffset",
    "summary",
    "message",
];

/// 做一句话参数摘要时优先看的字段，顺序即优先级。
const SUMMARY_KEYS: &[&str] = &[
    "path",
    "filepath",
    "parentPath",
    "nameOrPath",
    "query",
    "name",
    "title",
    "url",
    "command",
    "topic",
    "category",
    "view",
    "message",
    "format",
    "oid",
];

const AGENT_PLAN_TOOL: &str = "agent_plan";

/// 执行工具的能力。`tools::ToolRegistry` 实现了它；测试用假实现。
///
/// 返回值是**要原样回给模型的 JSON 字符串**（`{"ok":true,"data":…}` 形状）。
pub trait ToolRunner {
    fn execute(&self, call: &AiToolCall) -> String;
    fn disclosed_tools(&self) -> Option<Vec<AiToolDefinition>> {
        None
    }
}

impl ToolRunner for super::tools::ToolRegistry {
    fn execute(&self, call: &AiToolCall) -> String {
        super::tools::ToolRegistry::execute(self, call)
    }
    fn disclosed_tools(&self) -> Option<Vec<AiToolDefinition>> {
        super::tools::ToolRegistry::disclosed_tools(self)
    }
}

/// 调用模型的能力。`AiService` 实现了它。
///
/// 抽成 trait 是为了**让整个循环能脱网测试**——工具往返、计划去重、上下文压缩、
/// 回合上限等逻辑正是最值得测试、又最不应依赖真实服务方的部分。
/// 与 `SseAccumulator` 同样的思路：把可测的那一半从 I/O 里摘出来。
pub trait ModelClient {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn complete(
        &self,
        request: &AiCompletionRequest,
        on_chunk: &mut dyn FnMut(AiStreamChunk),
    ) -> Result<AiCompletionResponse, AiError>;
}

impl ModelClient for AiService {
    fn is_cancelled(&self) -> bool {
        AiService::is_cancelled(self)
    }
    fn complete(
        &self,
        request: &AiCompletionRequest,
        on_chunk: &mut dyn FnMut(AiStreamChunk),
    ) -> Result<AiCompletionResponse, AiError> {
        AiService::complete(self, request, Some(on_chunk))
    }
}

/// 一步执行轨迹。会话保存后供用户回看「它当时都干了什么」。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentTraceStep {
    /// Provider 明确返回的推理链（reasoning/reasoning_content）。中间轮的
    /// 普通 content 不是思维链，不能被标成 Thinking。
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    Tool {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        ok: bool,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        result_preview: Option<String>,
    },
    /// 计划快照。轨迹里**只保留最新一份**——否则一个改了五次计划的任务，
    /// 轨迹里会有五份几乎一样的清单，把真正的工具步骤淹掉。
    Plan {
        steps: Vec<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

/// 运行过程中的事件。UI 据此实时渲染。
#[derive(Debug)]
pub enum AgentEvent<'a> {
    ModelStart {
        iteration: usize,
    },
    ReasoningDelta {
        delta: &'a str,
    },
    ContentDelta {
        delta: &'a str,
    },
    ToolStart {
        name: &'a str,
        summary: Option<&'a str>,
    },
    ToolResult {
        name: &'a str,
        ok: bool,
        duration_ms: u64,
        summary: Option<&'a str>,
    },
    TraceStep {
        step: &'a AgentTraceStep,
    },
    Final {
        content: &'a str,
    },
    /// 软失败（超预算、轮次用尽）。硬失败走 `Err(AiError)`，不经过这里。
    Error {
        error: &'a str,
    },
}

pub struct AgentRequest {
    pub messages: Vec<AiMessage>,
    /// 覆盖内置系统提示词。空白视为未提供。
    pub system_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i32>,
    pub max_tool_iterations: usize,
    pub tools: Vec<AiToolDefinition>,
    pub context_window_tokens: i64,
}

impl Default for AgentRequest {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            tools: Vec::new(),
            context_window_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentResult {
    pub response: AiCompletionResponse,
    pub trace: Vec<AgentTraceStep>,
    /// 循环结束时的完整对话（含系统提示词、助手的工具调用、工具回执）。
    /// 调用方据此持久化会话——工具往返也要存，否则续聊时模型看不到自己干过什么。
    pub transcript: Vec<AiMessage>,
}

/// UTF-16 长度（JS 的 `String.length`）。
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// 截断到 `max` 个字符并加省略号。
///
/// **与 TS 有个小差异**：那边按 UTF-16 码元切，这里按 `char` 切。差异只在星文平面
/// 字符（emoji）上体现，而这些字符串全是给人看的摘要，不落盘、不参与任何契约。
/// 按 `char` 切换来的是「永远不会把代理对劈成两半」——按码元切则要额外处理这个。
fn truncate_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…")
}

/// 单条消息的 Token 数估算。
///
/// `+8` 与 `tool_calls` 始终序列化为数组（即使没有调用也会得到 `"[]"`，估作 1 个 Token）
/// 都照抄 TS——预算判定的临界点会因此改变，估算口径必须一致。
fn message_tokens(message: &AiMessage) -> i64 {
    let content = estimate_tokens(message.content.as_deref().unwrap_or("")) as i64;
    let calls = message.tool_calls.clone().unwrap_or_default();
    let calls_json = serde_json::to_string(&calls).unwrap_or_else(|_| "[]".into());
    content + estimate_tokens(&calls_json) as i64 + 8
}

fn transcript_tokens(messages: &[AiMessage], tools: &[AiToolDefinition]) -> i64 {
    let tools_json = serde_json::to_string(tools).unwrap_or_else(|_| "[]".into());
    messages.iter().map(message_tokens).sum::<i64>() + estimate_tokens(&tools_json) as i64
}

/// 把一条臃肿的工具结果压成引用摘要。
///
/// 保留 `ok`、工具名、若干引用型字段（`scanId`/`path`/计数…），
/// 并把每个数组换成 `<键>Count`。模型据此知道「那份数据还在，要用就再查一次」。
fn compact_tool_result(message: &AiMessage) -> String {
    let content = message.content.as_deref().unwrap_or("");
    let tool = message.name.clone().unwrap_or_default();

    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        // 不是 JSON 就只能报个长度。对齐 TS 的 catch 分支。
        return json!({
            "compacted": true,
            "tool": tool,
            "originalCharacters": utf16_len(content),
        })
        .to_string();
    };

    // `data` 是对象/数组时下钻，否则就地用整个结果（对齐 TS 的 `typeof === 'object'`，
    // 注意那边 `null` 因为前置的真值判断而落到 else 分支）。
    let data = match parsed.get("data") {
        Some(v) if v.is_object() || v.is_array() => v,
        _ => &parsed,
    };

    let mut summary = Map::new();
    summary.insert("compacted".into(), json!(true));
    if let Some(ok) = parsed.get("ok") {
        summary.insert("ok".into(), ok.clone());
    }
    summary.insert("tool".into(), json!(tool));

    if let Some(object) = data.as_object() {
        for key in COMPACT_KEEP_KEYS {
            if let Some(value) = object.get(*key) {
                summary.insert((*key).to_string(), value.clone());
            }
        }
        for (key, value) in object {
            if let Some(items) = value.as_array() {
                summary.insert(format!("{key}Count"), json!(items.len()));
            }
        }
    }

    Value::Object(summary).to_string()
}

/// 超出预算时就地压缩旧的工具结果，并返回压缩后的 Token 数估值。
///
/// **最新一条工具结果不压**——模型正要基于它下判断，压掉等于让它瞎猜。
/// 压完仍超预算时返回超预算的值，由调用方中止。
fn compact_transcript(messages: &mut [AiMessage], tools: &[AiToolDefinition], budget: i64) -> i64 {
    let mut tokens = transcript_tokens(messages, tools);
    if tokens <= budget {
        return tokens;
    }

    let mut tool_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "tool")
        .map(|(i, _)| i)
        .collect();
    tool_indices.pop(); // 留下最新的那条

    for index in tool_indices {
        if utf16_len(messages[index].content.as_deref().unwrap_or("")) < COMPACT_THRESHOLD {
            continue;
        }
        messages[index].content = Some(compact_tool_result(&messages[index]));
        tokens = transcript_tokens(messages, tools);
        if tokens <= budget {
            return tokens;
        }
    }
    tokens
}

fn context_budget(request: &AgentRequest) -> i64 {
    let window = (request.context_window_tokens as f64 * 0.8).floor() as i64;
    let reserved = request
        .max_tokens
        .map(i64::from)
        .unwrap_or(ASSUMED_MAX_TOKENS);
    (window - reserved).max(MIN_CONTEXT_BUDGET)
}

/// 从工具参数里挑一个最有代表性的字段做一句话摘要，供轨迹时间线展示。
fn summarize_tool_args(args: &Value) -> Option<String> {
    let object = args.as_object()?;
    for key in SUMMARY_KEYS {
        if let Some(text) = object.get(*key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(truncate_text(text.trim(), 80));
            }
        }
    }
    let (key, value) = object.iter().next()?;
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => {
            let rendered = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            Some(truncate_text(&format!("{key}={rendered}"), 80))
        }
        Value::Array(items) => Some(format!("{key} ×{}", items.len())),
        _ => None,
    }
}

/// 结果短预览：成功取 data 概要，失败取错误信息。
fn preview_tool_result(result: &Value) -> Option<String> {
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        return Some(match result.get("error").and_then(Value::as_str) {
            Some(error) => truncate_text(error, 200),
            None => "失败".to_owned(),
        });
    }
    match result.get("data") {
        None | Some(Value::Null) => None,
        Some(data) => Some(truncate_text(&data.to_string(), 160)),
    }
}

/// 跑一轮完整的 Agent 任务。
///
/// 硬失败（网络、鉴权、取消）返回 `Err`。软失败（上下文超预算、工具轮次用尽）
/// 返回 `Ok`，`finish_reason` 为 `"error"`，`content` 是给用户看的说明——
/// 对齐 TS，也符合实际：这两种情况下前面若干轮的工作和轨迹都是有效的，
/// 整个丢掉太浪费。
pub fn run(
    ai: &dyn ModelClient,
    tools: &dyn ToolRunner,
    request: &AgentRequest,
    mut on_event: impl FnMut(AgentEvent),
) -> Result<AgentResult, AiError> {
    let system = request
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(SYSTEM_PROMPT);

    let mut transcript = Vec::with_capacity(request.messages.len() + 1);
    transcript.push(AiMessage::new("system", system));
    transcript.extend(request.messages.iter().cloned());

    let budget = context_budget(request);
    let mut trace: Vec<AgentTraceStep> = Vec::new();

    for iteration in 0..request.max_tool_iterations {
        if ai.is_cancelled() {
            return Err(AiError::new("Request cancelled", "CANCELLED"));
        }
        let active_tools = tools
            .disclosed_tools()
            .unwrap_or_else(|| request.tools.clone());
        let estimated = compact_transcript(&mut transcript, &active_tools, budget);
        if estimated > budget {
            let error = format!(
                "当前 Agent 上下文预计 {estimated} tokens，超过安全预算 {budget} tokens。\
                 已停止继续请求，以避免消耗余额；请让工具保存完整数据并仅返回摘要或引用 ID。"
            );
            on_event(AgentEvent::Error { error: &error });
            return Ok(AgentResult {
                response: AiCompletionResponse {
                    content: error,
                    finish_reason: "error".into(),
                    ..Default::default()
                },
                trace,
                transcript,
            });
        }

        on_event(AgentEvent::ModelStart { iteration });
        let model_started = Instant::now();

        let completion = AiCompletionRequest {
            messages: transcript.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: None,
            tools: active_tools.clone(),
            tool_choice: Some("auto".into()),
        };

        let response = {
            let mut forward = |chunk: AiStreamChunk| {
                if let Some(delta) = chunk.reasoning_delta.as_deref() {
                    on_event(AgentEvent::ReasoningDelta { delta });
                }
                if let Some(delta) = chunk.delta.as_deref() {
                    on_event(AgentEvent::ContentDelta { delta });
                }
            };
            ai.complete(&completion, &mut forward)?
        };
        if ai.is_cancelled() {
            return Err(AiError::new("Request cancelled", "CANCELLED"));
        }

        // 推理模型的思维链进轨迹（流式时 UI 已实时看过，这里持久化供回看）
        if let Some(reasoning) = response.reasoning.as_deref().map(str::trim) {
            if !reasoning.is_empty() {
                push_trace(
                    &mut trace,
                    AgentTraceStep::Thinking {
                        text: reasoning.to_owned(),
                        duration_ms: Some(model_started.elapsed().as_millis() as u64),
                    },
                    &mut on_event,
                );
            }
        }

        if response.tool_calls.is_empty() {
            on_event(AgentEvent::Final {
                content: &response.content,
            });
            return Ok(AgentResult {
                response,
                trace,
                transcript,
            });
        }

        transcript.push(AiMessage {
            role: "assistant".into(),
            content: Some(response.content.clone()),
            tool_calls: Some(response.tool_calls.clone()),
            hidden: true,
            ..Default::default()
        });

        for call in &response.tool_calls {
            if ai.is_cancelled() {
                return Err(AiError::new("Request cancelled", "CANCELLED"));
            }
            let name = call.function.name.clone();
            let parsed_args: Value =
                serde_json::from_str(call.function.arguments.trim()).unwrap_or_else(|_| json!({}));
            let summary = summarize_tool_args(&parsed_args);

            on_event(AgentEvent::ToolStart {
                name: &name,
                summary: summary.as_deref(),
            });
            if ai.is_cancelled() {
                return Err(AiError::new("Request cancelled", "CANCELLED"));
            }

            let started = Instant::now();
            let raw = tools.execute(call);
            let duration_ms = started.elapsed().as_millis() as u64;

            let result: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({"ok": false}));
            let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);

            on_event(AgentEvent::ToolResult {
                name: &name,
                ok,
                duration_ms,
                summary: summary.as_deref(),
            });

            if name == AGENT_PLAN_TOOL && ok {
                if let Some(step) = plan_trace_step(&result) {
                    // 只留最新一份计划，否则改过五次计划的任务会把轨迹刷满
                    trace.retain(|s| !matches!(s, AgentTraceStep::Plan { .. }));
                    push_trace(&mut trace, step, &mut on_event);
                }
            }

            push_trace(
                &mut trace,
                AgentTraceStep::Tool {
                    name: name.clone(),
                    summary: summary.clone(),
                    ok,
                    duration_ms,
                    result_preview: preview_tool_result(&result),
                },
                &mut on_event,
            );

            transcript.push(AiMessage {
                role: "tool".into(),
                content: Some(raw),
                tool_call_id: Some(call.id.clone()),
                name: Some(name),
                hidden: true,
                ..Default::default()
            });
            compact_transcript(&mut transcript, &active_tools, budget);
        }
    }

    let error = "工具调用轮次过多，已停止。请缩小请求范围后重试。".to_owned();
    on_event(AgentEvent::Error { error: &error });
    Ok(AgentResult {
        response: AiCompletionResponse {
            content: error,
            finish_reason: "error".into(),
            ..Default::default()
        },
        trace,
        transcript,
    })
}

fn push_trace(
    trace: &mut Vec<AgentTraceStep>,
    step: AgentTraceStep,
    on_event: &mut impl FnMut(AgentEvent),
) {
    trace.push(step);
    on_event(AgentEvent::TraceStep {
        step: trace.last().unwrap(),
    });
}

/// 从 `agent_plan` 的成功结果里取出计划快照。
fn plan_trace_step(result: &Value) -> Option<AgentTraceStep> {
    let data = result.get("data")?;
    let steps = data.get("steps")?.as_array()?;
    if steps.is_empty() {
        return None;
    }
    Some(AgentTraceStep::Plan {
        steps: steps.clone(),
        note: data.get("note").and_then(Value::as_str).map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::AiToolFunction;
    use std::cell::RefCell;

    /// 按脚本逐轮吐出预设响应的假模型，并记录每轮实际收到的 transcript。
    struct ScriptedModel {
        script: RefCell<Vec<AiCompletionResponse>>,
        seen: RefCell<Vec<Vec<AiMessage>>>,
        /// 每轮要推给回调的流式分片
        chunks: Vec<AiStreamChunk>,
    }

    impl ScriptedModel {
        fn new(script: Vec<AiCompletionResponse>) -> Self {
            Self {
                script: RefCell::new(script),
                seen: RefCell::new(Vec::new()),
                chunks: Vec::new(),
            }
        }
        fn with_chunks(mut self, chunks: Vec<AiStreamChunk>) -> Self {
            self.chunks = chunks;
            self
        }
        fn rounds(&self) -> usize {
            self.seen.borrow().len()
        }
    }

    impl ModelClient for ScriptedModel {
        fn complete(
            &self,
            request: &AiCompletionRequest,
            on_chunk: &mut dyn FnMut(AiStreamChunk),
        ) -> Result<AiCompletionResponse, AiError> {
            self.seen.borrow_mut().push(request.messages.clone());
            for chunk in &self.chunks {
                on_chunk(chunk.clone());
            }
            let mut script = self.script.borrow_mut();
            if script.is_empty() {
                // 脚本用尽还在请求 = 循环没按预期停下来，让它变成显式失败
                return Err(AiError::new("脚本用尽", "API_ERROR"));
            }
            Ok(script.remove(0))
        }
    }

    /// 回声工具：把收到的调用记下来，按预设返回。
    struct ScriptedTools {
        replies: RefCell<Vec<String>>,
        calls: RefCell<Vec<AiToolCall>>,
    }

    impl ScriptedTools {
        fn new(replies: Vec<&str>) -> Self {
            Self {
                replies: RefCell::new(replies.into_iter().map(str::to_owned).collect()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl ToolRunner for ScriptedTools {
        fn execute(&self, call: &AiToolCall) -> String {
            self.calls.borrow_mut().push(call.clone());
            let mut replies = self.replies.borrow_mut();
            if replies.is_empty() {
                r#"{"ok":true,"data":{}}"#.to_owned()
            } else {
                replies.remove(0)
            }
        }
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> AiToolCall {
        AiToolCall {
            id: id.into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    #[test]
    fn cancelling_on_tool_start_never_executes_the_tool() {
        struct CancelModel {
            cancelled: std::cell::Cell<bool>,
        }
        impl ModelClient for CancelModel {
            fn is_cancelled(&self) -> bool {
                self.cancelled.get()
            }
            fn complete(
                &self,
                _: &AiCompletionRequest,
                _: &mut dyn FnMut(AiStreamChunk),
            ) -> Result<AiCompletionResponse, AiError> {
                Ok(AiCompletionResponse {
                    tool_calls: vec![
                        tool_call("1", "file_write", "{}"),
                        tool_call("2", "file_delete", "{}"),
                    ],
                    ..Default::default()
                })
            }
        }
        let model = CancelModel {
            cancelled: std::cell::Cell::new(false),
        };
        let tools = ScriptedTools::new(vec![]);
        let error = run(&model, &tools, &AgentRequest::default(), |event| {
            if matches!(event, AgentEvent::ToolStart { .. }) {
                model.cancelled.set(true)
            }
        })
        .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        assert!(tools.calls.borrow().is_empty());
    }

    fn says(content: &str) -> AiCompletionResponse {
        AiCompletionResponse {
            content: content.into(),
            ..Default::default()
        }
    }

    fn calls(content: &str, tool_calls: Vec<AiToolCall>) -> AiCompletionResponse {
        AiCompletionResponse {
            content: content.into(),
            finish_reason: "tool_calls".into(),
            tool_calls,
            ..Default::default()
        }
    }

    fn ask(text: &str) -> AgentRequest {
        AgentRequest {
            messages: vec![AiMessage::new("user", text)],
            ..Default::default()
        }
    }

    fn silent(_: AgentEvent) {}

    // ---------- 基本循环 ----------

    #[test]
    fn a_reply_without_tool_calls_ends_the_loop_immediately() {
        let model = ScriptedModel::new(vec![says("你好")]);
        let tools = ScriptedTools::new(vec![]);
        let out = run(&model, &tools, &ask("在吗"), silent).unwrap();

        assert_eq!(out.response.content, "你好");
        assert_eq!(model.rounds(), 1);
        assert!(tools.calls.borrow().is_empty());
        assert!(out.trace.is_empty(), "没工具没思维链就不该有轨迹");
    }

    /// 内置提示词默认注入，且排在最前。
    #[test]
    fn the_builtin_system_prompt_leads_the_transcript() {
        let model = ScriptedModel::new(vec![says("ok")]);
        run(&model, &ScriptedTools::new(vec![]), &ask("在吗"), silent).unwrap();

        let seen = &model.seen.borrow()[0];
        assert_eq!(seen[0].role, "system");
        assert_eq!(seen[0].content.as_deref(), Some(SYSTEM_PROMPT));
        assert_eq!(seen[1].role, "user");
    }

    #[test]
    fn a_custom_system_prompt_replaces_the_builtin_one() {
        let model = ScriptedModel::new(vec![says("ok")]);
        let request = AgentRequest {
            system_prompt: Some("  你是测试助手  ".into()),
            ..ask("在吗")
        };
        run(&model, &ScriptedTools::new(vec![]), &request, silent).unwrap();
        assert_eq!(
            model.seen.borrow()[0][0].content.as_deref(),
            Some("你是测试助手"),
            "自定义提示词应被 trim"
        );
    }

    /// 空白的自定义提示词退回内置——否则等于把 Agent 的行为约束整个抹掉。
    #[test]
    fn a_blank_custom_prompt_falls_back_to_the_builtin() {
        let model = ScriptedModel::new(vec![says("ok")]);
        let request = AgentRequest {
            system_prompt: Some("   ".into()),
            ..ask("在吗")
        };
        run(&model, &ScriptedTools::new(vec![]), &request, silent).unwrap();
        assert_eq!(
            model.seen.borrow()[0][0].content.as_deref(),
            Some(SYSTEM_PROMPT)
        );
    }

    // ---------- 工具往返 ----------

    #[test]
    fn a_tool_call_round_trips_into_the_next_request() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", "file_read", r#"{"path":"a.md"}"#)]),
            says("读完了"),
        ]);
        let tools = ScriptedTools::new(vec![r#"{"ok":true,"data":{"content":"正文"}}"#]);
        let out = run(&model, &tools, &ask("读 a.md"), silent).unwrap();

        assert_eq!(out.response.content, "读完了");
        assert_eq!(model.rounds(), 2);
        assert_eq!(tools.calls.borrow()[0].function.name, "file_read");

        // 第二轮请求里必须能看到助手的工具调用和工具回执，否则模型不知道自己干过什么
        let second = &model.seen.borrow()[1];
        let assistant = second.iter().find(|m| m.role == "assistant").unwrap();
        assert_eq!(assistant.tool_calls.as_ref().unwrap()[0].id, "c1");
        let tool_msg = second.iter().find(|m| m.role == "tool").unwrap();
        assert_eq!(tool_msg.tool_call_id.as_deref(), Some("c1"));
        assert_eq!(tool_msg.name.as_deref(), Some("file_read"));
        assert!(tool_msg.content.as_deref().unwrap().contains("正文"));
    }

    /// 一轮里的多个工具调用要按序全部执行。
    #[test]
    fn every_tool_call_in_a_round_is_executed() {
        let model = ScriptedModel::new(vec![
            calls(
                "",
                vec![
                    tool_call("c1", "file_read", r#"{"path":"a.md"}"#),
                    tool_call("c2", "file_read", r#"{"path":"b.md"}"#),
                ],
            ),
            says("都读完了"),
        ]);
        let tools = ScriptedTools::new(vec![
            r#"{"ok":true,"data":{"n":1}}"#,
            r#"{"ok":true,"data":{"n":2}}"#,
        ]);
        run(&model, &tools, &ask("读两篇"), silent).unwrap();

        assert_eq!(tools.calls.borrow().len(), 2);
        let second = &model.seen.borrow()[1];
        assert_eq!(second.iter().filter(|m| m.role == "tool").count(), 2);
    }

    /// 工具失败不中断循环——模型要看到错误才能自己换个方式重试。
    #[test]
    fn a_failing_tool_is_reported_but_does_not_abort_the_run() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", "file_read", "{}")]),
            says("那我换个办法"),
        ]);
        let tools = ScriptedTools::new(vec![r#"{"ok":false,"error":"文件不存在"}"#]);
        let out = run(&model, &tools, &ask("读"), silent).unwrap();

        assert_eq!(out.response.content, "那我换个办法");
        let AgentTraceStep::Tool {
            ok, result_preview, ..
        } = &out.trace[0]
        else {
            panic!("{:?}", out.trace)
        };
        assert!(!ok);
        assert_eq!(
            result_preview.as_deref(),
            Some("文件不存在"),
            "失败要取 error"
        );
    }

    /// 工具返回的不是合法 JSON 时按失败处理，且不能 panic。
    #[test]
    fn a_non_json_tool_reply_is_treated_as_failure() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", "file_read", "{}")]),
            says("好"),
        ]);
        let tools = ScriptedTools::new(vec!["这不是 JSON"]);
        let out = run(&model, &tools, &ask("读"), silent).unwrap();
        let AgentTraceStep::Tool { ok, .. } = &out.trace[0] else {
            panic!()
        };
        assert!(!ok);
        // 原始文本仍要原样回给模型，别替它加工
        let tool_msg = model.seen.borrow()[1]
            .iter()
            .find(|m| m.role == "tool")
            .cloned()
            .unwrap();
        assert_eq!(tool_msg.content.as_deref(), Some("这不是 JSON"));
    }

    // ---------- 轨迹 ----------

    #[test]
    fn intermediate_prose_is_not_misreported_as_a_thinking_step() {
        let model = ScriptedModel::new(vec![
            calls("我先看看文件", vec![tool_call("c1", "file_read", "{}")]),
            says("最终答案"),
        ]);
        let out = run(&model, &ScriptedTools::new(vec![]), &ask("干活"), silent).unwrap();

        assert_eq!(
            out.response.content, "最终答案",
            "中间轮正文不该混进最终回答"
        );
        assert!(
            out.trace
                .iter()
                .all(|step| !matches!(step, AgentTraceStep::Thinking { .. })),
            "普通 content 不是 provider reasoning，不应伪装成思维链: {:?}",
            out.trace
        );
        assert!(out
            .trace
            .iter()
            .any(|step| matches!(step, AgentTraceStep::Tool { .. })));
    }

    #[test]
    fn reasoning_lands_in_the_trace_with_a_duration() {
        let model = ScriptedModel::new(vec![AiCompletionResponse {
            content: "答案".into(),
            reasoning: Some("  想了想  ".into()),
            ..Default::default()
        }]);
        let out = run(&model, &ScriptedTools::new(vec![]), &ask("问"), silent).unwrap();
        let AgentTraceStep::Thinking { text, duration_ms } = &out.trace[0] else {
            panic!()
        };
        assert_eq!(text, "想了想");
        assert!(duration_ms.is_some(), "模型轮的思维链要带耗时");
    }

    /// 只保留最新一份计划，否则改过五次计划的任务轨迹里会有五份几乎一样的清单。
    #[test]
    fn only_the_newest_plan_snapshot_is_kept_in_the_trace() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", AGENT_PLAN_TOOL, "{}")]),
            calls("", vec![tool_call("c2", AGENT_PLAN_TOOL, "{}")]),
            says("完成"),
        ]);
        let tools = ScriptedTools::new(vec![
            r#"{"ok":true,"data":{"steps":[{"title":"第一版","status":"pending"}]}}"#,
            r#"{"ok":true,"data":{"steps":[{"title":"第二版","status":"done"}],"note":"改了"}}"#,
        ]);
        let out = run(&model, &tools, &ask("规划"), silent).unwrap();

        let plans: Vec<_> = out
            .trace
            .iter()
            .filter_map(|s| match s {
                AgentTraceStep::Plan { steps, note } => Some((steps, note)),
                _ => None,
            })
            .collect();
        assert_eq!(plans.len(), 1, "轨迹里只该有一份计划: {:?}", out.trace);
        assert_eq!(plans[0].0[0]["title"], "第二版");
        assert_eq!(plans[0].1.as_deref(), Some("改了"));

        // 但两次调用本身都要留在工具轨迹里——计划改过几次是有意义的信息
        let tool_steps = out
            .trace
            .iter()
            .filter(|s| matches!(s, AgentTraceStep::Tool { .. }))
            .count();
        assert_eq!(tool_steps, 2);
    }

    /// 失败的 agent_plan 不产生计划快照。
    #[test]
    fn a_rejected_plan_does_not_enter_the_trace() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", AGENT_PLAN_TOOL, "{}")]),
            says("好"),
        ]);
        let tools = ScriptedTools::new(vec![r#"{"ok":false,"error":"steps 需要至少一个步骤"}"#]);
        let out = run(&model, &tools, &ask("规划"), silent).unwrap();
        assert!(!out
            .trace
            .iter()
            .any(|s| matches!(s, AgentTraceStep::Plan { .. })));
    }

    // ---------- 事件 ----------

    #[test]
    fn events_are_emitted_in_order() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", "file_read", r#"{"path":"a.md"}"#)]),
            says("好了"),
        ])
        .with_chunks(vec![AiStreamChunk {
            delta: Some("片".into()),
            reasoning_delta: Some("思".into()),
            finish_reason: None,
        }]);

        let mut log: Vec<String> = Vec::new();
        run(
            &model,
            &ScriptedTools::new(vec![r#"{"ok":true,"data":{}}"#]),
            &ask("读"),
            |e| {
                log.push(match e {
                    AgentEvent::ModelStart { iteration } => format!("model_start:{iteration}"),
                    AgentEvent::ReasoningDelta { delta } => format!("reasoning:{delta}"),
                    AgentEvent::ContentDelta { delta } => format!("content:{delta}"),
                    AgentEvent::ToolStart { name, summary } => {
                        format!("tool_start:{name}:{}", summary.unwrap_or("-"))
                    }
                    AgentEvent::ToolResult { name, ok, .. } => format!("tool_result:{name}:{ok}"),
                    AgentEvent::TraceStep { .. } => "trace".into(),
                    AgentEvent::Final { content } => format!("final:{content}"),
                    AgentEvent::Error { error } => format!("error:{error}"),
                });
            },
        )
        .unwrap();

        assert_eq!(
            log,
            [
                "model_start:0",
                "reasoning:思",
                "content:片",
                "tool_start:file_read:a.md",
                "tool_result:file_read:true",
                "trace",
                "model_start:1",
                "reasoning:思",
                "content:片",
                "final:好了",
            ]
        );
    }

    // ---------- 轮次上限 ----------

    /// 模型一直调工具时必须停下来，而不是把余额跑光。
    #[test]
    fn the_iteration_cap_stops_a_runaway_loop() {
        let script = (0..10)
            .map(|i| calls("", vec![tool_call(&format!("c{i}"), "file_read", "{}")]))
            .collect();
        let model = ScriptedModel::new(script);
        let request = AgentRequest {
            max_tool_iterations: 3,
            ..ask("死循环")
        };

        let mut errors = Vec::new();
        let out = run(&model, &ScriptedTools::new(vec![]), &request, |e| {
            if let AgentEvent::Error { error } = e {
                errors.push(error.to_owned());
            }
        })
        .unwrap();

        assert_eq!(model.rounds(), 3, "不能超过上限");
        assert_eq!(out.response.finish_reason, "error");
        assert!(out.response.content.contains("轮次过多"));
        assert_eq!(errors.len(), 1, "要发一次 error 事件让 UI 知道为什么停了");
        assert_eq!(out.trace.len(), 3, "已完成的工作要留在轨迹里，不能整个丢掉");
    }

    // ---------- 上下文预算 ----------

    #[test]
    fn the_budget_has_a_floor_so_a_tiny_window_still_works() {
        let request = AgentRequest {
            context_window_tokens: 100,
            ..Default::default()
        };
        assert_eq!(context_budget(&request), MIN_CONTEXT_BUDGET);
    }

    #[test]
    fn the_budget_reserves_room_for_the_reply() {
        let request = AgentRequest {
            context_window_tokens: 1_000_000,
            max_tokens: Some(50_000),
            ..Default::default()
        };
        assert_eq!(context_budget(&request), 800_000 - 50_000);

        // 没给 max_tokens 时按 2000 预留
        let default = AgentRequest {
            context_window_tokens: 1_000_000,
            ..Default::default()
        };
        assert_eq!(context_budget(&default), 800_000 - ASSUMED_MAX_TOKENS);
    }

    /// 一进来就超预算的话，一个模型请求都不该发出去——这条省的是真金白银。
    #[test]
    fn an_oversized_transcript_aborts_before_calling_the_model() {
        let model = ScriptedModel::new(vec![says("不该被调用")]);
        let request = AgentRequest {
            messages: vec![AiMessage::new("user", &"啊".repeat(200_000))],
            context_window_tokens: 1_000,
            ..Default::default()
        };
        let out = run(&model, &ScriptedTools::new(vec![]), &request, silent).unwrap();

        assert_eq!(model.rounds(), 0, "超预算就不该发请求");
        assert_eq!(out.response.finish_reason, "error");
        assert!(out.response.content.contains("超过安全预算"));
    }

    /// 旧的工具结果被压成引用摘要，**最新那条原样保留**——模型正要基于它下判断。
    #[test]
    fn compaction_shrinks_old_tool_results_and_spares_the_newest() {
        let bulky = |n: usize| {
            let items: Vec<Value> = (0..n)
                .map(|i| json!({ "path": format!("第{i}篇笔记.md") }))
                .collect();
            json!({ "ok": true, "data": { "scanId": "s-1", "files": items } }).to_string()
        };
        let mut messages = vec![
            AiMessage::new("system", "sys"),
            AiMessage::tool_result("c1", "folder_list", &bulky(400)),
            AiMessage::tool_result("c2", "folder_list", &bulky(400)),
        ];
        let newest_before = messages[2].content.clone();

        let before = transcript_tokens(&messages, &[]);
        let after = compact_transcript(&mut messages, &[], 2_000);

        assert!(after < before, "{before} → {after} 应该变小");
        assert_eq!(
            messages[2].content, newest_before,
            "最新一条工具结果不能被压"
        );

        let compacted: Value =
            serde_json::from_str(messages[1].content.as_deref().unwrap()).unwrap();
        assert_eq!(compacted["compacted"], true);
        assert_eq!(compacted["ok"], true);
        assert_eq!(compacted["tool"], "folder_list");
        assert_eq!(compacted["scanId"], "s-1", "引用型字段要留下，模型靠它回查");
        assert_eq!(compacted["filesCount"], 400, "数组换成计数");
        assert!(compacted.get("files").is_none(), "数组本身要被丢掉");
    }

    /// 没超预算时一个字都不动——压缩是有损的，不该在不需要时发生。
    #[test]
    fn nothing_is_compacted_while_under_budget() {
        let payload = json!({"ok": true, "data": {"files": ["a", "b"]}}).to_string();
        let mut messages = vec![
            AiMessage::tool_result("c1", "folder_list", &payload),
            AiMessage::tool_result("c2", "folder_list", &payload),
        ];
        compact_transcript(&mut messages, &[], 1_000_000);
        assert_eq!(messages[0].content.as_deref(), Some(payload.as_str()));
    }

    /// 短结果不值得压——压了省不下多少，却丢了细节。
    #[test]
    fn short_tool_results_are_left_alone_even_over_budget() {
        let short = json!({"ok": true, "data": {"n": 1}}).to_string();
        let mut messages = vec![
            AiMessage::tool_result("c1", "t", &short),
            AiMessage::new("user", &"啊".repeat(50_000)),
            AiMessage::tool_result("c2", "t", &short),
        ];
        compact_transcript(&mut messages, &[], 100);
        assert_eq!(messages[0].content.as_deref(), Some(short.as_str()));
    }

    #[test]
    fn a_non_json_tool_result_compacts_to_a_length_reference() {
        let message = AiMessage::tool_result("c1", "shell_run", &"字".repeat(2000));
        let compacted: Value = serde_json::from_str(&compact_tool_result(&message)).unwrap();
        assert_eq!(compacted["compacted"], true);
        assert_eq!(compacted["tool"], "shell_run");
        assert_eq!(compacted["originalCharacters"], 2000);
    }

    /// `+8` 与工具调用恒序列化都照抄 TS——预算临界点由此决定，口径必须一致。
    #[test]
    fn message_token_accounting_matches_ts() {
        // estimateTokens('') = 0，JSON.stringify([]) = "[]" → 1 个 Token，再加 8
        assert_eq!(message_tokens(&AiMessage::new("user", "")), 9);
        assert_eq!(
            message_tokens(&AiMessage::new("user", "hello world")),
            3 + 1 + 8
        );
    }

    // ---------- 摘要与预览 ----------

    #[test]
    fn arg_summaries_prefer_the_most_telling_field() {
        // path 排在 query 前面
        assert_eq!(
            summarize_tool_args(&json!({"query": "找什么", "path": "笔记/a.md"})).as_deref(),
            Some("笔记/a.md")
        );
        assert_eq!(
            summarize_tool_args(&json!({"query": "找什么"})).as_deref(),
            Some("找什么")
        );
        // 都不在偏好表里就退回第一个字段
        assert_eq!(
            summarize_tool_args(&json!({"limit": 5})).as_deref(),
            Some("limit=5")
        );
        assert_eq!(
            summarize_tool_args(&json!({"items": [1, 2, 3]})).as_deref(),
            Some("items ×3")
        );
        assert_eq!(summarize_tool_args(&json!({})), None);
        // 偏好字段为空白时跳过偏好分支，改用“第一个字段”的备用分支——
        // 于是摘要成了 `path=  `。看起来像个问题，但 TS 版本也是如此（`value.trim()` 只把它挡在
        // 偏好分支外，备用分支使用未经 `trim()` 处理的原值）。这里只是轨迹中的一行标签，
        // 保留该行为以兼容 JavaScript 的参数摘要。
        assert_eq!(
            summarize_tool_args(&json!({"path": "  "})).as_deref(),
            Some("path=  ")
        );
    }

    #[test]
    fn long_summaries_are_truncated_with_an_ellipsis() {
        let long = "字".repeat(200);
        let summary = summarize_tool_args(&json!({ "path": long })).unwrap();
        assert_eq!(summary.chars().count(), 81, "80 字 + 省略号");
        assert!(summary.ends_with('…'));
    }

    /// 按 char 切，永远不会把代理对劈成两半产生乱码。
    #[test]
    fn truncation_never_splits_a_surrogate_pair() {
        let emoji = "🌸".repeat(10);
        assert_eq!(truncate_text(&emoji, 3), "🌸🌸🌸…");
        assert_eq!(truncate_text("短", 10), "短", "不超长就原样返回");
    }

    #[test]
    fn result_previews_cover_success_failure_and_empty() {
        assert_eq!(
            preview_tool_result(&json!({"ok": true, "data": {"n": 1}})).as_deref(),
            Some(r#"{"n":1}"#)
        );
        assert_eq!(
            preview_tool_result(&json!({"ok": false, "error": "炸了"})).as_deref(),
            Some("炸了")
        );
        assert_eq!(
            preview_tool_result(&json!({"ok": false})).as_deref(),
            Some("失败"),
            "失败但没给 error 时也要有个说法"
        );
        assert_eq!(preview_tool_result(&json!({"ok": true})), None);
        assert_eq!(
            preview_tool_result(&json!({"ok": true, "data": null})),
            None
        );
    }

    // ---------- 返回的 transcript ----------

    /// 返回完整 transcript 是为了让调用方能原样持久化会话——
    /// 工具往返也要存，否则续聊时模型看不到自己上一轮干过什么。
    #[test]
    fn the_result_carries_the_full_transcript_for_persistence() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", "file_read", "{}")]),
            says("好了"),
        ]);
        let out = run(
            &model,
            &ScriptedTools::new(vec![r#"{"ok":true,"data":{}}"#]),
            &ask("读"),
            silent,
        )
        .unwrap();

        let roles: Vec<&str> = out.transcript.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool"]);
        assert!(
            out.transcript[2].hidden && out.transcript[3].hidden,
            "工具往返在 UI 里默认折叠"
        );
    }

    /// `hidden` 是墨池内部字段，绝不能出现在发给服务方的报文中。
    #[test]
    fn the_hidden_flag_never_reaches_the_provider() {
        let model = ScriptedModel::new(vec![
            calls("", vec![tool_call("c1", "file_read", "{}")]),
            says("好"),
        ]);
        run(&model, &ScriptedTools::new(vec![]), &ask("读"), silent).unwrap();
        let payload = serde_json::to_string(&model.seen.borrow()[1]).unwrap();
        assert!(!payload.contains("hidden"), "{payload}");
    }

    // ---------- 硬失败 ----------

    /// 网络/鉴权错误直接向上抛，不伪装成一次「回答」。
    #[test]
    fn a_transport_error_propagates_instead_of_becoming_an_answer() {
        struct Broken;
        impl ModelClient for Broken {
            fn complete(
                &self,
                _: &AiCompletionRequest,
                _: &mut dyn FnMut(AiStreamChunk),
            ) -> Result<AiCompletionResponse, AiError> {
                Err(AiError::api("鉴权失败", 401))
            }
        }
        let err = run(&Broken, &ScriptedTools::new(vec![]), &ask("问"), silent).unwrap_err();
        assert_eq!(err.status_code, Some(401));
    }

    // ---------- 系统提示词资源 ----------

    /// 提示词由脚本从 TS 抽取，正面/反面 LaTeX 示例都必须在。
    /// 手抄最容易把两者抹平成同一个，那样这段规则就变成自相矛盾的废话。
    #[test]
    fn the_extracted_prompt_keeps_both_latex_examples() {
        assert!(SYSTEM_PROMPT.contains("你是墨池（Mochi）内置 AI 助手"));
        assert!(SYSTEM_PROMPT.contains(r"\frac"), "缺正面示例");
        assert!(SYSTEM_PROMPT.contains(r"\\frac"), "缺反面示例");
        assert!(!SYSTEM_PROMPT.contains('\r'), "提示词必须是 LF");
    }

    /// 提示词里点名的工具必须真的存在，否则模型会去调一个不存在的名字。
    #[test]
    fn every_tool_named_in_the_prompt_exists() {
        for name in [
            "current_document_get",
            "document_range_propose_edit",
            "file_write",
            "agenda_overview",
            "agenda_list",
            "agenda_batch",
            "shell_run",
            "web_search",
            "http_request",
            "memory_search",
            "memory_write",
            "agent_plan",
            "mochi_docs",
            "settings_list",
            "settings_update",
            "settings_reset",
            "kb_create",
            "folder_create",
            "path_rename",
        ] {
            assert!(
                SYSTEM_PROMPT.contains(name),
                "提示词没提到 {name}？抽取可能过时了"
            );
            assert!(
                crate::ai::tools::definitions::find(name).is_some(),
                "提示词点名了不存在的工具 {name}"
            );
        }
    }
}
