//! AI 统一数据模型；OpenAI 与 Anthropic 在传输边界适配。对齐 `src/services/ai.ts`。
//!
//! 中转参考 `winui/src/Mochi.Core/AI/AIModels.cs`。

use serde::{Deserialize, Serialize};

fn is_none<T>(v: &Option<T>) -> bool {
    v.is_none()
}

pub const OPENAI_COMPLETIONS_PROTOCOL: &str = "openai-completions";
pub const ANTHROPIC_MESSAGES_PROTOCOL: &str = "anthropic-messages";

fn default_protocol() -> String {
    OPENAI_COMPLETIONS_PROTOCOL.into()
}

/// AI Provider 配置。`api_key` 是**明文**——加密存储由 `settings::SettingsService` 负责。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub stream: bool,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

impl Default for AiProvider {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            stream: false,
            protocol: default_protocol(),
        }
    }
}

impl AiProvider {
    /// protocol 为空或缺失都是旧版 OpenAI 兼容配置。未知值也回落到
    /// 同一种传输方式，格式错误的设置项就不会误用 Anthropic 鉴权。
    pub fn is_anthropic_messages(&self) -> bool {
        self.protocol.trim() == ANTHROPIC_MESSAGES_PROTOCOL
    }

    pub fn normalized_protocol(&self) -> &'static str {
        if self.is_anthropic_messages() {
            ANTHROPIC_MESSAGES_PROTOCOL
        } else {
            OPENAI_COMPLETIONS_PROTOCOL
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiToolFunction {
    pub name: String,
    /// 保留原始 JSON 字符串（服务方就是这样传递的，不要提前解析）
    pub arguments: String,
}

impl Default for AiToolFunction {
    fn default() -> Self {
        Self {
            name: String::new(),
            arguments: "{}".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: AiToolFunction,
}

impl Default for AiToolCall {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: "function".into(),
            function: AiToolFunction::default(),
        }
    }
}

/// 临时文件内容，不写入共享会话 JSON。
#[derive(Debug, Clone, PartialEq)]
pub struct AiFile {
    pub filename: String,
    pub file_data: String,
}

/// 发送给模型的对话消息。
///
/// `hidden` 是墨池内部字段（用于在 UI 中隐藏工具往返），**不会发给服务方**。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiMessage {
    /// 仅为本次请求准备的图片。绝不序列化进存储的历史。
    #[serde(skip)]
    pub images: Vec<String>,
    #[serde(skip)]
    pub files: Vec<AiFile>,
    pub role: String,
    #[serde(skip_serializing_if = "is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "is_none")]
    pub tool_calls: Option<Vec<AiToolCall>>,
    #[serde(skip_serializing_if = "is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing, default)]
    pub hidden: bool,
}

impl Default for AiMessage {
    fn default() -> Self {
        Self {
            role: "user".into(),
            images: Vec::new(),
            files: Vec::new(),
            content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            hidden: false,
        }
    }
}

impl AiMessage {
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: role.to_owned(),
            content: Some(content.to_owned()),
            ..Default::default()
        }
    }

    /// 工具执行结果消息。
    pub fn tool_result(tool_call_id: &str, name: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.to_owned()),
            tool_call_id: Some(tool_call_id.to_owned()),
            name: Some(name.to_owned()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiToolFunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: AiToolFunctionDef,
}

impl AiToolDefinition {
    pub fn function(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self {
            kind: "function".into(),
            function: AiToolFunctionDef {
                name: name.to_owned(),
                description: description.to_owned(),
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AiCompletionRequest {
    pub messages: Vec<AiMessage>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i32>,
    pub stream: Option<bool>,
    pub tools: Vec<AiToolDefinition>,
    pub tool_choice: Option<String>,
}

/// SSE 流式分片。
#[derive(Debug, Clone, PartialEq)]
pub struct AiStreamChunk {
    pub delta: Option<String>,
    pub reasoning_delta: Option<String>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiCompletionResponse {
    pub content: String,
    /// `stop` | `tool_calls` | `length` | `error`
    pub finish_reason: String,
    pub tool_calls: Vec<AiToolCall>,
    pub reasoning: Option<String>,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
    /// `provider` | `estimated`
    pub usage_source: String,
}

impl Default for AiCompletionResponse {
    fn default() -> Self {
        Self {
            content: String::new(),
            finish_reason: "stop".into(),
            tool_calls: Vec::new(),
            reasoning: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            usage_source: "estimated".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiError {
    pub message: String,
    /// `API_ERROR` | `TIMEOUT` | `NETWORK_ERROR` | `CANCELLED`
    pub code: String,
    pub status_code: Option<u16>,
}

impl AiError {
    pub fn new(message: impl Into<String>, code: &str) -> Self {
        Self {
            message: message.into(),
            code: code.into(),
            status_code: None,
        }
    }
    pub fn api(message: impl Into<String>, status: u16) -> Self {
        Self {
            message: message.into(),
            code: "API_ERROR".into(),
            status_code: Some(status),
        }
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status_code {
            Some(s) => write!(f, "[{}:{s}] {}", self.code, self.message),
            None => write!(f, "[{}] {}", self.code, self.message),
        }
    }
}

impl std::error::Error for AiError {}

/// Token 数估算：每个中日韩文字按 0.75 个 Token 计算，其余字符每 4 个约为 1 个 Token。
///
/// **只把 `U+3400..=U+9FFF` 算作 CJK**，与 `src/services/ai.ts` 的 `/[㐀-鿿]/` 一致。
/// C# 版额外把假名（`U+3040..=U+30FF`）和谚文（`U+AC00..=U+D7AF`）也算进去了，
/// 日韩文本的估算会偏高（实测 `こんにちは` TS 给 2，C# 给 4）。
///
/// 长度用 **UTF-16 码元**计（JS 的 `String.length`）。
pub fn estimate_tokens(content: &str) -> i32 {
    if content.is_empty() {
        return 0;
    }
    let cjk = content
        .chars()
        .filter(|c| ('\u{3400}'..='\u{9FFF}').contains(c))
        .count();
    let total = content.encode_utf16().count();
    let other = total.saturating_sub(cjk);
    let estimate = (cjk as f64 * 0.75 + other as f64 / 4.0).ceil() as i32;
    estimate.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 期望值由 node 跑 `src/services/ai.ts` 的 `estimateTokens` 得出。
    #[test]
    fn token_estimates_match_ts() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("hello world"), 3);
        assert_eq!(estimate_tokens("你好世界"), 3);
        assert_eq!(estimate_tokens("墨池是一个 AI 原生笔记应用"), 10);
        assert_eq!(estimate_tokens(&"x".repeat(100)), 25);
    }

    /// 假名/谚文不算 CJK——C# 版在这里失配。
    #[test]
    fn kana_and_hangul_are_not_counted_as_cjk() {
        assert_eq!(estimate_tokens("こんにちは"), 2, "假名被当成 CJK 了");
        assert_eq!(estimate_tokens("안녕하세요"), 2, "谚文被当成 CJK 了");
    }

    #[test]
    fn message_omits_empty_optional_fields() {
        let json = serde_json::to_string(&AiMessage::new("user", "hi")).unwrap();
        assert_eq!(json, r#"{"role":"user","content":"hi"}"#);
    }

    /// `hidden` 是内部字段，绝不能出现在发给服务方的报文中。
    #[test]
    fn hidden_flag_is_never_serialized() {
        let msg = AiMessage {
            hidden: true,
            ..AiMessage::new("user", "hi")
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("hidden"), "{json}");
    }

    #[test]
    fn tool_result_message_shape() {
        let json =
            serde_json::to_string(&AiMessage::tool_result("call_1", "file_read", "ok")).unwrap();
        assert!(json.contains(r#""role":"tool""#));
        assert!(json.contains(r#""tool_call_id":"call_1""#));
        assert!(json.contains(r#""name":"file_read""#));
    }

    #[test]
    fn tool_call_uses_type_not_kind() {
        let json = serde_json::to_string(&AiToolCall::default()).unwrap();
        assert!(json.contains(r#""type":"function""#), "{json}");
        assert!(!json.contains("kind"), "泄漏了 Rust 侧字段名: {json}");
    }

    #[test]
    fn tool_definition_shape() {
        let def = AiToolDefinition::function(
            "file_read",
            "读取文件",
            serde_json::json!({"type": "object", "properties": {}}),
        );
        let json = serde_json::to_string(&def).unwrap();
        assert!(
            json.starts_with(r#"{"type":"function","function":{"name":"file_read""#),
            "{json}"
        );
    }

    #[test]
    fn error_display_includes_code_and_status() {
        assert_eq!(
            AiError::api("boom", 500).to_string(),
            "[API_ERROR:500] boom"
        );
        assert_eq!(
            AiError::new("timeout", "TIMEOUT").to_string(),
            "[TIMEOUT] timeout"
        );
    }

    #[test]
    fn provider_protocol_defaults_to_openai_for_missing_and_empty_values() {
        let missing: AiProvider = serde_json::from_str(
            r#"{"id":"legacy","baseUrl":"https://example.invalid/v1","model":"m"}"#,
        )
        .unwrap();
        assert_eq!(missing.protocol, OPENAI_COMPLETIONS_PROTOCOL);

        let empty: AiProvider = serde_json::from_str(
            r#"{"id":"legacy","baseUrl":"https://example.invalid/v1","model":"m","protocol":""}"#,
        )
        .unwrap();
        assert_eq!(empty.normalized_protocol(), OPENAI_COMPLETIONS_PROTOCOL);

        let anthropic: AiProvider =
            serde_json::from_str(r#"{"protocol":"anthropic-messages","model":"claude"}"#).unwrap();
        assert!(anthropic.is_anthropic_messages());
    }
}
