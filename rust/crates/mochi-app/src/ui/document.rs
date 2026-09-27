//! 编辑几何在 ui::live，富文本编辑事务在 ui::rich。

mod cache;
mod geometry;
mod layout_engine;
mod models;
mod painting;
mod parsing;

pub use cache::LayoutCache;
use cache::{parse_html_table, WrappedLine};
#[cfg(test)]
use cache::{MAX_WRAPPED_BYTES, MAX_WRAPPED_ENTRIES};
pub use geometry::{
    code_card_colors, code_card_height, code_header_height, code_pad_y, code_show_title,
    table_column_width, Decoration, Heading, ImageSizer, LaidOutLine, Layout, CHECKBOX_SIZE,
    CODE_MARGIN, IMAGE_PLACEHOLDER_HEIGHT, TABLE_CELL_MIN_WIDTH, TABLE_MARGIN,
};
use geometry::{
    code_line_number_width, code_margin, code_pad_x, code_radius, code_scrollbar_height,
    table_margin, table_row_height,
};
#[cfg(test)]
pub use geometry::{layout, TABLE_ROW_HEIGHT};
#[allow(unused_imports)]
pub use geometry::{CODE_BG, CODE_BORDER, CODE_FG, CODE_HEADER_BG, CODE_HEADER_H};
pub use layout_engine::{
    layout_editor, layout_editor_cached, layout_editor_chunk, layout_live, LayoutProgress,
};
use models::{
    block_gap, list_indent, max_line_width, padding_bottom, padding_top, padding_x, quote_bar,
    quote_indent,
};
#[cfg(test)]
pub use models::{parse, parse_count, reset_parse_count};
pub use models::{Block, Parsed, RangedBlock};
pub use painting::{
    card_width, code_horizontal_overflow, content_width, paint_export_in, paint_in,
    paint_in_indexed, resolve_image_src,
};
#[cfg(test)]
pub use painting::{max_scroll, paint};
#[cfg(test)]
use parsing::split_table_row;
pub use parsing::{fence_open, parse_ranged};

use super::draw::{DrawList, TextStyle};
use super::highlight;
use super::layout::Rect;
use super::text::{self, Emphasis, Run};
use super::theme::{self, Palette};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

#[cfg(test)]
thread_local! {
    static PARSE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod locator_tests;

#[cfg(test)]
mod source_mapping_regressions;

#[cfg(test)]
mod tests;
