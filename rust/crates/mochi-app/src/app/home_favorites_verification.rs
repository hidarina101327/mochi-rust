//! 使用现有的隔离 D2D 渲染器，对原生仪表盘进行端到端检查。
use super::*;
use anyhow::{ensure, Context};

impl App {
    fn click_home_verification_action(
        &mut self,
        action: crate::ui::home::Action,
    ) -> anyhow::Result<()> {
        self.state.view = WorkspaceView::Home;
        self.paint(HWND::default())?;
        let area = self.editor_area;
        self.home.scroll_by(area, 100_000.0);
        for _ in 0..30 {
            self.paint(HWND::default())?;
            for y in (area.top as i32 + 8..area.bottom as i32 - 8).step_by(12) {
                for x in (area.left as i32 + 8..area.right as i32 - 8).step_by(12) {
                    if self.home.hit(area, x as f32, y as f32) == Some(action.clone()) {
                        self.on_click(x as f32, y as f32);
                        return Ok(());
                    }
                }
            }
            self.home.scroll_by(area, -400.0);
        }
        anyhow::bail!("home action not reachable: {action:?}")
    }

    pub(super) fn verify_home_favorites(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        let center = |r: Rect| ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        let branch = folder.join("收藏目录").join("深层");
        std::fs::create_dir_all(&branch)?;

        let sample = branch.join("收藏验收.md");
        std::fs::write(&sample, "# 收藏验收\n\n这篇文档应该能在收藏树中单独打开。")?;
        let unstarred = branch.join("未收藏文档.md");
        std::fs::write(&unstarred, "# 不应出现在收藏树")?;

        // 保持与旧验收相同的 25 个滚动目标；多层目录另加一篇文档检查祖先树。
        let mut cards = Vec::new();
        for i in 0..24 {
            let path = folder.join(format!("知识卡片-{i:02}.md"));
            std::fs::write(&path, format!("# 知识卡片 {i}\n\n测试收藏树滚动。"))?;
            cards.push(path);
        }
        // 最后写入多维表格，使 Home 的第一个收藏仍稳定地覆盖 .mcb 查看器路径。
        let base = branch.join("项目资料.mcb");
        std::fs::write(
            &base,
            mochi_core::base::serialize_base_document(&mochi_core::base::create_base_document())?,
        )?;

        self.shell.refresh_tree();
        self.run_menu_action(MenuAction::ToggleFavorite(sample.clone()));
        ensure!(
            self.shell.is_favorite(&sample),
            "favorite action must persist"
        );
        for path in &cards {
            ensure!(
                self.shell.toggle_favorite(path)?,
                "card favorite must be added"
            );
        }
        ensure!(
            self.shell.toggle_favorite(&base)?,
            "mcb favorite must be added"
        );
        self.reload_favorites();

        // 导航收藏后仍是编辑器外壳：左侧收藏树、标签栏和右侧空态/文件区都可见。
        self.paint(HWND::default())?;
        let favorites_nav = center(
            self.nav_layout
                .rect_of(NavHit::Item(NavItem::Favorites))
                .context("favorites navigation")?,
        );
        self.on_click(favorites_nav.0, favorites_nav.1);
        self.paint(HWND::default())?;
        ensure!(self.state.view == WorkspaceView::Editor);
        ensure!(self.shell.favorites_selected());
        ensure!(
            !self.shell.show_favorite_parents(),
            "favorites must be flat by default"
        );
        ensure!(
            self.shell.rows().len() == 26
                && self
                    .shell
                    .rows()
                    .iter()
                    .all(|row| !row.is_dir && row.depth == 0),
            "default favorites leaked parent folders"
        );
        self.verify_frame(output, "flat-default", snapshot)?;
        let setting = app_settings::descriptor("sidebar.showFavoriteParents")
            .context("favorite parent setting")?;
        self.app_settings.write(setting, &SettingValue::Bool(true));
        self.apply_setting_side_effects(&setting.key);
        self.paint(HWND::default())?;
        ensure!(
            self.shell.show_favorite_parents(),
            "favorite setting was not applied live"
        );
        ensure!(self.nav_active() == Some(NavItem::Favorites));
        ensure!(matches!(
            self.content(),
            MainContent::NoTab | MainContent::Document | MainContent::Special
        ));
        let chrome = self.build_chrome();
        ensure!(
            !chrome.tree.is_hidden(chrome.sidebar),
            "favorites sidebar hidden"
        );
        ensure!(
            !chrome.tree.is_hidden(chrome.tab_bar),
            "favorites tab bar hidden"
        );

        let rows = self.shell.rows();
        ensure!(
            rows.iter().any(|r| r.is_dir && r.path == folder),
            "favorite tree omitted library ancestor"
        );
        ensure!(
            rows.iter().any(|r| r.is_dir && r.path == branch),
            "favorite tree omitted nested ancestor"
        );
        for path in [folder, branch.as_path()] {
            ensure!(
                rows.iter()
                    .any(|r| r.path == path && r.is_dir && r.expanded),
                "favorite ancestor is not expanded: {}",
                path.display()
            );
        }
        ensure!(
            rows.iter()
                .filter(|r| !r.is_dir)
                .all(|r| self.shell.is_favorite(&r.path)),
            "unstarred file leaked into favorite tree"
        );
        for path in self.shell.favorite_paths() {
            ensure!(
                rows.iter().any(|r| r.path == path && !r.is_dir),
                "favorite file missing from expanded tree: {}",
                path.display()
            );
        }
        ensure!(
            !rows.iter().any(|r| r.path == unstarred),
            "unstarred file appeared in favorite tree"
        );

        // 搜索复用普通侧栏输入框；多层祖先必须随匹配文件一并保留。
        let search = center(
            self.side
                .layout
                .rect_of(SidebarHit::Search)
                .context("favorite sidebar search")?,
        );
        self.on_click(search.0, search.1);
        for ch in "收藏验收".chars() {
            ensure!(self.on_char(ch), "favorite sidebar search rejected input");
        }
        ensure!(self.focus == Focus::SidebarSearch);
        self.paint(HWND::default())?;
        let visible = crate::ui::sidebar::visible_rows(self.shell.rows(), self.side.search.text());
        ensure!(
            visible.iter().any(|&i| self.shell.rows()[i].path == sample),
            "favorite search omitted matching document"
        );
        ensure!(
            visible
                .iter()
                .filter(|&&i| !self.shell.rows()[i].is_dir)
                .count()
                == 1,
            "favorite search did not filter documents"
        );
        ensure!(
            !visible
                .iter()
                .any(|&i| self.shell.rows()[i].path == unstarred),
            "favorite search exposed unstarred document"
        );

        // 重新点收藏入口，确认侧栏搜索焦点随导航释放并清空。
        let favorites_nav = center(
            self.nav_layout
                .rect_of(NavHit::Item(NavItem::Favorites))
                .context("favorites navigation while searching")?,
        );
        self.on_click(favorites_nav.0, favorites_nav.1);
        ensure!(self.focus == Focus::Main);
        ensure!(self.side.search.text().is_empty());
        self.paint(HWND::default())?;

        // 再走一次 SearchClear 命中，覆盖清除按钮本身以及空搜索后的树恢复。
        let search = center(
            self.side
                .layout
                .rect_of(SidebarHit::Search)
                .context("favorite sidebar search after navigation")?,
        );
        self.on_click(search.0, search.1);
        for ch in "收藏验收".chars() {
            ensure!(self.on_char(ch));
        }
        self.paint(HWND::default())?;
        let clear = center(
            self.side
                .layout
                .rect_of(SidebarHit::SearchClear)
                .context("favorite search clear")?,
        );
        ensure!(self.side.layout.hit(clear.0, clear.1) == Some(SidebarHit::SearchClear));
        self.on_click(clear.0, clear.1);
        ensure!(self.side.search.text().is_empty());
        ensure!(self.focus == Focus::Main);
        self.paint(HWND::default())?;

        // 行 chevron 折叠后子项消失；ExpandAll 再把收藏祖先全部展开。
        let branch_index = self
            .shell
            .rows()
            .iter()
            .position(|r| r.path == branch && r.is_dir)
            .context("favorite nested ancestor row")?;
        let chevron = center(
            self.side
                .layout
                .rect_of(SidebarHit::RowChevron(branch_index))
                .context("favorite ancestor chevron")?,
        );
        ensure!(
            self.side.layout.hit(chevron.0, chevron.1)
                == Some(SidebarHit::RowChevron(branch_index))
        );
        self.on_click(chevron.0, chevron.1);
        self.paint(HWND::default())?;
        ensure!(
            !self.shell.rows().iter().any(|r| r.path == sample),
            "collapsed favorite ancestor still exposed child"
        );
        ensure!(self.shell.favorites_selected());
        let expand_all = center(
            self.side
                .layout
                .rect_of(SidebarHit::ExpandAll)
                .context("favorite expand all")?,
        );
        self.on_click(expand_all.0, expand_all.1);
        self.paint(HWND::default())?;
        ensure!(self.shell.all_expanded());
        ensure!(self.shell.rows().iter().any(|r| r.path == sample));

        // 点击真实 Sidebar Row 打开 Markdown，并保持收藏导航高亮。
        let sample_index = self
            .shell
            .rows()
            .iter()
            .position(|r| r.path == sample && !r.is_dir)
            .context("favorite document row")?;
        let sample_row = center(
            self.side
                .layout
                .rect_of(SidebarHit::Row(sample_index))
                .context("visible favorite document row")?,
        );
        ensure!(
            self.side.layout.hit(sample_row.0, sample_row.1) == Some(SidebarHit::Row(sample_index))
        );
        self.on_click(sample_row.0, sample_row.1);
        self.paint(HWND::default())?;
        ensure!(self.state.view == WorkspaceView::Editor);
        ensure!(self.content() == MainContent::Document);
        ensure!(self.active_file_path().as_deref() == Some(sample.as_path()));
        ensure!(self.shell.favorites_selected());
        ensure!(self.nav_active() == Some(NavItem::Favorites));

        // 右侧编辑区可输入；内容只改隔离验收文件，稍后切工作区时由既有保存流程处理。
        let editor_area = self.editor_area;
        if editor_area.height() > 32.0 {
            self.on_click(
                (editor_area.left + editor_area.right) / 2.0,
                (editor_area.top + 96.0).min(editor_area.bottom - 8.0),
            );
        }
        self.focus = Focus::Main;
        self.editor_engaged = true;
        let before = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .map(|buffer| buffer.text().to_owned())
            .context("favorite markdown buffer")?;
        ensure!(
            self.on_char('改'),
            "favorite document rejected editor input"
        );
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text() != before),
            "favorite document did not accept editor input"
        );
        self.verify_frame(output, "favorites", snapshot)?;
        let opened = self.active_file_path();
        self.app_settings.write(setting, &SettingValue::Bool(false));
        self.apply_setting_side_effects(&setting.key);
        self.paint(HWND::default())?;
        ensure!(self.shell.rows().iter().all(|r| !r.is_dir && r.depth == 0));
        ensure!(
            self.active_file_path() == opened,
            "toggling parent tree changed current document"
        );
        self.verify_frame(output, "flat-document", snapshot)?;
        self.app_settings.write(setting, &SettingValue::Bool(true));
        self.apply_setting_side_effects(&setting.key);
        self.paint(HWND::default())?;

        // 通过实际右键菜单取消收藏；已打开的脏文档仍须留在标签中。
        self.paint(HWND::default())?;
        let sample_index = self
            .shell
            .rows()
            .iter()
            .position(|r| r.path == sample && !r.is_dir)
            .context("favorite row before context menu")?;
        let sample_row = center(
            self.side
                .layout
                .rect_of(SidebarHit::Row(sample_index))
                .context("favorite row context menu")?,
        );
        self.on_right_click(sample_row.0, sample_row.1);
        let (menu_x, menu_y) = {
            let menu = self.menu.as_ref().context("favorite context menu")?;
            let index = menu
                .items
                .iter()
                .position(|item| {
                    matches!(&item.action, MenuAction::ToggleFavorite(path) if path == &sample)
                })
                .context("favorite toggle menu item")?;
            let mut top = menu.rect.top + crate::ui::widgets::MENU_PADDING;
            for (i, item) in menu.items.iter().enumerate() {
                if item.separator_before {
                    top += crate::ui::widgets::MENU_SEPARATOR_HEIGHT;
                }
                if i == index {
                    break;
                }
                top += crate::ui::widgets::MENU_ITEM_HEIGHT;
            }
            (
                menu.rect.left + crate::ui::widgets::MENU_PADDING + 24.0,
                top + crate::ui::widgets::MENU_ITEM_HEIGHT / 2.0,
            )
        };
        self.on_click(menu_x, menu_y);
        self.paint(HWND::default())?;
        ensure!(!self.shell.is_favorite(&sample));
        ensure!(!self.shell.rows().iter().any(|r| r.path == sample));
        ensure!(self.active_file_path().as_deref() == Some(sample.as_path()));
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text() != before),
            "removing favorite closed or reset opened document"
        );
        ensure!(self.shell.favorites_selected());

        // 收藏树也必须能正常打开 .mcb 查看器。
        let base_index = self
            .shell
            .rows()
            .iter()
            .position(|r| r.path == base && !r.is_dir)
            .context("mcb favorite row")?;
        let base_row = center(
            self.side
                .layout
                .rect_of(SidebarHit::Row(base_index))
                .context("visible mcb favorite row")?,
        );
        self.on_click(base_row.0, base_row.1);
        self.paint(HWND::default())?;
        ensure!(self.content() == MainContent::Special);
        ensure!(self.active_file_path().as_deref() == Some(base.as_path()));
        ensure!(self
            .viewer_tab()
            .is_some_and(|(path, content)| path == base.as_path()
                && matches!(content, viewer::Content::Base(_))));
        ensure!(self.shell.favorites_selected());
        ensure!(self.nav_active() == Some(NavItem::Favorites));

        // 离开收藏恢复普通知识库树；展开普通分支能看到未收藏文件，再切回收藏。
        self.paint(HWND::default())?;
        let knowledge_nav = center(
            self.nav_layout
                .rect_of(NavHit::Item(NavItem::Knowledge))
                .context("knowledge navigation")?,
        );
        self.on_click(knowledge_nav.0, knowledge_nav.1);
        self.paint(HWND::default())?;
        ensure!(!self.shell.favorites_selected());
        ensure!(self.nav_active() == Some(NavItem::Knowledge));
        ensure!(
            !self.shell.rows().iter().any(|r| r.path == unstarred),
            "favorites expansion leaked into the normal knowledge tree"
        );
        let normal_expand = center(
            self.side
                .layout
                .rect_of(SidebarHit::ExpandAll)
                .context("normal knowledge expand button")?,
        );
        self.on_click(normal_expand.0, normal_expand.1);
        self.paint(HWND::default())?;
        ensure!(
            self.shell.rows().iter().any(|r| r.path == unstarred),
            "knowledge tree did not restore ordinary files"
        );

        self.paint(HWND::default())?;
        let favorites_nav = center(
            self.nav_layout
                .rect_of(NavHit::Item(NavItem::Favorites))
                .context("favorites navigation after knowledge")?,
        );
        self.on_click(favorites_nav.0, favorites_nav.1);
        self.paint(HWND::default())?;
        ensure!(self.shell.favorites_selected());
        ensure!(self.nav_active() == Some(NavItem::Favorites));
        ensure!(!self.shell.rows().iter().any(|r| r.path == unstarred));
        ensure!(self.shell.rows().iter().any(|r| r.path == base));

        // 滚动左树而不是正文，再从当前可见 Sidebar Row 取路径，防止行号/滚动错位。
        let sidebar_content = self.side.layout.content;
        self.on_wheel(
            sidebar_content.left + 10.0,
            sidebar_content.top + 18.0,
            -2400,
        );
        self.paint(HWND::default())?;
        ensure!(self.side.scroll > 0.0, "favorite tree did not scroll");
        let card_entry = self
            .side
            .layout
            .entries
            .iter()
            .find_map(|(rect, hit)| match hit {
                SidebarHit::Row(index)
                    if rect.top >= sidebar_content.top
                        && rect.bottom <= sidebar_content.bottom
                        && self.shell.rows().get(*index).is_some_and(|row| {
                            !row.is_dir && row.name.starts_with("知识卡片-")
                        }) =>
                {
                    Some((*rect, *index))
                }
                _ => None,
            })
            .context("visible scrolled favorite row")?;
        let scrolled_path = self.shell.rows()[card_entry.1].path.clone();
        let scrolled_center = center(card_entry.0);
        ensure!(
            self.side.layout.hit(scrolled_center.0, scrolled_center.1)
                == Some(SidebarHit::Row(card_entry.1))
        );
        self.on_click(scrolled_center.0, scrolled_center.1);
        self.paint(HWND::default())?;
        ensure!(self.active_file_path().as_deref() == Some(scrolled_path.as_path()));
        ensure!(self.content() == MainContent::Document);
        ensure!(self.shell.favorites_selected());
        ensure!(self.nav_active() == Some(NavItem::Favorites));
        self.verify_frame(output, "scrolled", snapshot)?;

        self.open_today_journal();
        let journal = self.today_journal_path().context("journal path")?;
        ensure!(self.active_file_path().as_deref() == Some(journal.as_path()));
        let body = std::fs::read_to_string(&journal)?;
        self.open_today_journal();
        ensure!(
            std::fs::read_to_string(&journal)? == body,
            "opening journal twice preserves contents"
        );

        // 原有 Home 动作检查保持不变；收藏入口现在回到 Editor + 收藏树。
        self.state.view = WorkspaceView::Home;
        self.reload_home_dashboard();
        let root = self.shell.workspace().unwrap().root.clone();
        self.home.set_analytics(mochi_core::analytics::build(
            &root,
            Some(365.0),
            chrono::Local::now(),
        ));
        self.home_dirty = false;
        self.click_home_verification_action(crate::ui::home::Action::NewNote)?;
        ensure!(
            self.dialog.is_some(),
            "home NewNote must open the real create dialog"
        );
        self.close_dialog();
        let home_favorite = self
            .home
            .dashboard()
            .favorite_documents
            .first()
            .map(|document| document.path.clone())
            .context("home favorite document")?;
        ensure!(
            home_favorite == base,
            "latest mcb favorite should lead the home list"
        );
        self.click_home_verification_action(crate::ui::home::Action::OpenFavoritesView)?;
        ensure!(self.state.view == WorkspaceView::Editor);
        ensure!(self.shell.favorites_selected());
        ensure!(self.nav_active() == Some(NavItem::Favorites));
        ensure!(matches!(
            self.content(),
            MainContent::NoTab | MainContent::Document | MainContent::Special
        ));
        self.click_home_verification_action(crate::ui::home::Action::OpenFavorite(0))?;
        ensure!(self.active_file_path().as_deref() == Some(base.as_path()));
        ensure!(!self.shell.favorites_selected());
        self.click_home_verification_action(crate::ui::home::Action::OpenSchedule)?;
        ensure!(self.state.view == WorkspaceView::Schedule);
        self.click_home_verification_action(crate::ui::home::Action::OpenAi)?;
        ensure!(self.state.view == WorkspaceView::MochiAi && self.focus == Focus::AiInput);
        self.state.view = WorkspaceView::Home;
        self.home.scroll_by(self.editor_area, 100_000.0);
        self.status_bar.toast.message.clear();
        self.paint(HWND::default())?;
        self.verify_frame(output, "home", snapshot)?;
        self.renderer.save_snapshot(snapshot, output)?;
        self.home.scroll_by(self.editor_area, -900.0);
        self.verify_frame(output, "home-lower", snapshot)?;

        self.views.inbox.loaded = true;
        self.views
            .inbox
            .items
            .push(mochi_core::capture::CaptureItem {
                id: "old-workspace".into(),
                ..Default::default()
            });
        let other_workspace = root.join(".verify-other-workspace");
        std::fs::create_dir_all(&other_workspace)?;
        self.open_workspace(HWND::default(), other_workspace.clone(), false)?;
        ensure!(
            self.views.favorites.docs.is_empty(),
            "workspace switch leaked favorites"
        );
        ensure!(
            self.views.inbox.items.is_empty(),
            "workspace switch leaked inbox"
        );
        ensure!(
            self.views
                .recent
                .docs
                .iter()
                .all(|doc| doc.path.starts_with(&other_workspace)),
            "workspace switch leaked recent documents"
        );
        let report = serde_json::json!({
            "passed": true,
            "favoriteOnly": true,
            "flatByDefault": true,
            "parentSettingAppliesLive": true,
            "parentSettingPreservesDocument": true,
            "favoriteAncestors": true,
            "favoriteCollapseExpand": true,
            "search": true,
            "openDocument": true,
            "editorInput": true,
            "removeFavorite": true,
            "multitableFavorite": true,
            "mcbViewer": true,
            "scrollHitTesting": true,
            "journalIdempotent": true,
            "realD2D": true,
            "isolatedWorkspace": true,
            "workspaceSwitchResetsLists": true,
            "navigationReleasesQueryFocus": true,
            "homeActions": ["newNote", "favorites", "favoriteDocument", "schedule", "ai"],
        });
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(())
    }
}
