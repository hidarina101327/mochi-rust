//! 会话元数据中的 projectId / color 保留显式 null；JSON 不加尾换行。

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::models::AiToolCall;
use crate::{json2, jstime, paths};

fn default_title() -> String {
    "新会话".into()
}

/// 会话元数据（`index.json` 的 `sessions[]`）。
///
/// `project_id` / `color` **不加** `skip_serializing_if`：真实数据里它们以 `null` 落盘。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiSessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub project_id: Option<String>,
    pub pinned: bool,
    pub color: Option<String>,
    pub order: i64,
    pub message_count: i32,
}

impl Default for AiSessionMeta {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: default_title(),
            created_at: 0,
            updated_at: 0,
            project_id: None,
            pinned: false,
            color: None,
            order: 0,
            message_count: 0,
        }
    }
}

/// AI 项目（会话归类树节点）。`color` 同样以 `null` 落盘。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiProject {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub order: i64,
    pub expanded: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for AiProject {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            color: None,
            order: 0,
            expanded: true,
            created_at: 0,
            updated_at: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiSessionIndex {
    pub version: i32,
    pub projects: Vec<AiProject>,
    pub sessions: Vec<AiSessionMeta>,
}

impl Default for AiSessionIndex {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
            sessions: Vec::new(),
        }
    }
}

/// 使用保序 JSON 对象，保留消息的原键序和未知字段，避免丢失附件及审批数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AiStoredMessage {
    fields: serde_json::Map<String, serde_json::Value>,
}

impl Default for AiStoredMessage {
    fn default() -> Self {
        Self::new("user", "")
    }
}

impl AiStoredMessage {
    /// 建一条新消息，键序对齐 TS 最常见的 `role, content, timestamp`。
    pub fn new(role: &str, content: &str) -> Self {
        let mut fields = serde_json::Map::new();
        fields.insert("role".into(), json!(role));
        fields.insert("content".into(), json!(content));
        fields.insert("timestamp".into(), json!(jstime::now_millis()));
        Self { fields }
    }

    pub fn from_map(fields: serde_json::Map<String, serde_json::Value>) -> Self {
        Self { fields }
    }

    pub fn fields(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.fields
    }

    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.fields.get(key)
    }

    /// 设置字段。已存在的键**保持原有位置**，新键追加到末尾（与 JS 对象一致）。
    pub fn set(&mut self, key: &str, value: serde_json::Value) {
        self.fields.insert(key.to_owned(), value);
    }

    pub fn role(&self) -> &str {
        self.fields
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("user")
    }

    pub fn content(&self) -> &str {
        self.fields
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    pub fn set_content(&mut self, content: &str) {
        self.set("content", json!(content));
    }

    /// 持久化在 `images` 的附件引用。真实数据使用工作区内
    /// `.mochi/ai-sessions` 下的相对路径；这里仍接受 URL/data URL，方便
    /// 原生 UI 与 Electron 互通时不丢失扩展格式。
    pub fn images(&self) -> Vec<String> {
        self.fields
            .get("images")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn id(&self) -> Option<&str> {
        self.fields.get("id").and_then(|v| v.as_str())
    }

    pub fn timestamp(&self) -> Option<i64> {
        self.fields.get("timestamp").and_then(|v| v.as_i64())
    }

    /// 隐藏消息（工具往返）不参与展示，也不发给模型。
    pub fn is_hidden(&self) -> bool {
        self.fields
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    pub fn tool_calls(&self) -> Vec<AiToolCall> {
        self.fields
            .get("tool_calls")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    pub fn tool_call_id(&self) -> Option<&str> {
        self.fields.get("tool_call_id").and_then(|v| v.as_str())
    }
}

/// 单个会话完整内容（`sessions/<id>.json`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiConversation {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub messages: Vec<AiStoredMessage>,
}

impl Default for AiConversation {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: default_title(),
            created_at: 0,
            updated_at: 0,
            messages: Vec::new(),
        }
    }
}

pub fn new_conversation_id() -> String {
    format!(
        "ai-conversation-{}-{}",
        jstime::now_millis(),
        paths::random_base36(6)
    )
}

pub fn new_project_id() -> String {
    format!(
        "ai-project-{}-{}",
        jstime::now_millis(),
        paths::random_base36(6)
    )
}

pub struct AiSessionService {
    base_path: PathBuf,
}

impl AiSessionService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: paths::mochi_dir(workspace_path).join("ai-sessions"),
        }
    }

    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    fn index_path(&self) -> PathBuf {
        self.base_path.join("index.json")
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.base_path.join("sessions").join(format!("{id}.json"))
    }

    /// 确保目录结构存在；首次创建时写入空索引。
    pub fn initialize(&self) -> Result<()> {
        std::fs::create_dir_all(self.base_path.join("sessions"))?;
        if !self.index_path().exists() {
            self.save_index(&AiSessionIndex::default())?;
        }
        Ok(())
    }

    /// 加载索引。文件不存在或损坏时返回空索引（不报错——不能因为索引坏了就打不开工作区）。
    pub fn load_index(&self) -> AiSessionIndex {
        std::fs::read_to_string(self.index_path())
            .ok()
            .and_then(|t| json2::deserialize(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_index(&self, index: &AiSessionIndex) -> Result<()> {
        std::fs::create_dir_all(self.base_path.join("sessions"))?;
        json2::write(self.index_path(), index)
    }

    /// 读取单个会话。文件不存在、损坏或 id 为空时返回 `None`。
    pub fn load_session(&self, id: &str) -> Option<AiConversation> {
        let text = std::fs::read_to_string(self.session_path(id)).ok()?;
        let parsed: AiConversation = json2::deserialize(&text).ok()?;
        (!parsed.id.is_empty()).then_some(parsed)
    }

    /// 写入单个会话（**只写会话文件**；索引由调用方另行 `save_index`）。
    pub fn save_session(&self, conversation: &AiConversation) -> Result<()> {
        self.initialize()?;
        json2::write(self.session_path(&conversation.id), conversation)
    }

    /// 删除单个会话文件（索引由调用方另行 `save_index`）。
    pub fn delete_session(&self, id: &str) -> Result<()> {
        let path = self.session_path(id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// 列出会话文件里实际存在的 id（用于检出索引与磁盘不一致）。
    pub fn list_session_files(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.base_path.join("sessions")) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".json").map(str::to_owned)
            })
            .collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("mochi-sess-{}-{tag}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn svc(&self) -> AiSessionService {
            AiSessionService::new(&self.0)
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 真实工作区里 47 个会话条目的 projectId/color 都是显式 null。
    /// C# 的 WhenWritingNull 会把字段删掉，两版交替写会反复增删。
    #[test]
    fn session_meta_writes_explicit_nulls() {
        let index = AiSessionIndex {
            version: 1,
            projects: vec![],
            sessions: vec![AiSessionMeta {
                id: "ai-conversation-1784989217136-odbudq".into(),
                title: "完成文档".into(),
                created_at: 1784989217136,
                updated_at: 1784989294513,
                order: 1784989217136,
                message_count: 2,
                ..Default::default()
            }],
        };
        // 逐字节比对真实文件里的那一条
        let json = json2::serialize(&index).unwrap();
        assert!(
            json.contains("\"projectId\": null"),
            "projectId 被省略了:\n{json}"
        );
        assert!(json.contains("\"color\": null"), "color 被省略了:\n{json}");
        let i = |k: &str| json.find(&format!("\"{k}\"")).unwrap();
        assert!(i("updatedAt") < i("projectId"));
        assert!(i("projectId") < i("pinned"));
        assert!(i("pinned") < i("color"));
        assert!(i("color") < i("order"));
        assert!(i("order") < i("messageCount"));
    }

    #[test]
    fn project_color_is_written_as_null() {
        let index = AiSessionIndex {
            projects: vec![AiProject {
                id: "ai-project-1782541685896-ktwz87".into(),
                name: "墨池".into(),
                order: 1782541685896,
                created_at: 1782541685896,
                updated_at: 1782541685896,
                ..Default::default()
            }],
            ..Default::default()
        };
        let json = json2::serialize(&index).unwrap();
        assert!(json.contains("\"color\": null"), "{json}");
        assert!(
            json.contains("\"expanded\": true"),
            "expanded 默认应为 true"
        );
    }

    /// 消息里的可选字段是**省略式**的——真实会话文件里没有 id/hidden/tool_calls 时字段不出现。
    #[test]
    fn stored_message_omits_absent_optionals() {
        let mut msg = AiStoredMessage::new("user", "分析这篇开发日志");
        msg.set("timestamp", serde_json::json!(1782541615176_i64));
        assert_eq!(
            json2::serialize(&msg).unwrap(),
            "{\n  \"role\": \"user\",\n  \"content\": \"分析这篇开发日志\",\n  \"timestamp\": 1782541615176\n}"
        );
    }

    /// 真实数据里存在 15 种键顺序，且有 7 个 C# 模型没有的字段。
    /// 读进来再写回去必须**原样**——丢字段就是丢用户的图片附件和待批准编辑。
    #[test]
    fn unknown_fields_and_key_order_survive_a_round_trip() {
        // 取自真实会话：id 在最前、带 usage/usageSource
        let src = "{\n  \"id\": \"m1\",\n  \"role\": \"assistant\",\n  \"content\": \"答\",\n  \"timestamp\": 1,\n  \"usage\": {\n    \"promptTokens\": 4196,\n    \"completionTokens\": 221,\n    \"totalTokens\": 4417\n  },\n  \"usageSource\": \"estimated\"\n}";
        let msg: AiStoredMessage = json2::deserialize(src).unwrap();
        assert_eq!(
            json2::serialize(&msg).unwrap(),
            src,
            "扩展字段或键序被改写了"
        );
        assert_eq!(msg.id(), Some("m1"));
        assert_eq!(msg.role(), "assistant");
        assert_eq!(
            msg.get("usage")
                .and_then(|u| u.get("totalTokens"))
                .and_then(|v| v.as_i64()),
            Some(4417)
        );
    }

    #[test]
    fn id_last_ordering_is_also_preserved() {
        // 另一种真实键序：id 在末尾
        let src = "{\n  \"role\": \"user\",\n  \"content\": \"问\",\n  \"timestamp\": 2,\n  \"id\": \"m2\"\n}";
        let msg: AiStoredMessage = json2::deserialize(src).unwrap();
        assert_eq!(json2::serialize(&msg).unwrap(), src);
    }

    #[test]
    fn image_attachments_are_never_dropped() {
        let src = r#"{"role":"user","content":"这个图我看不懂","images":["assets/1782561244102-tnjaahfjx.png"],"timestamp":1}"#;
        let msg: AiStoredMessage = json2::deserialize(src).unwrap();
        let images = msg
            .get("images")
            .and_then(|v| v.as_array())
            .expect("图片字段丢了");
        assert_eq!(images.len(), 1);
        assert_eq!(msg.images(), vec!["assets/1782561244102-tnjaahfjx.png"]);
    }

    #[test]
    fn setting_an_existing_key_keeps_its_position() {
        let src = r#"{"id":"m","role":"user","content":"旧","timestamp":1}"#;
        let mut msg: AiStoredMessage = json2::deserialize(src).unwrap();
        msg.set_content("新");
        let keys: Vec<&str> = msg.fields().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["id", "role", "content", "timestamp"],
            "改内容不该挪动键序"
        );
        assert_eq!(msg.content(), "新");
    }

    #[test]
    fn message_accessors_have_sane_defaults() {
        let msg: AiStoredMessage = json2::deserialize("{}").unwrap();
        assert_eq!(msg.role(), "user");
        assert_eq!(msg.content(), "");
        assert_eq!(msg.id(), None);
        assert!(!msg.is_hidden());
        assert!(msg.tool_calls().is_empty());
    }

    #[test]
    fn conversation_shape_matches_real_file() {
        let conv = AiConversation {
            id: "ai-conversation-1782541615175-evzcmu".into(),
            title: "分析这篇开发日志".into(),
            created_at: 1782541615175,
            updated_at: 1782541911292,
            messages: vec![{
                let mut m = AiStoredMessage::new("user", "分析这篇开发日志");
                m.set("timestamp", serde_json::json!(1782541615176_i64));
                m
            }],
        };
        let json = json2::serialize(&conv).unwrap();
        assert!(
            json.starts_with(
                "{\n  \"id\": \"ai-conversation-1782541615175-evzcmu\",\n  \"title\":"
            ),
            "{json}"
        );
        assert!(json.ends_with("]\n}"), "尾部形状不对");
    }

    #[test]
    fn ai_session_files_have_no_trailing_newline() {
        let ws = TempWs::new("newline");
        let svc = ws.svc();
        svc.initialize().unwrap();
        let raw = std::fs::read(svc.index_path()).unwrap();
        assert!(!raw.ends_with(b"\n"), ".mochi/ 下的文件不该有尾换行");
    }

    #[test]
    fn initialize_creates_dirs_and_empty_index() {
        let ws = TempWs::new("init");
        let svc = ws.svc();
        svc.initialize().unwrap();
        assert!(svc.base_path().join("sessions").is_dir());
        let index = svc.load_index();
        assert_eq!(index.version, 1);
        assert!(index.sessions.is_empty());
    }

    #[test]
    fn session_round_trip() {
        let ws = TempWs::new("roundtrip");
        let svc = ws.svc();
        let conv = AiConversation {
            id: new_conversation_id(),
            title: "带中文的会话".into(),
            created_at: 1,
            updated_at: 2,
            messages: vec![AiStoredMessage::new("user", "你好")],
        };
        svc.save_session(&conv).unwrap();

        let loaded = svc.load_session(&conv.id).expect("应能读回");
        assert_eq!(loaded.title, "带中文的会话");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content(), "你好");
    }

    #[test]
    fn missing_or_corrupt_session_reads_as_none() {
        let ws = TempWs::new("corrupt");
        let svc = ws.svc();
        svc.initialize().unwrap();
        assert!(svc.load_session("不存在").is_none());

        std::fs::write(svc.session_path("坏的"), "{ 不是 JSON").unwrap();
        assert!(svc.load_session("坏的").is_none());

        // id 为空的会话文件视为无效
        std::fs::write(svc.session_path("空id"), "{\"id\":\"\"}").unwrap();
        assert!(svc.load_session("空id").is_none());
    }

    #[test]
    fn corrupt_index_falls_back_to_empty_not_error() {
        let ws = TempWs::new("bad-index");
        let svc = ws.svc();
        svc.initialize().unwrap();
        std::fs::write(svc.index_path(), "{ 坏").unwrap();
        let index = svc.load_index();
        assert_eq!(index.version, 1);
        assert!(index.sessions.is_empty(), "索引坏了不该阻塞工作区打开");
    }

    #[test]
    fn delete_removes_only_the_session_file() {
        let ws = TempWs::new("delete");
        let svc = ws.svc();
        let a = AiConversation {
            id: new_conversation_id(),
            ..Default::default()
        };
        let b = AiConversation {
            id: new_conversation_id(),
            ..Default::default()
        };
        svc.save_session(&a).unwrap();
        svc.save_session(&b).unwrap();

        svc.delete_session(&a.id).unwrap();
        svc.delete_session(&a.id).unwrap(); // 幂等

        assert!(svc.load_session(&a.id).is_none());
        assert!(svc.load_session(&b.id).is_some());
        assert_eq!(svc.list_session_files(), vec![b.id.clone()]);
    }

    #[test]
    fn ids_follow_the_ts_shape() {
        for (id, prefix) in [
            (new_conversation_id(), "ai-conversation-"),
            (new_project_id(), "ai-project-"),
        ] {
            let rest = id.strip_prefix(prefix).expect("缺前缀");
            let (ts, rand) = rest.rsplit_once('-').expect("缺随机段");
            assert!(ts.parse::<i64>().is_ok(), "{id}");
            assert_eq!(rand.len(), 6, "{id}");
        }
    }
}
