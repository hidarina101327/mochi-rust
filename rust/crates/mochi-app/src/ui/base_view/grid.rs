//! 定义数据表网格的布局结构并提供网格坐标计算。
use super::*;

pub(super) const HEADER: f32 = 36.0;
pub(super) const GUTTER: f32 = 44.0;

#[derive(Debug)]
pub(super) struct Line {
    pub index: usize,
    pub position: usize,
    pub start: f32,
    pub end: f32,
    pub clip_start: f32,
    pub clip_end: f32,
    pub frozen: bool,
}

#[derive(Debug)]
pub(super) struct Grid {
    pub columns: Vec<Line>,
    pub rows: Vec<Line>,
    pub row_count: usize,
    pub frozen_x: f32,
    pub frozen_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

pub(super) fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let r = Rect::new(
        a.left.max(b.left),
        a.top.max(b.top),
        a.right.min(b.right),
        a.bottom.min(b.bottom),
    );
    (r.width() > 0.0 && r.height() > 0.0).then_some(r)
}

impl Grid {
    pub fn new(s: &State, body: Rect) -> Self {
        let fields = s.visible_fields();
        let records = s.visible_records();
        let height = grid_row_height(s);
        let left = (body.left + GUTTER).min(body.right);
        let top = (body.top + HEADER).min(body.bottom);
        let available_x = (body.right - left).max(0.0);
        let available_y = (body.bottom - top).max(0.0);
        let total_x: f32 = fields.iter().map(|&f| width(&s.table().fields[f])).sum();
        let max_x = (total_x - available_x).max(0.0);
        let max_y = (records.len() as f32 * height - available_y).max(0.0);
        let mut frozen_columns = 0;
        let mut frozen_width = 0.0;
        let budget = if max_x > 0.0 {
            (available_x - 100.0).max(0.0)
        } else {
            available_x
        };
        for &f in fields.iter().take(s.view().frozen_columns as usize) {
            let w = width(&s.table().fields[f]);
            if frozen_width + w > budget {
                break;
            }
            frozen_width += w;
            frozen_columns += 1;
        }
        let row_budget = if max_y > 0.0 {
            (available_y - height).max(0.0)
        } else {
            available_y
        };
        let frozen_rows = (s.view().frozen_rows as usize)
            .min(records.len())
            .min((row_budget / height) as usize);
        let frozen_x = left + frozen_width;
        let frozen_y = top + frozen_rows as f32 * height;
        let sx = s.scroll_x.clamp(0.0, max_x);
        let sy = s.scroll_y.clamp(0.0, max_y);
        let mut columns = Vec::new();
        let mut x = left;
        for (position, &index) in fields.iter().enumerate() {
            let w = width(&s.table().fields[index]);
            let frozen = position < frozen_columns;
            let start = x - if frozen { 0.0 } else { sx };
            let clip_start = start.max(if frozen { left } else { frozen_x });
            let clip_end = (start + w).min(body.right);
            if clip_end > clip_start {
                columns.push(Line {
                    index,
                    position,
                    start,
                    end: start + w,
                    clip_start,
                    clip_end,
                    frozen,
                });
            }
            x += w;
        }
        let mut rows = Vec::new();
        let first = frozen_rows + (sy / height) as usize;
        let count = ((available_y / height).ceil() as usize).saturating_add(1);
        for position in
            (0..frozen_rows).chain(first..first.saturating_add(count).min(records.len()))
        {
            let frozen = position < frozen_rows;
            let start = top + position as f32 * height - if frozen { 0.0 } else { sy };
            let clip_start = start.max(if frozen { top } else { frozen_y });
            let clip_end = (start + height).min(body.bottom);
            if clip_end > clip_start {
                rows.push(Line {
                    index: records[position],
                    position,
                    start,
                    end: start + height,
                    clip_start,
                    clip_end,
                    frozen,
                });
            }
        }
        Self {
            columns,
            rows,
            row_count: records.len(),
            frozen_x,
            frozen_y,
            max_x,
            max_y,
        }
    }

    pub fn add_hits(&self, l: &mut Layout, s: &State) {
        let mut resize_entries = Vec::new();
        for column in &self.columns {
            l.entries.push((
                Rect::new(
                    column.clip_start,
                    l.body.top,
                    column.clip_end,
                    l.body.top + HEADER,
                ),
                Hit::Field(column.index),
            ));
            if column.end >= column.clip_start + 4.0 && column.end <= column.clip_end {
                resize_entries.push((
                    Rect::new(
                        column.end - 4.0,
                        l.body.top,
                        (column.end + 4.0).min(l.body.right),
                        l.body.top + HEADER,
                    ),
                    Hit::Resize(column.index),
                ));
            }
        }
        l.entries.extend(resize_entries);
        for row in &self.rows {
            l.entries.push((
                Rect::new(
                    l.body.left,
                    row.clip_start,
                    (l.body.left + GUTTER).min(l.body.right),
                    row.clip_end,
                ),
                Hit::Record(row.index),
            ));
            for column in &self.columns {
                let clip = Rect::new(
                    column.clip_start,
                    row.clip_start,
                    column.clip_end,
                    row.clip_end,
                );
                l.entries.push((clip, Hit::Cell(row.index, column.index)));
                if s.table().fields[column.index].field_type == FieldType::Progress {
                    let cell = Rect::new(column.start, row.start, column.end, row.end);
                    let (_, input) = super::progress_regions(cell);
                    if let Some(input) = intersect(input, clip) {
                        l.entries
                            .push((input, Hit::ProgressInput(row.index, column.index)));
                    }
                }
            }
        }
    }
}

pub(super) fn paint(list: &mut DrawList, s: &State, l: &Layout, g: &Grid, p: &Palette) {
    let body = l.body;
    list.rect(
        Rect::new(body.left, body.top, body.right, body.top + HEADER),
        p.surface_muted,
    );
    list.rect(
        Rect::new(
            body.left,
            body.top + HEADER,
            (body.left + GUTTER).min(body.right),
            body.bottom,
        ),
        p.surface_muted,
    );
    for row in &g.rows {
        list.push_clip(Rect::new(
            body.left,
            row.clip_start,
            body.left + GUTTER,
            row.clip_end,
        ));
        list.text(
            Rect::new(body.left + 10.0, row.start, body.left + 42.0, row.end),
            (row.position + 1).to_string(),
            TextStyle::Caption,
            p.muted,
        );
        list.pop_clip();
        for column in &g.columns {
            let rect = Rect::new(column.start, row.start, column.end, row.end);
            list.push_clip(Rect::new(
                column.clip_start,
                row.clip_start,
                column.clip_end,
                row.clip_end,
            ));
            if row.frozen || column.frozen {
                list.rect(rect, p.background);
            }
            if s.selected == Some((row.index, column.index)) {
                list.rect(rect, theme::mix(p.accent, p.background, 0.07));
            } else if s.hover == Some(Hit::Cell(row.index, column.index)) {
                list.rect(rect, theme::mix(p.surface_muted, p.background, 0.6));
            }
            super::table_border(list, rect, p.border);
            let field = &s.table().fields[column.index];
            cell(
                list,
                rect,
                field,
                s.table().records[row.index]
                    .values
                    .get(&field.id)
                    .unwrap_or(&Value::Null),
                p,
            );
            if s.selected == Some((row.index, column.index)) {
                super::table_border(list, rect, p.accent);
            }
            list.pop_clip();
        }
    }
    for column in &g.columns {
        let field = &s.table().fields[column.index];
        let rect = Rect::new(column.start, body.top, column.end, body.top + HEADER);
        list.push_clip(Rect::new(
            column.clip_start,
            rect.top,
            column.clip_end,
            rect.bottom,
        ));
        if s.located
            .as_ref()
            .and_then(|location| location.field_id.as_deref())
            == Some(field.id.as_str())
        {
            list.rect(rect, theme::mix(p.accent, p.surface_muted, 0.12));
        }
        list.text(
            Rect::new(rect.left + 10.0, rect.top, rect.right - 10.0, rect.bottom),
            text::ellipsize(
                &format!(
                    "{} · {}{}",
                    field.name,
                    type_name(field.field_type),
                    if s.editing { " ▾" } else { "" }
                ),
                TextStyle::Caption,
                rect.width() - 20.0,
            ),
            TextStyle::Caption,
            p.muted,
        );
        let border_width = super::table_border_width();
        if border_width > 0.0 {
            list.rect(
                Rect::new(
                    rect.right - border_width,
                    rect.top + 8.0,
                    rect.right,
                    rect.bottom - 8.0,
                ),
                if s.hover == Some(Hit::Resize(column.index)) {
                    p.accent
                } else {
                    p.border
                },
            );
        }
        list.pop_clip();
    }
    let divider = theme::mix(p.foreground, p.border, 0.25);
    if g.frozen_x > body.left + GUTTER {
        list.rect(
            Rect::new(g.frozen_x - 1.0, body.top, g.frozen_x + 1.0, body.bottom),
            divider,
        );
    }
    if g.frozen_y > body.top + HEADER {
        list.hline(body.left, body.right, g.frozen_y, divider);
    }
    if g.row_count == 0 {
        list.text(
            Rect::new(
                body.left + 66.0,
                body.top + 65.0,
                body.right - 20.0,
                body.top + 100.0,
            ),
            if s.table().records.is_empty() {
                "数据表为空，点击「编辑」或「＋ 记录」开始填写"
            } else {
                "没有符合当前筛选条件的记录"
            },
            TextStyle::Label,
            p.muted,
        );
    }
}
