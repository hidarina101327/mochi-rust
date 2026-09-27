//! 调用方检查 memoryAutoExtract。提取失败不影响主对话；不确定的内容不写入记忆。

use serde_json::Value;

use crate::memory::{MemoryKind, MemoryScope, MemoryStore, MemoryWriteInput};

use super::models::{AiCompletionRequest, AiCompletionResponse, AiError, AiMessage};
use super::service::AiService;

/// 萃取用的系统提示词保存在 assets/memory-extraction-prompt.txt。
pub const EXTRACTION_PROMPT: &str = include_str!("../../assets/memory-extraction-prompt.txt");

/// 太短的寒暄轮次没有萃取价值，直接跳过，省一次 API 调用。
const MIN_USER_TEXT: usize = 12;

pub fn worth_extracting(user_text: &str) -> bool {
    user_text.trim().chars().count() >= MIN_USER_TEXT
}

/// 每轮写入上限。
const MAX_MEMORIES: usize = 2;

const MAX_TAGS: usize = 5;
const USER_CLIP: usize = 2_000;
const ASSISTANT_CLIP: usize = 1_500;
const TITLE_CLIP: usize = 80;
const CONTENT_CLIP: usize = 500;

/// 非流式一次性补全。
///
/// 单独一个 trait 而不是复用 `agent_runner::ModelClient`：萃取要的就是**非流式**
/// （边出边显示一段马上要被 JSON 解析掉的文本毫无意义），而且这样 Agent 循环的
/// 测试替身不必被迫实现一个它用不到的方法。
pub trait OneShotModel {
    fn complete_once(&self, request: &AiCompletionRequest)
        -> Result<AiCompletionResponse, AiError>;
}

impl OneShotModel for AiService {
    fn complete_once(
        &self,
        request: &AiCompletionRequest,
    ) -> Result<AiCompletionResponse, AiError> {
        AiService::complete_once(self, request)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedMemory {
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
}

/// 记忆萃取结果。失败通过 `error` 返回供日志记录，不中断主对话流程。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtractionOutcome {
    pub written: usize,
    pub error: Option<String>,
}

pub struct MemoryExtractionParams<'a> {
    pub session_id: Option<&'a str>,
    pub user_text: &'a str,
    pub assistant_text: &'a str,
}

/// 按 `char` 截断并加省略号。
///
/// **与 TS 有个无法消除的差异**：那边按 UTF-16 码元切。Rust 的 `String` 不能容纳
/// 半个代理对，所以码元级对齐在这里根本不可表达——把 emoji 劈成两半在 JS 里得到的是
/// 一个孤立代理，Rust 只能拒绝。按 `char` 切是唯一正确的选择，差异只在星文平面字符上。
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…")
}

/// 从模型回复里抠出 JSON 并解析。
///
/// 取**第一个 `{` 到最后一个 `}`**，因为模型经常无视「不要用代码块包裹」，
/// 在 JSON 前后加上 ```json 围栏或一句解释。整段解析会失败，掐头去尾才稳。
pub fn parse_extraction(raw: &str) -> Vec<ExtractedMemory> {
    let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) else {
        return Vec::new();
    };
    if end <= start {
        return Vec::new();
    }

    let Ok(parsed) = serde_json::from_str::<Value>(&raw[start..=end]) else {
        return Vec::new();
    };
    let Some(items) = parsed.get("memories").and_then(Value::as_array) else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let title = object.get("title")?.as_str()?.trim();
            let content = object.get("content")?.as_str()?.trim();
            if title.is_empty() || content.is_empty() {
                return None;
            }
            let tags: Vec<String> = object
                .get("tags")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .take(MAX_TAGS)
                        .map(str::to_owned)
                        .collect()
                })
                // 模型没给标签时打上来源标记，用户才分得清哪些是自动记的
                .unwrap_or_else(|| vec!["auto-extracted".to_owned()]);
            Some(ExtractedMemory {
                title: title.to_owned(),
                content: content.to_owned(),
                tags,
            })
        })
        .take(MAX_MEMORIES)
        .collect()
}

fn build_request(user_text: &str, assistant_text: &str) -> AiCompletionRequest {
    AiCompletionRequest {
        messages: vec![
            AiMessage::new("system", EXTRACTION_PROMPT),
            AiMessage::new(
                "user",
                &format!(
                    "[用户消息]\n{}\n\n[助手回复]\n{}",
                    clip(user_text, USER_CLIP),
                    clip(assistant_text, ASSISTANT_CLIP)
                ),
            ),
        ],
        temperature: Some(0.0),
        max_tokens: Some(400),
        stream: Some(false),
        ..Default::default()
    }
}

/// 从一轮对话里萃取长期记忆并写入。返回写入条数；失败只记在 `error` 里。
pub fn extract_from_exchange(
    model: &dyn OneShotModel,
    store: &MemoryStore,
    params: &MemoryExtractionParams,
) -> ExtractionOutcome {
    extract_from_exchange_guarded(model, store, params, &|| true)
}

/// 取消/设置/权限闸门在 API 调用前和每次写入前都会复查，
/// 模型返回之后也不例外。调用方让它独立于主对话请求的取消标志。
pub fn extract_from_exchange_guarded(
    model: &dyn OneShotModel,
    store: &MemoryStore,
    params: &MemoryExtractionParams,
    allowed: &dyn Fn() -> bool,
) -> ExtractionOutcome {
    extract_with_writer_guarded(model, params, allowed, &|input| {
        store.write(input).map(|_| true).map_err(|e| e.to_string())
    })
}

/// 网络推断与持久化分开，原生宿主就不必为一个缓慢/被取消的 HTTP 请求
/// 一直握着 SQLite 连接。返回 false 表示跳过。
pub fn extract_with_writer_guarded(
    model: &dyn OneShotModel,
    params: &MemoryExtractionParams,
    allowed: &dyn Fn() -> bool,
    persist: &dyn Fn(MemoryWriteInput) -> Result<bool, String>,
) -> ExtractionOutcome {
    if !allowed() {
        return ExtractionOutcome::default();
    }
    let user_text = params.user_text.trim();
    if !worth_extracting(user_text) {
        return ExtractionOutcome::default();
    }

    let request = build_request(user_text, params.assistant_text.trim());
    let response = match model.complete_once(&request) {
        Ok(response) => response,
        Err(error) => {
            return ExtractionOutcome {
                written: 0,
                error: Some(error.to_string()),
            };
        }
    };

    let mut outcome = ExtractionOutcome::default();
    for memory in parse_extraction(&response.content) {
        if !allowed() {
            break;
        }
        let input = MemoryWriteInput {
            kind: Some(MemoryKind::Fact),
            scope: Some(MemoryScope::Global),
            scope_ref: None,
            title: clip(&memory.title, TITLE_CLIP),
            content: clip(&memory.content, CONTENT_CLIP),
            tags: memory.tags,
            session_id: params.session_id.map(str::to_owned),
            // 同标题就更新——自动萃取最容易产出的正是「同一件事记了五遍」
            upsert: Some(true),
        };
        match persist(input) {
            Ok(written) => outcome.written += usize::from(written),
            Err(error) => outcome.error = Some(error),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemorySearchOptions;
    use serde_json::json;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    /// 通用的「够长到会触发萃取」的用户消息。
    ///
    /// 单独抽出来是因为踩过一次：随手写的中文测试输入只有 11 个字，刚好卡在
    /// `MIN_USER_TEXT` 下面，于是**每个用例都在测「短消息被跳过」**——
    /// 断言写的是写入条数，实际验的却是长度阈值，全绿也说明不了任何事。
    /// 下面的 `the_sample_input_is_actually_long_enough` 钉住这一点。
    const SAMPLE_USER: &str = "一段足够长的用户消息内容在这里";

    struct Fake {
        reply: String,
        seen: RefCell<Vec<AiCompletionRequest>>,
        fail: bool,
    }

    impl Fake {
        fn saying(reply: &str) -> Self {
            Self {
                reply: reply.to_owned(),
                seen: RefCell::new(Vec::new()),
                fail: false,
            }
        }
        fn broken() -> Self {
            Self {
                reply: String::new(),
                seen: RefCell::new(Vec::new()),
                fail: true,
            }
        }
        fn calls(&self) -> usize {
            self.seen.borrow().len()
        }
    }

    impl OneShotModel for Fake {
        fn complete_once(
            &self,
            request: &AiCompletionRequest,
        ) -> Result<AiCompletionResponse, AiError> {
            self.seen.borrow_mut().push(request.clone());
            if self.fail {
                return Err(AiError::api("provider 挂了", 500));
            }
            Ok(AiCompletionResponse {
                content: self.reply.clone(),
                ..Default::default()
            })
        }
    }

    struct Fixture {
        root: PathBuf,
        store: MemoryStore,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-memex-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let store = MemoryStore::open(&root).unwrap();
        Fixture { root, store }
    }

    fn params<'a>(user: &'a str, assistant: &'a str) -> MemoryExtractionParams<'a> {
        MemoryExtractionParams {
            session_id: Some("s-1"),
            user_text: user,
            assistant_text: assistant,
        }
    }

    fn extracted(f: &Fixture) -> Vec<crate::memory::MemoryRecord> {
        f.store.search(&MemorySearchOptions::default()).unwrap()
    }

    // ---------- 写入 ----------

    #[test]
    fn guard_blocks_calls_and_rechecks_after_model_completion() {
        let f = fixture("guard-before");
        let model = Fake::saying(r#"{"memories":[{"title":"偏好","content":"中文"}]}"#);
        let outcome = extract_from_exchange_guarded(
            &model,
            &f.store,
            &params(SAMPLE_USER, "回复"),
            &|| false,
        );
        assert_eq!(outcome.written, 0);
        assert_eq!(model.calls(), 0);
        assert!(extracted(&f).is_empty());
        struct TurningOff<'a>(&'a std::cell::Cell<bool>);
        impl OneShotModel for TurningOff<'_> {
            fn complete_once(
                &self,
                _: &AiCompletionRequest,
            ) -> Result<AiCompletionResponse, AiError> {
                self.0.set(false);
                Ok(AiCompletionResponse {
                    content: r#"{"memories":[{"title":"不能保存","content":"已关闭"}]}"#.into(),
                    ..Default::default()
                })
            }
        }
        let allowed = std::cell::Cell::new(true);
        let model = TurningOff(&allowed);
        let outcome = extract_from_exchange_guarded(
            &model,
            &f.store,
            &params(SAMPLE_USER, "回复"),
            &|| allowed.get(),
        );
        assert_eq!(outcome.written, 0);
        assert!(extracted(&f).is_empty());
    }

    #[test]
    fn a_stated_preference_is_written_to_long_term_memory() {
        let f = fixture("write");
        let model = Fake::saying(
            &json!({"memories": [
                {"title": "偏好中文回复", "content": "用户希望助手始终用中文回答", "tags": ["preference"]}
            ]})
            .to_string(),
        );

        let outcome = extract_from_exchange(
            &model,
            &f.store,
            &params("以后回答我的问题都请用中文", "好的"),
        );
        assert_eq!(
            outcome,
            ExtractionOutcome {
                written: 1,
                error: None
            }
        );

        let records = extracted(&f);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "偏好中文回复");
        assert_eq!(records[0].tags, ["preference"]);
        assert_eq!(
            records[0].session_id.as_deref(),
            Some("s-1"),
            "要能回溯是哪次对话记的"
        );
        assert_eq!(records[0].kind, MemoryKind::Fact);
        assert_eq!(records[0].scope, MemoryScope::Global);
    }

    /// 同标题的记忆更新而不是堆叠——自动萃取最容易产出的正是「同一件事记五遍」。
    #[test]
    fn re_extracting_the_same_title_updates_instead_of_piling_up() {
        let f = fixture("upsert");
        let first = Fake::saying(
            &json!({"memories": [{"title": "职业", "content": "用户是后端工程师"}]}).to_string(),
        );
        let second = Fake::saying(
            &json!({"memories": [{"title": "职业", "content": "用户是后端工程师，主攻分布式系统"}]})
                .to_string(),
        );

        extract_from_exchange(
            &first,
            &f.store,
            &params("我平时是做后端开发的工程师", "了解"),
        );
        extract_from_exchange(
            &second,
            &f.store,
            &params("我主要负责分布式系统的设计", "了解"),
        );

        let records = extracted(&f);
        assert_eq!(records.len(), 1, "同标题应更新: {records:?}");
        assert!(records[0].content.contains("分布式"));
    }

    #[test]
    fn at_most_two_memories_are_written_per_exchange() {
        let f = fixture("cap");
        let many: Vec<Value> = (0..5)
            .map(|i| json!({"title": format!("事实{i}"), "content": format!("内容{i}")}))
            .collect();
        let model = Fake::saying(&json!({ "memories": many }).to_string());

        let outcome =
            extract_from_exchange(&model, &f.store, &params("讲讲我的基本情况和背景吧", "好"));
        assert_eq!(
            outcome.written, MAX_MEMORIES,
            "一轮最多两条，多了就是流水账"
        );
        assert_eq!(extracted(&f).len(), MAX_MEMORIES);
    }

    #[test]
    fn missing_tags_get_a_provenance_marker() {
        let f = fixture("tags");
        let model =
            Fake::saying(&json!({"memories": [{"title": "T", "content": "C"}]}).to_string());
        extract_from_exchange(&model, &f.store, &params(SAMPLE_USER, "好"));
        assert_eq!(
            extracted(&f)[0].tags,
            ["auto-extracted"],
            "没标签时要标出来源，用户才分得清哪些是自动记的"
        );
    }

    #[test]
    fn overlong_fields_are_clipped() {
        let f = fixture("clip");
        let model = Fake::saying(
            &json!({"memories": [{"title": "标".repeat(200), "content": "容".repeat(900)}]})
                .to_string(),
        );
        extract_from_exchange(&model, &f.store, &params(SAMPLE_USER, "好"));

        let record = &extracted(&f)[0];
        assert_eq!(
            record.title.chars().count(),
            TITLE_CLIP + 1,
            "80 字 + 省略号"
        );
        assert_eq!(record.content.chars().count(), CONTENT_CLIP + 1);
        assert!(record.title.ends_with('…'));
    }

    // ---------- 宁缺毋滥 ----------

    #[test]
    fn an_empty_memory_list_writes_nothing() {
        let f = fixture("empty");
        let model = Fake::saying(r#"{"memories":[]}"#);
        assert_eq!(
            extract_from_exchange(&model, &f.store, &params("随便聊聊今天的天气怎么样", "嗯"))
                .written,
            0
        );
        assert!(extracted(&f).is_empty());
    }

    /// 太短的寒暄轮次连 API 都不该调——这是每轮对话都会跑的路径，省的是真钱。
    #[test]
    fn a_short_exchange_skips_the_api_call_entirely() {
        let f = fixture("short");
        let model = Fake::saying(r#"{"memories":[{"title":"T","content":"C"}]}"#);

        let outcome = extract_from_exchange(&model, &f.store, &params("你好", "你好"));
        assert_eq!(outcome.written, 0);
        assert_eq!(model.calls(), 0, "短消息不该发请求");

        // 刚好到阈值就要跑
        let long = "啊".repeat(MIN_USER_TEXT);
        extract_from_exchange(&model, &f.store, &params(&long, "回复"));
        assert_eq!(model.calls(), 1);
    }

    /// 钉住测试样本本身：它必须真的够长。
    ///
    /// 这条不是形式主义——第一版的样本是 11 个字，卡在阈值下面一格，于是上面每个
    /// 「写入了几条」的用例实际都在测「短消息被跳过」，断言和被验行为完全对不上。
    /// 有了这条，样本再被改短就会**在这里**失败，而不是让一堆用例悄悄失去意义。
    #[test]
    fn the_sample_input_is_actually_long_enough() {
        assert!(
            SAMPLE_USER.chars().count() >= MIN_USER_TEXT,
            "样本只有 {} 字，会被长度阈值挡掉，上面那些用例就都白测了",
            SAMPLE_USER.chars().count()
        );
    }

    /// 标题或正文为空的条目直接丢弃，不写进库。
    #[test]
    fn entries_without_a_title_or_content_are_discarded() {
        let f = fixture("invalid");
        let model = Fake::saying(
            &json!({"memories": [
                {"title": "  ", "content": "有正文没标题"},
                {"title": "有标题没正文", "content": ""},
                {"content": "缺 title 字段"},
                {"title": "合格的", "content": "合格的正文"},
            ]})
            .to_string(),
        );
        assert_eq!(
            extract_from_exchange(&model, &f.store, &params(SAMPLE_USER, "好")).written,
            1
        );
        assert_eq!(extracted(&f)[0].title, "合格的");
    }

    // ---------- 解析容错 ----------

    /// 模型经常无视「不要用代码块包裹」。掐头去尾取第一个 { 到最后一个 }。
    #[test]
    fn json_wrapped_in_a_code_fence_still_parses() {
        let wrapped = "好的，我分析了一下：\n```json\n{\"memories\":[{\"title\":\"T\",\"content\":\"C\"}]}\n```\n以上。";
        let parsed = parse_extraction(wrapped);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].title, "T");
    }

    #[test]
    fn unparseable_replies_yield_nothing_instead_of_failing() {
        for raw in [
            "",
            "我拿不准，什么都没记",
            "{ 这不是合法 JSON",
            r#"{"memories": "不是数组"}"#,
            r#"{"other": []}"#,
            "}{",
        ] {
            assert!(parse_extraction(raw).is_empty(), "{raw:?} 应解析为空");
        }
    }

    #[test]
    fn tags_are_capped_and_non_strings_dropped() {
        let raw = json!({"memories": [{
            "title": "T", "content": "C",
            "tags": ["a", "b", "c", "d", "e", "f", 123, null],
        }]})
        .to_string();
        assert_eq!(parse_extraction(&raw)[0].tags, ["a", "b", "c", "d", "e"]);
    }

    // ---------- 绝不影响主对话 ----------

    /// 服务方故障不能变成向上抛出的异常——记忆提取失败只表示“本轮没有记住内容”。
    #[test]
    fn a_provider_failure_is_reported_but_never_thrown() {
        let f = fixture("fail");
        let outcome = extract_from_exchange(&Fake::broken(), &f.store, &params(SAMPLE_USER, "好"));
        assert_eq!(outcome.written, 0);
        assert!(
            outcome.error.unwrap().contains("provider 挂了"),
            "错误要留给调用方打日志，不能咽掉"
        );
        assert!(extracted(&f).is_empty());
    }

    // ---------- 请求形状 ----------

    #[test]
    fn the_request_is_non_streaming_and_deterministic() {
        let f = fixture("shape");
        let model = Fake::saying(r#"{"memories":[]}"#);
        extract_from_exchange(&model, &f.store, &params(SAMPLE_USER, "回复内容"));

        let request = &model.seen.borrow()[0];
        assert_eq!(request.temperature, Some(0.0), "萃取要可复现，不能随机");
        assert_eq!(
            request.stream,
            Some(false),
            "一段马上要被解析掉的 JSON 没必要流式"
        );
        assert_eq!(request.max_tokens, Some(400));
        assert!(request.tools.is_empty(), "萃取不该带工具");

        assert_eq!(
            request.messages[0].content.as_deref(),
            Some(EXTRACTION_PROMPT)
        );
        let user = request.messages[1].content.as_deref().unwrap();
        assert!(user.contains(&format!("[用户消息]\n{SAMPLE_USER}")));
        assert!(user.contains("[助手回复]\n回复内容"));
    }

    #[test]
    fn long_turns_are_clipped_before_being_sent() {
        let f = fixture("clip-request");
        let model = Fake::saying(r#"{"memories":[]}"#);
        let long_user = "用".repeat(5_000);
        let long_assistant = "助".repeat(5_000);
        extract_from_exchange(&model, &f.store, &params(&long_user, &long_assistant));

        let user = model.seen.borrow()[0].messages[1].content.clone().unwrap();
        assert!(user.contains(&format!("{}…", "用".repeat(USER_CLIP))));
        assert!(user.contains(&format!("{}…", "助".repeat(ASSISTANT_CLIP))));
    }

    #[test]
    fn the_extracted_prompt_states_the_output_contract() {
        assert!(EXTRACTION_PROMPT.contains("记忆萃取器"));
        assert!(
            EXTRACTION_PROMPT.contains(r#"{"memories":[]}"#),
            "输出契约示例必须在"
        );
        assert!(
            EXTRACTION_PROMPT.contains("拿不准一律不记"),
            "宁缺毋滥是这块的核心约束"
        );
        assert!(!EXTRACTION_PROMPT.contains('\r'), "提示词必须是 LF");
    }
}
