//! 计算首页各面板的位置，并处理点击区域命中。
use super::*;

/// 在 `area` 指定的内容区域排版统计页。
#[cfg(test)]
pub fn layout(a: &HomeAnalytics, area: Rect) -> Layout {
    let mut b = Builder::new(area);

    b.analytics(a);

    Layout {
        height: b.y - area.top + panel_padding(),
        blocks: b.blocks,
    }
}

/// 排版首页完整工作台。
///
/// 与测试专用的统计页排版 `layout` 分离，保留两者各自的卡片顺序。
/// `analytics` 为 `None` 时仍会排版所有入口卡片；后台统计稍后通过
/// `HomePane::set_analytics` 注入，不会让首页变成一片空白。
pub fn layout_with_dashboard(
    analytics: Option<&HomeAnalytics>,
    dashboard: &Dashboard,
    area: Rect,
) -> Layout {
    // 使用有限大小的共享网格，让问候语、操作按钮和文档列表
    // 保持对齐。宽屏增加留白，而不是拉宽每一行。
    let max_width = crate::ui::settings_values::number("dashboard.maxWidth", 1220.0);
    let inset = ((area.width() - max_width) * 0.5).max(0.0);
    let page_area = Rect::new(area.left + inset, area.top, area.right - inset, area.bottom);
    let mut header = Builder::new(page_area);
    header.masthead(analytics);
    header.quick_actions();
    let body_top = header.y + 8.0;
    let column_gap = crate::ui::settings_values::number("dashboard.columnGap", 20.0);
    let side_width = crate::ui::settings_values::number("dashboard.sideWidth", 340.0);
    if page_area.width() >= (side_width + column_gap + 500.0).max(940.0) {
        let split = page_area.right - side_width - column_gap;
        let column_area_left = Rect::new(page_area.left, body_top, split, area.bottom);
        let column_area_right =
            Rect::new(split + column_gap, body_top, page_area.right, area.bottom);
        let mut left = Builder::new(column_area_left);
        left.documents("最近笔记", &dashboard.recent_documents, false);
        left.documents("收藏文档", &dashboard.favorite_documents, true);
        left.libraries(&dashboard.libraries);
        let mut right = Builder::new(column_area_right);
        right.quiet = true;
        right.schedule(
            &dashboard.schedule_items,
            dashboard.schedule_conflict_count,
            &dashboard.schedule_risk,
        );
        right.inbox(&dashboard.inbox_items);
        right.journal(&dashboard.journal);
        right.ai_sessions(&dashboard.ai_sessions);
        if let Some(a) = analytics {
            right.persona(a);
        } else {
            right.persona_pending();
        }
        right.graph(analytics, &dashboard.graph);
        if let Some(a) = analytics {
            right.stats(a);
        } else {
            right.stats_pending();
        }
        left.analytics_main(analytics);
        let height = left.y.max(right.y) - area.top + panel_padding();
        let mut blocks = header.blocks;
        blocks.extend(left.blocks);
        blocks.extend(right.blocks);
        return Layout { height, blocks };
    }

    // 窗口较窄时，先显示今天常用的入口，再显示报告。
    let mut b = Builder::new(Rect::new(
        page_area.left,
        body_top,
        page_area.right,
        area.bottom,
    ));
    b.blocks = header.blocks;
    b.documents("最近笔记", &dashboard.recent_documents, false);
    b.schedule(
        &dashboard.schedule_items,
        dashboard.schedule_conflict_count,
        &dashboard.schedule_risk,
    );
    b.inbox(&dashboard.inbox_items);
    b.journal(&dashboard.journal);
    b.documents("收藏文档", &dashboard.favorite_documents, true);
    b.libraries(&dashboard.libraries);
    b.ai_sessions(&dashboard.ai_sessions);
    if let Some(a) = analytics {
        b.persona(a);
    } else {
        b.persona_pending();
    }
    b.graph(analytics, &dashboard.graph);
    if let Some(a) = analytics {
        b.stats(a);
    } else {
        b.stats_pending();
    }
    b.analytics_main(analytics);

    Layout {
        height: b.y - area.top + panel_padding(),
        blocks: b.blocks,
    }
}

impl Layout {
    /// 返回滚动后视口中的首页入口。
    pub fn hit(&self, x: f32, y: f32, scroll: f32) -> Option<Action> {
        self.hit_index(x, y, scroll).and_then(|i| self.action(i))
    }

    pub fn hit_index(&self, x: f32, y: f32, scroll: f32) -> Option<usize> {
        self.blocks.iter().rposition(|block| {
            block_action(block).is_some() && shift(block_rect(block), -scroll).contains(x, y)
        })
    }

    pub fn action(&self, index: usize) -> Option<Action> {
        self.blocks.get(index).and_then(block_action)
    }

    pub fn action_rect(&self, index: usize) -> Option<Rect> {
        self.blocks
            .get(index)
            .filter(|b| block_action(b).is_some())
            .map(block_rect)
    }

    pub fn next_action(&self, current: Option<usize>, backwards: bool) -> Option<usize> {
        // 焦点按屏幕上的行顺序移动，而不是按
        // 绘制列表中各列加入的先后顺序移动。
        let mut actions: Vec<_> = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| block_action(b).is_some())
            .collect();
        actions.sort_by(|(_, a), (_, b)| {
            let a = block_rect(a);
            let b = block_rect(b);
            a.top.total_cmp(&b.top).then(a.left.total_cmp(&b.left))
        });
        let count = actions.len();
        if count == 0 {
            return None;
        }
        let next = match current.and_then(|i| actions.iter().position(|(index, _)| *index == i)) {
            Some(i) if backwards => (i + count - 1) % count,
            Some(i) => (i + 1) % count,
            None if backwards => count - 1,
            None => 0,
        };
        Some(actions[next].0)
    }
}

pub(super) struct Builder {
    pub(super) blocks: Vec<Block>,
    pub(super) area: Rect,
    /// 当前纵向游标（绝对坐标）。
    pub(super) y: f32,
    pub(super) quiet: bool,
}

pub(super) fn block_rect(b: &Block) -> Rect {
    match b {
        Block::Panel { rect, .. }
        | Block::SectionTitle { rect, .. }
        | Block::Headline { rect, .. }
        | Block::Greeting { rect, .. }
        | Block::Caption { rect, .. }
        | Block::Badge { rect, .. }
        | Block::Tile { rect, .. }
        | Block::Bars { rect, .. }
        | Block::Heatmap { rect, .. }
        | Block::HourlyHeatmap { rect, .. }
        | Block::StorageBar { rect, .. }
        | Block::PromptRow { rect, .. }
        | Block::DetailRow { rect, .. }
        | Block::ActionTile { rect, .. }
        | Block::DocumentRow { rect, .. }
        | Block::InboxRow { rect, .. }
        | Block::LibraryRow { rect, .. }
        | Block::SessionRow { rect, .. }
        | Block::ScheduleRow { rect, .. }
        | Block::JournalPreview { rect, .. }
        | Block::GraphNodeRow { rect, .. }
        | Block::Empty { rect, .. }
        | Block::ActionLink { rect, .. } => *rect,
    }
}

pub(super) fn block_action(b: &Block) -> Option<Action> {
    match b {
        Block::ActionTile { action, .. }
        | Block::PromptRow { action, .. }
        | Block::DocumentRow { action, .. }
        | Block::InboxRow { action, .. }
        | Block::LibraryRow { action, .. }
        | Block::SessionRow { action, .. }
        | Block::ScheduleRow { action, .. }
        | Block::JournalPreview { action, .. }
        | Block::GraphNodeRow { action, .. }
        | Block::ActionLink { action, .. } => Some(action.clone()),
        _ => None,
    }
}

pub(super) fn shift(r: Rect, dy: f32) -> Rect {
    Rect::new(r.left, r.top + dy, r.right, r.bottom + dy)
}

/// 滚动上界。
pub fn max_scroll(page: &Layout, area: Rect) -> f32 {
    (page.height - area.height()).max(0.0)
}
