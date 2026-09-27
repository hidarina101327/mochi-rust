//! 定义文档排版使用的行、装饰和表格尺寸。
use super::*;

/// 排好版的一个视觉行。文档可能很长，绘制时按视口切片，只画看得见的。
#[derive(Debug, Clone, PartialEq)]
pub struct LaidOutLine {
    /// 一行里可能有几段不同样式的文字（`**粗**` / `` `码` ``）。
    /// 整行一个 `String` 就只能把标记符原样画出来——那正是「看着像源码」的根源。
    pub runs: Vec<Run>,
    pub style: TextStyle,
    /// 相对内容区左边界的偏移。
    pub x: f32,
    /// 相对文档顶部的偏移。
    pub y: f32,
    pub height: f32,
    /// 引用块的竖线、代码块的底色——按块类型决定装饰。
    pub decoration: Decoration,
    /// 这一行属于第几个块（`Parsed::blocks` 的下标）。
    pub block: usize,
    /// 行文字在源码里的**精确**字节范围——原样显示的行（活动块、代码块）才有。
    /// 渲染过的行（标记符被吃掉了）用 `visible_start` 做近似映射。
    pub source: Option<(usize, usize)>,
    /// 渲染行：本行首字是块可见文本里的第几个字符。项目符号那一行是 `usize::MAX`。
    pub visible_start: usize,
    /// 代码行的语法着色（行内字节范围）。其它行为空。
    pub tokens: Vec<highlight::Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoration {
    Container {
        end: usize,
        details: bool,
        collapsed: bool,
        background: u32,
        border: u32,
    },
    AiLocator,
    ObjectReference {
        text_style: bool,
    },
    None,
    QuoteBar,
    HeadingUnderline,
    Math,
    /// 代码块正文行（卡片背景由 `CodeHeader` 一次画完，这里只画文字）。
    /// 软换行的续行不重复绘制行号。
    CodeBackground {
        line_number: Option<usize>,
    },
    /// 代码块卡片头部（`.mochi-code-block-header`）：语言名 + 复制按钮，并负责整张卡片的底色与边框。
    /// `body_lines` 决定卡片高度。
    CodeHeader {
        body_lines: usize,
        collapsed: bool,
    },
    Divider,
    /// 表格的一行：`runs` 一格一个 run。`header` 画底色与半粗。
    TableRow {
        cols: usize,
        header: bool,
    },
    /// 图片占位：`runs[0]` 是 alt，`runs[1]` 是 src。真正的位图由 gfx 按 src 解码。
    Image,
}

/// 表格几何（editor.css：单元格 `width: 160px`、`padding: 10px 14px`、边框 1px、字号 14/20，
/// 表格上下外边距 20）。
pub const TABLE_CELL_MIN_WIDTH: f32 = 160.0;

#[cfg(test)]
pub const TABLE_ROW_HEIGHT: f32 = 41.0;

pub(super) fn table_row_height() -> f32 {
    let p = crate::ui::editor_preferences::current();
    TextStyle::Table.line_height() + p.table_padding * 2.0 + p.table_border
}

pub const TABLE_MARGIN: f32 = 20.0;

pub(super) fn table_margin() -> f32 {
    crate::ui::editor_preferences::current().table_margin
}

/// 图片块的默认高度：没解码出尺寸之前先占这么高（editor.css 里图片 `max-width: 100%`，高度随图）。
pub const IMAGE_PLACEHOLDER_HEIGHT: f32 = 200.0;

/// 任务勾选框边长（TipTap TaskItem 的 checkbox 是 16px）。
pub const CHECKBOX_SIZE: f32 = 16.0;

/// 代码块卡片（editor.css `.mochi-code-block*`）：外边距 20、圆角 10、1px 边框；
/// 头部 36 高（`padding 6px 10px`）；正文 `padding 20px 24px`。
pub const CODE_MARGIN: f32 = 20.0;

pub(super) fn code_margin() -> f32 {
    crate::ui::editor_preferences::current().code_margin
}

pub(super) fn code_radius() -> f32 {
    crate::ui::editor_preferences::current().code_radius
}

pub const CODE_HEADER_H: f32 = 36.0;

/// 标题关闭时，不再为工具栏预留高度；语言标识绘制在卡片右上角的内边距中。
pub fn code_header_height() -> f32 {
    if crate::ui::editor_preferences::current().code_show_title {
        CODE_HEADER_H
    } else {
        0.0
    }
}

pub fn code_show_title() -> bool {
    crate::ui::editor_preferences::current().code_show_title
}

pub fn code_pad_y() -> f32 {
    crate::ui::editor_preferences::current().code_padding
}

pub(super) fn code_pad_x() -> f32 {
    code_pad_y() * 1.2
}

pub(super) fn code_line_number_width(lines: usize) -> f32 {
    if !crate::ui::editor_preferences::current().code_show_line_numbers {
        return 0.0;
    }
    text::measure(&lines.max(1).to_string(), TextStyle::DocumentMono) + 12.0
}

pub(super) fn code_scrollbar_height() -> f32 {
    (!crate::ui::editor_preferences::current().code_wrap)
        .then_some(14.0)
        .unwrap_or(0.0)
}

/// 卡片底色 / 头部底色 / 边框 / 前景（CSS 里按主题写死：浅色 `#f8f9fa/#f2f4f7/#e7e9ee/#24292e`，
/// 暗色 `#1e1e1e/#252525/#3e3e3e` + 浅前景）。
pub const CODE_BG: u32 = 0xF8F9FA;

pub const CODE_HEADER_BG: u32 = 0xF2F4F7;

pub const CODE_BORDER: u32 = 0xE7E9EE;

pub const CODE_FG: u32 = 0x24292E;

/// 按主题取代码卡片的四个颜色：(底, 头部底, 边框, 前景)。
pub fn code_card_colors(p: &Palette) -> (u32, u32, u32, u32) {
    let preferences = crate::ui::editor_preferences::current();
    let (mut background, mut header, mut border, mut foreground) = if preferences.code_style == 3 {
        (0x07130d, 0x0b1d13, 0x1e5134, 0xb7f7ca)
    } else if preferences.code_style == 2 {
        (0xfffdf8, 0xf8f2e7, 0xe8dfcf, CODE_FG)
    } else if preferences.code_style == 1 || theme::is_dark(p) {
        (0x16181d, 0x20232a, 0x30343c, 0xd8dee9)
    } else {
        (CODE_BG, CODE_HEADER_BG, CODE_BORDER, CODE_FG)
    };
    if let Some(color) = preferences.code_background_color {
        background = color;
        header = theme::mix(color, p.surface, 0.86);
    }
    border = preferences.code_border_color.unwrap_or(border);
    foreground = preferences.code_foreground_color.unwrap_or(foreground);
    (background, header, border, foreground)
}

/// 代码卡片总高：头部 + 上下留白 + 行数 × 行高（空代码块也留一行）。
pub fn code_card_height(body_lines: usize) -> f32 {
    code_header_height()
        + code_pad_y() * 2.0
        + body_lines.max(1) as f32 * TextStyle::DocumentMono.line_height()
        + code_scrollbar_height()
        + 1.0
}

/// 表格列宽：不小于 160，能填满内容宽度就均分填满（`min-width: 100%`）。
pub fn table_column_width(cols: usize, available: f32) -> f32 {
    if cols == 0 {
        return TABLE_CELL_MIN_WIDTH;
    }
    (available / cols as f32).max(TABLE_CELL_MIN_WIDTH).floor()
}

/// 大纲里的一条。`y` 是该标题在文档里的纵向偏移——点大纲要能跳过去，
/// 所以位置必须在排版时顺手记下，不能事后再猜。
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub y: f32,
}

/// 排版结果。
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub lines: Vec<LaidOutLine>,
    /// 文档总高，滚动条夹取用。
    pub height: f32,
    /// 文档大纲，按出现顺序。
    pub headings: Vec<Heading>,
    /// 仅保存用户拖动过的 Markdown 表格列宽，键是块下标。
    /// 它属于布局而不是 Markdown 源码，避免为了视觉列宽把文档改写成 HTML。
    pub table_widths: HashMap<usize, Vec<f32>>,
}

impl Layout {
    /// 布局按源码顺序生成块（连同各自的装饰）。二分查找让每个块的交互
    /// 开销与文档大小无关。
    pub fn block_line_range(&self, block: usize) -> std::ops::Range<usize> {
        let start = self.lines.partition_point(|line| line.block < block);
        let end = start + self.lines[start..].partition_point(|line| line.block == block);
        start..end
    }

    pub fn lines_for_block(&self, block: usize) -> &[LaidOutLine] {
        &self.lines[self.block_line_range(block)]
    }
}

/// 把块排成视觉行。`width` 是内容区可用宽度（已扣掉左右留白）。只读排版，测试用；
/// 产品代码走 [`layout_live`]。
#[cfg(test)]
pub fn layout(blocks: &[Block], width: f32) -> Layout {
    let ranged: Vec<RangedBlock> = blocks
        .iter()
        .cloned()
        .map(|block| RangedBlock {
            block,
            start: 0,
            end: 0,
        })
        .collect();
    layout_live(&ranged, "", None, width, &|_| None)
}

/// 图片尺寸查询：给 `src`，返回 (宽, 高) 像素。排版层不读文件，由调用方提供（并缓存）。
pub type ImageSizer<'a> = &'a dyn Fn(&str) -> Option<(u32, u32)>;
