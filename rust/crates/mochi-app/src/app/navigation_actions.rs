//! 处理页面导航、返回操作和新建资料库流程。
use super::*;

impl App {
    pub(super) fn open_auxiliary_page(&mut self, view: WorkspaceView) {
        if self.state.view != view {
            if !self.state.view.is_auxiliary_page() {
                self.page_history.clear();
            }
            self.page_history.push(self.state.view);
            self.state.view = view;
        }
        self.focus = Focus::Main;
        self.invalidate_main();
    }

    pub(super) fn return_from_auxiliary_page(&mut self) {
        if !self.state.view.is_auxiliary_page() {
            return;
        }
        self.state.view = self.page_history.pop().unwrap_or(WorkspaceView::Home);
        self.focus = Focus::Main;
        self.invalidate_main();
    }

    /// 导航轨的点击。每个分支对应 TSX 里一个 `onClick`。
    pub(super) fn on_navigation_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.nav_layout.hit(x, y) else {
            return;
        };
        self.focus = Focus::Main;
        match hit {
            NavHit::Collapse => {
                self.state.navigation_collapsed = !self.state.navigation_collapsed;
                self.persist_chrome_setting(
                    "navigation.collapsed",
                    SettingValue::Bool(self.state.navigation_collapsed),
                );
                self.invalidate_main();
            }
            NavHit::Search => self.open_search(),
            NavHit::Settings => self.open_settings("general"),
            NavHit::PluginEntry(index) => {
                let plugin_id = self
                    .shell
                    .workspace()
                    .and_then(|ws| {
                        mochi_core::plugins::PluginService::new(&ws.root)
                            .registered_slots()
                            .ok()
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|slot| {
                        slot.contribution.slot == mochi_core::plugins::PluginSlot::Navigation
                    })
                    .nth(index)
                    .map(|slot| slot.plugin.manifest.id);
                if let Some(plugin_id) = plugin_id {
                    self.plugin_workspace = Some(plugin_id);
                    self.state.view = WorkspaceView::Plugin;
                    self.invalidate_main();
                }
            }
            NavHit::Item(item) => match item {
                NavItem::Home => {
                    self.state.view = WorkspaceView::Home;
                    self.reload_home_dashboard();
                }
                NavItem::Automations => self.workflows_open(),
                NavItem::Inbox => self.state.view = WorkspaceView::Inbox,
                NavItem::Knowledge => self.open_knowledge(),
                NavItem::Favorites => self.open_favorites(),
                NavItem::Schedule => self.state.view = WorkspaceView::Schedule,
                NavItem::MochiAi => self.state.view = WorkspaceView::MochiAi,
                NavItem::QuickNote => {
                    if self.ensure_quick_note_tab() {
                        self.state.view = WorkspaceView::QuickNote;
                        self.editor_engaged = false;
                        self.invalidate_main();
                    }
                }
                NavItem::Recent => self.state.view = WorkspaceView::Recent,
                NavItem::Templates => self.open_auxiliary_page(WorkspaceView::Templates),
                NavItem::AgentConfig => {
                    self.agent.section = 0;
                    self.state.view = WorkspaceView::AgentConfig;
                    self.invalidate_main();
                }
            },
            NavHit::TypeHeader(ti) => {
                let Some(type_id) = self.type_id_at(ti) else {
                    return;
                };
                let was_editor = self.state.view == WorkspaceView::Editor;
                self.state.view = WorkspaceView::Editor;
                // 类型标题同时是该类型文件区的入口；再次点击才折叠列表。
                let selected_is_this_type = self.shell.selected_library().is_some()
                    && self.shell.scope_type_id() == Some(type_id.as_str());
                let was_this_type =
                    was_editor && self.shell.scope_type_id() == Some(type_id.as_str());
                self.shell.select_type(&type_id);
                if selected_is_this_type {
                    self.nav.expanded_types.insert(type_id);
                    self.invalidate_main();
                } else if !was_this_type {
                    self.nav.expanded_types.insert(type_id);
                } else if !self.nav.expanded_types.remove(&type_id) {
                    self.nav.expanded_types.insert(type_id);
                }
            }
            NavHit::TypeIcon(ti) => {
                // 收起状态下点库类型图标：直接跳到该类型的文件树（不展开导航栏）
                let Some(type_id) = self.type_id_at(ti) else {
                    return;
                };
                if self.shell.favorites_selected() {
                    self.side = SidebarState::default();
                }
                self.state.view = WorkspaceView::Editor;
                self.shell.select_type(&type_id);
                self.invalidate_main();
            }
            NavHit::TypeAdd(ti) => {
                let Some(type_id) = self.type_id_at(ti) else {
                    return;
                };
                self.nav.expanded_types.insert(type_id.clone());
                self.nav.creating = Some(CreatingLibrary {
                    type_id,
                    field: TextField::new("库名称"),
                    error: String::new(),
                });
                self.focus = Focus::NavNewLibrary;
            }
            NavHit::Library(li) => {
                if self.begin_navigation_library_drag(li, x, y) {
                    return;
                }
                let Some(li) = self.nav.painted_libraries.get(li).and_then(|id| {
                    self.shell
                        .workspace()?
                        .libraries
                        .iter()
                        .position(|library| &library.id == id)
                }) else {
                    return;
                };
                if self.shell.favorites_selected() {
                    self.side = SidebarState::default();
                }
                self.shell.select_library(li);
                self.state.view = WorkspaceView::Editor;
                self.invalidate_main();
            }
            NavHit::NewLibraryField => {
                self.focus = Focus::NavNewLibrary;
                if let (Some(c), Some(r)) = (
                    self.nav.creating.as_mut(),
                    self.nav_layout.rect_of(NavHit::NewLibraryField),
                ) {
                    // 输入框文字起点：见 navigation::paint 的 46 + 内边距 4
                    c.field.click(x - (r.left + 46.0 + 4.0), false);
                }
            }
        }
    }

    pub(super) fn type_id_at(&self, index: usize) -> Option<String> {
        self.shell
            .workspace()?
            .library_types
            .get(index)
            .map(|t| t.id.clone())
    }

    /// 校验并提交新建库。照抄 TSX 的 `validateLibraryName` + `confirmNewLibrary`。
    /// 返回 `true` 表示输入框应关闭（成功或取消），`false` 表示留着显示错误。
    pub(super) fn confirm_new_library(&mut self) -> bool {
        let Some(c) = self.nav.creating.as_mut() else {
            return true;
        };
        let name = c.field.text().trim().to_owned();
        let error = if name.is_empty() {
            Some("名称不能为空")
        } else if name
            .chars()
            .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        {
            Some("名称包含非法字符")
        } else if self
            .shell
            .workspace()
            .map(|ws| {
                ws.libraries
                    .iter()
                    .any(|l| l.kind == c.type_id && l.name == name)
            })
            .unwrap_or(false)
        {
            Some("该名称已存在")
        } else {
            None
        };
        if let Some(e) = error {
            c.error = e.to_owned();
            return false;
        }
        let type_id = c.type_id.clone();
        match self.shell.create_library(&type_id, &name) {
            Ok(_) => {
                self.nav.creating = None;
                self.focus = Focus::Main;
                self.state.view = WorkspaceView::Editor;
                self.invalidate_main();
                true
            }
            Err(_) => {
                if let Some(c) = self.nav.creating.as_mut() {
                    c.error = "创建失败".to_owned();
                }
                false
            }
        }
    }

    /// 失焦：有内容就确认，否则取消（TSX 的 `handleNewLibraryBlur`）。
    pub(super) fn finish_new_library(&mut self) {
        let has_text = self
            .nav
            .creating
            .as_ref()
            .map(|c| !c.field.text().trim().is_empty())
            .unwrap_or(false);
        if !has_text || !self.confirm_new_library() {
            // 校验失败时 TSX 会留着输入框显示错误；但失焦语义是离开，这里直接收起
            self.nav.creating = None;
            self.focus = Focus::Main;
        }
    }
}
