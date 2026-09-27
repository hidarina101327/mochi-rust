//! 处理数据表界面的指针移动、点击和滚动条拖动。
use super::*;

impl State {
    pub fn dragging(&self) -> bool {
        self.drag.is_some() || self.scrollbars.dragging()
    }

    pub fn cancel_scrollbar_drag(&mut self) -> bool {
        self.scrollbars.end()
    }

    pub fn scrollbar_dragging(&self) -> bool {
        self.scrollbars.dragging()
    }

    pub fn pointer_leave(&mut self) -> bool {
        self.scrollbars.hover.take().is_some()
    }

    pub fn pointer_down(&mut self, area: Rect, x: f32, y: f32) -> bool {
        let l = layout(self, area);
        if let Some((axis, value)) = self.scrollbars.begin(&l.scrollbars, x, y) {
            self.set_scroll_offset(axis, value);
            return true;
        }
        match l.hit(x, y) {
            Some(Hit::Resize(field)) => {
                self.drag = Some(Drag::Resize {
                    field,
                    start_x: x,
                    width: width(&self.table().fields[field]),
                });
                true
            }
            Some(Hit::Cell(record, field))
                if self.editing
                    && self
                        .table()
                        .fields
                        .get(field)
                        .is_some_and(|field| field.field_type == FieldType::Progress) =>
            {
                let Some(rect) = l.rect_of(Hit::Cell(record, field)) else {
                    return false;
                };
                let cell = l
                    .grid
                    .as_ref()
                    .filter(|_| self.detail.is_none())
                    .and_then(|grid| {
                        let column = grid.columns.iter().find(|column| column.index == field)?;
                        let row = grid.rows.iter().find(|row| row.index == record)?;
                        Some(Rect::new(column.start, row.start, column.end, row.end))
                    })
                    .unwrap_or(rect);
                let (bar, _) = progress_regions(cell);
                if x <= bar.right {
                    self.drag = Some(Drag::Progress {
                        record,
                        field,
                        left: bar.left,
                        width: bar.width().max(1.0),
                    });
                    self.drag_to(area, x);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn drag_to(&mut self, area: Rect, x: f32) -> bool {
        // 旧调用方用这个方法做横向滚动/列宽调整。
        if self.scrollbars.dragging() {
            let l = layout(self, area);
            if let Some((axis, value)) = self.scrollbars.drag_to(&l.scrollbars, x, area.top) {
                return self.set_scroll_offset(axis, value);
            }
        }
        match self.drag.clone() {
            Some(Drag::Resize {
                field,
                start_x,
                width,
            }) => {
                let next = (width + x - start_x).clamp(100.0, 600.0) as f64;
                let Some(field) = self.table_mut().fields.get_mut(field) else {
                    self.drag = None;
                    return false;
                };
                if field.width == Some(next) {
                    return false;
                }
                field.width = Some(next);
                self.changed();
                true
            }
            Some(Drag::Progress {
                record,
                field,
                left,
                width,
            }) => {
                let value = (((x - left) / width) * 100.0).round().clamp(0.0, 100.0);
                let Some(id) = self.table().fields.get(field).map(|field| field.id.clone()) else {
                    self.drag = None;
                    return false;
                };
                let Some(record) = self.table_mut().records.get_mut(record) else {
                    self.drag = None;
                    return false;
                };
                if record.values.get(&id).and_then(Value::as_f64) == Some(value as f64) {
                    return false;
                }
                record.values.insert(id, json!(value));
                self.changed();
                true
            }
            None => false,
        }
    }

    pub fn end_drag(&mut self) -> bool {
        self.scrollbars.end() | self.drag.take().is_some()
    }

    pub(super) fn set_scroll_offset(&mut self, axis: Axis, value: f32) -> bool {
        let offset = match axis {
            Axis::Vertical => &mut self.scroll_y,
            Axis::Horizontal => &mut self.scroll_x,
        };
        let changed = (*offset - value).abs() > 0.01;
        *offset = value;
        changed
    }

    pub fn pointer(&mut self, area: Rect, x: f32, y: f32) -> bool {
        if self.scrollbars.dragging() {
            let l = layout(self, area);
            self.scrollbars.pointer(&l.scrollbars, x, y);
            if let Some((axis, value)) = self.scrollbars.drag_to(&l.scrollbars, x, y) {
                return self.set_scroll_offset(axis, value);
            }
        }
        if self.dragging() {
            return self.drag_to(area, x);
        }
        if !area.contains(x, y) {
            return self.hover.take().is_some() | self.pointer_leave();
        }
        let l = layout(self, area);
        let bar_changed = self.scrollbars.pointer(&l.scrollbars, x, y);
        let hit = if area.contains(x, y) {
            l.hit(x, y)
        } else {
            None
        };
        let changed = self.hover != hit;
        self.hover = hit;
        changed || bar_changed
    }
}
