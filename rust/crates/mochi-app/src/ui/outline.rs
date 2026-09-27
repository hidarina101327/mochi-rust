//! 标题位置复用正文排版结果。

use super::document::Heading;
use super::draw::{DrawList, TextStyle};
use super::layout::{visible_rows, Rect};
use super::text;
use super::theme::{self, Palette, ROW_HEIGHT};

/// 每一级标题的缩进。比文件树小一点——大纲通常只有两三级，缩太多会浪费横向空间。
const INDENT: f32 = 12.0;
const PADDING_X: f32 = 12.0;
/// 面板顶部的标题栏高度。
const HEADER_HEIGHT: f32 = 32.0;

fn setting_number(key: &str, fallback: f32, min: f32, max: f32) -> f32 {
    let value = super::settings_values::number(key, fallback);
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn font_size() -> f32 {
    setting_number("outline.fontSize", TextStyle::Body.font_size(), 8.0, 72.0)
}

fn text_scale() -> f32 {
    (font_size() / TextStyle::Body.font_size().max(1.0)).clamp(0.25, 12.0)
}

fn row_height() -> f32 {
    (font_size() * 2.0).clamp(16.0, 144.0)
}

fn row_gap() -> f32 {
    setting_number("outline.rowSpacing", 0.0, 0.0, 24.0)
}

fn row_step() -> f32 {
    row_height() + row_gap()
}

fn indent() -> f32 {
    setting_number("outline.indent", INDENT, 0.0, 48.0)
}

fn padding_x() -> f32 {
    setting_number("outline.paddingX", PADDING_X, 0.0, 48.0)
}

fn header_height() -> f32 {
    setting_number("outline.headerHeight", HEADER_HEIGHT, 24.0, 64.0)
        .max(TextStyle::Title.line_height())
}

fn paint_outline_text(list: &mut DrawList, rect: Rect, text: impl Into<String>, color: u32) {
    let start = list.cmds().len();
    list.text(rect, text, TextStyle::Body, color);
    let scale = text_scale();
    if (scale - 1.0).abs() > f32::EPSILON {
        list.scale_text_since(start, scale);
    }
}

pub fn body(area: Rect) -> Rect {
    Rect::new(
        area.left,
        (area.top + header_height()).min(area.bottom),
        area.right,
        area.bottom,
    )
}

pub fn max_scroll(area: Rect, count: usize) -> usize {
    let visible = (body(area).height().max(0.0) / row_step()).floor() as usize;
    count.saturating_sub(visible.max(1))
}

pub fn hit(area: Rect, count: usize, scroll: usize, x: f32, y: f32) -> Option<usize> {
    let header_height = header_height();
    if !area.contains(x, y) || y < area.top + header_height {
        return None;
    }
    let relative = y - area.top - header_height;
    let step = row_step();
    if relative % step >= row_height() {
        return None;
    }
    let index = scroll.min(max_scroll(area, count)) + (relative / step) as usize;
    (index < count).then_some(index)
}

pub fn hover_rect(area: Rect, count: usize, scroll: usize, x: f32, y: f32) -> Option<Rect> {
    let index = hit(area, count, scroll, x, y)?;
    let body = body(area);
    let top = body.top + (index - scroll.min(max_scroll(area, count))) as f32 * row_step();
    Some(Rect::new(body.left + 4.0, top, body.right - 4.0, top + row_height()).intersect(&body))
}

/// 画大纲。`active` 是当前滚动位置所在的那条，高亮它。
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    headings: &[Heading],
    active: Option<usize>,
    scroll_top: usize,
    p: &Palette,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);

    let header = Rect::new(area.left, area.top, area.right, area.top + header_height());
    list.text(
        Rect::new(
            header.left + padding_x(),
            header.top,
            header.right - 36.0,
            header.bottom,
        ),
        "大纲",
        TextStyle::Caption,
        p.muted,
    );

    let body = Rect::new(area.left, header.bottom, area.right, area.bottom);
    if headings.is_empty() {
        list.text(
            Rect::new(
                body.left + padding_x(),
                body.top + 8.0,
                body.right,
                body.top + 8.0 + row_height(),
            ),
            "这篇文档没有标题",
            TextStyle::Caption,
            p.muted,
        );
        list.pop_clip();
        return;
    }

    let visible = visible_rows(
        headings.len(),
        row_step(),
        scroll_top.min(max_scroll(area, headings.len())),
        body.height(),
    );
    for (offset, index) in visible.range().enumerate() {
        let h = &headings[index];
        let top = body.top + offset as f32 * row_step();
        let row_height = row_height();
        let row = Rect::new(body.left, top, body.right, top + row_height);
        let is_active = active == Some(index);

        if is_active {
            list.rect(row, theme::mix(p.accent, p.area_assistant_default, 0.10));
        }
        // 一级标题用正文色，更深的层级用次要色——层级靠颜色比靠缩进更容易一眼看出
        let color = match (h.level, is_active) {
            (_, true) => p.accent,
            (1, _) => p.foreground,
            _ => p.muted,
        };
        let indent = indent();
        let padding_x = padding_x();
        let x = body.left + padding_x + (h.level.saturating_sub(1)) as f32 * indent;
        let guide = super::guides::Style::read(false, p.border);
        for level in 1..h.level {
            let gx = body.left + padding_x + f32::from(level - 1) * indent + indent / 2.0;
            guide.line(list, gx, top, top + row_height, true);
        }
        if h.level > 1 {
            guide.line(
                list,
                x - indent / 2.0,
                top + row_height / 2.0,
                (x - guide.gap).max(x - indent / 2.0),
                false,
            );
        }
        let scale = text_scale();
        let avail = ((body.right - 8.0 - x).max(0.0)) / scale;
        paint_outline_text(
            list,
            Rect::new(x, top, body.right - 8.0, top + row_height),
            text::ellipsize(&h.text, TextStyle::Body, avail),
            color,
        );
    }

    list.pop_clip();
}

/// 当前滚动位置对应大纲里的哪一条：**最后一个已经滚过去的标题**。
///
/// 不是"离得最近的"——那样在两个标题之间时会来回跳。用"已经越过的最后一个"，
/// 读者往下滚时高亮只会单向前进，符合直觉。
///
/// 一条例外：**以标题开篇的文档，在滚动位置 0 就该高亮首个标题**。按纯粹的
/// "已越过"判定，`# 标题` 在 y=48 处还没被越过，于是打开文档时大纲一片不亮，
/// 看着像坏了。落在开头留白区内的首个标题视为已进入。
pub fn active_at(headings: &[Heading], scroll: f32) -> Option<usize> {
    // 加一点余量：标题刚滚到视口顶部时就该高亮，而不是等它滚出去
    let probe = scroll + row_height();
    let passed = headings.partition_point(|heading| heading.y <= probe);
    if passed > 0 {
        return Some(passed - 1);
    }
    headings
        .first()
        .filter(|h| h.y <= FIRST_HEADING_GRACE)
        .map(|_| 0)
}

/// 文档开头的留白加上一个标题的上边距。首个标题落在这个范围内，
/// 说明这篇文档是以标题开篇的。
const FIRST_HEADING_GRACE: f32 = 96.0;

/// 点某条大纲后，编辑器该滚到哪。
///
/// 让标题停在视口顶部略下方，而不是正好贴顶——贴顶时标题会紧挨着标签栏，
/// 看起来像被切掉了。
pub fn scroll_target(heading: &Heading) -> f32 {
    (heading.y - row_height()).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const AREA: Rect = Rect {
        left: 780.0,
        top: 72.0,
        right: 1200.0,
        bottom: 776.0,
    };

    fn heads() -> Vec<Heading> {
        vec![
            Heading {
                level: 1,
                text: "计算机通识".into(),
                y: 42.0,
            },
            Heading {
                level: 2,
                text: "章节结构".into(),
                y: 200.0,
            },
            Heading {
                level: 3,
                text: "代办".into(),
                y: 400.0,
            },
            Heading {
                level: 2,
                text: "代码".into(),
                y: 600.0,
            },
        ]
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_heading_gets_a_row() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &heads(), None, 0, &p);

        let t = texts(&list);
        assert_eq!(t.first(), Some(&"大纲".to_owned()));
        for h in heads() {
            assert!(t.contains(&h.text), "少了 {}", h.text);
        }
    }

    #[test]
    fn deeper_headings_are_indented_further() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &heads(), None, 0, &p);

        let x_of = |label: &str| {
            list.cmds()
                .iter()
                .find_map(|c| match c {
                    DrawCmd::Text { text, rect, .. } if text == label => Some(rect.left),
                    _ => None,
                })
                .unwrap()
        };
        assert!(x_of("章节结构") > x_of("计算机通识"));
        assert!(x_of("代办") > x_of("章节结构"));
        assert_eq!(x_of("章节结构") - x_of("计算机通识"), INDENT);
    }

    #[test]
    fn an_empty_outline_says_so_instead_of_showing_a_blank_panel() {
        // 空白面板让人以为是坏了；写一句话说明是文档本身没有标题
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &[], None, 0, &p);
        assert!(texts(&list).iter().any(|t| t.contains("没有标题")));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_active_heading_is_highlighted() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &heads(), Some(2), 0, &p);

        let tint = theme::mix(p.accent, p.area_assistant_default, 0.10);
        let hits: Vec<Rect> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Rect { rect, color } if *color == tint => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(hits.len(), 1);
        // 第三条（下标 2）
        assert_eq!(hits[0].top, AREA.top + HEADER_HEIGHT + ROW_HEIGHT * 2.0);
    }

    #[test]
    fn the_active_heading_is_the_last_one_scrolled_past_not_the_nearest() {
        // 「最近的」在两个标题之间时会来回跳；「已越过的最后一个」只会单向前进
        let h = heads();
        assert_eq!(active_at(&h, 0.0), Some(0));
        assert_eq!(active_at(&h, 190.0), Some(1));
        // 滚到 390：第三个标题在 400，还没越过——仍是第二个
        assert_eq!(active_at(&h, 350.0), Some(1));
        assert_eq!(active_at(&h, 380.0), Some(2));
        assert_eq!(active_at(&h, 9999.0), Some(3));
    }

    #[test]
    fn there_is_no_active_heading_before_the_first_one() {
        // 首个标题远在下面（文档以几段正文开篇）：这时确实还没进入任何一节
        let h = vec![Heading {
            level: 1,
            text: "很靠下的标题".into(),
            y: 500.0,
        }];
        assert_eq!(active_at(&h, 0.0), None);
        assert_eq!(active_at(&[], 0.0), None);
    }

    #[test]
    fn a_document_starting_with_a_heading_highlights_it_from_the_very_top() {
        // 按纯粹的「已越过」判定，y=48 的标题在 scroll=0 时还没被越过，
        // 于是刚打开文档大纲一片不亮——看着像坏了
        let h = vec![Heading {
            level: 1,
            text: "开篇标题".into(),
            y: 48.0,
        }];
        assert_eq!(active_at(&h, 0.0), Some(0));
        // 边界：刚好在宽限范围内/外
        let inside = vec![Heading {
            level: 1,
            text: "内".into(),
            y: FIRST_HEADING_GRACE,
        }];
        let outside = vec![Heading {
            level: 1,
            text: "外".into(),
            y: FIRST_HEADING_GRACE + 1.0,
        }];
        assert_eq!(active_at(&inside, 0.0), Some(0));
        assert_eq!(active_at(&outside, 0.0), None);
    }

    #[test]
    fn jumping_to_a_heading_leaves_a_little_room_above_it() {
        // 正好贴顶时标题会紧挨着标签栏，看着像被切掉
        let h = &heads()[1];
        assert!(scroll_target(h) < h.y);
        assert_eq!(scroll_target(h), h.y - ROW_HEIGHT);
        // 首个标题不该滚出负数
        assert_eq!(
            scroll_target(&Heading {
                level: 1,
                text: "顶".into(),
                y: 10.0
            }),
            0.0
        );
    }

    #[test]
    fn a_long_heading_is_ellipsized_rather_than_overflowing() {
        let long = vec![Heading {
            level: 1,
            text: "非常长的标题".repeat(20),
            y: 0.0,
        }];
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &long, None, 0, &p);
        assert!(texts(&list).iter().any(|t| t.ends_with('…')));
    }

    #[test]
    fn a_zero_sized_panel_is_skipped_entirely() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(
            &mut list,
            Rect::new(0.0, 0.0, 0.0, 0.0),
            &heads(),
            None,
            0,
            &p,
        );
        assert!(list.is_empty());
        assert!(list.finish().is_ok());
    }
}
