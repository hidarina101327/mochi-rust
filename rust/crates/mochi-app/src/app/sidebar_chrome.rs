//! 绘制侧边栏、标签栏和窗口标题栏按钮。
use super::*;

impl App {
    /// 侧栏内容：头部 + 文件树。面板已经由 chrome 铺好底色。
    /// 当前标签是设置页时，左面板换成设置导航（TSX 的 `leftPanelContent === 'settings'`）。
    pub(super) fn paint_sidebar(&mut self, chrome: &Chrome, p: &Palette) {
        self.outline_tabs = None;
        if chrome.tree.is_hidden(chrome.sidebar) {
            self.side.layout = SidebarLayout::default();
            self.prefs.nav_layout = SettingsNavLayout::default();
            return;
        }
        // 拖动手柄占右侧 4px，内容区让开
        let area = chrome.tree.rect(chrome.sidebar);
        let mut area = Rect::new(
            area.left,
            area.top,
            area.right - crate::ui::chrome::configured_layout().resize_handle_width,
            area.bottom,
        );
        if self.content() == MainContent::Document && self.outline_mode() == "left-sidebar" {
            let mid = (area.left + area.right) / 2.0;
            let files = Rect::new(area.left, area.top, mid, area.top + 32.0);
            let heads = Rect::new(mid, area.top, area.right, area.top + 32.0);
            self.list.text_aligned(
                files,
                "文件",
                TextStyle::Caption,
                if !self.outline_left_active {
                    p.accent
                } else {
                    p.muted
                },
                Align::Center,
            );
            self.list.text_aligned(
                heads,
                "大纲",
                TextStyle::Caption,
                if self.outline_left_active {
                    p.accent
                } else {
                    p.muted
                },
                Align::Center,
            );
            self.outline_tabs = Some((files, heads));
            area.top += 32.0;
            if self.outline_left_active {
                self.side.layout = SidebarLayout::default();
                self.outline_area = area;
                outline::paint(
                    &mut self.list,
                    area,
                    self.doc.headings(),
                    self.doc.active_heading(self.shell.active_scroll()),
                    self.outline_scroll,
                    p,
                );
                return;
            }
        }
        if self.state.view == WorkspaceView::MochiAi {
            self.side.layout = SidebarLayout::default();
            let mut lay = ai_workspace::layout(&self.ai.workspace, &self.ai.panel.sessions, area);
            if self.ai.workspace.scroll > lay.max_scroll() {
                self.ai.workspace.scroll = lay.max_scroll();
                lay = ai_workspace::layout(&self.ai.workspace, &self.ai.panel.sessions, area);
            }
            ai_workspace::paint(
                &mut self.list,
                area,
                &mut self.ai.workspace,
                &lay,
                self.ai.panel.active.as_ref().map(|c| c.id.as_str()),
                self.focus == Focus::AiSessionQuery,
                self.ai.panel.is_streaming(),
                p,
            );
            self.ai.workspace_side = lay;
            return;
        }
        // 日程视图：左面板是日历侧栏（`leftPanelContent === 'calendar'`）
        if self.state.view == WorkspaceView::Schedule {
            self.side.layout = SidebarLayout::default();
            self.prefs.nav_layout = SettingsNavLayout::default();
            self.paint_schedule_sidebar(area, p);
            return;
        }
        self.sched.side_layout = Default::default();
        if self.settings_overlay.is_none() {
            if let Some((tab, section)) = self.settings_tab() {
                self.side.layout = SidebarLayout::default();
                let lay = settings::nav_layout(area, &tab, self.prefs.nav_scroll);
                settings::paint_nav(
                    &mut self.list,
                    area,
                    &lay,
                    &tab,
                    &section,
                    self.block_pointer,
                    p,
                );
                self.prefs.nav_scroll = lay.scroll;
                self.prefs.nav_layout = lay;
                return;
            }
        }
        self.prefs.nav_layout = SettingsNavLayout::default();
        // Agent 配置：左面板是「AI 定义」导航（`AIPromptsNavigation`）
        if let Some(section) = self.agent_config_section() {
            self.side.layout = SidebarLayout::default();
            self.ensure_agent_config_loaded();
            let mut lay = agent_config::nav_layout(area, &self.agent.data, self.agent.nav_scroll);
            if self.agent.nav_scroll > lay.max_scroll() {
                self.agent.nav_scroll = lay.max_scroll();
                lay = agent_config::nav_layout(area, &self.agent.data, self.agent.nav_scroll);
            }
            agent_config::paint_nav(
                &mut self.list,
                area,
                &lay,
                &self.agent.data,
                section,
                self.agent.nav_hover,
                p,
            );
            self.agent.nav_layout = lay;
            return;
        }
        self.agent.nav_layout = agent_config::NavLayout::default();
        let title = self.sidebar_title();
        let active = self.active_file_path();
        let drop_indicator = self.side.tree_drag.as_ref().and_then(|drag| {
            let drop = drag.drop.as_ref().filter(|_| drag.active)?;
            let row = self
                .shell
                .rows()
                .iter()
                .position(|row| row.path == drop.path)?;
            Some(match drop.kind {
                SidebarTreeDropKind::Before => TreeDropIndicator::Before(row),
                SidebarTreeDropKind::After => TreeDropIndicator::After(row),
                SidebarTreeDropKind::Into => TreeDropIndicator::Into(row),
            })
        });
        let model = SidebarModel {
            icons: Some(&self.file_icons),
            title: &title,
            rows: self.shell.rows(),
            loading_paths: self.shell.tree_loading_paths().collect(),
            active_path: active.as_deref(),
            selected_row: self.shell.selected(),
            hover_row: self.side.hover,
            search: &self.side.search,
            search_focused: self.focus == Focus::SidebarSearch,
            all_expanded: self.shell.all_expanded(),
            editing: self.side.editing.as_ref(),
            drop_indicator,
        };
        let plugin_sidebar_slots = self
            .shell
            .workspace()
            .and_then(|ws| {
                mochi_core::plugins::PluginService::new(&ws.root)
                    .registered_slots()
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|slot| slot.contribution.slot == mochi_core::plugins::PluginSlot::Sidebar)
            .collect::<Vec<_>>();
        // 插件侧栏是宿主保留的一段受控空间，而不是覆盖/篡改文件树。
        let tree_area = if plugin_sidebar_slots.is_empty() {
            area
        } else {
            Rect::new(area.left, area.top, area.right, area.bottom - 72.0)
        };
        let mut layout = sidebar::layout(&model, tree_area, self.side.scroll);
        let max = layout.max_scroll();
        if self.side.scroll > max {
            self.side.scroll = max;
            layout = sidebar::layout(&model, tree_area, max);
        }
        sidebar::paint(&mut self.list, tree_area, &model, &layout, p);
        self.paint_document_unread_tree(&layout, p);
        if !plugin_sidebar_slots.is_empty() {
            let host = Rect::new(area.left, area.bottom - 72.0, area.right, area.bottom);
            self.list.rect(
                Rect::new(host.left, host.top, host.right, host.top + 1.0),
                p.border,
            );
            let mut y = host.top + 8.0;
            for slot in plugin_sidebar_slots.iter().take(2) {
                self.list.text(
                    Rect::new(host.left + 12.0, y, host.right - 12.0, y + 24.0),
                    format!(
                        "{} · {}",
                        slot.plugin.manifest.name, slot.contribution.title
                    ),
                    TextStyle::Caption,
                    p.foreground,
                );
                y += 28.0;
            }
        }
        if self.shell.favorites_selected() && self.side.editing.is_none() {
            let hint = if !self.shell.favorites_error().is_empty() {
                Some(("无法读取收藏，请点击刷新", p.danger))
            } else if self.shell.rows().is_empty() {
                Some(("还没有收藏文档\n在文件上右键选择「收藏文档」", p.muted))
            } else if sidebar::visible_rows(self.shell.rows(), self.side.search.text()).is_empty() {
                Some(("没有匹配的收藏文档", p.muted))
            } else {
                None
            };
            if let Some((message, color)) = hint {
                let content = layout.content;
                self.list.push_clip(content);
                self.list.text(
                    Rect::new(
                        content.left + 12.0,
                        content.top + 16.0,
                        content.right - 12.0,
                        content.top + 88.0,
                    ),
                    message,
                    TextStyle::Caption,
                    color,
                );
                self.list.pop_clip();
            }
        }
        self.side.layout = layout;
    }

    pub(super) fn tab_fixed_width(&self) -> bool {
        crate::ui::settings_values::boolean("tabs.fixedWidth", true)
    }

    pub(super) fn paint_tab_bar(&mut self, chrome: &Chrome, p: &Palette) {
        if chrome.tree.is_hidden(chrome.tab_bar) {
            return;
        }
        let tabs = self.tab_projection();
        let full_area = chrome.tree.rect(chrome.tab_bar);
        let area = tab_bar::tabs_content_area(full_area);
        let active = self
            .shell
            .active_tab()
            .and_then(|i| tabs.get(i).map(|t| (i, t.title.clone())));
        let fixed_width = self.tab_fixed_width();
        if self.last_active_tab != active {
            if let Some((i, _)) = &active {
                if let Some(r) = tab_bar::layout_with_fixed_width(
                    area,
                    &tabs,
                    self.state.compact_tab_bar,
                    fixed_width,
                )
                .get(*i)
                {
                    if r.rect.left < area.left + self.tab_scroll {
                        self.tab_scroll = (r.rect.left - area.left).max(0.0);
                    } else if r.rect.right > area.right + self.tab_scroll {
                        self.tab_scroll = r.rect.right - area.right;
                    }
                }
            }
            self.last_active_tab = active;
        }
        self.tab_scroll = self.tab_scroll.min(tab_bar::max_scroll_with_fixed_width(
            area,
            &tabs,
            self.state.compact_tab_bar,
            fixed_width,
        ));
        tab_bar::paint_scrolled_with_fixed_width(
            &mut self.list,
            area,
            &tabs,
            self.shell.active_tab(),
            self.state.compact_tab_bar,
            fixed_width,
            self.tab_scroll,
            p,
        );
        if self.show_document_unread() {
            self.list.push_clip(area);
            for (index, tab) in tab_bar::layout_with_fixed_width(
                area,
                &tabs,
                self.state.compact_tab_bar,
                fixed_width,
            )
            .iter()
            .enumerate()
            {
                if self
                    .shell
                    .tabs()
                    .get(index)
                    .and_then(|tab| tab.path())
                    .is_some_and(|path| {
                        self.nav
                            .unread
                            .documents
                            .contains_key(&mochi_core::document_unread::key(path))
                    })
                {
                    self.list.rounded_rect(
                        Rect::from_size(
                            tab.rect.right - self.tab_scroll - 8.0,
                            tab.rect.top + 3.0,
                            6.0,
                            6.0,
                        ),
                        3.0,
                        p.accent,
                    );
                }
            }
            self.list.pop_clip();
        }
        let enabled = [
            self.tab_navigation_enabled(NavigationDirection::Back),
            self.tab_navigation_enabled(NavigationDirection::Forward),
        ];
        let navigation_hover = self
            .block_pointer
            .and_then(|(x, y)| tab_bar::navigation_hit(full_area, x, y));
        tab_bar::paint_navigation(&mut self.list, full_area, enabled, navigation_hover, p);
        let hover = self
            .block_pointer
            .is_some_and(|(x, y)| tab_bar::split_button_rect(full_area).contains(x, y));
        tab_bar::paint_split_button(
            &mut self.list,
            full_area,
            self.shell
                .active()
                .is_some_and(|tab| tab.buffer().is_some()),
            self.split.other.is_some(),
            hover,
            p,
        );
    }

    // 自定义非客户区后，系统不绘制标题栏按钮。
    pub(super) fn paint_caption_buttons(&mut self, p: &Palette, maximized: bool) {
        let viewport = self.renderer.viewport();
        let glyphs = [
            "\u{2500}",
            if maximized { "\u{2750}" } else { "\u{2610}" },
            "\u{2715}",
        ];
        for (i, rect) in platform::caption_buttons(viewport).iter().enumerate() {
            if self.caption_hover == Some(i) {
                // 关闭按钮的悬停色是红底白字，这是 Windows 通用约定，别自创
                let bg = if i == 2 { p.danger } else { p.surface_muted };
                self.list.rect(*rect, bg);
            }
            let fg = if self.caption_hover == Some(2) && i == 2 {
                p.surface
            } else {
                p.foreground
            };
            self.list
                .text_aligned(*rect, glyphs[i], TextStyle::Caption, fg, Align::Center);
        }
    }
}
