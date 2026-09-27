//! 维护首页面板状态，并切换仪表板和分析内容。
use super::*;

/// 首页仪表盘。快照是后台线程算出来的，排版按宽度缓存。
#[derive(Default)]
pub struct HomePane {
    /// `None` 表示后台还在算。
    pub(super) analytics: Option<HomeAnalytics>,
    pub(super) dashboard: home::Dashboard,
    /// Dashboard 可以先于 analytics 到达：首页入口和各列表不应等待统计扫描完成。
    pub(super) dashboard_loaded: bool,
    pub(super) page: Option<home::Layout>,
    /// 上次排版用的 (左边界, 上边界, 宽度)。坐标也参与缓存，
    /// 这样同宽度的编辑区在布局移动后不会继续使用旧的绝对位置。
    pub(super) key: Option<(u32, u32, u32)>,
    settings_revision: u64,
    pub(super) scroll: f32,
    pub(super) hover: Option<usize>,
    pub(super) keyboard_focus: Option<usize>,
}

impl HomePane {
    /// 换工作区：丢掉旧快照，回到「正在统计」态。
    pub fn reset(&mut self) {
        self.analytics = None;
        self.dashboard = home::Dashboard::default();
        self.dashboard_loaded = false;
        self.page = None;
        self.key = None;
        self.scroll = 0.0;
        self.hover = None;
        self.keyboard_focus = None;
    }

    /// 后台线程算完了。
    pub fn set_analytics(&mut self, analytics: HomeAnalytics) {
        self.analytics = Some(analytics);
        self.key = None;
    }

    /// 首页入口与列表的快照。它可以在 analytics 之前到达，空 analytics 时仍会
    /// 显示快速开始、最近/收藏、收件箱、知识库、AI、日程、日记和图谱占位。
    pub fn set_dashboard(&mut self, dashboard: home::Dashboard) {
        let changed = !self.dashboard_loaded || self.dashboard != dashboard;
        self.dashboard = dashboard;
        self.dashboard_loaded = true;
        if changed {
            self.key = None;
        }
    }

    pub fn dashboard(&self) -> &home::Dashboard {
        &self.dashboard
    }

    pub fn analytics(&self) -> Option<&HomeAnalytics> {
        self.analytics.as_ref()
    }

    pub fn ensure(&mut self, area: Rect) {
        if self.analytics.is_none() && !self.dashboard_loaded {
            return;
        }
        let key = (
            area.left.to_bits(),
            area.top.to_bits(),
            area.width().round().to_bits(),
        );
        let revision = crate::ui::settings_values::revision();
        if self.key == Some(key) && self.settings_revision == revision {
            return;
        }
        self.page = Some(home::layout_with_dashboard(
            self.analytics.as_ref(),
            &self.dashboard,
            area,
        ));
        self.hover = None;
        self.keyboard_focus = None;
        self.key = Some(key);
        self.settings_revision = revision;
    }

    pub fn paint(&mut self, list: &mut DrawList, area: Rect, p: &Palette) {
        self.ensure(area);
        match &self.page {
            Some(page) => {
                let scroll = self.scroll.min(home::max_scroll(page, area));
                home::paint_interactive(
                    list,
                    area,
                    page,
                    scroll,
                    p,
                    self.hover,
                    self.keyboard_focus,
                );
            }
            // 后台还在算。说一句而不是留白——扫完真实工作区要几秒
            None => placeholder(list, area, "正在统计…", p),
        }
    }

    pub fn scroll_by(&mut self, area: Rect, step: f32) {
        let Some(page) = &self.page else { return };
        self.scroll = (self.scroll - step).clamp(0.0, home::max_scroll(page, area));
        self.hover = None;
    }

    pub fn set_hover(&mut self, area: Rect, x: f32, y: f32) -> bool {
        self.ensure(area);
        let next = self.page.as_ref().and_then(|page| {
            area.contains(x, y)
                .then(|| page.hit_index(x, y, self.scroll.min(home::max_scroll(page, area))))
                .flatten()
        });
        let changed = next != self.hover;
        self.hover = next;
        changed
    }

    pub fn navigate(&mut self, area: Rect, backwards: bool) -> bool {
        self.ensure(area);
        let Some(page) = &self.page else {
            return false;
        };
        self.keyboard_focus = page.next_action(self.keyboard_focus, backwards);
        self.hover = None;
        let Some(rect) = self.keyboard_focus.and_then(|i| page.action_rect(i)) else {
            return false;
        };
        if rect.top - self.scroll < area.top + 8.0 {
            self.scroll = rect.top - area.top - 8.0;
        } else if rect.bottom - self.scroll > area.bottom - 8.0 {
            self.scroll = rect.bottom - area.bottom + 8.0;
        }
        self.scroll = self.scroll.clamp(0.0, home::max_scroll(page, area));
        true
    }

    pub fn focused_action(&self) -> Option<home::Action> {
        self.keyboard_focus
            .and_then(|i| self.page.as_ref()?.action(i))
    }

    pub fn clear_keyboard_focus(&mut self) -> bool {
        self.keyboard_focus.take().is_some()
    }

    /// 返回首页入口的动作。坐标是窗口坐标，滚动位置与 paint/scroll_by 共用。
    pub fn hit(&mut self, area: Rect, x: f32, y: f32) -> Option<home::Action> {
        if !area.contains(x, y) {
            return None;
        }
        self.ensure(area);
        let page = self.page.as_ref()?;
        page.hit(x, y, self.scroll.min(home::max_scroll(page, area)))
    }
}
