//! 绘制设置浮层并计算其布局区域。
use super::*;

impl App {
    pub(super) fn settings_overlay_rect(&self) -> Rect {
        let viewport = self.renderer.viewport();
        let width = (viewport.width() - 48.0)
            .min(1120.0_f32)
            .max(520.0_f32.min(viewport.width()));
        let height = (viewport.height() - 48.0)
            .min(760.0_f32)
            .max(420.0_f32.min(viewport.height()));
        Rect::from_size(
            viewport.left + (viewport.width() - width) / 2.0,
            viewport.top + (viewport.height() - height) / 2.0,
            width,
            height,
        )
    }

    pub(super) fn paint_settings_overlay(&mut self, p: &Palette) {
        let Some((tab, section)) = self.settings_overlay.clone() else {
            return;
        };
        let viewport = self.renderer.viewport();
        let modal = self.settings_overlay_rect();
        self.list.rect_alpha(viewport, 0x000000, 0.48);
        self.list.rounded_rect(modal, 14.0, p.surface_elevated);
        self.list.rounded_border(modal, 14.0, p.border);
        let nav = Rect::new(
            modal.left,
            modal.top,
            modal.left + 224.0_f32.min(modal.width() * 0.34),
            modal.bottom,
        );
        let content = Rect::new(nav.right + 1.0, modal.top, modal.right, modal.bottom);
        let nav_lay = settings::nav_layout(nav, &tab, self.prefs.nav_scroll);
        settings::paint_nav(
            &mut self.list,
            nav,
            &nav_lay,
            &tab,
            &section,
            self.block_pointer,
            p,
        );
        self.prefs.nav_scroll = nav_lay.scroll;
        self.prefs.nav_layout = nav_lay;
        if tab == "ai" && section != "parameters" {
            self.paint_provider_settings(
                Rect::new(
                    content.left,
                    content.top + 44.0,
                    content.right,
                    content.bottom,
                ),
                p,
            );
            self.list
                .icon_centered(settings::close_rect(modal), Icon::X, 18.0, p.muted);
            return;
        }
        if tab == "plugins" {
            self.list.text(
                Rect::new(
                    content.left + 24.0,
                    content.top + 16.0,
                    content.right - 52.0,
                    content.top + 48.0,
                ),
                "第三方库",
                TextStyle::Large,
                p.foreground,
            );
            self.list.text(
                Rect::new(
                    content.left + 24.0,
                    content.top + 72.0,
                    content.right - 24.0,
                    content.top + 104.0,
                ),
                "暂不支持",
                TextStyle::Label,
                p.foreground,
            );
            self.list.text(
                Rect::new(
                    content.left + 24.0,
                    content.top + 110.0,
                    content.right - 24.0,
                    content.top + 136.0,
                ),
                "导入后安装到当前工作区 .mochi/plugins；不加载 HTML/JS 或 Electron。",
                TextStyle::Caption,
                p.muted,
            );
            for (left, right, label) in [
                (24.0, 174.0, "导入原生 ZIP"),
                (190.0, 370.0, "打开原生插件目录"),
                (386.0, 556.0, "插件创作指南"),
                (572.0, 722.0, "打包插件工程"),
            ] {
                let r = Rect::new(
                    content.left + left,
                    content.top + 150.0,
                    content.left + right,
                    content.top + 186.0,
                );
                self.list.rounded_border(r, 8.0, p.border);
                self.list.text(
                    Rect::new(r.left + 10.0, r.top, r.right, r.bottom),
                    label,
                    TextStyle::Caption,
                    p.foreground,
                );
            }
            if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                match mochi_core::plugins::PluginService::new(root).list() {
                    Ok(plugins) if plugins.is_empty() => self.list.text(
                        Rect::new(
                            content.left + 24.0,
                            content.top + 210.0,
                            content.right - 24.0,
                            content.top + 236.0,
                        ),
                        "尚未安装原生插件。",
                        TextStyle::Label,
                        p.muted,
                    ),
                    Ok(plugins) => {
                        self.list.text(
                            Rect::new(
                                content.left + 24.0,
                                content.top + 210.0,
                                content.right - 24.0,
                                content.top + 236.0,
                            ),
                            "已安装的原生插件",
                            TextStyle::Label,
                            p.foreground,
                        );
                        for (index, plugin) in plugins.iter().enumerate() {
                            let y = content.top + 246.0 + index as f32 * 34.0;
                            self.list.text(
                                Rect::new(content.left + 36.0, y, content.right - 36.0, y + 28.0),
                                format!(
                                    "{} · v{} · {}",
                                    plugin.manifest.name,
                                    plugin.manifest.version,
                                    if plugin.enabled {
                                        "已启用"
                                    } else {
                                        "已禁用"
                                    }
                                ),
                                TextStyle::Caption,
                                p.foreground,
                            );
                        }
                    }
                    Err(error) => self.list.text(
                        Rect::new(
                            content.left + 24.0,
                            content.top + 210.0,
                            content.right - 24.0,
                            content.top + 236.0,
                        ),
                        format!("无法读取插件：{error}"),
                        TextStyle::Caption,
                        p.danger,
                    ),
                }
            }
            self.list
                .icon_centered(settings::close_rect(modal), Icon::X, 18.0, p.muted);
            return;
        }
        let section_opt = if tab == "customization" {
            Some(section.as_str())
        } else {
            None
        };
        let mut lay = settings::content_layout_filtered(
            content,
            &tab,
            section_opt,
            self.prefs.scroll,
            self.prefs.search.text(),
        );
        let max = lay.max_scroll();
        let clamped = self.prefs.scroll.clamp(0.0, max);
        if clamped != self.prefs.scroll {
            self.prefs.scroll = clamped;
            lay = settings::content_layout_filtered(
                content,
                &tab,
                section_opt,
                clamped,
                self.prefs.search.text(),
            );
        }
        settings::sync_editing_scroll(&lay, self.prefs.editing.as_mut());
        let app_settings = &self.app_settings;
        let read = |d: &app_settings::SettingDescriptor| app_settings.read(d);
        let model = settings::ContentModel {
            tab: &tab,
            section: section_opt,
            query: self.prefs.search.text(),
            read: &read,
            editing: self.prefs.editing.as_ref(),
        };
        settings::paint_content(&mut self.list, content, &lay, &model, self.prefs.scroll, p);
        if !lay.search_rect.is_empty() {
            let mut search = self.prefs.search.clone();
            search.paint(
                &mut self.list,
                lay.search_rect,
                self.focus == Focus::SettingsSearch,
                p,
                FieldLook::dialog(p),
            );
        }
        self.prefs.content_layout = lay;
        self.sync_settings_section();
        self.list
            .icon_centered(settings::close_rect(modal), Icon::X, 18.0, p.muted);
    }
}
