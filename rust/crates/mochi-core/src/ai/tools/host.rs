//! UI 状态和审批入口由 ToolHost 提供，核心工具不直接访问窗口。

use std::path::{Path, PathBuf};

use crate::paths;

/// 当前活动文档。由 UI 层提供——核心层没法知道用户正在看哪个标签页。
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveDocument {
    pub id: String,
    pub title: String,
    pub path: String,
    pub is_dirty: bool,
}

/// 文件变更通知，供 UI 刷新文件树/重载编辑器。
#[derive(Debug, Clone, PartialEq)]
pub enum FileChange {
    Write {
        path: String,
    },
    Delete {
        path: String,
    },
    FolderCreate {
        path: String,
    },
    Rename {
        path: String,
        new_path: String,
    },
    /// 子文档建在隐藏伴生夹里，文件监听看不到——宿主要按索引重建文件树。
    SubDocumentCreate {
        parent_path: String,
        path: String,
    },
}

/// suggest 级路径上的写操作不直接落盘，而是转成待批准提案。
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFileOperation {
    /// `write` | `overwrite` | `delete-file` | `delete-folder` | `rename`
    pub kind: String,
    pub path: String,
    pub new_path: Option<String>,
    pub title: String,
    pub summary: String,
    pub content: Option<String>,
    pub previous_content: Option<String>,
    /// 目录删除时列出直接子项，让用户知道波及范围
    pub entries: Vec<String>,
    pub reason: String,
}

/// 一条块级编辑提案。
///
/// 刻意停在 AI 宿主这一层，不去依赖块解析器的具体模型。
/// 块身份与替换校验归解析器管；当前编辑器缓冲和审批队列归宿主管。
/// 这个线上类型保持精简，无头宿主才能只返回结构化提案、绝不落盘。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockEditProposal {
    pub block_id: String,
    pub original_hash: String,
    pub old_text: String,
    pub new_text: String,
    pub path: String,
    pub summary: String,
}

/// 知识库摘要。对齐 TS 的 `summarizeNode`。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBase {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: u64,
    pub mtime: Option<String>,
    pub child_count: usize,
}

/// 列出工作区根下的顶层目录。忽略点目录与构建目录，与文件树的规则一致。
pub fn list_top_level_directories(root: &Path) -> Vec<KnowledgeBase> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<KnowledgeBase> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || crate::files::ignored_dirs().contains(name.as_str()) {
                return None;
            }
            let path = e.path();
            let meta = e.metadata().ok()?;
            let child_count = std::fs::read_dir(&path)
                .map(|d| d.flatten().count())
                .unwrap_or(0);
            Some(KnowledgeBase {
                name,
                path: paths::to_forward_slashes(&path.to_string_lossy()),
                kind: "directory".into(),
                size: meta.len(),
                mtime: meta.modified().ok().map(|t| crate::jstime::from(t.into())),
                child_count,
            })
        })
        .collect();
    out.sort_by(|a, b| crate::files::locale_compare(&a.name, &b.name));
    out
}

pub trait ToolHost: Send + Sync {
    fn supports_console(&self) -> bool {
        false
    }
    fn console(&self, _name: &str, _args: &serde_json::Value) -> Result<serde_json::Value, String> {
        Err("当前宿主不支持控制台操作".into())
    }
    /// 文件夹建议默认强制走审批。UI 宿主可以尊重一个显式的
    /// 自动应用偏好，与其他编辑工具保持一致。
    fn canvas_requires_proposal(&self, path: &str) -> bool {
        crate::ai::permission::AiPermissionService::new(self.workspace_root())
            .path_requires_proposal(path)
    }
    /// UI 宿主把画布的读取/编辑都排在 UI 线程上，实时编辑与撤销才不受影响。
    fn canvas_action(
        &self,
        path: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        // 无头实现没有打开的编辑器，也没有撤销栈。
        super::canvas_tools::file_action(self, path, name, args)
    }
    /// 受信任的工作流脚本直接用 OS 文件访问，不受 AI 路径范围约束。
    fn trusted_workflow(&self) -> bool {
        false
    }
    fn read_document_text(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
    fn propose_document_edit(
        &self,
        edit: crate::document_range::PendingDocumentEdit,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"pendingEdit":edit}))
    }

    /// 返回已解析文档路径的块解析结果。
    ///
    /// 返回对象在这一层刻意是不透明的 JSON。实现应包含 `path`、
    /// `documentId`、`hasPersistedIds`/`needsConversion`，以及一个
    /// `blocks` 数组，其记录至少带 `blockId`、`text`（或 `content`）
    /// 和 `originalHash`。凡是生成出来的、按位置推导的 ID，
    /// 绝不能冒充持久化 ID 返回。
    fn list_document_blocks(&self, absolute_path: &str) -> Result<serde_json::Value, String> {
        let source = self.read_document_text(absolute_path)?;
        let document =
            crate::document_blocks::document_from_source(Path::new(absolute_path), &source)
                .map_err(|e| format!("解析文档块失败: {e}"))?;
        let persisted = document.has_persisted_ids();
        let blocks = if persisted {
            crate::document_blocks::list_blocks(&document)
                .into_iter()
                .map(|block| {
                    serde_json::json!({
                        "blockId": block.id,
                        "kind": block.kind,
                        "text": block.content,
                        "content": block.content,
                        "originalHash": block.hash,
                        "sourceSpan": block.source_span,
                        "persisted": true
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        Ok(serde_json::json!({
            "path": absolute_path,
            "documentId": document.document_id,
            "sourceHash": document.content_hash(),
            "hasPersistedIds": persisted,
            "needsConversion": !persisted,
            "blockCount": document.blocks().len(),
            "blocks": blocks
        }))
    }

    /// 从已解析的文档路径读取一个块。
    ///
    /// 从已解析的文档路径读取一个块。
    ///
    /// 文档还没带标记的宿主，应返回结构化的 `needsConversion: true`
    ///（把转换请求交给 UI），或一个明确的错误。读取操作绝不能顺手
    /// 写标记注释——尤其当内存里的编辑器缓冲还是脏的时候。
    fn read_document_block(
        &self,
        absolute_path: &str,
        block_id: &str,
    ) -> Result<serde_json::Value, String> {
        let source = self.read_document_text(absolute_path)?;
        let document =
            crate::document_blocks::document_from_source(Path::new(absolute_path), &source)
                .map_err(|e| format!("解析文档块失败: {e}"))?;
        if !document.has_persisted_ids() {
            return Ok(serde_json::json!({
                "path": absolute_path,
                "blockId": block_id,
                "hasPersistedIds": false,
                "needsConversion": true,
                "message": "该文档尚未持久化块 ID，请先在界面中转换文档"
            }));
        }
        let block = crate::document_blocks::get_block(&document, block_id)
            .map_err(|e| format!("读取块失败: {e}"))?;
        Ok(serde_json::json!({
            "path": absolute_path,
            "documentId": document.document_id,
            "blockId": block.id,
            "kind": block.kind,
            "text": block.content,
            "content": block.content,
            "originalHash": block.hash,
            "sourceSpan": block.source_span,
            "persisted": true
        }))
    }

    /// 请 UI 把一个没有标记的文档转换成持久化块 ID。
    ///
    /// 这个钩子只是请求/通知。默认响应是结构化的，不产生文件系统副作用；
    /// UI 宿主可以附上请求 id，等用户确认后再执行显式转换。
    fn request_document_block_conversion(
        &self,
        absolute_path: &str,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "status": "needsConversion",
            "needsConversion": true,
            "path": absolute_path,
            "message": "该文档尚未持久化块 ID，请先在界面中转换文档"
        }))
    }

    /// 把一条可独立审批的块编辑排队。
    ///
    /// 实现必须重新读取当前编辑器缓冲、核对 `oldText` 和 `originalHash`，
    /// 并在返回前持久化一条 `kind: block-edit` 的收件箱操作。
    /// 没有真实审批队列的宿主必须拒绝请求，而不是返回一个
    /// 谁也批不了的「待审批」假值。
    fn propose_block_edit(&self, _edit: BlockEditProposal) -> Result<serde_json::Value, String> {
        Err("当前宿主没有块修改审批队列，请使用支持审批的宿主".into())
    }
    fn fixed_setting(&self, _key: &str) -> Option<crate::app_settings::SettingValue> {
        None
    }
    fn workspace_root(&self) -> &Path;
    fn supports_document_export(&self) -> bool {
        false
    }
    fn propose_document_export(
        &self,
        _source: &str,
        _format: &str,
        _output: &str,
    ) -> Result<serde_json::Value, String> {
        Err("当前宿主无法创建导出审批，请使用界面导出".into())
    }
    fn export_document(&self, _source: &str, _format: &str, _output: &str) -> Result<(), String> {
        Err("当前宿主没有文档导出能力".into())
    }

    /// 当前选中的知识库绝对路径。相对路径以它为基准解析。
    fn selected_knowledge_base(&self) -> Option<PathBuf> {
        None
    }

    fn active_document(&self) -> Option<ActiveDocument> {
        None
    }

    /// 路径对 AI 是否可见（设为不可见的路径会被隐藏）。默认全部可见，由上层接入权限服务。
    fn is_path_visible(&self, _absolute_path: &str) -> bool {
        true
    }

    /// 该路径的写操作是否需要转成提案（suggest 级）。
    fn should_propose(&self, _absolute_path: &str) -> bool {
        false
    }

    /// 提交待批准提案。默认实现直接拒绝——没有 UI 承接提案时，
    /// 宁可让工具失败也不能悄悄把 suggest 级的写操作直接落盘。
    fn submit_proposal(
        &self,
        operation: PendingFileOperation,
    ) -> Result<serde_json::Value, String> {
        Err(format!(
            "路径处于「建议」级权限，{} 需要用户批准，但当前没有可用的批准界面",
            operation.summary
        ))
    }

    /// 顶层知识库（工作区根下的目录）。默认从磁盘直接列，UI 层可覆盖成
    /// 「当前文件树里的目录」以保持与界面一致。
    fn knowledge_bases(&self) -> Vec<KnowledgeBase> {
        list_top_level_directories(self.workspace_root())
    }

    /// 切换当前选中的知识库。默认拒绝——这是 UI 状态，无头环境下没有"选中"这回事。
    fn select_knowledge_base(&self, _absolute_path: &str) -> Result<(), String> {
        Err("当前环境不支持切换知识库".into())
    }

    /// 允许 AI 免批准直接执行的程序名（小写、不带扩展名）。
    /// 默认空——白名单之外的命令一律转成待批准提案。
    fn shell_whitelist(&self) -> Vec<String> {
        Vec::new()
    }
    fn propose_shell_command(
        &self,
        proposal: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Ok(proposal)
    }

    /// 打开设置窗口并跳到指定分页；无窗口宿主的默认实现返回错误。
    fn open_settings_ui(&self, _tab: &str) -> Result<(), String> {
        Err("当前环境没有可打开的设置界面".into())
    }

    /// 当前 AI 会话 id，写进记忆里用于回溯「这条是哪次对话记下的」。
    fn active_session_id(&self) -> Option<String> {
        None
    }

    /// 更新本轮任务的执行计划（`agent_plan`）。UI 据此渲染步骤清单；
    /// 无头环境下没人看，丢掉即可。
    fn update_plan(&self, _steps: &[serde_json::Value], _note: Option<&str>) {}

    /// 落盘前对内容做宿主特有的归一化。渲染进程那边会修正 Markdown 里的 LaTeX 转义，
    /// 核心层不该知道这种细节，所以留成钩子。
    fn transform_content_before_write(&self, _absolute_path: &str, content: &str) -> String {
        content.to_owned()
    }

    fn supports_desktop_cards(&self) -> bool {
        false
    }
    fn desktop_cards(
        &self,
        _name: &str,
        _args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("当前宿主未连接桌面卡片，请在墨池中使用此工具".into())
    }
    fn notify_change(&self, _change: FileChange) {}
}

/// 最小宿主：只有工作区根，没有 UI 状态。用于后台任务与测试。
pub struct HeadlessHost {
    workspace_root: PathBuf,
}

impl HeadlessHost {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        Self {
            workspace_root: workspace_root.as_ref().to_path_buf(),
        }
    }
}

impl ToolHost for HeadlessHost {
    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

/// 统一成正斜杠并去掉尾部斜杠（根路径除外）。
pub fn normalize_agent_path(input: &str) -> String {
    let slashed = paths::to_forward_slashes(input.trim());
    let trimmed = slashed.trim_end_matches('/');
    if trimmed.is_empty() && slashed.starts_with('/') {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub fn join_agent_path(base: &str, child: &str) -> String {
    let base = base.trim_end_matches('/');
    let child = child.trim_start_matches('/');
    if child.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{child}")
    }
}

pub fn agent_path_basename(path: &str) -> String {
    normalize_agent_path(path)
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[derive(Debug, Clone, Copy)]
pub struct ResolveOptions {
    /// 相对路径是否以「当前选中的知识库」为基准（否则以工作区根为基准）。
    pub fallback_to_selection: bool,
    /// 是否允许触达 `.mochi` 元数据目录。
    pub allow_mochi_dir: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            fallback_to_selection: true,
            allow_mochi_dir: false,
        }
    }
}

/// 把模型给的路径解析成工作区内的绝对路径。**这是一道安全边界**：
/// 输入看起来是不是绝对路径：`C:/…`、`/…`、UNC `//server/…`。
/// 已归一成正斜杠，所以只认这三种形状。
fn is_absolute_input(normalized: &str) -> bool {
    if normalized.starts_with('/') {
        return true;
    }
    let bytes = normalized.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// 找到目标路径上最近的真实存在项。对不存在的写入目标，检查它的
/// existing ancestor；这样工作区内的 junction/symlink 目录也会被解析到
/// 它真正指向的位置，而不是只靠字符串前缀判断。
fn canonical_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        if current.exists() {
            return std::fs::canonicalize(current).ok();
        }
        if !current.pop() {
            return None;
        }
    }
}

fn canonical_is_within(root: &Path, target: &Path) -> bool {
    paths::path_is_within(root, target)
}

fn lexical_is_within(root: &str, target: &str) -> bool {
    let windows_path =
        root.as_bytes().get(1) == Some(&b':') || target.as_bytes().get(1) == Some(&b':');
    if windows_path {
        let root = root.to_lowercase();
        let target = target.to_lowercase();
        target == root || target.starts_with(&format!("{root}/"))
    } else {
        target == root || target.starts_with(&format!("{root}/"))
    }
}

fn validate_real_containment(workspace: &str, absolute: &str) -> Result<(), String> {
    let root = Path::new(workspace);
    let Some(canonical_root) = std::fs::canonicalize(root).ok() else {
        // 单元测试和无头调用方可以故意用一个虚拟根来演练词法路径规则。
        // 这种情况下没有可解析的文件系统目标，保留既有的线上行为。
        return Ok(());
    };
    let Some(canonical_target) = canonical_existing_ancestor(Path::new(absolute)) else {
        return Err("路径无法解析".into());
    };
    if canonical_is_within(&canonical_root, &canonical_target) {
        Ok(())
    } else {
        Err("路径真实位置超出当前工作区".into())
    }
}

/// 拒绝 `..`、拒绝越出工作区、默认拒绝 `.mochi`。
pub fn resolve_workspace_path(
    host: &dyn ToolHost,
    input_path: Option<&str>,
    options: ResolveOptions,
) -> Result<String, String> {
    let workspace = normalize_agent_path(&host.workspace_root().to_string_lossy());
    if workspace.is_empty() {
        return Err("尚未打开工作区".into());
    }

    let base = if options.fallback_to_selection {
        host.selected_knowledge_base()
            .map(|p| normalize_agent_path(&p.to_string_lossy()))
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| workspace.clone())
    } else {
        workspace.clone()
    };

    let normalized = normalize_agent_path(input_path.unwrap_or(""));

    if host.trusted_workflow() {
        let path = Path::new(&normalized);
        return Ok(if path.is_absolute() {
            normalized
        } else {
            join_agent_path(&workspace, &normalized)
        });
    }

    // 先于任何拼接检查——`a/../../etc` 拼完再查就晚了
    if normalized.split('/').any(|seg| seg == "..") {
        return Err("路径不能包含 ..".into());
    }

    let absolute = if normalized.is_empty() {
        base
    } else if lexical_is_within(&workspace, &normalized) {
        normalized
    } else if is_absolute_input(&normalized) {
        // **有意偏离 TS**：那边会把 `C:/Windows/win.ini` 直接拼到基准目录后面，
        // 造出 `<工作区>/C:/Windows/win.ini` 这种不存在的路径，然后一本正经地
        // 报告"这篇文档没有批注"。没有越权风险，但模型会被这个假路径带偏。
        // 工作区外的绝对路径就是越界，如实拒绝。
        return Err("路径超出当前工作区".into());
    } else {
        join_agent_path(&base, &normalized)
    };

    if !lexical_is_within(&workspace, &absolute) {
        return Err("路径超出当前工作区".into());
    }

    if !options.allow_mochi_dir && absolute.split('/').any(|seg| seg == paths::MOCHI_DIR_NAME) {
        return Err("AI 工具默认不能访问 .mochi 元数据目录".into());
    }

    validate_real_containment(&workspace, &absolute)?;

    Ok(absolute)
}

/// 文件/文件夹名合法性。字符集对齐 TS 的 `validateName`：
/// `^[^<>:"/\\|?*]+$`——那正是 Windows 文件名的非法字符集。
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("名称不能为空".into());
    }
    if name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c)) {
        return Err("名称包含非法字符".into());
    }
    if name == "." || name == ".." || name.ends_with(['.', ' ']) {
        return Err("名称非法".into());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if reserved {
        return Err("名称是 Windows 保留名称".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Host {
        root: PathBuf,
        selected: Option<PathBuf>,
    }
    impl ToolHost for Host {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn selected_knowledge_base(&self) -> Option<PathBuf> {
            self.selected.clone()
        }
    }

    fn host(selected: Option<&str>) -> Host {
        Host {
            root: PathBuf::from("D:/ws"),
            selected: selected.map(PathBuf::from),
        }
    }

    fn resolve(h: &Host, input: Option<&str>) -> Result<String, String> {
        resolve_workspace_path(h, input, ResolveOptions::default())
    }

    #[test]
    fn relative_paths_resolve_against_the_selected_knowledge_base() {
        let h = host(Some("D:/ws/知识库/计算机通识"));
        assert_eq!(
            resolve(&h, Some("a.md")).unwrap(),
            "D:/ws/知识库/计算机通识/a.md"
        );
        assert_eq!(
            resolve(&h, None).unwrap(),
            "D:/ws/知识库/计算机通识",
            "空路径即当前知识库"
        );
    }

    #[test]
    fn without_a_selection_relative_paths_fall_back_to_the_workspace_root() {
        let h = host(None);
        assert_eq!(resolve(&h, Some("a.md")).unwrap(), "D:/ws/a.md");
    }

    #[test]
    fn fallback_can_be_disabled() {
        let h = host(Some("D:/ws/知识库/计算机通识"));
        let opts = ResolveOptions {
            fallback_to_selection: false,
            ..Default::default()
        };
        assert_eq!(
            resolve_workspace_path(&h, Some("a.md"), opts).unwrap(),
            "D:/ws/a.md"
        );
    }

    #[test]
    fn absolute_paths_inside_the_workspace_pass_through() {
        let h = host(Some("D:/ws/知识库"));
        assert_eq!(
            resolve(&h, Some("D:/ws/别处/b.md")).unwrap(),
            "D:/ws/别处/b.md"
        );
    }

    #[test]
    fn backslashes_are_normalized() {
        let h = host(None);
        assert_eq!(
            resolve(&h, Some("知识库\\a.md")).unwrap(),
            "D:/ws/知识库/a.md"
        );
    }

    /// 目录穿越必须在拼接**之前**就被拒——拼完再查就晚了。
    #[test]
    fn parent_traversal_is_rejected() {
        let h = host(Some("D:/ws/知识库"));
        for evil in ["../secret", "a/../../etc", "..", "知识库/../../x"] {
            let err = resolve(&h, Some(evil)).unwrap_err();
            assert!(err.contains(".."), "{evil} 应被拒: {err}");
        }
    }

    /// 只有「字符串前缀像工作区、实际却不在里面」的路径会被显式拒绝。
    /// `D:/wsx` 前缀匹配 `D:/ws`，但不是它的子路径。
    #[test]
    fn sibling_prefix_directory_is_rejected() {
        let h = host(None);
        let err = resolve(&h, Some("D:/wsx/a.md")).unwrap_err();
        assert!(err.contains("超出"), "{err}");
    }

    // 工作区外的绝对路径必须拒绝，不能当相对路径拼接。
    #[test]
    fn foreign_absolute_paths_are_rejected_not_joined() {
        let h = host(None);
        for input in [
            "C:/Windows/System32",
            "D:/别的工作区/a.md",
            "/etc/passwd",
            "//server/share/a.md",
        ] {
            let err = resolve(&h, Some(input)).unwrap_err();
            assert!(err.contains("超出"), "{input} → {err}");
        }
        // 相对路径不受影响
        assert_eq!(
            resolve(&h, Some("知识库/a.md")).unwrap(),
            "D:/ws/知识库/a.md"
        );
    }

    #[test]
    fn mochi_metadata_dir_is_off_limits_by_default() {
        let h = host(None);
        let err = resolve(&h, Some(".mochi/index.db")).unwrap_err();
        assert!(err.contains(".mochi"), "{err}");

        let opts = ResolveOptions {
            allow_mochi_dir: true,
            ..Default::default()
        };
        assert!(resolve_workspace_path(&h, Some(".mochi/index.db"), opts).is_ok());
    }

    #[test]
    fn workspace_root_itself_is_allowed() {
        let h = host(None);
        assert_eq!(resolve(&h, Some("D:/ws")).unwrap(), "D:/ws");
    }

    #[test]
    fn path_helpers() {
        assert_eq!(normalize_agent_path("D:\\ws\\a\\"), "D:/ws/a");
        assert_eq!(join_agent_path("D:/ws", "/a.md"), "D:/ws/a.md");
        assert_eq!(join_agent_path("D:/ws/", "a.md"), "D:/ws/a.md");
        assert_eq!(join_agent_path("D:/ws", ""), "D:/ws");
        assert_eq!(agent_path_basename("D:/ws/知识库/a.md"), "a.md");
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("笔记.md").is_ok());
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            ".",
            "..",
            "CON",
            "nul.txt",
            "COM1.md",
            "LPT9",
            "尾点.",
            "尾空格 ",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} 应被拒");
        }
    }

    #[test]
    fn existing_junction_targets_cannot_escape_the_workspace() {
        let suffix = crate::paths::random_base36(10);
        let root = std::env::temp_dir().join(format!("mochi-resolve-root-{suffix}"));
        let outside = std::env::temp_dir().join(format!("mochi-resolve-outside-{suffix}"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.md"), "outside").unwrap();
        let link = root.join("linked");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.to_string_lossy(),
                &outside.to_string_lossy(),
            ])
            .status()
            .unwrap();
        if !status.success() {
            // 宿主策略可能禁用 junction 创建；这种机器上词法路径
            // 测试照样要能跑。
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let h = Host {
            root: root.clone(),
            selected: None,
        };
        let input = format!("linked{}secret.md", std::path::MAIN_SEPARATOR);
        let error = resolve(&h, Some(&input)).unwrap_err();
        assert!(error.contains("真实位置"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 没有 UI 承接提案时，suggest 级的写操作必须失败，绝不能悄悄落盘。
    #[test]
    fn headless_host_refuses_proposals_instead_of_writing() {
        let h = HeadlessHost::new("D:/ws");
        let err = h
            .submit_proposal(PendingFileOperation {
                kind: "overwrite".into(),
                path: "D:/ws/a.md".into(),
                new_path: None,
                title: "a.md".into(),
                summary: "覆盖写入 a.md".into(),
                content: Some("x".into()),
                previous_content: None,
                entries: Vec::new(),
                reason: "destructive".into(),
            })
            .unwrap_err();
        assert!(err.contains("批准"), "{err}");
    }

    #[test]
    fn headless_host_refuses_block_proposals_without_a_queue() {
        let h = HeadlessHost::new("D:/ws");
        let err = h
            .propose_block_edit(BlockEditProposal {
                block_id: "block-1".into(),
                original_hash: "hash-1".into(),
                old_text: "old".into(),
                new_text: "new".into(),
                path: "D:/ws/a.md".into(),
                summary: "edit block".into(),
            })
            .unwrap_err();
        assert!(err.contains("审批"), "{err}");
    }
}
