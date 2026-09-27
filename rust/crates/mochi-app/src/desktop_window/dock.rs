//! 边缘抽屉始终显示在其他应用上方；关闭时只露出窄窄的把手。
use super::*;
use std::time::{Duration, Instant};
pub(super) const TIMER_ID: usize = 0x4d44_4f43;
#[derive(Default)]
pub(super) struct DockState {
    pub progress: f32,
    pub suspended: bool,
    pub leave_at: Option<Instant>,
    pub last: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}
pub(super) fn edge_position(
    spec: &Spec,
    work: RECT,
    scale: f32,
    progress: f32,
) -> (i32, i32, Edge) {
    let width = (spec.width as f32 * scale) as i32;
    let height = (spec.height as f32 * scale) as i32;
    let distances = [
        ((spec.x - work.left).abs(), Edge::Left),
        ((work.right - spec.x - width).abs(), Edge::Right),
        ((spec.y - work.top).abs(), Edge::Top),
        ((work.bottom - spec.y - height).abs(), Edge::Bottom),
    ];
    let edge = distances
        .into_iter()
        .min_by_key(|(distance, _)| *distance)
        .unwrap()
        .1;
    let mut x = spec.x.clamp(work.left, (work.right - width).max(work.left));
    let mut y = spec.y.clamp(work.top, (work.bottom - height).max(work.top));
    let t = progress.clamp(0.0, 1.0);
    let ease = 1.0 - (1.0 - t).powi(3);
    let handle = (4.0 * scale) as i32;
    match edge {
        Edge::Left => x = work.left - width + handle + ((width - handle) as f32 * ease) as i32,
        Edge::Right => x = work.right - handle - ((width - handle) as f32 * ease) as i32,
        Edge::Top => y = work.top - height + handle + ((height - handle) as f32 * ease) as i32,
        Edge::Bottom => y = work.bottom - handle - ((height - handle) as f32 * ease) as i32,
    }
    (x, y, edge)
}
pub(super) fn configure(hwnd: HWND) {
    unsafe {
        let floating = {
            let Some(s) = state(hwnd) else { return };
            if s.context_active {
                return;
            }
            s.allow_z = true;
            s.spec.appearance.edge_dock || s.spec.appearance.pinned
        };
        let _ = SetWindowPos(
            hwnd,
            Some(if floating {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            }),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        let Some(s) = state(hwnd) else { return };
        s.allow_z = false;
        if s.spec.appearance.edge_dock && !s.dock.suspended {
            let _ = SetTimer(Some(hwnd), TIMER_ID, 80, None);
        } else {
            let _ = KillTimer(Some(hwnd), TIMER_ID);
            if !s.spec.appearance.edge_dock {
                s.dock = DockState::default();
            }
        }
    }
}
pub(super) fn tick(hwnd: HWND) {
    unsafe {
        let Some(s) = state(hwnd) else {
            return;
        };
        if !s.spec.appearance.edge_dock || s.dock.suspended || s.dragging || s.context_active {
            return;
        }
        let monitor = MonitorFromPoint(
            POINT {
                x: s.spec.x,
                y: s.spec.y,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let (open_x, open_y, edge) = edge_position(&s.spec, info.rcWork, scale, 1.0);
        let width = (s.spec.width as f32 * scale) as i32;
        let height = (s.spec.height as f32 * scale) as i32;
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let near = match edge {
            Edge::Left => {
                cursor.x >= info.rcWork.left
                    && cursor.x <= info.rcWork.left + 6
                    && cursor.y >= open_y
                    && cursor.y <= open_y + height
            }
            Edge::Right => {
                cursor.x <= info.rcWork.right
                    && cursor.x >= info.rcWork.right - 6
                    && cursor.y >= open_y
                    && cursor.y <= open_y + height
            }
            Edge::Top => {
                cursor.y >= info.rcWork.top
                    && cursor.y <= info.rcWork.top + 6
                    && cursor.x >= open_x
                    && cursor.x <= open_x + width
            }
            Edge::Bottom => {
                cursor.y <= info.rcWork.bottom
                    && cursor.y >= info.rcWork.bottom - 6
                    && cursor.x >= open_x
                    && cursor.x <= open_x + width
            }
        };
        let within = cursor.x >= open_x
            && cursor.x < open_x + width
            && cursor.y >= open_y
            && cursor.y < open_y + height;
        let now = Instant::now();
        let elapsed = s.dock.last.map_or(0.08, |last| {
            now.duration_since(last).as_secs_f32().min(0.08)
        });
        s.dock.last = Some(now);
        let editing = s
            .composer
            .is_some_and(|edit| windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == edit);
        let keep = s.spec.appearance.pinned || editing || near || (s.dock.progress > 0.0 && within);
        if keep {
            s.dock.leave_at = None;
        } else if s.dock.leave_at.is_none() {
            s.dock.leave_at = Some(now);
        }
        let opening = keep
            || s.dock.leave_at.is_some_and(|left| {
                now.duration_since(left)
                    < Duration::from_millis(
                        [0, 100, 350, 800][s.spec.appearance.dock_speed.min(3) as usize],
                    )
            });
        let before = s.dock.progress;
        s.dock.progress = (before
            + if opening {
                elapsed / 0.20
            } else {
                -elapsed / [0.001, 0.16, 0.36, 0.70][s.spec.appearance.dock_speed.min(3) as usize]
            })
        .clamp(0.0, 1.0);
        let (x, y, _) = edge_position(&s.spec, info.rcWork, scale, s.dock.progress);
        let mut current = RECT::default();
        let _ = GetWindowRect(hwnd, &mut current);
        if current.left != x
            || current.top != y
            || current.right - current.left != width
            || current.bottom - current.top != height
        {
            s.allow_z = true;
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE,
            );
            s.allow_z = false;
        }
        let _ = SetTimer(
            Some(hwnd),
            TIMER_ID,
            if s.dock.progress > 0.0 && s.dock.progress < 1.0 {
                16
            } else {
                80
            },
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drawer_positions_support_negative_monitors_and_keep_a_handle() {
        let spec = Spec {
            appearance: Default::default(),
            id: "test".into(),
            title: String::new(),
            x: -1920,
            y: 100,
            width: 400,
            height: 400,
            locked: false,
            dark: false,
        };
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        assert_eq!(
            edge_position(&spec, work, 1.0, 0.0),
            (-2316, 100, Edge::Left)
        );
        assert_eq!(
            edge_position(&spec, work, 1.0, 1.0),
            (-1920, 100, Edge::Left)
        );
        let halfway = edge_position(&spec, work, 1.0, 0.5).0;
        assert!(halfway > -2316 && halfway < -1920);
    }
}

pub(super) fn toggle_suspended(hwnd: HWND) {
    unsafe {
        if let Some(s) = state(hwnd) {
            s.dock.suspended = !s.dock.suspended;
            s.dock.progress = 1.0;
            s.dock.leave_at = None;
            s.dock.last = None;
            if s.dock.suspended {
                let mut shown = s.spec.clone();
                let monitor = MonitorFromPoint(
                    POINT {
                        x: shown.x,
                        y: shown.y,
                    },
                    MONITOR_DEFAULTTONEAREST,
                );
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(monitor, &mut info).as_bool() {
                    let (x, y, _) = edge_position(
                        &shown,
                        info.rcWork,
                        GetDpiForWindow(hwnd).max(96) as f32 / 96.0,
                        1.0,
                    );
                    shown.x = x;
                    shown.y = y;
                }
                place(hwnd, &shown);
            }
        }
    }
    configure(hwnd);
    invalidate(hwnd);
}
