//! 文档起始模板支持按场景或格式筛选，也可分别复制。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::{self, Palette},
    workspace_ui,
};
use mochi_core::templates::Template;

fn page_title_height() -> f32 {
    TextStyle::Display.line_height().max(36.0)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    #[default]
    All,
    Mochi,
    Markdown,
    Text,
}
impl Format {
    const ALL: [Self; 4] = [Self::All, Self::Mochi, Self::Markdown, Self::Text];
    fn label(self) -> &'static str {
        match self {
            Self::All => "全部格式",
            Self::Mochi => "墨池文档",
            Self::Markdown => "Markdown",
            Self::Text => "纯文本",
        }
    }
    fn matches(self, template: &Template) -> bool {
        self == Self::All || template.format_label() == self.label()
    }
}

#[derive(Default)]
pub struct State {
    pub items: Vec<Template>,
    pub groups: Vec<String>,
    pub selected_group: String,
    pub format: Format,
    pub error: String,
    pub scroll: f32,
    pub hover: Option<Hit>,
}
impl State {
    fn filtered(&self) -> impl Iterator<Item = (usize, &Template)> {
        self.items.iter().enumerate().filter(|(_, item)| {
            (self.selected_group.is_empty() || item.group == self.selected_group)
                && self.format.matches(item)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    New,
    NewGroup,
    AllGroups,
    Group(usize),
    Format(Format),
    Template(usize),
    Use(usize),
    Delete(usize),
    Blank,
}
#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub list: Rect,
    pub max_scroll: f32,
    pub content: Rect,
    pub rows_top: f32,
    pub visible_count: usize,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, h)| {
                r.contains(x, y)
                    && (matches!(h, Hit::New | Hit::NewGroup | Hit::Blank)
                        || self.list.contains(x, y))
            })
            .map(|(_, h)| *h)
    }
}

fn chip(out: &mut Layout, x: &mut f32, y: &mut f32, label: &str, hit: Hit) {
    let width = (text::measure(label, TextStyle::Label) + 28.0)
        .max(72.0)
        .min(out.content.width());
    if *x > out.content.left && *x + width > out.content.right {
        *x = out.content.left;
        *y += 40.0;
    }
    out.entries
        .push((Rect::from_size(*x, *y, width, 32.0), hit));
    *x += width + 8.0;
}

pub fn layout(state: &State, area: Rect) -> Layout {
    let c = workspace_ui::column(area, 1120.0);
    let title_extra = page_title_height() - 36.0;
    let mut out = Layout {
        content: c,
        ..Default::default()
    };
    out.entries.push((area, Hit::Blank));
    out.entries.push((
        Rect::from_size(c.left, c.top + 78.0 + title_extra, 120.0, 36.0),
        Hit::New,
    ));
    out.entries.push((
        Rect::from_size(c.left + 132.0, c.top + 78.0 + title_extra, 112.0, 36.0),
        Hit::NewGroup,
    ));
    out.list = Rect::new(
        c.left,
        c.top + 134.0 + title_extra,
        c.right,
        c.bottom - if state.error.is_empty() { 0.0 } else { 36.0 },
    );
    let mut x = c.left;
    let mut y = out.list.top;
    chip(&mut out, &mut x, &mut y, "全部场景", Hit::AllGroups);
    for (i, group) in state.groups.iter().enumerate() {
        chip(&mut out, &mut x, &mut y, group, Hit::Group(i));
    }
    x = c.left;
    y += 44.0;
    for format in Format::ALL {
        chip(
            &mut out,
            &mut x,
            &mut y,
            format.label(),
            Hit::Format(format),
        );
    }
    y += 56.0;
    out.rows_top = y;
    const GAP: f32 = 14.0;
    const HEIGHT: f32 = 232.0;
    let columns = if c.width() >= 900.0 {
        3
    } else if c.width() >= 570.0 {
        2
    } else {
        1
    };
    let width = (c.width() - GAP * (columns - 1) as f32) / columns as f32;
    for (offset, (i, _)) in state.filtered().enumerate() {
        let r = Rect::from_size(
            c.left + (offset % columns) as f32 * (width + GAP),
            y + (offset / columns) as f32 * (HEIGHT + GAP),
            width,
            HEIGHT,
        );
        out.entries.push((r, Hit::Template(i)));
        out.entries.push((
            Rect::from_size(r.left + 16.0, r.bottom - 48.0, 100.0, 32.0),
            Hit::Use(i),
        ));
        out.entries.push((
            Rect::from_size(r.right - 44.0, r.bottom - 48.0, 28.0, 32.0),
            Hit::Delete(i),
        ));
        out.visible_count += 1;
    }
    let rows = out.visible_count.div_ceil(columns);
    let height = if rows == 0 {
        150.0
    } else {
        rows as f32 * (HEIGHT + GAP) - GAP
    };
    out.max_scroll = (y + height - out.list.bottom).max(0.0);
    let scroll = state.scroll.clamp(0.0, out.max_scroll);
    out.rows_top -= scroll;
    for (r, hit) in &mut out.entries {
        if !matches!(hit, Hit::New | Hit::NewGroup | Hit::Blank) {
            r.top -= scroll;
            r.bottom -= scroll;
        }
    }
    out
}

pub fn paint(list: &mut DrawList, area: Rect, state: &State, lay: &Layout, p: &Palette) {
    list.push_clip(area);
    list.rect(area, p.area_main_default);
    let c = lay.content;
    let title_height = page_title_height();
    list.text(
        Rect::new(c.left, c.top, c.right, c.top + title_height),
        "模板中心",
        TextStyle::Display,
        p.foreground,
    );
    list.text(
        Rect::new(
            c.left,
            c.top + title_height + 8.0,
            c.right,
            c.top + title_height + 28.0,
        ),
        text::ellipsize(
            &format!(
                "{} 个模板 · 选择适合的结构，从一份新文档开始。",
                state.items.len()
            ),
            TextStyle::Caption,
            c.width(),
        ),
        TextStyle::Caption,
        p.muted,
    );
    for (r, h) in &lay.entries {
        match h {
            Hit::New => workspace_ui::button(
                list,
                *r,
                "新建模板",
                Some(Icon::PLUS),
                true,
                state.hover == Some(*h),
                p,
            ),
            Hit::NewGroup => workspace_ui::button(
                list,
                *r,
                "新建分组",
                None,
                false,
                state.hover == Some(*h),
                p,
            ),
            _ => {}
        }
    }
    list.push_clip(lay.list);
    for (r, h) in &lay.entries {
        if r.bottom < lay.list.top || r.top > lay.list.bottom {
            continue;
        }
        match *h {
            Hit::AllGroups => workspace_ui::tab(
                list,
                *r,
                "全部场景",
                state.selected_group.is_empty(),
                state.hover == Some(*h),
                p,
            ),
            Hit::Group(i) => workspace_ui::tab(
                list,
                *r,
                &state.groups[i],
                state.groups[i] == state.selected_group,
                state.hover == Some(*h),
                p,
            ),
            Hit::Format(format) => workspace_ui::tab(
                list,
                *r,
                format.label(),
                state.format == format,
                state.hover == Some(*h),
                p,
            ),
            Hit::Template(i) => paint_card(
                list,
                *r,
                &state.items[i],
                matches!(state.hover, Some(Hit::Template(j) | Hit::Use(j) | Hit::Delete(j)) if i == j),
                p,
            ),
            Hit::Use(_) => workspace_ui::button(
                list,
                *r,
                "使用模板",
                None,
                false,
                state.hover == Some(*h),
                p,
            ),
            Hit::Delete(_) => {
                if state.hover == Some(*h) {
                    list.rounded_rect(*r, 6.0, theme::mix(p.danger, p.surface, 0.08));
                }
                list.icon_centered(
                    *r,
                    Icon::TRASH2,
                    15.0,
                    if state.hover == Some(*h) {
                        p.danger
                    } else {
                        p.muted
                    },
                );
            }
            _ => {}
        }
    }
    if lay.visible_count == 0 {
        list.text(
            Rect::new(
                c.left + 12.0,
                lay.rows_top + 24.0,
                c.right - 12.0,
                lay.rows_top + 52.0,
            ),
            if state.items.is_empty() {
                "还没有模板"
            } else {
                "当前筛选下没有模板"
            },
            TextStyle::Large,
            p.foreground,
        );
        list.text(
            Rect::new(
                c.left + 12.0,
                lay.rows_top + 62.0,
                c.right - 12.0,
                lay.rows_top + 90.0,
            ),
            text::ellipsize(
                if state.items.is_empty() {
                    "新建模板，或在文档菜单中选择“另存为模板”。"
                } else {
                    "试试全部场景、全部格式，或新建自己的模板。"
                },
                TextStyle::Caption,
                c.width() - 24.0,
            ),
            TextStyle::Caption,
            p.muted,
        );
    }
    list.pop_clip();
    if lay.max_scroll > 0.0 && lay.list.height() > 0.0 {
        let track = lay.list.height();
        let height = (track * track / (track + lay.max_scroll))
            .max(28.0)
            .min(track);
        let top = lay.list.top
            + (track - height) * state.scroll.clamp(0.0, lay.max_scroll) / lay.max_scroll;
        list.rounded_rect(
            Rect::from_size(lay.list.right + 10.0, top, 3.0, height),
            1.5,
            p.muted,
        );
    }
    if !state.error.is_empty() {
        list.text(
            Rect::new(c.left, c.bottom - 28.0, c.right, c.bottom),
            text::ellipsize(&state.error, TextStyle::Label, c.width()),
            TextStyle::Label,
            p.danger,
        );
    }
    list.pop_clip();
}

fn paint_card(list: &mut DrawList, r: Rect, template: &Template, hovered: bool, p: &Palette) {
    list.rounded_rect(
        r,
        9.0,
        if hovered {
            theme::mix(p.surface_muted, p.surface, 0.3)
        } else {
            p.surface
        },
    );
    list.rounded_border(
        r,
        9.0,
        if hovered {
            theme::mix(p.muted, p.border, 0.35)
        } else {
            p.border
        },
    );
    let left = r.left + 16.0;
    let right = r.right - 16.0;
    let icon = match template.group.as_str() {
        "日常" => Icon::CALENDAR,
        "学习" | "研究" => Icon::BOOK_OPEN,
        "开发" => Icon::CODE2,
        "写作" => Icon::PEN_LINE,
        _ => Icon::FILE_TEXT,
    };
    list.icon_centered(
        Rect::from_size(left, r.top + 16.0, 24.0, 24.0),
        icon,
        18.0,
        p.muted,
    );
    list.text(
        Rect::new(left + 32.0, r.top + 16.0, right, r.top + 40.0),
        text::ellipsize(
            &format!("{} · {}", template.group, template.format_label()),
            TextStyle::Caption,
            r.width() - 64.0,
        ),
        TextStyle::Caption,
        p.muted,
    );
    list.text(
        Rect::new(
            left,
            r.top + 48.0,
            right,
            r.top + 48.0 + TextStyle::Large.line_height().max(28.0),
        ),
        text::ellipsize(&template.name, TextStyle::Large, right - left),
        TextStyle::Large,
        p.foreground,
    );
    let title_extra = (TextStyle::Large.line_height() - 28.0).max(0.0);
    for (i, line) in text::wrap_source(template.description(), TextStyle::Caption, right - left)
        .into_iter()
        .take(2)
        .enumerate()
    {
        let line: String = line.into_iter().map(|run| run.text).collect();
        list.text(
            Rect::from_size(
                left,
                r.top + 83.0 + title_extra + i as f32 * 21.0,
                right - left,
                21.0,
            ),
            line,
            TextStyle::Caption,
            p.muted,
        );
    }
    let features = template
        .builtin()
        .map(|b| b.features.join(" · "))
        .unwrap_or_else(|| "自定义内容 · 可重复使用".into());
    list.text(
        Rect::new(left, r.top + 137.0, right, r.top + 160.0),
        text::ellipsize(&features, TextStyle::Small, right - left),
        TextStyle::Small,
        p.muted,
    );
    list.hline(left, right, r.bottom - 61.0, p.border);
    list.text(
        Rect::new(left + 112.0, r.bottom - 48.0, right - 36.0, r.bottom - 16.0),
        "查看 / 编辑",
        TextStyle::Caption,
        p.muted,
    );
}

#[cfg(test)]
mod tests;
