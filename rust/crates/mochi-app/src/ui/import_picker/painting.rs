//! 绘制导入来源选择界面。
use super::*;
use crate::ui::draw::{Align, TextStyle};

impl State {
    pub fn paint(&self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        let layout = self.layout(viewport);
        let panel = layout.panel;
        let title_extra = large_title_extra();
        list.rect_alpha(viewport, 0x000000, 0.42);
        list.rounded_rect(panel, 10.0, p.surface);
        list.rounded_border(panel, 10.0, p.border);
        list.text(
            Rect::new(
                panel.left + 24.0,
                panel.top + 20.0,
                layout.close.left - 8.0,
                panel.top + 48.0 + title_extra,
            ),
            "导入文件",
            TextStyle::Large,
            p.foreground,
        );
        list.text(
            Rect::new(
                panel.left + 24.0,
                panel.top + 54.0 + title_extra,
                panel.right - 24.0,
                panel.top + 78.0 + title_extra,
            ),
            &self.summary,
            TextStyle::Small,
            p.muted,
        );
        list.text(
            Rect::new(
                panel.left + 24.0,
                panel.top + 86.0 + title_extra,
                panel.right - 24.0,
                panel.top + 108.0 + title_extra,
            ),
            "选择保存位置",
            TextStyle::Label,
            p.foreground,
        );
        list.icon_centered(layout.close, Icon::X, 16.0, p.muted);
        list.rounded_border(layout.tree, 6.0, p.border);
        list.push_clip(layout.tree);
        for (index, folder) in self.folders.iter().enumerate() {
            let row = self.row_rect(&layout, index);
            if row.bottom <= layout.tree.top || row.top >= layout.tree.bottom {
                continue;
            }
            let selected = Some(&folder.path) == self.selected.as_ref();
            if selected {
                list.rounded_rect_alpha(row, 4.0, p.accent, 0.12);
            } else if matches!(self.hover, Some(Hit::Select(i) | Hit::Toggle(i)) if i == index) {
                list.rounded_rect(row, 4.0, p.surface_muted);
            }
            if selected && self.focus == Focus::Tree {
                list.rounded_border(row, 4.0, p.accent);
            }
            let toggle = self.toggle_rect(&layout, index);
            if folder.expandable {
                list.icon_centered(
                    toggle,
                    if folder.expanded {
                        Icon::CHEVRON_DOWN
                    } else {
                        Icon::CHEVRON_RIGHT
                    },
                    14.0,
                    p.muted,
                );
            }
            let icon = Rect::from_size(toggle.right + 2.0, row.top + 7.0, 18.0, 18.0);
            list.icon_centered(
                icon,
                if folder.expanded {
                    Icon::FOLDER_OPEN
                } else {
                    Icon::FOLDER
                },
                17.0,
                if selected { p.accent } else { p.muted },
            );
            list.text(
                Rect::new(
                    icon.right + 8.0,
                    row.top + 3.0,
                    row.right - 30.0,
                    row.bottom - 3.0,
                ),
                &folder.name,
                TextStyle::Label,
                p.foreground,
            );
            if selected {
                list.icon_centered(
                    Rect::from_size(row.right - 26.0, row.top + 7.0, 18.0, 18.0),
                    Icon::CHECK,
                    16.0,
                    p.accent,
                );
            }
        }
        if self.folders.is_empty() {
            list.text(
                Rect::new(
                    layout.tree.left + 16.0,
                    layout.tree.top + 12.0,
                    layout.tree.right - 16.0,
                    layout.tree.top + 42.0,
                ),
                "暂无可用的知识库，请先创建知识库。",
                TextStyle::Small,
                p.muted,
            );
        }
        list.pop_clip();
        if layout.max_scroll > 0.0 {
            let height = (layout.tree.height().powi(2)
                / (layout.tree.height() + layout.max_scroll))
                .max(24.0)
                .min(layout.tree.height());
            let top = layout.tree.top
                + (layout.tree.height() - height) * self.scroll.clamp(0.0, layout.max_scroll)
                    / layout.max_scroll;
            list.rounded_rect(
                Rect::from_size(layout.tree.right + 6.0, top, 3.0, height),
                1.5,
                p.muted,
            );
        }
        let target = self.destination_label();
        list.text(
            Rect::new(
                panel.left + 24.0,
                panel.bottom - 138.0,
                panel.right - 24.0,
                panel.bottom - 114.0,
            ),
            format!("保存到：{target}"),
            TextStyle::Small,
            p.foreground,
        );
        list.text(
            Rect::new(
                panel.left + 24.0,
                panel.bottom - 112.0,
                panel.right - 24.0,
                panel.bottom - 88.0,
            ),
            if self.error.is_empty() {
                "保留源文件，同名文件自动保留副本。"
            } else {
                &self.error
            },
            TextStyle::Small,
            if self.error.is_empty() {
                p.muted
            } else {
                p.accent
            },
        );
        for (rect, hit, focus, label) in [
            (layout.cancel, Hit::Cancel, Focus::Cancel, "取消"),
            (layout.confirm, Hit::Confirm, Focus::Confirm, "导入到此处"),
        ] {
            let primary = hit == Hit::Confirm;
            let enabled = !primary || self.selected.is_some();
            let start = list.cmds().len();
            if primary {
                list.glass_button(rect, 6.0, p, enabled && self.hover == Some(hit));
            } else {
                list.rounded_rect(rect, 6.0, p.surface_muted);
            }
            if self.focus == focus || self.hover == Some(hit) {
                list.rounded_border(rect, 6.0, p.accent);
            }
            list.text_aligned(
                rect,
                label,
                TextStyle::Label,
                if primary {
                    p.button_foreground()
                } else if enabled {
                    p.foreground
                } else {
                    p.muted
                },
                Align::Center,
            );
            if !enabled {
                list.fade_since(start, rect, 0.45);
            }
        }
    }
}
