//! 从模板创建文档的模态选择器。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    theme::Palette,
};
use mochi_core::templates::{Template, UNGROUPED};

const OUTER_PADDING: f32 = 24.0;
const GRID_GAP: f32 = 12.0;
const BLANK_CARD_HEIGHT: f32 = 76.0;
const TEMPLATE_CARD_HEIGHT: f32 = 94.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Blank,
    Template(usize),
    Cancel,
    /// 点击在弹窗内的留白处不应关闭弹窗。
    Inside,
    Outside,
}

pub struct GroupLayout {
    pub name: String,
    pub heading: Rect,
    pub cards: Vec<(Rect, usize)>,
}

#[derive(Default)]
pub struct Layout {
    pub panel: Rect,
    pub content: Rect,
    pub blank: Rect,
    pub template_divider_y: f32,
    pub groups: Vec<GroupLayout>,
    pub cancel: Rect,
    pub close: Rect,
    pub rows: Vec<(Rect, Hit)>,
    pub max_scroll: f32,
}

pub struct Picker {
    pub parent: std::path::PathBuf,
    pub templates: Vec<Template>,
    pub scroll: f32,
    pub hover: Option<Hit>,
    pub focused: Option<Hit>,
}

impl Picker {
    pub fn new(parent: std::path::PathBuf, templates: Vec<Template>) -> Self {
        Self {
            parent,
            templates,
            scroll: 0.0,
            hover: None,
            focused: None,
        }
    }

    pub fn layout(&self, viewport: Rect) -> Layout {
        let width = (viewport.width() - 40.0).max(0.0).min(700.0);
        let height = (viewport.height() - 40.0).max(0.0).min(580.0);
        let panel = Rect::from_size(
            (viewport.left + viewport.right - width) / 2.0,
            (viewport.top + viewport.bottom - height) / 2.0,
            width,
            height,
        );
        let content = Rect::new(
            panel.left + OUTER_PADDING,
            panel.top + 92.0,
            panel.right - OUTER_PADDING,
            panel.bottom - 58.0,
        );
        let groups = self.grouped_templates();
        let grid_columns = if content.width() >= 460.0 { 2usize } else { 1 };

        // 首先用未滚动的坐标测量总高度，之后再统一减去 scroll。
        let mut cursor = BLANK_CARD_HEIGHT + 34.0;
        for (_, indices) in &groups {
            cursor += 22.0; // 小号分组标题
            let row_count = indices.len().div_ceil(grid_columns);
            cursor += row_count as f32 * TEMPLATE_CARD_HEIGHT;
            cursor += row_count.saturating_sub(1) as f32 * GRID_GAP;
            cursor += 16.0;
        }
        let content_height = cursor.max(BLANK_CARD_HEIGHT);
        let max_scroll = (content_height - content.height()).max(0.0);
        let scroll = self.scroll.clamp(0.0, max_scroll);

        let blank = Rect::new(
            content.left,
            content.top - scroll,
            content.right,
            content.top + BLANK_CARD_HEIGHT - scroll,
        );
        let template_divider_y = content.top + BLANK_CARD_HEIGHT + 20.0 - scroll;
        let mut cursor = content.top + BLANK_CARD_HEIGHT + 34.0 - scroll;
        let card_width =
            (content.width() - GRID_GAP * (grid_columns - 1) as f32) / grid_columns as f32;
        let mut group_layouts = Vec::with_capacity(groups.len());
        let mut rows = vec![(blank, Hit::Blank)];

        for (name, indices) in groups {
            let heading = Rect::new(content.left, cursor, content.right, cursor + 20.0);
            cursor += 22.0;
            let mut cards = Vec::with_capacity(indices.len());
            for (offset, index) in indices.into_iter().enumerate() {
                let row = offset / grid_columns;
                let column = offset % grid_columns;
                let left = content.left + column as f32 * (card_width + GRID_GAP);
                let top = cursor + row as f32 * (TEMPLATE_CARD_HEIGHT + GRID_GAP);
                let card = Rect::from_size(left, top, card_width, TEMPLATE_CARD_HEIGHT);
                cards.push((card, index));
                rows.push((card, Hit::Template(index)));
            }
            let row_count = cards.len().div_ceil(grid_columns);
            cursor += row_count as f32 * TEMPLATE_CARD_HEIGHT;
            cursor += row_count.saturating_sub(1) as f32 * GRID_GAP;
            cursor += 16.0;
            group_layouts.push(GroupLayout {
                name,
                heading,
                cards,
            });
        }

        let cancel = Rect::new(
            panel.right - 118.0,
            panel.bottom - 46.0,
            panel.right - OUTER_PADDING,
            panel.bottom - 16.0,
        );
        rows.push((cancel, Hit::Cancel));
        let close = Rect::from_size(panel.right - 48.0, panel.top + 18.0, 32.0, 32.0);
        Layout {
            panel,
            content,
            blank,
            template_divider_y,
            groups: group_layouts,
            cancel,
            close,
            rows,
            max_scroll,
        }
    }

    pub fn hit(&self, viewport: Rect, x: f32, y: f32) -> Hit {
        let layout = self.layout(viewport);
        if !layout.panel.contains(x, y) {
            return Hit::Outside;
        }
        if layout.cancel.contains(x, y) || layout.close.contains(x, y) {
            return Hit::Cancel;
        }
        if layout.content.contains(x, y) {
            return layout
                .rows
                .iter()
                .find(|(rect, hit)| !matches!(hit, Hit::Cancel) && rect.contains(x, y))
                .map(|(_, hit)| *hit)
                .unwrap_or(Hit::Inside);
        }
        Hit::Inside
    }

    pub fn paint(&self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        let layout = self.layout(viewport);
        list.rect_alpha(viewport, 0x000000, 0.42);
        list.rounded_rect(layout.panel, 10.0, p.surface);
        list.rounded_border(layout.panel, 10.0, p.border);
        list.text(
            Rect::new(
                layout.panel.left + OUTER_PADDING,
                layout.panel.top + 22.0,
                layout.close.left - 12.0,
                layout.panel.top + 50.0,
            ),
            "新建文档",
            TextStyle::Large,
            p.foreground,
        );
        list.text(
            Rect::new(
                layout.panel.left + OUTER_PADDING,
                layout.panel.top + 54.0,
                layout.panel.right - OUTER_PADDING,
                layout.panel.top + 76.0,
            ),
            if layout.content.width() < 420.0 {
                "选择模板创建副本，原模板会保留。"
            } else {
                "从空白开始，或选择一个模板。原模板会保留。"
            },
            TextStyle::Small,
            p.muted,
        );
        if self.hover == Some(Hit::Cancel) {
            list.rounded_rect(layout.close, 6.0, p.surface_muted);
        }
        list.icon_centered(layout.close, Icon::X, 16.0, p.muted);

        list.push_clip(layout.content);
        self.paint_blank_card(list, layout.blank, p);
        list.hline(
            layout.content.left,
            layout.content.right,
            layout.template_divider_y,
            p.border,
        );
        for group in &layout.groups {
            list.text(group.heading, group.name.clone(), TextStyle::Small, p.muted);
            for (card, index) in &group.cards {
                if card.bottom < layout.content.top || card.top > layout.content.bottom {
                    continue;
                }
                self.paint_template_card(list, *card, &self.templates[*index], p);
                self.paint_feedback(list, *card, Hit::Template(*index), p);
            }
        }
        if self.templates.is_empty() {
            list.text(
                Rect::new(
                    layout.content.left,
                    layout.template_divider_y + 22.0,
                    layout.content.right,
                    layout.template_divider_y + 46.0,
                ),
                "还没有模板，可以先从空白文档开始。",
                TextStyle::Label,
                p.muted,
            );
        }
        list.pop_clip();

        if layout.max_scroll > 0.0 {
            let track = layout.content.height();
            let height = (track * track / (track + layout.max_scroll))
                .max(28.0)
                .min(track);
            let top = layout.content.top
                + (track - height) * self.scroll.clamp(0.0, layout.max_scroll) / layout.max_scroll;
            list.rounded_rect(
                Rect::from_size(layout.content.right + 9.0, top, 3.0, height),
                1.5,
                p.muted,
            );
        }

        list.hline(
            layout.panel.left,
            layout.panel.right,
            layout.content.bottom + 12.0,
            p.border,
        );
        list.text(
            Rect::new(
                layout.content.left,
                layout.cancel.top,
                layout.cancel.left - 12.0,
                layout.cancel.bottom,
            ),
            if layout.content.width() < 420.0 {
                format!("{} 个模板", self.templates.len())
            } else {
                format!("{} 个模板 · Tab 切换 · Enter 创建", self.templates.len())
            },
            TextStyle::Caption,
            p.muted,
        );
        super::workspace_ui::button(
            list,
            layout.cancel,
            "取消",
            None,
            false,
            self.hover == Some(Hit::Cancel),
            p,
        );
        if self.focused == Some(Hit::Cancel) {
            list.rounded_border(layout.cancel, 7.0, p.accent);
        }
    }

    fn grouped_templates(&self) -> Vec<(String, Vec<usize>)> {
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for (index, template) in self.templates.iter().enumerate() {
            let group = if template.group.trim().is_empty() {
                UNGROUPED.to_owned()
            } else {
                template.group.clone()
            };
            if let Some((_, indices)) = groups.iter_mut().find(|(name, _)| *name == group) {
                indices.push(index);
            } else {
                groups.push((group, vec![index]));
            }
        }
        groups
    }

    fn paint_blank_card(&self, list: &mut DrawList, rect: Rect, p: &Palette) {
        list.rounded_rect(
            rect,
            7.0,
            super::theme::mix(p.surface_muted, p.surface, 0.5),
        );
        self.paint_feedback(list, rect, Hit::Blank, p);
        list.icon_centered(
            Rect::new(
                rect.left + 18.0,
                rect.top + 18.0,
                rect.left + 42.0,
                rect.bottom - 18.0,
            ),
            Icon::FILE,
            18.0,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 58.0,
                rect.top + 14.0,
                rect.right - 44.0,
                rect.top + 42.0,
            ),
            "从空白文档开始",
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 58.0,
                rect.top + 40.0,
                rect.right - 44.0,
                rect.bottom - 10.0,
            ),
            "自由书写，不使用模板",
            TextStyle::Small,
            p.muted,
        );
        list.icon_centered(
            Rect::new(rect.right - 38.0, rect.top, rect.right - 14.0, rect.bottom),
            Icon::ARROW_RIGHT,
            16.0,
            p.foreground,
        );
    }

    fn paint_template_card(
        &self,
        list: &mut DrawList,
        rect: Rect,
        template: &Template,
        p: &Palette,
    ) {
        list.rounded_border(rect, 7.0, p.border);
        list.icon_centered(
            Rect::new(
                rect.left + 16.0,
                rect.top + 18.0,
                rect.left + 40.0,
                rect.top + 42.0,
            ),
            Icon::FILE_TEXT,
            18.0,
            p.muted,
        );
        list.text(
            Rect::new(
                rect.left + 50.0,
                rect.top + 10.0,
                rect.right - 14.0,
                rect.top + 34.0,
            ),
            super::text::ellipsize(
                &template.name,
                TextStyle::Label,
                (rect.width() - 64.0).max(0.0),
            ),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 50.0,
                rect.top + 36.0,
                rect.right - 14.0,
                rect.top + 56.0,
            ),
            super::text::ellipsize(
                template.description(),
                TextStyle::Caption,
                (rect.width() - 64.0).max(0.0),
            ),
            TextStyle::Caption,
            p.muted,
        );
        list.text(
            Rect::new(
                rect.left + 50.0,
                rect.top + 62.0,
                rect.right - 14.0,
                rect.top + 82.0,
            ),
            template.format_label(),
            TextStyle::Small,
            p.muted,
        );
    }

    fn paint_feedback(&self, list: &mut DrawList, rect: Rect, hit: Hit, p: &Palette) {
        if self.hover == Some(hit) || self.focused == Some(hit) {
            list.rounded_border(
                rect,
                7.0,
                if self.focused == Some(hit) {
                    p.accent
                } else {
                    p.muted
                },
            );
        }
    }

    pub fn navigate(&mut self, viewport: Rect, backwards: bool) {
        let layout = self.layout(viewport);
        let count = layout.rows.len();
        let current = self
            .focused
            .and_then(|hit| layout.rows.iter().position(|(_, h)| *h == hit));
        let next = match current {
            Some(i) if backwards => (i + count - 1) % count,
            Some(i) => (i + 1) % count,
            None if backwards => count - 1,
            None => 0,
        };
        let (r, hit) = layout.rows[next];
        self.focused = Some(hit);
        if hit != Hit::Cancel {
            if r.top < layout.content.top {
                self.scroll -= layout.content.top - r.top;
            }
            if r.bottom > layout.content.bottom {
                self.scroll += r.bottom - layout.content.bottom;
            }
            self.scroll = self.scroll.clamp(0.0, layout.max_scroll);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn template(name: &str, group: &str) -> Template {
        Template {
            name: name.into(),
            group: group.into(),
            path: PathBuf::from(format!("{name}.md")),
        }
    }

    #[test]
    fn templates_are_grouped_into_two_column_cards() {
        let picker = Picker::new(
            PathBuf::new(),
            vec![
                template("读书笔记", "学习"),
                template("学习笔记", "学习"),
                template("会议纪要", "工作"),
            ],
        );
        let layout = picker.layout(Rect::from_size(0.0, 0.0, 1280.0, 800.0));

        assert_eq!(layout.groups.len(), 2);
        assert_eq!(layout.groups[0].name, "学习");
        assert_eq!(layout.groups[0].cards.len(), 2);
        assert!(layout.groups[0].cards[0].0.left < layout.groups[0].cards[1].0.left);
        assert_eq!(
            layout.groups[0].cards[0].0.top,
            layout.groups[0].cards[1].0.top
        );
        assert!(layout.template_divider_y < layout.groups[0].heading.top);
    }

    #[test]
    fn unused_dialog_space_does_not_dismiss_the_picker() {
        let picker = Picker::new(PathBuf::new(), vec![template("日计划", "日常")]);
        let viewport = Rect::from_size(0.0, 0.0, 1280.0, 800.0);
        let layout = picker.layout(viewport);

        assert_eq!(
            picker.hit(viewport, layout.panel.left + 4.0, layout.panel.top + 4.0),
            Hit::Inside
        );
    }

    #[test]
    fn starter_templates_scroll_to_the_last_category() {
        let mut picker = Picker::new(
            PathBuf::new(),
            vec![
                template("读书笔记", "学习"),
                template("学习笔记", "学习"),
                template("会议纪要", "工作"),
                template("项目计划", "工作"),
                template("日记", "日常"),
                template("周计划", "日常"),
            ],
        );
        let viewport = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
        picker.scroll = picker.layout(viewport).max_scroll;
        let lay = picker.layout(viewport);
        let last = lay.groups.last().unwrap().cards.last().unwrap().0;
        assert!(last.bottom <= lay.content.bottom);
        assert!(last.top >= lay.content.top);
    }
}
