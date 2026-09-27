//! 启动器使用固定网格；这里不会使用已保存的节点位置。
use super::*;
use crate::ui::{draw::Align, icons::Icon, layout::Edges, theme};
use mochi_core::desktop_cards::{
    studio::{Kind, Trigger},
    Module,
};
use std::time::{Duration, Instant};
pub(super) struct Drag {
    id: String,
    start: (f32, f32),
    pub(super) since: Instant,
    active: bool,
}
pub(super) fn enabled(view: &View) -> bool {
    view.module == Some(Module::Shortcuts)
}
fn cells(spec: &Spec, view: &View, a: Rect, offset: usize) -> Vec<(Rect, usize)> {
    let body = painting::content_rect(spec, view, a);
    let cols = view.presentation.columns.max(1) as usize;
    let height = view.presentation.grid_height.max(48) as f32;
    view.studio
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == Kind::Shortcut)
        .enumerate()
        .map(|(i, (index, _))| {
            (
                Rect::from_size(
                    body.left + (i % cols) as f32 * body.width() / cols as f32,
                    body.top + (i / cols) as f32 * height - offset as f32,
                    body.width() / cols as f32,
                    height,
                ),
                index,
            )
        })
        .collect()
}
pub(super) fn scroll_max(spec: &Spec, view: &View, a: Rect) -> usize {
    let count = view
        .studio
        .nodes
        .iter()
        .filter(|n| n.kind == Kind::Shortcut)
        .count();
    ((count.div_ceil(view.presentation.columns.max(1) as usize)
        * view.presentation.grid_height.max(48) as usize) as f32
        - painting::content_rect(spec, view, a).height())
    .max(0.0) as usize
}
pub(super) fn controls(spec: &Spec, view: &View, a: Rect, offset: usize) -> Vec<(Rect, Hit)> {
    let body = painting::content_rect(spec, view, a);
    cells(spec, view, a, offset)
        .into_iter()
        .filter_map(|(r, i)| {
            let r = r.intersect(&body);
            (!r.is_empty()).then(|| {
                (
                    r,
                    Hit::Widget(view.studio.nodes[i].id.clone(), Trigger::Click),
                )
            })
        })
        .collect()
}
pub(super) fn paint(
    list: &mut DrawList,
    spec: &Spec,
    view: &View,
    a: Rect,
    offset: usize,
    hover: Option<&Hit>,
) {
    let body = painting::content_rect(spec, view, a);
    let base = painting::palette(spec);
    list.push_clip(body);
    let cells = cells(spec, view, a, offset);
    for (ordinal, (r, i)) in cells.iter().enumerate() {
        if r.intersect(&body).is_empty() {
            continue;
        }
        let node = &view.studio.nodes[*i];
        let (p, bg) = painting::item_palette(&base, view, &format!("node:{}", node.id));
        let tile = r.inset(Edges::xy(3.0, 3.0));
        if let Some(bg) = bg {
            list.rounded_rect(tile, 7.0, bg);
        }
        if hover == Some(&Hit::Widget(node.id.clone(), Trigger::Click)) {
            list.rounded_rect(tile, 7.0, p.surface_muted);
        }
        if view.presentation.grid_lines {
            list.rounded_border(tile, 7.0, p.border);
        }
        let size = 32.0_f32.min((r.width() - 12.0).max(8.0)).min(
            (r.height()
                - if view.presentation.show_names {
                    32.0
                } else {
                    12.0
                })
            .max(16.0),
        );
        let group = size
            + if view.presentation.show_names {
                28.0
            } else {
                0.0
            };
        let y = r.top + (r.height() - group) / 2.0;
        if view.presentation.show_icons {
            let icon = Rect::from_size(r.left + (r.width() - size) / 2.0, y, size, size);
            if let Some(path) = shortcut_icon::path(&view.workspace, &node.target) {
                list.image(icon, path, &node.title);
            } else {
                list.icon_centered(icon, Icon::FILE_TEXT, size, p.muted);
            }
        }
        if view.presentation.show_names {
            painting::scaled(
                list,
                Rect::new(
                    r.left + 6.0,
                    if view.presentation.show_icons {
                        y + size + 4.0
                    } else {
                        r.top
                    },
                    r.right - 6.0,
                    if view.presentation.show_icons {
                        y + size + 28.0
                    } else {
                        r.bottom
                    },
                ),
                &node.title,
                spec.appearance.font_size as f32,
                p.foreground,
                Align::Center,
            );
        }
        if view.shortcut_insertion == Some(ordinal) {
            list.rect(
                Rect::from_size(r.left, r.top + 5.0, 3.0, r.height() - 10.0),
                p.accent,
            );
        }
    }
    if view.shortcut_insertion == Some(cells.len()) {
        if let Some((r, _)) = cells.last() {
            list.rect(
                Rect::from_size(r.right - 3.0, r.top + 5.0, 3.0, r.height() - 10.0),
                base.accent,
            );
        }
    }
    if cells.is_empty() {
        painting::scaled(
            list,
            body,
            "拖入文件或桌面图标",
            14.0,
            base.muted,
            Align::Center,
        );
    }
    list.pop_clip();
    let max = scroll_max(spec, view, a);
    if max > 0 {
        let h = (body.height() * body.height() / (body.height() + max as f32)).max(18.0);
        list.rounded_rect(
            Rect::from_size(
                a.right - 5.0,
                body.top + (body.height() - h) * offset.min(max) as f32 / max as f32,
                3.0,
                h,
            ),
            1.5,
            theme::mix(base.muted, base.surface, 0.4),
        );
    }
}
pub(super) fn pointer_down(hwnd: HWND, x: f32, y: f32) {
    unsafe {
        if let Some(s) = state(hwnd) {
            if enabled(&s.view) {
                if let Some(Hit::Widget(id, _)) = painting::hit(s, area(hwnd), x, y) {
                    s.shortcut_drag = Some(Drag {
                        id,
                        start: (x, y),
                        since: Instant::now(),
                        active: false,
                    });
                }
            }
        }
    }
}
pub(super) fn pointer_move(hwnd: HWND, x: f32, y: f32) -> bool {
    unsafe {
        let Some(s) = state(hwnd) else { return false };
        let Some(drag) = s.shortcut_drag.as_mut() else {
            return false;
        };
        if !drag.active
            && drag.since.elapsed() >= Duration::from_millis(350)
            && (x - drag.start.0).hypot(y - drag.start.1) > 4.0
        {
            drag.active = true;
            s.dragging = true;
        }
        if !drag.active {
            return false;
        }
        let a = area(hwnd);
        let body = painting::content_rect(&s.spec, &s.view, a);
        if y < body.top + 20.0 {
            s.offset = s.offset.saturating_sub(12);
        } else if y > body.bottom - 20.0 {
            s.offset = (s.offset + 12).min(scroll_max(&s.spec, &s.view, a));
        }
        let cells = cells(&s.spec, &s.view, a, s.offset);
        s.view.shortcut_insertion = Some(
            cells
                .iter()
                .position(|(r, _)| y < r.bottom && (y < r.top || x < (r.left + r.right) / 2.0))
                .unwrap_or(cells.len()),
        );
        invalidate(hwnd);
        true
    }
}
pub(super) fn pointer_up(hwnd: HWND) -> bool {
    let event = unsafe {
        let Some(s) = state(hwnd) else { return false };
        let Some(drag) = s.shortcut_drag.take() else {
            return false;
        };
        let target = s.view.shortcut_insertion.take();
        s.dragging = false;
        if !drag.active {
            return false;
        }
        s.pressed = None;
        Some(EventKind::ShortcutMove(drag.id, target.unwrap_or(0)))
    };
    unsafe {
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
    }
    if let Some(event) = event {
        emit(hwnd, event);
    }
    invalidate(hwnd);
    true
}
