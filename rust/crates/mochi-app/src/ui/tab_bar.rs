//! 标签栏。对应 `src/components/TabBar/TabBar.tsx`。
//!
//! 与文件树同样的边界：只吃投影出来的标签数据，不吃 `Shell`。

use super::draw::{Align, DrawList, TextStyle};
use super::icons::{self, Icon};
use super::layout::Rect;
use super::text;
use super::theme::{self, Palette};

/// 标签左右内边距。TSX 是 `px-4`（紧凑档 `px-3`）。
const PADDING_X: f32 = 16.0;
const PADDING_X_COMPACT: f32 = 12.0;
/// 关闭按钮的宽度（含与标题之间的间隙）。
const CLOSE_WIDTH: f32 = 20.0;
/// 保存/刷新状态点固定占位。状态变化只改变点的可见性，不能推动标题、关闭按钮或后续标签。
const STATUS_WIDTH: f32 = 12.0;
/// 单个标签的宽度上下界。TSX 靠 flex-shrink-0 + 内容宽度，这里给个区间避免
/// 一个长文件名把整条标签栏挤爆。
const MIN_TAB_WIDTH: f32 = 90.0;
const MAX_TAB_WIDTH: f32 = 220.0;
/// 固定标签宽度开关打开时使用的宽度。紧凑标签栏仍保留更小的固定档，
/// 这样窄窗口通过横向滚动容纳标签，关闭按钮不会挤到拆分按钮区域。
pub const FIXED_TAB_WIDTH: f32 = 160.0;
pub const FIXED_TAB_WIDTH_COMPACT: f32 = 136.0;
/// 激活标签顶部的强调色条。TSX 是 `shadow-[inset_0_2px_0_var(--accent)]`。
const ACTIVE_BAR: f32 = 2.0;
/// Electron 标签栏末尾的拆分按钮是 `w-8 h-8`，即 32 DIP。
const SPLIT_BUTTON_SIZE: f32 = 32.0;
const NAVIGATION_BUTTON_SIZE: f32 = 30.0;

fn setting_number(key: &str, fallback: f32, min: f32, max: f32) -> f32 {
    let value = super::settings_values::number(key, fallback);
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn horizontal_padding(compact: bool) -> f32 {
    let key = if compact {
        "tabs.compactPaddingX"
    } else {
        "tabs.paddingX"
    };
    let fallback = if compact {
        PADDING_X_COMPACT
    } else {
        PADDING_X
    };
    setting_number(key, fallback, 0.0, 40.0)
}

fn close_slot_width() -> f32 {
    setting_number("tabs.closeButtonSlotWidth", CLOSE_WIDTH, 12.0, 48.0)
}

fn close_button_size() -> f32 {
    setting_number("tabs.closeButtonSize", 16.0, 10.0, 32.0)
}

fn status_slot_width() -> f32 {
    setting_number("tabs.statusSlotWidth", STATUS_WIDTH, 0.0, 32.0)
}

fn min_tab_width() -> f32 {
    setting_number("tabs.minWidth", MIN_TAB_WIDTH, 48.0, 320.0)
}

fn max_tab_width() -> f32 {
    setting_number("tabs.maxWidth", MAX_TAB_WIDTH, min_tab_width(), 640.0)
}

fn fixed_tab_width(compact: bool) -> f32 {
    let key = if compact {
        "tabs.compactFixedWidth"
    } else {
        "tabs.fixedWidthValue"
    };
    let fallback = if compact {
        FIXED_TAB_WIDTH_COMPACT
    } else {
        FIXED_TAB_WIDTH
    };
    setting_number(key, fallback, 48.0, 480.0)
}

fn split_button_size() -> f32 {
    setting_number("tabs.splitButtonSize", SPLIT_BUTTON_SIZE, 16.0, 64.0)
}

fn navigation_button_size() -> f32 {
    setting_number(
        "tabs.navigationButtonSize",
        NAVIGATION_BUTTON_SIZE,
        16.0,
        64.0,
    )
}

fn active_bar_height() -> f32 {
    setting_number("tabs.activeIndicatorHeight", ACTIVE_BAR, 1.0, 6.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    Back,
    Forward,
}

/// 固定控件共用同一套几何信息和命中测试。即使控件被禁用，也会保留
/// 原有空间；可滚动的标签页永远不会遮住或拦截两侧箭头。
pub fn navigation_button_rects(area: Rect) -> [Rect; 2] {
    if area.is_empty() {
        return [Rect::ZERO; 2];
    }
    let end = split_button_rect(area).left;
    let size = navigation_button_size();
    let width = size.min((end - area.left).max(0.0) / 2.0);
    let height = size.min(area.height());
    let top = area.top + (area.height() - height) * 0.5;
    [
        Rect::new(area.left, top, area.left + width, top + height),
        Rect::new(
            area.left + width,
            top,
            area.left + width * 2.0,
            top + height,
        ),
    ]
}

pub fn navigation_hit(area: Rect, x: f32, y: f32) -> Option<Navigation> {
    let [back, forward] = navigation_button_rects(area);
    if back.contains(x, y) {
        Some(Navigation::Back)
    } else if forward.contains(x, y) {
        Some(Navigation::Forward)
    } else {
        None
    }
}

pub fn paint_navigation(
    list: &mut DrawList,
    area: Rect,
    enabled: [bool; 2],
    hover: Option<Navigation>,
    p: &Palette,
) {
    for (i, rect) in navigation_button_rects(area).into_iter().enumerate() {
        if rect.is_empty() {
            continue;
        }
        let direction = [Navigation::Back, Navigation::Forward][i];
        let hovered = enabled[i] && hover == Some(direction);
        if hovered {
            list.rect(rect, p.surface);
        }
        let color = if !enabled[i] {
            theme::mix(p.muted, p.surface, 0.35)
        } else if hovered {
            p.foreground
        } else {
            p.muted
        };
        let size = 16.0_f32.min(rect.width()).min(rect.height());
        let icon = [Icon::CHEVRON_LEFT, Icon::CHEVRON_RIGHT][i];
        if icons::lookup(icon).is_some() {
            list.icon_centered(rect, icon, size, color);
        } else {
            list.text_aligned(
                rect,
                if i == 0 { "‹" } else { "›" },
                TextStyle::Body,
                color,
                Align::Center,
            );
        }
    }
}
/// 与共享图标资源中的 `SplitSquareVertical` 对应。
const SPLIT_ICON: Icon = Icon("SplitSquareVertical");
thread_local! {static HOVER:std::cell::Cell<Option<usize>>=const{std::cell::Cell::new(None)};}
pub fn set_hover(index: Option<usize>) -> bool {
    HOVER.with(|h| h.replace(index) != index)
}

/// 标签栏右端固定拆分按钮的可命中矩形。
///
/// 按钮按 Electron 的 32×32 DIP 尺寸垂直居中；标签栏窄于按钮时，按钮只取
/// 剩余宽度，避免矩形倒置或与内容区相交。
pub fn split_button_rect(area: Rect) -> Rect {
    if area.is_empty() {
        return Rect::ZERO;
    }
    let size = split_button_size();
    let width = size.min(area.width());
    let height = size.min(area.height());
    let top = area.top + (area.height() - height) / 2.0;
    Rect::new(area.right - width, top, area.right, top + height)
}

/// 标签内容区域：左端进退键、右端拆分按钮都固定，不参与横向滚动。
///
/// 调用方应把这个矩形同时传给 `layout`、`max_scroll`、`paint_scrolled` 和
/// `hit_scrolled`；这样标签的关闭按钮与横向滚动内容都不会进入拆分按钮区域。
pub fn tabs_content_area(area: Rect) -> Rect {
    if area.is_empty() {
        return Rect::ZERO;
    }
    let button = split_button_rect(area);
    let left = navigation_button_rects(area)[1].right;
    Rect::new(left, area.top, button.left.max(left), area.bottom)
}

/// 没有拆分图标资源时画一个轻量的左右两列线框，保持按钮仍然可识别。
fn paint_split_fallback(list: &mut DrawList, rect: Rect, color: u32) {
    let size = rect.width().min(rect.height()).min(16.0);
    if size <= 2.0 {
        return;
    }
    let cx = (rect.left + rect.right) / 2.0;
    let cy = (rect.top + rect.bottom) / 2.0;
    let glyph = Rect::new(
        cx - size / 2.0,
        cy - size / 2.0,
        cx + size / 2.0,
        cy + size / 2.0,
    );
    list.border_left(glyph, color);
    list.border_right(glyph, color);
    list.hline(glyph.left, glyph.right, glyph.top, color);
    list.hline(glyph.left, glyph.right, glyph.bottom - 1.0, color);
    list.vline(cx, glyph.top, glyph.bottom, color);
}

/// 画标签栏右端的“向右拆分编辑器”按钮。
///
/// `enabled` 通常等于是否存在活动标签，`active` 表示当前已经有副编辑器，
/// `hover` 表示指针在按钮上。状态优先级与 Electron 端一致：禁用时保持灰色，
/// 已拆分时使用强调色，悬停时使用表面色。
pub fn paint_split_button(
    list: &mut DrawList,
    area: Rect,
    enabled: bool,
    active: bool,
    hover: bool,
    p: &Palette,
) {
    let button = split_button_rect(area);
    if button.is_empty() {
        return;
    }

    let active = enabled && active;
    let hover = enabled && hover;
    if active {
        list.rect(button, theme::mix(p.accent, p.surface, 0.10));
    } else if hover {
        list.rect(button, p.surface);
    }

    let color = if !enabled {
        p.muted
    } else if active {
        p.accent
    } else if hover {
        p.foreground
    } else {
        p.muted
    };
    let icon_size = 16.0_f32.min(button.width()).min(button.height());
    if icon_size <= 0.0 {
        return;
    }
    if icons::lookup(SPLIT_ICON).is_some() {
        list.icon_centered(button, SPLIT_ICON, icon_size, color);
    } else {
        paint_split_fallback(list, button, color);
    }
}

/// 一个标签的可绘制投影。
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub title: String,
    /// 有未保存改动时标题后面跟一个圆点。
    pub dirty: bool,
    pub pinned: bool,
}

/// 标签在标签栏里的几何。命中测试和绘制共用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabRect {
    pub index: usize,
    pub rect: Rect,
    /// 关闭按钮的矩形。固定标签没有关闭按钮，此时是 `Rect::ZERO`。
    pub close: Rect,
}

/// 点到了标签栏的什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// 切到这个标签。
    Select(usize),
    /// 关掉这个标签。
    Close(usize),
}

/// 算出每个标签的矩形。超出标签栏宽度的标签仍会算出坐标，调用方可以通过
/// `*_scrolled` 系列在内容区内横向滚动，绘制时靠裁剪切掉溢出部分。
pub fn layout(area: Rect, tabs: &[Tab], compact: bool) -> Vec<TabRect> {
    layout_with_fixed_width(area, tabs, compact, false)
}

pub fn layout_with_fixed_width(
    area: Rect,
    tabs: &[Tab],
    compact: bool,
    fixed_width: bool,
) -> Vec<TabRect> {
    let style = TextStyle::Body;
    let pad = horizontal_padding(compact);
    let close_slot = close_slot_width();
    let status_slot = status_slot_width();
    let mut out = Vec::with_capacity(tabs.len());
    let mut x = area.left;

    for (index, tab) in tabs.iter().enumerate() {
        let mut w = text::measure(&tab.title, style) + pad * 2.0;
        w += status_slot;
        if !tab.pinned {
            w += close_slot;
        }
        let w = if fixed_width {
            fixed_tab_width(compact)
        } else {
            w.clamp(min_tab_width(), max_tab_width())
        };
        let rect = Rect::new(x, area.top, x + w, area.bottom);
        let close = if tab.pinned {
            Rect::ZERO
        } else {
            // 靠右，纵向居中
            let cy = (area.top + area.bottom) / 2.0;
            let right = rect.right - pad + 2.0;
            let size = close_button_size().min((rect.width() - pad).max(0.0));
            Rect::new(right - size, cy - size / 2.0, right, cy + size / 2.0)
        };
        out.push(TabRect { index, rect, close });
        x += w;
    }
    out
}

/// 画标签栏。底色与下边框由 `chrome` 铺好，这里只画标签。
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    tabs: &[Tab],
    active: Option<usize>,
    compact: bool,
    p: &Palette,
) {
    paint_with_fixed_width(list, area, tabs, active, compact, false, p);
}

pub fn paint_with_fixed_width(
    list: &mut DrawList,
    area: Rect,
    tabs: &[Tab],
    active: Option<usize>,
    compact: bool,
    fixed_width: bool,
    p: &Palette,
) {
    if area.is_empty() || tabs.is_empty() {
        return;
    }
    list.push_clip(area);
    let pad = horizontal_padding(compact);
    let close_slot = close_slot_width();
    let status_slot = status_slot_width();
    let style = TextStyle::Body;

    for tr in layout_with_fixed_width(area, tabs, compact, fixed_width) {
        let tab = &tabs[tr.index];
        let is_active = active == Some(tr.index);

        if is_active {
            // 激活标签用主区底色，看起来像是和下面的编辑器连成一片
            list.rect(tr.rect, p.area_main_default);
            list.rect(
                Rect::new(
                    tr.rect.left,
                    tr.rect.top,
                    tr.rect.right,
                    tr.rect.top + active_bar_height(),
                ),
                p.accent,
            );
        }
        list.border_right(tr.rect, p.border);

        // 标题：状态点始终占位，刷新时只显隐，不触发布局跳变。
        let mut text_right = tr.rect.right - pad;
        if !tab.pinned {
            text_right -= close_slot;
        }
        text_right -= status_slot;
        let avail = (text_right - tr.rect.left - pad).max(0.0);
        let title = text::ellipsize(&tab.title, style, avail);
        list.text(
            Rect::new(tr.rect.left + pad, tr.rect.top, text_right, tr.rect.bottom),
            title,
            style,
            if is_active { p.foreground } else { p.muted },
        );

        if tab.dirty {
            list.text_aligned(
                Rect::new(
                    text_right,
                    tr.rect.top,
                    text_right + status_slot,
                    tr.rect.bottom,
                ),
                "●",
                TextStyle::Caption,
                p.accent,
                Align::Center,
            );
        }
        if !tab.pinned
            && (is_active
                || super::settings_values::text("tabs.showCloseButtons", "always") == "always"
                || HOVER.with(|h| h.get() == Some(tr.index)))
        {
            list.text_aligned(tr.close, "✕", TextStyle::Caption, p.muted, Align::Center);
        }
    }

    list.pop_clip();
}

/// 点在标签栏的哪里。关闭按钮优先于标签本体——它压在标签上面。
pub fn hit(area: Rect, tabs: &[Tab], compact: bool, x: f32, y: f32) -> Option<Hit> {
    hit_with_fixed_width(area, tabs, compact, false, x, y)
}

pub fn hit_with_fixed_width(
    area: Rect,
    tabs: &[Tab],
    compact: bool,
    fixed_width: bool,
    x: f32,
    y: f32,
) -> Option<Hit> {
    if !area.contains(x, y) {
        return None;
    }
    for tr in layout_with_fixed_width(area, tabs, compact, fixed_width) {
        if tr.close.contains(x, y) {
            return Some(Hit::Close(tr.index));
        }
        if tr.rect.contains(x, y) {
            return Some(Hit::Select(tr.index));
        }
    }
    None
}

pub fn max_scroll(area: Rect, tabs: &[Tab], compact: bool) -> f32 {
    max_scroll_with_fixed_width(area, tabs, compact, false)
}

pub fn max_scroll_with_fixed_width(
    area: Rect,
    tabs: &[Tab],
    compact: bool,
    fixed_width: bool,
) -> f32 {
    layout_with_fixed_width(area, tabs, compact, fixed_width)
        .last()
        .map(|r| (r.rect.right - area.right).max(0.0))
        .unwrap_or(0.0)
}
#[allow(dead_code)]
pub fn paint_scrolled(
    list: &mut DrawList,
    area: Rect,
    tabs: &[Tab],
    active: Option<usize>,
    compact: bool,
    scroll: f32,
    p: &Palette,
) {
    paint_scrolled_with_fixed_width(list, area, tabs, active, compact, false, scroll, p);
}

pub fn paint_scrolled_with_fixed_width(
    list: &mut DrawList,
    area: Rect,
    tabs: &[Tab],
    active: Option<usize>,
    compact: bool,
    fixed_width: bool,
    scroll: f32,
    p: &Palette,
) {
    list.push_clip(area);
    paint_with_fixed_width(
        list,
        Rect::new(area.left - scroll, area.top, area.right, area.bottom),
        tabs,
        active,
        compact,
        fixed_width,
        p,
    );
    list.pop_clip();
}
pub fn hit_scrolled(
    area: Rect,
    tabs: &[Tab],
    compact: bool,
    scroll: f32,
    x: f32,
    y: f32,
) -> Option<Hit> {
    hit_scrolled_with_fixed_width(area, tabs, compact, false, scroll, x, y)
}

pub fn hit_scrolled_with_fixed_width(
    area: Rect,
    tabs: &[Tab],
    compact: bool,
    fixed_width: bool,
    scroll: f32,
    x: f32,
    y: f32,
) -> Option<Hit> {
    if !area.contains(x, y) {
        return None;
    }
    hit_with_fixed_width(
        Rect::new(area.left - scroll, area.top, area.right, area.bottom),
        tabs,
        compact,
        fixed_width,
        x,
        y,
    )
}

/// 所有标签的总宽度。超出标签栏时调用方应启用横向滚动。
#[allow(dead_code)]
pub fn total_width(area: Rect, tabs: &[Tab], compact: bool) -> f32 {
    total_width_with_fixed_width(area, tabs, compact, false)
}

#[allow(dead_code)]
pub fn total_width_with_fixed_width(
    area: Rect,
    tabs: &[Tab],
    compact: bool,
    fixed_width: bool,
) -> f32 {
    layout_with_fixed_width(area, tabs, compact, fixed_width)
        .last()
        .map(|t| t.rect.right - area.left)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_controls_are_fixed_and_disjoint_even_in_tiny_bars() {
        for width in [0.0, 20.0, 32.0, 60.0, 92.0, 100.0, 800.0] {
            let area = Rect::new(100.0, 32.0, 100.0 + width, 65.0);
            let rects = navigation_button_rects(area);
            let split = split_button_rect(area);
            let content = tabs_content_area(area);
            for (i, rect) in rects.into_iter().enumerate() {
                assert!(rect.intersect(&split).is_empty());
                assert!(rect.intersect(&content).is_empty());
                assert!(rect.intersect(&rects[1 - i]).is_empty());
                if !rect.is_empty() {
                    let (x, y) = (
                        (rect.left + rect.right) * 0.5,
                        (rect.top + rect.bottom) * 0.5,
                    );
                    assert_eq!(
                        navigation_hit(area, x, y),
                        Some([Navigation::Back, Navigation::Forward][i])
                    );
                    for scroll in [0.0, 250.0] {
                        assert_eq!(
                            hit_scrolled_with_fixed_width(
                                content,
                                &[tab("test")],
                                false,
                                true,
                                scroll,
                                x,
                                y
                            ),
                            None
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn navigation_disabled_hover_does_not_paint_active_feedback() {
        let p = *theme::tokens().palette(false);
        let mut disabled = DrawList::new();
        paint_navigation(
            &mut disabled,
            AREA,
            [false, false],
            Some(Navigation::Back),
            &p,
        );
        assert!(!disabled
            .cmds()
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::Rect { .. })));
        let mut enabled = DrawList::new();
        paint_navigation(
            &mut enabled,
            AREA,
            [true, false],
            Some(Navigation::Back),
            &p,
        );
        assert!(enabled.cmds().iter().any(|cmd| matches!(cmd, DrawCmd::Rect { rect, .. } if *rect == navigation_button_rects(AREA)[0])));
    }

    #[test]
    fn scrolled_tab_paint_and_hit_use_the_same_positions() {
        let tabs = (0..20)
            .map(|i| Tab {
                title: format!("笔记 {i}"),
                dirty: false,
                pinned: false,
            })
            .collect::<Vec<_>>();
        let area = Rect::new(100.0, 0.0, 500.0, 40.0);
        let scroll = max_scroll(area, &tabs, false);
        assert!(scroll > 0.0);
        let r = layout(
            Rect::new(area.left - scroll, area.top, area.right, area.bottom),
            &tabs,
            false,
        )[19];
        assert_eq!(
            hit_scrolled(area, &tabs, false, scroll, r.rect.left + 8.0, 20.0),
            Some(Hit::Select(19))
        );
        assert!(hit_scrolled(area, &tabs, false, scroll, area.left - 1.0, 20.0).is_none());
    }
    use crate::ui::draw::DrawCmd;
    use crate::ui::theme;

    const AREA: Rect = Rect {
        left: 480.0,
        top: 32.0,
        right: 1200.0,
        bottom: 69.0,
    };

    fn tab(title: &str) -> Tab {
        Tab {
            title: title.to_owned(),
            dirty: false,
            pinned: false,
        }
    }

    #[test]
    fn a_narrow_bar_keeps_tab_hits_outside_the_fixed_split_button() {
        let area = Rect::new(100.0, 32.0, 200.0, 69.0);
        let content = tabs_content_area(area);
        let button = split_button_rect(area);

        assert_eq!(button.width(), SPLIT_BUTTON_SIZE);
        assert_eq!(content.right, button.left);
        assert!(content.intersect(&button).is_empty());

        // 最小标签比内容区宽：它可以被裁剪，但不能把关闭按钮带到固定按钮上。
        let tabs = vec![tab("窄窗口中的长文件名.md")];
        assert!(layout(content, &tabs, false)[0].rect.right > content.right);
        assert!(max_scroll(content, &tabs, false) > 0.0);

        let button_center = (
            (button.left + button.right) / 2.0,
            (button.top + button.bottom) / 2.0,
        );
        assert!(button.contains(button_center.0, button_center.1));
        assert_eq!(
            hit_scrolled(content, &tabs, false, 0.0, button_center.0, button_center.1),
            None,
            "固定按钮的命中不能落回标签"
        );
        assert_eq!(
            hit_scrolled(
                content,
                &tabs,
                false,
                0.0,
                content.right - 0.5,
                button_center.1,
            )
            .map(|hit| match hit {
                Hit::Select(index) | Hit::Close(index) => index,
            }),
            Some(0),
            "按钮左边的最后半个 DIP 仍属于标签内容区"
        );
    }

    #[test]
    fn split_button_paint_matches_electron_enabled_and_active_states() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        let area = Rect::new(0.0, 0.0, 240.0, 37.0);
        paint_split_button(&mut list, area, true, true, false, &p);

        assert!(list.cmds().iter().any(|cmd| {
            matches!(cmd, DrawCmd::Rect { rect, color }
                if *rect == split_button_rect(area)
                    && *color == theme::mix(p.accent, p.surface, 0.10))
        }));
        assert!(list.cmds().iter().any(|cmd| {
            matches!(cmd, DrawCmd::Icon { rect, icon, color }
                if *rect == Rect::new(216.0, 10.5, 232.0, 26.5)
                    && *icon == SPLIT_ICON
                    && *color == p.accent)
        }));

        let mut disabled = DrawList::new();
        paint_split_button(&mut disabled, area, false, false, true, &p);
        assert!(!disabled.cmds().iter().any(|cmd| {
            matches!(cmd, DrawCmd::Rect { rect, .. } if *rect == split_button_rect(area))
        }));
        assert!(disabled.cmds().iter().any(|cmd| {
            matches!(cmd, DrawCmd::Icon { icon, color, .. }
                if *icon == SPLIT_ICON && *color == p.muted)
        }));
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
    fn tabs_are_laid_out_left_to_right_without_gaps() {
        let tabs = vec![tab("甲.md"), tab("乙.md"), tab("丙.md")];
        let rects = layout(AREA, &tabs, false);

        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0].rect.left, AREA.left);
        for pair in rects.windows(2) {
            assert_eq!(pair[0].rect.right, pair[1].rect.left, "标签之间不该有缝");
        }
    }

    #[test]
    fn a_long_title_is_capped_and_a_short_one_is_floored() {
        let long = layout(AREA, &[tab(&"很长的文件名".repeat(10))], false);
        assert_eq!(long[0].rect.width(), MAX_TAB_WIDTH);

        let short = layout(AREA, &[tab("a")], false);
        assert_eq!(short[0].rect.width(), MIN_TAB_WIDTH);
    }

    #[test]
    fn the_active_tab_gets_an_accent_bar_on_top() {
        let tabs = vec![tab("甲.md"), tab("乙.md")];
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &tabs, Some(1), false, &p);

        let bar = list
            .cmds()
            .iter()
            .find_map(|c| match c {
                DrawCmd::Rect { rect, color } if *color == p.accent => Some(*rect),
                _ => None,
            })
            .expect("激活标签没有强调色条");
        assert_eq!(bar.height(), ACTIVE_BAR);
        assert_eq!(bar.top, AREA.top);
        // 在第二个标签上，不是第一个
        assert!(bar.left > AREA.left);
    }

    #[test]
    fn an_inactive_tab_has_no_accent_bar() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &[tab("甲.md")], None, false, &p);
        assert!(!list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == p.accent)));
    }

    #[test]
    fn a_dirty_tab_shows_a_dot() {
        let tabs = vec![Tab {
            title: "甲.md".into(),
            dirty: true,
            pinned: false,
        }];
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &tabs, Some(0), false, &p);
        assert!(texts(&list).contains(&"●".to_owned()));
    }

    #[test]
    fn status_dot_space_is_reserved_before_refresh_state_changes() {
        let clean = Tab {
            title: "KMP算法.mc".into(),
            dirty: false,
            pinned: false,
        };
        let dirty = Tab {
            dirty: true,
            ..clean.clone()
        };
        let clean_rect = layout(AREA, &[clean], false)[0];
        let dirty_rect = layout(AREA, &[dirty], false)[0];
        assert_eq!(clean_rect.rect, dirty_rect.rect, "绿点显隐不能改变标签宽度");
        assert_eq!(
            clean_rect.close, dirty_rect.close,
            "绿点显隐不能推动关闭按钮"
        );
    }

    #[test]
    fn a_pinned_tab_has_no_close_button() {
        let tabs = vec![Tab {
            title: "甲.md".into(),
            dirty: false,
            pinned: true,
        }];
        let rects = layout(AREA, &tabs, false);
        assert_eq!(rects[0].close, Rect::ZERO);

        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &tabs, Some(0), false, &p);
        assert!(!texts(&list).contains(&"✕".to_owned()));
    }

    #[test]
    fn clicking_a_tab_selects_it() {
        let tabs = vec![tab("甲.md"), tab("乙.md"), tab("丙.md")];
        let rects = layout(AREA, &tabs, false);
        let mid = |i: usize| {
            let r = rects[i].rect;
            (r.left + 10.0, (r.top + r.bottom) / 2.0)
        };

        for i in 0..3 {
            let (x, y) = mid(i);
            assert_eq!(hit(AREA, &tabs, false, x, y), Some(Hit::Select(i)));
        }
    }

    #[test]
    fn clicking_the_close_button_closes_instead_of_selecting() {
        // 关闭按钮优先命中，不能落入标签切换。
        let tabs = vec![tab("甲.md"), tab("乙.md")];
        let rects = layout(AREA, &tabs, false);
        let c = rects[1].close;
        let (x, y) = ((c.left + c.right) / 2.0, (c.top + c.bottom) / 2.0);

        assert_eq!(hit(AREA, &tabs, false, x, y), Some(Hit::Close(1)));
    }

    #[test]
    fn clicking_the_empty_area_past_the_last_tab_does_nothing() {
        let tabs = vec![tab("甲.md")];
        assert_eq!(hit(AREA, &tabs, false, AREA.right - 10.0, 50.0), None);
    }

    #[test]
    fn clicking_outside_the_bar_does_nothing() {
        let tabs = vec![tab("甲.md")];
        assert_eq!(hit(AREA, &tabs, false, 500.0, 500.0), None);
    }

    #[test]
    fn hit_testing_and_painting_use_the_same_geometry() {
        // 两处各算一遍坐标就会「看着在这、点着在那」
        let tabs = vec![tab("甲.md"), tab("乙乙乙.md"), tab("丙.md")];
        for compact in [false, true] {
            let rects = layout(AREA, &tabs, compact);
            for tr in &rects {
                let cx = tr.rect.left + 5.0;
                let cy = (tr.rect.top + tr.rect.bottom) / 2.0;
                assert_eq!(
                    hit(AREA, &tabs, compact, cx, cy),
                    Some(Hit::Select(tr.index)),
                    "compact={compact} 标签 {} 命中不一致",
                    tr.index
                );
            }
        }
    }

    #[test]
    fn the_title_is_ellipsized_rather_than_overflowing_the_tab() {
        let tabs = vec![tab("一个非常非常非常长的中文文件名.md")];
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &tabs, Some(0), false, &p);

        let title = texts(&list).into_iter().find(|t| t.contains('…'));
        assert!(title.is_some(), "过长的标题应当被截断");
    }

    #[test]
    fn an_empty_tab_bar_paints_nothing() {
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &[], None, false, &p);
        assert!(list.is_empty());
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_compact_bar_makes_tabs_narrower() {
        let tabs = vec![tab("一个中等长度的名字.md")];
        let normal = layout(AREA, &tabs, false)[0].rect.width();
        let compact = layout(AREA, &tabs, true)[0].rect.width();
        assert!(compact < normal);
    }

    #[test]
    fn fixed_width_mode_keeps_every_tab_equal_and_adaptive_mode_uses_title_width() {
        let tabs = vec![tab("a.md"), tab("一个很长的文件名.md"), tab("b.md")];
        let fixed = layout_with_fixed_width(AREA, &tabs, false, true);
        assert!(fixed
            .windows(2)
            .all(|pair| pair[0].rect.width() == pair[1].rect.width()));
        assert_eq!(fixed[0].rect.width(), FIXED_TAB_WIDTH);

        let adaptive = layout_with_fixed_width(AREA, &tabs, false, false);
        assert_ne!(adaptive[0].rect.width(), adaptive[1].rect.width());
    }

    #[test]
    fn fixed_width_mode_scrolls_in_a_narrow_bar_without_overlapping_close_or_split() {
        // 预留两侧箭头后，仍保留相同的窄标签页视口。
        let full = Rect::new(100.0, 32.0, 260.0, 69.0);
        let content = tabs_content_area(full);
        let split = split_button_rect(full);
        let tabs = vec![tab("一个很长的文件名.md"), tab("第二个文件.md")];
        let rects = layout_with_fixed_width(content, &tabs, false, true);
        let max = max_scroll_with_fixed_width(content, &tabs, false, true);
        assert!(max > 0.0);
        for tab_rect in &rects {
            assert!(tab_rect.close.left >= tab_rect.rect.left);
            assert!(tab_rect.close.right <= tab_rect.rect.right);
        }

        let scroll = max;
        let last = layout_with_fixed_width(
            Rect::new(
                content.left - scroll,
                content.top,
                content.right,
                content.bottom,
            ),
            &tabs,
            false,
            true,
        )[1];
        let close = last.close;
        assert_eq!(
            hit_scrolled_with_fixed_width(
                content,
                &tabs,
                false,
                true,
                scroll,
                (close.left + close.right) / 2.0,
                (close.top + close.bottom) / 2.0,
            ),
            Some(Hit::Close(1))
        );
        assert!(content.intersect(&split).is_empty());
        assert!(hit_scrolled_with_fixed_width(
            content,
            &tabs,
            false,
            true,
            0.0,
            (split.left + split.right) / 2.0,
            (split.top + split.bottom) / 2.0,
        )
        .is_none());
    }

    #[test]
    fn tabs_overflowing_the_bar_are_clipped_not_drawn_outside() {
        let tabs: Vec<Tab> = (0..40).map(|i| tab(&format!("文件{i}.md"))).collect();
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &tabs, Some(0), false, &p);

        assert_eq!(list.cmds().first(), Some(&DrawCmd::PushClip { rect: AREA }));
        assert!(list.finish().is_ok());
        assert!(total_width(AREA, &tabs, false) > AREA.width());
    }
}
