//! 读取、保存和调整收藏路径，并处理收藏操作。
use super::*;
use mochi_core::favorites::Favorites;

impl Shell {
    pub fn reload_favorites(&mut self) {
        let previous_visible = if self.favorites_selected {
            self.favorite_paths()
                .into_iter()
                .filter(|path| path.is_file())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.favorites = None;
        self.favorites_error.clear();
        let Some(root) = self.workspace.as_ref().map(|ws| ws.root.clone()) else {
            return;
        };
        match Favorites::load(&root) {
            Ok(store) => self.favorites = Some(store),
            Err(error) => self.favorites_error = format!("无法读取收藏：{error}"),
        }
        if self.favorites_selected {
            if self.favorites_expanded_initialized {
                let newly_visible = self
                    .favorite_paths()
                    .into_iter()
                    .filter(|path| path.is_file() && !previous_visible.contains(path))
                    .collect::<Vec<_>>();
                let newly_expanded = self
                    .workspace
                    .as_ref()
                    .map(|ws| favorite_ancestor_dirs(&ws.root, &newly_visible))
                    .unwrap_or_default();
                self.favorites_expanded.extend(newly_expanded);
            }
            self.rebuild_favorite_tree();
        }
    }

    pub fn is_favorite(&self, path: &Path) -> bool {
        self.favorites
            .as_ref()
            .is_some_and(|store| store.contains(path))
    }

    pub fn favorite_paths(&self) -> Vec<PathBuf> {
        self.favorites
            .as_ref()
            .map(Favorites::paths)
            .unwrap_or_default()
    }

    pub fn favorites_error(&self) -> &str {
        &self.favorites_error
    }

    pub fn toggle_favorite(&mut self, path: &Path) -> Result<bool> {
        // 修改前重新读取数据，以保留其他窗口所做的更改。
        self.reload_favorites();
        let result = {
            let store = self
                .favorites
                .as_mut()
                .context(if self.favorites_error.is_empty() {
                    "请先打开工作区".to_owned()
                } else {
                    self.favorites_error.clone()
                })?;
            store.toggle(path)
        };
        if let Ok(added) = result.as_ref() {
            if self.favorites_selected {
                if *added {
                    if let Some(root) = self.workspace.as_ref().map(|ws| ws.root.clone()) {
                        // 存储层会返回规范化后的路径，并移除 Windows 的 verbatim
                        // 前缀，使路径与树视图使用的工作区根目录一致。
                        if let Some(candidate) = self.favorite_paths().last() {
                            self.favorites_expanded.extend(favorite_ancestor_dirs(
                                &root,
                                std::slice::from_ref(candidate),
                            ));
                        }
                    }
                }
                self.rebuild_favorite_tree();
            }
        }
        result
    }

    pub(super) fn remap_favorites(&mut self, from: &Path, to: &Path) -> Option<String> {
        self.reload_favorites();
        if !self.favorites_error.is_empty() {
            return Some(self.favorites_error.clone());
        }
        let old_children = PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &from.to_string_lossy(),
        ));
        let new_children = PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &to.to_string_lossy(),
        ));
        let error = self.favorites.as_mut().and_then(|store| {
            store
                .remap_path(from, to)
                .and_then(|_| store.remap_path(&old_children, &new_children))
                .err()
        });
        if error.is_none() && self.favorites_selected {
            self.rebuild_favorite_tree();
        }
        error.map(|error| format!("文件已移动，但收藏路径更新失败：{error}"))
    }

    pub(super) fn remove_favorites_under(&mut self, path: &Path) {
        self.reload_favorites();
        if !self.favorites_error.is_empty() {
            self.status = self.favorites_error.clone();
        } else if let Some(error) = self.favorites.as_mut().and_then(|store| {
            let children = PathBuf::from(mochi_core::sub_documents::sidecar_path(
                &path.to_string_lossy(),
            ));
            store
                .remove_under(path)
                .and_then(|_| store.remove_under(&children))
                .err()
        }) {
            self.status = format!("文件已删除，但收藏清理失败：{error}");
        }
        if self.favorites_selected {
            self.rebuild_favorite_tree();
        }
    }

    pub(super) fn rebuild_favorite_tree(&mut self) {
        self.tree_loads = Default::default();
        let Some(root) = self.workspace.as_ref().map(|ws| ws.root.clone()) else {
            self.rows.clear();
            self.selected = None;
            return;
        };
        let paths = self.favorite_paths();
        let tree = build_favorites_tree(&root, &paths, self.show_favorite_parents);
        if let Some(ws) = self.workspace.as_mut() {
            ws.tree = tree;
        }
        let selected_path = self
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.path.clone());
        self.rebuild_rows();
        self.selected =
            selected_path.and_then(|path| self.rows.iter().position(|row| row.path == path));
    }
}
