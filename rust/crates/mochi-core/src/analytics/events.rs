//! JSONL 文件按本地月份命名，事件时间戳存 UTC。事件名使用 snake_case。
//! 可选字段省略后，剩余字段仍按固定声明序写入。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{jstime, paths};

/// 只保留最近这么多个月的文件参与读取，防止历史越攒越多拖慢首页。
const MAX_ACTIVITY_MONTHS: usize = 24;

pub const ACTIVITY_DIR: &str = "activity";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEventType {
    HomeView,
    FileOpen,
    FileCreate,
    FileSave,
    Search,
    ScheduleOpen,
    AiSessionOpen,
    AiMessage,
    FocusSession,
    TaskComplete,
}

impl ActivityEventType {
    /// 活动热力图的权重。写作类事件比浏览类事件更能代表「今天干了活」。
    pub fn weight(self) -> i64 {
        match self {
            Self::FocusSession => 5,
            Self::FileCreate => 4,
            Self::FileSave | Self::TaskComplete => 3,
            Self::AiMessage => 2,
            _ => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub prompt_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completion_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_tokens: Option<i64>,
}

impl TokenUsage {
    pub fn total(&self) -> i64 {
        self.total_tokens.unwrap_or(0)
    }

    /// 归一化：负数抬到 0，缺失的 total 由 prompt + completion 补齐。
    /// 三项全 0 视为「没有用量信息」，返回 `None`——否则会在图表上画出一堆零柱。
    pub fn normalize(raw: &TokenUsage) -> Option<TokenUsage> {
        let prompt = raw.prompt_tokens.unwrap_or(0).max(0);
        let completion = raw.completion_tokens.unwrap_or(0).max(0);
        let total = raw.total_tokens.unwrap_or(prompt + completion).max(0);
        if prompt == 0 && completion == 0 && total == 0 {
            return None;
        }
        Some(TokenUsage {
            prompt_tokens: Some(prompt),
            completion_tokens: Some(completion),
            total_tokens: Some(total),
        })
    }
}

/// 落盘的事件记录。**字段声明序即磁盘上的键序**，不要重排。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub version: u8,
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ActivityEventType,
    /// UTC ISO8601（`toISOString()` 形状）。
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub relative_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub extension: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage: Option<TokenUsage>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub usage_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub words: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub words_delta: Option<i64>,
    /// 分钟，保留两位小数。整数值必须写成 `5` 而不是 `5.0`——JS 那边就是整数。
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        serialize_with = "serialize_js_number"
    )]
    pub duration_minutes: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub count: Option<i64>,
}

/// 整数值序列化成整数字面量。`serde_json` 默认把 `5.0f64` 写成 `5.0`，
/// 而 `JSON.stringify(5)` 是 `5`——一个字节的差别就够让两版互相改写文件。
fn serialize_js_number<S: serde::Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) if v.is_finite() && v.fract() == 0.0 => serializer.serialize_i64(*v as i64),
        Some(v) => serializer.serialize_f64(*v),
        None => serializer.serialize_none(),
    }
}

impl ActivityEvent {
    pub fn timestamp_millis(&self) -> Option<i64> {
        jstime::try_parse_millis(&self.timestamp)
    }

    pub fn words_delta_or_zero(&self) -> i64 {
        self.words_delta.unwrap_or(0)
    }
}

/// 记录一次活动所需的入参。对应 TS 的 `ActivityEventInput`。
#[derive(Debug, Clone, Default)]
pub struct ActivityInput {
    pub kind: Option<ActivityEventType>,
    /// 毫秒时间戳；缺失或非法时用当前时间。
    pub timestamp: Option<i64>,
    /// 绝对或相对路径；工作区外的路径会让整条事件作废。
    pub path: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub role: Option<String>,
    pub model: Option<String>,
    pub provider_id: Option<String>,
    pub usage: Option<TokenUsage>,
    pub usage_source: Option<String>,
    pub words: Option<f64>,
    pub words_delta: Option<f64>,
    pub duration_minutes: Option<f64>,
    pub count: Option<f64>,
}

/// JS 的 `Math.round`：**遇 .5 向上取整**（朝 +∞），不是 Rust 的「远离零」。
/// `Math.round(-0.5)` 是 `-0`，而 `(-0.5f64).round()` 是 `-1`。
/// `wordsDelta` 允许负值，舍入方向影响删除字数统计。
pub fn js_round(value: f64) -> i64 {
    (value + 0.5).floor() as i64
}

/// 把输入归一化成可落盘的记录；不合法返回 `None`。
///
/// `random_suffix` 注入是为了让测试能断言完整的落盘字节——
/// 上游用 `Math.random().toString(36).slice(2, 10)`，8 位 base36。
pub fn normalize_event(
    workspace: &Path,
    input: &ActivityInput,
    now_millis: i64,
    random_suffix: &str,
) -> Option<ActivityEvent> {
    let kind = input.kind?;

    let millis = input.timestamp.filter(|_| true).unwrap_or(now_millis);
    let timestamp = jstime::from_millis(millis).unwrap_or_else(jstime::now);

    let relative_path = normalize_relative_path(workspace, input.path.as_deref());
    // 给了路径却解析不出相对路径 = 指向工作区外，整条事件作废。
    // 但形如 `mochi://…` 的内部 URL 例外：它本来就不是文件路径。
    if let Some(raw) = input.path.as_deref() {
        if relative_path.is_none() && !raw.contains("://") {
            return None;
        }
    }

    let extension = relative_path.as_deref().map(|p| {
        let ext = Path::new(p)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        if ext.is_empty() {
            "none".to_owned()
        } else {
            ext
        }
    });

    let usage = input.usage.as_ref().and_then(TokenUsage::normalize);
    let usage_source = usage.as_ref().map(|_| {
        input
            .usage_source
            .clone()
            .unwrap_or_else(|| "provider".to_owned())
    });

    Some(ActivityEvent {
        version: 1,
        id: format!("{millis}-{random_suffix}"),
        kind,
        timestamp,
        relative_path,
        extension,
        session_id: input.session_id.clone(),
        agent_id: input.agent_id.clone(),
        role: input.role.clone(),
        model: input.model.clone(),
        provider_id: input.provider_id.clone(),
        usage,
        usage_source,
        words: finite(input.words).map(|w| js_round(w).max(0)),
        words_delta: finite(input.words_delta).map(js_round),
        // 保留两位小数后再夹到非负
        duration_minutes: finite(input.duration_minutes)
            .map(|d| (js_round(d * 100.0) as f64 / 100.0).max(0.0)),
        count: finite(input.count).map(|c| js_round(c).max(1)),
    })
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite())
}

/// 路径归一化成工作区内的正斜杠相对路径。工作区外、含 `://` 的都返回 `None`。
pub fn normalize_relative_path(workspace: &Path, input: Option<&str>) -> Option<String> {
    let raw = input.filter(|p| !p.is_empty())?;
    if raw.contains("://") {
        return None;
    }

    let root = paths::to_forward_slashes(&workspace.to_string_lossy());
    let root = root.trim_end_matches('/');
    let candidate = paths::to_forward_slashes(raw);

    let absolute = if is_absolute(&candidate) {
        candidate
    } else {
        format!("{root}/{}", candidate.trim_start_matches('/'))
    };
    let absolute = collapse_dot_segments(&absolute);

    // 大小写敏感地比：`path.relative` 在 Windows 上确实不区分大小写，
    // 但活动日志里的相对路径要能和文件树的路径直接对上，混用大小写只会制造两条记录。
    if absolute == root {
        return None; // 工作区根本身不是一篇文档
    }
    let prefix = format!("{root}/");
    let relative = absolute.strip_prefix(&prefix)?;
    if relative.is_empty() {
        return None;
    }
    Some(relative.to_owned())
}

fn is_absolute(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// 处理 `.` 与 `..`，等价于 `path.resolve` 的规范化部分。
/// 越过根的 `..` 会被丢弃，随后前缀判断自然失败，路径被拒。
fn collapse_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let leading_slash = path.starts_with('/');
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    let joined = out.join("/");
    if leading_slash {
        format!("/{joined}")
    } else {
        joined
    }
}

fn activity_dir(workspace: &Path) -> PathBuf {
    paths::mochi_dir(workspace).join(ACTIVITY_DIR)
}

/// 本地月份的文件名。**本地**是关键，见模块头。
pub fn month_key(local: chrono::DateTime<chrono::Local>) -> String {
    use chrono::Datelike;
    format!("{}-{:02}", local.year(), local.month())
}

/// 追加一条活动记录。返回是否写成功。
///
/// 记录失败时返回 `false`，不向上传播错误，避免统计失败中断文件保存。
pub fn record(workspace: &Path, event: &ActivityEvent) -> bool {
    use std::io::Write;

    let Some(millis) = event.timestamp_millis() else {
        return false;
    };
    let local = chrono::DateTime::from_timestamp_millis(millis)
        .map(|utc| utc.with_timezone(&chrono::Local));
    let Some(local) = local else { return false };

    let dir = activity_dir(workspace);
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let Ok(line) = serde_json::to_string(event) else {
        return false;
    };
    let target = dir.join(format!("{}.jsonl", month_key(local)));
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)
    else {
        return false;
    };
    writeln!(file, "{line}").is_ok()
}

/// 归一化并落盘。
pub fn record_input(workspace: &Path, input: &ActivityInput) -> bool {
    if !workspace.is_dir() {
        return false;
    }
    let now = jstime::now_millis();
    let Some(event) = normalize_event(workspace, input, now, &paths::random_base36(8)) else {
        return false;
    };
    record(workspace, &event)
}

/// range 内涉及的月份文件名，最多 `MAX_ACTIVITY_MONTHS` 个。
pub fn months_in_range(
    start: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> Vec<String> {
    use chrono::Datelike;
    let mut out = Vec::new();
    let (mut year, mut month) = (start.year(), start.month());
    let (end_year, end_month) = (now.year(), now.month());

    while (year, month) <= (end_year, end_month) && out.len() < MAX_ACTIVITY_MONTHS {
        out.push(format!("{year}-{month:02}"));
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    out
}

/// 读出 `[start, now]` 区间内的事件，按时间升序。
///
/// 损坏的行、未知的事件类型、区间外的时间戳都被跳过——一行坏数据不该让整张首页空掉。
pub fn read_events(
    workspace: &Path,
    start: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> Vec<ActivityEvent> {
    let dir = activity_dir(workspace);
    let (start_ms, now_ms) = (start.timestamp_millis(), now.timestamp_millis());
    let mut events: Vec<(i64, ActivityEvent)> = Vec::new();

    for key in months_in_range(start, now) {
        let Ok(content) = std::fs::read_to_string(dir.join(format!("{key}.jsonl"))) else {
            continue;
        };
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<ActivityEvent>(line) else {
                continue;
            };
            let Some(millis) = event.timestamp_millis() else {
                continue;
            };
            if millis < start_ms || millis > now_ms {
                continue;
            }
            events.push((millis, event));
        }
    }

    events.sort_by_key(|(millis, _)| *millis);
    events.into_iter().map(|(_, event)| event).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Ws(PathBuf);
    impl Drop for Ws {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Ws {
        fn file(&self, key: &str) -> PathBuf {
            activity_dir(&self.0).join(format!("{key}.jsonl"))
        }
        fn write(&self, key: &str, body: &str) {
            std::fs::create_dir_all(activity_dir(&self.0)).unwrap();
            std::fs::write(self.file(key), body).unwrap();
        }
    }

    fn ws(tag: &str) -> Ws {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-activity-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Ws(root)
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn input(kind: ActivityEventType) -> ActivityInput {
        ActivityInput {
            kind: Some(kind),
            ..Default::default()
        }
    }

    // ---------- 类型与权重 ----------

    /// 事件类型必须是 snake_case——C# 版写成了 PascalCase，两版日志互不认识。
    #[test]
    fn event_types_serialize_as_snake_case() {
        for (kind, expected) in [
            (ActivityEventType::HomeView, "\"home_view\""),
            (ActivityEventType::FileSave, "\"file_save\""),
            (ActivityEventType::AiSessionOpen, "\"ai_session_open\""),
            (ActivityEventType::TaskComplete, "\"task_complete\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), expected);
        }
    }

    #[test]
    fn weights_favour_producing_over_browsing() {
        assert_eq!(ActivityEventType::FocusSession.weight(), 5);
        assert_eq!(ActivityEventType::FileCreate.weight(), 4);
        assert_eq!(ActivityEventType::FileSave.weight(), 3);
        assert_eq!(ActivityEventType::TaskComplete.weight(), 3);
        assert_eq!(ActivityEventType::AiMessage.weight(), 2);
        assert_eq!(ActivityEventType::HomeView.weight(), 1);
        assert_eq!(ActivityEventType::Search.weight(), 1);
    }

    // ---------- 落盘形状 ----------

    /// 取自真实工作区的三行，逐字节复现。
    #[test]
    fn records_match_the_bytes_of_real_events() {
        let w = ws("bytes");
        let cases = [
            (
                ActivityInput {
                    timestamp: Some(1_783_181_688_184),
                    ..input(ActivityEventType::HomeView)
                },
                "0nrabbrr",
                r#"{"version":1,"id":"1783181688184-0nrabbrr","type":"home_view","timestamp":"2026-07-04T16:14:48.184Z"}"#,
            ),
            (
                ActivityInput {
                    timestamp: Some(1_785_522_860_734),
                    path: Some("知识库/Personal Experience/2026年暑.mc".into()),
                    ..input(ActivityEventType::FileCreate)
                },
                "7d5smmon",
                r#"{"version":1,"id":"1785522860734-7d5smmon","type":"file_create","timestamp":"2026-07-31T18:34:20.734Z","relativePath":"知识库/Personal Experience/2026年暑.mc","extension":".mc"}"#,
            ),
            (
                ActivityInput {
                    timestamp: Some(1_785_522_894_477),
                    path: Some("知识库/a.mc".into()),
                    words: Some(120.0),
                    words_delta: Some(-8.0),
                    ..input(ActivityEventType::FileSave)
                },
                "odpc3o6c",
                r#"{"version":1,"id":"1785522894477-odpc3o6c","type":"file_save","timestamp":"2026-07-31T18:34:54.477Z","relativePath":"知识库/a.mc","extension":".mc","words":120,"wordsDelta":-8}"#,
            ),
        ];

        for (input, suffix, expected) in cases {
            let event = normalize_event(&w.0, &input, 0, suffix).unwrap();
            assert_eq!(serde_json::to_string(&event).unwrap(), expected);
        }
    }

    #[test]
    fn ai_message_events_carry_usage_and_provider_fields() {
        let w = ws("ai-shape");
        let event = normalize_event(
            &w.0,
            &ActivityInput {
                timestamp: Some(1_785_000_000_000),
                session_id: Some("s1".into()),
                agent_id: Some("general".into()),
                role: Some("assistant".into()),
                model: Some("gpt-4o".into()),
                provider_id: Some("openai".into()),
                usage: Some(TokenUsage {
                    prompt_tokens: Some(100),
                    completion_tokens: Some(50),
                    total_tokens: None,
                }),
                ..input(ActivityEventType::AiMessage)
            },
            0,
            "abcdefgh",
        )
        .unwrap();

        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains(r#""sessionId":"s1","agentId":"general","role":"assistant","model":"gpt-4o","providerId":"openai""#),
            "{json}"
        );
        assert!(
            json.contains(
                r#""usage":{"promptTokens":100,"completionTokens":50,"totalTokens":150}"#
            ),
            "缺失的 total 要由 prompt+completion 补齐: {json}"
        );
        assert!(
            json.contains(r#""usageSource":"provider""#),
            "有用量就默认标 provider"
        );
    }

    /// 整数分钟必须写成 `5` 而不是 `5.0`——JS 那边就是整数。
    #[test]
    fn whole_minute_durations_serialize_as_integers() {
        let w = ws("duration");
        let render = |minutes: f64| {
            let event = normalize_event(
                &w.0,
                &ActivityInput {
                    duration_minutes: Some(minutes),
                    ..input(ActivityEventType::FocusSession)
                },
                0,
                "x",
            )
            .unwrap();
            serde_json::to_string(&event).unwrap()
        };
        assert!(render(25.0).contains(r#""durationMinutes":25"#));
        assert!(render(25.5).contains(r#""durationMinutes":25.5"#));
        assert!(
            render(25.456).contains(r#""durationMinutes":25.46"#),
            "保留两位"
        );
        assert!(
            render(-3.0).contains(r#""durationMinutes":0"#),
            "负时长夹到 0"
        );
    }

    #[test]
    fn absent_optional_fields_are_omitted_entirely() {
        let w = ws("omit");
        let event = normalize_event(&w.0, &input(ActivityEventType::Search), 1_000, "s").unwrap();
        let json = serde_json::to_string(&event).unwrap();
        for key in [
            "relativePath",
            "extension",
            "sessionId",
            "usage",
            "words",
            "count",
        ] {
            assert!(!json.contains(key), "{key} 不该出现: {json}");
        }
    }

    // ---------- 归一化 ----------

    #[test]
    fn paths_become_workspace_relative_with_forward_slashes() {
        let w = ws("paths");
        let root = paths::to_forward_slashes(&w.0.to_string_lossy());

        let relative = |p: &str| {
            normalize_event(
                &w.0,
                &ActivityInput {
                    path: Some(p.into()),
                    ..input(ActivityEventType::FileOpen)
                },
                0,
                "x",
            )
            .map(|e| e.relative_path.unwrap_or_default())
        };

        assert_eq!(relative("知识库/a.mc").as_deref(), Some("知识库/a.mc"));
        assert_eq!(
            relative("知识库\\a.mc").as_deref(),
            Some("知识库/a.mc"),
            "反斜杠要归一"
        );
        assert_eq!(
            relative(&format!("{root}/知识库/a.mc")).as_deref(),
            Some("知识库/a.mc")
        );
        assert_eq!(relative("./知识库/a.mc").as_deref(), Some("知识库/a.mc"));
        assert_eq!(
            relative("知识库/子/../a.mc").as_deref(),
            Some("知识库/a.mc")
        );
    }

    /// 工作区外的路径让整条事件作废——记下去只会污染统计。
    #[test]
    fn a_path_outside_the_workspace_voids_the_event() {
        let w = ws("outside");
        for evil in ["../别处/a.mc", "C:/Windows/win.ini", "D:/另一个工作区/a.mc"] {
            let event = normalize_event(
                &w.0,
                &ActivityInput {
                    path: Some(evil.into()),
                    ..input(ActivityEventType::FileOpen)
                },
                0,
                "x",
            );
            assert!(event.is_none(), "{evil} 应作废");
        }
    }

    /// `mochi://` 这类内部 URL 不是文件路径，事件本身仍然有效，只是没有 relativePath。
    #[test]
    fn internal_urls_are_kept_as_events_without_a_path() {
        let w = ws("url");
        let event = normalize_event(
            &w.0,
            &ActivityInput {
                path: Some("mochi://ai-locate/x".into()),
                ..input(ActivityEventType::FileOpen)
            },
            0,
            "x",
        )
        .unwrap();
        assert!(event.relative_path.is_none());
        assert!(event.extension.is_none());
    }

    #[test]
    fn extensions_are_lowercased_and_default_to_none() {
        let w = ws("ext");
        let ext = |p: &str| {
            normalize_event(
                &w.0,
                &ActivityInput {
                    path: Some(p.into()),
                    ..input(ActivityEventType::FileOpen)
                },
                0,
                "x",
            )
            .unwrap()
            .extension
        };
        assert_eq!(ext("a.MC").as_deref(), Some(".mc"));
        assert_eq!(ext("README").as_deref(), Some("none"), "无扩展名记 none");
    }

    #[test]
    fn usage_that_is_all_zero_is_treated_as_absent() {
        assert!(TokenUsage::normalize(&TokenUsage::default()).is_none());
        assert!(TokenUsage::normalize(&TokenUsage {
            prompt_tokens: Some(0),
            completion_tokens: Some(0),
            total_tokens: Some(0),
        })
        .is_none());
        let normalized = TokenUsage::normalize(&TokenUsage {
            prompt_tokens: Some(-5),
            completion_tokens: Some(3),
            total_tokens: None,
        })
        .unwrap();
        assert_eq!(normalized.prompt_tokens, Some(0), "负数抬到 0");
        assert_eq!(normalized.total_tokens, Some(3));
    }

    #[test]
    fn counts_and_words_are_clamped() {
        let w = ws("clamp");
        let event = normalize_event(
            &w.0,
            &ActivityInput {
                words: Some(-10.0),
                words_delta: Some(-10.0),
                count: Some(0.0),
                ..input(ActivityEventType::TaskComplete)
            },
            0,
            "x",
        )
        .unwrap();
        assert_eq!(event.words, Some(0), "字数不能为负");
        assert_eq!(event.words_delta, Some(-10), "但增量可以为负");
        assert_eq!(event.count, Some(1), "次数至少 1");
    }

    /// JS 的 Math.round 遇 .5 朝 +∞，不是「远离零」。wordsDelta 会取负值。
    #[test]
    fn rounding_matches_js_semantics() {
        assert_eq!(js_round(0.5), 1);
        assert_eq!(js_round(1.5), 2);
        assert_eq!(js_round(-0.5), 0, "JS 给 -0，不是 -1");
        assert_eq!(js_round(-1.5), -1, "JS 给 -1，Rust 的 round 会给 -2");
        assert_eq!(js_round(-2.5), -2);
        assert_eq!(js_round(2.4), 2);
    }

    #[test]
    fn a_non_finite_number_is_dropped_not_recorded_as_zero() {
        let w = ws("nan");
        let event = normalize_event(
            &w.0,
            &ActivityInput {
                words: Some(f64::NAN),
                duration_minutes: Some(f64::INFINITY),
                ..input(ActivityEventType::FileSave)
            },
            0,
            "x",
        )
        .unwrap();
        assert!(event.words.is_none());
        assert!(event.duration_minutes.is_none());
    }

    // ---------- 月份文件 ----------

    /// 文件名按本地月份、时间戳按 UTC，两者会错开——真实数据即如此。
    #[test]
    fn the_file_is_named_by_local_month_while_the_timestamp_stays_utc() {
        let w = ws("month");
        // 本地 2026-08-01 02:34（东八区）= UTC 2026-07-31 18:34
        let local = at(2026, 8, 1, 2);
        let event = normalize_event(
            &w.0,
            &ActivityInput {
                timestamp: Some(local.timestamp_millis()),
                ..input(ActivityEventType::HomeView)
            },
            0,
            "x",
        )
        .unwrap();

        assert!(record(&w.0, &event));
        let expected = w.file(&month_key(local));
        assert!(
            expected.exists(),
            "应写进本地月份的文件: {}",
            expected.display()
        );

        let raw = std::fs::read_to_string(expected).unwrap();
        assert!(raw.ends_with('\n'), "JSONL 每行都要有换行");
        let written: ActivityEvent = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(written.timestamp_millis(), Some(local.timestamp_millis()));
    }

    #[test]
    fn months_in_range_covers_year_boundaries_and_is_capped() {
        assert_eq!(
            months_in_range(at(2026, 11, 1, 0), at(2027, 2, 1, 0)),
            ["2026-11", "2026-12", "2027-01", "2027-02"]
        );
        assert_eq!(
            months_in_range(at(2026, 8, 1, 0), at(2026, 8, 31, 0)),
            ["2026-08"]
        );
        assert_eq!(
            months_in_range(at(2000, 1, 1, 0), at(2026, 1, 1, 0)).len(),
            MAX_ACTIVITY_MONTHS,
            "历史再长也只读最近 24 个月"
        );
    }

    // ---------- 读取 ----------

    #[test]
    fn events_are_read_sorted_and_filtered_to_the_range() {
        let w = ws("read");
        let day = |d: u32| at(2026, 8, d, 12);
        let line = |d: u32, kind: &str| {
            format!(
                r#"{{"version":1,"id":"{d}-x","type":"{kind}","timestamp":"{}"}}"#,
                jstime::from_millis(day(d).timestamp_millis()).unwrap()
            )
        };
        // 故意乱序写入
        w.write(
            "2026-08",
            &format!(
                "{}\n{}\n{}\n",
                line(20, "file_save"),
                line(10, "home_view"),
                line(15, "search")
            ),
        );

        let events = read_events(&w.0, day(12), day(25));
        assert_eq!(events.len(), 2, "区间外的 8/10 应被排除");
        assert_eq!(events[0].kind, ActivityEventType::Search, "要按时间升序");
        assert_eq!(events[1].kind, ActivityEventType::FileSave);
    }

    /// 一行坏数据不该让整张首页空掉。
    #[test]
    fn corrupt_lines_and_unknown_types_are_skipped() {
        let w = ws("corrupt");
        let ts = jstime::from_millis(at(2026, 8, 15, 12).timestamp_millis()).unwrap();
        w.write(
            "2026-08",
            &format!(
                "{{不是 JSON\n\n   \n{{\"version\":1,\"id\":\"a\",\"type\":\"未来的新类型\",\"timestamp\":\"{ts}\"}}\n{{\"version\":1,\"id\":\"b\",\"type\":\"file_save\",\"timestamp\":\"{ts}\"}}\n"
            ),
        );

        let events = read_events(&w.0, at(2026, 8, 1, 0), at(2026, 8, 31, 0));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "b");
    }

    #[test]
    fn a_missing_activity_directory_reads_as_empty() {
        let w = ws("empty");
        assert!(read_events(&w.0, at(2026, 8, 1, 0), at(2026, 8, 31, 0)).is_empty());
    }

    /// 写进去的能原样读回来，包括全部可选字段。
    #[test]
    fn a_recorded_event_round_trips() {
        let w = ws("roundtrip");
        let now = at(2026, 8, 15, 12);
        let event = normalize_event(
            &w.0,
            &ActivityInput {
                timestamp: Some(now.timestamp_millis()),
                path: Some("知识库/a.mc".into()),
                words: Some(500.0),
                words_delta: Some(-12.0),
                count: Some(3.0),
                duration_minutes: Some(25.5),
                usage: Some(TokenUsage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(20),
                    total_tokens: None,
                }),
                usage_source: Some("estimated".into()),
                ..input(ActivityEventType::FileSave)
            },
            0,
            "abcdefgh",
        )
        .unwrap();
        assert!(record(&w.0, &event));

        let read = read_events(&w.0, at(2026, 8, 1, 0), at(2026, 8, 31, 0));
        assert_eq!(read.len(), 1);
        assert_eq!(read[0], event, "读回来的必须与写进去的完全相同");
    }

    #[test]
    fn recording_into_a_missing_workspace_fails_quietly() {
        let missing = std::env::temp_dir().join("mochi-activity-绝不存在的目录");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(!record_input(&missing, &input(ActivityEventType::HomeView)));
    }

    #[test]
    fn an_event_without_a_type_is_refused() {
        let w = ws("no-type");
        assert!(normalize_event(&w.0, &ActivityInput::default(), 0, "x").is_none());
        assert!(!record_input(&w.0, &ActivityInput::default()));
    }
}
