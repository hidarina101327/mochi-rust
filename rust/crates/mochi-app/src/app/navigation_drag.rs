//! 知识库排序用长按手势触发，与文件移动分开。
use super::*;
use std::time::{Duration, Instant};

const HOLD_MS: u32 = 350;
const MOVE_TOLERANCE: f32 = 6.0;

pub(super) struct LibraryDrag {
    root: PathBuf,
    source: String,
    start: (f32, f32),
    pointer: (f32, f32),
    pressed_at: Instant,
    active: bool,
    cancelled: bool,
}

impl App {
    pub(super) fn begin_navigation_library_drag(&mut self, index: usize, x: f32, y: f32) -> bool {
        let Some(ws) = self.shell.workspace() else {
            return false;
        };
        let Some(id) = self.nav.painted_libraries.get(index) else {
            return false;
        };
        let Some(library) = ws.libraries.iter().find(|l| &l.id == id) else {
            return false;
        };
        if library.kind != navigation::DEFAULT_EXPANDED_TYPE {
            return false;
        }
        self.nav.library_drag = Some(LibraryDrag {
            root: ws.root.clone(),
            source: library.id.clone(),
            start: (x, y),
            pointer: (x, y),
            pressed_at: Instant::now(),
            active: false,
            cancelled: false,
        });
        self.nav.drag_timer = Some(HOLD_MS);
        true
    }

    pub(super) fn update_navigation_library_drag(&mut self, x: f32, y: f32) {
        let Some(drag) = self.nav.library_drag.as_mut() else {
            return;
        };
        drag.pointer = (x, y);
        if drag.cancelled || drag.active {
            return;
        }
        // 长按完成前的移动既不算点击也不算重排序。
        if drag.pressed_at.elapsed() >= Duration::from_millis(u64::from(HOLD_MS)) {
            drag.active = true;
        } else if (x - drag.start.0).hypot(y - drag.start.1) > MOVE_TOLERANCE {
            drag.cancelled = true;
        }
    }

    /// 从最后一帧的绘制结果解析稳定 ID，绝不用实时行号。
    fn navigation_library_drop(&self, x: f32, y: f32) -> Option<(String, bool, Rect)> {
        if !self.nav_layout.scroll_area.contains(x, y) {
            return None;
        }
        let ws = self.shell.workspace()?;
        let (_, hit) = self.nav_layout.entries.iter().find(|(r, hit)| {
            matches!(hit, NavHit::Library(_))
                && x >= r.left
                && x < r.right
                && y >= r.top - 0.5
                && y < r.bottom + 0.5
        })?;
        let NavHit::Library(index) = hit else {
            return None;
        };
        let id = self.nav.painted_libraries.get(*index)?;
        let library = ws.libraries.iter().find(|l| &l.id == id)?;
        if library.kind != navigation::DEFAULT_EXPANDED_TYPE {
            return None;
        }
        let rect = self.nav_layout.rect_of(*hit)?;
        Some((id.clone(), y >= (rect.top + rect.bottom) / 2.0, rect))
    }

    pub(super) fn navigation_library_drag_timer(&mut self) {
        let Some(drag) = self.nav.library_drag.as_ref() else {
            return;
        };
        if self.shell.workspace().is_none_or(|ws| ws.root != drag.root) {
            self.cancel_navigation_library_drag();
            return;
        }
        let (x, y) = drag.pointer;
        self.update_navigation_library_drag(x, y);
        let drag = self.nav.library_drag.as_ref().unwrap();
        if drag.cancelled {
            return;
        }
        if drag.active {
            let area = self.nav_layout.scroll_area;
            if area.contains(x, y) {
                let step = if y < area.top + 28.0 {
                    -10.0
                } else if y > area.bottom - 28.0 {
                    10.0
                } else {
                    0.0
                };
                self.nav.scroll = (self.nav.scroll + step).clamp(0.0, self.nav_layout.max_scroll());
            }
        }
        // 顺带兜底 Windows 定时器过早触发的情况，同时不缩短长按时长。
        self.nav.drag_timer = Some(40);
    }

    pub(super) fn finish_navigation_library_drag(&mut self, x: f32, y: f32) {
        self.update_navigation_library_drag(x, y);
        let Some(drag) = self.nav.library_drag.take() else {
            return;
        };
        self.nav.drag_timer = None;
        if drag.cancelled || self.shell.workspace().is_none_or(|ws| ws.root != drag.root) {
            return;
        }
        let Some((target, after, _)) = self.navigation_library_drop(x, y) else {
            return;
        };
        if drag.active {
            match self.shell.reorder_library(&drag.source, &target, after) {
                Ok(true) => self.invalidate_main(),
                Ok(false) => {}
                Err(error) => self.show_global_notice(&format!("知识库排序保存失败：{error}")),
            }
        } else if target == drag.source {
            let index = self
                .shell
                .workspace()
                .and_then(|ws| ws.libraries.iter().position(|l| l.id == drag.source));
            if let Some(index) = index {
                if self.shell.favorites_selected() {
                    self.side = SidebarState::default();
                }
                self.shell.select_library(index);
                self.state.view = WorkspaceView::Editor;
                self.invalidate_main();
            }
        }
    }

    pub fn cancel_navigation_library_drag(&mut self) -> bool {
        self.nav.drag_timer = None;
        self.nav.library_drag.take().is_some()
    }

    pub(super) fn navigation_library_wheel(&mut self, x: f32, y: f32, delta: i16) -> bool {
        let Some(drag) = self.nav.library_drag.as_mut() else {
            return false;
        };
        if !drag.active {
            drag.cancelled = true;
        }
        drag.pointer = (x, y);
        if self.nav_layout.scroll_area.contains(x, y) {
            self.nav.scroll = (self.nav.scroll - f32::from(delta) / 120.0 * 72.0)
                .clamp(0.0, self.nav_layout.max_scroll());
        }
        true
    }

    pub(super) fn navigation_library_drag_active(&self) -> bool {
        self.nav
            .library_drag
            .as_ref()
            .is_some_and(|drag| drag.active && !drag.cancelled)
    }

    pub(super) fn paint_navigation_library_drag(&mut self, p: &Palette) {
        let Some(drag) = self
            .nav
            .library_drag
            .as_ref()
            .filter(|d| d.active && !d.cancelled)
        else {
            return;
        };
        self.list.push_clip(self.nav_layout.scroll_area);
        if let Some(index) = self
            .nav
            .painted_libraries
            .iter()
            .position(|id| id == &drag.source)
        {
            if let Some(rect) = self.nav_layout.rect_of(NavHit::Library(index)) {
                self.list.rounded_rect_alpha(rect, 6.0, p.accent, 0.12);
                self.list.rounded_border(rect, 6.0, p.accent);
            }
        }
        if let Some((target, after, rect)) =
            self.navigation_library_drop(drag.pointer.0, drag.pointer.1)
        {
            if target != drag.source {
                let y = if after { rect.bottom } else { rect.top };
                self.list.rect(
                    Rect::new(rect.left + 8.0, y - 1.0, rect.right - 8.0, y + 1.0),
                    p.accent,
                );
            }
        }
        self.list.pop_clip();
    }
}

#[cfg(all(test, debug_assertions))]
mod tests;
