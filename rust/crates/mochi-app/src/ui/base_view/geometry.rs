//! 计算数据表视图的行高、列宽和控件位置。
use super::*;

pub(super) fn grid_row_height(s: &State) -> f32 {
    let minimum = (TextStyle::Table.line_height() + 4.0).clamp(28.0, 96.0);
    s.view()
        .row_height
        .unwrap_or(DEFAULT_GRID_ROW_HEIGHT as f64)
        .clamp(minimum as f64, 96.0) as f32
}

pub(super) fn type_name(kind: FieldType) -> &'static str {
    TYPES
        .iter()
        .find(|(t, _)| *t == kind)
        .map(|(_, name)| *name)
        .unwrap_or("文本")
}

pub(super) fn width(field: &BaseField) -> f32 {
    field
        .width
        .unwrap_or(match field.field_type {
            FieldType::DateRange => 330.0,
            FieldType::DateTime => 190.0,
            _ => 180.0,
        })
        .clamp(100.0, 600.0) as f32
}

fn add_button(l: &mut Layout, x: &mut f32, y: f32, label: &str, hit: Hit, right: f32) {
    let w = (text::measure(label, TextStyle::Caption) + 22.0).max(34.0);
    if *x + w <= right {
        l.entries.push((Rect::new(*x, y, *x + w, y + 30.0), hit));
        *x += w + 5.0;
    }
}

fn add_trailing_button(l: &mut Layout, right: &mut f32, y: f32, label: &str, hit: Hit, left: f32) {
    let w = (text::measure(label, TextStyle::Caption) + 22.0).max(34.0);
    if *right - w >= left {
        l.entries
            .push((Rect::new(*right - w, y, *right, y + 30.0), hit));
        *right -= w + 5.0;
    }
}

const DATE_TIME_PARTS: [(DateTimePart, &str); 5] = [
    (DateTimePart::Year, "年"),
    (DateTimePart::Month, "月"),
    (DateTimePart::Day, "日"),
    (DateTimePart::Hour, "时"),
    (DateTimePart::Minute, "分"),
];

pub(super) fn date_time_picker_controls(
    menu: Rect,
) -> Vec<(DateTimePart, &'static str, Rect, Rect, Rect, Rect)> {
    DATE_TIME_PARTS
        .iter()
        .enumerate()
        .map(|(index, &(part, label))| {
            let top = menu.top + 62.0 + index as f32 * 27.0;
            let label_rect = Rect::new(menu.left + 14.0, top, menu.left + 70.0, top + 23.0);
            let minus = Rect::new(menu.right - 148.0, top, menu.right - 112.0, top + 23.0);
            let value = Rect::new(menu.right - 108.0, top, menu.right - 44.0, top + 23.0);
            let plus = Rect::new(menu.right - 40.0, top, menu.right - 4.0, top + 23.0);
            (part, label, label_rect, minus, value, plus)
        })
        .collect()
}

pub(super) fn date_time_part_value(picker: DateTimePicker, part: DateTimePart) -> String {
    match part {
        DateTimePart::Year => format!("{:04}", picker.year),
        DateTimePart::Month | DateTimePart::Day | DateTimePart::Hour | DateTimePart::Minute => {
            let value = match part {
                DateTimePart::Month => picker.month,
                DateTimePart::Day => picker.day,
                DateTimePart::Hour => picker.hour,
                DateTimePart::Minute => picker.minute,
                DateTimePart::Year => unreachable!(),
            };
            format!("{value:02}")
        }
    }
}

pub fn layout(s: &State, area: Rect) -> Layout {
    let chrome = if s.action_row() { 132.0 } else { 85.0 };
    let status = if s.error.is_empty() { 0.0 } else { 32.0 };
    let mut l = Layout {
        body: Rect::new(
            area.left,
            area.top + chrome,
            area.right,
            area.bottom - status,
        ),
        ..Default::default()
    };
    l.entries.push((area, Hit::Body));
    let mut x = area.left + 12.0;
    add_button(
        &mut l,
        &mut x,
        area.top + 10.0,
        "数据表",
        Hit::TablesMenu,
        area.right - 8.0,
    );
    for (i, t) in s.document.tables.iter().enumerate() {
        add_button(
            &mut l,
            &mut x,
            area.top + 10.0,
            &t.name,
            Hit::Table(i),
            area.right - 8.0,
        );
    }
    if s.editing {
        add_button(
            &mut l,
            &mut x,
            area.top + 10.0,
            "＋",
            Hit::NewTable,
            area.right - 8.0,
        );
    }
    let mut presentation_tools = area.right - 10.0;
    for (label, hit) in [("导出", Hit::ExportXlsx), ("显示列", Hit::Columns)] {
        add_trailing_button(
            &mut l,
            &mut presentation_tools,
            area.top + 10.0,
            label,
            hit,
            x,
        );
    }
    if s.grid() {
        add_trailing_button(
            &mut l,
            &mut presentation_tools,
            area.top + 10.0,
            "冻结",
            Hit::Freeze,
            x,
        );
    }
    let mut tools = area.right - 10.0;
    add_trailing_button(
        &mut l,
        &mut tools,
        area.top + 51.0,
        if s.editing { "查看" } else { "编辑" },
        Hit::ToggleEditing,
        area.left + 120.0,
    );
    add_trailing_button(
        &mut l,
        &mut tools,
        area.top + 51.0,
        "自动化",
        Hit::Automations,
        area.left + 120.0,
    );
    add_trailing_button(
        &mut l,
        &mut tools,
        area.top + 51.0,
        "排序",
        Hit::Sort,
        area.left + 120.0,
    );
    if s.grid() {
        add_trailing_button(
            &mut l,
            &mut tools,
            area.top + 51.0,
            "行高",
            Hit::RowHeight,
            area.left + 120.0,
        );
    }
    add_trailing_button(
        &mut l,
        &mut tools,
        area.top + 51.0,
        "筛选",
        Hit::Filter,
        area.left + 120.0,
    );
    x = area.left + 12.0;
    add_button(
        &mut l,
        &mut x,
        area.top + 51.0,
        "视图",
        Hit::ViewsMenu,
        tools,
    );
    for (i, v) in s.table().views.iter().enumerate() {
        add_button(
            &mut l,
            &mut x,
            area.top + 51.0,
            &format!(
                "{} {}",
                if v.view_type == ViewType::Grid {
                    "▦"
                } else {
                    "▥"
                },
                v.name
            ),
            Hit::View(i),
            tools,
        );
    }
    if s.editing {
        add_button(&mut l, &mut x, area.top + 51.0, "＋", Hit::NewView, tools);
    }
    if s.action_row() {
        x = area.left + 12.0;
        if s.editing {
            for (label, hit) in [("＋ 记录", Hit::NewRecord), ("＋ 字段", Hit::NewField)] {
                add_button(
                    &mut l,
                    &mut x,
                    area.top + 91.0,
                    label,
                    hit,
                    area.right - 8.0,
                );
            }
            add_button(
                &mut l,
                &mut x,
                area.top + 91.0,
                "＋ 临时文档",
                Hit::NewTemporaryDocument,
                area.right - 8.0,
            );
        }
        if s.located.is_some() {
            add_button(
                &mut l,
                &mut x,
                area.top + 91.0,
                "返回视图",
                Hit::ClearLocation,
                area.right - 8.0,
            );
        }
        if s.view().view_type == ViewType::Board {
            add_button(
                &mut l,
                &mut x,
                area.top + 91.0,
                "分组",
                Hit::Group,
                area.right - 8.0,
            );
        }
    }
    if s.grid() {
        let grid = grid::Grid::new(s, l.body);
        l.max_x = grid.max_x;
        l.max_y = grid.max_y;
        grid.add_hits(&mut l, s);
        l.grid = Some(grid);
    } else {
        let groups = s.groups();
        let fields = s.visible_fields();
        let card_h = 38.0 + fields.len() as f32 * 34.0;
        l.max_x = (groups.len() as f32 * 280.0 + 12.0 - l.body.width()).max(0.0);
        l.max_y = (groups.iter().map(|(_, rs)| rs.len()).max().unwrap_or(0) as f32
            * (card_h + 12.0)
            + 48.0
            - l.body.height())
        .max(0.0);
        for (g, (_, rows)) in groups.iter().enumerate() {
            let left = l.body.left + 12.0 + g as f32 * 280.0 - s.scroll_x.min(l.max_x);
            if left + 268.0 < l.body.left || left > l.body.right {
                continue;
            }
            for (pos, &r) in rows.iter().enumerate() {
                let y = l.body.top + 48.0 + pos as f32 * (card_h + 12.0) - s.scroll_y.min(l.max_y);
                if y + card_h < l.body.top + 42.0 {
                    continue;
                }
                if y > l.body.bottom {
                    break;
                }
                let mut add = |rect: Rect, hit: Hit| {
                    let rect = Rect::new(
                        rect.left.max(l.body.left),
                        rect.top.max(l.body.top + 42.0),
                        rect.right.min(l.body.right),
                        rect.bottom.min(l.body.bottom),
                    );
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        l.entries.push((rect, hit));
                    }
                };
                add(
                    Rect::new(left + 6.0, y, left + 262.0, y + 32.0),
                    Hit::Record(r),
                );
                for (position, &f) in fields.iter().enumerate() {
                    add(
                        Rect::new(
                            left + 8.0,
                            y + 34.0 + position as f32 * 34.0,
                            left + 260.0,
                            y + 68.0 + position as f32 * 34.0,
                        ),
                        Hit::Cell(r, f),
                    );
                }
            }
        }
    }
    if s.popup.is_none() && s.detail.is_none() && s.date_time_picker.is_none() {
        let body = Rect::new(
            l.body.left,
            l.body.top + if s.grid() { 36.0 } else { 42.0 },
            l.body.right,
            l.body.bottom,
        );
        for (axis, max, offset, other) in [
            (Axis::Horizontal, l.max_x, s.scroll_x, l.max_y > 0.0),
            (Axis::Vertical, l.max_y, s.scroll_y, l.max_x > 0.0),
        ] {
            let body = if let Some(grid) = &l.grid {
                Rect::new(
                    if axis == Axis::Horizontal {
                        grid.frozen_x
                    } else {
                        body.left
                    },
                    if axis == Axis::Vertical {
                        grid.frozen_y
                    } else {
                        body.top
                    },
                    body.right,
                    body.bottom,
                )
            } else {
                body
            };
            if let Some(bar) = Bar::new(body, axis, max, offset, other) {
                if axis == Axis::Horizontal {
                    l.scroll_track = Some(bar.track);
                    l.scroll_thumb = Some(bar.thumb);
                }
                l.entries.push((bar.hotzone, Hit::ScrollTrack));
                l.entries.push((bar.thumb, Hit::ScrollThumb));
                l.scrollbars.push((axis, bar));
            }
        }
    }
    if let Some(record) = s.detail.filter(|record| *record < s.table().records.len()) {
        let total = s
            .table()
            .fields
            .iter()
            .map(|field| detail_height(s, record, field))
            .sum::<f32>();
        let width = 600.0f32.min((area.width() - 64.0).max(160.0));
        let height = (total + 118.0).min((area.height() - 80.0).max(180.0));
        let left = area.left + (area.width() - width) / 2.0;
        let top = area.top + (area.height() - height) / 2.0;
        let panel = Rect::new(left, top, left + width, top + height);
        l.detail = Some(panel);
        l.entries.push((area, Hit::DetailBackdrop));
        l.entries.push((panel, Hit::DetailBody));
        l.entries.push((
            Rect::new(
                panel.right - 40.0,
                panel.top + 10.0,
                panel.right - 10.0,
                panel.top + 40.0,
            ),
            Hit::CloseDetail,
        ));
        l.entries.push((
            Rect::new(
                panel.left + 16.0,
                panel.top + 52.0,
                panel.left + 136.0,
                panel.top + 82.0,
            ),
            Hit::CopyRecord(record),
        ));
        l.entries.push((
            Rect::new(
                panel.right - 148.0,
                panel.top + 52.0,
                panel.right - 24.0,
                panel.top + 82.0,
            ),
            Hit::ToggleEditing,
        ));
        if s.editing {
            l.entries.push((
                Rect::new(
                    panel.left + 144.0,
                    panel.top + 52.0,
                    panel.left + 190.0,
                    panel.top + 82.0,
                ),
                Hit::RecordMenu(record),
            ));
        }
        let content = Rect::new(
            panel.left + 16.0,
            panel.top + 98.0,
            panel.right - 16.0,
            panel.bottom - 12.0,
        );
        l.detail_max = (total - content.height()).max(0.0);
        let mut y = content.top - s.detail_scroll.min(l.detail_max);
        for (field_index, field) in s.table().fields.iter().enumerate() {
            let h = detail_height(s, record, field);
            let mut add = |rect: Rect, hit: Hit| {
                let rect = Rect::new(
                    rect.left,
                    rect.top.max(content.top),
                    rect.right,
                    rect.bottom.min(content.bottom),
                );
                if rect.height() > 0.0 {
                    l.entries.push((rect, hit));
                }
            };
            if matches!(field.field_type, FieldType::Reference | FieldType::Document) {
                let urls = s.table().records[record]
                    .values
                    .get(&field.id)
                    .and_then(Value::as_array);
                for index in 0..urls.map_or(0, Vec::len) {
                    let top = y + 22.0 + index as f32 * 34.0;
                    add(
                        Rect::new(
                            content.left,
                            top,
                            content.right - if s.editing { 34.0 } else { 0.0 },
                            top + 30.0,
                        ),
                        Hit::OpenReference(record, field_index, index),
                    );
                    if s.editing {
                        add(
                            Rect::new(content.right - 30.0, top, content.right, top + 30.0),
                            Hit::RemoveReference(record, field_index, index),
                        );
                    }
                }
                if s.editing {
                    let top = y + 22.0 + urls.map_or(0, Vec::len) as f32 * 34.0;
                    add(
                        Rect::new(content.left, top, content.right, top + 30.0),
                        Hit::AddReference(record, field_index),
                    );
                }
            } else {
                add(
                    Rect::new(content.left, y + 22.0, content.right, y + h - 12.0),
                    Hit::Cell(record, field_index),
                );
            }
            y += h;
        }
    }
    if let Some(Popup::DateTime(record, field)) = s.popup {
        // 原生的显式年/月/日/时/分选择器。它有自己的命中目标，
        // 不复用通用的下拉菜单行。
        let h = 238.0f32.min((area.height() - 20.0).max(120.0));
        let w = 332.0f32.min((area.width() - 24.0).max(220.0));
        let anchor = l.rect_of(Hit::Cell(record, field));
        let menu = popup_geometry::place_fixed(area, anchor, w, h);
        l.menu = Some(menu);
        l.entries.push((area, Hit::Dismiss));
        for (part, _, _, minus, _, plus) in date_time_picker_controls(menu) {
            l.entries.push((minus, Hit::DateTimeAdjust(part, -1)));
            l.entries.push((plus, Hit::DateTimeAdjust(part, 1)));
        }
        l.entries.push((
            Rect::new(
                menu.left + 14.0,
                menu.bottom - 38.0,
                menu.right - 88.0,
                menu.bottom - 10.0,
            ),
            Hit::DateTimeConfirm,
        ));
        l.entries.push((
            Rect::new(
                menu.right - 80.0,
                menu.bottom - 38.0,
                menu.right - 14.0,
                menu.bottom - 10.0,
            ),
            Hit::DateTimeClear,
        ));
    } else if s.popup.is_some() {
        let items = s.menu_items();
        let h = (items.len() as f32 * 34.0 + 12.0).min((area.height() - 28.0).max(48.0));
        let w = 300.0f32.min((area.width() - 24.0).max(40.0));
        let anchor = s
            .popup_anchor
            .and_then(|hit| l.rect_of(hit))
            .map(|mut rect| {
                if matches!(
                    s.popup_anchor,
                    Some(
                        Hit::Sort
                            | Hit::Filter
                            | Hit::RowHeight
                            | Hit::Group
                            | Hit::Columns
                            | Hit::Freeze
                    )
                ) {
                    rect.left = rect.right - w;
                }
                rect
            });
        let r = popup_geometry::place(area, anchor, w, h);
        l.menu = Some(r);
        l.menu_max = (items.len() as f32 * 34.0 + 12.0 - r.height()).max(0.0);
        l.entries.push((area, Hit::Dismiss));
        for i in 0..items.len() {
            let y = r.top + 6.0 + i as f32 * 34.0 - s.popup_scroll.min(l.menu_max);
            let top = y.max(r.top + 6.0);
            let bottom = (y + 34.0).min(r.bottom - 6.0);
            if bottom > top {
                l.entries.push((
                    Rect::new(r.left + 6.0, top, r.right - 6.0, bottom),
                    Hit::Menu(i),
                ));
            }
        }
    }
    l
}
