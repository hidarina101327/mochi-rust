//! 管理标签页导航、当前文件路由和导航位置恢复。
use super::*;

impl App {
    pub(super) fn paint_navigation(&mut self, chrome: &Chrome, p: &Palette) {
        let area = chrome.tree.rect(chrome.navigation);
        let (types, libs) = self.nav_types();
        let active = self.nav_active();
        let selected = self.shell.selected_library();
        let plugin_entries = self
            .shell
            .workspace()
            .and_then(|ws| {
                mochi_core::plugins::PluginService::new(&ws.root)
                    .registered_slots()
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|slot| slot.contribution.slot == mochi_core::plugins::PluginSlot::Navigation)
            .map(|slot| (slot.plugin.manifest.id, slot.contribution.title))
            .collect::<Vec<_>>();
        let active_plugin = self.plugin_workspace.as_deref().and_then(|id| {
            plugin_entries
                .iter()
                .position(|(plugin_id, _)| plugin_id == id)
        });
        let creating = self
            .nav
            .creating
            .as_ref()
            .map(|c| (c.type_id.as_str(), &c.field, c.error.as_str()));
        let model = NavModel {
            types: &types,
            libraries: &libs,
            expanded_types: &self.nav.expanded_types,
            creating,
            inbox_count: self.nav.inbox_count,
            collapsed: self.state.navigation_collapsed,
            active_item: active,
            plugin_entries: &plugin_entries,
            active_plugin,
            selected_library: selected,
        };
        let layout = navigation::layout(&model, area, self.nav.scroll);
        // 内容变短后滚动可能越界，绘制前夹一次
        let max = layout.max_scroll();
        let layout = if self.nav.scroll > max {
            self.nav.scroll = max;
            navigation::layout(&model, area, max)
        } else {
            layout
        };
        navigation::paint(&mut self.list, area, &model, &layout, self.nav.scroll, p);
        self.nav_layout = layout;
        self.nav.painted_libraries = self
            .shell
            .workspace()
            .map(|ws| {
                ws.libraries
                    .iter()
                    .map(|library| library.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.paint_navigation_library_drag(p);
    }

    /// 侧栏标题：当前库名，或按类型显示时的类型名。
    pub(super) fn sidebar_title(&self) -> String {
        if self.shell.favorites_selected() {
            return "收藏".to_owned();
        }
        let Some(ws) = self.shell.workspace() else {
            return String::new();
        };
        match self.shell.selected_library() {
            Some(i) => ws
                .libraries
                .get(i)
                .map(|l| l.name.clone())
                .unwrap_or_default(),
            None => {
                let type_id = self.shell.scope_type_id().unwrap_or_default();
                ws.library_types
                    .iter()
                    .find(|t| t.id == type_id)
                    .map(|t| t.name.clone())
                    .unwrap_or_default()
            }
        }
    }

    pub(super) fn active_file_path(&self) -> Option<PathBuf> {
        self.shell
            .active()
            .and_then(|t| t.path())
            .map(|p| p.to_path_buf())
    }

    pub(super) fn open_file_from_ui(&mut self, path: &Path) -> bool {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
        {
            platform::open_external(&path.to_string_lossy());
            return false;
        }
        self.capture_tab_navigation_position();
        let force_append =
            unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) < 0 };
        self.shell.set_protected_view_path(self.split.other.clone());
        let opened = self.shell.open_file_with_mode(path, force_append);
        if opened {
            self.remember_split_active();
        }
        opened
    }

    pub(super) fn tab_navigation_enabled(&self, direction: NavigationDirection) -> bool {
        !self.state.view.is_standalone() && self.shell.can_navigate(direction)
    }

    pub(super) fn capture_tab_navigation_position(&mut self) {
        if self.content() == MainContent::Document {
            if let Some(path) = self.active_file_path() {
                self.shell
                    .set_navigation_chunk(self.doc.navigation_chunk_for(&path));
            }
        }
    }

    pub(super) fn navigate_tab(&mut self, direction: NavigationDirection) -> bool {
        if self.dialog.is_some()
            || self.settings_overlay.is_some()
            || self.menu.is_some()
            || self.search.is_some()
            || self.command.is_some()
            || self.image_preview.is_some()
            || self.base_detail_open()
            || self.table_picker.is_some()
            || self.link_create.is_some()
            || self.mapped_folder.is_some()
            || !self.tab_navigation_enabled(direction)
        {
            return false;
        }
        if !self.commit_title() || !self.commit_table_cell() {
            return true;
        }
        self.capture_tab_navigation_position();
        self.shell.set_protected_view_path(self.split.other.clone());
        if self.shell.navigate(direction) {
            self.state.view = WorkspaceView::Editor;
            self.focus = Focus::Main;
            self.find = None;
            self.remember_split_active();
            self.invalidate_main();
            self.sync_state();
            if let (Some(path), Some(chunk)) =
                (self.active_file_path(), self.shell.navigation_chunk())
            {
                self.doc.restore_navigation_chunk(&path, chunk);
            }
        } else {
            self.show_global_notice(&self.shell.status().to_owned());
        }
        true
    }

    pub(super) fn open_link_file_from_ui(&mut self, path: &Path) -> bool {
        self.capture_tab_navigation_position();
        let new_tab = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) < 0 };
        if new_tab {
            return self.open_file_from_ui(path);
        }
        if !self.commit_title() || !self.commit_table_cell() {
            return false;
        }
        self.shell.set_protected_view_path(self.split.other.clone());
        let opened = self.shell.navigate_file(path);
        if opened {
            self.remember_split_active();
        } else {
            self.show_global_notice(&self.shell.status().to_owned());
        }
        opened
    }

    /// 当前是不是设置标签。是的话返回 (分页, 分区)。
    pub(super) fn settings_tab(&self) -> Option<(String, String)> {
        if let Some(settings) = &self.settings_overlay {
            return Some(settings.clone());
        }
        if self.state.view != WorkspaceView::Editor {
            return None;
        }
        match self.shell.active().map(|t| &t.kind) {
            Some(TabKind::Settings { tab, section }) => Some((tab.clone(), section.clone())),
            _ => None,
        }
    }

    pub(super) fn viewer_tab(&self) -> Option<(&Path, &viewer::Content)> {
        match self.shell.active().map(|t| &t.kind) {
            Some(TabKind::Viewer { path, content }) => Some((path.as_path(), content)),
            _ => None,
        }
    }

    pub(super) fn viewer_content_mut(&mut self) -> Option<&mut viewer::Content> {
        match self.shell.active_mut().map(|t| &mut t.kind) {
            Some(TabKind::Viewer { content, .. }) => Some(content),
            _ => None,
        }
    }
}
