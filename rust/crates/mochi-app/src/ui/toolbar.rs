//! 定义工具栏的高度、间距和按钮布局尺寸。
use super::draw::{DrawList, TextStyle};
use super::format::{self, Format};
use super::icons::Icon;
use super::layout::Rect;
use super::text;
use super::theme::{self, Palette};

pub const HEIGHT: f32 = 41.0;
const PAD_X: f32 = 20.0;
const PAD_Y: f32 = 6.0;
const BUTTON: f32 = 28.0;
const GAP: f32 = 2.0;
const GROUP_PAD: f32 = 12.0;
const GROUP_MARGIN: f32 = 4.0;
const ICON: f32 = 16.0;
const HEADING_MIN_WIDTH: f32 = 76.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hit {
    /// 「+」插入菜单。
    Insert,
    Format(Format),
    /// 段落格式下拉。
    HeadingPicker,
    ImportMarkdown,
    ExportMarkdown,
    Color(bool),
    Align(super::draw::Align),
}

/// 一组按钮。`None` 表示组尾分隔。
enum Item {
    Button { icon: Icon, hit: Hit },
    Heading,
    Separator,
}

fn items() -> Vec<Item> {
    use Item::*;
    vec![
        Button {
            icon: Icon::PLUS,
            hit: Hit::Insert,
        },
        Separator,
        Button {
            icon: Icon::BOLD,
            hit: Hit::Format(Format::Bold),
        },
        Button {
            icon: Icon::ITALIC,
            hit: Hit::Format(Format::Italic),
        },
        Button {
            icon: Icon::STRIKETHROUGH,
            hit: Hit::Format(Format::Strike),
        },
        Button {
            icon: Icon::UNDERLINE,
            hit: Hit::Format(Format::Underline),
        },
        Button {
            icon: Icon::CODE,
            hit: Hit::Format(Format::Code),
        },
        Button {
            icon: Icon::REMOVE_FORMATTING,
            hit: Hit::Format(Format::ClearFormat),
        },
        Separator,
        Heading,
        Separator,
        Button {
            icon: Icon::LIST,
            hit: Hit::Format(Format::BulletList),
        },
        Button {
            icon: Icon::LIST_ORDERED,
            hit: Hit::Format(Format::OrderedList),
        },
        Button {
            icon: Icon::CHECK_SQUARE,
            hit: Hit::Format(Format::TaskList),
        },
        Separator,
        Button {
            icon: Icon::TABLE,
            hit: Hit::Format(Format::Table),
        },
        Button {
            icon: Icon::IMAGE,
            hit: Hit::Format(Format::Image),
        },
        Button {
            icon: Icon::LINK,
            hit: Hit::Format(Format::Link),
        },
        Separator,
        Button {
            icon: Icon::CODE2,
            hit: Hit::Format(Format::CodeBlock),
        },
        Button {
            icon: Icon::QUOTE,
            hit: Hit::Format(Format::Quote),
        },
        Button {
            icon: Icon::MINUS,
            hit: Hit::Format(Format::Rule),
        },
        Separator,
        Button {
            icon: Icon::PALETTE,
            hit: Hit::Color(false),
        },
        Button {
            icon: Icon::PENCIL_LINE,
            hit: Hit::Color(true),
        },
        Separator,
        Button {
            icon: Icon::ALIGN_LEFT,
            hit: Hit::Align(super::draw::Align::Leading),
        },
        Button {
            icon: Icon::ALIGN_CENTER,
            hit: Hit::Align(super::draw::Align::Center),
        },
        Button {
            icon: Icon::ALIGN_RIGHT,
            hit: Hit::Align(super::draw::Align::Trailing),
        },
        Separator,
        Button {
            icon: Icon::UPLOAD,
            hit: Hit::ImportMarkdown,
        },
        Button {
            icon: Icon::DOWNLOAD,
            hit: Hit::ExportMarkdown,
        },
    ]
}

/// 段落格式选项：(级别, 标签, 图标)。与 `headingOptions` 一致。
pub const HEADING_OPTIONS: &[(u8, &str, Icon)] = &[
    (0, "正文", Icon::PILCROW),
    (1, "标题 1", Icon::HEADING1),
    (2, "标题 2", Icon::HEADING2),
    (3, "标题 3", Icon::HEADING3),
    (4, "标题 4", Icon::HEADING4),
    (5, "标题 5", Icon::HEADING5),
    (6, "标题 6", Icon::HEADING6),
];

/// 光标处的状态：哪些按钮该亮。
#[derive(Debug, Clone, Default)]
pub struct State {
    pub heading_level: u8,
    pub active: Vec<Format>,
    pub hover: Option<Hit>,
    pub insert_open: bool,
    pub heading_open: bool,
}

impl State {
    /// 从源码与光标算出来。
    pub fn at(text: &str, cursor: usize) -> State {
        let mut active = Vec::new();
        for f in [
            Format::BulletList,
            Format::OrderedList,
            Format::TaskList,
            Format::Quote,
        ] {
            if format::line_has(text, cursor, f) {
                active.push(f);
            }
        }
        let (start, end) = format::line_range(text, cursor);
        for span in text::parse_inline_spans(&text[start..end]) {
            if start + span.start <= cursor && cursor < start + span.end {
                let flags = match span.run.emphasis {
                    text::Emphasis::Bold => 1,
                    text::Emphasis::Italic => 2,
                    text::Emphasis::BoldItalic => 3,
                    text::Emphasis::Code => 16,
                    text::Emphasis::Styled { flags, .. } => flags,
                    _ => 0,
                };
                for (flag, f) in [
                    (1, Format::Bold),
                    (2, Format::Italic),
                    (4, Format::Underline),
                    (8, Format::Strike),
                    (16, Format::Code),
                ] {
                    if flags & flag != 0 && !active.contains(&f) {
                        active.push(f);
                    }
                }
            }
        }
        State {
            heading_level: format::heading_level_at(text, cursor),
            active,
            ..State::default()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub separators: Vec<Rect>,
    /// 段落格式按钮的矩形（下拉菜单从这里弹出）。
    pub heading_rect: Rect,
    pub insert_rect: Rect,
    /// 工具栏总高。窄窗口下按钮换行（`flex-wrap`），高度随之增加。
    pub height: f32,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }
}

/// 段落格式按钮的宽度：图标 + 文字 + 箭头，不小于 `min-w-[76px]`。
fn heading_button_width(level: u8) -> f32 {
    let label = HEADING_OPTIONS
        .iter()
        .find(|(l, _, _)| *l == level)
        .map(|(_, s, _)| *s)
        .unwrap_or("正文");
    (8.0 + ICON + 6.0 + text::measure(label, TextStyle::Label) + 6.0 + 13.0 + 8.0)
        .max(HEADING_MIN_WIDTH)
        .ceil()
}

/// 排布。`heading_level` 决定段落格式按钮上的文字（也就决定它的宽度）。
/// 按钮按组换行：一组放不下就整组挪到下一行，与 `flex-wrap` 对每个 `div` 组的行为一致。
pub fn layout(area: Rect, heading_level: u8) -> Layout {
    layout_mode(area, heading_level, false)
}

pub fn layout_mode(area: Rect, heading_level: u8, simple: bool) -> Layout {
    let mut lay = Layout::default();
    let right_limit = area.right - PAD_X;
    let mut x = area.left + PAD_X;
    let mut top = area.top + PAD_Y;

    // 先把 items 切成组，量出每组宽度，再逐组放置
    let all = items()
        .into_iter()
        .filter(|item| {
            !simple
                || !matches!(
                    item,
                    Item::Button {
                        hit: Hit::Color(_) | Hit::Align(_) | Hit::Format(Format::Underline),
                        ..
                    }
                )
        })
        .collect::<Vec<_>>();
    let mut groups: Vec<Vec<&Item>> = vec![Vec::new()];
    for item in &all {
        if matches!(item, Item::Separator) {
            groups.push(Vec::new());
        } else {
            groups.last_mut().unwrap().push(item);
        }
    }
    groups.retain(|group| !group.is_empty());
    let item_width = |item: &Item| match item {
        Item::Heading => heading_button_width(heading_level),
        _ => BUTTON,
    };
    let count = groups.len();
    for (gi, group) in groups.iter().enumerate() {
        let last = gi + 1 == count;
        let inner: f32 = group.iter().map(|i| item_width(i)).sum::<f32>()
            + GAP * group.len().saturating_sub(1) as f32;
        let full = if last {
            inner
        } else {
            inner + GROUP_PAD + 1.0 + GROUP_MARGIN
        };
        // 放不下且不是行首：换行
        if x > area.left + PAD_X && x + full > right_limit {
            x = area.left + PAD_X;
            top += BUTTON + GAP;
        }
        for (ii, item) in group.iter().enumerate() {
            let w = item_width(item);
            let r = Rect::new(x, top, x + w, top + BUTTON);
            match item {
                Item::Button { hit, .. } => {
                    if *hit == Hit::Insert {
                        lay.insert_rect = r;
                    }
                    lay.entries.push((r, *hit));
                }
                Item::Heading => {
                    lay.heading_rect = r;
                    lay.entries.push((r, Hit::HeadingPicker));
                }
                Item::Separator => {}
            }
            x += w;
            if ii + 1 < group.len() {
                x += GAP;
            }
        }
        if !last {
            x += GROUP_PAD;
            lay.separators
                .push(Rect::new(x, top, x + 1.0, top + BUTTON));
            x += 1.0 + GROUP_MARGIN;
        }
    }
    lay.height = (top + BUTTON + PAD_Y + 1.0) - area.top;
    lay
}

pub fn paint(list: &mut DrawList, area: Rect, lay: &Layout, state: &State, p: &Palette) {
    list.rect(
        Rect::new(area.left, area.bottom - 1.0, area.right, area.bottom),
        p.border,
    );
    for r in &lay.separators {
        list.rect(*r, p.border);
    }
    for item in items() {
        match item {
            Item::Button { icon, hit, .. } => {
                let Some(&(r, _)) = lay.entries.iter().find(|(_, h)| *h == hit) else {
                    continue;
                };
                let is_active = match hit {
                    Hit::Format(f) => state.active.contains(&f),
                    Hit::Insert => state.insert_open,
                    _ => false,
                };
                let hovered = state.hover == Some(hit);
                let color = if is_active {
                    list.rounded_rect(r, 6.0, theme::mix(p.accent, p.area_main_default, 0.10));
                    p.accent
                } else if hovered {
                    list.rounded_rect(r, 6.0, p.background);
                    p.foreground
                } else {
                    p.muted
                };
                list.icon_centered(r, icon, ICON, color);
            }
            Item::Heading => {
                let Some(&(r, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::HeadingPicker)
                else {
                    continue;
                };
                let (level, label, icon) = HEADING_OPTIONS
                    .iter()
                    .find(|(l, _, _)| *l == state.heading_level)
                    .copied()
                    .unwrap_or(HEADING_OPTIONS[0]);
                let lit = state.heading_open || level > 0;
                let color = if lit {
                    list.rounded_rect(r, 6.0, theme::mix(p.accent, p.area_main_default, 0.10));
                    p.accent
                } else if state.hover == Some(Hit::HeadingPicker) {
                    list.rounded_rect(r, 6.0, p.background);
                    p.foreground
                } else {
                    p.muted
                };
                let icon_rect = Rect::new(r.left + 8.0, r.top, r.left + 8.0 + ICON, r.bottom);
                list.icon_centered(icon_rect, icon, ICON, color);
                let text_left = icon_rect.right + 6.0;
                let chevron = Rect::new(r.right - 8.0 - 13.0, r.top, r.right - 8.0, r.bottom);
                list.text(
                    Rect::new(text_left, r.top + 4.0, chevron.left - 2.0, r.bottom - 4.0),
                    label.to_owned(),
                    TextStyle::Label,
                    color,
                );
                list.icon_centered(chevron, Icon::CHEVRON_DOWN, 13.0, color);
            }
            Item::Separator => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const AREA: Rect = Rect {
        left: 100.0,
        top: 40.0,
        right: 1100.0,
        bottom: 81.0,
    };

    #[test]
    fn simple_mode_keeps_markdown_actions_and_removes_mochi_formatting() {
        let simple = layout_mode(AREA, 0, true);
        assert!(!simple.entries.iter().any(|(_, hit)| matches!(
            hit,
            Hit::Color(_) | Hit::Align(_) | Hit::Format(Format::Underline)
        )));
        for required in [
            Hit::Insert,
            Hit::HeadingPicker,
            Hit::Format(Format::Bold),
            Hit::Format(Format::Table),
            Hit::Format(Format::Link),
        ] {
            assert!(simple.entries.iter().any(|(_, hit)| *hit == required));
        }
        assert!(simple.entries.len() < layout_mode(AREA, 0, false).entries.len());
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &simple,
            &State::default(),
            theme::tokens().palette(false),
        );
        assert_eq!(
            list.cmds()
                .iter()
                .filter(|cmd| matches!(cmd, DrawCmd::Icon { .. }))
                .count(),
            simple.entries.len() + 1
        );
        assert!(list.finish().is_ok());
    }

    #[test]
    fn buttons_are_28px_squares_two_pixels_apart_and_groups_have_a_divider() {
        let lay = layout(AREA, 0);
        assert_eq!(lay.height, HEIGHT, "一行放得下时高度是 41");
        let (first, hit) = lay.entries[0];
        assert_eq!(hit, Hit::Insert);
        assert_eq!(first.left, AREA.left + 20.0, "px-5");
        assert_eq!(first.top, AREA.top + 6.0, "py-1.5");
        assert_eq!(first.width(), 28.0);
        assert_eq!(first.height(), 28.0);
        let (bold, hit) = lay.entries[1];
        assert_eq!(hit, Hit::Format(Format::Bold));
        // 组间距：pr-3 (12) + 1px 线 + mr-1 (4)
        assert_eq!(bold.left, first.right + 12.0 + 1.0 + 4.0);
        let (italic, _) = lay.entries[2];
        assert_eq!(italic.left, bold.right + 2.0, "gap-0.5");
        // 九组按钮之间八条竖线（最后一组之后没有）
        assert_eq!(lay.separators.len(), 8);
    }

    #[test]
    fn the_heading_picker_is_at_least_76px_wide_and_grows_with_its_label() {
        let lay = layout(AREA, 0);
        assert!(lay.heading_rect.width() >= 76.0);
        let wide = layout(AREA, 1);
        assert!(
            wide.heading_rect.width()
                >= 8.0 + 16.0 + 6.0 + text::measure("标题 1", TextStyle::Label) + 6.0 + 13.0 + 8.0,
            "文字不能被裁掉"
        );
        let r = lay.heading_rect;
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        assert_eq!(lay.hit(cx, cy), Some(Hit::HeadingPicker));
        assert_eq!(lay.hit(AREA.left + 2.0, cy), None, "左留白不是按钮");
    }

    #[test]
    fn state_reflects_the_line_under_the_cursor() {
        let s = State::at("## 标题\n- 项", 2);
        assert_eq!(s.heading_level, 2);
        assert!(s.active.is_empty());
        let s = State::at("## 标题\n- 项", 10);
        assert_eq!(s.heading_level, 0);
        assert_eq!(s.active, vec![Format::BulletList]);
    }

    #[test]
    fn narrow_areas_wrap_whole_groups_onto_a_second_row() {
        let narrow = Rect::new(0.0, 0.0, 500.0, 41.0);
        let lay = layout(narrow, 0);
        assert!(lay.height > HEIGHT, "放不下要换行，高度增加");
        let rows: std::collections::BTreeSet<i32> =
            lay.entries.iter().map(|(r, _)| r.top as i32).collect();
        assert!(rows.len() >= 2);
        assert!(
            lay.entries
                .iter()
                .all(|(r, _)| r.right <= narrow.right - 20.0),
            "没有按钮伸出右留白"
        );
        // 每组不拆：同一组内的按钮在同一行
        let bold = lay
            .entries
            .iter()
            .find(|(_, h)| *h == Hit::Format(Format::Bold))
            .unwrap()
            .0;
        let clear = lay
            .entries
            .iter()
            .find(|(_, h)| *h == Hit::Format(Format::ClearFormat))
            .unwrap()
            .0;
        assert_eq!(bold.top, clear.top);
    }

    #[test]
    fn painting_shows_the_current_block_label_and_every_icon() {
        let lay = layout(AREA, 3);
        let state = State {
            heading_level: 3,
            ..State::default()
        };
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &lay,
            &state,
            theme::tokens().palette(false),
        );
        let texts: Vec<String> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["标题 3"]);
        let icons = list
            .cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Icon { .. }))
            .count();
        // 23 个按钮图标 + 段落格式的图标与下箭头
        assert_eq!(icons, 23 + 2);
        assert!(list.finish().is_ok());
    }
}
