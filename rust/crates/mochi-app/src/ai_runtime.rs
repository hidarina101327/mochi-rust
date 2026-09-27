//! AppHost 向工具提供 UI 状态快照，后台 Agent 事件投回 UI 线程。
//! Provider 设置与 Electron 共用 mochi/settings.json；Windows 密钥使用 CurrentUser DPAPI。

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use mochi_core::agenda::AgendaStore;
use mochi_core::ai::agent_inbox::AgentInboxService;
use mochi_core::ai::agent_runner::{self, AgentEvent, AgentRequest, AgentTraceStep};
use mochi_core::ai::models::{AiMessage, AiProvider};
use mochi_core::ai::permission::AiPermissionService;
use mochi_core::ai::service::AiService;
use mochi_core::ai::tools::host::{
    ActiveDocument, BlockEditProposal, FileChange, PendingFileOperation, ToolHost,
};
use mochi_core::ai::tools::registry::{self, ToolServices};
use mochi_core::app_settings::AppSettings;
use mochi_core::git::GitService;
use mochi_core::memory::MemoryStore;
use mochi_core::metadata_index::MetadataIndexService;
use mochi_core::search::SearchService;
use mochi_core::settings::SettingsService;

// ---------- 提供方 ----------

pub const KEY_NAME: &str = "ai.provider.name";
pub const KEY_BASE_URL: &str = "ai.provider.baseUrl";
pub const KEY_MODEL: &str = "ai.provider.model";
pub const KEY_API_KEY: &str = "ai.provider.apiKey";

/// 读取设置中的服务方配置。缺少 baseUrl 或 model 时，视为尚未配置。
pub fn load_provider(settings: &SettingsService) -> Option<AiProvider> {
    if settings.get("ai.providers").is_some() {
        return mochi_core::ai::providers::selected(settings);
    }
    let base_url = settings
        .get(KEY_BASE_URL)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())?;
    let model = settings
        .get(KEY_MODEL)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())?;
    Some(AiProvider {
        id: "default".into(),
        name: settings.get(KEY_NAME).unwrap_or_else(|| "默认".into()),
        base_url,
        api_key: settings.get_secret(KEY_API_KEY).unwrap_or_default(),
        model,
        stream: true,
        protocol: "openai-completions".into(),
    })
}

// ---------- 宿主 ----------

/// UI 状态的快照。Agent 在后台线程跑，不能直接读 `App`，
/// 所以每次开跑前把它需要知道的几样抄一份进来。
#[derive(Debug, Clone, Default)]
pub struct HostSnapshot {
    pub edit_apply_mode: String,
    pub typography: crate::ui::editor_preferences::Preferences,
    pub active_text: Option<String>,
    pub selected_knowledge_base: Option<PathBuf>,
    pub active_document: Option<ActiveDocument>,
    /// 活动页签之外、处于打开状态的文档文本缓冲。多个页签都有未保存
    /// 修改时 UI 会填充它；块工具优先用这里的内容，再回落到磁盘，
    /// 提案就不会基于过期字节。若两边恰好同路径，仍以活动文档为准。
    pub buffered_documents: Vec<BufferedDocument>,
    pub session_id: Option<String>,
    pub selected_agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BufferedDocument {
    pub document: ActiveDocument,
    pub text: String,
}

pub struct DesktopRequest {
    pub name: String,
    pub args: serde_json::Value,
    pub deadline: std::time::Instant,
    pub reply: Sender<Result<serde_json::Value, String>>,
}
pub struct AppHost {
    console_request: Arc<Mutex<Option<DesktopRequest>>>,
    canvas_request: Arc<Mutex<Option<DesktopRequest>>>,
    desktop_request: Arc<Mutex<Option<DesktopRequest>>>,
    file_changes: Arc<Mutex<Vec<FileChange>>>,
    settings: Option<Arc<SettingsService>>,
    knowledge_request: Arc<Mutex<Option<PathBuf>>>,
    settings_request: Arc<Mutex<Option<String>>>,
    exports: Option<Arc<crate::export_requests::Queue>>,
    root: PathBuf,
    snapshot: Arc<Mutex<HostSnapshot>>,
    permissions: Arc<AiPermissionService>,
    inbox: AgentInboxService,
    /// 文件变了 → 让 UI 刷新。跨线程只能发消息。
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl AppHost {
    pub fn new(
        root: &Path,
        snapshot: Arc<Mutex<HostSnapshot>>,
        permissions: Arc<AiPermissionService>,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            console_request: Arc::new(Mutex::new(None)),
            canvas_request: Arc::new(Mutex::new(None)),
            file_changes: Arc::new(Mutex::new(Vec::new())),
            desktop_request: Arc::new(Mutex::new(None)),
            settings: None,
            knowledge_request: Arc::new(Mutex::new(None)),
            settings_request: Arc::new(Mutex::new(None)),
            exports: None,
            root: root.to_path_buf(),
            snapshot,
            permissions,
            inbox: AgentInboxService::new(root),
            notify: Arc::new(notify),
        }
    }
    pub fn with_export_queue(
        mut self,
        exports: Option<Arc<crate::export_requests::Queue>>,
    ) -> Self {
        self.exports = exports;
        self
    }
    pub fn with_runtime_settings(mut self, settings: Arc<SettingsService>) -> Self {
        self.settings = Some(settings);
        self
    }
    pub fn take_knowledge_request(&self) -> Option<PathBuf> {
        self.knowledge_request.lock().ok()?.take()
    }
    pub fn take_file_changes(&self) -> Vec<FileChange> {
        self.file_changes
            .lock()
            .map(|mut changes| std::mem::take(&mut *changes))
            .unwrap_or_default()
    }
    pub fn take_desktop_request(&self) -> Option<DesktopRequest> {
        self.desktop_request.lock().ok()?.take()
    }
    pub fn take_console_request(&self) -> Option<DesktopRequest> {
        self.console_request.lock().ok()?.take()
    }
    pub fn take_canvas_request(&self) -> Option<DesktopRequest> {
        self.canvas_request.lock().ok()?.take()
    }
    pub fn take_settings_request(&self) -> Option<String> {
        self.settings_request.lock().ok()?.take()
    }
    /// 当前编辑器/磁盘源码里是否带有任何正式的块标记。部分转换过的
    /// 文档同样受保护：整文件覆盖会把已转换块的 ID 抹掉。
    /// 探测是只读的；转换仍由用户在 UI 里显式决定。
    fn has_persisted_blocks(&self, path: &str) -> bool {
        let source_path = Path::new(path);
        let extension = source_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !extension.eq_ignore_ascii_case("md") && !extension.eq_ignore_ascii_case("mc") {
            return false;
        }
        // 转换备份是「这条路径走过块转换」的持久证据。就算当前源码
        // 后来被剥掉了标记注释，整文件写入的闸门也要保留。
        if mochi_core::document_blocks::conversion_backup_path(source_path).exists() {
            return true;
        }
        let Ok(source) = <Self as ToolHost>::read_document_text(self, path) else {
            // 真正的新文件没有可覆盖的块身份。
            // 已存在但读不出来的文档依旧受保护。
            return !matches!(source_path.try_exists(), Ok(false));
        };
        // 报错说明扫描器发现了畸形/未闭合的标记语法。视为受保护内容：
        // 放行覆盖会让损坏的块文档受损更重，还绕过了审批。
        let scanned_marker = match mochi_blocks::model::scan_markers(&source) {
            Ok(markers) => !markers.is_empty(),
            Err(_) => true,
        };
        match mochi_core::document_blocks::document_from_source(source_path, &source) {
            Ok(document) => {
                scanned_marker
                    || document
                        .blocks()
                        .iter()
                        .any(|block| block.marker_span.is_some())
            }
            // 一次合法的标记扫描就足以保护畸形源码；
            // 调用方之后可以在编辑器里显式修复/转换。
            Err(_) => scanned_marker,
        }
    }
    fn frozen(&self) -> Self {
        Self {
            console_request: self.console_request.clone(),
            canvas_request: self.canvas_request.clone(),
            root: self.root.clone(),
            desktop_request: self.desktop_request.clone(),
            snapshot: Arc::new(Mutex::new(
                self.snapshot.lock().map(|s| s.clone()).unwrap_or_default(),
            )),
            permissions: self.permissions.clone(),
            inbox: AgentInboxService::new(&self.root),
            exports: self.exports.clone(),
            settings: self.settings.clone(),
            file_changes: self.file_changes.clone(),
            knowledge_request: self.knowledge_request.clone(),
            settings_request: self.settings_request.clone(),
            notify: self.notify.clone(),
        }
    }
}

impl ToolHost for AppHost {
    fn canvas_requires_proposal(&self, path: &str) -> bool {
        self.permissions.path_requires_proposal(path) && self.should_propose(path)
    }

    fn canvas_action(
        &self,
        path: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let (tx, rx) = channel();
        let mut args = args.clone();
        args["path"] = serde_json::json!(path);
        {
            let mut pending = self.canvas_request.lock().map_err(|_| "画布请求锁不可用")?;
            if pending.is_some() {
                return Err("画布正在处理另一项 AI 操作，请稍后重试".into());
            }
            *pending = Some(DesktopRequest {
                name: name.into(),
                args,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(25),
                reply: tx,
            });
        }
        (self.notify)();
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| "画布请求超时，请先 canvas_get 确认结果再重试".to_string())?
    }
    fn shell_whitelist(&self) -> Vec<String> {
        self.settings
            .as_ref()
            .and_then(|settings| settings.get("mochi-ai"))
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|store| store.get("state")?.get("shellWhitelist").cloned())
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }
    fn read_document_text(&self, path: &str) -> Result<String, String> {
        let snapshot = self.snapshot.lock().map_err(|_| "上下文不可用")?;
        if snapshot.active_document.as_ref().is_some_and(|d| {
            d.path
                .replace('\\', "/")
                .eq_ignore_ascii_case(&path.replace('\\', "/"))
        }) {
            if let Some(content) = &snapshot.active_text {
                return Ok(content.clone());
            }
        }
        if let Some(buffer) = snapshot.buffered_documents.iter().find(|buffer| {
            buffer
                .document
                .path
                .replace('\\', "/")
                .eq_ignore_ascii_case(&path.replace('\\', "/"))
        }) {
            return Ok(buffer.text.clone());
        }
        drop(snapshot);
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
    fn list_document_blocks(&self, path: &str) -> Result<serde_json::Value, String> {
        let source = self.read_document_text(path)?;
        let document = mochi_core::document_blocks::document_from_source(Path::new(path), &source)
            .map_err(|e| format!("解析文档块失败: {e}"))?;
        let persisted = document.has_persisted_ids();
        let blocks = if persisted {
            mochi_core::document_blocks::list_blocks(&document).into_iter().map(|block|serde_json::json!({"blockId":block.id,"kind":block.kind,"text":block.content,"content":block.content,"originalHash":block.hash,"sourceSpan":block.source_span,"persisted":true})).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(
            serde_json::json!({"path":path,"documentId":document.document_id,"sourceHash":document.content_hash(),"hasPersistedIds":persisted,"needsConversion":!persisted,"blockCount":document.blocks().len(),"blocks":blocks}),
        )
    }
    fn read_document_block(&self, path: &str, block_id: &str) -> Result<serde_json::Value, String> {
        let source = self.read_document_text(path)?;
        let document = mochi_core::document_blocks::document_from_source(Path::new(path), &source)
            .map_err(|e| format!("解析文档块失败: {e}"))?;
        if !document.has_persisted_ids() {
            return Ok(
                serde_json::json!({"path":path,"blockId":block_id,"hasPersistedIds":false,"needsConversion":true,"message":"该文档尚未持久化块 ID，请先在界面中转换文档"}),
            );
        }
        let block = mochi_core::document_blocks::get_block(&document, block_id)
            .map_err(|e| format!("读取块失败: {e}"))?;
        Ok(
            serde_json::json!({"path":path,"documentId":document.document_id,"blockId":block.id,"kind":block.kind,"text":block.content,"content":block.content,"originalHash":block.hash,"sourceSpan":block.source_span,"persisted":true}),
        )
    }
    fn propose_document_edit(
        &self,
        edit: mochi_core::document_range::PendingDocumentEdit,
    ) -> Result<serde_json::Value, String> {
        if self.has_persisted_blocks(&edit.path) {
            return Err("该文档包含遗留 mochi:block 标识；请先清理标识后再创建范围修改提案".into());
        }
        let source = self.read_document_text(&edit.path)?;
        let original =
            mochi_core::document_range::line_range_text(&source, edit.start_line, edit.end_line);
        if mochi_core::document_range::hash_text(&original) != edit.original_hash {
            return Err("局部修改的原文已变化，请重新生成提案".into());
        }
        let content = mochi_core::document_range::replace_line_range_preserving_endings(
            &source,
            edit.start_line,
            edit.end_line,
            &edit.new_text,
        );
        let mut result = self.submit_proposal(PendingFileOperation {
            kind: "overwrite".into(),
            path: edit.path.clone(),
            new_path: None,
            title: edit.title.clone().unwrap_or_else(|| "局部修改".into()),
            summary: edit.summary.clone(),
            content: Some(content),
            previous_content: Some(source),
            entries: Vec::new(),
            reason: "document-range".into(),
        })?;
        // Electron 会把 pendingEdit 作为会话消息的一部分返回。原生版此前只返回
        // “已进入收件箱”，导致工具回执被隐藏后，用户在对话里看不到可操作的修改卡。
        // 保留 inboxId/operationId 让 UI 能回到同一条已持久化提案，不另造写文件路径。
        let mut pending = serde_json::to_value(&edit).map_err(|e| e.to_string())?;
        if let Some(object) = pending.as_object_mut() {
            if let Some(value) = result.get("inboxId").cloned() {
                object.insert("inboxId".into(), value);
            }
            if let Some(value) = result.get("operationId").cloned() {
                object.insert("operationId".into(), value);
            }
        }
        result["pendingEdit"] = pending;
        Ok(result)
    }
    fn propose_block_edit(&self, edit: BlockEditProposal) -> Result<serde_json::Value, String> {
        // 提案时重新读取。执行器已经做过预检，但这里再查一次，
        // 才能关掉「读取块」到「入队」之间编辑器并发修改的窗口。
        let source = self.read_document_text(&edit.path)?;
        let document =
            mochi_core::document_blocks::document_from_source(Path::new(&edit.path), &source)
                .map_err(|e| format!("解析文档块失败: {e}"))?;
        if !document.has_persisted_ids() {
            return Err("该文档尚未持久化块 ID，请先在界面中转换文档".into());
        }
        let block = mochi_core::document_blocks::get_block(&document, &edit.block_id)
            .map_err(|e| format!("读取块失败: {e}"))?;
        if block.content != edit.old_text || block.hash != edit.original_hash {
            return Err("块原文或哈希已变化，请重新生成提案".into());
        }
        mochi_core::document_blocks::replace_block(
            &document,
            &edit.block_id,
            &edit.original_hash,
            &edit.new_text,
        )
        .map_err(|e| format!("块修改无效: {e}"))?;
        let block_edit = serde_json::to_value(&edit).map_err(|e| e.to_string())?;
        let operation_id = format!(
            "block-op-{}-{}",
            mochi_core::jstime::now_millis(),
            mochi_core::paths::random_base36(12)
        );
        let mut operation = serde_json::Map::new();
        operation.insert("id".into(), operation_id.clone().into());
        operation.insert("kind".into(), "block-edit".into());
        operation.insert("path".into(), edit.path.clone().into());
        operation.insert(
            "title".into(),
            format!(
                "块级修改 · {}",
                Path::new(&edit.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
            .into(),
        );
        operation.insert("summary".into(), edit.summary.clone().into());
        operation.insert("reason".into(), "document-block".into());
        operation.insert("status".into(), "pending".into());
        operation.insert("blockEdit".into(), block_edit.clone());
        // 在操作层重复一份乐观并发字段，供旧版 UI 读取；
        // blockEdit 才是规范的结构化载荷。
        operation.insert("blockId".into(), edit.block_id.clone().into());
        operation.insert("originalHash".into(), edit.original_hash.clone().into());
        operation.insert("oldText".into(), edit.old_text.clone().into());
        operation.insert("newText".into(), edit.new_text.clone().into());
        let entry = self
            .inbox
            .add(
                serde_json::Value::Object(operation.clone()),
                None,
                Some("AI 助手"),
            )
            .map_err(|e| e.to_string())?;
        (self.notify)();
        let pending_file_operation = serde_json::json!({"id":operation_id,"kind":"block-edit","path":edit.path,"blockEdit":block_edit,"blockId":edit.block_id,"originalHash":edit.original_hash,"oldText":edit.old_text,"newText":edit.new_text,"summary":edit.summary,"status":"pending","inboxId":entry.id()});
        Ok(
            serde_json::json!({"status":"pending","operationId":operation_id,"inboxId":entry.id(),"pendingBlockEdit":block_edit,"pendingFileOperation":pending_file_operation,"message":"块修改已进入待批准操作，请逐块确认"}),
        )
    }
    fn knowledge_bases(&self) -> Vec<mochi_core::ai::tools::host::KnowledgeBase> {
        let path = self.root.join(".mochi/libraries.json");
        let libraries = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<mochi_core::domain::Library>>(&s).ok())
            .unwrap_or_default();
        libraries
            .into_iter()
            .filter_map(|library| {
                let path = PathBuf::from(&library.path);
                let meta = std::fs::metadata(&path).ok()?;
                Some(mochi_core::ai::tools::host::KnowledgeBase {
                    name: library.name,
                    path: path.to_string_lossy().replace('\\', "/"),
                    kind: "directory".into(),
                    size: meta.len(),
                    mtime: Some(library.updated_at),
                    child_count: std::fs::read_dir(path)
                        .map(|r| r.flatten().count())
                        .unwrap_or(0),
                })
            })
            .collect()
    }
    fn select_knowledge_base(&self, path: &str) -> Result<(), String> {
        let path = path.replace('\\', "/");
        let target = self
            .knowledge_bases()
            .into_iter()
            .find(|k| k.path.eq_ignore_ascii_case(&path))
            .ok_or("知识库不存在")?;
        let target = PathBuf::from(target.path);
        self.snapshot
            .lock()
            .map_err(|_| "上下文锁不可用")?
            .selected_knowledge_base = Some(target.clone());
        *self
            .knowledge_request
            .lock()
            .map_err(|_| "界面请求锁不可用")? = Some(target);
        (self.notify)();
        Ok(())
    }
    fn propose_shell_command(
        &self,
        mut proposal: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let pending = proposal.get("pendingShellCommand").ok_or("命令提案缺失")?;
        let command = pending["command"].as_str().ok_or("命令缺失")?;
        if command.len() > 65536 {
            return Err("命令过长，请先保存为脚本后再请求执行".into());
        }
        let cwd = pending["cwd"].as_str().ok_or("工作目录缺失")?;
        if pending.get("script").is_some() || pending["kind"] == "script" {
            let spec = mochi_core::script::approved_spec(pending, command, cwd)?;
            spec.working_directory(self.workspace_root())?;
        }
        let queue = self.exports.as_ref().ok_or("原生审批队列不可用")?;
        let request = queue
            .add_for_session(
                command,
                "shell",
                cwd,
                Some(pending.to_string()),
                self.active_session_id(),
            )
            .map_err(|e| e.to_string())?;
        proposal["pendingShellCommand"] = request.shell_data();
        proposal["message"] = "请在原生版右侧栏“待批准操作”中查看完整命令并批准".into();
        (self.notify)();
        Ok(proposal)
    }
    fn fixed_setting(&self, _key: &str) -> Option<mochi_core::app_settings::SettingValue> {
        None
    }
    fn supports_desktop_cards(&self) -> bool {
        true
    }
    fn supports_console(&self) -> bool {
        true
    }
    fn console(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let (tx, rx) = channel();
        {
            let mut pending = self
                .console_request
                .lock()
                .map_err(|_| "控制台请求锁不可用")?;
            if pending.is_some() {
                return Err("控制台正在处理上一个请求".into());
            }
            *pending = Some(DesktopRequest {
                name: name.into(),
                args: args.clone(),
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(25),
                reply: tx,
            });
        }
        (self.notify)();
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| "控制台请求超时；请先回读状态，不要盲目重试写操作".to_string())?
    }
    fn desktop_cards(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let (tx, rx) = channel();
        {
            let mut pending = self
                .desktop_request
                .lock()
                .map_err(|_| "桌面请求锁不可用")?;
            if pending.is_some() {
                return Err("桌面卡片正忙，请稍后重试".into());
            }
            *pending = Some(DesktopRequest {
                name: name.into(),
                args: args.clone(),
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(25),
                reply: tx,
            });
        }
        (self.notify)();
        rx.recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| "桌面卡片请求未及时完成，请先读取状态确认结果".to_string())?
    }
    fn open_settings_ui(&self, tab: &str) -> Result<(), String> {
        if !crate::ui::settings::TABS
            .iter()
            .any(|(key, _, _)| *key == tab)
        {
            return Err("未知设置分页".into());
        }
        *self
            .settings_request
            .lock()
            .map_err(|_| "设置请求锁不可用")? = Some(tab.into());
        (self.notify)();
        Ok(())
    }
    fn supports_document_export(&self) -> bool {
        true
    }
    fn propose_document_export(
        &self,
        source: &str,
        format: &str,
        output: &str,
    ) -> Result<serde_json::Value, String> {
        let queue = self
            .exports
            .as_ref()
            .ok_or("原生导出审批队列不可用，请检查设置目录")?;
        let content = self.snapshot.lock().ok().and_then(|s| {
            s.active_document
                .as_ref()
                .filter(|d| {
                    d.path
                        .replace('\\', "/")
                        .eq_ignore_ascii_case(&source.replace('\\', "/"))
                })
                .and(s.active_text.clone())
        });
        let request = queue
            .add(source, format, output, content)
            .map_err(|e| e.to_string())?;
        (self.notify)();
        Ok(
            serde_json::json!({"status":"pending","id":request.id,"sourcePath":source,"outputPath":output,"format":format,"message":"请在原生版右侧栏“待批准操作”中批准导出"}),
        )
    }
    fn export_document(&self, source: &str, format: &str, output: &str) -> Result<(), String> {
        if let Ok(snapshot) = self.snapshot.lock() {
            crate::ui::editor_preferences::set(snapshot.typography.clone());
        }
        let active = self.snapshot.lock().ok().and_then(|s| {
            s.active_document
                .as_ref()
                .filter(|d| {
                    d.path
                        .replace('\\', "/")
                        .eq_ignore_ascii_case(&source.replace('\\', "/"))
                })
                .and(s.active_text.clone())
        });
        let content = match active {
            Some(text) => text,
            None => std::fs::read_to_string(source).map_err(|e| e.to_string())?,
        };
        let content = mochi_core::exports::enrichment::prepare(
            &content,
            &mochi_core::exports::enrichment::Options {
                format: format.into(),
                ..Default::default()
            },
            &[],
            |_| None,
        )
        .map_err(|e| e.to_string())?;
        match format {
            "pdf" => {
                crate::gfx::export_pdf(&content, Path::new(output), Path::new(source).parent())
            }
            "html" => mochi_core::files::FileService::new().write_file_safe(
                output,
                &mochi_core::exports::html(
                    &content,
                    &Path::new(source)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy(),
                ),
            ),
            "markdown" => mochi_core::files::FileService::new().write_file_safe(output, &content),
            _ => return Err("不支持的导出格式".into()),
        }
        .map_err(|e| e.to_string())
    }
    fn workspace_root(&self) -> &Path {
        &self.root
    }

    fn selected_knowledge_base(&self) -> Option<PathBuf> {
        self.snapshot.lock().ok()?.selected_knowledge_base.clone()
    }

    fn active_document(&self) -> Option<ActiveDocument> {
        self.snapshot.lock().ok()?.active_document.clone()
    }

    fn is_path_visible(&self, absolute_path: &str) -> bool {
        self.permissions.is_path_visible(absolute_path)
    }

    fn should_propose(&self, absolute_path: &str) -> bool {
        let dirty = self
            .snapshot
            .lock()
            .map(|s| {
                s.active_document.as_ref().is_some_and(|d| {
                    d.is_dirty
                        && d.path
                            .replace('\\', "/")
                            .eq_ignore_ascii_case(&absolute_path.replace('\\', "/"))
                })
            })
            .unwrap_or(true);
        let automatic = self
            .settings
            .as_ref()
            .and_then(|settings| settings.get("app.ai.editApplyMode"))
            .map(|mode| mode == "auto")
            .unwrap_or_else(|| {
                self.snapshot
                    .lock()
                    .map(|s| s.edit_apply_mode == "auto")
                    .unwrap_or(false)
            });

        // 用户可见的开关才是「走审批还是直接应用」的最终裁决。
        // 文件夹权限仍会由 `assert_tool_action_allowed` 强制执行：不可见路径禁止操作，
        // 只读路径禁止写入；`suggest` 规则绝不能悄悄压过
        // 用户明确确认过的自动模式。
        self.has_persisted_blocks(absolute_path) || dirty || !automatic
    }

    /// 需要批准的写操作进「待批准操作」收件箱，与后台任务走同一条路。
    /// 文档范围编辑会同时返回会话里的 `pendingEdit` 卡片，并把同一操作
    /// 链接到收件箱；文件仍只能通过收件箱的批准流程写入。
    fn submit_proposal(&self, op: PendingFileOperation) -> Result<serde_json::Value, String> {
        if matches!(op.kind.as_str(), "write" | "overwrite") && self.has_persisted_blocks(&op.path)
        {
            return Err("该文档包含遗留 mochi:block 标识，文件覆盖提案被拒绝；请先清理标识后再创建范围修改提案".into());
        }
        let id = format!(
            "op-{}-{}",
            mochi_core::jstime::now_millis(),
            mochi_core::paths::random_base36(12)
        );
        let mut operation = serde_json::Map::new();
        operation.insert("id".into(), id.clone().into());
        operation.insert("kind".into(), op.kind.clone().into());
        operation.insert("path".into(), op.path.clone().into());
        operation.insert("title".into(), op.title.clone().into());
        operation.insert("summary".into(), op.summary.clone().into());
        if let Some(p) = &op.new_path {
            operation.insert("newPath".into(), p.clone().into());
        }
        if let Some(c) = &op.content {
            operation.insert("content".into(), c.clone().into());
        }
        if let Some(c) = &op.previous_content {
            operation.insert("previousContent".into(), c.clone().into());
        }
        if !op.entries.is_empty() {
            operation.insert(
                "entries".into(),
                serde_json::Value::Array(op.entries.iter().map(|e| e.clone().into()).collect()),
            );
        }
        operation.insert("reason".into(), op.reason.clone().into());
        operation.insert("status".into(), "pending".into());
        let entry = self
            .inbox
            .add(serde_json::Value::Object(operation), None, Some("AI 助手"))
            .map_err(|e| e.to_string())?;
        (self.notify)();
        Ok(serde_json::json!({
            "status": "pending",
            "operationId": id,
            "inboxId": entry.id(),
            "message": format!("{} 已转为待批准提案，请在右侧栏「待批准操作」中确认", op.summary),
        }))
    }

    fn active_session_id(&self) -> Option<String> {
        self.snapshot.lock().ok()?.session_id.clone()
    }

    fn notify_change(&self, change: FileChange) {
        let same = |a: &str, b: &str| {
            a.replace('\\', "/")
                .eq_ignore_ascii_case(&b.replace('\\', "/"))
        };
        if let Ok(mut snapshot) = self.snapshot.lock() {
            match &change {
                FileChange::Write { path } => {
                    if snapshot
                        .active_document
                        .as_ref()
                        .is_some_and(|d| same(&d.path, path))
                    {
                        snapshot.active_text = std::fs::read_to_string(path).ok();
                    }
                }
                FileChange::Rename { path, new_path } => {
                    if let Some(d) = snapshot
                        .active_document
                        .as_mut()
                        .filter(|d| same(&d.path, path))
                    {
                        d.path = new_path.clone();
                        d.title = Path::new(new_path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                    }
                }
                FileChange::Delete { path } => {
                    if snapshot
                        .active_document
                        .as_ref()
                        .is_some_and(|d| same(&d.path, path))
                    {
                        snapshot.active_document = None;
                        snapshot.active_text = None;
                    }
                }
                _ => {}
            }
        }
        if let Ok(mut changes) = self.file_changes.lock() {
            changes.push(change);
        }
        (self.notify)();
    }
}

// ---------- 后台运行 ----------

/// 投回 UI 线程的事件。`AgentEvent` 借用运行时的缓冲区，跨线程要拥有数据，
/// 所以这里是它的 owned 版本。
#[derive(Debug, Clone)]
pub enum RunEvent {
    Skills(Vec<String>),
    Runtime {
        agent_id: Option<String>,
        agent_name: String,
        skills: Vec<String>,
        model: String,
    },
    Status(String),
    ModelStart,
    Reasoning(String),
    Content(String),
    Tool {
        name: String,
        ok: Option<bool>,
        summary: Option<String>,
    },
    /// 有序轨迹同时用于实时展示和会话回看。
    Trace(AgentTraceStep),
    Done {
        content: String,
        transcript: Vec<AiMessage>,
        trace: Vec<AgentTraceStep>,
        finish_reason: String,
        usage: serde_json::Value,
        usage_source: String,
    },
    Failed(String),
}

/// 一次运行的句柄。UI 线程持有 `rx` 取事件；`cancel` 让循环在下一个事件边界停下。
pub struct RunHandle {
    pub rx: Receiver<RunEvent>,
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

/// 工具往返保留协议角色；轨迹说明与旧格式的孤立工具回执不发给模型。
pub fn conversation_messages(
    conversation: &mochi_core::ai::session::AiConversation,
) -> Vec<AiMessage> {
    convert_messages(&conversation.messages)
}
fn convert_messages(records: &[mochi_core::ai::session::AiStoredMessage]) -> Vec<AiMessage> {
    let mut messages = Vec::new();
    let mut pending = std::collections::HashSet::new();
    for stored in records {
        let calls = stored.tool_calls();
        if stored.role() == "tool" {
            let Some(id) = stored.tool_call_id().filter(|id| pending.remove(*id)) else {
                continue;
            };
            messages.push(AiMessage::tool_result(
                id,
                stored
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool"),
                stored.content(),
            ));
        } else if matches!(stored.role(), "user" | "assistant")
            && (!stored.is_hidden() || !calls.is_empty())
        {
            let image_only = stored.role() == "user"
                && stored
                    .get("images")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| a.iter().any(|v| v.as_str().is_some()));
            if stored.content().is_empty() && calls.is_empty() && !image_only {
                continue;
            }
            pending.extend(calls.iter().map(|c| c.id.clone()));
            let mut message = AiMessage::new(stored.role(), stored.content());
            if !calls.is_empty() {
                message.tool_calls = Some(calls);
            }
            messages.push(message);
        }
    }
    let replied = messages
        .iter()
        .filter(|m| m.role == "tool")
        .filter_map(|m| m.tool_call_id.clone())
        .collect::<std::collections::HashSet<_>>();
    for message in &mut messages {
        if let Some(calls) = message.tool_calls.as_mut() {
            calls.retain(|c| replied.contains(&c.id));
            if calls.is_empty() {
                message.tool_calls = None;
            }
        }
    }
    messages.retain(|m| {
        m.role == "user"
            || m.content.as_ref().is_some_and(|s| !s.is_empty())
            || m.tool_calls.as_ref().is_some_and(|c| !c.is_empty())
    });
    messages
}
#[cfg(test)]
pub fn messages_with_current_images(
    conversation: &mochi_core::ai::session::AiConversation,
    images: Vec<String>,
) -> anyhow::Result<Vec<AiMessage>> {
    messages_with_current_attachments(conversation, images, None)
}
pub fn messages_with_current_attachments(
    conversation: &mochi_core::ai::session::AiConversation,
    images: Vec<String>,
    attachment: Option<mochi_core::ai::attachments::Prepared>,
) -> anyhow::Result<Vec<AiMessage>> {
    if images.is_empty() && attachment.is_none() {
        return Ok(conversation_messages(conversation));
    }
    let current = conversation
        .messages
        .last()
        .filter(|m| m.role() == "user" && !m.is_hidden())
        .ok_or_else(|| anyhow::anyhow!("图片必须属于本轮用户消息"))?;
    let mut messages = convert_messages(&conversation.messages[..conversation.messages.len() - 1]);
    let mut user = AiMessage::new("user", current.content());
    user.images = images;
    if let Some(attachment) = attachment {
        user.content = Some(attachment.content);
        user.files = attachment.files;
    }
    messages.push(user);
    Ok(messages)
}
impl Drop for RunHandle {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// 组装工具集并在后台线程跑一轮 Agent。每个事件到达后调 `wake()`（投 Win32 消息）。
pub fn spawn_run(
    root: &Path,
    provider: AiProvider,
    messages: Vec<AiMessage>,
    host: Arc<AppHost>,
    settings: Arc<SettingsService>,
    index: Arc<MetadataIndexService>,
    git: Arc<GitService>,
    wake: impl Fn() + Send + Sync + 'static,
) -> RunHandle {
    let host = Arc::new(host.frozen());
    let (tx, rx): (Sender<RunEvent>, Receiver<RunEvent>) = channel();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_flag = Arc::clone(&cancel);
    let root = root.to_path_buf();
    let permissions = Arc::clone(&host.permissions);
    std::thread::spawn(move || {
        let send = |ev: RunEvent| {
            if tx.send(ev).is_ok() {
                wake();
            }
        };
        let memory = match MemoryStore::open(&root) {
            Ok(m) => Some(Arc::new(Mutex::new(m))),
            Err(e) => {
                let _ = e;
                send(RunEvent::Status("记忆库暂不可用，继续本轮对话".into()));
                None
            }
        };
        let app_settings = Arc::new(AppSettings::new(Arc::clone(&settings)));
        let services = ToolServices {
            host: host.clone() as Arc<dyn ToolHost>,
            permissions,
            git,
            agenda: Arc::new(AgendaStore::new(&root)),
            settings: app_settings.clone(),
            search: Arc::new(SearchService::new(&root, index)),
            memory,
        };
        let snapshot = host
            .snapshot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let user_request = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .and_then(|m| m.content.as_deref())
            .unwrap_or("");
        let routing_request = if snapshot
            .active_document
            .as_ref()
            .is_some_and(|document| document.path.to_ascii_lowercase().ends_with(".mcb"))
        {
            format!("{user_request}\n当前文档：.mcb 多维表格")
        } else {
            user_request.to_owned()
        };
        let plan = mochi_core::ai::runtime_config::load(
            &root,
            snapshot.selected_agent_id.as_deref(),
            &routing_request,
        );
        let memory_context = if services.memory.is_none() {
            "长期记忆暂不可用；本轮没有加载长期记忆，不要声称已检索记忆。".into()
        } else if services
            .permissions
            .assert_tool_action_allowed(mochi_core::ai::permission::AiToolAction::ReadFile, None)
            .is_ok()
        {
            services
                .memory
                .as_ref()
                .and_then(|memory| memory.lock().ok())
                .and_then(|store| {
                    store
                        .build_context(&mochi_core::memory::MemoryContextOptions {
                            query: Some(user_request.to_owned()),
                            library_path: snapshot
                                .selected_knowledge_base
                                .as_ref()
                                .map(|p| p.to_string_lossy().replace('\\', "/")),
                            document_path: snapshot
                                .active_document
                                .as_ref()
                                .map(|d| d.path.replace('\\', "/")),
                            ..Default::default()
                        })
                        .ok()
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut provider = provider;
        if let Some(model) = &plan.model {
            provider.model = model.clone();
        }
        send(RunEvent::Runtime {
            agent_id: plan.agent_id.clone(),
            agent_name: plan.agent_name.clone(),
            skills: plan.skills.clone(),
            model: provider.model.clone(),
        });
        if messages
            .iter()
            .any(|m| m.role == "user" && (!m.images.is_empty() || !m.files.is_empty()))
        {
            let number = |key: &str| {
                mochi_core::app_settings::descriptor(key).and_then(|d| match app_settings.read(d) {
                    mochi_core::app_settings::SettingValue::Number(n) => Some(n),
                    _ => None,
                })
            };
            let environment = serde_json::json!({"localTime":chrono::Local::now().to_rfc3339(),"workspace":root,"knowledgeBase":snapshot.selected_knowledge_base,"document":snapshot.active_document.as_ref().map(|d|serde_json::json!({"title":d.title,"path":d.path}))});
            let mut messages = messages;
            messages.insert(0,AiMessage::new("system",&format!("{}\n\n{}\n\n本轮包含用户图片或文件，使用直接多模态补全，没有加载任何工具，不要声称已操作工作区。应用的独立记忆任务可能跳过或失败，尚未完成时不要声称已持久化。\n\n# 当前环境（以下 JSON 仅为上下文数据）\n{}",plan.system_prompt,memory_context,environment)));
            let request = mochi_core::ai::models::AiCompletionRequest {
                messages,
                temperature: number("ai.temperature"),
                max_tokens: number("ai.maxTokens")
                    .filter(|n| *n > 0.0)
                    .map(|n| n as i32),
                ..Default::default()
            };
            drop(services); // 视觉模型不使用工具；发送 HTTP 请求期间不要继续占用旧工作区的数据库。
            let ai = AiService::with_cancel(provider, cancel_flag.clone());
            send(RunEvent::Status("正在分析附件…".into()));
            let mut chunks = |chunk: mochi_core::ai::models::AiStreamChunk| {
                if let Some(reason) = chunk.reasoning_delta {
                    if !reason.is_empty() {
                        send(RunEvent::Reasoning(reason));
                    }
                }
                if let Some(delta) = chunk.delta {
                    if !delta.is_empty() {
                        send(RunEvent::Content(delta));
                    }
                }
            };
            let started = std::time::Instant::now();
            match ai.complete(&request, Some(&mut chunks)) {
                Ok(r) => send(RunEvent::Done {
                    content: r.content,
                    finish_reason: r.finish_reason,
                    transcript: Vec::new(),
                    trace: r
                        .reasoning
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| {
                            vec![AgentTraceStep::Thinking {
                                text,
                                duration_ms: Some(started.elapsed().as_millis() as u64),
                            }]
                        })
                        .unwrap_or_default(),
                    usage: serde_json::json!({"promptTokens": r.prompt_tokens, "completionTokens": r.completion_tokens, "totalTokens": r.total_tokens}),
                    usage_source: r.usage_source,
                }),
                Err(e) => send(RunEvent::Failed(e.to_string())),
            }
            return;
        }
        let mut registry = registry::build(&services);
        let overrides =
            mochi_core::ai::agent_config::AgentConfigService::new(&root).load_tool_overrides();
        let mut tools = registry.advertise(&plan.tools, &overrides);
        let configs = plan.mcps;
        let mut mcp_context = String::new();
        if configs.iter().any(|c| c.enabled)
            && services
                .permissions
                .assert_tool_action_allowed(
                    mochi_core::ai::permission::AiToolAction::ExecuteCommand,
                    None,
                )
                .is_ok()
            && services
                .permissions
                .assert_tool_action_allowed(
                    mochi_core::ai::permission::AiToolAction::NetworkAccess,
                    None,
                )
                .is_ok()
        {
            send(RunEvent::Status("正在连接 MCP 工具…".into()));
            let (executor, errors) =
                mochi_core::ai::mcp::Executor::connect_with_cancel(configs, cancel_flag.clone());
            let mcp_error_count = errors.len();
            let definitions = executor.definitions();
            mcp_context=format!("\n\n# MCP 连接状态\n已连接工具：{}\n连接失败：{} 个服务器（详细错误仅在客户端显示）。\n未列出的工具当前不可调用，不要声称已执行。",definitions.iter().map(|d|d.function.name.as_str()).collect::<Vec<_>>().join(", "),errors.len());
            if plan.progressive {
                mcp_context = format!("\n\n外部服务状态：已连接 {} 个工具，{} 个服务器连接失败。需要调用外部服务时，先 skill_load 加载“外部服务”。", definitions.len(), errors.len());
            }
            tools.extend(definitions);
            registry.register(Arc::new(executor));
            for error in errors {
                send(RunEvent::Tool {
                    name: "MCP 连接".into(),
                    ok: Some(false),
                    summary: Some(error),
                });
            }
            if mcp_error_count > 0 {
                send(RunEvent::Status(
                    "MCP 部分连接失败，已跳过失败服务器，继续请求 AI…".into(),
                ));
            }
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                send(RunEvent::Failed("已取消 MCP 连接".into()));
                return;
            }
        } else if !configs.is_empty() {
            mcp_context = "\n\n# MCP 连接状态\n当前权限不允许连接 MCP；没有挂载 MCP 工具。".into();
        }
        registry.restrict_to(tools.iter().map(|t| t.function.name.clone()));
        let can_write_memory = tools.iter().any(|t| t.function.name == "memory_write");
        if plan.progressive {
            registry.enable_progressive(tools);
            tools = registry.disclosed_tools().unwrap_or_default();
        }
        let ai = AiService::with_cancel(provider, cancel_flag.clone());
        let number = |key: &str| {
            mochi_core::app_settings::descriptor(key).and_then(|d| match app_settings.read(d) {
                mochi_core::app_settings::SettingValue::Number(n) => Some(n),
                _ => None,
            })
        };
        let memory_enabled = mochi_core::app_settings::descriptor("ai.memoryAutoExtract")
            .is_some_and(|d| {
                matches!(
                    app_settings.read(d),
                    mochi_core::app_settings::SettingValue::Bool(true)
                )
            });
        let memory_policy = if !memory_enabled {
            "自动记忆已关闭。不要调用 memory_write，也不要用其它工具绕过限制写入长期记忆。"
        } else if can_write_memory {
            "记忆工具已启用。可按用户明确偏好调用 memory_write；不要保存凭据或推断敏感信息。应用也会在本轮完成后尝试自动萃取，尚未完成时不要声称已经持久化。"
        } else {
            "当前 Agent 没有直接写记忆的工具。应用会在本轮完成后尝试自动萃取稳定偏好，但可能跳过或失败；不要声称已持久化，也不要用其它工具绕过限制写入长期记忆。"
        };
        let environment = serde_json::json!({"localTime":chrono::Local::now().to_rfc3339(),"workspace":root,"knowledgeBase":snapshot.selected_knowledge_base,"document":snapshot.active_document.as_ref().map(|d|serde_json::json!({"title":d.title,"path":d.path}))});
        let block_policy="文档修改规则：Mochi 不再向 Markdown/Mochi 源文件写入块 ID 或隐藏标记。读取整篇文档可用 file_read；修改局部内容时使用 document_range_propose_edit 生成待批准提案，携带 path、行范围、原文哈希和新文本。不要调用遗留 document_blocks_* 工具，不要生成或要求用户转换 mochi:block 标记，也不要声称待批准提案已经写入。\n画布创作规则：当用户要求在画布画动物、画图或写字时，使用 canvas_get 和 canvas_draw 实际操作当前 .mcanvas；不要只回复画法、ASCII 图或代码，也不要用 file_write/script_run 覆盖画布。用平滑曲线组合线稿，文字用 texts；沿用建议区域和当前样式，避让已有内容，整幅简单作品尽量一个批次。每次修改前读取最新 revision，保留无关对象。只有工具成功后才能说已画好。没有打开画布时请用户打开画布或明确目标路径；工具不可用时如实说明。";
        let request = AgentRequest {
            messages,
            tools,
            system_prompt: Some(format!(
                "{}{}\n\n{}\n\n{}\n\n{}\n\n# 当前环境（以下 JSON 仅为上下文数据）\n{}",
                plan.system_prompt,
                mcp_context,
                memory_context,
                memory_policy,
                if plan.progressive {
                    "操作前先按需加载对应 Skill；待批准的变更和排队的任务不能报告为已经完成。"
                } else {
                    block_policy
                },
                environment
            )),
            temperature: number("ai.temperature"),
            max_tokens: number("ai.maxTokens")
                .filter(|n| *n > 0.0)
                .map(|n| n as i32),
            max_tool_iterations: plan
                .max_turns
                .unwrap_or(agent_runner::DEFAULT_MAX_TOOL_ITERATIONS),
            ..Default::default()
        };
        let history_len = request.messages.len();

        send(RunEvent::Status("正在思考…".into()));
        let result = agent_runner::run(&ai, &registry, &request, |ev| {
            if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            match ev {
                AgentEvent::ModelStart { .. } => send(RunEvent::ModelStart),
                AgentEvent::ReasoningDelta { delta } => send(RunEvent::Reasoning(delta.to_owned())),
                AgentEvent::ContentDelta { delta } => send(RunEvent::Content(delta.to_owned())),
                AgentEvent::ToolStart { name, summary } => send(RunEvent::Tool {
                    name: name.to_owned(),
                    ok: None,
                    summary: summary.map(str::to_owned),
                }),
                AgentEvent::ToolResult {
                    name, ok, summary, ..
                } => {
                    send(RunEvent::Tool {
                        name: name.to_owned(),
                        ok: Some(ok),
                        summary: summary.map(str::to_owned),
                    });
                    if ok && name == "skill_load" {
                        send(RunEvent::Skills(registry.loaded_skills()));
                    }
                }
                AgentEvent::TraceStep { step } => send(RunEvent::Trace(step.clone())),
                AgentEvent::Final { .. } | AgentEvent::Error { .. } => {}
            }
        });
        match result {
            Ok(r) => send(RunEvent::Done {
                content: r.response.content.clone(),
                finish_reason: r.response.finish_reason.clone(),
                usage: serde_json::json!({"promptTokens": r.response.prompt_tokens, "completionTokens": r.response.completion_tokens, "totalTokens": r.response.total_tokens}),
                usage_source: r.response.usage_source.clone(),
                transcript: r.transcript.into_iter().skip(history_len + 1).collect(),
                trace: r.trace,
            }),
            Err(e) => send(RunEvent::Failed(e.to_string())),
        }
    });
    RunHandle { rx, cancel }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_current_explicit_images_enter_request() {
        use mochi_core::ai::session::{AiConversation, AiStoredMessage};
        let mut old = AiStoredMessage::new("user", "");
        old.set("images", serde_json::json!(["assets/private.png"]));
        let mut current = AiStoredMessage::new("user", "");
        current.set("images", serde_json::json!(["assets/current.png"]));
        let mut c = AiConversation {
            messages: vec![old, AiStoredMessage::new("assistant", "reply"), current],
            ..Default::default()
        };
        let m =
            messages_with_current_images(&c, vec!["data:image/png;base64,YQ==".into()]).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].role, "user");
        assert!(m[0].images.is_empty());
        assert_eq!(m[2].images.len(), 1);
        c.messages
            .push(AiStoredMessage::new("assistant", "changed"));
        assert!(messages_with_current_images(&c, vec!["image".into()]).is_err());
    }

    #[test]
    fn selected_agent_is_sent_to_model_and_unadvertised_write_is_rejected() {
        verify_scoped_model(true);
    }
    #[test]
    fn broken_memory_database_does_not_interrupt_the_main_chat() {
        verify_scoped_model(false);
    }
    fn verify_scoped_model(healthy_memory: bool) {
        use std::io::{Read, Write};
        let root = std::env::temp_dir().join(format!(
            "mochi-agent-http-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(root.join("Agent配置/Agents")).unwrap();
        std::fs::write(root.join("Agent配置/Agents/reader.md"),"---\nname: Reader\ntools: [file_read]\nmodel: scoped-model\nmaxTurns: 3\n---\nselected-agent-sentinel").unwrap();
        if healthy_memory {
            MemoryStore::open(&root)
                .unwrap()
                .write(mochi_core::memory::MemoryWriteInput {
                    kind: Some(mochi_core::memory::MemoryKind::Pinned),
                    title: "测试偏好".into(),
                    content: "pinned-memory-sentinel".into(),
                    ..Default::default()
                })
                .unwrap();
        } else {
            std::fs::create_dir_all(root.join(".mochi")).unwrap();
            std::fs::write(MemoryStore::db_path(&root), "broken-memory-sentinel").unwrap();
        }
        let target = root.join("must-not-exist.md");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let destination = target.to_string_lossy().into_owned();
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            for turn in 0..2 {
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "model request timed out"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let body = loop {
                    let mut buf = [0u8; 4096];
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    assert!(bytes.len() < 1024 * 1024);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let size = headers
                            .lines()
                            .find_map(|l| {
                                l.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + size {
                            break serde_json::from_slice::<serde_json::Value>(
                                &bytes[end + 4..end + 4 + size],
                            )
                            .unwrap();
                        }
                    }
                };
                bodies.push(body);
                let message = if turn == 0 {
                    serde_json::json!({"role":"assistant","content":"","tool_calls":[{"id":"forbidden","type":"function","function":{"name":"file_write","arguments":serde_json::json!({"path":destination,"content":"forbidden"}).to_string()}}]})
                } else {
                    serde_json::json!({"role":"assistant","content":"scope verified"})
                };
                let response=serde_json::json!({"choices":[{"index":0,"message":message,"finish_reason":if turn==0{"tool_calls"}else{"stop"}}]}).to_string();
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            }
            bodies
        });
        let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            selected_agent_id: Some("reader".into()),
            edit_apply_mode: "auto".into(),
            ..Default::default()
        }));
        let host = Arc::new(AppHost::new(
            &root,
            snapshot,
            Arc::new(AiPermissionService::new(&root)),
            || {},
        ));
        let handle = spawn_run(
            &root,
            AiProvider {
                base_url: format!("http://{address}/v1"),
                model: "wrong-default".into(),
                stream: false,
                protocol: "openai-completions".into(),
                ..Default::default()
            },
            vec![AiMessage {
                role: "user".into(),
                content: Some("review only".into()),
                ..Default::default()
            }],
            host,
            settings,
            Arc::new(MetadataIndexService::new(&root)),
            Arc::new(GitService::new(&root)),
            || {},
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut done = false;
        while std::time::Instant::now() < deadline {
            match handle
                .rx
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                Ok(RunEvent::Done { content, .. }) => {
                    assert_eq!(content, "scope verified");
                    done = true;
                    break;
                }
                Ok(RunEvent::Failed(error)) => panic!("{error}"),
                _ => {}
            }
        }
        let bodies = server.join().unwrap();
        assert!(done);
        assert_eq!(bodies[0]["model"], "scoped-model");
        assert!(bodies[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("selected-agent-sentinel"));
        assert_eq!(
            bodies[0]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("pinned-memory-sentinel"),
            healthy_memory
        );
        if !healthy_memory {
            assert_eq!(
                std::fs::read_to_string(MemoryStore::db_path(&root)).unwrap(),
                "broken-memory-sentinel"
            );
            assert!(bodies[0]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("长期记忆暂不可用"));
            assert!(!bodies[0].to_string().contains("broken-memory-sentinel"));
        }
        let names = bodies[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, ["file_read"]);
        assert!(bodies[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool"
                && m["content"]
                    .as_str()
                    .is_some_and(|s| s.contains("当前 Agent 未启用工具"))));
        assert!(!target.exists());
        assert!(AgentInboxService::new(&root).list_pending().is_empty());
        drop(handle);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_mcp_discovery_does_not_block_the_model_request() {
        use mochi_core::ai::permission::ActionPermissions;
        use std::io::{Read, Write};

        let root = std::env::temp_dir().join(format!(
            "mochi-mcp-runtime-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(root.join("Agent配置/Agents")).unwrap();
        std::fs::create_dir_all(root.join("Agent配置/MCPs/broken")).unwrap();
        std::fs::write(
            root.join("Agent配置/Agents/reader.md"),
            "---\nname: Reader\ntools: [file_read]\nmcps: [broken]\nmodel: mcp-runtime-test\nmaxTurns: 1\n---\n只读审阅",
        )
        .unwrap();
        std::fs::write(
            root.join("Agent配置/MCPs/broken/MCP.md"),
            "---\nname: broken\ntransport: stdio\ncommand: mochi-command-that-does-not-exist\nenabled: true\n---\n",
        )
        .unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "model request timed out"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            // Windows 上接受的 socket 会继承非阻塞模式。mock 服务器
            // 读的是完整请求，所以用有界的阻塞读。
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut chunk = [0_u8; 4096];
                let size = socket.read(&mut chunk).unwrap();
                assert!(size > 0);
                bytes.extend_from_slice(&chunk[..size]);
                let Some(headers_end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&bytes[..headers_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                if bytes.len() >= headers_end + 4 + content_length {
                    break serde_json::from_slice::<serde_json::Value>(
                        &bytes[headers_end + 4..headers_end + 4 + content_length],
                    )
                    .unwrap();
                }
            };
            assert_eq!(body["model"], "mcp-runtime-test");
            let response = serde_json::json!({
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "模型继续完成"},
                    "finish_reason": "stop"
                }]
            })
            .to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            )
            .unwrap();
            socket.flush().unwrap();
        });

        let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            selected_agent_id: Some("reader".into()),
            edit_apply_mode: "auto".into(),
            ..Default::default()
        }));
        let permissions = Arc::new(AiPermissionService::new(&root));
        let mut actions = ActionPermissions::default();
        actions.set(
            mochi_core::ai::permission::AiToolAction::ExecuteCommand,
            true,
        );
        actions.set(
            mochi_core::ai::permission::AiToolAction::NetworkAccess,
            true,
        );
        permissions.set_action_permissions(actions);
        let host = Arc::new(AppHost::new(&root, snapshot, permissions, || {}));
        let handle = spawn_run(
            &root,
            AiProvider {
                base_url: format!("http://{address}/v1"),
                model: "wrong-default".into(),
                stream: false,
                protocol: "openai-completions".into(),
                ..Default::default()
            },
            vec![AiMessage::new("user", "请继续")],
            host,
            settings,
            Arc::new(MetadataIndexService::new(&root)),
            Arc::new(GitService::new(&root)),
            || {},
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        let mut saw_failure = false;
        let mut saw_continue = false;
        let mut done = false;
        while std::time::Instant::now() < deadline {
            match handle
                .rx
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                Ok(RunEvent::Tool {
                    name,
                    ok: Some(false),
                    summary: Some(summary),
                }) if name == "MCP 连接" => {
                    saw_failure = summary.contains("MCP 进程启动失败");
                }
                Ok(RunEvent::Status(status)) if status.contains("继续请求 AI") => {
                    saw_continue = true;
                }
                Ok(RunEvent::Done { content, .. }) => {
                    assert_eq!(content, "模型继续完成");
                    done = true;
                    break;
                }
                Ok(RunEvent::Failed(error)) => panic!("{error}"),
                _ => {}
            }
        }
        assert!(
            saw_failure,
            "MCP failure should be surfaced as a tool event"
        );
        assert!(
            saw_continue,
            "the base AI request should continue after MCP failure"
        );
        assert!(done, "the model should receive a request after MCP failure");
        server.join().unwrap();
        drop(handle);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn conversation_round_trips_tool_protocol_without_internal_trace_or_orphans() {
        use mochi_core::ai::session::{AiConversation, AiStoredMessage};
        let mut call = AiStoredMessage::new("assistant", "");
        call.set("hidden", true.into());
        call.set("tool_calls",serde_json::json!([{"id":"c1","type":"function","function":{"name":"file_read","arguments":"{}"}},{"id":"missing","type":"function","function":{"name":"file_read","arguments":"{}"}}]));
        let mut result = AiStoredMessage::new("tool", "读取结果");
        result.set("tool_call_id", "c1".into());
        result.set("name", "file_read".into());
        result.set("hidden", true.into());
        let mut trace = AiStoredMessage::new("assistant", "内部轨迹");
        trace.set("hidden", true.into());
        let mut orphan = AiStoredMessage::new("tool", "孤立回执");
        orphan.set("tool_call_id", "orphan".into());
        let c = AiConversation {
            id: "c".into(),
            title: "测试".into(),
            created_at: 0,
            updated_at: 0,
            messages: vec![
                AiStoredMessage::new("user", "读取文件"),
                call,
                result,
                trace,
                orphan,
                AiStoredMessage::new("assistant", "已读取"),
            ],
        };
        let messages = conversation_messages(&c);
        assert_eq!(
            messages.iter().map(|m| m.role.as_str()).collect::<Vec<_>>(),
            ["user", "assistant", "tool", "assistant"]
        );
        assert_eq!(messages[1].tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn a_provider_needs_at_least_a_base_url_and_a_model() {
        let dir = std::env::temp_dir().join(format!("mochi-ai-provider-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let settings = SettingsService::new(Some(dir.join("settings.json")));
        assert!(load_provider(&settings).is_none());
        settings.set(KEY_BASE_URL, "https://api.example.com/v1");
        assert!(load_provider(&settings).is_none(), "还缺 model");
        settings.set(KEY_MODEL, "gpt-4o-mini");
        let p = load_provider(&settings).expect("配齐了");
        assert_eq!(p.model, "gpt-4o-mini");
        assert!(p.stream);
        assert_eq!(p.name, "默认");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_host_reads_ui_state_through_the_snapshot() {
        let dir = std::env::temp_dir().join(format!("mochi-ai-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot = Arc::new(Mutex::new(HostSnapshot::default()));
        let permissions = Arc::new(AiPermissionService::new(&dir));
        let runtime_settings = Arc::new(SettingsService::new(Some(dir.join("settings.json"))));
        permissions
            .save_permissions(vec![mochi_core::domain::AiFolderPermission {
                path: "/".into(),
                level: "suggest".into(),
                inherited: None,
            }])
            .unwrap();
        let host = AppHost::new(&dir, Arc::clone(&snapshot), permissions, || {})
            .with_runtime_settings(Arc::clone(&runtime_settings));
        assert!(host.selected_knowledge_base().is_none());
        snapshot.lock().unwrap().selected_knowledge_base = Some(dir.join("知识库"));
        snapshot.lock().unwrap().session_id = Some("s1".into());
        assert_eq!(host.selected_knowledge_base(), Some(dir.join("知识库")));
        assert_eq!(host.active_session_id(), Some("s1".into()));
        assert!(
            host.should_propose(&dir.join("note.md").to_string_lossy()),
            "默认改动必须生成提案"
        );
        snapshot.lock().unwrap().edit_apply_mode = "auto".into();
        assert!(
            !host.should_propose(&dir.join("note.md").to_string_lossy()),
            "用户确认自动执行后，suggest 路径不能再次生成审批"
        );
        runtime_settings.set("app.ai.editApplyMode", "approve");
        assert!(
            host.should_propose(&dir.join("note.md").to_string_lossy()),
            "持久化的批准模式应覆盖快照"
        );
        runtime_settings.set("app.ai.editApplyMode", "auto");
        assert!(
            !host.should_propose(&dir.join("note.md").to_string_lossy()),
            "运行时必须读取共享设置中的自动执行模式"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn automatic_mode_applies_one_base_batch_under_suggest_and_still_blocks_readonly() {
        use mochi_core::ai::models::{AiToolCall, AiToolFunction};
        use mochi_core::ai::permission::AiPermissionLevel;
        use mochi_core::ai::tools::base_tools::BaseToolExecutor;
        use mochi_core::ai::tools::ToolRegistry;

        const SOURCE: &str = include_str!("../../../../tests/fixtures/base-v1.mcb");
        let dir = std::env::temp_dir().join(format!(
            "mochi-ai-auto-base-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("记录.mcb");
        std::fs::write(&path, SOURCE).unwrap();
        let settings = Arc::new(SettingsService::new(Some(dir.join("settings.json"))));
        settings.set("app.ai.editApplyMode", "auto");
        let permissions = Arc::new(AiPermissionService::new(&dir));
        permissions
            .save_permissions(vec![mochi_core::domain::AiFolderPermission {
                path: "/".into(),
                level: "suggest".into(),
                inherited: None,
            }])
            .unwrap();
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            edit_apply_mode: "auto".into(),
            active_document: Some(ActiveDocument {
                id: "base".into(),
                title: "记录".into(),
                path: path.to_string_lossy().into_owned(),
                is_dirty: false,
            }),
            ..Default::default()
        }));
        let host = Arc::new(
            AppHost::new(&dir, snapshot, Arc::clone(&permissions), || {})
                .with_runtime_settings(settings),
        );
        let registry = ToolRegistry::new(Arc::clone(&permissions))
            .with(Arc::new(BaseToolExecutor::new(host.clone())));
        let call = |record_ids: &[&str]| AiToolCall {
            id: "batch".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: "base_create_records".into(),
                arguments: serde_json::json!({
                    "tableId": "table_secondary",
                    "records": record_ids
                        .iter()
                        .map(|id| serde_json::json!({"recordId": id, "values": {"name": id}}))
                        .collect::<Vec<_>>()
                })
                .to_string(),
            },
        };

        let applied: serde_json::Value =
            serde_json::from_str(&registry.execute(&call(&["auto_a", "auto_b"]))).unwrap();
        assert_eq!(applied["ok"], true, "{applied}");
        assert_eq!(applied["data"]["applied"], true, "{applied}");
        assert_eq!(
            mochi_core::base::parse_base_document(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .tables[1]
                .records
                .len(),
            2
        );
        assert!(host.inbox.list_pending().is_empty());

        permissions
            .set_folder_permission(&path.to_string_lossy(), AiPermissionLevel::ReadOnly)
            .unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        let denied: serde_json::Value =
            serde_json::from_str(&registry.execute(&call(&["blocked_a", "blocked_b"]))).unwrap();
        assert_eq!(denied["ok"], false, "{denied}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_proposal_lands_in_the_agent_inbox_as_pending() {
        let dir = std::env::temp_dir().join(format!("mochi-ai-proposal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let notified = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n = Arc::clone(&notified);
        let host = AppHost::new(
            &dir,
            Arc::new(Mutex::new(HostSnapshot::default())),
            Arc::new(AiPermissionService::new(&dir)),
            move || {
                n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        );
        let out = host
            .submit_proposal(PendingFileOperation {
                kind: "write".into(),
                path: dir.join("a.md").to_string_lossy().into_owned(),
                new_path: None,
                title: "写入 a.md".into(),
                summary: "新建 a.md".into(),
                content: Some("# a".into()),
                previous_content: None,
                entries: Vec::new(),
                reason: "permission-suggest".into(),
            })
            .unwrap();
        assert_eq!(out["status"], "pending");
        let pending = AgentInboxService::new(&dir).list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].title(), "写入 a.md");
        assert_eq!(pending[0].task_name(), Some("AI 助手"));
        assert_eq!(
            notified.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "提案落盘后要叫 UI 刷新"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_range_edit_returns_the_pending_edit_linked_to_its_inbox_entry() {
        use mochi_core::ai::tools::host::ToolHost;
        let dir = std::env::temp_dir().join(format!(
            "mochi-ai-range-proposal-{}",
            mochi_core::paths::random_base36(8)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "第一行\n第二行\n").unwrap();
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            active_document: Some(ActiveDocument {
                id: "tab".into(),
                title: "note.md".into(),
                path: path.to_string_lossy().into_owned(),
                is_dirty: false,
            }),
            active_text: Some("第一行\n第二行\n".into()),
            ..Default::default()
        }));
        let host = AppHost::new(
            &dir,
            snapshot,
            Arc::new(AiPermissionService::new(&dir)),
            || {},
        );
        let edit = mochi_core::document_range::PendingDocumentEdit {
            id: "edit-1".into(),
            path: path.to_string_lossy().into_owned(),
            title: Some("note.md".into()),
            start_line: 2,
            end_line: 2,
            old_text: "第二行".into(),
            new_text: "替换行".into(),
            summary: "替换第二行".into(),
            original_hash: mochi_core::document_range::hash_text("第二行"),
            status: "pending".into(),
            error: None,
        };
        let out = host.propose_document_edit(edit).unwrap();
        assert_eq!(out["status"], "pending");
        assert_eq!(out["pendingEdit"]["id"], "edit-1");
        let inbox_id = out["pendingEdit"]["inboxId"].as_str().unwrap();
        assert_eq!(out["inboxId"].as_str(), Some(inbox_id));
        assert_eq!(AgentInboxService::new(&dir).list_pending().len(), 1);
        assert!(!path.exists() || std::fs::read_to_string(&path).unwrap() == "第一行\n第二行\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_range_edit_with_stale_hash_never_enqueues_a_write() {
        use mochi_core::ai::tools::host::ToolHost;
        let dir = std::env::temp_dir().join(format!(
            "mochi-ai-stale-range-{}",
            mochi_core::paths::random_base36(8)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        let source = "第一行\n已被修改的第二行\n";
        std::fs::write(&path, source).unwrap();
        let host = AppHost::new(
            &dir,
            Arc::new(Mutex::new(HostSnapshot::default())),
            Arc::new(AiPermissionService::new(&dir)),
            || {},
        );
        let edit = mochi_core::document_range::PendingDocumentEdit {
            id: "stale-edit".into(),
            path: path.to_string_lossy().into_owned(),
            title: Some("note.md".into()),
            start_line: 2,
            end_line: 2,
            old_text: "原来的第二行".into(),
            new_text: "替换行".into(),
            summary: "不应写入".into(),
            original_hash: mochi_core::document_range::hash_text("原来的第二行"),
            status: "pending".into(),
            error: None,
        };
        let error = host.propose_document_edit(edit).unwrap_err();
        assert!(error.contains("原文已变化"), "{error}");
        assert!(AgentInboxService::new(&dir).list_pending().is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auto_mode_cannot_route_persisted_blocks_through_file_or_range_tools() {
        use mochi_core::ai::models::{AiToolCall, AiToolFunction};
        use mochi_core::ai::tools::core::CoreToolExecutor;
        use mochi_core::ai::tools::host::ToolHost;
        use mochi_core::ai::tools::range_edit_tools::RangeEditToolExecutor;
        use mochi_core::ai::tools::ToolRegistry;

        let dir = std::env::temp_dir().join(format!(
            "mochi-ai-block-bypass-{}",
            mochi_core::paths::random_base36(12)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        // 保留一个块标记和一个新插入的无标记块。即使文档只转换了一部分、
        // `has_persisted_ids()` 因此为 false，整文件工具也必须继续被拦。
        let source = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->\n第一块。\n\n第二块。\n";
        std::fs::write(&path, source).unwrap();
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            edit_apply_mode: "auto".into(),
            active_document: Some(ActiveDocument {
                id: "tab-1".into(),
                title: "note.md".into(),
                path: path.to_string_lossy().into_owned(),
                is_dirty: false,
            }),
            active_text: Some(source.to_owned()),
            ..Default::default()
        }));
        let permissions = Arc::new(AiPermissionService::new(&dir));
        let host = Arc::new(AppHost::new(&dir, snapshot, permissions.clone(), || {}));
        assert!(host.should_propose(&path.to_string_lossy()));
        let registry = ToolRegistry::new(permissions)
            .with(Arc::new(CoreToolExecutor::new(host.clone())))
            .with(Arc::new(RangeEditToolExecutor::new(host.clone())));

        let call = |name: &str, args: serde_json::Value| AiToolCall {
            id: format!("call-{name}"),
            kind: "function".into(),
            function: AiToolFunction {
                name: name.into(),
                arguments: args.to_string(),
            },
        };
        let file_result: serde_json::Value = serde_json::from_str(&registry.execute(&call(
            "file_write",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "mode": "overwrite",
                "content": "整篇覆盖不应发生"
            }),
        )))
        .unwrap();
        assert_eq!(file_result["ok"], false, "{file_result}");
        assert!(file_result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("遗留 mochi:block"));

        let range_result: serde_json::Value = serde_json::from_str(&registry.execute(&call(
            "document_range_propose_edit",
            serde_json::json!({
                "path": path.to_string_lossy(),
                "startLine": 1,
                "endLine": 1,
                "replacement": "行工具绕过不应发生"
            }),
        )))
        .unwrap();
        assert_eq!(range_result["ok"], false, "{range_result}");
        assert!(range_result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("遗留 mochi:block"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        assert!(AgentInboxService::new(&dir).list_pending().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_marker_or_conversion_backup_blocks_auto_overwrite() {
        use mochi_core::ai::models::{AiToolCall, AiToolFunction};
        use mochi_core::ai::tools::core::CoreToolExecutor;
        use mochi_core::ai::tools::ToolRegistry;

        let dir = std::env::temp_dir().join(format!(
            "mochi-ai-block-malformed-{}",
            mochi_core::paths::random_base36(12)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        let path_text = path.to_string_lossy().into_owned();
        let malformed = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001\n损坏的块\n";
        std::fs::write(&path, malformed).unwrap();
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            edit_apply_mode: "auto".into(),
            active_document: Some(ActiveDocument {
                id: "tab-1".into(),
                title: "note.md".into(),
                path: path_text.clone(),
                is_dirty: false,
            }),
            active_text: Some(malformed.to_owned()),
            ..Default::default()
        }));
        let permissions = Arc::new(AiPermissionService::new(&dir));
        let host = Arc::new(AppHost::new(
            &dir,
            snapshot.clone(),
            permissions.clone(),
            || {},
        ));
        assert!(!host.has_persisted_blocks(&dir.join("new-document.md").to_string_lossy()));
        let registry = ToolRegistry::new(permissions).with(Arc::new(CoreToolExecutor::new(host)));
        let call = |content: &str| AiToolCall {
            id: "malformed-file-write".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: "file_write".into(),
                arguments: serde_json::json!({
                    "path": path_text.clone(),
                    "mode": "overwrite",
                    "content": content,
                })
                .to_string(),
            },
        };
        let malformed_result: serde_json::Value =
            serde_json::from_str(&registry.execute(&call("损坏文档也不能整篇覆盖"))).unwrap();
        assert_eq!(malformed_result["ok"], false, "{malformed_result}");
        assert!(malformed_result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("遗留 mochi:block"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);

        // 就算当前源码里已没有标记注释，转换备份依旧是保护信号。
        let plain = "当前正文没有 marker\n";
        std::fs::write(&path, plain).unwrap();
        std::fs::write(
            mochi_core::document_blocks::conversion_backup_path(&path),
            "转换前正文\n",
        )
        .unwrap();
        snapshot.lock().unwrap().active_text = Some(plain.to_owned());
        let backup_result: serde_json::Value =
            serde_json::from_str(&registry.execute(&call("存在转换备份也不能整篇覆盖"))).unwrap();
        assert_eq!(backup_result["ok"], false, "{backup_result}");
        assert!(backup_result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("遗留 mochi:block"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), plain);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dirty_host_queues_each_block_edit_as_an_independent_inbox_entry() {
        use mochi_core::ai::tools::host::ToolHost;

        let dir = std::env::temp_dir().join(format!(
            "mochi-ai-block-proposals-{}",
            mochi_core::paths::random_base36(12)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("note.md");
        // 编辑器缓冲是含持久化 ID 的源码，磁盘上的文件则刻意是旧的。
        // 宿主的每次读取都必须用脏缓冲。
        let disk_source = "磁盘上的旧内容\n";
        std::fs::write(&path, disk_source).unwrap();
        let dirty_source =
            mochi_core::document_blocks::prepare_conversion(&path, "第一块。\n\n第二块。\n")
                .unwrap()
                .converted_source;
        let snapshot = Arc::new(Mutex::new(HostSnapshot {
            active_document: Some(ActiveDocument {
                id: "tab-1".into(),
                title: "note.md".into(),
                path: path.to_string_lossy().into_owned(),
                is_dirty: true,
            }),
            active_text: Some(dirty_source.clone()),
            ..Default::default()
        }));
        let host = AppHost::new(
            &dir,
            snapshot,
            Arc::new(AiPermissionService::new(&dir)),
            || {},
        );

        let path_text = path.to_string_lossy().into_owned();
        let listed = host.list_document_blocks(&path_text).unwrap();
        assert_eq!(listed["hasPersistedIds"], true);
        assert_eq!(listed["needsConversion"], false);
        let blocks = listed["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);

        let proposal_for = |block: &serde_json::Value, replacement: &str| BlockEditProposal {
            block_id: block["blockId"].as_str().unwrap().into(),
            original_hash: block["originalHash"].as_str().unwrap().into(),
            old_text: block["text"].as_str().unwrap().into(),
            new_text: replacement.into(),
            path: path_text.clone(),
            summary: format!("修改 {}", block["blockId"].as_str().unwrap()),
        };
        let first = proposal_for(&blocks[0], "第一块（建议）");
        let second = proposal_for(&blocks[1], "第二块（建议）");

        // 真实的 AppHost 每次宿主调用持久化一条收件箱条目，
        // 且绝不写活动缓冲或过期文件。
        let first_result = host.propose_block_edit(first.clone()).unwrap();
        let second_result = host.propose_block_edit(second.clone()).unwrap();
        assert_eq!(first_result["status"], "pending");
        assert_eq!(second_result["status"], "pending");
        assert_ne!(first_result["inboxId"], second_result["inboxId"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), disk_source);

        let inbox = AgentInboxService::new(&dir);
        let pending = inbox.list_pending();
        assert_eq!(pending.len(), 2);
        for (entry, expected) in pending.iter().zip([first, second]) {
            assert_eq!(entry.kind(), "block-edit");
            assert_eq!(entry.path(), path_text);
            let operation = entry.to_value()["operation"].clone();
            assert_eq!(operation["status"], "pending");
            assert_eq!(operation["kind"], "block-edit");
            assert_eq!(operation["blockEdit"]["blockId"], expected.block_id);
            assert_eq!(
                operation["blockEdit"]["originalHash"],
                expected.original_hash
            );
            assert_eq!(operation["blockEdit"]["oldText"], expected.old_text);
            assert_eq!(operation["blockEdit"]["newText"], expected.new_text);
            assert_eq!(operation["blockEdit"]["path"], expected.path);
            assert_eq!(operation["blockId"], operation["blockEdit"]["blockId"]);
        }

        // 处理一条后，另一条必须仍可独立审批；
        // 重启后的服务从磁盘读到同样的待处理状态。
        let rejected_id = pending[0].id().to_owned();
        inbox.resolve(&rejected_id, "rejected", None).unwrap();
        let after_restart = AgentInboxService::new(&dir).list_pending();
        assert_eq!(after_restart.len(), 1);
        assert_ne!(after_restart[0].id(), rejected_id);
        assert_eq!(after_restart[0].kind(), "block-edit");
        assert_eq!(after_restart[0].status(), "pending");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), disk_source);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
