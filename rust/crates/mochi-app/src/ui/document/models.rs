//! 定义文档排版中各类内容块的尺寸和缩进规则。
use super::*;

/// 排版值取设计令牌；editorMaxWidth = 0 表示不限宽。
pub(super) fn padding_x() -> f32 {
    crate::ui::editor_preferences::current().padding_left
}

pub(super) fn padding_top() -> f32 {
    crate::ui::editor_preferences::current().padding_top
}

pub(super) fn padding_bottom() -> f32 {
    crate::ui::editor_preferences::current().padding_bottom
}

/// 正文最大行宽；`0` 表示不限。
pub(super) fn max_line_width() -> f32 {
    let w = crate::ui::editor_preferences::current().max_width;
    if w > 0.0 {
        w
    } else {
        f32::INFINITY
    }
}

/// 块与块之间的间距。
pub(super) fn block_gap() -> f32 {
    crate::ui::editor_preferences::current().block_spacing
}

/// 引用块左侧竖线的宽度与留白。
pub(super) fn quote_bar() -> f32 {
    crate::ui::editor_preferences::current().quote_border
}

pub(super) fn quote_indent() -> f32 {
    crate::ui::editor_preferences::current().quote_padding
}

/// 列表项的缩进与项目符号宽度。
pub(super) fn list_indent() -> f32 {
    crate::ui::editor_preferences::current().list_indent
}

/// 一个块级元素。
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Container(crate::ui::containers::Container),
    AiLocator(mochi_core::ai::locator::Locator),
    ObjectReference(crate::ui::object_link::ObjectLink),
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Aligned {
        align: crate::ui::draw::Align,
        level: u8,
        text: String,
    },
    /// 显示公式。保留 TeX 源码，活动块仍直接编辑原始围栏。
    Math(String),
    /// `marker` 是渲染出来的项目符号（`•` 或 `1.` 或 `☐`/`☑`）。
    ListItem {
        marker: String,
        text: String,
        depth: usize,
    },
    Quote(String),
    /// 围栏代码块。`lang` 目前只存着，语法高亮还没做。
    Code {
        lang: String,
        lines: Vec<String>,
    },
    /// 管道表格。`rows[0]` 是表头（有分隔行时）。各行单元格数补齐到最宽的一行。
    Table {
        rows: Vec<Vec<String>>,
        header: bool,
    },
    /// 图片 `![alt](src)` 独占一行。
    Image {
        alt: String,
        src: String,
        width: Option<String>,
    },
    Divider,
    /// 源码中的空行分隔符。源码编辑布局会保留它供光标导航，
    /// Markdown 富文本布局按 Electron 语义折叠它。
    Blank,
}

/// note_parser 会屏蔽代码块，不能用于正文渲染。编辑器使用带范围的 parse_ranged。
#[cfg(test)]
pub fn parse(source: &str) -> Vec<Block> {
    parse_ranged(source)
        .blocks
        .into_iter()
        .map(|b| b.block)
        .collect()
}

/// 带源码范围的块。`start..end` 是块在源码里的字节范围（不含最后一行的换行）。
#[derive(Debug, Clone, PartialEq)]
pub struct RangedBlock {
    pub block: Block,
    pub start: usize,
    pub end: usize,
}

/// 解析结果：块，以及正文开始位置（文档头部之后）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub blocks: Vec<RangedBlock>,
    /// 正文起点。文档头部既不参与绘制，也不参与编辑，光标不能移入其中。
    pub body_start: usize,
}

#[cfg(test)]
pub fn reset_parse_count() {
    PARSE_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub fn parse_count() -> usize {
    PARSE_CALLS.with(std::cell::Cell::get)
}

impl Parsed {
    /// 光标所在的块。范围**含右端**：光标停在块最后一个字之后仍属于该块；
    /// 下一个块从换行之后开始，所以不会有歧义。
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        let index = self.blocks.partition_point(|block| block.end < offset);
        self.blocks.get(index).filter(|block| block.start <= offset).map(|_| index)
            .or_else(|| self.blocks.iter().rposition(|b| matches!(&b.block, Block::Container(p) if p.header.start <= offset && offset <= p.footer.end)))
    }
}
