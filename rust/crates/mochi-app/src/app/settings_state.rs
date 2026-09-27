//! 维护设置页面的打开状态、分类和滚动位置。
use super::*;

impl App {
    pub(super) fn open_settings(&mut self, requested_tab: &str) {
        if !self.commit_title() {
            return;
        }
        let requested_tab = if settings::TABS.iter().any(|(id, _, _)| *id == requested_tab) {
            requested_tab
        } else {
            "general"
        };
        let remembered_tab = self.settings.get(SETTINGS_LAST_TAB_KEY).filter(|value| {
            settings::TABS
                .iter()
                .any(|(id, _, _)| *id == value.as_str())
        });
        // 普通的“设置”入口会重新打开上次查看的位置；专用入口（AI 设置和第三方资料库）
        // 必须按用户要求打开指定位置，
        // 不能跳到上次记住的
        // 通用设置标签页。
        let restore_last_location = requested_tab == "general" && remembered_tab.is_some();
        let tab = if restore_last_location {
            remembered_tab.unwrap()
        } else {
            requested_tab.to_owned()
        };
        let remembered_section = restore_last_location
            .then(|| self.settings.get(SETTINGS_LAST_SECTION_KEY))
            .flatten()
            .unwrap_or_else(|| "appearance".to_owned());
        let section =
            if tab == "customization" && settings::is_customization_section(&remembered_section) {
                remembered_section
            } else {
                "appearance".to_owned()
            };
        let saved_scroll = restore_last_location
            .then(|| self.settings.get(SETTINGS_SCROLL_KEY))
            .flatten();
        self.settings_overlay = Some((tab.clone(), section.clone()));
        self.prefs.search.set_text("");

        // 根据当前视口的内容高度恢复位置。paint() 会在 DPI 或布局变化后
        // 再次限制滚动范围，避免内容变短时
        // 底部出现多余的空白区域。
        if Self::is_provider_settings(&tab, &section) && !self.prefs.providers.loaded {
            self.reload_providers();
        }
        let max_scroll = self.settings_content_max_scroll(&tab, &section);
        if Self::is_provider_settings(&tab, &section) {
            self.set_settings_scroll(
                &tab,
                &section,
                settings::restore_scroll(saved_scroll.as_deref(), max_scroll),
            );
            self.prefs.scroll = 0.0;
        } else {
            self.set_settings_scroll(
                &tab,
                &section,
                settings::restore_scroll(saved_scroll.as_deref(), max_scroll),
            );
            self.prefs.providers.scroll = 0.0;
        }
        self.invalidate_main();
        self.sync_state();
    }

    pub(super) fn close_settings(&mut self) {
        self.commit_settings_field();
        self.cancel_provider_test();
        self.cancel_provider_models();
        let Some((tab, section)) = self.settings_overlay.take() else {
            return;
        };
        let actual_scroll = self.settings_scroll(&tab, &section);
        let raw_scroll = actual_scroll.to_string();
        let scroll = settings::restore_scroll(
            Some(&raw_scroll),
            self.settings_content_max_scroll(&tab, &section),
        );
        self.set_settings_scroll(&tab, &section, scroll);
        self.settings.set(SETTINGS_LAST_TAB_KEY, &tab);
        self.settings.set(SETTINGS_LAST_SECTION_KEY, &section);
        self.settings.set(SETTINGS_SCROLL_KEY, &scroll.to_string());
        if let Err(error) = self.settings.flush() {
            self.state.status_text = format!("设置位置保存失败：{error}");
        }
        self.focus = Focus::Main;
    }

    pub(super) fn is_provider_settings(tab: &str, section: &str) -> bool {
        tab == "ai" && section != "parameters"
    }

    pub(super) fn settings_scroll(&self, tab: &str, section: &str) -> f32 {
        if Self::is_provider_settings(tab, section) {
            self.prefs.providers.scroll
        } else {
            self.prefs.scroll
        }
    }

    pub(super) fn set_settings_scroll(&mut self, tab: &str, section: &str, scroll: f32) {
        if Self::is_provider_settings(tab, section) {
            self.prefs.providers.scroll = scroll;
        } else {
            self.prefs.scroll = scroll;
        }
    }

    pub(super) fn settings_content_max_scroll(&self, tab: &str, section: &str) -> f32 {
        let modal = self.settings_overlay_rect();
        let nav_width = 224.0_f32.min(modal.width() * 0.34);
        let content = Rect::new(
            modal.left + nav_width + 1.0,
            modal.top,
            modal.right,
            modal.bottom,
        );
        if Self::is_provider_settings(tab, section) {
            let area = Rect::new(
                content.left,
                content.top + 44.0,
                content.right,
                content.bottom,
            );
            return providers::layout(&self.prefs.providers, area).max_scroll();
        }
        let section_opt = (tab == "customization").then_some(section);
        settings::content_layout_filtered(content, tab, section_opt, 0.0, self.prefs.search.text())
            .max_scroll()
    }

    pub(super) fn sync_settings_section(&mut self) {
        let Some((tab, current_section)) = self.settings_overlay.clone() else {
            return;
        };
        if tab != "customization" {
            return;
        }
        let Some(section) = self
            .prefs
            .content_layout
            .section_at_scroll(self.prefs.scroll)
        else {
            return;
        };
        if section != current_section {
            self.settings_overlay = Some((tab, section.to_owned()));
        }
    }

    pub(super) fn set_mcp_status(&mut self, source: &str, status: String) {
        if let Some(card) = self
            .agent
            .data
            .sections
            .get_mut(3)
            .and_then(|cards| cards.iter_mut().find(|c| c.source_path == source))
        {
            if let Some(agent_config::Field::Text { value, .. }) = card
                .fields
                .iter_mut()
                .find(|f| matches!(f,agent_config::Field::Text{label,..}if label=="运行状态"))
            {
                *value = status;
            }
        }
    }

    pub(super) fn start_mcp_test(&mut self, index: usize) {
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let Some(source) = self
            .agent
            .data
            .cards(3)
            .get(index)
            .map(|c| c.source_path.clone())
        else {
            return;
        };
        let Some(config) = mochi_core::ai::agent_config::AgentConfigService::new(&ws.root)
            .load_mcp_servers()
            .into_iter()
            .find(|c| c.source_path == source)
        else {
            return;
        };
        self.set_mcp_status(&source, "正在测试…".into());
        let (tx, rx) = channel();
        self.agent.mcp_rx = Some(rx);
        let hwnd = self.hwnd_raw;
        std::thread::spawn(move || {
            let result = mochi_core::ai::mcp::Client::connect(&config)
                .map(|c| c.tools.len())
                .map_err(|e| e.to_string());
            if tx.send((source, result)).is_ok() {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        platform::WM_APP_MCP_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    pub fn take_mcp_test(&mut self) {
        if let Some((source, result)) = self.agent.mcp_rx.as_ref().and_then(|rx| rx.try_recv().ok())
        {
            self.agent.mcp_rx = None;
            self.set_mcp_status(
                &source,
                match result {
                    Ok(n) => format!("测试通过 · {n} 个工具"),
                    Err(e) => format!("连接失败：{e}"),
                },
            );
        }
    }
}
