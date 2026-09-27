//! 绘制插件提供的工作区页面。
use super::*;

impl App {
    /// 第三方插件的独立主区。导航只负责切换到这里，绝不绕回设置页。
    pub(super) fn paint_plugin_workspace(&mut self, area: Rect, p: &Palette) {
        let Some(id) = self.plugin_workspace.as_deref() else {
            crate::view::placeholder(&mut self.list, area, "请选择一个插件", p);
            return;
        };
        if id == "quick-navigation" {
            self.paint_quick_navigation(area, p);
            return;
        }
        if id == "english-lab" {
            self.paint_english_lab(area, p);
            return;
        }
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            crate::view::placeholder(&mut self.list, area, "请先打开工作区", p);
            return;
        };
        let service = mochi_core::plugins::PluginService::new(root);
        let Ok(plugins) = service.list() else {
            crate::view::placeholder(&mut self.list, area, "无法读取插件", p);
            return;
        };
        let Some(plugin) = plugins
            .iter()
            .find(|plugin| plugin.enabled && plugin.manifest.id == id)
        else {
            crate::view::placeholder(&mut self.list, area, "插件未安装或已禁用", p);
            return;
        };
        match service.load_ui(plugin) {
            Ok(Some(ui)) => {
                self.list.text(
                    Rect::new(
                        area.left + 32.0,
                        area.top + 24.0,
                        area.right - 32.0,
                        area.top + 58.0,
                    ),
                    ui.title,
                    TextStyle::Large,
                    p.foreground,
                );
                self.list.text(
                    Rect::new(
                        area.left + 32.0,
                        area.top + 64.0,
                        area.right - 32.0,
                        area.top + 88.0,
                    ),
                    plugin.manifest.description.clone(),
                    TextStyle::Caption,
                    p.muted,
                );
                let mut y = area.top + 112.0;
                for section in &ui.sections {
                    self.list.rounded_rect(
                        Rect::new(area.left + 32.0, y, area.right - 32.0, y + 84.0),
                        8.0,
                        p.surface,
                    );
                    self.list.text(
                        Rect::new(area.left + 48.0, y + 12.0, area.right - 48.0, y + 36.0),
                        &section.title,
                        TextStyle::Label,
                        p.foreground,
                    );
                    self.list.text(
                        Rect::new(area.left + 48.0, y + 42.0, area.right - 48.0, y + 66.0),
                        section.cards.join("  ·  "),
                        TextStyle::Caption,
                        p.muted,
                    );
                    y += 96.0;
                }
                if ui.sections.is_empty() {
                    self.list.text(
                        Rect::new(area.left + 32.0, y, area.right - 32.0, y + 28.0),
                        "该插件尚未声明内容区。",
                        TextStyle::Caption,
                        p.muted,
                    );
                }
            }
            Ok(None) => crate::view::placeholder(&mut self.list, area, "该插件没有界面", p),
            Err(error) => crate::view::placeholder(
                &mut self.list,
                area,
                &format!("插件界面加载失败：{error}"),
                p,
            ),
        }
    }
}
