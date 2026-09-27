//! 文件树用 arena 下标关联父子，不持有渲染资源。

mod external_changes;
mod favorite_tree;
mod file_operations;
mod library_order;
mod saving;
mod tab_access;
mod tree_interactions;
mod tree_jobs;
mod tree_loading;
mod tree_reorder;
mod tree_scope;
mod workspace_loading;

use favorite_tree::{build_favorites_tree, favorite_ancestor_dirs};
use file_operations::companion_target_conflict;
use tree_loading::{
    build_tree, library_root, path_key, read_manual_sort_orders, remap_expanded_paths,
    remove_expanded_under,
};
use workspace_loading::{configure_git, flatten};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use mochi_core::domain::{FileNode, Library, LibraryType};
use mochi_core::files::FileService;
use mochi_core::git::GitService;
use mochi_core::metadata_index::MetadataIndexService;
use mochi_core::watcher::{WatchChangeType, WatcherService};
use mochi_core::workspace::WorkspaceService;

mod base_references;
mod favorites;
mod navigation;
pub use navigation::Direction as NavigationDirection;

fn native_path_identity(path: &Path) -> PathBuf {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy().replace('\\', "/");
    PathBuf::from(if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    })
}

fn same_native_path(left: &Path, right: &Path) -> bool {
    native_path_identity(left) == native_path_identity(right)
}

fn native_path_is_same_or_under(path: &Path, parent: &Path) -> bool {
    let path = native_path_identity(path);
    let parent = native_path_identity(parent);
    path == parent || path.starts_with(parent)
}

/// 扁平化后的一行。渲染层只认这个，不认树。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub path: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
    /// 映射文件夹是挂载在知识库目录中的虚拟入口，不能参与物理文件移动。
    pub is_mapped_folder: bool,
    /// 目录或挂载了子文档的文档是否已展开。
    pub expanded: bool,
    /// 目录或文档是否有子项——决定画不画折叠三角
    pub has_children: bool,
}

pub struct Shell {
    favorites: Option<mochi_core::favorites::Favorites>,
    favorites_error: String,
    /// 收藏树是当前文件树的一个独立视图，知识库范围仍保存在 `WorkspaceState::scope`。
    favorites_selected: bool,
    /// 收藏侧栏是否保留收藏文件的父级目录。默认关闭时直接平铺收藏文件。
    show_favorite_parents: bool,
    sort_mode: String,
    manual_sort_orders: HashMap<String, Vec<String>>,
    workspace: Option<WorkspaceState>,
    /// 展开的目录集合。用绝对路径而不是下标，这样刷新文件树后展开状态不会错位。
    expanded: HashSet<PathBuf>,
    /// 收藏树自己的展开状态，切回知识库时不丢失用户的折叠选择。
    favorites_expanded: HashSet<PathBuf>,
    favorites_expanded_initialized: bool,
    rows: Vec<Row>,
    tree_loads: tree_jobs::TreeLoads,
    selected: Option<usize>,
    status: String,
    /// 打开的标签页。顺序即标签栏顺序。
    tabs: Vec<OpenTab>,
    active_tab: Option<usize>,
    /// 即使当前窗格显示的是同一个标签页，仍可能有另一个可见编辑器正在使用此文档。
    /// 替换文档时不能丢弃另一个编辑器中的缓冲区。
    protected_view_path: Option<PathBuf>,
    /// 保存时由 Markdown/TXT 自动升级为 `.mc` 的路径变更，交给 App 同步 UI 状态。
    document_format_changes: Vec<(PathBuf, PathBuf)>,
}

/// 一个打开的标签。
///
/// 文件标签的内容读进来就留着——切回来时不该再读一次盘，编辑器改的就是这份。
/// 文档内容由编辑器持有，切回标签时不会重复读取磁盘。
pub struct OpenTab {
    pub kind: TabKind,
    pub title: String,
    pub pinned: bool,
    /// 该标签自己的滚动位置（像素）。切走再切回来要停在原处。
    pub scroll: f32,
    history: navigation::History,
    navigation_chunk: Option<usize>,
}

/// 标签的种类。Electron 的 `Tab.type` 有 file / settings / ai-prompts / welcome 等，
/// 这里只搬会出现在标签栏里的三种。
pub enum TabKind {
    /// 资料库首页标签页没有选中的文件。
    Library { path: PathBuf },
    File {
        path: PathBuf,
        /// 可编辑的内容。**每个标签一份**——共用一个缓冲区的话，
        /// 切标签时光标和撤销历史会串到别的文件上。
        buffer: crate::ui::editor::TextBuffer,
        /// 源码编辑模式（Ctrl+Shift+E 切换）。默认是渲染视图。
        source_mode: bool,
    },
    /// 设置页。`tab` 是分页（general / shortcuts / version-history / customization / ai / plugins），
    /// `section` 是自定义设置下的分区（appearance / sidebar / …）。
    Settings { tab: String, section: String },
    /// Agent 配置（`ai-prompts://workspace`）。`section` 是 agents / skills / tools / mcps / quick-actions。
    AgentConfig { section: String },
    /// 非 Markdown 文件的查看器（图片 / 代码 / 链接文件 / 不支持的类型）。对应 `FileViewer.tsx`。
    Viewer {
        path: PathBuf,
        content: crate::ui::viewer::Content,
    },
}

impl OpenTab {
    /// 标签背后的文件。查看器标签也算——侧栏高亮、标签去重、快速打开的「最近」都按它。
    pub fn path(&self) -> Option<&Path> {
        match &self.kind {
            TabKind::File { path, .. } | TabKind::Viewer { path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn buffer(&self) -> Option<&crate::ui::editor::TextBuffer> {
        match &self.kind {
            TabKind::File { buffer, .. } => Some(buffer),
            _ => None,
        }
    }

    pub fn buffer_mut(&mut self) -> Option<&mut crate::ui::editor::TextBuffer> {
        match &mut self.kind {
            TabKind::File { buffer, .. } => Some(buffer),
            _ => None,
        }
    }

    pub fn source_mode(&self) -> bool {
        matches!(
            &self.kind,
            TabKind::File {
                source_mode: true,
                ..
            }
        )
    }

    pub fn dirty(&self) -> bool {
        if let TabKind::Viewer {
            content: crate::ui::viewer::Content::Base(state),
            ..
        } = &self.kind
        {
            return state.dirty;
        }
        self.buffer().map(|b| b.dirty()).unwrap_or(false)
    }

    pub fn is_pdf(&self) -> bool {
        self.path()
            .and_then(|p| p.extension())
            .map(|e| e.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false)
    }
}

/// 侧栏文件树的范围：某一个库，或某个类型下的全部库（各自作为顶层文件夹）。
/// 对应 Electron 的 `selectedLibraryId` 为空时按类型显示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeScope {
    Library(usize),
    Type(String),
}

/// 文件树拖放的落点。`Before` / `After` 调整同级顺序，`Into` 把项目移入目录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeDropPosition {
    Before,
    After,
    Into,
}

pub struct WorkspaceState {
    pub root: PathBuf,
    pub name: String,
    /// 库类型。导航轨按它分组列出库实例——`Library::kind` 存的是类型 id，
    /// 要显示中文名就得留着这张表。
    pub library_types: Vec<LibraryType>,
    pub libraries: Vec<Library>,
    pub scope: TreeScope,
    pub tree: Vec<FileNode>,
    pub index: Arc<MetadataIndexService>,
    /// 版本历史。保存时登记变更，面板里手动/自动提交。
    pub git: Arc<GitService>,
    pub indexed_files: usize,
    /// 持有着即保持监听；随 WorkspaceState 一起 drop 时自动停。
    _watcher: Option<WatcherService>,
}

impl Drop for WorkspaceState {
    fn drop(&mut self) {
        self.git.stop_auto_commit();
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    pub fn new() -> Self {
        Self {
            favorites: None,
            favorites_error: String::new(),
            favorites_selected: false,
            show_favorite_parents: false,
            sort_mode: "manual".into(),
            manual_sort_orders: HashMap::new(),
            workspace: None,
            expanded: HashSet::new(),
            favorites_expanded: HashSet::new(),
            favorites_expanded_initialized: false,
            rows: Vec::new(),
            tree_loads: Default::default(),
            selected: None,
            status: "未打开工作区".into(),
            tabs: Vec::new(),
            active_tab: None,
            protected_view_path: None,
            document_format_changes: Vec::new(),
        }
    }

    pub fn workspace(&self) -> Option<&WorkspaceState> {
        self.workspace.as_ref()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// 取出最近一次保存产生的文档格式路径变更。
    ///
    /// App 在下一次同步时消费它，用于重映射分屏、图标、引用和编辑器附件等
    /// 仍然以旧路径标识的状态。取出后队列即清空，避免重复应用同一变更。
    pub fn take_document_format_changes(&mut self) -> Vec<(PathBuf, PathBuf)> {
        std::mem::take(&mut self.document_format_changes)
    }

    /// 外部变更到达后重算状态栏里的索引计数。
    pub fn refresh_status(&mut self) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        ws.indexed_files = ws.index.indexed_file_count();
        self.status = format!(
            "{} · {} 个库 · 索引 {} 个文件",
            ws.name,
            ws.libraries.len(),
            ws.indexed_files
        );
    }

    // ---------- 文件操作 ----------

    // ---------- 标签页 ----------
}

#[cfg(test)]
mod tests;

/// 桌面卡片也使用相同的浅层读取、子文档挂接和排序逻辑。
pub(crate) fn desktop_tree_children(
    root: &Path,
    path: &Path,
    sort_mode: &str,
) -> Result<Vec<FileNode>> {
    let mut nodes = tree_loading::read_children(root, path)?;
    tree_interactions::sort_tree(&mut nodes, path, sort_mode, &read_manual_sort_orders(root));
    Ok(nodes)
}
pub(crate) fn desktop_tree_rows(nodes: &[FileNode], expanded: &HashSet<PathBuf>) -> Vec<Row> {
    let mut rows = vec![];
    flatten(nodes, 0, expanded, &mut rows);
    rows
}
