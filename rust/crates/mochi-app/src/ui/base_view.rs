//! 由架构定义驱动、可编辑的多维表格。持久化时使用 mochi-core 的 .mcb 解析器。

mod detail;
mod geometry;
mod grid;
mod menus;
mod models;
mod mutations;
mod painting;
mod pointer;
mod popup_actions;
mod popup_geometry;
#[cfg(test)]
mod presentation_tests;
mod queries;
mod state;

use detail::{detail_height, paint_detail};
pub use geometry::layout;
use geometry::{
    date_time_part_value, date_time_picker_controls, grid_row_height, type_name, width,
};
#[allow(unused_imports)]
pub use models::ReferenceCandidate;
pub use models::{
    Action, AiCellContext, DateTimePart, Edit, Hit, InsertDirection, Layout, Prompt, State,
};
use models::{
    Choice, DateTimePicker, Drag, MenuItem, Popup, COLORS, DEFAULT_GRID_ROW_HEIGHT,
    GRID_ROW_HEIGHTS, OPERATORS, TYPES,
};
use painting::cell;
#[cfg(test)]
use painting::option_color;
pub use painting::{paint, paint_modal};

use super::overlay_scrollbar::{Axis, Bar, Interaction};
use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    text,
    theme::{self, Palette},
};
use chrono::{Datelike, Local, NaiveDate, SecondsFormat, TimeZone, Timelike};
use mochi_core::base::*;
use mochi_core::object_reference::ObjectReference;
use serde_json::{json, Value};

pub(super) fn table_cell_padding() -> f32 {
    crate::ui::settings_values::number("tables.cellPadding", 10.0).clamp(0.0, 32.0)
}

pub(super) fn table_border_width() -> f32 {
    crate::ui::settings_values::number("tables.borderWidth", 1.0).clamp(0.0, 6.0)
}

pub(super) fn board_card_radius() -> f32 {
    crate::ui::settings_values::number("tables.boardCardRadius", 8.0).clamp(0.0, 24.0)
}

pub(super) fn table_reference_row_height() -> f32 {
    (TextStyle::Table.line_height() + 8.0).max(34.0)
}

pub(super) fn progress_regions(cell: Rect) -> (Rect, Rect) {
    let padding = table_cell_padding().min(cell.width().max(0.0) / 2.0);
    let inner_left = cell.left + padding;
    let inner_right = (cell.right - padding).max(inner_left);
    let input_left = (inner_right - 49.0).max(inner_left);
    let input = Rect::new(input_left, cell.top + 5.0, inner_right, cell.bottom - 5.0);
    let bar = Rect::new(
        inner_left,
        cell.top + 15.0,
        (input_left - 6.0).max(inner_left),
        cell.top + 23.0,
    );
    (bar, input)
}

pub(super) fn table_border(list: &mut DrawList, rect: Rect, color: u32) {
    let width = table_border_width()
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    if width <= 0.0 || rect.is_empty() {
        return;
    }
    if width <= 1.0 {
        list.rounded_border(rect, 0.0, color);
        return;
    }
    list.rect(
        Rect::new(rect.left, rect.top, rect.right, rect.top + width),
        color,
    );
    list.rect(
        Rect::new(rect.left, rect.bottom - width, rect.right, rect.bottom),
        color,
    );
    list.rect(
        Rect::new(
            rect.left,
            rect.top + width,
            rect.left + width,
            rect.bottom - width,
        ),
        color,
    );
    list.rect(
        Rect::new(
            rect.right - width,
            rect.top + width,
            rect.right,
            rect.bottom - width,
        ),
        color,
    );
}

#[cfg(test)]
mod tests;
