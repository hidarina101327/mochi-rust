//! 定义通知筛选条件、分类标签和匹配规则。
mod geometry;
mod model;
mod painting;
#[cfg(test)]
mod tests;

use super::overlay_scrollbar::Interaction;
pub use geometry::{layout, Layout};
pub use model::{Category, History, Notice};
pub use painting::{blue, paint};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Closed,
    Popover,
    Center,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Category(Category),
}

impl Filter {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Unread,
        Self::Category(Category::Document),
        Self::Category(Category::Assistant),
        Self::Category(Category::Schedule),
        Self::Category(Category::System),
        Self::Category(Category::Automation),
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Unread => "未读",
            Self::Category(c) => c.label(),
        }
    }
    pub fn matches(self, n: &Notice) -> bool {
        match self {
            Self::All => true,
            Self::Unread => !n.read,
            Self::Category(c) => n.category == c,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Close,
    More,
    MarkAllRead,
    Filter(Filter),
    Notice(u64),
    Back,
    ToggleCategory(Category),
    Settings,
}

#[derive(Default)]
pub struct State {
    pub history: History,
    pub mode: Mode,
    pub filter: Filter,
    pub selected: Option<u64>,
    pub scroll: f32,
    pub hover: Option<Hit>,
    pub focused: Option<Hit>,
    pub scrollbar: Interaction<()>,
    pub save_failed: bool,
    pub muted: Vec<Category>,
    pub windows_failed: bool,
}

impl State {
    pub fn is_open(&self) -> bool {
        self.mode != Mode::Closed
    }
    pub fn open(&mut self, mode: Mode) {
        self.mode = mode;
        self.filter = Filter::All;
        self.selected = None;
        self.reset_navigation();
    }
    pub fn reset_navigation(&mut self) {
        self.scroll = 0.0;
        self.hover = None;
        self.focused = None;
        self.scrollbar = Interaction::default();
    }
    pub fn close(&mut self) {
        self.mode = Mode::Closed;
        self.reset_navigation();
    }
    pub fn selected_notice(&self) -> Option<&Notice> {
        self.selected
            .and_then(|id| self.history.entries.iter().find(|n| n.id == id))
    }
    pub fn activate(&mut self, hit: Hit) -> bool {
        match hit {
            Hit::Close => self.close(),
            Hit::More => self.open(Mode::Center),
            Hit::Filter(filter) => {
                self.filter = filter;
                self.selected = None;
                self.reset_navigation();
                self.focused = Some(hit);
            }
            Hit::Back => {
                self.selected = None;
                self.reset_navigation();
            }
            Hit::Notice(id) => {
                if !self.history.entries.iter().any(|n| n.id == id) {
                    return false;
                }
                self.mode = Mode::Center;
                self.selected = Some(id);
                self.reset_navigation();
                self.focused = Some(Hit::Back);
                return self.history.mark_read(id);
            }
            Hit::MarkAllRead => return self.history.mark_all_read(),
            Hit::ToggleCategory(_) | Hit::Settings => {} // 持久化设置由 App 处理。
        }
        false
    }
    pub fn navigate(&mut self, layout: &Layout, backwards: bool) {
        let hits: Vec<_> = layout
            .controls
            .iter()
            .map(|(_, h)| *h)
            .chain(layout.rows.iter().map(|(_, id)| Hit::Notice(*id)))
            .collect();
        if hits.is_empty() {
            return;
        }
        let i = self.focused.and_then(|h| hits.iter().position(|x| *x == h));
        let next = match (i, backwards) {
            (Some(i), true) => (i + hits.len() - 1) % hits.len(),
            (Some(i), false) => (i + 1) % hits.len(),
            (None, true) => hits.len() - 1,
            (None, false) => 0,
        };
        self.focused = Some(hits[next]);
        if let Hit::Notice(id) = hits[next] {
            if let Some((r, _)) = layout.rows.iter().find(|(_, i)| *i == id) {
                if r.top < layout.body.top {
                    self.scroll -= layout.body.top - r.top;
                }
                if r.bottom > layout.body.bottom {
                    self.scroll += r.bottom - layout.body.bottom;
                }
                self.scroll = self.scroll.clamp(0.0, layout.max_scroll);
            }
        }
    }
}
