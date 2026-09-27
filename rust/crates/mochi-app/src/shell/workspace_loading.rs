//! 打开工作区、配置 Git，并启动文件变更监视。
use super::*;

impl Shell {
    /// 打开工作区：种子/迁移/库配置/Git → 打开既有索引 → 建文件树 → 起文件监听。
    ///
    /// 全量索引由 App 在工作区可用后投到后台；这里绝不能扫描整个工作区，
    /// 否则首次打开大型知识库会卡住主窗口。
    ///
    /// `on_change` 在**监听线程**上被调用（不是 UI 线程），实现方只应做一件事：
    /// 把消息投递给 UI 线程。索引的增量更新在这里就地做掉了，UI 只需要重画。
    pub fn open_workspace(
        &mut self,
        root: impl AsRef<Path>,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> Result<()> {
        self.save_dirty_tabs().context("切换工作区前保存失败")?;
        // 路径变更属于旧工作区；切换成功后不得把它交给新工作区的 App 状态。
        self.document_format_changes.clear();
        let service = WorkspaceService::new(root)?;
        let info = service.open()?;

        let index = Arc::new(MetadataIndexService::new(&info.root_path));
        index.open()?;
        let indexed_files = index.indexed_file_count();

        let scope = TreeScope::Library(0);
        let tree = build_tree(&info.root_path, &info.libraries, &scope);
        self.manual_sort_orders = read_manual_sort_orders(&info.root_path);

        // 仓库由 WorkspaceService::open 初始化（gitdir = .mochi/git）；这里只是拿个句柄。
        // 自动提交按设置里的间隔在后台跑，与 Electron 版一致
        let git = Arc::new(GitService::new(&info.root_path));
        let _ = git.ensure_repository();
        configure_git(&git);
        git.start_auto_commit();

        let watcher = start_watcher(&info.root_path, Arc::clone(&index), on_change);

        self.status = format!(
            "{} · {} 个库 · 已有索引 {} 个文件",
            info.name,
            info.libraries.len(),
            indexed_files
        );
        self.tree_loads = Default::default();
        self.workspace = Some(WorkspaceState {
            root: info.root_path,
            name: info.name,
            library_types: info.library_types,
            libraries: info.libraries,
            scope,
            tree,
            index,
            git,
            indexed_files,
            _watcher: watcher,
        });
        self.favorites_selected = false;
        self.favorites_expanded.clear();
        self.favorites_expanded_initialized = false;
        self.expanded.clear();
        self.selected = None;
        // 换工作区就清空标签：留着上一个工作区的文件会让「当前文档」指向别处
        self.tabs.clear();
        self.active_tab = None;
        self.protected_view_path = None;
        self.reload_favorites();
        self.rebuild_rows();
        Ok(())
    }
}

pub(super) fn configure_git(git: &GitService) {
    use crate::ui::settings_values::{boolean, number, text};
    git.set_settings(mochi_core::git::GitSettings {
        version_history_enabled: boolean("git.versionHistoryEnabled", true),
        auto_commit_enabled: boolean("git.autoCommitEnabled", true),
        auto_commit_interval_minutes: number("git.autoCommitInterval", 10.0).max(1.0) as u64,
    });
    git.set_author(&text("git.authorName", ""), &text("git.authorEmail", ""));
}

pub(super) fn flatten(
    nodes: &[FileNode],
    depth: usize,
    expanded: &HashSet<PathBuf>,
    out: &mut Vec<Row>,
) {
    for node in nodes {
        let path = PathBuf::from(&node.path);
        let is_dir = node.is_directory();
        let children = node.children.as_deref().unwrap_or_default();
        let is_expandable = !children.is_empty() || node.lazy;
        let is_expanded = is_expandable && expanded.contains(&path);

        out.push(Row {
            name: node.name.clone(),
            path,
            depth,
            is_dir,
            is_mapped_folder: node.kind == "mapped-directory",
            expanded: is_expanded,
            has_children: is_expandable,
        });

        if is_expanded {
            flatten(children, depth + 1, expanded, out);
        }
    }
}

/// 起文件监听。每批合并后的事件先就地更新索引（增量，便宜），再通知 UI 重画。
///
/// 监听失败不致命——工作区照样能用，只是外部改动不会自动反映，用户手动刷新即可。
fn start_watcher(
    root: &Path,
    index: Arc<MetadataIndexService>,
    on_change: impl Fn() + Send + Sync + 'static,
) -> Option<WatcherService> {
    let watcher = WatcherService::new(root);
    watcher.on_events(move |events| {
        // watcher 已经合并过事件：每批只解析一次链接，
        // 而不是每个文件解析一次（批量导入时那样会平方级放大）。
        let _ = index.index_files(
            events
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        WatchChangeType::Unlink | WatchChangeType::Add | WatchChangeType::Change
                    )
                })
                .map(|e| &e.path),
        );
        on_change();
    });
    watcher.start().ok().map(|()| watcher)
}
