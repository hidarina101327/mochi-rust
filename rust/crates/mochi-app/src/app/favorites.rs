//! 维护收藏列表，并处理收藏入口和知识库导航。
use super::*;

impl App {
    pub(super) fn favorite_menu_item(&self, path: &Path) -> MenuItem<MenuAction> {
        MenuItem::new(
            if self.shell.is_favorite(path) {
                "取消收藏"
            } else {
                "收藏文档"
            },
            MenuAction::ToggleFavorite(path.to_path_buf()),
        )
        .icon(Icon::STAR)
    }

    pub(super) fn toggle_favorite(&mut self, path: &Path) {
        let result = self.shell.toggle_favorite(path);
        self.reload_favorites();
        self.reload_home_dashboard();
        let message = match result {
            Ok(true) => "已收藏，可在左侧「收藏」查看".to_owned(),
            Ok(false) => "已取消收藏".to_owned(),
            Err(error) => format!("收藏操作失败：{error}"),
        };
        if message.starts_with("收藏操作失败") {
            self.views.favorites.error = message.clone();
        }
        self.state.status_text = message;
    }

    pub(super) fn reload_favorites(&mut self) {
        self.shell.reload_favorites();
        self.views.favorites.docs.clear();
        self.views.favorites.error = self.shell.favorites_error().to_owned();
        if let Some(ws) = self.shell.workspace() {
            let libraries = ws
                .libraries
                .iter()
                .map(|l| (l.name.clone(), PathBuf::from(&l.path)))
                .collect::<Vec<_>>();
            self.views.favorites.docs =
                views::favorites::collect(&self.shell.favorite_paths(), &ws.root, &libraries);
        }
        self.views.favorites.loaded = true;
    }

    pub(super) fn open_favorites(&mut self) {
        self.finish_sidebar_edit();
        self.shell.select_favorites();
        self.reset_file_sidebar();
        self.reload_favorites();
        self.sync_state();
    }

    pub(super) fn open_knowledge(&mut self) {
        self.finish_sidebar_edit();
        self.shell.leave_favorites();
        self.shell.show_file_tab();
        self.reset_file_sidebar();
        self.sync_state();
    }

    fn reset_file_sidebar(&mut self) {
        self.side = SidebarState::default();
        self.outline_left_active = false;
        self.state.sidebar_visible = true;
        self.state.view = WorkspaceView::Editor;
        self.focus = Focus::Main;
        self.invalidate_main();
    }

    pub(super) fn on_document_list_context(&mut self, x: f32, y: f32) {
        let path = match self.views.recent_layout.hit(x, y) {
            Some(views::recent::Hit::Doc(i)) => self
                .views
                .recent
                .filtered()
                .get(i)
                .map(|doc| doc.path.clone()),
            _ => None,
        };
        if let Some(path) = path {
            self.menu = Some(Menu::open_at(
                vec![self.favorite_menu_item(&path)],
                x,
                y,
                self.renderer.viewport(),
            ));
        }
    }

    pub(super) fn open_dashboard_document(&mut self, path: &Path) {
        if self.shell.favorites_selected() {
            self.side = SidebarState::default();
            self.outline_left_active = false;
        }
        self.shell.leave_favorites();
        // 按路径匹配，而不是按库标题：不同类型的库可能同名。
        let library = self.shell.workspace().and_then(|ws| {
            ws.libraries
                .iter()
                .enumerate()
                .filter(|(_, lib)| path.starts_with(&lib.path))
                .max_by_key(|(_, lib)| lib.path.len())
                .map(|(index, _)| index)
        });
        if let Some(index) = library {
            self.shell.select_library(index);
        }
        if self.open_file_from_ui(path) {
            self.state.view = WorkspaceView::Editor;
            self.focus = Focus::Main;
            self.invalidate_main();
            self.sync_state();
        } else {
            self.state.status_text = self.shell.status().to_owned();
            self.reload_favorites();
        }
    }
}
