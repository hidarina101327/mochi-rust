//! Anthropic Messages API 的线上格式。
//!
//! AI 核心的其余部分刻意只暴露一种「提供方中立」的模型
//!（AiCompletionRequest/AiCompletionResponse）。本模块负责在这个
//! 边界上做翻译，OpenAI 兼容客户端的既有行为因此原样保留。

use std::collections::BTreeMap;

use base64::Engine;
use serde_json::{json, Map, Value};

use super::models::*;

/// 直连 Anthropic API 要求的稳定 API 版本。
pub const API_VERSION: &str = "2023-06-01";
pub const DEFAULT_MAX_TOKENS: i32 = 4096;

/// 把提供方设置归一到 Anthropic 的 Messages 端点。
///
/// 用户常粘贴主机名、/v1 或完整的 /v1/messages。/v1 之前的网关路径保留。
pub fn messages_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/messages") {
        return trimmed.to_owned();
    }

    let mut root = trimmed;
    for suffix in ["/chat/completions", "/responses", "/completions", "/models"] {
        if let Some(stripped) = root.strip_suffix(suffix) {
            root = stripped;
            break;
        }
    }
    if root.ends_with("/v1") {
        format!("{root}/messages")
    } else {
        format!("{root}/v1/messages")
    }
}

pub fn models_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/models") {
        return trimmed.to_owned();
    }
    let messages = messages_url(base_url);
    messages
        .strip_suffix("/messages")
        .map(|base| format!("{base}/models"))
        .unwrap_or_else(|| format!("{messages}/models"))
}

pub fn models_url_after(base_url: &str, after_id: &str) -> String {
    let mut url = models_url(base_url);
    let encoded: String = url::form_urlencoded::byte_serialize(after_id.as_bytes()).collect();
    url.push_str("?after_id=");
    url.push_str(&encoded);
    url
}

fn data_url(value: &str) -> Option<(&str, &str)> {
    let rest = value.strip_prefix("data:")?;
    let (metadata, data) = rest.split_once(',')?;
    if !metadata
        .split(';')
        .any(|part| part.eq_ignore_ascii_case("base64"))
    {
        return None;
    }
    let media_type = metadata.split(';').next().filter(|mime| !mime.is_empty())?;
    Some((media_type, data))
}

fn mime_from_filename(filename: &str) -> &'static str {
    match filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt") | Some("md") | Some("csv") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn decode_plain_text(data: &str) -> String {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| data.to_owned())
}

fn image_block(url: &str) -> Result<Value, AiError> {
    if let Some((media_type, data)) = data_url(url) {
        if !matches!(
            media_type.to_ascii_lowercase().as_str(),
            "image/jpeg" | "image/png" | "image/gif" | "image/webp"
        ) {
            return Err(AiError::new(
                format!("Anthropic 不支持该图片类型：{media_type}"),
                "API_ERROR",
            ));
        }
        Ok(json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data}
        }))
    } else {
        Ok(json!({
            "type": "image",
            "source": {"type": "url", "url": url}
        }))
    }
}

fn file_block(file: &AiFile) -> Result<Value, AiError> {
    let fallback_mime = mime_from_filename(&file.filename);
    let is_url = file.file_data.starts_with("http://") || file.file_data.starts_with("https://");
    let (media_type, data, encoded) = if let Some((media_type, data)) = data_url(&file.file_data) {
        (media_type.to_owned(), data.to_owned(), true)
    } else if is_url {
        (fallback_mime.to_owned(), file.file_data.clone(), false)
    } else if !file.file_data.is_empty() {
        (fallback_mime.to_owned(), file.file_data.clone(), true)
    } else {
        return Err(AiError::new(
            format!(
                "Anthropic 文件附件 {} 缺少 file_data",
                if file.filename.is_empty() {
                    "未命名"
                } else {
                    &file.filename
                }
            ),
            "API_ERROR",
        ));
    };

    let lower = media_type.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    ) {
        let source = if is_url {
            json!({"type": "url", "url": data})
        } else {
            json!({"type": "base64", "media_type": lower, "data": data})
        };
        return Ok(json!({"type": "image", "source": source}));
    }

    if lower != "application/pdf" && lower != "text/plain" {
        return Err(AiError::new(
            format!("Anthropic 不支持文件类型 {media_type}"),
            "API_ERROR",
        ));
    }
    let source = if is_url {
        json!({"type": "url", "url": data})
    } else if lower == "text/plain" {
        json!({"type": "text", "media_type": "text/plain", "data": if encoded {
            decode_plain_text(&data)
        } else {
            data
        }})
    } else {
        json!({"type": "base64", "media_type": "application/pdf", "data": data})
    };
    let mut block = json!({"type": "document", "source": source});
    if !file.filename.trim().is_empty() {
        block["title"] = json!(file.filename);
    }
    Ok(block)
}

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn multimodal_blocks(message: &AiMessage) -> Result<Vec<Value>, AiError> {
    let mut blocks = Vec::new();
    for file in &message.files {
        blocks.push(file_block(file)?);
    }
    for url in &message.images {
        blocks.push(image_block(url)?);
    }
    if let Some(text) = message.content.as_deref().filter(|text| !text.is_empty()) {
        blocks.push(text_block(text));
    }
    Ok(blocks)
}

fn message_content(message: &AiMessage) -> Result<Value, AiError> {
    let blocks = multimodal_blocks(message)?;
    if blocks.is_empty() {
        Ok(json!(message.content.as_deref().unwrap_or_default()))
    } else {
        Ok(json!(blocks))
    }
}

fn tool_input(arguments: &str) -> Result<Value, AiError> {
    if arguments.trim().is_empty() {
        return Ok(json!({}));
    }
    let input: Value = serde_json::from_str(arguments).map_err(|error| {
        AiError::new(
            format!("Anthropic tool_use 参数 JSON 无效：{error}"),
            "API_ERROR",
        )
    })?;
    if !input.is_object() {
        return Err(AiError::new(
            "Anthropic tool_use 参数必须是 JSON 对象",
            "API_ERROR",
        ));
    }
    Ok(input)
}

fn assistant_content(message: &AiMessage) -> Result<Value, AiError> {
    if message.tool_calls.as_ref().is_none_or(Vec::is_empty)
        && message.images.is_empty()
        && message.files.is_empty()
    {
        return Ok(json!(message.content.as_deref().unwrap_or_default()));
    }

    let mut blocks = multimodal_blocks(message)?;
    for call in message.tool_calls.as_deref().unwrap_or_default() {
        blocks.push(json!({
            "type": "tool_use",
            "id": call.id,
            "name": call.function.name,
            "input": tool_input(&call.function.arguments)?,
        }));
    }
    Ok(json!(blocks))
}

fn tool_result_content(message: &AiMessage) -> Value {
    json!([{
        "type": "tool_result",
        "tool_use_id": message.tool_call_id.as_deref().unwrap_or_default(),
        "content": message.content.as_deref().unwrap_or_default(),
    }])
}

fn tool_choice(choice: &str) -> Value {
    match choice.trim() {
        "none" => json!({"type": "none"}),
        "required" | "any" => json!({"type": "any"}),
        "auto" | "" => json!({"type": "auto"}),
        name if name.strip_prefix("tool:").is_some() => {
            json!({"type": "tool", "name": name.trim_start_matches("tool:")})
        }
        name => json!({"type": "tool", "name": name}),
    }
}

/// 把提供方中立的请求转换成 Anthropic Messages 请求。
pub fn build_payload(
    provider: &AiProvider,
    request: &AiCompletionRequest,
    stream: bool,
) -> Result<Value, AiError> {
    let mut payload = Map::new();
    payload.insert("model".into(), json!(provider.model));
    payload.insert(
        "max_tokens".into(),
        json!(request
            .max_tokens
            .filter(|max| *max > 0)
            .unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    payload.insert("messages".into(), json!([]));
    payload.insert("stream".into(), json!(stream));

    let mut messages = Vec::new();
    let mut system = Vec::new();
    for message in &request.messages {
        match message.role.as_str() {
            "system" => {
                if let Some(text) = message.content.as_deref().filter(|text| !text.is_empty()) {
                    system.push(text.to_owned());
                }
            }
            "tool" => messages.push(json!({
                "role": "user",
                "content": tool_result_content(message),
            })),
            "assistant" => messages.push(json!({
                "role": "assistant",
                "content": assistant_content(message)?,
            })),
            _ => messages.push(json!({
                "role": "user",
                "content": message_content(message)?,
            })),
        }
    }
    payload.insert("messages".into(), json!(messages));
    if !system.is_empty() {
        payload.insert("system".into(), json!(system.join("\n\n")));
    }

    // Anthropic 在较新的模型上已弃用采样温度。即便共享请求带了
    // temperature 也刻意不传——那些模型只接受 1.0，其他值一律拒绝。
    if !request.tools.is_empty() {
        payload.insert(
            "tools".into(),
            json!(request
                .tools
                .iter()
                .map(|tool| json!({
                    "name": tool.function.name,
                    "description": tool.function.description,
                    "input_schema": tool.function.parameters,
                }))
                .collect::<Vec<_>>()),
        );
        payload.insert(
            "tool_choice".into(),
            tool_choice(request.tool_choice.as_deref().unwrap_or("auto")),
        );
    } else if let Some(choice) = request.tool_choice.as_deref() {
        // 就算调用方没附工具，显式要求「不使用工具」也要照办。
        if choice.trim() == "none" {
            payload.insert("tool_choice".into(), json!({"type": "none"}));
        }
    }
    Ok(Value::Object(payload))
}

fn stop_reason(reason: Option<&str>) -> String {
    match reason {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") | Some("model_context_window_exceeded") => "length",
        Some("end_turn") | Some("stop_sequence") | Some("pause_turn") | None => "stop",
        Some(_) => "error",
    }
    .into()
}

fn usage_value(usage: &Value, key: &str) -> i32 {
    usage
        .get(key)
        .and_then(Value::as_i64)
        .unwrap_or_default()
        .clamp(0, i32::MAX as i64) as i32
}

/// 解析非流式的 Anthropic Message 响应。
pub fn parse_message(
    body: &Value,
    request: &AiCompletionRequest,
) -> Result<AiCompletionResponse, AiError> {
    let blocks = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| AiError::new("响应缺少 content", "API_ERROR"))?;

    let mapped_stop_reason = stop_reason(body.get("stop_reason").and_then(Value::as_str));
    let mut result = AiCompletionResponse::default();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    result.content.push_str(text);
                }
            }
            Some("thinking") => {
                if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                    result
                        .reasoning
                        .get_or_insert_with(String::new)
                        .push_str(thinking);
                }
            }
            Some("tool_use") => {
                if mapped_stop_reason == "length" {
                    continue;
                }
                let input = block.get("input").cloned().ok_or_else(|| {
                    AiError::new("Anthropic tool_use 响应缺少 input", "API_ERROR")
                })?;
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| AiError::new("Anthropic tool_use 响应缺少 id", "API_ERROR"))?;
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| AiError::new("Anthropic tool_use 响应缺少 name", "API_ERROR"))?;
                if !input.is_object() {
                    return Err(AiError::new(
                        "Anthropic tool_use 参数必须是 JSON 对象",
                        "API_ERROR",
                    ));
                }
                tool_calls.push(AiToolCall {
                    id: id.to_owned(),
                    kind: "function".into(),
                    function: AiToolFunction {
                        name: name.to_owned(),
                        arguments: serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()),
                    },
                });
            }
            _ => {}
        }
    }
    result.tool_calls = tool_calls;
    result.finish_reason = if mapped_stop_reason == "length" {
        "length".into()
    } else if result.tool_calls.is_empty() {
        mapped_stop_reason
    } else {
        "tool_calls".into()
    };

    if let Some(usage) = body.get("usage") {
        result.prompt_tokens = usage_value(usage, "input_tokens");
        result.completion_tokens = usage_value(usage, "output_tokens");
        result.total_tokens = result
            .prompt_tokens
            .saturating_add(result.completion_tokens);
        result.usage_source = "provider".into();
    } else {
        result.prompt_tokens = prompt_tokens(request);
        result.completion_tokens = estimate_tokens(&result.content)
            + result
                .reasoning
                .as_deref()
                .map(estimate_tokens)
                .unwrap_or_default();
        result.total_tokens = result
            .prompt_tokens
            .saturating_add(result.completion_tokens);
    }
    Ok(result)
}

fn prompt_tokens(request: &AiCompletionRequest) -> i32 {
    request
        .messages
        .iter()
        .map(|message| estimate_tokens(message.content.as_deref().unwrap_or_default()))
        .sum()
}

#[derive(Default, Clone)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
    input_present: bool,
    closed: bool,
}

/// 把 Anthropic 的命名 SSE 事件累积成提供方中立的流式分块和最终响应。
/// 未知事件类型刻意忽略，未来 API 新增字段时保持前向兼容。
#[derive(Default)]
pub struct AnthropicSseAccumulator {
    content: String,
    reasoning: String,
    finish_reason: Option<String>,
    message_stop: bool,
    event_name: Option<String>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    saw_usage: bool,
    tool_calls: BTreeMap<i64, ToolCallAccumulator>,
}

impl AnthropicSseAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_line(&mut self, line: &str) -> Result<Vec<AiStreamChunk>, AiError> {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            self.event_name = None;
            return Ok(Vec::new());
        }
        if let Some(event) = line.strip_prefix("event:") {
            self.event_name = Some(event.trim().to_owned());
            return Ok(Vec::new());
        }
        let Some(payload) = line.strip_prefix("data:") else {
            return Ok(Vec::new());
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(payload).map_err(|error| {
            AiError::new(format!("Anthropic SSE 数据无效：{error}"), "API_ERROR")
        })?;
        let event_type = self
            .event_name
            .take()
            .or_else(|| value.get("type").and_then(Value::as_str).map(str::to_owned));

        if event_type.as_deref() == Some("error")
            || value.get("type").and_then(Value::as_str) == Some("error")
        {
            let error = value.get("error").unwrap_or(&value);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Anthropic stream error");
            return Err(AiError::new(message, "API_ERROR"));
        }

        let mut chunks = Vec::new();
        match event_type
            .as_deref()
            .or_else(|| value.get("type").and_then(Value::as_str))
        {
            Some("message_start") => {
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    self.read_usage(usage);
                }
            }
            Some("content_block_start") => {
                let block = value.get("content_block").unwrap_or(&Value::Null);
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            self.content.push_str(text);
                            if !text.is_empty() {
                                chunks.push(AiStreamChunk {
                                    delta: Some(text.to_owned()),
                                    reasoning_delta: None,
                                    finish_reason: None,
                                });
                            }
                        }
                    }
                    Some("thinking") => {
                        if let Some(thinking) = block.get("thinking").and_then(Value::as_str) {
                            self.reasoning.push_str(thinking);
                            if !thinking.is_empty() {
                                chunks.push(AiStreamChunk {
                                    delta: Some(String::new()),
                                    reasoning_delta: Some(thinking.to_owned()),
                                    finish_reason: None,
                                });
                            }
                        }
                    }
                    Some("tool_use") => {
                        let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                        let entry = self.tool_calls.entry(index).or_default();
                        if entry.closed {
                            return Err(AiError::new(
                                "Anthropic tool_use 内容块重复开始",
                                "API_ERROR",
                            ));
                        }
                        if let Some(id) = block.get("id").and_then(Value::as_str) {
                            entry.id = id.to_owned();
                        }
                        if let Some(name) = block.get("name").and_then(Value::as_str) {
                            entry.name = name.to_owned();
                        }
                        entry.input_present = block.get("input").is_some();
                        if let Some(input) = block.get("input") {
                            if input.as_object().is_some_and(|object| !object.is_empty()) {
                                entry.arguments =
                                    serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("content_block_delta") => {
                let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                if let Some(delta) = value.get("delta") {
                    match delta.get("type").and_then(Value::as_str) {
                        Some("text_delta") => {
                            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                self.content.push_str(text);
                                chunks.push(AiStreamChunk {
                                    delta: Some(text.to_owned()),
                                    reasoning_delta: None,
                                    finish_reason: None,
                                });
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
                                self.reasoning.push_str(thinking);
                                chunks.push(AiStreamChunk {
                                    delta: Some(String::new()),
                                    reasoning_delta: Some(thinking.to_owned()),
                                    finish_reason: None,
                                });
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                            {
                                let entry = self.tool_calls.entry(index).or_default();
                                if entry.closed {
                                    return Err(AiError::new(
                                        "Anthropic tool_use 内容块已结束后仍收到参数",
                                        "API_ERROR",
                                    ));
                                }
                                entry.arguments.push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("content_block_stop") => {
                let index = value.get("index").and_then(Value::as_i64).unwrap_or(0);
                if let Some(entry) = self.tool_calls.get_mut(&index) {
                    entry.closed = true;
                }
            }
            Some("message_delta") => {
                if let Some(reason) = value
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.finish_reason = Some(stop_reason(Some(reason)));
                }
                if let Some(usage) = value.get("usage") {
                    self.read_usage(usage);
                }
            }
            Some("message_stop") => {
                self.message_stop = true;
            }
            _ => {}
        }
        Ok(chunks)
    }

    fn read_usage(&mut self, usage: &Value) {
        self.saw_usage = true;
        if usage.get("input_tokens").is_some() {
            self.input_tokens = Some(usage_value(usage, "input_tokens"));
        }
        if usage.get("output_tokens").is_some() {
            self.output_tokens = Some(usage_value(usage, "output_tokens"));
        }
    }

    pub fn has_completion_marker(&self) -> bool {
        self.message_stop
    }

    pub fn finish_reason(&self) -> String {
        self.finish_reason.clone().unwrap_or_else(|| "stop".into())
    }

    pub fn finish(self, fallback_prompt_tokens: i32) -> Result<AiCompletionResponse, AiError> {
        let mut tool_calls = Vec::new();
        let length_limited = self.finish_reason.as_deref() == Some("length");
        for call in self.tool_calls.into_values() {
            if length_limited {
                continue;
            }
            if !call.closed {
                return Err(AiError::new("Anthropic tool_use 内容块未结束", "API_ERROR"));
            }
            if call.id.is_empty() || call.name.is_empty() {
                return Err(AiError::new(
                    "Anthropic tool_use 响应缺少 id 或 name",
                    "API_ERROR",
                ));
            }
            let arguments = if call.arguments.is_empty() {
                if !call.input_present {
                    return Err(AiError::new(
                        "Anthropic tool_use 响应缺少 input",
                        "API_ERROR",
                    ));
                }
                "{}".into()
            } else {
                let parsed: Value = serde_json::from_str(&call.arguments).map_err(|error| {
                    AiError::new(
                        format!("Anthropic tool_use 参数 JSON 不完整：{error}"),
                        "API_ERROR",
                    )
                })?;
                if !parsed.is_object() {
                    return Err(AiError::new(
                        "Anthropic tool_use 参数必须是 JSON 对象",
                        "API_ERROR",
                    ));
                }
                call.arguments
            };
            tool_calls.push(AiToolCall {
                id: call.id,
                kind: "function".into(),
                function: AiToolFunction {
                    name: call.name,
                    arguments,
                },
            });
        }
        let prompt_tokens = self.input_tokens.unwrap_or(fallback_prompt_tokens);
        let finish_reason = if tool_calls.is_empty() {
            self.finish_reason.unwrap_or_else(|| "stop".into())
        } else {
            "tool_calls".into()
        };
        let content = self.content;
        let reasoning = self.reasoning;
        let completion_tokens = self
            .output_tokens
            .unwrap_or_else(|| estimate_tokens(&content) + estimate_tokens(&reasoning));
        Ok(AiCompletionResponse {
            content,
            finish_reason,
            tool_calls,
            reasoning: (!reasoning.is_empty()).then_some(reasoning),
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
            usage_source: if self.saw_usage {
                "provider".into()
            } else {
                "estimated".into()
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalization_always_uses_v1_messages() {
        assert_eq!(
            messages_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            messages_url("https://api.anthropic.com/v1/"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            messages_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            messages_url("http://localhost:8000/anthropic/messages"),
            "http://localhost:8000/anthropic/messages"
        );
        assert_eq!(
            models_url_after("https://api.anthropic.com/v1", "claude-sonnet-4"),
            "https://api.anthropic.com/v1/models?after_id=claude-sonnet-4"
        );
    }

    #[test]
    fn payload_moves_system_and_multimodal_content_and_tools() {
        let mut user = AiMessage::new("user", "看这个");
        user.images = vec!["data:image/png;base64,YQ==".into()];
        user.files = vec![AiFile {
            filename: "guide.pdf".into(),
            file_data: "data:application/pdf;base64,Yg==".into(),
        }];
        let request = AiCompletionRequest {
            messages: vec![AiMessage::new("system", "be concise"), user],
            max_tokens: Some(0),
            temperature: Some(0.7),
            tools: vec![AiToolDefinition::function(
                "lookup",
                "Look something up",
                json!({"type":"object","properties":{"q":{"type":"string"}}}),
            )],
            tool_choice: Some("required".into()),
            ..Default::default()
        };
        let body = build_payload(
            &AiProvider {
                model: "claude-sonnet-4".into(),
                protocol: "anthropic-messages".into(),
                ..Default::default()
            },
            &request,
            true,
        )
        .unwrap();
        assert_eq!(body["max_tokens"], DEFAULT_MAX_TOKENS);
        assert_eq!(body["system"], "be concise");
        assert_eq!(body["stream"], true);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["messages"][0]["content"][0]["type"], "document");
        assert_eq!(body["messages"][0]["content"][1]["type"], "image");
        assert_eq!(body["messages"][0]["content"][2]["type"], "text");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["tool_choice"]["type"], "any");
    }

    #[test]
    fn non_stream_message_maps_text_thinking_tools_and_usage() {
        let body = json!({
            "content": [
                {"type":"text","text":"answer"},
                {"type":"thinking","thinking":"plan"},
                {"type":"tool_use","id":"toolu_1","name":"lookup","input":{"q":"rust"}}
            ],
            "stop_reason":"tool_use",
            "usage":{"input_tokens":12,"output_tokens":8}
        });
        let result = parse_message(&body, &AiCompletionRequest::default()).unwrap();
        assert_eq!(result.content, "answer");
        assert_eq!(result.reasoning.as_deref(), Some("plan"));
        assert_eq!(result.tool_calls[0].function.arguments, r#"{"q":"rust"}"#);
        assert_eq!(result.finish_reason, "tool_calls");
        assert_eq!(
            (
                result.prompt_tokens,
                result.completion_tokens,
                result.total_tokens
            ),
            (12, 8, 20)
        );
        assert_eq!(result.usage_source, "provider");
    }

    #[test]
    fn length_stop_never_dispatches_tool_use_blocks() {
        let body = json!({
            "content": [
                {"type":"text","text":"partial"},
                {"type":"tool_use","id":"toolu_1","name":"lookup","input":{"q":"rust"}}
            ],
            "stop_reason":"max_tokens"
        });
        let result = parse_message(&body, &AiCompletionRequest::default()).unwrap();
        assert_eq!(result.finish_reason, "length");
        assert!(result.tool_calls.is_empty());

        let mut stream = AnthropicSseAccumulator::new();
        for line in [
            "event: content_block_start",
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}}"#,
            "event: message_delta",
            r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
            "event: message_stop",
            r#"data: {"type":"message_stop"}"#,
        ] {
            stream.push_line(line).unwrap();
        }
        let result = stream.finish(0).unwrap();
        assert_eq!(result.finish_reason, "length");
        assert!(result.tool_calls.is_empty());
    }

    #[test]
    fn stream_accumulates_named_events_and_usage() {
        let mut acc = AnthropicSseAccumulator::new();
        let lines = [
            r#"event: message_start"#,
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#,
            r#"event: content_block_start"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
            r#"event: content_block_start"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"rust\"}"}}"#,
            r#"event: content_block_stop"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"event: message_delta"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
            r#"event: message_stop"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let chunks: Vec<_> = lines
            .iter()
            .flat_map(|line| acc.push_line(line).unwrap())
            .collect();
        assert_eq!(chunks[0].delta.as_deref(), Some("Hi"));
        assert!(acc.has_completion_marker());
        let result = acc.finish(0).unwrap();
        assert_eq!(result.content, "Hi");
        assert_eq!(result.tool_calls[0].function.arguments, r#"{"q":"rust"}"#);
        assert_eq!(result.finish_reason, "tool_calls");
        assert_eq!((result.prompt_tokens, result.completion_tokens), (10, 7));
        assert_eq!(result.usage_source, "provider");
    }

    #[test]
    fn stream_tool_without_content_block_stop_is_rejected() {
        let mut acc = AnthropicSseAccumulator::new();
        for line in [
            r#"event: content_block_start"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}}"#,
            r#"event: content_block_delta"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\"rust\"}"}}"#,
            r#"event: message_delta"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"event: message_stop"#,
            r#"data: {"type":"message_stop"}"#,
        ] {
            acc.push_line(line).unwrap();
        }
        let error = acc.finish(0).unwrap_err();
        assert_eq!(error.code, "API_ERROR");
        assert!(error.message.contains("未结束"));
    }

    #[test]
    fn stream_error_event_becomes_api_error() {
        let mut acc = AnthropicSseAccumulator::new();
        acc.push_line("event: error").unwrap();
        let error = acc
            .push_line(
                r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            )
            .unwrap_err();
        assert_eq!(error.code, "API_ERROR");
        assert!(error.message.contains("Overloaded"));
    }
}
