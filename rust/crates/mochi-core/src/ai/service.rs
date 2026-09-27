//! 阻塞式 SSE 客户端，调用方须放到工作线程；TLS 使用系统证书库。

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::models::*;

/// 正常的补全请求可能要等一会儿，但必须有有限上界。设置页用下面
/// 更短的连接超时，坏端点不会让 UI 看起来像挂了。
pub const REQUEST_TIMEOUT_SECONDS: u64 = 60;
pub const CONNECTION_TIMEOUT_SECONDS: u64 = 8;
const MAX_RETRIES: u32 = 1;

/// 把各种 base_url 归一到 `.../chat/completions`。
///
/// 用户常把完整端点粘进设置框，这里逐个剥掉已知后缀再补规范后缀。
pub fn chat_url(base_url: &str) -> String {
    format!("{}/chat/completions", endpoint_base_url(base_url))
}

/// 剥掉粘贴进来的 OpenAI 兼容端点后缀，保留提供方的版本前缀
///（例如 `/v1`）。DeepSeek 和大多数本地网关用同一组
/// `/models` 和 `/chat/completions` 端点。
pub fn endpoint_base_url(base_url: &str) -> String {
    let mut base = base_url.trim_end_matches('/');
    for suffix in [
        "/chat/completions",
        "/responses",
        "/completions",
        "/messages",
        "/models",
    ] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped;
            break;
        }
    }
    base.trim_end_matches('/').to_owned()
}

/// OpenAI 兼容的模型枚举端点。只实现了 chat completions 的提供方，
/// [`AiService::list_models`] 会返回明确的「不支持」结果，
/// 而不是编一份假模型列表。
pub fn models_url(base_url: &str) -> String {
    format!("{}/models", endpoint_base_url(base_url))
}

pub fn map_finish_reason(reason: Option<&str>) -> String {
    match reason {
        Some("stop") => "stop",
        Some("tool_calls") => "tool_calls",
        Some("length") | Some("max_tokens") => "length",
        _ => "error",
    }
    .into()
}

// ---------- SSE 累积器（纯逻辑，可脱网测试）----------

#[derive(Default, Clone)]
struct ToolCallAcc {
    id: String,
    name: String,
    args: String,
}

/// 逐行吃 SSE，攒出最终响应。这是整个客户端最容易出错的部分，
/// 因此抽成不碰网络的纯结构，用录制的报文做回归。
#[derive(Default)]
pub struct SseAccumulator {
    content: String,
    reasoning: String,
    finish_reason: Option<String>,
    done: bool,
    /// 按服务方提供的 `index` 顺序累积，保证多工具调用顺序稳定
    tool_calls: BTreeMap<i64, ToolCallAcc>,
}

impl SseAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 处理一行；返回本行产生的增量分片（可能为空）。
    pub fn push_line(&mut self, line: &str) -> Vec<AiStreamChunk> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        if trimmed == "data: [DONE]" {
            self.done = true;
            return Vec::new();
        }
        let Some(payload) = trimmed.strip_prefix("data: ") else {
            return Vec::new();
        };
        // 无法解析的行直接跳过（服务方有时会插入心跳或注释）
        let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
            return Vec::new();
        };
        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        if let Some(delta) = choice.get("delta") {
            // `reasoning_content` 和 `reasoning` 是两个服务方使用的不同字段名，采用先匹配到的值。
            let reasoning = delta
                .get("reasoning_content")
                .and_then(Value::as_str)
                .or_else(|| delta.get("reasoning").and_then(Value::as_str));
            if let Some(r) = reasoning {
                self.reasoning.push_str(r);
                out.push(AiStreamChunk {
                    delta: Some(String::new()),
                    reasoning_delta: Some(r.to_owned()),
                    finish_reason: None,
                });
            }

            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                self.content.push_str(text);
                out.push(AiStreamChunk {
                    delta: Some(text.to_owned()),
                    reasoning_delta: None,
                    finish_reason: None,
                });
            }

            if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                for tc in tcs {
                    let index = tc.get("index").and_then(Value::as_i64).unwrap_or(0);
                    let entry = self.tool_calls.entry(index).or_default();
                    if let Some(id) = tc.get("id").and_then(Value::as_str) {
                        entry.id = id.to_owned();
                    }
                    if let Some(f) = tc.get("function") {
                        if let Some(name) = f.get("name").and_then(Value::as_str) {
                            entry.name = name.to_owned();
                        }
                        if let Some(args) = f.get("arguments").and_then(Value::as_str) {
                            entry.args.push_str(args);
                        }
                    }
                }
            }
        }

        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(map_finish_reason(Some(fr)));
        }
        out
    }

    pub fn finish_reason(&self) -> String {
        self.finish_reason.clone().unwrap_or_else(|| "stop".into())
    }

    fn has_completion_marker(&self) -> bool {
        self.done || self.finish_reason.is_some()
    }

    /// 收尾：组装工具调用并计算模型用量。
    pub fn finish(self, prompt_tokens: i32) -> AiCompletionResponse {
        let finish_reason = self.finish_reason();
        let tool_calls: Vec<AiToolCall> = self
            .tool_calls
            .into_values()
            .filter(|t| !t.name.is_empty())
            .map(|t| AiToolCall {
                id: t.id,
                kind: "function".into(),
                function: AiToolFunction {
                    name: t.name,
                    // 分片可能一个参数都没带，补个空对象免得下游 JSON 解析炸
                    arguments: if t.args.is_empty() {
                        "{}".into()
                    } else {
                        t.args
                    },
                },
            })
            .collect();

        let completion_tokens = estimate_tokens(&self.content);
        AiCompletionResponse {
            finish_reason: if tool_calls.is_empty() {
                finish_reason
            } else {
                "tool_calls".into()
            },
            tool_calls,
            reasoning: (!self.reasoning.is_empty()).then_some(self.reasoning),
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            usage_source: "estimated".into(),
            content: self.content,
        }
    }
}

// ---------- 客户端 ----------

pub struct AiService {
    provider: AiProvider,
    agent: ureq::Agent,
    cancelled: Arc<AtomicBool>,
}

impl AiService {
    pub fn new(provider: AiProvider) -> Self {
        Self::with_cancel(provider, Arc::new(AtomicBool::new(false)))
    }
    pub fn with_cancel(provider: AiProvider, cancelled: Arc<AtomicBool>) -> Self {
        Self::with_cancel_timeout(
            provider,
            cancelled,
            Duration::from_secs(REQUEST_TIMEOUT_SECONDS),
        )
    }
    /// 用显式截止时间构建服务。设置页动作（连接测试、模型枚举）用
    /// 短截止时间；普通对话保持与提供方一致的 60 秒。
    pub fn with_cancel_timeout(
        provider: AiProvider,
        cancelled: Arc<AtomicBool>,
        timeout: Duration,
    ) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            // `ureq` 3.x 默认使用 Rustls 的加密提供方，即使 crate 只启用了
            // `native-tls`。不显式选 NativeTls，每个 HTTPS 请求都会
            // 直接 panic，连把错误返回给 UI 的机会都没有。
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .provider(ureq::tls::TlsProvider::NativeTls)
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            // 即使状态码不是 2xx，也要读取响应体，才能把服务方返回的错误信息传给用户
            .http_status_as_error(false)
            .build();
        Self {
            provider,
            agent: config.into(),
            cancelled,
        }
    }

    pub fn provider(&self) -> &AiProvider {
        &self.provider
    }

    /// 请求取消标志。流式读取每行都会检查。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn reset_cancel(&self) {
        self.cancelled.store(false, Ordering::Relaxed);
    }

    fn build_payload(&self, request: &AiCompletionRequest, stream: bool) -> Result<Value, AiError> {
        if self.provider.is_anthropic_messages() {
            return super::anthropic::build_payload(&self.provider, request, stream);
        }
        let mut payload = json!({
            "model": self.provider.model,
            "messages": request.messages,
            "temperature": request.temperature.unwrap_or(0.7),
            "stream": stream,
        });
        for (wire, message) in payload["messages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&request.messages)
        {
            if message.role == "user" && (!message.images.is_empty() || !message.files.is_empty()) {
                let mut parts=message.files.iter().map(|file|json!({"type":"file","file":{"filename":file.filename,"file_data":file.file_data}})).collect::<Vec<_>>();
                parts.extend(
                    message
                        .images
                        .iter()
                        .map(|url| json!({"type":"image_url","image_url":{"url":url}})),
                );
                if let Some(text) = message.content.as_ref().filter(|s| !s.is_empty()) {
                    parts.push(json!({"type":"text","text":text}));
                }
                wire["content"] = json!(parts);
            }
        }
        if let Some(max) = request.max_tokens.filter(|m| *m > 0) {
            payload["max_tokens"] = json!(max);
        }
        if !request.tools.is_empty() {
            payload["tools"] = json!(request.tools);
            payload["tool_choice"] =
                json!(request.tool_choice.clone().unwrap_or_else(|| "auto".into()));
        }
        Ok(payload)
    }

    /// 发请求，5xx 重试一次。返回成功响应；非 2xx 转成 `AiError`。
    fn fetch_with_retry(
        &self,
        request: &AiCompletionRequest,
        stream: bool,
    ) -> Result<ureq::http::Response<ureq::Body>, AiError> {
        let anthropic = self.provider.is_anthropic_messages();
        let url = if anthropic {
            super::anthropic::messages_url(&self.provider.base_url)
        } else {
            chat_url(&self.provider.base_url)
        };
        let payload = self.build_payload(request, stream)?;

        let mut last_error = AiError::new("重试次数已用尽", "NETWORK_ERROR");
        for attempt in 0..=MAX_RETRIES {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(AiError::new("请求已取消", "CANCELLED"));
            }

            let mut request = self
                .agent
                .post(&url)
                .header("Content-Type", "application/json");
            let api_key = self.provider.api_key.trim();
            if anthropic {
                if !api_key.is_empty() {
                    request = request.header("x-api-key", api_key);
                }
                request = request.header("anthropic-version", super::anthropic::API_VERSION);
            } else if !api_key.is_empty() {
                request = request.header("Authorization", &format!("Bearer {api_key}"));
            }
            let sent = request.send_json(&payload);

            match sent {
                Ok(mut response) => {
                    let status = response.status().as_u16();
                    if (200..300).contains(&status) {
                        return Ok(response);
                    }
                    let body = match response.body_mut().read_to_string() {
                        Ok(body) => body,
                        Err(error) => return Err(self.body_error("错误响应读取", error)),
                    };
                    if status >= 500 && attempt < MAX_RETRIES {
                        last_error = AiError::api(format!("API request failed: {body}"), status);
                        self.interruptible_delay(attempt)?;
                        continue;
                    }
                    return Err(AiError::api(format!("API request failed: {body}"), status));
                }
                Err(e) => {
                    let timed_out = matches!(e, ureq::Error::Timeout(_));
                    if self.cancelled.load(Ordering::Relaxed) {
                        return Err(AiError::new("请求已取消", "CANCELLED"));
                    }
                    last_error = if timed_out {
                        AiError::new(
                            format!(
                                "请求超时（等待 provider 响应超过配置时限，attempt {}）",
                                attempt + 1
                            ),
                            "TIMEOUT",
                        )
                    } else {
                        AiError::new(
                            format!("网络错误（attempt {}）：{e}", attempt + 1),
                            "NETWORK_ERROR",
                        )
                    };
                    if attempt < MAX_RETRIES && !timed_out {
                        self.interruptible_delay(attempt)?;
                        continue;
                    }
                    return Err(last_error);
                }
            }
        }
        Err(last_error)
    }

    /// 重试之间休眠，但取消操作不用陪完整段退避。设置页请求很依赖
    /// 这一点：第二次点击会顶掉第一次。
    fn interruptible_delay(&self, attempt: u32) -> Result<(), AiError> {
        let total = Duration::from_millis(1000 * (attempt as u64 + 1));
        let step = Duration::from_millis(25);
        let mut elapsed = Duration::ZERO;
        while elapsed < total {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(AiError::new("请求已取消", "CANCELLED"));
            }
            let remaining = total.saturating_sub(elapsed);
            std::thread::sleep(remaining.min(step));
            elapsed += remaining.min(step);
        }
        Ok(())
    }

    fn body_error(&self, context: &str, error: ureq::Error) -> AiError {
        if self.cancelled.load(Ordering::Relaxed) {
            return AiError::new("请求已取消", "CANCELLED");
        }
        if matches!(error, ureq::Error::Timeout(_)) {
            AiError::new(
                format!("{context}超时：provider 在配置时限内没有继续返回数据"),
                "TIMEOUT",
            )
        } else {
            AiError::new(format!("{context}失败：{error}"), "NETWORK_ERROR")
        }
    }

    fn stream_body_error(&self, context: &str, error: std::io::Error) -> AiError {
        if self.cancelled.load(Ordering::Relaxed) {
            return AiError::new("请求已取消", "CANCELLED");
        }
        if error.kind() == std::io::ErrorKind::TimedOut {
            AiError::new(
                format!("{context}超时：provider 在配置时限内没有继续返回数据"),
                "TIMEOUT",
            )
        } else {
            AiError::new(format!("{context}失败：{error}"), "NETWORK_ERROR")
        }
    }

    fn prompt_tokens(request: &AiCompletionRequest) -> i32 {
        request
            .messages
            .iter()
            .map(|m| estimate_tokens(m.content.as_deref().unwrap_or("")))
            .sum()
    }

    /// 非流式补全。
    pub fn complete_once(
        &self,
        request: &AiCompletionRequest,
    ) -> Result<AiCompletionResponse, AiError> {
        let mut response = self.fetch_with_retry(request, false)?;
        let body: Value = response.body_mut().read_json().map_err(|e| {
            if self.cancelled.load(Ordering::Relaxed) {
                AiError::new("请求已取消", "CANCELLED")
            } else if matches!(&e, ureq::Error::Timeout(_)) {
                AiError::new(
                    "响应读取超时：provider 未在配置时限内返回完整 JSON",
                    "TIMEOUT",
                )
            } else {
                AiError::new(format!("响应不是合法 JSON：{e}"), "API_ERROR")
            }
        })?;

        if self.provider.is_anthropic_messages() {
            return super::anthropic::parse_message(&body, request);
        }

        let choice = body
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| AiError::new("响应缺少 choices", "API_ERROR"))?;
        let message = choice
            .get("message")
            .ok_or_else(|| AiError::new("响应缺少 message", "API_ERROR"))?;

        let mut result = AiCompletionResponse {
            content: message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            finish_reason: map_finish_reason(choice.get("finish_reason").and_then(Value::as_str)),
            reasoning: message
                .get("reasoning_content")
                .and_then(Value::as_str)
                .or_else(|| message.get("reasoning").and_then(Value::as_str))
                .map(str::to_owned),
            ..Default::default()
        };

        if let Some(tcs) = message.get("tool_calls") {
            if let Ok(parsed) = serde_json::from_value::<Vec<AiToolCall>>(tcs.clone()) {
                if !parsed.is_empty() {
                    result.tool_calls = parsed;
                    result.finish_reason = "tool_calls".into();
                }
            }
        }

        match body.get("usage") {
            Some(usage) => {
                let get = |k: &str| usage.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
                result.prompt_tokens = get("prompt_tokens");
                result.completion_tokens = get("completion_tokens");
                result.total_tokens = get("total_tokens");
                result.usage_source = "provider".into();
            }
            None => {
                let prompt = Self::prompt_tokens(request);
                result.prompt_tokens = prompt;
                result.completion_tokens = estimate_tokens(&result.content);
                result.total_tokens = prompt + result.completion_tokens;
                result.usage_source = "estimated".into();
            }
        }
        Ok(result)
    }

    /// 流式补全：SSE 逐行读取。`on_chunk` 在调用线程上同步触发。
    pub fn stream_complete(
        &self,
        request: &AiCompletionRequest,
        mut on_chunk: impl FnMut(AiStreamChunk),
    ) -> Result<AiCompletionResponse, AiError> {
        let response = self.fetch_with_retry(request, true)?;
        if self.provider.is_anthropic_messages() {
            let mut acc = super::anthropic::AnthropicSseAccumulator::new();
            let reader = BufReader::new(response.into_body().into_reader());
            'lines: for line in reader.lines() {
                if self.cancelled.load(Ordering::Relaxed) {
                    return Err(AiError::new("请求已取消", "CANCELLED"));
                }
                let line = line.map_err(|error| self.stream_body_error("流式响应读取", error))?;
                for chunk in acc.push_line(&line)? {
                    on_chunk(chunk);
                }
                if acc.has_completion_marker() {
                    break 'lines;
                }
            }
            if !acc.has_completion_marker() {
                return Err(AiError::new(
                    "Anthropic 流式响应提前结束：provider 未发送 message_stop",
                    "STREAM_ERROR",
                ));
            }
            let finish_reason = acc.finish_reason();
            let result = acc.finish(Self::prompt_tokens(request))?;
            on_chunk(AiStreamChunk {
                delta: Some(String::new()),
                reasoning_delta: None,
                finish_reason: Some(finish_reason),
            });
            return Ok(result);
        }
        let mut acc = SseAccumulator::new();
        let reader = BufReader::new(response.into_body().into_reader());

        for line in reader.lines() {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(AiError::new("请求已取消", "CANCELLED"));
            }
            let line = line.map_err(|error| self.stream_body_error("流式响应读取", error))?;
            for chunk in acc.push_line(&line) {
                on_chunk(chunk);
            }
        }

        if !acc.has_completion_marker() {
            return Err(AiError::new(
                "流式响应提前结束：provider 未发送完成标记",
                "STREAM_ERROR",
            ));
        }

        on_chunk(AiStreamChunk {
            delta: Some(String::new()),
            reasoning_delta: None,
            finish_reason: Some(acc.finish_reason()),
        });
        Ok(acc.finish(Self::prompt_tokens(request)))
    }

    /// 走提供方真实的 OpenAI 兼容 `GET /models` 端点枚举模型。
    /// 只暴露提供方返回的 id；刻意不做任何演示/兜底模型列表。
    pub fn list_models(&self) -> Result<Vec<String>, AiError> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(AiError::new("请求已取消", "CANCELLED"));
        }
        if self.provider.is_anthropic_messages() {
            return self.list_anthropic_models();
        }
        let url = models_url(&self.provider.base_url);
        let mut request = self.agent.get(&url).header("Accept", "application/json");
        let api_key = self.provider.api_key.trim();
        if !api_key.is_empty() {
            request = request.header("Authorization", &format!("Bearer {api_key}"));
        }
        let mut response = request.call().map_err(|error| {
            if self.cancelled.load(Ordering::Relaxed) {
                AiError::new("请求已取消", "CANCELLED")
            } else if matches!(&error, ureq::Error::Timeout(_)) {
                AiError::new(
                    "获取模型列表超时（provider 在配置时限内没有响应）",
                    "TIMEOUT",
                )
            } else {
                AiError::new(format!("获取模型列表失败：{error}"), "NETWORK_ERROR")
            }
        })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = match response.body_mut().read_to_string() {
                Ok(body) => body,
                Err(error) => return Err(self.body_error("模型列表错误响应读取", error)),
            };
            if matches!(status, 404 | 405 | 501) {
                return Err(AiError::new(
                    "该提供商不支持 OpenAI 兼容的 GET /models 模型枚举，请手动填写模型名",
                    "MODEL_LIST_UNSUPPORTED",
                ));
            }
            return Err(AiError::api(format!("模型列表请求失败：{body}"), status));
        }
        let body: Value = response.body_mut().read_json().map_err(|error| {
            if self.cancelled.load(Ordering::Relaxed) {
                AiError::new("请求已取消", "CANCELLED")
            } else if matches!(error, ureq::Error::Timeout(_)) {
                AiError::new(
                    "模型列表响应读取超时：provider 未在配置时限内返回完整 JSON",
                    "TIMEOUT",
                )
            } else {
                AiError::new(format!("模型列表响应不是合法 JSON：{error}"), "API_ERROR")
            }
        })?;
        let Some(models) = parse_model_ids(&body) else {
            return Err(AiError::new(
                "该提供商未返回可识别的模型列表（需要 data[].id 或 models[].id），请手动填写模型名",
                "MODEL_LIST_UNSUPPORTED",
            ));
        };
        if models.is_empty() {
            return Err(AiError::new(
                "提供商返回了空模型列表，请手动填写模型名",
                "MODEL_LIST_EMPTY",
            ));
        }
        Ok(models)
    }

    fn list_anthropic_models(&self) -> Result<Vec<String>, AiError> {
        let mut url = super::anthropic::models_url(&self.provider.base_url);
        let mut models = Vec::new();
        let mut seen_after = std::collections::HashSet::new();

        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return Err(AiError::new("请求已取消", "CANCELLED"));
            }
            let mut request = self.agent.get(&url).header("Accept", "application/json");
            let api_key = self.provider.api_key.trim();
            if !api_key.is_empty() {
                request = request.header("x-api-key", api_key);
            }
            request = request.header("anthropic-version", super::anthropic::API_VERSION);

            let mut response = request.call().map_err(|error| {
                if self.cancelled.load(Ordering::Relaxed) {
                    AiError::new("请求已取消", "CANCELLED")
                } else if matches!(&error, ureq::Error::Timeout(_)) {
                    AiError::new(
                        "获取 Anthropic 模型列表超时（provider 在配置时限内没有响应）",
                        "TIMEOUT",
                    )
                } else {
                    AiError::new(
                        format!("获取 Anthropic 模型列表失败：{error}"),
                        "NETWORK_ERROR",
                    )
                }
            })?;
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                let body = response
                    .body_mut()
                    .read_to_string()
                    .map_err(|error| self.body_error("Anthropic 模型列表错误响应读取", error))?;
                if matches!(status, 404 | 405 | 501) {
                    return Err(AiError::new(
                        "该 Anthropic 提供商不支持 GET /v1/models，请手动填写模型名",
                        "MODEL_LIST_UNSUPPORTED",
                    ));
                }
                return Err(AiError::api(
                    format!("Anthropic 模型列表请求失败：{body}"),
                    status,
                ));
            }
            let body: Value = response.body_mut().read_json().map_err(|error| {
                if self.cancelled.load(Ordering::Relaxed) {
                    AiError::new("请求已取消", "CANCELLED")
                } else if matches!(error, ureq::Error::Timeout(_)) {
                    AiError::new("Anthropic 模型列表响应读取超时", "TIMEOUT")
                } else {
                    AiError::new(
                        format!("Anthropic 模型列表响应不是合法 JSON：{error}"),
                        "API_ERROR",
                    )
                }
            })?;
            let Some(page) = parse_model_ids(&body) else {
                return Err(AiError::new(
                    "Anthropic 模型列表未返回可识别的 data[].id",
                    "MODEL_LIST_UNSUPPORTED",
                ));
            };
            for model in page {
                if !models.iter().any(|existing| existing == &model) {
                    models.push(model);
                }
            }

            let has_more = body
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !has_more {
                break;
            }
            let Some(last_id) = body
                .get("last_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
            else {
                return Err(AiError::new(
                    "Anthropic 模型列表分页缺少 last_id",
                    "MODEL_LIST_UNSUPPORTED",
                ));
            };
            if !seen_after.insert(last_id.to_owned()) {
                return Err(AiError::new(
                    "Anthropic 模型列表分页游标没有前进",
                    "MODEL_LIST_UNSUPPORTED",
                ));
            }
            url = super::anthropic::models_url_after(&self.provider.base_url, last_id);
        }

        if models.is_empty() {
            return Err(AiError::new(
                "Anthropic 提供商返回了空模型列表，请手动填写模型名",
                "MODEL_LIST_EMPTY",
            ));
        }
        Ok(models)
    }

    /// 根据服务方配置和是否提供回调，决定使用流式还是非流式响应。
    pub fn complete(
        &self,
        request: &AiCompletionRequest,
        on_chunk: Option<&mut dyn FnMut(AiStreamChunk)>,
    ) -> Result<AiCompletionResponse, AiError> {
        match on_chunk {
            Some(cb) if request.stream != Some(false) && self.provider.stream => {
                self.stream_complete(request, cb)
            }
            _ => self.complete_once(request),
        }
    }

    /// 测试连接（发一条最短消息）。
    pub fn test_connection(&self) -> (bool, Option<String>) {
        let request = AiCompletionRequest {
            messages: vec![AiMessage::new("user", "Hello")],
            max_tokens: Some(10),
            stream: Some(false),
            ..Default::default()
        };
        match self.complete_once(&request) {
            // 只有推理内容或工具调用的响应同样算有效的连通性响应；
            // 最终文本为空不算失败。
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        }
    }
}

/// 解析标准的 OpenAI/DeepSeek 模型列表信封。少数兼容网关用 `models`
/// 而不是 `data`；两种形态都要求提供方返回真实的字符串 id。
fn parse_model_ids(body: &Value) -> Option<Vec<String>> {
    let entries = body
        .get("data")
        .or_else(|| body.get("models"))
        .and_then(Value::as_array)?;
    let mut ids = Vec::new();
    for id in entries.iter().filter_map(|entry| {
        entry
            .as_str()
            .or_else(|| entry.get("id").and_then(Value::as_str))
            .or_else(|| entry.get("model").and_then(Value::as_str))
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    }) {
        if !ids.iter().any(|existing| existing == &id) {
            ids.push(id);
        }
    }
    Some(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_precede_images_and_text_and_never_serialize_as_metadata() {
        let mut message = AiMessage::new("user", "text");
        message.files.push(crate::ai::AiFile {
            filename: "中文.pdf".into(),
            file_data: "data:application/pdf;base64,YQ==".into(),
        });
        message.images.push("data:image/png;base64,Yg==".into());
        let request = AiCompletionRequest {
            messages: vec![message],
            ..Default::default()
        };
        let body = AiService::new(AiProvider::default())
            .build_payload(&request, false)
            .unwrap();
        let user = &body["messages"][0];
        assert_eq!(
            user["content"][0],
            json!({"type":"file","file":{"filename":"中文.pdf","file_data":"data:application/pdf;base64,YQ=="}})
        );
        assert_eq!(user["content"][1]["type"], "image_url");
        assert_eq!(user["content"][2]["text"], "text");
        assert!(user.get("files").is_none());
        assert!(user.get("images").is_none());
    }
    #[test]
    fn image_content_is_typed_and_plain_messages_keep_the_old_shape() {
        let mut image = AiMessage::new("user", "看图");
        image.images = vec!["data:image/png;base64,YQ==".into()];
        let request = AiCompletionRequest {
            messages: vec![AiMessage::new("system", "system"), image],
            ..Default::default()
        };
        let body = AiService::new(AiProvider::default())
            .build_payload(&request, false)
            .unwrap();
        assert_eq!(body["messages"][0]["content"], "system");
        assert_eq!(body["messages"][1]["content"][0]["type"], "image_url");
        assert_eq!(
            body["messages"][1]["content"][1],
            json!({"type":"text","text":"看图"})
        );
        assert!(body["messages"][1].get("images").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn chat_url_normalizes_user_pasted_endpoints() {
        // 用户可能只填 base，也可能把完整端点粘进来
        assert_eq!(
            chat_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://x.ai/v1/messages"),
            "https://x.ai/v1/chat/completions"
        );
        assert_eq!(
            chat_url("https://x.ai/v1/responses"),
            "https://x.ai/v1/chat/completions"
        );
        assert_eq!(
            models_url("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            models_url("https://api.openai.com/v1/models"),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn https_agent_uses_the_tls_provider_compiled_into_mochi() {
        let service = AiService::new(AiProvider::default());
        assert_eq!(
            service.agent.config().tls_config().provider(),
            ureq::tls::TlsProvider::NativeTls
        );
    }

    #[test]
    fn model_list_parser_only_returns_provider_ids_and_deduplicates() {
        let body = json!({
            "data": [
                {"id": "deepseek-chat"},
                {"id": " deepseek-reasoner "},
                {"id": "deepseek-chat"},
                {"owned_by": "provider"}
            ]
        });
        assert_eq!(
            parse_model_ids(&body).unwrap(),
            vec!["deepseek-chat", "deepseek-reasoner"]
        );
        assert_eq!(
            parse_model_ids(&json!({"models": ["local-a", {"model": "local-b"}]})).unwrap(),
            vec!["local-a", "local-b"]
        );
        assert!(parse_model_ids(&json!({"data": {}})).is_none());
    }

    #[test]
    fn model_list_uses_the_real_openai_compatible_transport() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0_u8; 512];
                let read = socket.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("GET /v1/models HTTP/1.1"), "{request}");
            assert!(
                request.contains("authorization: Bearer fixture-key"),
                "{request}"
            );
            let body = r#"{"object":"list","data":[{"id":"fixture-model","owned_by":"fixture"}]}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            socket.flush().unwrap();
        });
        let service = AiService::with_cancel_timeout(
            AiProvider {
                base_url: format!("http://{address}/v1"),
                api_key: "fixture-key".into(),
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
        );
        assert_eq!(service.list_models().unwrap(), vec!["fixture-model"]);
        server.join().unwrap();
    }

    #[test]
    fn empty_completion_content_does_not_make_connection_test_fail() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0_u8; 1024];
                let read = socket.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                let Some(headers_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    })
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + length {
                    break;
                }
            }
            assert!(
                String::from_utf8_lossy(&request).starts_with("POST /v1/chat/completions HTTP/1.1")
            );
            let body = r#"{"choices":[{"message":{"role":"assistant","content":""},"finish_reason":"stop"}]}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            socket.flush().unwrap();
        });
        let service = AiService::with_cancel_timeout(
            AiProvider {
                base_url: format!("http://{address}/v1"),
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
            Duration::from_secs(2),
        );
        assert_eq!(service.test_connection(), (true, None));
        server.join().unwrap();
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(map_finish_reason(Some("stop")), "stop");
        assert_eq!(map_finish_reason(Some("tool_calls")), "tool_calls");
        assert_eq!(map_finish_reason(Some("length")), "length");
        assert_eq!(map_finish_reason(Some("max_tokens")), "length");
        assert_eq!(map_finish_reason(Some("content_filter")), "error");
        assert_eq!(map_finish_reason(None), "error");
    }

    fn feed(acc: &mut SseAccumulator, lines: &[&str]) -> Vec<AiStreamChunk> {
        lines.iter().flat_map(|l| acc.push_line(l)).collect()
    }

    #[test]
    fn accumulates_content_deltas() {
        let mut acc = SseAccumulator::new();
        let chunks = feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"content":"你"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"好"}}]}"#,
                r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                "data: [DONE]",
            ],
        );
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].delta.as_deref(), Some("你"));

        let r = acc.finish(10);
        assert_eq!(r.content, "你好");
        assert_eq!(r.finish_reason, "stop");
        assert_eq!(r.prompt_tokens, 10);
        assert_eq!(r.total_tokens, 10 + r.completion_tokens);
    }

    #[test]
    fn ignores_blank_lines_done_marker_and_garbage() {
        let mut acc = SseAccumulator::new();
        let chunks = feed(
            &mut acc,
            &[
                "",
                "   ",
                ": keep-alive",
                "data: [DONE]",
                "data: {不是合法 JSON",
                "event: ping",
            ],
        );
        assert!(chunks.is_empty());
        assert_eq!(acc.finish(0).content, "");
    }

    /// 两个服务方使用了不同的字段名，都要识别。
    #[test]
    fn accepts_both_reasoning_spellings() {
        let mut a = SseAccumulator::new();
        feed(
            &mut a,
            &[r#"data: {"choices":[{"delta":{"reasoning_content":"想"}}]}"#],
        );
        assert_eq!(a.finish(0).reasoning.as_deref(), Some("想"));

        let mut b = SseAccumulator::new();
        feed(
            &mut b,
            &[r#"data: {"choices":[{"delta":{"reasoning":"想"}}]}"#],
        );
        assert_eq!(b.finish(0).reasoning.as_deref(), Some("想"));
    }

    #[test]
    fn reasoning_and_content_are_kept_separate() {
        let mut acc = SseAccumulator::new();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"reasoning_content":"思考"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"答案"}}]}"#,
            ],
        );
        let r = acc.finish(0);
        assert_eq!(r.content, "答案");
        assert_eq!(r.reasoning.as_deref(), Some("思考"));
    }

    /// 工具调用的参数是**跨多个分片**流过来的，必须按 index 拼接。
    #[test]
    fn reassembles_tool_call_arguments_across_chunks() {
        let mut acc = SseAccumulator::new();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"file_read","arguments":"{\"pa"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.md\"}"}}]}}]}"#,
            ],
        );
        let r = acc.finish(0);
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].id, "call_1");
        assert_eq!(r.tool_calls[0].function.name, "file_read");
        assert_eq!(r.tool_calls[0].function.arguments, r#"{"path":"a.md"}"#);
        assert_eq!(
            r.finish_reason, "tool_calls",
            "有工具调用时应覆盖 finish_reason"
        );
    }

    #[test]
    fn multiple_tool_calls_keep_provider_index_order() {
        let mut acc = SseAccumulator::new();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"second","arguments":"{}"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"first","arguments":"{}"}}]}}]}"#,
            ],
        );
        let names: Vec<String> = acc
            .finish(0)
            .tool_calls
            .into_iter()
            .map(|t| t.function.name)
            .collect();
        assert_eq!(names, ["first", "second"], "应按 index 而非到达顺序排列");
    }

    #[test]
    fn tool_call_without_arguments_gets_empty_object() {
        let mut acc = SseAccumulator::new();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"x","function":{"name":"noop"}}]}}]}"#,
            ],
        );
        assert_eq!(acc.finish(0).tool_calls[0].function.arguments, "{}");
    }

    /// 只有 id 没有 name 的分片是残缺的，不该产出工具调用。
    #[test]
    fn nameless_tool_call_chunks_are_dropped() {
        let mut acc = SseAccumulator::new();
        feed(
            &mut acc,
            &[r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"x"}]}}]}"#],
        );
        assert!(acc.finish(0).tool_calls.is_empty());
    }

    #[test]
    fn empty_choices_array_is_tolerated() {
        let mut acc = SseAccumulator::new();
        assert!(acc.push_line(r#"data: {"choices":[]}"#).is_empty());
        assert_eq!(acc.finish(0).content, "");
    }

    #[test]
    fn payload_includes_tools_only_when_present() {
        let provider = AiProvider {
            model: "gpt-4o".into(),
            ..Default::default()
        };
        let svc = AiService::new(provider);

        let bare = svc
            .build_payload(
                &AiCompletionRequest {
                    messages: vec![AiMessage::new("user", "hi")],
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert_eq!(bare["model"], "gpt-4o");
        assert_eq!(bare["temperature"], 0.7, "缺省温度应是 0.7");
        assert_eq!(bare["stream"], false);
        assert!(bare.get("tools").is_none());
        assert!(
            bare.get("max_tokens").is_none(),
            "未指定时不该发 max_tokens"
        );

        let with_tools = svc
            .build_payload(
                &AiCompletionRequest {
                    messages: vec![AiMessage::new("user", "hi")],
                    max_tokens: Some(1024),
                    tools: vec![AiToolDefinition::function("t", "d", json!({}))],
                    ..Default::default()
                },
                true,
            )
            .unwrap();
        assert_eq!(with_tools["stream"], true);
        assert_eq!(with_tools["max_tokens"], 1024);
        assert_eq!(with_tools["tool_choice"], "auto");
        assert_eq!(with_tools["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn zero_max_tokens_is_treated_as_unset() {
        let svc = AiService::new(AiProvider::default());
        let payload = svc
            .build_payload(
                &AiCompletionRequest {
                    max_tokens: Some(0),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        assert!(payload.get("max_tokens").is_none());
    }

    #[test]
    fn cancel_flag_round_trips() {
        let svc = AiService::new(AiProvider::default());
        assert!(!svc.cancelled.load(Ordering::Relaxed));
        svc.cancel();
        assert!(svc.cancelled.load(Ordering::Relaxed));
        svc.reset_cancel();
        assert!(!svc.cancelled.load(Ordering::Relaxed));
    }

    /// 取消后不该真的发出请求。
    #[test]
    fn cancelled_request_fails_fast_without_network() {
        let svc = AiService::new(AiProvider {
            base_url: "http://127.0.0.1:9/v1".into(),
            ..Default::default()
        });
        svc.cancel();
        let err = svc
            .complete_once(&AiCompletionRequest::default())
            .unwrap_err();
        assert_eq!(err.code, "CANCELLED");
    }

    #[test]
    fn cancelled_model_list_fails_fast_without_network() {
        let service = AiService::new(AiProvider {
            base_url: "http://127.0.0.1:9/v1".into(),
            ..Default::default()
        });
        service.cancel();
        let error = service.list_models().unwrap_err();
        assert_eq!(error.code, "CANCELLED");
    }
}
