//! 挂载记录保留未知字段和原有键顺序；缺少的 `scope` 字段追加到末尾。JSON 文件不加尾换行。

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::object_reference::{ObjectKind, ObjectReference};
use crate::{json2, jstime, paths};

const INDEX_FILE: &str = "document-mounts.json";

/// 挂载范围。`message` 是缺省——认不出的值一律当 `message`（对齐 TS 的三元判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountScope {
    Session,
    Message,
}

impl MountScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Message => "message",
        }
    }
    fn parse(value: Option<&str>) -> Self {
        if value == Some("session") {
            Self::Session
        } else {
            Self::Message
        }
    }
}

/// 一条挂载记录。**保序** JSON 对象，见模块头。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AiDocumentMount {
    fields: Map<String, Value>,
}

impl AiDocumentMount {
    pub fn from_map(fields: Map<String, Value>) -> Self {
        Self { fields }
    }

    pub fn fields(&self) -> &Map<String, Value> {
        &self.fields
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    fn str_field(&self, key: &str) -> &str {
        self.fields.get(key).and_then(Value::as_str).unwrap_or("")
    }

    fn opt_str_field(&self, key: &str) -> Option<&str> {
        self.fields
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn id(&self) -> &str {
        self.str_field("id")
    }
    pub fn session_id(&self) -> &str {
        self.str_field("sessionId")
    }
    pub fn scope(&self) -> MountScope {
        MountScope::parse(self.fields.get("scope").and_then(Value::as_str))
    }
    pub fn message_id(&self) -> Option<&str> {
        self.opt_str_field("messageId")
    }
    pub fn user_message_id(&self) -> Option<&str> {
        self.opt_str_field("userMessageId")
    }
    pub fn document_path(&self) -> &str {
        self.str_field("documentPath")
    }
    pub fn document_relative_path(&self) -> &str {
        self.str_field("documentRelativePath")
    }
    pub fn session_title(&self) -> &str {
        self.str_field("sessionTitle")
    }
    pub fn snippet(&self) -> &str {
        self.str_field("snippet")
    }

    /// 指向具体对象的挂载会保存稳定的对象 URL。早期只记录文档路径的挂载没有
    /// `objectUrl`，依旧表示整篇文档。`referenceUrl` 单向兼容：
    /// 持久化契约敲定之前，早期的原生调用方用过这个名字。
    pub fn object_url(&self) -> Option<&str> {
        self.opt_str_field("objectUrl")
            .or_else(|| self.opt_str_field("referenceUrl"))
    }

    /// 把挂载当作普通引用的调用方用的别名。
    pub fn reference_url(&self) -> Option<&str> {
        self.object_url()
    }

    /// 挂载指向块时，返回其稳定块 ID。
    pub fn block_id(&self) -> Option<String> {
        self.object_url().and_then(|url| {
            ObjectReference::parse(url)
                .filter(|reference| reference.kind == ObjectKind::Block)
                .and_then(|reference| reference.block_id)
        })
    }
    pub fn created_at(&self) -> i64 {
        self.fields
            .get("createdAt")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }

    /// 一条记录要能用，这四个字段必须都是字符串。对齐 TS `normalizeIndex` 的过滤条件。
    fn is_valid(&self) -> bool {
        ["id", "sessionId", "documentPath", "documentRelativePath"]
            .iter()
            .all(|key| self.fields.get(*key).is_some_and(Value::is_string))
    }

    /// 规范化 `scope`：已有的保持原位只改值，没有的追加到末尾——
    /// 这正是 TS 的 `{ ...mount, scope }` 的行为，也是磁盘上三种键序的来源。
    fn normalize_scope(&mut self) {
        let scope = self.scope();
        self.fields.insert("scope".into(), json!(scope.as_str()));
    }
}

/// 索引文件。顶层键序 `version, mounts, byDocument, bySession, byConversation`
/// 即字段声明序。
///
/// **只派生 `Serialize`**：反序列化走 `from_value`，因为 serde 的派生实现是
/// 全有或全无的——`mounts` 里混进一个 `null`，整个文件就解析失败，用户的挂载全没了。
/// TS 那边是逐条 `filter`，坏一条只丢一条。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDocumentMountIndex {
    pub version: u8,
    pub mounts: Vec<AiDocumentMount>,
    /// 以下三张表都是**派生缓存**，每次规范化时从 `mounts` 重建。
    /// 它们仍然落盘，因为 Electron 版就是这么存的。
    pub by_document: Map<String, Value>,
    pub by_session: Map<String, Value>,
    pub by_conversation: Map<String, Value>,
}

impl Default for AiDocumentMountIndex {
    fn default() -> Self {
        Self {
            version: 1,
            mounts: Vec::new(),
            by_document: Map::new(),
            by_session: Map::new(),
            by_conversation: Map::new(),
        }
    }
}

impl AiDocumentMountIndex {
    /// 从已解析的 JSON 逐条读出挂载，对齐 TS 的 `normalizeIndex(JSON.parse(...))`。
    ///
    /// 非数组的 `mounts`、数组里的 `null` 或标量，都只是**那一项**被丢掉，
    /// 不影响其余记录。派生表不读——反正要重建。
    pub fn from_value(value: &Value) -> Self {
        let mounts = value
            .get("mounts")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|object| AiDocumentMount::from_map(object.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            mounts,
            ..Self::default()
        }
        .normalize()
    }

    /// 丢弃残缺记录、补齐 `scope` 字段，并重建三张派生表。
    ///
    /// 派生表按 `mounts` 的顺序插入键，和 JS 对象的插入序一致——
    /// 用 `BTreeMap` 会按字典序排列，与上游产生不必要的差异。
    pub fn normalize(mut self) -> Self {
        self.mounts.retain(AiDocumentMount::is_valid);
        for mount in &mut self.mounts {
            mount.normalize_scope();
        }

        let (by_document, by_session, by_conversation) = build_indexes(&self.mounts);
        Self {
            version: 1,
            mounts: self.mounts,
            by_document,
            by_session,
            by_conversation,
        }
    }
}

fn push_id(table: &mut Map<String, Value>, key: &str, id: &str) {
    table
        .entry(key.to_owned())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .expect("派生表的值恒为数组")
        .push(json!(id));
}

fn object_identity(value: &str) -> String {
    ObjectReference::parse(value)
        .filter(|reference| reference.kind == ObjectKind::Block)
        .map(|reference| {
            format!(
                "{}#{}",
                reference.path.unwrap_or_default().to_lowercase(),
                reference.block_id.unwrap_or_default().to_lowercase()
            )
        })
        .unwrap_or_else(|| value.to_lowercase())
}

fn build_indexes(
    mounts: &[AiDocumentMount],
) -> (Map<String, Value>, Map<String, Value>, Map<String, Value>) {
    let mut by_document = Map::new();
    let mut by_session = Map::new();
    let mut by_conversation: Map<String, Value> = Map::new();

    for mount in mounts {
        push_id(&mut by_document, mount.document_relative_path(), mount.id());
        push_id(&mut by_session, mount.session_id(), mount.id());

        // 会话级挂载不进按消息分组的表——它不属于任何一条消息
        if mount.scope() == MountScope::Session {
            continue;
        }
        let Some(message_id) = mount.message_id() else {
            continue;
        };

        let session_table = by_conversation
            .entry(mount.session_id().to_owned())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("会话分组恒为对象");
        push_id(session_table, message_id, mount.id());
    }

    (by_document, by_session, by_conversation)
}

/// 文档相对工作区的路径。**大小写不敏感**地判断是否在工作区内
/// （Windows 路径大小写不敏感，用户可能给 `d:/mochi/…`）。
/// 不在工作区内就原样返回绝对路径——对齐 TS。
pub fn document_relative_path(workspace_path: &str, document_path: &str) -> String {
    let workspace = paths::to_forward_slashes(workspace_path)
        .trim_end_matches('/')
        .to_owned();
    let absolute = paths::to_forward_slashes(document_path);

    let lower_workspace = workspace.to_lowercase();
    let lower_absolute = absolute.to_lowercase();
    if lower_absolute == lower_workspace {
        return String::new();
    }
    if lower_absolute.starts_with(&format!("{lower_workspace}/")) {
        return absolute[workspace.len() + 1..].to_owned();
    }
    absolute
}

pub fn new_mount_id() -> String {
    format!(
        "ai-mount-{}-{}",
        jstime::now_millis(),
        paths::random_base36(6)
    )
}

/// 新建挂载的入参。
pub struct CreateMountInput {
    pub session_id: String,
    pub scope: MountScope,
    pub message_id: Option<String>,
    pub user_message_id: Option<String>,
    pub document_path: String,
    pub session_title: String,
    pub snippet: String,
    pub created_at: Option<i64>,
}

/// 一条持久化对象 URL 的新建挂载输入。path 字段留在记录里，
/// 兼容旧的文档索引和权限展示；`object_url` 才是读取具体块时用的身份。
pub struct CreateReferenceMountInput {
    pub session_id: String,
    pub scope: MountScope,
    pub message_id: Option<String>,
    pub user_message_id: Option<String>,
    pub document_path: String,
    pub object_url: String,
    pub session_title: String,
    pub snippet: String,
    pub created_at: Option<i64>,
}

/// AI 上下文解析挂载对象时，从源码读出的内容。
/// 块挂载的 `content` 是可见块文本；只记录文档路径的挂载则包含
/// 完整 UTF-8 源码。URL 始终是调用方的稳定键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedReferenceContent {
    pub url: String,
    pub path: PathBuf,
    pub block_id: Option<String>,
    pub content: String,
    pub raw_content: String,
    pub source_span: Option<(usize, usize)>,
}

pub struct AiDocumentMountService {
    workspace_path: PathBuf,
    base_path: PathBuf,
}

impl AiDocumentMountService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        let workspace_path = workspace_path.as_ref().to_path_buf();
        let base_path = paths::mochi_dir(&workspace_path).join("ai-sessions");
        Self {
            workspace_path,
            base_path,
        }
    }

    fn index_path(&self) -> PathBuf {
        self.base_path.join(INDEX_FILE)
    }

    fn workspace_str(&self) -> String {
        paths::to_forward_slashes(&self.workspace_path.to_string_lossy())
    }

    /// 读索引。文件缺失或损坏都返回空索引——挂载是锦上添花的功能，
    /// 读不出来不该让打开文档失败。
    pub fn load(&self) -> AiDocumentMountIndex {
        std::fs::read_to_string(self.index_path())
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .map(|value| AiDocumentMountIndex::from_value(&value))
            .unwrap_or_default()
    }

    /// 写索引（规范化后落盘，**不带尾换行**）。
    pub fn save(&self, index: AiDocumentMountIndex) -> Result<AiDocumentMountIndex> {
        let normalized = index.normalize();
        json2::write(self.index_path(), &normalized)?;
        Ok(normalized)
    }

    /// 建目录与空索引文件。已存在则原样保留，绝不覆盖用户数据。
    pub fn initialize(&self) -> Result<()> {
        std::fs::create_dir_all(&self.base_path)?;
        if !self.index_path().exists() {
            json2::write(self.index_path(), &AiDocumentMountIndex::default())?;
        }
        Ok(())
    }

    /// 批量新增挂载，返回**实际新建的**那些（已存在的会被跳过）。
    pub fn add_mounts(&self, inputs: Vec<CreateMountInput>) -> Result<Vec<AiDocumentMount>> {
        self.add_mounts_internal(inputs.into_iter().map(|input| (input, None)).collect())
    }

    /// 为具体对象 URL 添加挂载。与只记录文档路径的挂载不同，同一文档的
    /// 两个块是彼此独立的条目，可以在同一条 AI 消息里一起选中。
    pub fn add_reference_mounts(
        &self,
        inputs: Vec<CreateReferenceMountInput>,
    ) -> Result<Vec<AiDocumentMount>> {
        let mut mapped = Vec::with_capacity(inputs.len());
        for input in inputs {
            let reference =
                ObjectReference::parse(&input.object_url).context("文档块挂载引用无效")?;
            if reference.kind != ObjectKind::Block {
                bail!("AI 文档挂载需要具体的文档块引用")
            }
            let expected = Path::new(&input.document_path)
                .canonicalize()
                .with_context(|| format!("文档不存在: {}", input.document_path))?;
            let (_, actual) = self.validate_document_path(&reference)?;
            if expected != actual {
                bail!("文档块挂载的路径与文档不一致")
            }
            mapped.push((
                CreateMountInput {
                    session_id: input.session_id,
                    scope: input.scope,
                    message_id: input.message_id,
                    user_message_id: input.user_message_id,
                    document_path: input.document_path,
                    session_title: input.session_title,
                    snippet: input.snippet,
                    created_at: input.created_at,
                },
                Some(input.object_url),
            ));
        }
        self.add_mounts_internal(mapped)
    }

    /// 给习惯说「object」而非「reference」的调用方保留的别名。
    pub fn add_object_mounts(
        &self,
        inputs: Vec<CreateReferenceMountInput>,
    ) -> Result<Vec<AiDocumentMount>> {
        self.add_reference_mounts(inputs)
    }

    fn add_mounts_internal(
        &self,
        inputs: Vec<(CreateMountInput, Option<String>)>,
    ) -> Result<Vec<AiDocumentMount>> {
        let workspace = self.workspace_str();

        // 入参内先去重：路径挂载按文档去重；对象挂载按稳定 URL 去重，
        // 因而同一文档里的多个块可以同时加入上下文。
        // 顺带丢掉缺 sessionId / documentPath、以及消息级但没给 messageId 的无效项。
        let mut seen_objects: Vec<String> = Vec::new();
        let mut unique: Vec<(&CreateMountInput, Option<&str>)> = Vec::new();
        for (input, object_url) in &inputs {
            if input.document_path.is_empty() || input.session_id.is_empty() {
                continue;
            }
            if input.scope == MountScope::Message && input.message_id.is_none() {
                continue;
            }
            let key = object_url
                .as_deref()
                .map(object_identity)
                .unwrap_or_else(|| paths::to_forward_slashes(&input.document_path).to_lowercase());
            if seen_objects.contains(&key) {
                continue;
            }
            seen_objects.push(key);
            unique.push((input, object_url.as_deref()));
        }
        if unique.is_empty() {
            return Ok(Vec::new());
        }

        let mut index = self.load();
        let mut created = Vec::new();

        for (input, object_url) in unique {
            let relative = document_relative_path(&workspace, &input.document_path);
            // 同一会话 + 同一范围 + 同一消息 + 同一对象，只记一次。
            let duplicate = index.mounts.iter().any(|mount| {
                mount.session_id() == input.session_id
                    && mount.scope() == input.scope
                    && (input.scope == MountScope::Session
                        || mount.message_id() == input.message_id.as_deref())
                    && match object_url {
                        Some(url) => mount
                            .object_url()
                            .is_some_and(|stored| object_identity(stored) == object_identity(url)),
                        None => {
                            mount.object_url().is_none()
                                && mount.document_relative_path() == relative
                        }
                    }
            });
            if duplicate {
                continue;
            }

            // 键序对齐 TS 新建时的字面量顺序。历史数据里的另外两种顺序由保序模型原样保留。
            let mut fields = Map::new();
            fields.insert("id".into(), json!(new_mount_id()));
            fields.insert("scope".into(), json!(input.scope.as_str()));
            fields.insert("sessionId".into(), json!(input.session_id));
            if input.scope == MountScope::Message {
                if let Some(message_id) = &input.message_id {
                    fields.insert("messageId".into(), json!(message_id));
                }
            }
            if let Some(user_message_id) = &input.user_message_id {
                fields.insert("userMessageId".into(), json!(user_message_id));
            }
            fields.insert("documentPath".into(), json!(input.document_path));
            fields.insert("documentRelativePath".into(), json!(relative));
            if let Some(object_url) = object_url {
                fields.insert("objectUrl".into(), json!(object_url));
            }
            fields.insert("sessionTitle".into(), json!(input.session_title));
            fields.insert("snippet".into(), json!(input.snippet));
            fields.insert(
                "createdAt".into(),
                json!(input.created_at.unwrap_or_else(jstime::now_millis)),
            );

            let mount = AiDocumentMount::from_map(fields);
            index.mounts.push(mount.clone());
            created.push(mount);
        }

        if !created.is_empty() {
            self.initialize()?;
            self.save(index)?;
        }
        Ok(created)
    }

    fn validate_document_path(&self, reference: &ObjectReference) -> Result<(PathBuf, PathBuf)> {
        if !reference.is_document() || reference.kind == ObjectKind::Directory {
            bail!("AI 文档挂载只支持文件或具体文档块")
        }
        let root = self
            .workspace_path
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_path.clone());
        let path = reference
            .resolve_path(Some(&self.workspace_path))
            .context("AI 文档引用缺少路径")?;
        let existing = path
            .canonicalize()
            .with_context(|| format!("AI 文档不存在: {}", path.display()))?;
        if !crate::paths::path_is_within(&root, &existing) || !existing.is_file() {
            bail!("AI 文档引用超出当前工作区")
        }
        Ok((path, existing))
    }

    fn mounted_content_from_source(
        &self,
        value: &str,
        reference: &ObjectReference,
        path: &Path,
        source: &str,
    ) -> Result<MountedReferenceContent> {
        if reference.kind == ObjectKind::Block {
            let block_id = reference
                .block_id
                .clone()
                .context("文档块引用缺少 block id")?;
            let document = crate::document_blocks::document_from_source(path, source)
                .with_context(|| format!("解析文档块所在文件失败: {}", path.display()))?;
            let block = document
                .find_block(&block_id)
                .with_context(|| format!("文档中找不到文档块 {block_id}；可能已被删除"))?;
            if block.marker_span.is_none() {
                bail!("文档块 {block_id} 尚未持久化；请先显式转换文档")
            }
            return Ok(MountedReferenceContent {
                url: value.trim().to_owned(),
                path: path.to_path_buf(),
                block_id: Some(block_id),
                content: block.content.clone(),
                raw_content: block.raw_content.clone(),
                source_span: Some((block.source_span.start, block.source_span.end)),
            });
        }
        Ok(MountedReferenceContent {
            url: value.trim().to_owned(),
            path: path.to_path_buf(),
            block_id: None,
            raw_content: source.to_owned(),
            content: source.to_owned(),
            source_span: None,
        })
    }

    /// 读取挂载对象 URL 背后的当前源码。只记录文档路径的挂载返回
    /// 完整 UTF-8 源码；块挂载返回当前持久化的块内容和源码区间。
    /// 读取永远以本服务的工作区为根，伪造的绝对 URL 无法越过
    /// 普通 AI 文档挂载的同一条边界。
    pub fn read_reference(&self, value: &str) -> Result<MountedReferenceContent> {
        let reference = ObjectReference::parse(value).context("AI 文档引用无效")?;
        let (_, existing) = self.validate_document_path(&reference)?;
        let content = std::fs::read_to_string(&existing)
            .with_context(|| format!("读取 AI 文档失败: {}", existing.display()))?;
        self.mounted_content_from_source(value, &reference, &existing, &content)
    }

    /// 基于编辑器或宿主提供的源码快照解析引用。源路径必须
    /// canonicalize 到 URL 对应的工作区文件：调用方不能拿一个
    /// 合法块 URL 配一段不相干的脏文本；未保存的编辑仍可进入 AI 上下文。
    pub fn read_reference_with_source(
        &self,
        value: &str,
        source_path: &Path,
        source: &str,
    ) -> Result<MountedReferenceContent> {
        let reference = ObjectReference::parse(value).context("AI 文档引用无效")?;
        let (path, existing) = self.validate_document_path(&reference)?;
        let supplied = source_path
            .canonicalize()
            .with_context(|| format!("AI 文档快照路径无法解析: {}", source_path.display()))?;
        if supplied != existing {
            bail!("AI 文档快照路径与引用不一致")
        }
        self.mounted_content_from_source(value, &reference, &path, source)
    }

    /// read_mounted_reference 的变体，供已有实时编辑器/宿主快照的
    /// 调用方使用。持久化的挂载 URL 仍是身份，同时沿用同样的
    /// 路径与工作区校验。
    pub fn read_mounted_reference_with_source(
        &self,
        mount: &AiDocumentMount,
        source_path: &Path,
        source: &str,
    ) -> Result<MountedReferenceContent> {
        let url = mount.object_url().context("该 AI 挂载没有具体对象 URL")?;
        self.read_reference_with_source(url, source_path, source)
    }

    /// 解析挂载记录的具体对象。旧的只记录文档路径的挂载会明确报错；
    /// 它的调用方应改用 `document_path` 和既有的整文件上下文路径。
    pub fn read_mounted_reference(
        &self,
        mount: &AiDocumentMount,
    ) -> Result<MountedReferenceContent> {
        let url = mount.object_url().context("该 AI 挂载没有具体对象 URL")?;
        self.read_reference(url)
    }

    /// 某篇文档上的挂载，**按创建时间倒序**（最近的对话排在前面）。
    pub fn mounts_for_document(&self, document_path: &str) -> Vec<AiDocumentMount> {
        let index = self.load();
        let relative = document_relative_path(&self.workspace_str(), document_path);
        let mut found: Vec<AiDocumentMount> = index
            .mounts
            .iter()
            .filter(|mount| mount.document_relative_path() == relative)
            .cloned()
            .collect();
        found.sort_by_key(|b| std::cmp::Reverse(b.created_at()));
        found
    }

    pub fn remove_mount(&self, mount_id: &str) -> Result<()> {
        let mut index = self.load();
        index.mounts.retain(|mount| mount.id() != mount_id);
        self.save(index)?;
        Ok(())
    }

    pub fn remove_session_mounts(&self, session_id: &str) -> Result<()> {
        let mut index = self.load();
        index
            .mounts
            .retain(|mount| mount.session_id() != session_id);
        self.save(index)?;
        Ok(())
    }

    /// 删掉某会话里这些消息相关的挂载。
    ///
    /// `messageId` 与 `userMessageId` **任一命中**就删——删一轮对话时，
    /// 助手消息和触发它的用户消息是一对，只按其中一个删会留下孤儿挂载。
    pub fn remove_message_mounts(&self, session_id: &str, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        let mut index = self.load();
        index.mounts.retain(|mount| {
            if mount.session_id() != session_id || mount.scope() != MountScope::Message {
                return true;
            }
            let hit = |id: Option<&str>| id.is_some_and(|v| message_ids.iter().any(|m| m == v));
            !hit(mount.message_id()) && !hit(mount.user_message_id())
        });
        self.save(index)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
        service: AiDocumentMountService,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn raw(&self) -> String {
            std::fs::read_to_string(self.service.index_path()).unwrap()
        }
        fn write_raw(&self, body: &str) {
            std::fs::create_dir_all(&self.service.base_path).unwrap();
            std::fs::write(self.service.index_path(), body).unwrap();
        }
        fn doc(&self, relative: &str) -> String {
            format!(
                "{}/{relative}",
                paths::to_forward_slashes(&self.root.to_string_lossy())
            )
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-mounts-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let service = AiDocumentMountService::new(&root);
        Fixture { root, service }
    }

    fn input(f: &Fixture, session: &str, message: &str, relative: &str) -> CreateMountInput {
        CreateMountInput {
            session_id: session.into(),
            scope: MountScope::Message,
            message_id: Some(message.into()),
            user_message_id: Some(format!("{message}-user")),
            document_path: f.doc(relative),
            session_title: "会话标题".into(),
            snippet: "片段".into(),
            created_at: Some(1_000),
        }
    }

    // ---------- 相对路径 ----------

    #[test]
    fn relative_paths_are_computed_case_insensitively() {
        assert_eq!(
            document_relative_path("D:/mochi", "D:/mochi/知识库/a.mc"),
            "知识库/a.mc"
        );
        assert_eq!(
            document_relative_path("D:/mochi", "d:/MOCHI/知识库/a.mc"),
            "知识库/a.mc",
            "Windows 路径大小写不敏感"
        );
        assert_eq!(
            document_relative_path("D:/mochi/", "D:/mochi/a.mc"),
            "a.mc",
            "尾斜杠不影响"
        );
        assert_eq!(
            document_relative_path("D:/mochi", "D:\\mochi\\a.mc"),
            "a.mc",
            "反斜杠要归一"
        );
        assert_eq!(
            document_relative_path("D:/mochi", "D:/mochi"),
            "",
            "文档就是工作区根"
        );
    }

    /// 前缀像但不在里面的路径不该被截断成相对路径。
    #[test]
    fn a_document_outside_the_workspace_keeps_its_absolute_path() {
        assert_eq!(
            document_relative_path("D:/mochi", "D:/mochix/a.mc"),
            "D:/mochix/a.mc"
        );
        assert_eq!(
            document_relative_path("D:/mochi", "C:/别处/a.mc"),
            "C:/别处/a.mc"
        );
    }

    // ---------- 新增 ----------

    #[test]
    fn a_mount_is_created_with_the_expected_shape() {
        let f = fixture("add");
        let created = f
            .service
            .add_mounts(vec![input(&f, "s1", "m1", "知识库/a.mc")])
            .unwrap();

        assert_eq!(created.len(), 1);
        let mount = &created[0];
        assert!(mount.id().starts_with("ai-mount-"));
        assert_eq!(mount.session_id(), "s1");
        assert_eq!(mount.scope(), MountScope::Message);
        assert_eq!(mount.message_id(), Some("m1"));
        assert_eq!(mount.user_message_id(), Some("m1-user"));
        assert_eq!(mount.document_relative_path(), "知识库/a.mc");
        assert_eq!(mount.created_at(), 1_000);
    }

    #[test]
    fn concrete_block_mounts_keep_object_urls_and_read_current_content() {
        let f = fixture("block");
        let path = f.root.join("note.md");
        let id1 = "block_00000000-0000-4000-8000-000000000001";
        let id2 = "block_00000000-0000-4000-8000-000000000002";
        std::fs::write(
            &path,
            format!(
                "<!-- mochi:block {id1} -->\n\n第一块\n\n<!-- mochi:block {id2} -->\n\n第二块\n"
            ),
        )
        .unwrap();
        let url1 = crate::object_reference::build_block_reference(
            &path,
            id1,
            Some(&f.root),
            Some("第一块"),
        )
        .unwrap();
        let url2 = crate::object_reference::build_block_reference(&path, id2, Some(&f.root), None)
            .unwrap();
        let make = |url: String| CreateReferenceMountInput {
            session_id: "s1".into(),
            scope: MountScope::Message,
            message_id: Some("m1".into()),
            user_message_id: Some("m1-user".into()),
            document_path: path.to_string_lossy().replace('\\', "/"),
            object_url: url,
            session_title: "会话".into(),
            snippet: "问题".into(),
            created_at: Some(1),
        };
        let created = f
            .service
            .add_reference_mounts(vec![make(url1.clone()), make(url2)])
            .unwrap();
        assert_eq!(created.len(), 2);
        assert_eq!(created[0].object_url(), Some(url1.as_str()));
        assert_eq!(created[0].block_id().as_deref(), Some(id1));

        std::fs::write(
            &path,
            format!(
                "<!-- mochi:block {id1} -->\n\n第一块已更新\n\n<!-- mochi:block {id2} -->\n\n第二块\n"
            ),
        )
        .unwrap();
        let read = f.service.read_mounted_reference(&created[0]).unwrap();
        assert_eq!(read.block_id.as_deref(), Some(id1));
        assert!(read.content.contains("第一块已更新"));
        assert!(read.source_span.is_some());

        let dirty_source = format!(
            "<!-- mochi:block {id1} -->\n\n第一块未保存\n\n<!-- mochi:block {id2} -->\n\n第二块\n"
        );
        let dirty = f
            .service
            .read_reference_with_source(&url1, &path, &dirty_source)
            .unwrap();
        assert!(dirty.content.contains("第一块未保存"));
        assert!(dirty.raw_content.contains("第一块未保存"));
    }

    #[test]
    fn block_mount_reads_reject_unpersisted_ids_and_workspace_escape() {
        let f = fixture("block-errors");
        let path = f.root.join("note.md");
        std::fs::write(&path, "# 普通文档\n").unwrap();
        let id = "block_00000000-0000-4000-8000-000000000001";
        let url =
            crate::object_reference::build_block_reference(&path, id, Some(&f.root), None).unwrap();
        assert!(f.service.read_reference(&url).is_err());
        let outside = f.root.parent().unwrap().join("outside.md");
        std::fs::write(
            &outside,
            "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->\n\n外部\n",
        )
        .unwrap();
        let outside_url =
            crate::object_reference::build_block_reference(&outside, id, Some(&f.root), None)
                .unwrap();
        assert!(f.service.read_reference(&outside_url).is_err());
        let _ = std::fs::remove_file(outside);
    }

    #[test]
    fn the_same_document_is_not_mounted_twice_for_one_message() {
        let f = fixture("dedup");
        f.service
            .add_mounts(vec![input(&f, "s1", "m1", "a.mc")])
            .unwrap();
        let again = f
            .service
            .add_mounts(vec![input(&f, "s1", "m1", "a.mc")])
            .unwrap();

        assert!(again.is_empty(), "重复挂载应被跳过");
        assert_eq!(f.service.load().mounts.len(), 1);
    }

    /// 同一批入参里指向同一文档的只留第一条。
    #[test]
    fn duplicates_within_one_batch_are_collapsed() {
        let f = fixture("batch-dedup");
        let created = f
            .service
            .add_mounts(vec![
                input(&f, "s1", "m1", "a.mc"),
                CreateMountInput {
                    snippet: "另一段".into(),
                    ..input(&f, "s1", "m1", "a.mc")
                },
                // 大小写不同也算同一个文档
                CreateMountInput {
                    document_path: f.doc("A.MC"),
                    ..input(&f, "s1", "m1", "a.mc")
                },
                input(&f, "s1", "m1", "b.mc"),
            ])
            .unwrap();

        assert_eq!(created.len(), 2);
        assert_eq!(created[0].snippet(), "片段", "保留的是第一条");
        assert_eq!(created[1].document_relative_path(), "b.mc");
    }

    #[test]
    fn invalid_inputs_are_dropped() {
        let f = fixture("invalid");
        let created = f
            .service
            .add_mounts(vec![
                CreateMountInput {
                    document_path: String::new(),
                    ..input(&f, "s1", "m1", "a.mc")
                },
                CreateMountInput {
                    session_id: String::new(),
                    ..input(&f, "s1", "m1", "b.mc")
                },
                // 消息级却没给 messageId
                CreateMountInput {
                    message_id: None,
                    ..input(&f, "s1", "m1", "c.mc")
                },
            ])
            .unwrap();
        assert!(created.is_empty());
        assert!(!f.service.index_path().exists(), "一条都没建就不该写文件");
    }

    /// 会话级挂载不需要 messageId，也不进按消息分组的表。
    #[test]
    fn a_session_scoped_mount_carries_no_message_id() {
        let f = fixture("session-scope");
        let created = f
            .service
            .add_mounts(vec![CreateMountInput {
                scope: MountScope::Session,
                message_id: None,
                ..input(&f, "s1", "unused", "a.mc")
            }])
            .unwrap();

        assert_eq!(created[0].scope(), MountScope::Session);
        assert!(
            created[0].get("messageId").is_none(),
            "会话级不该带 messageId"
        );

        let index = f.service.load();
        assert!(index.by_session.contains_key("s1"));
        assert!(index.by_conversation.is_empty(), "会话级不进按消息分组的表");
    }

    // ---------- 派生表 ----------

    #[test]
    fn derived_tables_are_rebuilt_from_the_mounts() {
        let f = fixture("indexes");
        f.service
            .add_mounts(vec![
                input(&f, "s1", "m1", "a.mc"),
                input(&f, "s1", "m1", "b.mc"),
            ])
            .unwrap();
        f.service
            .add_mounts(vec![input(&f, "s2", "m9", "a.mc")])
            .unwrap();

        let index = f.service.load();
        assert_eq!(
            index.by_document["a.mc"].as_array().unwrap().len(),
            2,
            "两个会话都挂了 a.mc"
        );
        assert_eq!(index.by_document["b.mc"].as_array().unwrap().len(), 1);
        assert_eq!(index.by_session["s1"].as_array().unwrap().len(), 2);
        assert_eq!(
            index.by_conversation["s1"]["m1"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            index.by_conversation["s2"]["m9"].as_array().unwrap().len(),
            1
        );
    }

    /// 派生表的键按 mounts 顺序插入，不能按字典序排列，否则会与上游产生无谓的差异。
    #[test]
    fn derived_table_keys_follow_mount_order_not_alphabetical_order() {
        let f = fixture("order");
        for name in ["z.mc", "a.mc", "m.mc"] {
            f.service
                .add_mounts(vec![input(&f, "s1", "m1", name)])
                .unwrap();
        }
        let index = f.service.load();
        let keys: Vec<&String> = index.by_document.keys().collect();
        assert_eq!(
            keys,
            ["z.mc", "a.mc", "m.mc"],
            "JS 对象是插入序，不是字典序"
        );
    }

    // ---------- 保序与容错 ----------

    /// 真实数据里存在三种键顺序，读写一遍必须原样返回，不能重排。
    #[test]
    fn all_three_real_world_key_orders_round_trip_byte_for_byte() {
        let f = fixture("key-order");
        // 取自真实工作区的三种键顺序（`scope` 在末尾 / `scope` 在第二位 / 会话级缺少 `messageId`）
        let original = r#"{
  "version": 1,
  "mounts": [
    {
      "id": "ai-mount-1-aaaaaa",
      "sessionId": "s1",
      "messageId": "m1",
      "userMessageId": "u1",
      "documentPath": "D:/ws/a.mc",
      "documentRelativePath": "a.mc",
      "sessionTitle": "标题",
      "snippet": "片段",
      "createdAt": 1,
      "scope": "message"
    },
    {
      "id": "ai-mount-2-bbbbbb",
      "scope": "message",
      "sessionId": "s1",
      "messageId": "m2",
      "userMessageId": "u2",
      "documentPath": "D:/ws/b.mc",
      "documentRelativePath": "b.mc",
      "sessionTitle": "标题",
      "snippet": "片段",
      "createdAt": 2
    },
    {
      "id": "ai-mount-3-cccccc",
      "scope": "session",
      "sessionId": "s2",
      "documentPath": "D:/ws/c.mc",
      "documentRelativePath": "c.mc",
      "sessionTitle": "标题",
      "snippet": "片段",
      "createdAt": 3
    }
  ],
  "byDocument": {
    "a.mc": [
      "ai-mount-1-aaaaaa"
    ],
    "b.mc": [
      "ai-mount-2-bbbbbb"
    ],
    "c.mc": [
      "ai-mount-3-cccccc"
    ]
  },
  "bySession": {
    "s1": [
      "ai-mount-1-aaaaaa",
      "ai-mount-2-bbbbbb"
    ],
    "s2": [
      "ai-mount-3-cccccc"
    ]
  },
  "byConversation": {
    "s1": {
      "m1": [
        "ai-mount-1-aaaaaa"
      ],
      "m2": [
        "ai-mount-2-bbbbbb"
      ]
    }
  }
}"#;
        f.write_raw(original);
        let index = f.service.load();
        f.service.save(index).unwrap();

        assert_eq!(f.raw(), original, "读写一遍必须逐字节一致");
    }

    #[test]
    fn unknown_fields_on_a_mount_survive_a_round_trip() {
        let f = fixture("unknown");
        f.write_raw(
            r#"{"version":1,"mounts":[{"id":"m","sessionId":"s","documentPath":"p","documentRelativePath":"p","未来字段":{"x":1}}],"byDocument":{},"bySession":{},"byConversation":{}}"#,
        );
        let index = f.service.load();
        assert_eq!(index.mounts[0].get("未来字段").unwrap()["x"], 1);

        f.service.save(index).unwrap();
        assert!(f.raw().contains("未来字段"), "未知字段不能被静默丢弃");
    }

    /// 旧数据缺少 `scope` 字段时，会将其补在**末尾**；这正是磁盘上第一种键顺序的来源。
    #[test]
    fn a_missing_scope_is_appended_at_the_end_like_upstream() {
        let f = fixture("scope-append");
        f.write_raw(
            r#"{"version":1,"mounts":[{"id":"m","sessionId":"s","documentPath":"p","documentRelativePath":"p","createdAt":1}],"byDocument":{},"bySession":{},"byConversation":{}}"#,
        );
        let index = f.service.load();
        let keys: Vec<&String> = index.mounts[0].fields().keys().collect();
        assert_eq!(
            keys.last().unwrap().as_str(),
            "scope",
            "补的 scope 要在末尾"
        );
    }

    #[test]
    fn malformed_mounts_are_dropped_and_a_broken_file_yields_an_empty_index() {
        let f = fixture("malformed");
        f.write_raw(
            r#"{"version":1,"mounts":[{"id":"good","sessionId":"s","documentPath":"p","documentRelativePath":"p"},{"id":123},{"sessionId":"缺id"},null],"byDocument":{},"bySession":{},"byConversation":{}}"#,
        );
        let index = f.service.load();
        assert_eq!(index.mounts.len(), 1);
        assert_eq!(index.mounts[0].id(), "good");

        f.write_raw("{这不是 JSON");
        assert!(
            f.service.load().mounts.is_empty(),
            "损坏的索引不该让打开文档失败"
        );
    }

    #[test]
    fn the_index_file_has_no_trailing_newline() {
        let f = fixture("newline");
        f.service
            .add_mounts(vec![input(&f, "s1", "m1", "a.mc")])
            .unwrap();
        assert!(f.raw().ends_with('}'), ".mochi/ 下的文件不带尾换行");
    }

    #[test]
    fn initialize_never_clobbers_an_existing_index() {
        let f = fixture("init");
        f.service
            .add_mounts(vec![input(&f, "s1", "m1", "a.mc")])
            .unwrap();
        f.service.initialize().unwrap();
        assert_eq!(f.service.load().mounts.len(), 1, "已有数据不能被空索引覆盖");
    }

    // ---------- 查询与删除 ----------

    #[test]
    fn mounts_for_a_document_come_back_newest_first() {
        let f = fixture("query");
        for (i, message) in ["m1", "m2", "m3"].iter().enumerate() {
            f.service
                .add_mounts(vec![CreateMountInput {
                    created_at: Some(i as i64 * 100),
                    ..input(&f, "s1", message, "a.mc")
                }])
                .unwrap();
        }

        let found = f.service.mounts_for_document(&f.doc("a.mc"));
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].created_at(), 200, "最近的排最前");
        assert_eq!(found[2].created_at(), 0);

        // 用绝对路径的另一种大小写也要查得到
        assert_eq!(
            f.service.mounts_for_document(&f.doc("A.MC")).len(),
            0,
            "文件名大小写不同即不同文档"
        );
        assert!(f
            .service
            .mounts_for_document(&f.doc("没挂过.mc"))
            .is_empty());
    }

    #[test]
    fn removing_by_id_session_and_message() {
        let f = fixture("remove");
        f.service
            .add_mounts(vec![
                input(&f, "s1", "m1", "a.mc"),
                input(&f, "s1", "m2", "b.mc"),
                input(&f, "s2", "m3", "c.mc"),
            ])
            .unwrap();
        let first_id = f.service.load().mounts[0].id().to_owned();

        f.service.remove_mount(&first_id).unwrap();
        assert_eq!(f.service.load().mounts.len(), 2);

        f.service.remove_session_mounts("s2").unwrap();
        let left = f.service.load();
        assert_eq!(left.mounts.len(), 1);
        assert_eq!(left.mounts[0].session_id(), "s1");
        assert!(!left.by_session.contains_key("s2"), "派生表要跟着更新");
    }

    /// messageId 和 userMessageId 任一命中都要删——只按其中一个删会留下孤儿挂载。
    #[test]
    fn removing_a_round_matches_both_the_assistant_and_user_message_id() {
        let f = fixture("remove-round");
        f.service
            .add_mounts(vec![
                input(&f, "s1", "m1", "a.mc"), // userMessageId = m1-user
                input(&f, "s1", "m2", "b.mc"),
            ])
            .unwrap();

        // 只报用户消息 id，也要能删掉对应的助手侧挂载
        f.service
            .remove_message_mounts("s1", &["m1-user".to_string()])
            .unwrap();
        let left = f.service.load();
        assert_eq!(left.mounts.len(), 1);
        assert_eq!(left.mounts[0].message_id(), Some("m2"));
    }

    #[test]
    fn removing_with_an_empty_id_list_is_a_no_op() {
        let f = fixture("remove-empty");
        f.service
            .add_mounts(vec![input(&f, "s1", "m1", "a.mc")])
            .unwrap();
        f.service.remove_message_mounts("s1", &[]).unwrap();
        assert_eq!(f.service.load().mounts.len(), 1);
    }

    /// 会话级挂载不受「删某几条消息」影响——它不属于任何一条消息。
    #[test]
    fn session_scoped_mounts_survive_message_removal() {
        let f = fixture("remove-session-scope");
        f.service
            .add_mounts(vec![CreateMountInput {
                scope: MountScope::Session,
                message_id: None,
                ..input(&f, "s1", "unused", "a.mc")
            }])
            .unwrap();
        f.service
            .remove_message_mounts("s1", &["m1".into(), "m1-user".into()])
            .unwrap();
        assert_eq!(f.service.load().mounts.len(), 1);
    }
}
