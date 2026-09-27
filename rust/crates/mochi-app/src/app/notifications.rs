//! 读取通知设置，并维护通知状态与合并规则。
use super::*;
use crate::ui::notifications::{self as ui, Category, Hit, Mode};

const HISTORY_KEY: &str = "notifications.history";
const WINDOWS_KEY: &str = "notifications.windowsEnabled";
const AGGREGATE_KEY: &str = "notifications.aggregateEnabled";
const BURST_GAP: std::time::Duration = std::time::Duration::from_secs(5);

fn enabled(settings: &AppSettings, key: &str) -> bool {
    matches!(
        app_settings::descriptor(key).map(|d| settings.read(d)),
        Some(SettingValue::Bool(true))
    )
}

pub(super) struct State {
    pub view: ui::State,
    pub save_timer: Option<u32>,
    previous_focus: Option<Focus>,
    pub title_hover: Option<NodeKey>,
    pending_native: Option<ui::Notice>,
    native: platform::notifications::Notifier,
    native_category: Option<Category>,
    last_native_activity: Option<std::time::Instant>,
}

impl State {
    pub fn new(settings: &Arc<SettingsService>) -> Self {
        let preferences = AppSettings::new(settings.clone());
        Self {
            view: ui::State {
                history: ui::History::restore(settings.get(HISTORY_KEY).as_deref()),
                muted: Category::ALL
                    .into_iter()
                    .filter(|c| !enabled(&preferences, c.setting_key()))
                    .collect(),
                ..Default::default()
            },
            save_timer: None,
            previous_focus: None,
            title_hover: None,
            pending_native: None,
            native: Default::default(),
            native_category: None,
            last_native_activity: None,
        }
    }
}

impl App {
    pub(super) fn notification_open(&self) -> bool {
        self.notifications.view.is_open()
    }

    pub(super) fn publish_notification(&mut self, category: Category, title: &str, message: &str) {
        let previous_read = self
            .notifications
            .view
            .history
            .entries
            .first()
            .is_some_and(|n| n.read);
        if let Some(id) =
            self.notifications
                .view
                .history
                .push(category, title, message, Self::now_ms())
        {
            self.notifications.save_timer.get_or_insert(500);
            if !self.notification_category_enabled(category) {
                self.notifications.view.history.mark_read(id);
                return;
            }
            let notice = &self.notifications.view.history.entries[0];
            let now = std::time::Instant::now();
            let windows_enabled = enabled(&self.app_settings, WINDOWS_KEY);
            let in_burst = enabled(&self.app_settings, AGGREGATE_KEY)
                && self
                    .notifications
                    .last_native_activity
                    .is_some_and(|last| now.duration_since(last) < BURST_GAP);
            if windows_enabled {
                // 滑动静默期：即使消息被抑制，也会顺延当前这波突发。
                self.notifications.last_native_activity = Some(now);
            }
            if (notice.occurrences == 1 || previous_read) && windows_enabled && !in_burst {
                // 一波突发里只弹第一条提醒；后续消息都留在历史记录中。
                self.notifications.pending_native = Some(notice.clone());
                if self.hwnd_raw != 0 {
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(HWND(self.hwnd_raw as *mut _)),
                            platform::WM_APP_NOTIFICATION_DELIVER,
                            windows::Win32::Foundation::WPARAM(0),
                            windows::Win32::Foundation::LPARAM(0),
                        );
                    }
                }
            }
        }
    }

    pub(super) fn notification_category_enabled(&self, category: Category) -> bool {
        enabled(&self.app_settings, category.setting_key())
    }

    pub(super) fn refresh_notification_preferences(&mut self) {
        let muted: Vec<_> = Category::ALL
            .into_iter()
            .filter(|c| !self.notification_category_enabled(*c))
            .collect();
        for notice in &mut self.notifications.view.history.entries {
            if !notice.read && muted.contains(&notice.category) {
                notice.read = true;
                self.notifications.save_timer.get_or_insert(500);
            }
        }
        let windows_enabled = enabled(&self.app_settings, WINDOWS_KEY);
        if self
            .notifications
            .pending_native
            .as_ref()
            .is_some_and(|n| !windows_enabled || muted.contains(&n.category))
        {
            self.notifications.pending_native = None;
        }
        if !windows_enabled
            || self
                .notifications
                .native_category
                .is_some_and(|c| muted.contains(&c))
        {
            self.notifications.native.clear();
            self.notifications.native_category = None;
            self.notifications.view.windows_failed = false;
        }
        if !windows_enabled || !enabled(&self.app_settings, AGGREGATE_KEY) {
            self.notifications.last_native_activity = None;
        }
        if self.status_bar.toast.progress.is_none()
            && muted.contains(&Category::for_status(&self.status_bar.toast.message))
        {
            self.status_bar.toast = Default::default();
        }
        self.notifications.view.muted = muted;
    }

    fn take_native_notification(&mut self) -> Option<ui::Notice> {
        self.notifications.pending_native.take().filter(|notice| {
            enabled(&self.app_settings, WINDOWS_KEY)
                && self.notification_category_enabled(notice.category)
        })
    }

    pub fn deliver_native_notification(&mut self, hwnd: HWND) {
        let Some(notice) = self.take_native_notification() else {
            return;
        };
        if std::env::var_os("MOCHI_VERIFY_OFFSCREEN").is_some() {
            return;
        }
        let title = format!("墨池 · {} · {}", notice.category.label(), notice.title);
        let result = self
            .notifications
            .native
            .show(hwnd, &title, &notice.message);
        self.notifications.view.windows_failed = result.is_err();
        if let Err(error) = result {
            eprintln!("Windows 通知发送失败：{error}");
        }
        self.notifications.native_category = Some(notice.category);
    }

    pub fn open_notification_center(&mut self) {
        if !self.notification_open() {
            self.toggle_notifications();
        }
        self.notifications.view.open(Mode::Center);
    }

    pub(super) fn record_status_notification(&mut self, message: &str) {
        if ["已复制", "已剪切", "已粘贴", "已打开"]
            .iter()
            .any(|prefix| message.starts_with(prefix))
        {
            return;
        }
        let category = Category::for_status(message);
        let title = if ["失败", "错误", "无法"].iter().any(|s| message.contains(s)) {
            "操作未完成"
        } else {
            "操作通知"
        };
        self.publish_notification(category, title, message);
    }

    pub(super) fn save_notifications(&mut self) {
        self.notifications.save_timer = None;
        if let Ok(raw) = serde_json::to_string(&self.notifications.view.history.entries) {
            self.settings.set(HISTORY_KEY, &raw);
            self.notifications.view.save_failed = self.settings.flush().is_err();
            if self.notifications.view.save_failed {
                self.notifications.save_timer = Some(5000);
            }
        }
    }

    pub(super) fn notification_layout(&self) -> ui::Layout {
        let chrome = Chrome::build(&self.state, self.renderer.viewport());
        ui::layout(
            &self.notifications.view,
            self.renderer.viewport(),
            chrome.tree.rect(chrome.notifications_toggle),
        )
    }

    pub(super) fn toggle_notifications(&mut self) {
        if self.notification_open() {
            self.close_notifications();
            return;
        }
        self.on_ime_cancel();
        self.high_surrogate = None;
        self.notifications.previous_focus = Some(self.focus);
        self.menu = None;
        self.toolbar_menu = None;
        self.table_picker = None;
        self.wiki_suggestion = None;
        self.notifications.view.open(Mode::Popover);
    }

    pub(super) fn close_notifications(&mut self) {
        self.notifications.view.close();
        if let Some(focus) = self.notifications.previous_focus.take() {
            self.focus = focus;
        }
        self.high_surrogate = None;
        self.input_activity();
    }

    fn notification_activate(&mut self, hit: Hit) {
        if hit == Hit::Close {
            self.close_notifications();
            return;
        }
        if let Hit::ToggleCategory(category) = hit {
            let was_enabled = self.notification_category_enabled(category);
            if let Some(descriptor) = app_settings::descriptor(category.setting_key()) {
                self.app_settings
                    .write(descriptor, &SettingValue::Bool(!was_enabled));
            }
            self.refresh_notification_preferences();
            crate::ui::settings_values::load(&self.app_settings);
            self.save_notifications();
            return;
        }
        if hit == Hit::Settings {
            self.close_notifications();
            self.open_settings("notifications");
            return;
        }
        if self.notifications.view.activate(hit) {
            self.notifications.save_timer.get_or_insert(500);
        }
    }

    pub(super) fn notification_click(&mut self, x: f32, y: f32) {
        let lay = self.notification_layout();
        if let Some((_, offset)) = self.notifications.view.scrollbar.begin(&lay.bars(), x, y) {
            self.notifications.view.scroll = offset;
        } else if let Some(hit) = lay.hit(x, y) {
            self.notification_activate(hit);
        } else if !lay.frame.contains(x, y) {
            self.close_notifications();
        }
    }

    pub(super) fn notification_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        match key {
            0x1b => {
                if self.notifications.view.selected.is_some() {
                    self.notification_activate(Hit::Back);
                } else {
                    self.close_notifications();
                }
            }
            0x09 | 0x26 | 0x28 if !ctrl => {
                let lay = self.notification_layout();
                self.notifications
                    .view
                    .navigate(&lay, key == 0x26 || (key == 0x09 && shift));
            }
            0x0d | 0x20 if !ctrl => {
                if let Some(hit) = self.notifications.view.focused {
                    self.notification_activate(hit);
                }
            }
            0x21 | 0x22 | 0x24 | 0x23 => {
                let lay = self.notification_layout();
                self.notifications.view.scroll = match key {
                    0x24 => 0.0,
                    0x23 => lay.max_scroll,
                    0x21 => (lay.scroll - lay.body.height()).max(0.0),
                    _ => (lay.scroll + lay.body.height()).min(lay.max_scroll),
                };
            }
            _ => {}
        }
        true
    }

    pub(super) fn notification_pointer(&mut self, x: f32, y: f32) -> bool {
        let lay = self.notification_layout();
        let hover = lay.hit(x, y);
        let changed = self.notifications.view.hover != hover;
        self.notifications.view.hover = hover;
        let bars = lay.bars();
        let changed = self.notifications.view.scrollbar.pointer(&bars, x, y) || changed;
        if let Some((_, offset)) = self.notifications.view.scrollbar.drag_to(&bars, x, y) {
            self.notifications.view.scroll = offset;
            return true;
        }
        changed
    }

    pub(super) fn notification_wheel(&mut self, x: f32, y: f32, delta: i16) {
        let lay = self.notification_layout();
        if lay.body.contains(x, y) {
            self.notifications.view.scroll =
                (lay.scroll - f32::from(delta) / 120.0 * 72.0).clamp(0.0, lay.max_scroll);
        }
    }

    pub(super) fn paint_notification_chrome(&mut self, chrome: &Chrome, p: &Palette) {
        let bell = chrome.tree.rect(chrome.notifications_toggle);
        let robot = chrome.tree.rect(chrome.ai_toggle);
        let desktop = chrome.tree.rect(chrome.desktop_toggle);
        for (key, rect, icon) in [
            (
                NodeKey::TitleBarBack,
                chrome.tree.rect(chrome.back),
                Icon::ARROW_LEFT,
            ),
            (
                NodeKey::TitleBarThemeToggle,
                chrome.tree.rect(chrome.theme_toggle),
                if self.state.dark {
                    Icon::SUN
                } else {
                    Icon::MOON
                },
            ),
            (
                NodeKey::TitleBarAutomations,
                chrome.tree.rect(chrome.automations_toggle),
                Icon::GIT_BRANCH,
            ),
            (
                NodeKey::TitleBarTemplates,
                chrome.tree.rect(chrome.templates_toggle),
                Icon::LAYOUT_GRID,
            ),
            (
                NodeKey::TitleBarMarketplace,
                chrome.tree.rect(chrome.marketplace_toggle),
                Icon::BLOCKS,
            ),
            (NodeKey::TitleBarDesktop, desktop, Icon::MONITOR),
            (NodeKey::TitleBarNotifications, bell, Icon::BELL),
            (NodeKey::TitleBarAiToggle, robot, Icon::BOT),
        ] {
            if rect.is_empty() {
                continue;
            }
            if self.notifications.title_hover == Some(key)
                || (key == NodeKey::TitleBarNotifications && self.notification_open())
            {
                self.list.rounded_rect(
                    rect.inset(crate::ui::layout::Edges::xy(3.0, 3.0)),
                    4.0,
                    p.surface_muted,
                );
                self.list.icon_centered(rect, icon, 18.0, p.foreground);
            }
        }
        if !bell.is_empty() && self.notifications.view.history.unread() > 0 {
            let dot = Rect::from_size(bell.right - 10.0, bell.top + 5.0, 6.0, 6.0);
            self.list.rounded_rect(
                dot.inset(crate::ui::layout::Edges::xy(-2.0, -2.0)),
                5.0,
                p.surface,
            );
            self.list.rounded_rect(dot, 3.0, ui::blue(p));
        }
        if self.notification_open() {
            return;
        }
        if let Some(key) = self.notifications.title_hover {
            let (rect, label) = if key == NodeKey::TitleBarBack {
                (chrome.tree.rect(chrome.back), "返回上一页")
            } else if key == NodeKey::TitleBarThemeToggle {
                (
                    chrome.tree.rect(chrome.theme_toggle),
                    if self.state.dark {
                        "切换到亮色"
                    } else {
                        "切换到暗色"
                    },
                )
            } else if key == NodeKey::TitleBarAutomations {
                (chrome.tree.rect(chrome.automations_toggle), "自动化")
            } else if key == NodeKey::TitleBarTemplates {
                (chrome.tree.rect(chrome.templates_toggle), "模板中心")
            } else if key == NodeKey::TitleBarMarketplace {
                (chrome.tree.rect(chrome.marketplace_toggle), "官方市场")
            } else if key == NodeKey::TitleBarAiToggle {
                (robot, "AI 助手")
            } else if key == NodeKey::TitleBarDesktop {
                (desktop, "桌面卡片")
            } else {
                (bell, "通知中心")
            };
            if rect.is_empty() {
                return;
            }
            let tip = Rect::from_size((rect.right - 90.0).max(0.0), rect.bottom + 6.0, 90.0, 26.0);
            self.list.rounded_rect(tip, 4.0, p.surface);
            self.list.rounded_border(tip, 4.0, p.border);
            self.list
                .text_aligned(tip, label, TextStyle::Caption, p.foreground, Align::Center);
        }
    }

    pub(super) fn paint_notifications(&mut self, p: &Palette) {
        if !self.notification_open() {
            return;
        }
        let layout = self.notification_layout();
        self.notifications.view.scroll = layout.scroll;
        self.list.clear_carets();
        ui::paint(
            &mut self.list,
            &self.notifications.view,
            &layout,
            self.renderer.viewport(),
            p,
        );
    }
}

#[cfg(test)]
mod tests;
