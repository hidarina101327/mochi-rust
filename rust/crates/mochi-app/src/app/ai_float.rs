//! 把右侧栏里的 AI 助手收成墨池窗口内的悬浮窗。
//!
//! 绘制、点击、滚动都走原来的 `assistant` 面板，这里只负责一块不同大小的区域，
//! 以及拖动标题栏移动、拖动右下角改大小。
use super::*;

const FLOAT_W: f32 = 440.0;
const FLOAT_H: f32 = 640.0;

impl App {
    /// 侧栏里的「悬浮窗」：收起右侧栏，在窗口里打开一块较小的同一面板。
    /// 悬浮窗上再点一次，回到右侧栏。
    pub(super) fn toggle_ai_float(&mut self) {
        if self.ai.float.take().is_some() {
            self.state.ai_panel_open = true;
            self.set_right_panel(RightPanel::Assistant);
            self.focus = Focus::AiInput;
            self.invalidate_main();
            return;
        }
        let view = self.renderer.viewport();
        let mut float = AiFloat {
            x: view.right - FLOAT_W - 28.0,
            y: view.top + 64.0,
            w: FLOAT_W.min(view.width() - 32.0),
            h: FLOAT_H.min(view.height() - 80.0),
            drag: None,
        };
        float.clamp_into(view);
        self.ai.float = Some(float);
        self.state.ai_panel_open = false;
        self.focus = Focus::AiInput;
        self.invalidate_main();
    }

    pub(super) fn paint_ai_float(&mut self, p: &Palette) {
        let Some(mut float) = self.ai.float else {
            return;
        };
        let view = self.renderer.viewport();
        float.clamp_into(view);
        self.ai.float = Some(float);
        let rect = float.rect();
        let shadow = Rect::new(
            rect.left + 8.0,
            rect.top + 10.0,
            rect.right + 8.0,
            rect.bottom + 14.0,
        );
        self.list.rounded_rect_alpha(shadow, 16.0, 0x000000, 0.16);
        self.list.rounded_rect(rect, 12.0, p.background);
        self.list.rounded_border(rect, 12.0, p.border);
        self.paint_assistant_surface(rect, p);
        // 右下角的缩放提示，不挡输入框时才画。
        if self
            .ai
            .layout
            .hit(rect.right - 8.0, rect.bottom - 8.0)
            .is_none()
        {
            self.list.icon_centered(
                Rect::new(
                    rect.right - 18.0,
                    rect.bottom - 18.0,
                    rect.right - 2.0,
                    rect.bottom - 2.0,
                ),
                Icon::MAXIMIZE2,
                11.0,
                p.muted,
            );
        }
    }

    /// 右侧栏和悬浮窗共用这一次排版和绘制。
    pub(super) fn paint_assistant_surface(&mut self, body: Rect, p: &Palette) {
        self.ai.panel.standalone = false;
        self.ai.panel.automatic_edits =
            crate::ui::settings_values::text("ai.editApplyMode", "approve") == "auto";
        self.ai.panel.provider_missing = ai_runtime::load_provider(&self.settings).is_none();
        self.ai.panel.search_focused = self.focus == Focus::AiMessageQuery;
        let layout_scroll = self.ai.panel.scroll;
        let mut lay = assistant::layout(&self.ai.panel, body);
        self.ai.panel.horizontal.sync(&lay);
        self.ai.panel.apply_locator(&lay);
        self.ai.panel.scroll = self.ai.panel.scroll.min(lay.max_scroll());
        if layout_scroll != self.ai.panel.scroll {
            lay = assistant::layout(&self.ai.panel, body);
        }
        if let Some(r) = lay.rect_of(assistant::Hit::SearchInput) {
            self.ai
                .panel
                .search_query
                .sync_singleline_scroll(r.width() - 138.0);
        }
        if self.focus == Focus::AiInput {
            if let Some(r) = lay.rect_of(assistant::Hit::Input) {
                self.ai.panel.input.sync_multiline_scroll(r);
            }
        }
        assistant::paint(
            &mut self.list,
            body,
            &self.ai.panel,
            &lay,
            self.focus == Focus::AiInput,
            p,
        );
        self.ai.layout = lay;
    }

    /// 点在悬浮窗里就由 AI 面板处理，不再落到下面的页面。
    pub(super) fn ai_float_click(&mut self, x: f32, y: f32) -> bool {
        let Some(float) = self.ai.float else {
            return false;
        };
        let rect = float.rect();
        if !rect.contains(x, y) {
            return false;
        }
        let hit = self.ai.layout.hit(x, y);
        let corner = rect.right - x <= 16.0 && rect.bottom - y <= 16.0 && hit.is_none();
        if corner {
            if let Some(float) = self.ai.float.as_mut() {
                float.drag = Some(AiFloatDrag::Resize {
                    right: float.x + float.w - x,
                    bottom: float.y + float.h - y,
                });
            }
            return true;
        }
        if hit == Some(assistant::Hit::Header) {
            if let Some(float) = self.ai.float.as_mut() {
                float.drag = Some(AiFloatDrag::Move {
                    dx: x - float.x,
                    dy: y - float.y,
                });
            }
            return true;
        }
        self.on_assistant_click(HWND(self.hwnd_raw as *mut _), x, y);
        true
    }

    pub(super) fn ai_float_drag_to(&mut self, x: f32, y: f32) -> bool {
        let Some(drag) = self.ai.float.as_ref().and_then(|float| float.drag) else {
            return false;
        };
        let view = self.renderer.viewport();
        let Some(float) = self.ai.float.as_mut() else {
            return false;
        };
        match drag {
            AiFloatDrag::Move { dx, dy } => {
                float.x = x - dx;
                float.y = y - dy;
            }
            AiFloatDrag::Resize { right, bottom } => {
                float.w = x + right - float.x;
                float.h = y + bottom - float.y;
            }
        }
        float.clamp_into(view);
        true
    }

    pub(super) fn ai_float_end_drag(&mut self) {
        if let Some(float) = self.ai.float.as_mut() {
            float.drag = None;
        }
    }

    pub(super) fn ai_float_cursor(&self, x: f32, y: f32) -> Option<PCWSTR> {
        let float = self.ai.float?;
        let rect = float.rect();
        if float.drag.is_none() && !rect.contains(x, y) {
            return None;
        }
        let messaging = windows::Win32::UI::WindowsAndMessaging::IDC_ARROW;
        Some(match float.drag {
            Some(AiFloatDrag::Move { .. }) => windows::Win32::UI::WindowsAndMessaging::IDC_SIZEALL,
            Some(AiFloatDrag::Resize { .. }) => {
                windows::Win32::UI::WindowsAndMessaging::IDC_SIZENWSE
            }
            None if rect.right - x <= 16.0
                && rect.bottom - y <= 16.0
                && self.ai.layout.hit(x, y).is_none() =>
            {
                windows::Win32::UI::WindowsAndMessaging::IDC_SIZENWSE
            }
            None if self.ai.layout.hit(x, y) == Some(assistant::Hit::Header) => {
                windows::Win32::UI::WindowsAndMessaging::IDC_SIZEALL
            }
            None if matches!(
                self.ai.layout.hit(x, y),
                Some(assistant::Hit::Input | assistant::Hit::SearchInput)
            ) =>
            {
                windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM
            }
            None if self.ai.layout.hit(x, y).is_some_and(|hit| {
                !matches!(
                    hit,
                    assistant::Hit::Messages
                        | assistant::Hit::Header
                        | assistant::Hit::PopoverInside
                )
            }) =>
            {
                windows::Win32::UI::WindowsAndMessaging::IDC_HAND
            }
            _ => messaging,
        })
    }
}
