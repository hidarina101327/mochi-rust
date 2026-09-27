//! 切换侧边栏当前显示的资料库、类型、收藏和文件标签。
use super::*;

impl Shell {
    pub fn select_library(&mut self, index: usize) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };
        if index >= ws.libraries.len() {
            return;
        }
        self.leave_favorites();
        self.set_scope(TreeScope::Library(index));
    }

    /// 按类型显示：该类型下的每个库各是一个顶层文件夹。
    pub fn select_type(&mut self, type_id: &str) {
        self.leave_favorites();
        self.set_scope(TreeScope::Type(type_id.to_owned()));
    }

    /// 切换到跨库的收藏文件树。收藏树只包含收藏文件及其必要的父目录，
    /// `WorkspaceState::scope` 继续保存上一次的知识库范围，便于稍后恢复。
    pub fn select_favorites(&mut self) {
        if self.workspace.is_none() {
            return;
        }

        self.favorites_selected = true;
        self.reload_favorites();

        if !self.favorites_expanded_initialized {
            let root = self.workspace.as_ref().map(|ws| ws.root.clone());
            let paths = self.favorite_paths();
            self.favorites_expanded = root
                .as_deref()
                .map(|root| favorite_ancestor_dirs(root, &paths))
                .unwrap_or_default();
            self.favorites_expanded_initialized = true;
            self.rebuild_favorite_tree();
        }

        // 收藏入口可以从设置等特殊标签触发；收藏视图右侧应显示文件标签或空态。
        self.show_file_tab();
    }

    /// 文件范围入口只激活文件标签；没有文件时显示空态，保留特殊标签供显式切回。
    pub fn show_file_tab(&mut self) {
        if self.active().is_none_or(|tab| tab.path().is_none()) {
            self.active_tab = self.tabs.iter().rposition(|tab| tab.path().is_some());
        }
    }

    /// 离开收藏树，恢复进入收藏前的知识库范围与树展开状态。
    pub fn leave_favorites(&mut self) {
        if !self.favorites_selected {
            return;
        }
        self.favorites_selected = false;
        let Some((root, libraries, scope)) = self
            .workspace
            .as_ref()
            .map(|ws| (ws.root.clone(), ws.libraries.clone(), ws.scope.clone()))
        else {
            self.selected = None;
            self.rows.clear();
            return;
        };
        let tree = build_tree(&root, &libraries, &scope);
        self.manual_sort_orders = read_manual_sort_orders(&root);
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

    /// 当前是否正在显示收藏文件树。
    pub fn favorites_selected(&self) -> bool {
        self.favorites_selected
    }

    /// 当前收藏侧栏是否显示收藏文件的父级目录树。
    pub fn show_favorite_parents(&self) -> bool {
        self.show_favorite_parents
    }

    /// 切换收藏侧栏的父级目录显示方式。
    ///
    /// 收藏树和普通文件树分别持有展开集合，因此刷新不会改变普通树的展开状态；
    /// `rebuild_favorite_tree` 也只重算行列表，不触碰打开的标签页。
    pub fn set_favorite_show_parents(&mut self, enabled: bool) {
        if self.show_favorite_parents == enabled {
            return;
        }
        self.show_favorite_parents = enabled;
        if self.favorites_selected {
            self.rebuild_favorite_tree();
        }
    }

    pub(super) fn set_scope(&mut self, scope: TreeScope) {
        let Some(ws) = self.workspace.as_mut() else {
            return;
        };
        self.tree_loads = Default::default();
        ws.scope = scope;
        ws.tree = build_tree(&ws.root, &ws.libraries, &ws.scope);
        self.expanded.clear();
        self.selected = None;
        self.rebuild_rows();
        self.show_file_tab();
    }

    /// 当前选中的库（按类型显示时为 `None`）。
    pub fn selected_library(&self) -> Option<usize> {
        if self.favorites_selected {
            return None;
        }
        match &self.workspace.as_ref()?.scope {
            TreeScope::Library(i) => Some(*i),
            TreeScope::Type(_) => None,
        }
    }

    /// 当前范围对应的类型 id：选中库时是库的类型，按类型显示时就是那个类型。
    pub fn scope_type_id(&self) -> Option<&str> {
        if self.favorites_selected {
            return None;
        }
        let ws = self.workspace.as_ref()?;
        match &ws.scope {
            TreeScope::Library(i) => ws.libraries.get(*i).map(|l| l.kind.as_str()),
            TreeScope::Type(t) => Some(t.as_str()),
        }
    }

    /// 新建库：建目录 + 登记 `libraries.json` + 选中它。返回新库的下标。
    pub fn create_library(&mut self, type_id: &str, name: &str) -> Result<usize> {
        let ws = self
            .workspace
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("未打开工作区"))?;
        let type_name = ws
            .library_types
            .iter()
            .find(|t| t.id == type_id)
            .map(|t| t.name.clone())
            .ok_or_else(|| anyhow::anyhow!("未知的库类型 {type_id}"))?;
        let service = WorkspaceService::new(&ws.root)?;
        let lib = service.create_library(type_id, &type_name, name, None)?;
        ws.libraries.push(lib);
        let index = ws.libraries.len() - 1;
        self.select_library(index);
        Ok(index)
    }
}
