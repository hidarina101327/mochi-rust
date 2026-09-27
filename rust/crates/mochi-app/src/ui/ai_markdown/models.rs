//! 定义 AI Markdown 排版使用的节点和行内内容结构。
use super::*;

#[derive(Debug, Clone)]
pub(super) enum Kind {
    Root,
    Paragraph,
    Inline,
    Heading(u8),
    Quote,
    List,
    Item,
    Code { language: String },
    Table,
    Row(bool),
    Cell,
    Link(String),
    Rule,
    Math,
    Html,
}

#[derive(Debug, Clone)]
pub(super) struct Node {
    pub(super) anchor: usize,
    pub(super) kind: Kind,
    pub(super) runs: Vec<Run>,
    pub(super) children: Vec<Node>,
}

impl Node {
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            anchor: 0,
            kind,
            runs: Vec::new(),
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct InlinePart {
    pub(super) run: Run,
    pub(super) target: Option<String>,
}

pub(super) fn collect_inline_parts(node: &Node, target: Option<&str>, out: &mut Vec<InlinePart>) {
    let target = match &node.kind {
        Kind::Link(value) => Some(value.as_str()),
        _ => target,
    };
    out.extend(node.runs.iter().cloned().map(|run| InlinePart {
        run,
        target: target.map(str::to_owned),
    }));
    for child in &node.children {
        if matches!(child.kind, Kind::Inline | Kind::Link(_)) {
            collect_inline_parts(child, target, out);
        }
    }
}

pub(super) fn inline_parts(node: &Node) -> Vec<InlinePart> {
    let mut out = Vec::new();
    collect_inline_parts(node, None, &mut out);
    out
}

pub(super) fn inline_runs(node: &Node) -> Vec<Run> {
    inline_parts(node)
        .into_iter()
        .map(|part| part.run)
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Tone {
    Normal,
    Muted,
    Code,
}

#[derive(Debug, Clone)]
pub(super) enum Item {
    Text {
        rect: Rect,
        run: Run,
        style: TextStyle,
        tone: Tone,
        /// 代码块的语法高亮标记。绘制时才按当前配色解析，
        /// 同一份缓存布局在明暗两种主题下都能用。
        syntax: Option<highlight::Token>,
    },
    CodeBox(Rect),
    CodeHeader {
        rect: Rect,
        language: String,
    },
    ParagraphCard(Rect),
    QuoteBar(Rect),
    Cell(Rect, bool),
    Rule(Rect),
    Math {
        rect: Rect,
        tex: String,
        size: f32,
        wrap: f32,
    },
    PushClip(Rect),
    PopClip,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub scroll_regions: Vec<ScrollRegion>,
    pub(super) items: Vec<Item>,
    pub(super) actions: Vec<CopyTarget>,
    pub(super) links: Vec<LinkTarget>,
    pub height: f32,
    pub tail: (f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyKind {
    Formula,
    Table,
    /// 围栏/缩进代码块。`CopyPayload::text` 是不带 Markdown 围栏的代码本体；
    /// `html` 刻意不提供。
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyPayload {
    pub kind: CopyKind,
    pub text: String,
    pub html: Option<String>,
}

/// AI 回复中可选择的可见文本 run。矩形是 Markdown 正文坐标，
/// 已包含当前横向滚动偏移。`text` 是用户实际看到的文本（不含 Markdown
/// 分隔符），保证拖拽复制在正文和代码卡里都可用。
#[derive(Debug, Clone, PartialEq)]
pub struct SelectableText {
    pub rect: Rect,
    pub text: String,
    pub style: TextStyle,
    pub emphasis: Emphasis,
}

/// [`Layout::selectable_text`] 里的一个稳定位置。偏移是 UTF-8 字节边界，
/// 与原生编辑器其余部分使用的文本整形、切片 API 保持一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPoint {
    pub fragment: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub(super) struct CopyTarget {
    pub(super) scroll: Option<usize>,
    pub(super) area: Rect,
    pub(super) hit: Rect,
    pub(super) payload: CopyPayload,
}

#[derive(Debug, Clone)]
pub(super) struct LinkTarget {
    pub(super) scroll: Option<usize>,
    pub(super) hit: Rect,
    pub(super) target: String,
}

pub(super) fn horizontal_distance(rect: Rect, x: f32) -> f32 {
    if x < rect.left {
        rect.left - x
    } else if x > rect.right {
        x - rect.right
    } else {
        0.0
    }
}

pub(super) fn vertical_distance(rect: Rect, y: f32) -> f32 {
    if y < rect.top {
        rect.top - y
    } else if y > rect.bottom {
        y - rect.bottom
    } else {
        0.0
    }
}

pub(super) fn utf8_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while offset > 0 && !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScrollId {
    pub anchor: usize,
    pub kind: u8,
}

pub type Offsets = std::collections::HashMap<ScrollId, f32>;

#[derive(Debug, Clone)]
pub struct ScrollRegion {
    pub id: ScrollId,
    pub viewport: Rect,
    pub track: Rect,
    pub content_width: f32,
    pub(super) items: std::ops::Range<usize>,
}

impl ScrollRegion {
    pub fn max_x(&self) -> f32 {
        (self.content_width - self.viewport.width()).max(0.0)
    }
    pub fn offset(&self, offsets: Option<&Offsets>) -> f32 {
        offsets
            .and_then(|m| m.get(&self.id))
            .copied()
            .filter(|x| x.is_finite())
            .unwrap_or(0.0)
            .clamp(0.0, self.max_x())
    }
    pub fn thumb(&self, offset: f32) -> Rect {
        let width = (self.track.width() * self.viewport.width() / self.content_width)
            .max(20.0)
            .min(self.track.width());
        let left = self.track.left
            + offset.clamp(0.0, self.max_x()) / self.max_x().max(1.0)
                * (self.track.width() - width);
        Rect::from_size(left, self.track.top, width, self.track.height())
    }
}
