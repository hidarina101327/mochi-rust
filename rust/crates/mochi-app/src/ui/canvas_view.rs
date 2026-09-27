//! 在无限画布上记录自由笔记，并可添加对象引用。
use super::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    theme::{self, Palette},
    widgets::TextField,
};
use mochi_core::canvas::{Bounds, CanvasDocument, Point, Stroke, TextNote};

mod interaction;
mod painting;
#[cfg(test)]
mod tests;
pub use painting::{layout, paint};

pub const COLORS: [u32; 6] = [0x334155, 0x2563eb, 0xe05252, 0x26966b, 0x9763d2, 0xd99621];
pub const WIDTHS: [f64; 3] = [2.0, 4.0, 8.0];
pub const FONT_SIZES: [f64; 3] = [16.0, 24.0, 36.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Text,
    Eraser,
    Hand,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Card(usize),
    Text(usize),
    Stroke(usize),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Gesture {
    Pan {
        pointer: (f32, f32),
        viewport: (f64, f64),
    },
    Draw,
    Erase(Point),
    Move(Point),
    Resize,
    TextSelect,
}
#[derive(Debug, Clone)]
pub struct TextEditor {
    pub index: usize,
    pub field: TextField,
}

#[derive(Debug, Clone)]
pub struct State {
    pub document: CanvasDocument,
    pub tool: Tool,
    pub color: usize,
    pub width: usize,
    pub font_size: usize,
    pub selected: Option<Element>,
    pub editor: Option<TextEditor>,
    pub grid: bool,
    pub(super) gesture: Option<Gesture>,
    pub(super) draft: Option<Stroke>,
    pub(super) checkpoint: Option<CanvasDocument>,
    pub(super) undo: Vec<CanvasDocument>,
    pub(super) redo: Vec<CanvasDocument>,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.document == other.document
            && self.tool == other.tool
            && self.color == other.color
            && self.width == other.width
            && self.font_size == other.font_size
            && self.selected == other.selected
            && self.grid == other.grid
            && self.gesture == other.gesture
            && self.draft == other.draft
            && self
                .editor
                .as_ref()
                .map(|e| (e.index, e.field.text(), e.field.buffer.cursor()))
                == other
                    .editor
                    .as_ref()
                    .map(|e| (e.index, e.field.text(), e.field.buffer.cursor()))
    }
}

impl State {
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(Self {
            document: mochi_core::canvas::parse(raw)?,
            tool: Tool::Select,
            color: 0,
            width: 1,
            font_size: 1,
            selected: None,
            editor: None,
            grid: true,
            gesture: None,
            draft: None,
            checkpoint: None,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub fn serialized(&self) -> anyhow::Result<String> {
        mochi_core::canvas::serialize(&self.document)
    }
    pub fn add_references(
        &mut self,
        urls: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<usize> {
        self.finish_editing();
        let mut next = self.document.clone();
        let before = next.cards.len();
        for url in urls {
            next.add_mochi_reference(url)?;
        }
        for (offset, card) in next.cards[before..].iter_mut().enumerate() {
            card.x = next.viewport.x + 48.0 + offset as f64 * 32.0;
            card.y = next.viewport.y + 48.0 + offset as f64 * 32.0;
        }
        self.begin_change();
        self.document = next;
        self.finish_change();
        Ok(self.document.cards.len() - before)
    }
    pub fn world(&self, body: Rect, x: f32, y: f32) -> Point {
        let v = &self.document.viewport;
        Point {
            x: v.x + f64::from(x - body.left) / v.zoom,
            y: v.y + f64::from(y - body.top) / v.zoom,
        }
    }
    pub fn screen_rect(&self, body: Rect, b: Bounds) -> Rect {
        let v = &self.document.viewport;
        Rect::new(
            body.left + ((b.left - v.x) * v.zoom) as f32,
            body.top + ((b.top - v.y) * v.zoom) as f32,
            body.left + ((b.right - v.x) * v.zoom) as f32,
            body.top + ((b.bottom - v.y) * v.zoom) as f32,
        )
    }
    pub fn zoom_at(&mut self, body: Rect, x: f32, y: f32, factor: f64) -> bool {
        if !factor.is_finite() || factor <= 0.0 || self.gesture.is_some() {
            return false;
        }
        let anchor = self.world(body, x, y);
        let v = &mut self.document.viewport;
        let next =
            (v.zoom * factor).clamp(mochi_core::canvas::MIN_ZOOM, mochi_core::canvas::MAX_ZOOM);
        if (next - v.zoom).abs() < f64::EPSILON {
            return false;
        }
        v.zoom = next;
        v.x = anchor.x - f64::from(x - body.left) / next;
        v.y = anchor.y - f64::from(y - body.top) / next;
        true
    }
    pub fn reset_view(&mut self) -> bool {
        let changed = self.document.viewport != Default::default();
        self.document.viewport = Default::default();
        changed
    }
    pub fn fit_to_content(&mut self, body: Rect) -> bool {
        let Some(b) = self.document.bounds() else {
            return self.reset_view();
        };
        if body.is_empty() {
            return false;
        }
        let zoom = (f64::from((body.width() - 80.0).max(1.0)) / (b.right - b.left).max(1.0))
            .min(f64::from((body.height() - 80.0).max(1.0)) / (b.bottom - b.top).max(1.0))
            .clamp(mochi_core::canvas::MIN_ZOOM, 1.0);
        self.document.viewport = mochi_core::canvas::Viewport {
            x: (b.left + b.right) / 2.0 - f64::from(body.width()) / (2.0 * zoom),
            y: (b.top + b.bottom) / 2.0 - f64::from(body.height()) / (2.0 * zoom),
            zoom,
        };
        true
    }
    pub fn bounds(&self, element: Element) -> Option<Bounds> {
        match element {
            Element::Card(i) => self.document.cards.get(i).map(|c| Bounds {
                left: c.x,
                top: c.y,
                right: c.x + c.width,
                bottom: c.y + c.height,
            }),
            Element::Text(i) => self.document.texts.get(i).map(TextNote::bounds),
            Element::Stroke(i) => self.document.strokes.get(i).and_then(Stroke::bounds),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    AddReference,
    ZoomOut,
    ZoomIn,
    ResetZoom,
    FitContent,
    Surface,
    Tool(Tool),
    Color(usize),
    Width(usize),
    FontSize(usize),
    Undo,
    Redo,
    Delete,
    Grid,
    EditText,
    Resize,
}
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub body: Rect,
    pub entries: Vec<(Rect, Hit)>,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }
}
