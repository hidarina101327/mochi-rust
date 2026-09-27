//! PDF 标注状态、页面几何及列表；数据契约来自 sidecars，UI 不调用平台 API。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::Palette,
};
use mochi_core::sidecars::PdfAnnotation;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Circle,
    Rect,
    Text,
}
impl Tool {
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "选择标注" => Some(Self::Select),
            "圆圈标注" => Some(Self::Circle),
            "矩形标注" => Some(Self::Rect),
            "文字标注" => Some(Self::Text),
            _ => None,
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::Circle => "circle",
            Self::Rect => "rect",
            Self::Text => "text",
            Self::Select => "select",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub page: i64,
    pub start: (f64, f64),
    pub annotation: PdfAnnotation,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub items: Vec<PdfAnnotation>,
    pub selected: Option<String>,
    pub tool: Tool,
    pub draft: Option<Draft>,
}
impl State {
    pub fn begin(&mut self, page: i64, point: (f64, f64), id: String, now: i64) {
        self.selected = None;
        self.draft = Some(Draft {
            page,
            start: point,
            annotation: new_annotation(self.tool, page, point, id, now),
        });
    }
    pub fn move_to(&mut self, point: (f64, f64)) {
        if let Some(d) = &mut self.draft {
            d.annotation.x = d.start.0.min(point.0);
            d.annotation.y = d.start.1.min(point.1);
            d.annotation.width = (point.0 - d.start.0).abs();
            d.annotation.height = (point.1 - d.start.1).abs();
        }
    }
    pub fn finish(&mut self) -> Option<PdfAnnotation> {
        let d = self.draft.take()?;
        (d.annotation.width >= 0.01 && d.annotation.height >= 0.01).then_some(d.annotation)
    }
    pub fn selected(&self) -> Option<&PdfAnnotation> {
        self.items
            .iter()
            .find(|a| Some(&a.id) == self.selected.as_ref())
    }
}
pub fn new_annotation(
    tool: Tool,
    page: i64,
    point: (f64, f64),
    id: String,
    now: i64,
) -> PdfAnnotation {
    PdfAnnotation {
        id,
        kind: tool.kind().into(),
        page,
        x: point.0,
        y: point.1,
        width: if tool == Tool::Text { 0.24 } else { 0.0 },
        height: if tool == Tool::Text { 0.055 } else { 0.0 },
        color: "#e11d48".into(),
        text: None,
        created_at: now,
        updated_at: now,
    }
}
pub fn position(page: Rect, x: f32, y: f32) -> (f64, f64) {
    (
        ((x - page.left) / page.width().max(1.0)).clamp(0.0, 1.0) as f64,
        ((y - page.top) / page.height().max(1.0)).clamp(0.0, 1.0) as f64,
    )
}
pub fn rect(page: Rect, a: &PdfAnnotation) -> Rect {
    Rect::new(
        page.left + a.x as f32 * page.width(),
        page.top + a.y as f32 * page.height(),
        page.left + (a.x + a.width) as f32 * page.width(),
        page.top + (a.y + a.height) as f32 * page.height(),
    )
}
pub fn hit(s: &State, page: Rect, number: i64, x: f32, y: f32) -> Option<String> {
    s.items
        .iter()
        .rev()
        .find(|a| {
            if a.page != number {
                return false;
            }
            let r = rect(page, a);
            if !r.contains(x, y) {
                return false;
            }
            if a.kind != "circle" {
                return true;
            }
            let dx = (x - (r.left + r.right) / 2.0) / (r.width() / 2.0).max(1.0);
            let dy = (y - (r.top + r.bottom) / 2.0) / (r.height() / 2.0).max(1.0);
            dx * dx + dy * dy <= 1.0
        })
        .map(|a| a.id.clone())
}
pub fn paint_page(list: &mut DrawList, page: Rect, number: i64, s: &State) {
    list.push_clip(page);
    for a in s
        .items
        .iter()
        .chain(s.draft.as_ref().map(|d| &d.annotation))
        .filter(|a| a.page == number)
    {
        let r = rect(page, a);
        let color = u32::from_str_radix(a.color.trim_start_matches('#'), 16).unwrap_or(0xe11d48);
        if a.kind == "text" {
            list.rect_alpha(r, 0xe11d48, 0.12);
        }
        list.shape_border(
            r,
            a.kind == "circle",
            if s.selected.as_ref() == Some(&a.id)
                || s.draft.as_ref().is_some_and(|d| d.annotation.id == a.id)
            {
                3.0
            } else {
                2.0
            },
            color,
        );
        if a.kind == "text" {
            list.text(
                Rect::new(r.left + page.width() * 0.008, r.top, r.right, r.bottom),
                text::ellipsize(
                    a.text.as_deref().unwrap_or(""),
                    TextStyle::Label,
                    (r.width() - 8.0).max(0.0),
                ),
                TextStyle::Label,
                color,
            );
        }
    }
    list.pop_clip();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    Locate(String),
    Delete(String),
}
#[derive(Default)]
pub struct Layout {
    pub list: Rect,
    pub entries: Vec<(Rect, Hit)>,
    pub content_height: f32,
    rows: Vec<(Rect, usize)>,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if !self.list.contains(x, y) {
            return None;
        }
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h.clone())
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.list.height()).max(0.0)
    }
}
pub fn layout(area: Rect, items: &[PdfAnnotation], scroll: f32) -> Layout {
    let mut lay = Layout {
        list: Rect::new(area.left, area.top + 57.0, area.right, area.bottom),
        content_height: 16.0 + items.len() as f32 * 64.0,
        ..Layout::default()
    };
    let mut order = (0..items.len()).collect::<Vec<_>>();
    order.sort_by_key(|i| (items[*i].page, items[*i].created_at));
    for (row, i) in order.into_iter().enumerate() {
        let y = lay.list.top + 8.0 + row as f32 * 64.0 - scroll;
        let r = Rect::new(area.left + 8.0, y, area.right - 8.0, y + 60.0);
        lay.entries.push((r, Hit::Locate(items[i].id.clone())));
        lay.entries.push((
            Rect::new(r.right - 36.0, y + 8.0, r.right - 8.0, y + 36.0),
            Hit::Delete(items[i].id.clone()),
        ));
        lay.rows.push((r, i));
    }
    lay
}
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    name: Option<&str>,
    items: &[PdfAnnotation],
    lay: &Layout,
    p: &Palette,
) {
    list.push_clip(area);
    list.text(
        Rect::new(
            area.left + 12.0,
            area.top + 8.0,
            area.right - 36.0,
            area.top + 28.0,
        ),
        "PDF 标注",
        TextStyle::Label,
        p.foreground,
    );
    list.text(
        Rect::new(
            area.left + 12.0,
            area.top + 32.0,
            area.right - 36.0,
            area.top + 48.0,
        ),
        name.unwrap_or("打开 PDF 后查看标注"),
        TextStyle::Caption,
        p.muted,
    );
    list.icon_centered(
        Rect::new(
            area.right - 32.0,
            area.top,
            area.right - 12.0,
            area.top + 56.0,
        ),
        Icon::FILE_TEXT,
        16.0,
        p.muted,
    );
    list.hline(area.left, area.right, area.top + 56.0, p.border);
    list.push_clip(lay.list);
    if name.is_none() || items.is_empty() {
        list.text(
            Rect::new(
                area.left + 16.0,
                lay.list.top + 32.0,
                area.right - 16.0,
                lay.list.top + 56.0,
            ),
            if name.is_none() {
                "当前文档不是 PDF"
            } else {
                "该 PDF 还没有标注"
            },
            TextStyle::Label,
            p.muted,
        );
    }
    for (r, i) in &lay.rows {
        if r.bottom < lay.list.top || r.top > lay.list.bottom {
            continue;
        }
        let a = &items[*i];
        list.icon_centered(
            Rect::new(r.left + 8.0, r.top + 8.0, r.left + 22.0, r.top + 28.0),
            match a.kind.as_str() {
                "circle" => Icon::CIRCLE,
                "rect" => Icon::SQUARE,
                _ => Icon::TYPE,
            },
            14.0,
            p.muted,
        );
        list.text(
            Rect::new(r.left + 30.0, r.top + 8.0, r.right - 40.0, r.top + 28.0),
            text::ellipsize(&a.label(), TextStyle::Label, (r.width() - 70.0).max(0.0)),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(r.left + 8.0, r.top + 32.0, r.right - 40.0, r.top + 48.0),
            format!("第 {} 页", a.page),
            TextStyle::Caption,
            p.muted,
        );
        list.icon_centered(
            Rect::new(r.right - 36.0, r.top + 8.0, r.right - 8.0, r.top + 36.0),
            Icon::TRASH2,
            14.0,
            p.muted,
        );
    }
    list.pop_clip();
    list.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_drag_is_normalized_and_tiny_drags_do_not_create_notes() {
        let mut s = State {
            tool: Tool::Rect,
            ..State::default()
        };
        s.begin(2, (0.8, 0.7), "id".into(), 1);
        s.move_to((0.2, 0.3));
        let a = s.finish().unwrap();
        assert!((a.width - 0.6).abs() < 0.00001);
        assert_eq!(a.x, 0.2);
        s.begin(2, (0.1, 0.1), "tiny".into(), 2);
        assert!(s.finish().is_none());
    }
    #[test]
    fn coordinates_follow_page_zoom() {
        let a = new_annotation(Tool::Text, 1, (0.5, 0.25), "a".into(), 1);
        let page = Rect::new(100.0, 200.0, 900.0, 1200.0);
        assert_eq!(position(page, 500.0, 450.0), (0.5, 0.25));
        assert_eq!(rect(page, &a).left, 500.0);
        assert_eq!(position(page, 0.0, 2000.0), (0.0, 1.0));
    }
    #[test]
    fn delete_hit_wins_and_list_is_sorted() {
        let area = Rect::new(0.0, 0.0, 320.0, 600.0);
        let items = vec![
            new_annotation(Tool::Rect, 3, (0.0, 0.0), "last".into(), 1),
            new_annotation(Tool::Text, 1, (0.0, 0.0), "first".into(), 1),
        ];
        let lay = layout(area, &items, 0.0);
        assert_eq!(lay.hit(300.0, 80.0), Some(Hit::Delete("first".into())));
        assert_eq!(lay.hit(20.0, 80.0), Some(Hit::Locate("first".into())));
        let mut list = DrawList::new();
        paint(
            &mut list,
            area,
            Some("a.pdf"),
            &items,
            &lay,
            super::super::theme::tokens().palette(false),
        );
        assert!(list.finish().is_ok());
    }
}
