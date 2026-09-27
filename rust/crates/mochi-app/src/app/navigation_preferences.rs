//! 保存导航栏设置时，不影响当前打开的文档。
use super::*;
use crate::ui::navigation_preferences::{Action, Preferences, HIDDEN_KEY, ORDER_KEY};

impl App {
    #[cfg(debug_assertions)]
    fn click_navigation_preference(&mut self, action: Action) -> anyhow::Result<()> {
        use anyhow::Context;
        self.paint(HWND::default())?;
        let rect = self
            .prefs
            .content_layout
            .navigation
            .control(action)
            .context("navigation preference control is missing")?;
        anyhow::ensure!(
            self.prefs.content_layout.body.contains(
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0
            ),
            "navigation preference control is outside the viewport"
        );
        self.on_settings_content_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        self.paint(HWND::default())?;
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub(super) fn verify_navigation_preferences(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        use anyhow::ensure;
        let file = folder.join("保留的笔记.md");
        std::fs::write(&file, "# 保留的笔记\n")?;
        ensure!(self.shell.open_file(&file));
        self.shell
            .active_buffer_mut()
            .unwrap()
            .insert("未保存的输入\n");
        let original = self.shell.active_buffer_mut().unwrap().text().to_owned();
        self.open_settings("customization");
        if let Some(TabKind::Settings { section, .. }) =
            self.shell.active_mut().map(|tab| &mut tab.kind)
        {
            *section = "navigation".into();
        }
        self.prefs.scroll = 0.0;
        self.paint(HWND::default())?;
        ensure!(Preferences::read().visible() == NavItem::ALL);
        self.verify_frame(output, "default", snapshot)?;

        self.click_navigation_preference(Action::Toggle(NavItem::Inbox))?;
        ensure!(self
            .nav_layout
            .rect_of(NavHit::Item(NavItem::Inbox))
            .is_none());
        for _ in 0..3 {
            self.click_navigation_preference(Action::Up(NavItem::Favorites))?;
        }
        ensure!(Preferences::read().visible().first() == Some(&NavItem::Favorites));
        let visible = self
            .nav_layout
            .entries
            .iter()
            .filter_map(|(_, hit)| {
                if let NavHit::Item(item) = hit {
                    Some(*item)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        ensure!(visible == Preferences::read().visible());
        self.verify_frame(output, "customized", snapshot)?;
        let root = self.shell.workspace().unwrap().root.clone();
        let persisted = app_settings::AppSettings::new(Arc::new(SettingsService::new(Some(
            root.join(".settings.json"),
        ))));
        let read = |key| {
            persisted
                .read(app_settings::descriptor(key).unwrap())
                .to_storage()
        };
        ensure!(Preferences::parse(&read(ORDER_KEY), &read(HIDDEN_KEY)) == Preferences::read());

        let collapse = self.nav_layout.rect_of(NavHit::Collapse).unwrap();
        self.on_navigation_click(collapse.left + 4.0, collapse.top + 4.0);
        self.paint(HWND::default())?;
        ensure!(self.state.navigation_collapsed);
        ensure!(self
            .nav_layout
            .rect_of(NavHit::Item(NavItem::Inbox))
            .is_none());
        self.verify_frame(output, "collapsed", snapshot)?;
        for item in Preferences::read().visible() {
            self.click_navigation_preference(Action::Toggle(item))?;
        }
        ensure!(self
            .nav_layout
            .entries
            .iter()
            .all(|(_, hit)| !matches!(hit, NavHit::Item(_))));
        ensure!(self.nav_layout.rect_of(NavHit::Settings).is_some());
        ensure!(self.nav_layout.rect_of(NavHit::Search).is_some());
        self.verify_frame(output, "all-hidden", snapshot)?;
        self.click_navigation_preference(Action::Reset)?;
        ensure!(Preferences::read().visible() == NavItem::ALL);
        let tab = self
            .shell
            .tabs()
            .iter()
            .find(|tab| tab.path() == Some(file.as_path()))
            .unwrap();
        ensure!(tab.dirty() && tab.buffer().unwrap().text() == original);
        ensure!(std::fs::read_to_string(&file)? == "# 保留的笔记\n");
        let collapse = self.nav_layout.rect_of(NavHit::Collapse).unwrap();
        self.on_navigation_click(collapse.left + 4.0, collapse.top + 4.0);
        self.paint(HWND::default())?;
        self.verify_frame(output, "reset", snapshot)?;
        self.renderer.save_snapshot(snapshot, output)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed":true,"realD2D":true,"visibilityAppliesImmediately":true,"orderAppliesImmediately":true,
                "persistedAcrossReload":true,"collapsedNavigation":true,"allHiddenCanBeReset":true,"dirtyDocumentPreserved":true
            }))?,
        )?;
        Ok(())
    }

    pub(super) fn edit_navigation_preferences(&mut self, action: Action) {
        if let Err(error) = self.settings.reload() {
            self.show_global_notice(format!("导航设置读取失败：{error}"));
            return;
        }
        crate::ui::settings_values::load(&self.app_settings);
        let mut preferences = Preferences::read();
        if !preferences.apply(action) {
            return;
        }
        let changes = match action {
            Action::Toggle(_) => vec![(HIDDEN_KEY, preferences.hidden_value())],
            Action::Up(_) | Action::Down(_) => vec![(ORDER_KEY, preferences.order_value())],
            Action::Reset => vec![
                (ORDER_KEY, preferences.order_value()),
                (HIDDEN_KEY, preferences.hidden_value()),
            ],
        };
        for (key, value) in changes {
            if let Some(descriptor) = app_settings::descriptor(key) {
                self.app_settings
                    .write(descriptor, &SettingValue::Text(value));
            }
        }
        if let Err(error) = self.app_settings.flush() {
            self.show_global_notice(format!("导航设置保存失败：{error}"));
            return;
        }
        self.apply_setting_side_effects(ORDER_KEY);
        self.focus = Focus::Main;
    }
}
