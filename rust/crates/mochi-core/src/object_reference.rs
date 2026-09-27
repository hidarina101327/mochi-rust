//! 引用只保存 Mochi URL，不复制目标内容；尚无消息的会话使用 mochi://ai-session。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agenda::AgendaStore;
use crate::ai::locator::{safe_session_id, Locator};
use crate::ai::session::{AiSessionService, AiStoredMessage};
use crate::base::{format_cell_value, parse_base_document, BaseDocument};
use crate::files::FileService;
use crate::mochi_url::{self, ResourceKind, ResourceRef};

/// 选择器可以列出的对象类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Document,
    Directory,
    /// 已持久化的 Markdown 块，通过稳定的 `block_<UUID>` id 寻址。
    Block,
    PdfAnnotation,
    Record,
    Task,
    Event,
    Project,
    AiSession,
    AiMessage,
}

impl ObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "文档",
            Self::Directory => "文件夹",
            Self::Block => "文档块",
            Self::PdfAnnotation => "PDF 批注",
            Self::Record => "记录",
            Self::Task => "任务",
            Self::Event => "日程",
            Self::Project => "项目",
            Self::AiSession => "AI 会话",
            Self::AiMessage => "AI 对话",
        }
    }

    pub fn scope(self) -> ObjectScope {
        match self {
            Self::Document | Self::Directory | Self::Block | Self::PdfAnnotation => {
                ObjectScope::Files
            }
            Self::Record => ObjectScope::Records,
            Self::Task | Self::Event | Self::Project => ObjectScope::Schedule,
            Self::AiSession | Self::AiMessage => ObjectScope::Ai,
        }
    }
}

/// 选择器页签。「全部」一页对 AI 挂载和 base 单元格里的引用最实用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ObjectScope {
    #[default]
    All,
    Files,
    Records,
    Schedule,
    Ai,
}

impl ObjectScope {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Files,
        Self::Records,
        Self::Schedule,
        Self::Ai,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Files => "文档与资料",
            Self::Records => "表格记录",
            Self::Schedule => "日程与项目",
            Self::Ai => "AI 会话",
        }
    }

    pub fn accepts(self, kind: ObjectKind) -> bool {
        self == Self::All || self == kind.scope()
    }
}

/// Mochi URL 的类型化视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectReference {
    pub url: String,
    pub kind: ObjectKind,
    /// mochi://open 的 path，POSIX 形式。AI 引用可以没有。
    pub path: Option<String>,
    /// [`ObjectKind::Block`] 的持久化 `block_<UUID>` 身份。
    pub block_id: Option<String>,
    pub table_id: Option<String>,
    pub record_id: Option<String>,
    pub field_id: Option<String>,
    pub item_id: Option<String>,
    pub session_id: Option<String>,
    pub message_id: Option<String>,
    pub label: Option<String>,
    pub snippet: Option<String>,
}

impl ObjectReference {
    pub fn parse(value: &str) -> Option<Self> {
        let url = value.trim();
        if url.is_empty() || url.chars().any(char::is_whitespace) || url.contains('#') {
            return None;
        }
        if url
            .get(.."mochi://block?".len())
            .is_some_and(|head| head.eq_ignore_ascii_case("mochi://block?"))
        {
            return Self::parse_block(url);
        }
        if url
            .get(.."mochi://pdf-annotation?".len())
            .is_some_and(|head| head.eq_ignore_ascii_case("mochi://pdf-annotation?"))
        {
            return Self::parse_pdf_annotation(url);
        }
        if url
            .get(.."mochi://ai-locate?".len())
            .is_some_and(|head| head.eq_ignore_ascii_case("mochi://ai-locate?"))
        {
            return Self::parse_ai_message(url);
        }
        if url
            .get(.."mochi://ai-session?".len())
            .is_some_and(|head| head.eq_ignore_ascii_case("mochi://ai-session?"))
        {
            return Self::parse_ai_session(url);
        }
        Self::parse_open(url)
    }

    fn parse_ai_message(url: &str) -> Option<Self> {
        let locator = Locator::parse(url)?;
        if !safe_session_id(&locator.session_id) || !safe_component(&locator.message_id) {
            return None;
        }
        Some(Self {
            url: url.to_owned(),
            kind: ObjectKind::AiMessage,
            path: None,
            block_id: None,
            table_id: None,
            record_id: None,
            field_id: None,
            item_id: None,
            session_id: Some(locator.session_id),
            message_id: Some(locator.message_id),
            label: (!locator.title.is_empty()).then_some(locator.title),
            snippet: (!locator.snippet.is_empty()).then_some(locator.snippet),
        })
    }

    fn parse_ai_session(url: &str) -> Option<Self> {
        let (scheme, query) = url.split_once('?')?;
        if !scheme.eq_ignore_ascii_case("mochi://ai-session") || query.is_empty() {
            return None;
        }
        let fields = parse_query(query)?;
        let session = field_once(&fields, "session")?;
        if !safe_session_id(session) {
            return None;
        }
        let label = optional_field(&fields, "label").or_else(|| optional_field(&fields, "title"));
        let label = label.filter(|value| valid_component(value));
        Some(Self {
            url: url.to_owned(),
            kind: ObjectKind::AiSession,
            path: None,
            block_id: None,
            table_id: None,
            record_id: None,
            field_id: None,
            item_id: None,
            session_id: Some(session.to_owned()),
            message_id: None,
            label,
            snippet: optional_field(&fields, "snippet").filter(|value| valid_component(value)),
        })
    }

    fn parse_block(url: &str) -> Option<Self> {
        let (scheme, query) = url.split_once('?')?;
        if !scheme.eq_ignore_ascii_case("mochi://block") || query.is_empty() {
            return None;
        }
        let fields = parse_query(query)?;
        let path = field_once(&fields, "path")?.to_owned();
        let block_id = canonical_block_id(field_once(&fields, "id")?)?;
        if !valid_reference_path(&path) {
            return None;
        }
        let label = optional_field(&fields, "label").filter(|value| valid_component(value));
        Some(Self {
            url: url.to_owned(),
            kind: ObjectKind::Block,
            path: Some(path.replace('\\', "/")),
            block_id: Some(block_id),
            table_id: None,
            record_id: None,
            field_id: None,
            item_id: None,
            session_id: None,
            message_id: None,
            label,
            snippet: optional_field(&fields, "snippet").filter(|value| valid_component(value)),
        })
    }

    fn parse_pdf_annotation(url: &str) -> Option<Self> {
        let (scheme, query) = url.split_once('?')?;
        if !scheme.eq_ignore_ascii_case("mochi://pdf-annotation") || query.is_empty() {
            return None;
        }
        let fields = parse_query(query)?;
        let path = field_once(&fields, "path")?.to_owned().replace('\\', "/");
        let id = field_once(&fields, "id")?.to_owned();
        if !valid_reference_path(&path)
            || !path.to_ascii_lowercase().ends_with(".pdf")
            || !safe_component(&id)
        {
            return None;
        }
        Some(Self {
            url: url.to_owned(),
            kind: ObjectKind::PdfAnnotation,
            path: Some(path),
            block_id: None,
            table_id: None,
            record_id: None,
            field_id: None,
            item_id: Some(id),
            session_id: None,
            message_id: None,
            label: optional_field(&fields, "label").filter(|value| valid_component(value)),
            snippet: None,
        })
    }

    fn parse_open(url: &str) -> Option<Self> {
        let query = url.split_once('?')?.1;
        if !valid_percent_encoding(query) {
            return None;
        }
        let resource = mochi_url::parse_mochi_resource_url(url)?;
        let kind = if resource.table_id.is_some() || resource.record_id.is_some() {
            ObjectKind::Record
        } else {
            match resource.kind {
                ResourceKind::File => ObjectKind::Document,
                ResourceKind::Directory => ObjectKind::Directory,
                ResourceKind::Task => ObjectKind::Task,
                ResourceKind::Event => ObjectKind::Event,
                ResourceKind::Project => ObjectKind::Project,
                ResourceKind::AiSession => ObjectKind::AiSession,
            }
        };
        if !valid_resource_components(&resource) {
            return None;
        }
        match kind {
            ObjectKind::Record => {
                if resource.kind != ResourceKind::File
                    || !resource.path.to_ascii_lowercase().ends_with(".mcb")
                    || resource.table_id.is_none()
                    || resource.item_id.is_some()
                {
                    return None;
                }
            }
            ObjectKind::Task | ObjectKind::Event | ObjectKind::Project => {
                if resource.item_id.is_none()
                    || resource.table_id.is_some()
                    || resource.record_id.is_some()
                    || resource.field_id.is_some()
                {
                    return None;
                }
            }
            ObjectKind::AiSession => {
                if resource.item_id.as_deref().is_none()
                    || resource.table_id.is_some()
                    || resource.record_id.is_some()
                    || resource.field_id.is_some()
                {
                    return None;
                }
            }
            ObjectKind::Document | ObjectKind::Directory => {
                if resource.table_id.is_some()
                    || resource.record_id.is_some()
                    || resource.field_id.is_some()
                    || resource.item_id.is_some()
                {
                    return None;
                }
            }
            // `mochi://open` 携带不了块身份。块链接走上面专门的
            // `mochi://block` 解析；这个分支要显式写出来，
            // 免得资源链接被悄悄降级成文档块。
            ObjectKind::Block => return None,
            ObjectKind::PdfAnnotation => return None,
            ObjectKind::AiMessage => return None,
        }
        Some(Self {
            url: url.to_owned(),
            kind,
            path: Some(resource.path),
            block_id: None,
            table_id: resource.table_id,
            record_id: resource.record_id,
            field_id: resource.field_id,
            item_id: resource.item_id.clone(),
            session_id: (kind == ObjectKind::AiSession)
                .then(|| resource.item_id.clone())
                .flatten(),
            message_id: None,
            label: resource.label,
            snippet: None,
        })
    }

    pub fn display_label(&self) -> String {
        if let Some(label) = self.label.as_deref().filter(|value| !value.is_empty()) {
            return label.to_owned();
        }
        if let Some(path) = self.path.as_deref() {
            let name = path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(path);
            let name = name
                .strip_suffix(".md")
                .or_else(|| name.strip_suffix(".mc"))
                .unwrap_or(name);
            if let Some(id) = self
                .block_id
                .as_deref()
                .or(self.record_id.as_deref())
                .or(self.item_id.as_deref())
            {
                return format!("{name} · {id}");
            }
            return name.to_owned();
        }
        self.message_id
            .as_deref()
            .or(self.session_id.as_deref())
            .unwrap_or("未命名引用")
            .to_owned()
    }

    pub fn resolve_path(&self, workspace: Option<&Path>) -> Option<PathBuf> {
        let path = self.path.as_deref()?;
        let workspace = workspace.map(|path| path.to_string_lossy().into_owned());
        mochi_url::resolve_workspace_path(path, workspace.as_deref()).map(PathBuf::from)
    }

    pub fn is_document(&self) -> bool {
        matches!(
            self.kind,
            ObjectKind::Document | ObjectKind::Directory | ObjectKind::Block
        )
    }
}

/// 解析查询串，坏的百分号转义和重复键一律拒绝。网页端跑完
/// base 引用校验器之后，URLSearchParams 的实际契约与此一致。
fn parse_query(query: &str) -> Option<Vec<(String, String)>> {
    let mut seen = HashSet::new();
    let mut fields = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if !valid_percent_encoding(key) || !valid_percent_encoding(value) {
            return None;
        }
        let key = mochi_url::form_decode(key);
        let value = mochi_url::form_decode(value);
        if !seen.insert(key.clone()) {
            return None;
        }
        fields.push((key, value));
    }
    Some(fields)
}

fn field_once<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn optional_field(fields: &[(String, String)], key: &str) -> Option<String> {
    field_once(fields, key).map(str::to_owned)
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.iter().enumerate().any(|(i, byte)| {
        *byte == b'%'
            && !bytes
                .get(i + 1..i + 3)
                .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    })
}

fn valid_component(value: &str) -> bool {
    !value.trim().is_empty()
        && !value
            .chars()
            .any(|c| c <= '\u{1f}' || c == '\u{7f}' || c == '\u{fffd}')
}

/// 块 ID 由 `mochi-blocks` 生成，形如 `block_<UUID>`。URL 校验不依赖那个
/// 可选 crate，核心引用解析器因此能被无头环境使用；需要类型化模型的
/// 调用方可用 `mochi_blocks::model::BlockId` 解析返回的字符串。
fn canonical_block_id(value: &str) -> Option<String> {
    let uuid = value.strip_prefix("block_")?;
    if uuid.len() != 36
        || !uuid.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return None;
    }
    Some(format!("block_{}", uuid.to_ascii_lowercase()))
}

fn valid_reference_path(path: &str) -> bool {
    let path = path.replace('\\', "/");
    valid_component(path.as_str()) && !path.split('/').any(|part| part == "..")
}

fn safe_component(value: &str) -> bool {
    valid_component(value)
        && value.len() <= 240
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !value.chars().any(|c| "<>:\\\"/\\\\|?*".contains(c))
}

fn valid_resource_components(resource: &ResourceRef) -> bool {
    let valid = |value: &str| valid_component(value);
    valid(&resource.path)
        && !resource.path.split('/').any(|part| part == "..")
        && resource
            .table_id
            .as_deref()
            .into_iter()
            .chain(resource.record_id.as_deref())
            .chain(resource.field_id.as_deref())
            .chain(resource.item_id.as_deref())
            .chain(resource.label.as_deref())
            .all(valid)
}

/// 按既有 URL 契约构造文档或目录引用。
pub fn build_document_reference(
    path: &Path,
    kind: ObjectKind,
    workspace: Option<&Path>,
    label: Option<&str>,
) -> String {
    let resource_kind = if kind == ObjectKind::Directory {
        ResourceKind::Directory
    } else {
        ResourceKind::File
    };
    append_label(
        mochi_url::build_mochi_resource_url(path, resource_kind, workspace),
        label,
    )
}

/// 为已持久化的 Markdown 块构造稳定 URL。块 URL 只带文档路径和块身份；
/// 块内容始终在打开或挂载时从文档里现读。
pub fn build_block_reference(
    path: &Path,
    block_id: impl AsRef<str>,
    workspace: Option<&Path>,
    label: Option<&str>,
) -> Option<String> {
    let block_id = canonical_block_id(block_id.as_ref())?;
    let url = mochi_url::build_mochi_block_url(path, &block_id, workspace);
    Some(append_label(url, label))
}

/// 带显式 URL 后缀的别名，给在同一个模块里构造多类对象的调用方用。
pub fn build_block_reference_url(
    path: &Path,
    block_id: impl AsRef<str>,
    workspace: Option<&Path>,
    label: Option<&str>,
) -> Option<String> {
    build_block_reference(path, block_id, workspace, label)
}

pub fn build_record_reference(
    path: &Path,
    table_id: &str,
    record_id: &str,
    field_id: Option<&str>,
    workspace: Option<&Path>,
    label: Option<&str>,
) -> String {
    let url = mochi_url::build_mochi_record_url(path, table_id, record_id, field_id, workspace);
    append_label(url, label)
}

pub fn build_schedule_reference(
    schedule_root: &Path,
    kind: ObjectKind,
    item_id: &str,
    workspace: Option<&Path>,
    label: Option<&str>,
) -> String {
    let kind_name = match kind {
        ObjectKind::Task => "task",
        ObjectKind::Event => "event",
        ObjectKind::Project => "project",
        _ => "event",
    };
    let resource_kind = match kind_name {
        "task" => ResourceKind::Task,
        "project" => ResourceKind::Project,
        _ => ResourceKind::Event,
    };
    let mut url = mochi_url::build_mochi_resource_url(schedule_root, resource_kind, workspace);
    url.push_str("&item=");
    url.push_str(&mochi_url::form_encode(item_id));
    if let Some(label) = label.filter(|value| !value.is_empty()) {
        url.push_str("&label=");
        url.push_str(&mochi_url::form_encode(label));
    }
    url
}

pub fn build_ai_session_reference(session_id: &str, label: Option<&str>) -> Option<String> {
    if !safe_session_id(session_id) {
        return None;
    }
    // 会话没有文件系统路径。保留 AI locator 系列专用的形式；
    // 文档里已经写出的旧式 `path=ai&kind=ai-session&item=…`
    // 链接，ObjectReference 也继续接受。
    let mut url = "mochi://ai-session?session=".to_owned();
    url.push_str(&mochi_url::form_encode(session_id));
    if let Some(label) = label.filter(|value| !value.is_empty()) {
        url.push_str("&label=");
        url.push_str(&mochi_url::form_encode(label));
    }
    Some(url)
}

/// 添加或替换文档块使用的呈现提示。`view` 参数刻意只当元数据：
/// 解析和打开路由仍按原来的对象身份走，不关心它显示成卡片还是行内文本。
pub fn with_view(url: &str, text_style: bool) -> Option<String> {
    let url = url.trim();
    ObjectReference::parse(url)?;
    let (head, query) = url.split_once('?')?;
    let mut fields = query
        .split('&')
        .filter(|pair| {
            pair.split_once('=')
                .map(|(key, _)| mochi_url::form_decode(key) != "view")
                .unwrap_or(true)
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    fields.push(format!("view={}", if text_style { "text" } else { "card" }));
    Some(format!("{head}?{}", fields.join("&")))
}

pub fn build_ai_message_reference(
    session_id: &str,
    message_id: &str,
    title: Option<&str>,
    snippet: Option<&str>,
) -> Option<String> {
    if !safe_session_id(session_id) || !safe_component(message_id) {
        return None;
    }
    let locator = Locator {
        session_id: session_id.to_owned(),
        message_id: message_id.to_owned(),
        title: title.unwrap_or_default().to_owned(),
        snippet: snippet.unwrap_or_default().to_owned(),
    };
    Some(locator.to_url())
}

fn append_label(mut url: String, label: Option<&str>) -> String {
    if let Some(label) = label.filter(|value| !value.is_empty()) {
        url.push_str("&label=");
        url.push_str(&mochi_url::form_encode(label));
    }
    url
}

/// 选择器的一行。URL 和文案放在一起，各处不用重拼标签，
/// 选中值也能逐字节往返。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectCandidate {
    pub url: String,
    pub title: String,
    pub description: String,
    pub kind: ObjectKind,
}

/// `mochi://block` URL 背后从源码读出的目标。URL 本身才是持久化的值；
/// 这里只是导航、预览和 AI 组装上下文时临时读出来的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBlock {
    pub url: String,
    pub path: PathBuf,
    pub block_id: String,
    pub content: String,
    pub raw_content: String,
    pub source_span: (usize, usize),
}

/// 解析并读取一个已持久化的块。URL 看着合法但文档里没有对应标记的，
/// 一律拒绝；导入普通文档绝不会为了凑合一条链接而凭空生成 ID。
pub fn resolve_block_reference(value: &str, workspace: Option<&Path>) -> Result<ResolvedBlock> {
    let reference = ObjectReference::parse(value).context("无效的文档块引用")?;
    ensure_block_reference(&reference)?;
    let path = reference
        .resolve_path(workspace)
        .context("文档块引用缺少可解析路径")?;
    if let Some(root) = workspace {
        ensure_path_within_workspace(root, &path)?;
    }
    let source = std::fs::read_to_string(&path)
        .with_context(|| format!("读取文档块所在文件失败: {}", path.display()))?;
    let block_id = reference
        .block_id
        .clone()
        .context("文档块引用缺少 block id")?;
    let document = crate::document_blocks::document_from_source(&path, &source)
        .with_context(|| format!("解析文档块所在文件失败: {}", path.display()))?;
    let block = document
        .find_block(&block_id)
        .with_context(|| format!("文档中找不到文档块 {block_id}；可能已被删除"))?;
    // `Document::import` 会给没有标记的块一个临时 ID，编辑器靠它渲染。
    // URL 只有在身份真正持久化到源码标记里时才允许解析成功。
    if block.marker_span.is_none() {
        bail!("文档块 {block_id} 尚未持久化；请先显式转换文档")
    }
    let span = (block.source_span.start, block.source_span.end);
    let raw_content = block.raw_content.clone();
    let content = block.content.clone();
    Ok(ResolvedBlock {
        url: value.trim().to_owned(),
        path,
        block_id,
        content,
        raw_content,
        source_span: span,
    })
}

fn ensure_block_reference(reference: &ObjectReference) -> Result<()> {
    if reference.kind != ObjectKind::Block {
        bail!("引用不是文档块")
    }
    if reference.path.is_none() || reference.block_id.is_none() {
        bail!("文档块引用不完整")
    }
    Ok(())
}

fn ensure_path_within_workspace(workspace: &Path, target: &Path) -> Result<()> {
    let workspace = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let target_existing = std::fs::canonicalize(target)
        .with_context(|| format!("路径无法解析: {}", target.display()))?;
    if target_existing.starts_with(&workspace) {
        Ok(())
    } else {
        bail!("文档块引用超出当前工作区")
    }
}

impl ObjectCandidate {
    pub fn new(
        url: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        kind: ObjectKind,
    ) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            description: description.into(),
            kind,
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || format!("{} {} {}", self.title, self.description, self.url)
                .to_lowercase()
                .contains(&query)
    }
}

/// 枚举当前工作区里可挂载、可引用的全部对象。刻意做成同步接口：
/// 调用方可以把它交给现有的文件工作线程处理，同时由 UI 显示加载状态。
pub fn collect_candidates(
    workspace: &Path,
    current_path: Option<&Path>,
    current_document: Option<&BaseDocument>,
) -> Vec<ObjectCandidate> {
    let mut out = Vec::new();
    let mut files = Vec::new();
    collect_files(workspace, &mut files, 0);
    files.sort();
    for path in files {
        let Some(relative) = mochi_url::to_workspace_relative_path(
            &path.to_string_lossy(),
            Some(&workspace.to_string_lossy()),
        ) else {
            continue;
        };
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative.clone());
        let kind = if path.is_dir() {
            ObjectKind::Directory
        } else {
            ObjectKind::Document
        };
        let url = build_document_reference(&path, kind, Some(workspace), None);
        out.push(ObjectCandidate::new(url, title, relative.clone(), kind));

        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mcb"))
            && std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 10 * 1024 * 1024)
        {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(document) = parse_base_document(&raw) {
                    append_record_candidates(&mut out, &path, workspace, &document);
                }
            }
        }

        // 块引用是主动开启的：只有已经写有持久化标记注释的文档才会被索引。
        // 打开选择器绝不能顺带给普通 Markdown 文件分配 ID 或改写它。
        if is_markdown_document(&path) {
            append_block_candidates(&mut out, &path, workspace, &relative);
        }
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            append_pdf_annotation_candidates(&mut out, &path, &relative);
        }
    }

    if let (Some(path), Some(document)) = (current_path, current_document) {
        replace_record_candidates(&mut out, path, workspace, document);
    }
    append_schedule_candidates(&mut out, workspace);
    append_ai_candidates(&mut out, workspace);
    dedup_candidates(out)
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > 16 || files.len() >= 5000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || [
                "node_modules",
                "target",
                "dist",
                "build",
                "coverage",
                "vendor",
                "schedule",
            ]
            .contains(&name.as_str())
        {
            continue;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_files(&path, files, depth + 1);
        } else if file_type.is_file() {
            files.push(path);
        }
    }
}

/// 叠加当前编辑器的记录，且不把未保存的数据漏进共享快照。
pub fn replace_record_candidates(
    out: &mut Vec<ObjectCandidate>,
    path: &Path,
    workspace: &Path,
    document: &BaseDocument,
) {
    let relative = mochi_url::to_workspace_relative_path(
        &path.to_string_lossy(),
        Some(&workspace.to_string_lossy()),
    );
    out.retain(|candidate| {
        candidate.kind != ObjectKind::Record
            || !ObjectReference::parse(&candidate.url)
                .is_some_and(|reference| reference.path == relative)
    });
    append_record_candidates(out, path, workspace, document);
}

fn append_record_candidates(
    out: &mut Vec<ObjectCandidate>,
    path: &Path,
    workspace: &Path,
    document: &BaseDocument,
) {
    for table in &document.tables {
        let field = table.fields.first();
        for record in &table.records {
            let title = field
                .map(|field| {
                    format_cell_value(field, record.values.get(&field.id).unwrap_or(&Value::Null))
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| record.id.clone());
            let url = build_record_reference(
                path,
                &table.id,
                &record.id,
                None,
                Some(workspace),
                Some(&title),
            );
            out.push(ObjectCandidate::new(
                url,
                title,
                table.name.clone(),
                ObjectKind::Record,
            ));
        }
    }
}

fn is_markdown_document(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        matches!(
            extension.to_string_lossy().to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mc"
        )
    })
}

fn append_pdf_annotation_candidates(out: &mut Vec<ObjectCandidate>, path: &Path, relative: &str) {
    for annotation in crate::sidecars::load_pdf_annotations(&path.to_string_lossy()).annotations {
        let label = annotation
            .text
            .clone()
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| format!("第 {} 页批注", annotation.page));
        let url = format!(
            "mochi://pdf-annotation?path={}&id={}&label={}",
            mochi_url::form_encode(relative),
            mochi_url::form_encode(&annotation.id),
            mochi_url::form_encode(&label)
        );
        out.push(ObjectCandidate::new(
            url,
            label,
            relative,
            ObjectKind::PdfAnnotation,
        ));
    }
}

fn block_title(snippet: &str, id: &str) -> String {
    let title = snippet
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches(['#', '-', '*', '+'])
        .trim();
    if title.is_empty() {
        id.to_owned()
    } else {
        title.chars().take(120).collect()
    }
}

fn append_block_candidates(
    out: &mut Vec<ObjectCandidate>,
    path: &Path,
    workspace: &Path,
    relative: &str,
) {
    let Ok(source) = std::fs::read_to_string(path) else {
        return;
    };
    // 导入是只读的。没标记的块可能拿到临时 ID 供编辑器渲染，
    // 所以只有带持久化标记的记录才会作为可搬移对象暴露出去。
    let Ok(document) = crate::document_blocks::document_from_source(path, &source) else {
        return;
    };
    for block in document
        .blocks()
        .iter()
        .filter(|block| block.marker_span.is_some())
    {
        let id = block.id.as_str();
        let Some(url) = build_block_reference(path, id, Some(workspace), None) else {
            continue;
        };
        let snippet = block.content.trim();
        let title = block_title(snippet, id);
        let description = if snippet.is_empty() {
            format!("{relative} · {id}")
        } else {
            format!(
                "{relative} · {id} · {}",
                snippet.chars().take(240).collect::<String>()
            )
        };
        out.push(ObjectCandidate::new(
            url,
            title,
            description,
            ObjectKind::Block,
        ));
    }
}

fn append_schedule_candidates(out: &mut Vec<ObjectCandidate>, workspace: &Path) {
    let store = AgendaStore::new(workspace);
    let Ok(data) = store.load() else {
        return;
    };
    let root = store.dir();
    let mut push = |kind: ObjectKind, id: &str, title: &str, description: &str| {
        let url = build_schedule_reference(&root, kind, id, Some(workspace), Some(title));
        out.push(ObjectCandidate::new(
            url,
            title.to_owned(),
            description,
            kind,
        ));
    };
    let visible = |deleted: &Option<String>, archived: &Option<String>| {
        deleted.is_none() && archived.is_none()
    };
    for task in data
        .tasks
        .iter()
        .filter(|t| visible(&t.deleted_at, &t.archived_at))
    {
        push(ObjectKind::Task, &task.id, &task.title, "待办任务");
    }
    for entry in data.entries.iter().filter(|e| e.task_id.is_none()) {
        push(
            ObjectKind::Event,
            &entry.id,
            &data.entry_title(entry),
            "日程",
        );
    }
    for goal in data
        .goals
        .iter()
        .filter(|g| visible(&g.deleted_at, &g.archived_at))
    {
        push(ObjectKind::Project, &goal.id, &goal.title, "目标");
    }
    for wish in data
        .wishes
        .iter()
        .filter(|w| visible(&w.deleted_at, &w.archived_at))
    {
        push(ObjectKind::Project, &wish.id, &wish.title, "愿望");
    }
    for project in data
        .projects
        .iter()
        .filter(|p| visible(&p.deleted_at, &p.archived_at))
    {
        push(ObjectKind::Project, &project.id, &project.name, "项目");
    }
}

fn append_ai_candidates(out: &mut Vec<ObjectCandidate>, workspace: &Path) {
    let service = AiSessionService::new(workspace);
    let index = service.load_index();
    for session in index
        .sessions
        .into_iter()
        .filter(|session| safe_session_id(&session.id))
    {
        let title = if session.title.trim().is_empty() {
            session.id.clone()
        } else {
            session.title.clone()
        };
        let Some(url) = build_ai_session_reference(&session.id, Some(&title)) else {
            continue;
        };
        out.push(ObjectCandidate::new(
            url,
            title.clone(),
            "AI 会话",
            ObjectKind::AiSession,
        ));
        let Some(conversation) = service.load_session(&session.id) else {
            continue;
        };
        for message in conversation.messages.into_iter().filter(ai_message_visible) {
            let Some(message_id) = message.id() else {
                continue;
            };
            let snippet = message.content().chars().take(120).collect::<String>();
            let Some(url) =
                build_ai_message_reference(&session.id, message_id, Some(&title), Some(&snippet))
            else {
                continue;
            };
            let message_title = if message.role() == "user" {
                "提问"
            } else {
                "回答"
            };
            out.push(ObjectCandidate::new(
                url,
                format!("{title} · {message_title}"),
                snippet,
                ObjectKind::AiMessage,
            ));
        }
    }
}

fn ai_message_visible(message: &AiStoredMessage) -> bool {
    crate::ai::locator::visible(message)
}

fn dedup_candidates(candidates: Vec<ObjectCandidate>) -> Vec<ObjectCandidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.url.clone()))
        .collect()
}

pub const TEMPORARY_DOCUMENTS_SUFFIX: &str = ".documents";

/// .mcb 文件旁边的同目录文件夹，存放表格专属的临时文档。
pub fn temporary_documents_dir(base_path: &Path) -> Result<PathBuf> {
    let parent = base_path.parent().context("多维表格没有父目录")?;
    let stem = base_path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .context("无法确定多维表格名称")?;
    Ok(parent.join(format!(
        "{}{}",
        stem.to_string_lossy(),
        TEMPORARY_DOCUMENTS_SUFFIX
    )))
}

/// 在 base 文件旁边新建临时文档。扩展名跟 simpleDocumentMode 走（.md），
/// 否则用 Mochi 富文档格式（.mc）。
pub fn create_temporary_document(
    base_path: &Path,
    title: &str,
    simple_document_mode: bool,
) -> Result<PathBuf> {
    let directory = temporary_documents_dir(base_path)?;
    std::fs::create_dir_all(&directory)?;
    let title = sanitize_document_title(title);
    let extension = if simple_document_mode { "md" } else { "mc" };
    let stem = if title.is_empty() {
        "临时文档"
    } else {
        title.as_str()
    };
    for index in 0..10_000usize {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let path = directory.join(format!("{stem}{suffix}.{extension}"));
        if path.exists() {
            continue;
        }
        let content = format!("# {stem}\n\n");
        FileService::new().write_file_safe(&path, &content)?;
        return Ok(path);
    }
    bail!("临时文档名称已达到上限")
}

/// 按确定顺序列出表格专属的临时文档。
pub fn list_temporary_documents(base_path: &Path) -> Vec<PathBuf> {
    let Ok(directory) = temporary_documents_dir(base_path) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths = entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_file())
                .map(|_| entry.path())
        })
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("mc")
            })
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn sanitize_document_title(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_control()
                || ['<', '>', ':', '\"', '/', '\\', '|', '?', '*'].contains(&character)
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn ai_message_references_reject_non_session_paths() {
        assert!(
            super::ObjectReference::parse("mochi://ai-locate?session=..%2Foutside&message=m")
                .is_none()
        );
        assert!(super::ObjectReference::parse("MOCHI://AI-LOCATE?session=s&message=m").is_some());
    }
    use super::*;
    use crate::base::create_base_document;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn workspace(tag: &str) -> PathBuf {
        let index = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mochi-object-reference-{tag}-{index}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn existing_open_and_ai_locator_links_round_trip_into_one_model() {
        let file = Path::new("D:/workspace/notes/学习.md");
        let url = build_document_reference(
            file,
            ObjectKind::Document,
            Some(Path::new("D:/workspace")),
            Some("学习"),
        );
        let reference = ObjectReference::parse(&url).unwrap();
        assert_eq!(reference.kind, ObjectKind::Document);
        assert_eq!(reference.path.as_deref(), Some("notes/学习.md"));
        assert_eq!(reference.display_label(), "学习");

        let ai =
            build_ai_message_reference("session-1", "message-1", Some("会话"), Some("回答片段"))
                .unwrap();
        let ai_reference = ObjectReference::parse(&ai).unwrap();
        assert_eq!(ai_reference.kind, ObjectKind::AiMessage);
        assert_eq!(ai_reference.session_id.as_deref(), Some("session-1"));
        assert_eq!(ai_reference.message_id.as_deref(), Some("message-1"));
        assert_eq!(ai_reference.snippet.as_deref(), Some("回答片段"));
    }

    #[test]
    fn ai_session_links_are_distinct_from_message_locators() {
        let url = build_ai_session_reference("s-1", Some("会话一")).unwrap();
        assert_eq!(
            url,
            "mochi://ai-session?session=s-1&label=%E4%BC%9A%E8%AF%9D%E4%B8%80"
        );
        let reference = ObjectReference::parse(&url).unwrap();
        assert_eq!(reference.kind, ObjectKind::AiSession);
        assert_eq!(reference.session_id.as_deref(), Some("s-1"));
        assert_eq!(reference.label.as_deref(), Some("会话一"));
        assert!(ObjectReference::parse("mochi://ai-session?session=s-1&session=s-2").is_none());
        let open = "mochi://open?path=ai&kind=ai-session&item=s-1&view=text";
        assert_eq!(
            ObjectReference::parse(open).unwrap().kind,
            ObjectKind::AiSession
        );
    }

    #[test]
    fn block_urls_round_trip_with_a_workspace_relative_path_and_stable_id() {
        let id = "block_00000000-0000-4000-8000-000000000001";
        let url = build_block_reference(
            Path::new("D:/workspace/notes/计划.md"),
            id,
            Some(Path::new("D:/workspace")),
            Some("发布计划"),
        )
        .unwrap();
        assert_eq!(
            url,
            "mochi://block?path=notes%2F%E8%AE%A1%E5%88%92.md&id=block_00000000-0000-4000-8000-000000000001&label=%E5%8F%91%E5%B8%83%E8%AE%A1%E5%88%92"
        );
        let reference = ObjectReference::parse(&url).unwrap();
        assert_eq!(reference.kind, ObjectKind::Block);
        assert_eq!(reference.path.as_deref(), Some("notes/计划.md"));
        assert_eq!(reference.block_id.as_deref(), Some(id));
        assert!(reference.is_document());
        assert_eq!(reference.display_label(), "发布计划");
    }

    #[test]
    fn block_urls_reject_bad_ids_and_path_traversal_without_touching_files() {
        for value in [
            "mochi://block?path=notes%2Fa.md&id=block_bad",
            "mochi://block?path=notes%2F..%2Fsecret.md&id=block_00000000-0000-4000-8000-000000000001",
            "mochi://block?path=notes%2Fa.md&id=block_00000000-0000-4000-8000-000000000001&id=block_00000000-0000-4000-8000-000000000002",
        ] {
            assert!(ObjectReference::parse(value).is_none(), "accepted {value}");
        }
    }

    #[test]
    fn block_candidates_only_include_documents_with_persisted_markers() {
        let root = workspace("blocks");
        let marked = root.join("marked.md");
        let plain = root.join("plain.md");
        let id = "block_00000000-0000-4000-8000-000000000001";
        std::fs::write(
            &marked,
            format!("<!-- mochi:block {id} -->\n\n# 发布计划\n\n正文\n"),
        )
        .unwrap();
        std::fs::write(&plain, "# 没有块 ID\n\n正文\n").unwrap();
        let candidates = collect_candidates(&root, None, None);
        let blocks = candidates
            .iter()
            .filter(|candidate| candidate.kind == ObjectKind::Block)
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].url.contains("marked.md"));
        assert!(blocks[0].description.contains("发布计划"));
        assert!(!candidates.iter().any(|candidate| {
            candidate.kind == ObjectKind::Block && candidate.url.contains("plain.md")
        }));
        assert_eq!(
            std::fs::read_to_string(&plain).unwrap(),
            "# 没有块 ID\n\n正文\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn resolving_a_block_reads_the_current_source_and_rejects_missing_markers() {
        let root = workspace("resolve-block");
        let path = root.join("notes.md");
        let id = "block_00000000-0000-4000-8000-000000000001";
        std::fs::write(
            &path,
            format!("<!-- mochi:block {id} -->\n\n# 当前内容\n\n第二段\n"),
        )
        .unwrap();
        let url = build_block_reference(&path, id, Some(&root), None).unwrap();
        let resolved = resolve_block_reference(&url, Some(&root)).unwrap();
        assert_eq!(resolved.block_id, id);
        assert!(resolved.content.contains("当前内容"));
        assert!(resolved.raw_content.contains("当前内容"));
        let missing = build_block_reference(
            &path,
            "block_00000000-0000-4000-8000-000000000002",
            Some(&root),
            None,
        )
        .unwrap();
        assert!(resolve_block_reference(&missing, Some(&root)).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn view_metadata_changes_presentation_without_changing_identity() {
        let url = "mochi://open?path=notes%2Fa.md&label=旧标题&future=x%2Fy";
        let text = with_view(url, true).unwrap();
        assert!(text.ends_with("&view=text"));
        assert!(text.contains("future=x%2Fy"));
        assert_eq!(
            ObjectReference::parse(&text).unwrap().path.as_deref(),
            Some("notes/a.md")
        );
        let card = with_view(&text, false).unwrap();
        assert!(card.ends_with("&view=card"));
        assert!(!card.contains("view=text"));
        assert!(with_view(url, true).is_some());
        assert!(with_view("https://example.com", true).is_none());
    }

    #[test]
    fn malformed_and_traversal_links_are_rejected() {
        for value in [
            "mochi://open?path=notes%2F..%2Fsecret.md",
            "mochi://open?path=notes%2Fsecret.md&table=t",
            "mochi://ai-session?session=../outside",
            "mochi://ai-locate?session=s&message=../outside",
            "mochi://open?path=a%ZZ.md",
        ] {
            assert!(ObjectReference::parse(value).is_none(), "accepted {value}");
        }
    }

    #[test]
    fn temporary_documents_live_in_a_sibling_folder_and_follow_mode() {
        let root = workspace("temporary");
        let base = root.join("学习.mcb");
        std::fs::write(
            &base,
            crate::base::serialize_base_document(&create_base_document()).unwrap(),
        )
        .unwrap();
        let plain = create_temporary_document(&base, "单元测试", true).unwrap();
        let rich = create_temporary_document(&base, "单元测试", false).unwrap();
        assert_eq!(plain.parent().unwrap(), root.join("学习.documents"));
        assert_eq!(plain.extension().unwrap(), "md");
        assert_eq!(std::fs::read_to_string(&plain).unwrap(), "# 单元测试\n\n");
        assert_eq!(rich.extension().unwrap(), "mc");
        assert_eq!(list_temporary_documents(&base), vec![rich, plain]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn candidate_search_matches_title_description_and_scope() {
        let candidate = ObjectCandidate::new(
            "mochi://open?path=notes%2Fa.md",
            "学习笔记",
            "notes/a.md",
            ObjectKind::Document,
        );
        assert!(candidate.matches("笔记"));
        assert!(ObjectScope::Files.accepts(candidate.kind));
        assert!(!ObjectScope::Schedule.accepts(candidate.kind));
    }
}
