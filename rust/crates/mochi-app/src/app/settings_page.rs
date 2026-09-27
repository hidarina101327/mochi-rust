//! 绘制快速导航设置页面。
use super::*;

impl App {
    pub(super) fn paint_quick_navigation_settings(
        &mut self,
        area: Rect,
        model: &mochi_core::quick_navigation::Model,
        p: &Palette,
    ) {
        self.list.text(
            Rect::new(
                area.left + 20.0,
                area.top + 22.0,
                area.right - 20.0,
                area.top + 52.0,
            ),
            "快捷导航设置",
            TextStyle::Large,
            p.foreground,
        );
        self.list.text(
            Rect::new(
                area.left + 20.0,
                area.top + 58.0,
                area.right - 20.0,
                area.top + 80.0,
            ),
            "所有选项保存在旧版兼容的 data.json，Electron 与 Rust 可读取同一份配置。",
            TextStyle::Caption,
            p.muted,
        );
        let mut y = area.top + 104.0;
        self.list.rounded_rect(
            Rect::new(area.left, y, area.right, y + 84.0),
            8.0,
            p.surface,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 12.0, area.right - 16.0, y + 34.0),
            "默认排序",
            TextStyle::Label,
            p.foreground,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 38.0, area.left + 180.0, y + 60.0),
            "决定全部与分组视图的排列。",
            TextStyle::Caption,
            p.muted,
        );
        let sorts = [
            ("manual", "手动"),
            ("recent", "最近"),
            ("usage", "使用次数"),
            ("created", "创建时间"),
            ("name", "名称"),
        ];
        let mut x = area.left + 202.0;
        for (value, label) in sorts {
            let rect = Rect::new(x, y + 27.0, x + 76.0, y + 59.0);
            let active = model.settings.default_sort == value;
            self.list.rounded_rect(
                rect,
                5.0,
                if active { p.accent } else { p.surface_elevated },
            );
            self.list.text(
                rect,
                label,
                TextStyle::Caption,
                if active {
                    p.accent_foreground
                } else {
                    p.foreground
                },
            );
            self.quick_nav_hits
                .push((rect, QuickNavHit::SetSort(value.into())));
            x += 84.0;
        }
        y += 98.0;
        self.list.rounded_rect(
            Rect::new(area.left, y, area.right, y + 84.0),
            8.0,
            p.surface,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 12.0, area.right - 16.0, y + 34.0),
            "网格密度",
            TextStyle::Label,
            p.foreground,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 38.0, area.left + 190.0, y + 60.0),
            "控制列数与可视行数。",
            TextStyle::Caption,
            p.muted,
        );
        for (index, (label, value, hit_minus, hit_plus)) in [
            (
                "列数",
                model.settings.grid.columns,
                QuickNavHit::SetGridColumns(-1),
                QuickNavHit::SetGridColumns(1),
            ),
            (
                "行数",
                model.settings.grid.rows,
                QuickNavHit::SetGridRows(-1),
                QuickNavHit::SetGridRows(1),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let left = area.left + 214.0 + index as f32 * 150.0;
            self.list.text(
                Rect::new(left, y + 34.0, left + 34.0, y + 56.0),
                label,
                TextStyle::Caption,
                p.muted,
            );
            let minus = Rect::new(left + 40.0, y + 28.0, left + 66.0, y + 58.0);
            let plus = Rect::new(left + 104.0, y + 28.0, left + 130.0, y + 58.0);
            self.list.rounded_rect(minus, 5.0, p.surface_elevated);
            self.list.rounded_rect(plus, 5.0, p.surface_elevated);
            self.list.text(minus, "−", TextStyle::Caption, p.foreground);
            self.list.text(
                Rect::new(left + 70.0, y + 34.0, left + 100.0, y + 56.0),
                value.to_string(),
                TextStyle::Caption,
                p.foreground,
            );
            self.list.text(plus, "＋", TextStyle::Caption, p.foreground);
            self.quick_nav_hits.push((minus, hit_minus));
            self.quick_nav_hits.push((plus, hit_plus));
        }
        y += 98.0;
        self.list.rounded_rect(
            Rect::new(area.left, y, area.right, y + 84.0),
            8.0,
            p.surface,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 12.0, area.right - 16.0, y + 34.0),
            "展示布局",
            TextStyle::Label,
            p.foreground,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 38.0, area.left + 180.0, y + 60.0),
            "网格适合图标入口，列表适合较长路径。",
            TextStyle::Caption,
            p.muted,
        );
        for (index, (value, label)) in [("grid", "网格"), ("list", "列表")].into_iter().enumerate()
        {
            let rect = Rect::new(
                area.left + 202.0 + index as f32 * 88.0,
                y + 27.0,
                area.left + 278.0 + index as f32 * 88.0,
                y + 59.0,
            );
            let active = model.settings.appearance.layout == value;
            self.list.rounded_rect(
                rect,
                5.0,
                if active { p.accent } else { p.surface_elevated },
            );
            self.list.text(
                rect,
                label,
                TextStyle::Caption,
                if active {
                    p.accent_foreground
                } else {
                    p.foreground
                },
            );
            self.quick_nav_hits
                .push((rect, QuickNavHit::SetLayout(value.into())));
        }
        y += 98.0;
        self.list.rounded_rect(
            Rect::new(area.left, y, area.right, y + 84.0),
            8.0,
            p.surface,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 12.0, area.right - 16.0, y + 34.0),
            "卡片字段",
            TextStyle::Label,
            p.foreground,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 38.0, area.left + 180.0, y + 60.0),
            "控制快捷项展示的名称、位置和备注。",
            TextStyle::Caption,
            p.muted,
        );
        for (index, (field, label, active)) in [
            ("icon", "图标", model.settings.appearance.fields.icon),
            ("name", "名称", model.settings.appearance.fields.name),
            ("target", "位置", model.settings.appearance.fields.target),
            ("note", "备注", model.settings.appearance.fields.note),
        ]
        .into_iter()
        .enumerate()
        {
            let rect = Rect::new(
                area.left + 202.0 + index as f32 * 88.0,
                y + 27.0,
                area.left + 278.0 + index as f32 * 88.0,
                y + 59.0,
            );
            self.list.rounded_rect(
                rect,
                5.0,
                if active { p.accent } else { p.surface_elevated },
            );
            self.list.text(
                rect,
                label,
                TextStyle::Caption,
                if active {
                    p.accent_foreground
                } else {
                    p.foreground
                },
            );
            self.quick_nav_hits
                .push((rect, QuickNavHit::ToggleField(field.into())));
        }
        y += 98.0;
        self.list.rounded_rect(
            Rect::new(area.left, y, area.right, y + 84.0),
            8.0,
            p.surface,
        );
        self.list.text(
            Rect::new(area.left + 16.0, y + 12.0, area.right - 16.0, y + 34.0),
            "网页浏览器",
            TextStyle::Label,
            p.foreground,
        );
        let browser_label = if model.settings.browser.mode == "custom"
            && !model.settings.browser.executable_path.is_empty()
        {
            format!("指定程序：{}", model.settings.browser.executable_path)
        } else {
            "系统默认浏览器".into()
        };
        self.list.text(
            Rect::new(area.left + 16.0, y + 40.0, area.right - 168.0, y + 63.0),
            browser_label,
            TextStyle::Caption,
            p.muted,
        );
        let configure = Rect::new(area.right - 144.0, y + 27.0, area.right - 16.0, y + 59.0);
        self.list.rounded_rect(configure, 5.0, p.surface_elevated);
        self.list
            .text(configure, "设置浏览器", TextStyle::Caption, p.foreground);
        self.quick_nav_hits
            .push((configure, QuickNavHit::ConfigureBrowser));
    }
}
