//! 导航栏的显示方式和排序，与 Electron 版偏好设置共用。
use std::collections::HashSet;

use super::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    navigation::NavItem,
    text::{self, Emphasis},
    theme::{self, Palette},
};

pub const ORDER_KEY: &str = "navigation.itemOrder";
pub const HIDDEN_KEY: &str = "navigation.hiddenItems";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    pub items: Vec<NavItem>,
    pub hidden: HashSet<NavItem>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self::parse("", "")
    }
}

impl Preferences {
    pub fn parse(order: &str, hidden: &str) -> Self {
        let mut items = Vec::new();
        for item in order
            .split(',')
            .filter_map(NavItem::from_id)
            .chain(NavItem::ALL)
            .filter(|item| !matches!(item, NavItem::Templates | NavItem::Automations))
        {
            if !items.contains(&item) {
                items.push(item);
            }
        }
        Self {
            items,
            hidden: hidden.split(',').filter_map(NavItem::from_id).collect(),
        }
    }

    pub fn read() -> Self {
        Self::parse(
            &super::settings_values::text(ORDER_KEY, ""),
            &super::settings_values::text(HIDDEN_KEY, ""),
        )
    }

    pub fn visible(&self) -> Vec<NavItem> {
        self.items
            .iter()
            .copied()
            .filter(|item| !self.hidden.contains(item))
            .collect()
    }

    pub fn order_value(&self) -> String {
        self.items
            .iter()
            .map(|item| item.id())
            .collect::<Vec<_>>()
            .join(",")
    }
    pub fn hidden_value(&self) -> String {
        NavItem::ALL
            .into_iter()
            .filter(|item| self.hidden.contains(item))
            .map(NavItem::id)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn apply(&mut self, action: Action) -> bool {
        match action {
            Action::Reset => {
                *self = Self::default();
                true
            }
            Action::Toggle(item) => {
                if !self.hidden.remove(&item) {
                    self.hidden.insert(item);
                }
                true
            }
            Action::Up(item) | Action::Down(item) => {
                let Some(index) = self.items.iter().position(|value| *value == item) else {
                    return false;
                };
                let next = if matches!(action, Action::Up(_)) {
                    index.checked_sub(1)
                } else {
                    (index + 1 < self.items.len()).then_some(index + 1)
                };
                if let Some(next) = next {
                    self.items.swap(index, next);
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Toggle(NavItem),
    Up(NavItem),
    Down(NavItem),
    Reset,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub item: NavItem,
    pub rect: Rect,
    pub visible: bool,
    pub up: bool,
    pub down: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub rect: Rect,
    pub rows: Vec<Row>,
    pub entries: Vec<(Rect, Action)>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.entries
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, action)| *action)
    }
    pub fn control(&self, action: Action) -> Option<Rect> {
        self.entries
            .iter()
            .rev()
            .find(|(_, value)| *value == action)
            .map(|(rect, _)| *rect)
    }
}

const ROW_HEIGHT: f32 = 48.0;

fn title_height() -> f32 {
    TextStyle::Large.line_height().max(28.0)
}

fn header_height() -> f32 {
    title_height() + 44.0
}

pub fn height() -> f32 {
    header_height() + ROW_HEIGHT * Preferences::default().items.len() as f32 + 1.0
}

pub fn layout(left: f32, right: f32, top: f32, preferences: &Preferences) -> Layout {
    let mut result = Layout {
        rect: Rect::new(left, top, right, top + height()),
        ..Default::default()
    };
    result.entries.push((
        Rect::new(right - 96.0, top, right, top + 30.0),
        Action::Reset,
    ));
    for (i, item) in preferences.items.iter().copied().enumerate() {
        let y = top + header_height() + i as f32 * ROW_HEIGHT;
        let row = Rect::new(left, y, right, y + ROW_HEIGHT);
        let toggle = Rect::new(right - 138.0, y + 12.0, right - 94.0, y + 36.0);
        let up = Rect::new(right - 80.0, y + 8.0, right - 48.0, y + 40.0);
        let down = Rect::new(right - 44.0, y + 8.0, right - 12.0, y + 40.0);
        result.entries.push((
            Rect::new(left + 8.0, y + 4.0, toggle.left - 8.0, y + ROW_HEIGHT - 4.0),
            Action::Toggle(item),
        ));
        result.entries.push((toggle, Action::Toggle(item)));
        if i > 0 {
            result.entries.push((up, Action::Up(item)));
        }
        if i + 1 < preferences.items.len() {
            result.entries.push((down, Action::Down(item)));
        }
        result.rows.push(Row {
            item,
            rect: row,
            visible: !preferences.hidden.contains(&item),
            up: i > 0,
            down: i + 1 < preferences.items.len(),
        });
    }
    result
}

pub fn paint(list: &mut DrawList, layout: &Layout, p: &Palette) {
    if layout.rect.is_empty() {
        return;
    }
    let r = layout.rect;
    let title_height = title_height();
    list.text_run(
        Rect::new(r.left, r.top, r.right - 112.0, r.top + title_height),
        "导航按钮",
        TextStyle::Large,
        p.foreground,
        Align::Leading,
        Emphasis::Bold,
    );
    list.text(
        Rect::new(
            r.left,
            r.top + title_height + 8.0,
            r.right,
            r.top + title_height + 28.0,
        ),
        "选择要显示的入口，点击箭头调整顺序。",
        TextStyle::Caption,
        p.muted,
    );
    if let Some(reset) = layout.control(Action::Reset) {
        list.rounded_rect(reset, 6.0, p.background);
        list.rounded_border(reset, 6.0, p.border);
        list.text_aligned(
            reset,
            "恢复默认",
            TextStyle::Caption,
            p.foreground,
            Align::Center,
        );
    }
    let card = Rect::new(r.left, r.top + header_height(), r.right, r.bottom);
    list.rounded_rect(card, 12.0, p.background);
    list.rounded_border(card, 12.0, p.border);
    for (index, row) in layout.rows.iter().enumerate() {
        let r = row.rect;
        if index > 0 {
            list.hline(r.left + 1.0, r.right - 1.0, r.top, p.border);
        }
        let color = if row.visible { p.foreground } else { p.muted };
        list.icon_centered(
            Rect::new(r.left + 12.0, r.top, r.left + 32.0, r.bottom),
            row.item.icon(),
            18.0,
            color,
        );
        let label = Rect::new(r.left + 44.0, r.top, r.right - 150.0, r.bottom);
        list.text(
            label,
            text::ellipsize(row.item.label(), TextStyle::Label, label.width()),
            TextStyle::Label,
            color,
        );
        if let Some(toggle) = layout.control(Action::Toggle(row.item)) {
            list.rounded_rect(toggle, 12.0, if row.visible { p.accent } else { p.border });
            let left = if row.visible {
                toggle.right - 20.0
            } else {
                toggle.left + 4.0
            };
            list.rounded_rect(
                Rect::new(left, toggle.top + 4.0, left + 16.0, toggle.bottom - 4.0),
                8.0,
                if row.visible {
                    p.accent_foreground
                } else {
                    p.surface
                },
            );
        }
        for (rect, icon, enabled) in [
            (
                Rect::new(r.right - 80.0, r.top + 8.0, r.right - 48.0, r.top + 40.0),
                Icon::CHEVRON_UP,
                row.up,
            ),
            (
                Rect::new(r.right - 44.0, r.top + 8.0, r.right - 12.0, r.top + 40.0),
                Icon::CHEVRON_DOWN,
                row.down,
            ),
        ] {
            list.rounded_rect(rect, 6.0, if enabled { p.surface } else { p.background });
            list.icon_centered(
                rect,
                icon,
                16.0,
                if enabled {
                    p.muted
                } else {
                    theme::mix(p.muted, p.background, 0.7)
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_defaults_match_the_shared_setting_contract() {
        let read = |key| {
            mochi_core::app_settings::descriptor(key)
                .unwrap()
                .default_value
                .to_storage()
        };
        assert_eq!(
            Preferences::parse(&read(ORDER_KEY), &read(HIDDEN_KEY)),
            Preferences::default()
        );
        // 旧版共享设置中仍可能包含已经移动的两个项目。
        let preferences = Preferences::parse(&read(ORDER_KEY), &read(HIDDEN_KEY));
        assert_eq!(
            preferences.order_value(),
            Preferences::default().order_value()
        );
        assert!(!preferences.items.contains(&NavItem::Knowledge));
        assert!(!preferences.items.contains(&NavItem::AgentConfig));
    }

    #[test]
    fn malformed_preferences_keep_every_known_item_once_and_ignore_unknown_ids() {
        let preferences =
            Preferences::parse("notes,unknown,home,notes, favorites ", "home,missing,home");
        assert_eq!(preferences.items.len(), NavItem::ALL.len() - 2);
        assert!(!preferences.items.contains(&NavItem::Templates));
        assert!(!preferences.items.contains(&NavItem::Automations));
        assert_eq!(
            &preferences.items[..3],
            &[NavItem::QuickNote, NavItem::Home, NavItem::Favorites]
        );
        assert_eq!(preferences.hidden, HashSet::from([NavItem::Home]));
        assert_eq!(preferences.visible().first(), Some(&NavItem::QuickNote));
        assert_eq!(
            Preferences::parse(&preferences.order_value(), &preferences.hidden_value()),
            preferences
        );
    }

    #[test]
    fn moving_hidden_items_keeps_their_visibility_and_reset_restores_all_entries() {
        let mut preferences = Preferences::default();
        assert!(preferences.apply(Action::Toggle(NavItem::Inbox)));
        assert!(preferences.apply(Action::Up(NavItem::Inbox)));
        assert_eq!(preferences.items[0], NavItem::Inbox);
        assert!(!preferences.visible().contains(&NavItem::Inbox));
        assert!(!preferences.apply(Action::Up(NavItem::Inbox)));
        for item in NavItem::ALL {
            preferences.hidden.insert(item);
        }
        assert!(preferences.visible().is_empty());
        preferences.apply(Action::Reset);
        assert_eq!(preferences.visible(), Preferences::default().items);
    }

    #[test]
    fn control_hit_regions_follow_the_reordered_rows() {
        let preferences = Preferences::parse("favorites", "inbox");
        let layout = layout(480.0, 1176.0, 160.0, &preferences);
        assert!(layout.control(Action::Up(NavItem::Favorites)).is_none());
        let control = layout.control(Action::Down(NavItem::Favorites)).unwrap();
        assert_eq!(
            layout.hit(control.left + 8.0, control.top + 8.0),
            Some(Action::Down(NavItem::Favorites))
        );
        let mut list = DrawList::new();
        paint(&mut list, &layout, theme::tokens().palette(false));
        assert!(list.finish().is_ok());
    }
}
