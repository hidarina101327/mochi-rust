//! 计算桌面卡片窗口的导航栏布局、控件位置和滚动范围。
use super::*;

fn geometry(spec: &Spec, view: &View, a: Rect) -> (usize, usize, f32) {
    let left = spec.appearance.tabs_left;
    let available = if left {
        a.height() - 92.0
    } else {
        // Reserve the title-bar buttons plus the right-side overflow arrows.
        // The arrow pair ends 62 DIP from the edge; the capsule starts at 60.
        a.width() - 136.0
    };
    let step = if left {
        36.0
    } else {
        112.0_f32.min(available.max(60.0))
    };
    let capacity = (available / step).floor().max(1.0) as usize;
    let active = view
        .tabs
        .iter()
        .position(|(id, _)| id == &view.page_id)
        .unwrap_or(0);
    let start = view
        .tab_scroll
        .unwrap_or_else(|| active.saturating_sub(capacity - 1))
        .min(view.tabs.len().saturating_sub(capacity));
    (start, capacity, step)
}
pub(super) fn controls(spec: &Spec, view: &View, a: Rect) -> Vec<(Rect, Hit)> {
    if view.tabs.len() <= 1 && !spec.appearance.show_single_page_name {
        return vec![];
    }
    let (start, capacity, step) = geometry(spec, view, a);
    let left = spec.appearance.tabs_left;
    let extent = super::painting::navigation_extent(spec, view, a);
    let mut hits = vec![];
    let overflow = view.tabs.len() > capacity;
    for (index, (id, _)) in view.tabs.iter().enumerate().skip(start).take(capacity) {
        let i = (index - start) as f32;
        let r = if left {
            Rect::from_size(
                a.left + 8.0,
                a.top + 36.0 + i * step,
                (extent - 16.0).max(24.0),
                32.0,
            )
        } else {
            Rect::from_size(
                a.left + 12.0 + i * step,
                a.top + (extent - 32.0) / 2.0,
                step - 4.0,
                32.0,
            )
        };
        hits.push((r, Hit::Page(id.clone())));
    }
    if overflow {
        let y = if left {
            a.bottom - 42.0
        } else {
            a.top + (extent - 28.0) / 2.0
        };
        let x = if left { a.left + 8.0 } else { a.right - 116.0 };
        if start > 0 {
            hits.push((Rect::from_size(x, y, 26.0, 28.0), Hit::TabScroll(-1)));
        }
        if start + capacity < view.tabs.len() {
            hits.push((Rect::from_size(x + 28.0, y, 26.0, 28.0), Hit::TabScroll(1)));
        }
    }
    hits
}
pub(super) fn scroll(spec: &Spec, view: &mut View, a: Rect, delta: i32) {
    let (start, capacity, _) = geometry(spec, view, a);
    view.tab_scroll = Some(
        (start as i32 + delta).clamp(0, view.tabs.len().saturating_sub(capacity) as i32) as usize,
    );
}
