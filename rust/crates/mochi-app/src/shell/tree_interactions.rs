//! 处理文件树节点的选择、展开、折叠和延迟加载。
use super::*;

impl Shell {
    /// 重新从磁盘读文件树（watcher 回调用）。**保留展开状态与选中项**。
    pub fn refresh_tree(&mut self) {
        self.tree_loads = Default::default();
        if let Some(ws) = self.workspace.as_mut() {
            // 浏览器剪藏可能会在接收工作线程中创建资料库。
            if let Ok(Some(libraries)) = mochi_core::json2::read_from_file::<
                Vec<mochi_core::domain::Library>,
            >(&mochi_core::paths::libraries_file(&ws.root))
            {
                ws.libraries = libraries;
            }
        }
        if self.favorites_selected {
            // 收藏文件可被另一个窗口改动；刷新时同步存储后再建最小树。
            // `reload_favorites` 直接重建，不回调这里，避免递归。
            self.reload_favorites();
            return;
        }
        let Some((root, libraries, scope)) = self
            .workspace
            .as_ref()
            .map(|ws| (ws.root.clone(), ws.libraries.clone(), ws.scope.clone()))
        else {
            return;
        };
        let tree = build_tree(&root, &libraries, &scope);
        if let Some(ws) = self.workspace.as_mut() {
            ws.tree = tree;
        }
        // build_tree 只读取第一层目录。展开状态是按路径持久保存的，若这里仅恢复
        // expanded 集合而不重新按需读取已展开节点，就会出现箭头仍朝下、子项却消失
        // 的“假展开”状态。父级先于子级加载，仍然保持每层只做一次浅层 I/O。
        let mut expanded_paths = self.active_expanded().iter().cloned().collect::<Vec<_>>();
        expanded_paths.sort_by_key(|path| path.components().count());
        for path in expanded_paths {
            self.load_lazy_children(&path);
        }
        let selected_path = self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| r.path.clone());
        self.rebuild_rows();
        self.selected = selected_path.and_then(|p| self.rows.iter().position(|r| r.path == p));
    }

    /// 展开/折叠第 `row` 行的目录或带子文档的文档。
    pub fn toggle(&mut self, row: usize) {
        let Some(entry) = self.rows.get(row) else {
            return;
        };
        if !entry.has_children {
            return;
        }
        self.tree_loads.expand_all = false;
        let path = entry.path.clone();
        if entry.is_dir && !entry.expanded {
            // 箭头反馈要即时；命中缓存的展开不做磁盘 I/O。
            self.request_tree_children(path.clone());
        }
        {
            let expanded = self.active_expanded_mut();
            if !expanded.remove(&path) {
                expanded.insert(path);
            }
        }
        let selected_path = self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| r.path.clone());
        self.rebuild_visible_rows();
        self.selected = selected_path.and_then(|p| self.rows.iter().position(|r| r.path == p));
    }

    pub(super) fn load_lazy_children(&mut self, path: &Path) {
        // 先查缓存再读盘（refresh 会恢复已展开的路径）。
        if !self.tree_node_is_lazy(path) {
            return;
        }
        let Some(root) = self.workspace.as_ref().map(|ws| ws.root.clone()) else {
            return;
        };
        if let Ok(children) = tree_loading::read_children(&root, path) {
            self.cache_tree_children(path, children);
        }
    }

    pub fn select(&mut self, row: usize) {
        if row < self.rows.len() {
            self.selected = Some(row);
        }
    }

    /// 全部展开：把树里每个目录和带子文档的文档都加进展开集合。
    pub fn expand_all(&mut self) {
        let paths = self
            .workspace
            .as_ref()
            .map(|ws| tree_jobs::expandable_paths(&ws.tree))
            .unwrap_or_default();
        self.tree_loads.expand_all = true;
        self.expand_tree_paths(paths);
        self.rebuild_visible_rows();
    }

    pub fn collapse_all(&mut self) {
        self.tree_loads.expand_all = false;
        self.tree_loads.queued.clear();
        self.active_expanded_mut().clear();
        let selected_path = self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| r.path.clone());
        self.rebuild_visible_rows();
        self.selected = selected_path.and_then(|p| self.rows.iter().position(|r| r.path == p));
    }

    /// 是否所有可展开项都展开着（决定头部画「全部折叠」还是「全部展开」）。
    pub fn all_expanded(&self) -> bool {
        !self.rows.is_empty()
            && self
                .rows
                .iter()
                .filter(|r| r.has_children)
                .all(|r| r.expanded)
    }

    /// 让某个目录展开（新建其子项后要能看见它）。
    pub fn expand(&mut self, dir: &Path) {
        // 显式展开要先加载子项，才能定位到对应行。
        self.load_lazy_children(dir);
        self.active_expanded_mut().insert(dir.to_path_buf());
        self.rebuild_rows();
    }

    pub(super) fn active_expanded(&self) -> &HashSet<PathBuf> {
        if self.favorites_selected {
            &self.favorites_expanded
        } else {
            &self.expanded
        }
    }

    pub(super) fn active_expanded_mut(&mut self) -> &mut HashSet<PathBuf> {
        if self.favorites_selected {
            &mut self.favorites_expanded
        } else {
            &mut self.expanded
        }
    }

    /// 当前文件树的根目录（新建到「空白处」时的父目录）。
    pub fn tree_root(&self) -> Option<PathBuf> {
        let ws = self.workspace.as_ref()?;
        if self.favorites_selected {
            return Some(ws.root.clone());
        }
        Some(match &ws.scope {
            TreeScope::Library(i) => library_root(&ws.root, &ws.libraries, *i),
            TreeScope::Type(_) => ws.root.clone(),
        })
    }

    /// 返回包含此实际知识库路径的库根。映射目录的来源在知识库之外，因而不会
    /// 被误当成新映射的挂载位置。
    pub fn library_root_for_path(&self, path: &Path) -> Option<PathBuf> {
        let workspace = self.workspace.as_ref()?;
        workspace
            .libraries
            .iter()
            .map(|library| PathBuf::from(&library.path))
            .filter(|root| native_path_is_same_or_under(path, root))
            .max_by_key(|root| root.as_os_str().len())
    }

    /// 把树按当前展开状态压成一维行列表。
    pub fn manual_sort_enabled(&self) -> bool {
        self.sort_mode == "manual"
    }

    pub fn set_sort_mode(&mut self, mode: &str) {
        self.sort_mode = match mode {
            "name" | "name-desc" => mode,
            _ => "manual",
        }
        .into();
        self.selected = None;
        self.rebuild_rows();
    }

    pub fn apply_git_settings(&self) {
        if let Some(git) = self.git() {
            configure_git(&git);
            git.start_auto_commit();
        }
    }

    pub(super) fn rebuild_rows(&mut self) {
        if let Some(ws) = &mut self.workspace {
            let root = match &ws.scope {
                TreeScope::Library(index) => library_root(&ws.root, &ws.libraries, *index),
                TreeScope::Type(_) => ws
                    .tree
                    .first()
                    .and_then(|node| Path::new(&node.path).parent())
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| ws.root.clone()),
            };
            sort_tree(
                &mut ws.tree,
                &root,
                &self.sort_mode,
                &self.manual_sort_orders,
            );
        }
        self.rebuild_visible_rows();
    }

    // 展开/折叠只影响可见性，缓存的树本身已排好序。
    fn rebuild_visible_rows(&mut self) {
        let mut rows = Vec::new();
        if let Some(ws) = &self.workspace {
            flatten(&ws.tree, 0, self.active_expanded(), &mut rows);
        }
        self.rows = rows;
    }
}

pub(super) fn sort_tree(
    nodes: &mut [FileNode],
    parent: &Path,
    mode: &str,
    orders: &HashMap<String, Vec<String>>,
) {
    if mode == "manual" {
        if let Some(order) = orders.get(&path_key(parent)) {
            let positions = order
                .iter()
                .enumerate()
                .map(|(i, path)| (path.as_str(), i))
                .collect::<HashMap<_, _>>();
            nodes.sort_by_key(|node| {
                positions
                    .get(path_key(Path::new(&node.path)).as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
        }
        for node in nodes {
            if let Some(children) = &mut node.children {
                sort_tree(children, Path::new(&node.path), mode, orders);
            }
        }
        return;
    }
    nodes.sort_by(|a, b| {
        b.is_directory().cmp(&a.is_directory()).then_with(|| {
            let names = mochi_core::files::compare_names(&a.name, &b.name);
            if a.is_directory() || b.is_directory() || mode == "name" {
                names
            } else {
                names.reverse()
            }
        })
    });
    for node in nodes {
        if let Some(children) = &mut node.children {
            sort_tree(children, Path::new(&node.path), mode, orders);
        }
    }
}
