//! 统计和入口摘要由宿主提供；此模块不查询工作区。

mod analytics;
mod builder;
mod dashboard;
mod formatting;
mod geometry;
mod models;
mod painting;

#[cfg(test)]
use formatting::weekday_of;
use formatting::{
    bytes, chronotype_label, compact, duration, greeting, headline, heat_cells, hourly_cells,
    now_ms, relative_time, schedule_kind_label, schedule_risk_label, schedule_status_label, signed,
    today_narrative, weekday_label,
};
#[cfg(test)]
pub use geometry::layout;
use geometry::{block_action, block_rect, shift, Builder};
pub use geometry::{layout_with_dashboard, max_scroll};
use models::{
    action_tile_height, chart_height, document_row_height, gap, graph_row_height, heat_cell,
    heat_gap, inbox_row_height, journal_height, library_row_height, panel_padding,
    schedule_row_height, session_row_height, tile_height, Block, TILES_PER_ROW,
};
pub use models::{
    Action, Dashboard, DashboardDocument, DashboardGraph, DashboardGraphNode, DashboardInboxItem,
    DashboardJournal, DashboardLibrary, DashboardScheduleItem, DashboardSession, Layout,
};
#[allow(unused_imports)]
pub use painting::paint;
pub use painting::paint_interactive;

use std::path::PathBuf;

use mochi_core::analytics::{
    AiTrendPoint, DailyActivityPoint, FileTypeStat, HomeAnalytics, HourlyActivityPoint,
    TokenTrendPoint, TrendPoint, WritingTrendPoint,
};

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::{Edges, Rect};
use super::text;
use super::theme::{self, Palette};

#[cfg(test)]
mod tests;
