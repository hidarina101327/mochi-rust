//! 维护窗口过渡和悬停动画的状态与进度。
use super::*;
use std::time::Instant;

const ENTER_MS: f32 = 160.0;
const HOVER_MS: f32 = 110.0;

#[derive(Default)]
pub(super) struct State {
    active: [bool; 8],
    starts: [Option<Instant>; 8],
    pub keyboard: bool,
    pub pointer_pos: Option<(f32, f32)>,
    hover: Option<Rect>,
    fades: Vec<(Rect, f32, f32, Instant)>,
}

impl State {
    pub fn sync(&mut self, active: [bool; 8], animate: bool) {
        if active != self.active {
            self.hover = None;
            self.fades.clear();
        }
        for (i, shown) in active.into_iter().enumerate() {
            if !shown || !animate {
                self.starts[i] = None;
            } else if !self.active[i] {
                self.starts[i] = Some(Instant::now());
            }
            self.active[i] = shown;
        }
        self.fades.retain(|(_, _, to, start)| {
            *to > 0.0 || start.elapsed().as_secs_f32() * 1000.0 < HOVER_MS
        });
    }
    pub fn opacity(&self, index: usize) -> f32 {
        self.starts[index]
            .map(|start| ease(start.elapsed().as_secs_f32() * 1000.0 / ENTER_MS))
            .unwrap_or(1.0)
    }
    pub fn animating(&self) -> bool {
        (0..8).any(|i| self.opacity(i) < 1.0)
            || self.fades.iter().any(|(_, from, to, start)| {
                from != to && start.elapsed().as_secs_f32() * 1000.0 < HOVER_MS
            })
    }
    pub fn pointer(&mut self, rect: Option<Rect>) -> bool {
        if self.hover == rect {
            return false;
        }
        self.hover = rect;
        let now = Instant::now();
        for (r, from, to, start) in &mut self.fades {
            *from += (*to - *from) * ease(start.elapsed().as_secs_f32() * 1000.0 / HOVER_MS);
            *to = if Some(*r) == rect { 1.0 } else { 0.0 };
            *start = now;
        }
        if let Some(r) = rect.filter(|r| !self.fades.iter().any(|(v, ..)| v == r)) {
            self.fades.push((r, 0.0, 1.0, now));
        }
        true
    }
    pub fn paint_hover(&self, list: &mut DrawList, p: &Palette) {
        for (r, from, to, start) in &self.fades {
            let strength =
                from + (to - from) * ease(start.elapsed().as_secs_f32() * 1000.0 / HOVER_MS);
            list.rounded_rect_alpha(
                *r,
                5.0,
                if theme::is_dark(p) {
                    0xffffff
                } else {
                    p.accent
                },
                strength * 0.09,
            );
        }
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn window_motion_finishes_without_idle_timer_and_does_not_restart_on_repaint() {
        let mut motion = State::default();
        let open = [true, false, false, false, false, false, false, false];
        motion.sync(open, true);
        assert!(motion.opacity(0) < 1.0);
        let started = motion.starts[0];
        motion.sync(open, true);
        assert_eq!(motion.starts[0], started);
        motion.starts[0] = Some(Instant::now() - Duration::from_millis(200));
        assert_eq!(motion.opacity(0), 1.0);
        assert!(!motion.animating());
        motion.sync([false; 8], true);
        motion.sync(open, false);
        assert_eq!(motion.opacity(0), 1.0);
        assert!(!motion.animating());
        motion.pointer(Some(Rect::from_size(10., 10., 40., 30.)));
        motion.sync([false; 8], true);
        assert!(motion.hover.is_none() && motion.fades.is_empty());
    }

    #[test]
    fn window_motion_d2d_layers_render_real_settings_without_changing_hit_regions() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-window-motion-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            let snapshot = app.renderer.prepare_snapshot(1200, 800, 96.).unwrap();
            app.open_settings("notifications");
            let palette = theme::configured_palette(false);
            let viewport = app.renderer.viewport();
            let mut reference = None;
            for opacity in [0.25, 0.7, 1.0] {
                app.list.clear();
                app.list.rect(viewport, 0x64748b);
                let start = app.list.cmds().len();
                app.paint_settings_overlay(&palette);
                let rect = app.prefs.nav_layout.entries[0].0;
                if let Some(previous) = reference {
                    assert_eq!(rect, previous);
                }
                reference = Some(rect);
                app.list.fade_since(start, viewport, opacity);
                assert!(app.list.finish().is_ok());
                app.renderer
                    .present(HWND::default(), palette.background, &app.list)
                    .unwrap();
                if let Some(output) = std::env::var_os("MOCHI_WINDOW_MOTION_QA") {
                    let output = PathBuf::from(output);
                    std::fs::create_dir_all(&output).unwrap();
                    app.renderer
                        .save_snapshot(&snapshot, &output.join(format!("settings-{opacity}.png")))
                        .unwrap();
                }
            }
            let rect = app.prefs.nav_layout.entries[1].0;
            assert!(app.on_mouse_move(rect.left + 8., rect.top + 8.));
            assert_eq!(app.window_motion.hover, Some(rect));
            assert!(app
                .take_timer_requests()
                .iter()
                .any(|(id, _)| *id == platform::TIMER_WINDOW_MOTION));
            app.close_settings();
            app.paint(HWND::default()).unwrap();
            assert_ne!(app.window_motion.hover, Some(rect));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

impl App {
    pub(super) fn window_hover_rect(&self, x: f32, y: f32) -> Option<Rect> {
        if self.global_import.is_some() {
            return None;
        }
        let viewport = self.renderer.viewport();
        if self.menu.is_some()
            || self.command.is_some()
            || self.search.is_some()
            || self.image_preview.is_some()
            || self.commands.review.is_some()
            || self.export_form.is_some()
            || self.template_picker.is_some()
        {
            return None;
        }
        if self.notification_open() {
            return self
                .notification_layout()
                .controls
                .into_iter()
                .find(|(r, _)| r.contains(x, y))
                .map(|(r, _)| r);
        }
        if let Some(dialog) = &self.dialog {
            let (_, _, _, buttons, close) = dialog.parts(viewport);
            return std::iter::once(close)
                .chain(buttons)
                .find(|r| r.contains(x, y));
        }
        if self.link_create.is_some()
            || self.mapped_folder.is_some()
            || self.object_picker.is_some()
        {
            return None;
        }
        let Some((tab, section)) = self.settings_overlay.as_ref() else {
            if self.automation.panel.is_some() || self.base_detail_open() {
                return None;
            }
            if self.state.view == WorkspaceView::MochiAi {
                if let Some(hit @ assistant::Hit::NavigateMessage(_)) = self.ai.layout.hit(x, y) {
                    return self.ai.layout.rect_of(hit).and_then(|rect| {
                        let nav = self.ai.layout.navigation_rect?;
                        Some(rect.intersect(&Rect::new(
                            nav.left,
                            nav.top + 48.0,
                            nav.right,
                            nav.bottom,
                        )))
                    });
                }
            }
            if self.content() == MainContent::Document {
                if let Some(rect) = outline::hover_rect(
                    self.outline_area,
                    self.doc.headings().len(),
                    self.outline_scroll,
                    x,
                    y,
                ) {
                    return Some(rect);
                }
            }
            let hit = self.nav_layout.hit(x, y)?;
            return matches!(
                hit,
                NavHit::Settings
                    | NavHit::Search
                    | NavHit::Collapse
                    | NavHit::Item(_)
                    | NavHit::Library(_)
                    | NavHit::TypeHeader(_)
                    | NavHit::TypeAdd(_)
                    | NavHit::TypeIcon(_)
                    | NavHit::PluginEntry(_)
            )
            .then(|| self.nav_layout.rect_of(hit))
            .flatten();
        };
        let modal = self.settings_overlay_rect();
        if !modal.contains(x, y) {
            return None;
        }
        let close = settings::close_rect(modal);
        if close.contains(x, y) {
            return Some(close);
        }
        if let Some((r, _)) = self
            .prefs
            .nav_layout
            .entries
            .iter()
            .find(|(r, _)| r.contains(x, y))
        {
            return Some(*r);
        }
        if Self::is_provider_settings(tab, section) {
            return None;
        }
        let lay = &self.prefs.content_layout;
        if lay.update_check_rect.contains(x, y) {
            return Some(lay.update_check_rect);
        }
        if let Some(i) = lay.hit(x, y) {
            return lay.control_rect(i);
        }
        if lay.body.contains(x, y) {
            if let Some((r, _)) = lay
                .navigation
                .entries
                .iter()
                .find(|(r, _)| r.contains(x, y))
            {
                return Some(*r);
            }
            if let Some((r, _)) = lay.shortcuts.iter().find(|(r, _)| r.contains(x, y)) {
                return Some(*r);
            }
        }
        lay.background_buttons
            .iter()
            .find(|(r, _)| r.contains(x, y) && lay.body.contains(x, y))
            .map(|(r, _)| *r)
    }
}
