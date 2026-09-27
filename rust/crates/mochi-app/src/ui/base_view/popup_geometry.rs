//! 计算数据表弹出菜单的位置和边界。
use super::*;

pub(super) fn origin(popup: Popup) -> Option<Hit> {
    Some(match popup {
        Popup::Table => Hit::TablesMenu,
        Popup::View => Hit::ViewsMenu,
        Popup::NewView => Hit::NewView,
        Popup::NewField => Hit::NewField,
        Popup::Field(f) => Hit::Field(f),
        Popup::Record(r) => Hit::RecordMenu(r),
        Popup::Select(r, f) | Popup::DateTime(r, f) | Popup::Reference(r, f) => Hit::Cell(r, f),
        Popup::Sort => Hit::Sort,
        Popup::Filter => Hit::Filter,
        Popup::RowHeight => Hit::RowHeight,
        Popup::Group => Hit::Group,
        Popup::Columns => Hit::Columns,
        Popup::Freeze => Hit::Freeze,
        _ => return None,
    })
}

pub(super) fn place(area: Rect, anchor: Option<Rect>, width: f32, height: f32) -> Rect {
    let margin = 8.0f32
        .min(area.width().max(0.0) / 4.0)
        .min(area.height().max(0.0) / 4.0);
    let bounds = Rect::new(
        area.left + margin,
        area.top + margin,
        area.right - margin,
        area.bottom - margin,
    );
    let width = width.min(bounds.width()).max(0.0);
    let anchor = anchor.unwrap_or(Rect::new(bounds.left, bounds.top, bounds.left, bounds.top));
    let below = (bounds.bottom - anchor.bottom - 6.0).max(0.0);
    let above = (anchor.top - bounds.top - 6.0).max(0.0);
    // 长菜单在空间较大的一侧滚动展开，而不是盖住触发它的控件。
    let down = height <= below || below >= above;
    let available = if down { below } else { above };
    let h = height
        .min(if available >= 46.0 {
            available
        } else {
            bounds.height()
        })
        .max(0.0);
    let y = if down {
        anchor.bottom + 6.0
    } else {
        anchor.top - h - 6.0
    };
    let y = y.clamp(bounds.top, (bounds.bottom - h).max(bounds.top));
    let x = anchor
        .left
        .clamp(bounds.left, (bounds.right - width).max(bounds.left));
    Rect::new(x, y, x + width, y + h)
}

pub(super) fn place_fixed(area: Rect, anchor: Option<Rect>, width: f32, height: f32) -> Rect {
    let mut rect = place(area, anchor, width, height);
    let h = height.min((area.height() - 16.0).max(0.0));
    if rect.height() < h {
        let top = rect.top.min(area.bottom - 8.0 - h).max(area.top + 8.0);
        rect.top = top;
        rect.bottom = top + h;
    }
    rect
}
