//! 计算并绘制数据表记录的详情视图。
use super::*;

pub(super) fn detail_height(s: &State, record: usize, field: &BaseField) -> f32 {
    if matches!(field.field_type, FieldType::Reference | FieldType::Document) {
        let count = s.table().records[record]
            .values
            .get(&field.id)
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        34.0 + (count + usize::from(s.editing)).max(1) as f32 * table_reference_row_height()
    } else {
        72.0
    }
}

pub(super) fn paint_detail(list: &mut DrawList, s: &State, l: &Layout, p: &Palette) {
    let (Some(panel), Some(record_index)) = (l.detail, s.detail) else {
        return;
    };
    let Some(record) = s.table().records.get(record_index) else {
        return;
    };
    list.rounded_rect_alpha(
        Rect::new(
            panel.left - 6.0,
            panel.top + 5.0,
            panel.right + 6.0,
            panel.bottom + 10.0,
        ),
        16.0,
        0x000000,
        0.12,
    );
    list.rounded_rect(panel, 14.0, p.surface_elevated);
    list.rounded_border(panel, 14.0, p.border);
    let title = s
        .table()
        .fields
        .first()
        .map(|field| format_cell_value(field, record.values.get(&field.id).unwrap_or(&Value::Null)))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "记录详情".into());
    list.text(
        Rect::new(
            panel.left + 18.0,
            panel.top + 10.0,
            panel.right - 46.0,
            panel.top + 42.0,
        ),
        text::ellipsize(&title, TextStyle::Large, panel.width() - 66.0),
        TextStyle::Large,
        p.foreground,
    );
    for (rect, hit) in &l.entries {
        if rect.left < panel.left
            || rect.right > panel.right
            || rect.top < panel.top
            || rect.bottom > panel.bottom
        {
            continue;
        }
        let label = match hit {
            Hit::CloseDetail => "×",
            Hit::CopyRecord(_) => "复制记录链接",
            Hit::RecordMenu(_) => "···",
            Hit::ToggleEditing => {
                if s.editing {
                    "完成编辑"
                } else {
                    "编辑记录"
                }
            }
            _ => continue,
        };
        if s.hover == Some(*hit) {
            list.rounded_rect(*rect, 6.0, p.surface_muted);
        }
        if matches!(hit, Hit::CopyRecord(_)) {
            list.rounded_border(*rect, 6.0, p.border);
        }
        if *hit == Hit::ToggleEditing {
            list.rounded_rect(*rect, 6.0, theme::mix(p.accent, p.background, 0.1));
        }
        list.text(
            Rect::new(rect.left + 8.0, rect.top, rect.right - 6.0, rect.bottom),
            label,
            TextStyle::Caption,
            if *hit == Hit::ToggleEditing {
                p.accent
            } else {
                p.muted
            },
        );
    }
    list.hline(
        panel.left + 16.0,
        panel.right - 16.0,
        panel.top + 90.0,
        p.border,
    );
    let content = Rect::new(
        panel.left + 16.0,
        panel.top + 98.0,
        panel.right - 16.0,
        panel.bottom - 12.0,
    );
    list.push_clip(content);
    let mut y = content.top - s.detail_scroll.min(l.detail_max);
    for (field_index, field) in s.table().fields.iter().enumerate() {
        let height = detail_height(s, record_index, field);
        let value = record.values.get(&field.id).unwrap_or(&Value::Null);
        list.text(
            Rect::new(content.left, y, content.right, y + 22.0),
            &field.name,
            TextStyle::Tiny,
            p.muted,
        );
        if matches!(field.field_type, FieldType::Reference | FieldType::Document) {
            let row_height = table_reference_row_height();
            let values = value.as_array();
            for (index, url) in values
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .enumerate()
            {
                let top = y + 22.0 + index as f32 * row_height;
                let rect = Rect::new(
                    content.left,
                    top,
                    content.right - if s.editing { 34.0 } else { 0.0 },
                    top + row_height - 4.0,
                );
                list.rounded_rect(
                    rect,
                    6.0,
                    theme::mix(
                        p.accent,
                        p.background,
                        if s.hover == Some(Hit::OpenReference(record_index, field_index, index)) {
                            0.12
                        } else {
                            0.06
                        },
                    ),
                );
                list.text(
                    Rect::new(rect.left + 9.0, rect.top, rect.right - 8.0, rect.bottom),
                    text::ellipsize(
                        &format!("↗ {}", format_base_reference(url)),
                        TextStyle::Table,
                        rect.width() - 17.0,
                    ),
                    TextStyle::Table,
                    p.accent,
                );
                if s.editing {
                    list.text(
                        Rect::new(content.right - 26.0, top, content.right, top + 30.0),
                        "×",
                        TextStyle::Caption,
                        p.muted,
                    );
                }
            }
            let count = values.map_or(0, Vec::len);
            if s.editing {
                let top = y + 22.0 + count as f32 * row_height;
                list.text(
                    Rect::new(
                        content.left + 8.0,
                        top,
                        content.right,
                        top + row_height - 4.0,
                    ),
                    "＋ 添加笔记、资料或日程",
                    TextStyle::Table,
                    p.accent,
                );
            } else if count == 0 {
                list.text(
                    Rect::new(content.left + 9.0, y + 22.0, content.right, y + 52.0),
                    "未关联",
                    TextStyle::Caption,
                    p.muted,
                );
            }
        } else {
            let rect = Rect::new(content.left, y + 22.0, content.right, y + height - 12.0);
            list.rounded_rect(rect, 6.0, p.surface_muted);
            if s.selected == Some((record_index, field_index)) {
                list.rounded_border(rect, 6.0, p.accent);
            }
            cell(list, rect, field, value, p);
        }
        y += height;
    }
    list.pop_clip();
    if l.detail_max > 0.0 {
        let track = Rect::new(
            panel.right - 9.0,
            content.top,
            panel.right - 5.0,
            content.bottom,
        );
        let height =
            (track.height() * content.height() / (content.height() + l.detail_max)).max(26.0);
        let top = track.top
            + (track.height() - height) * s.detail_scroll.clamp(0.0, l.detail_max) / l.detail_max;
        list.rounded_rect(
            Rect::new(track.left, top, track.right, top + height),
            2.0,
            theme::mix(p.muted, p.background, 0.4),
        );
    }
}
