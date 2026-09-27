//! 绘制数据表网格、单元格、弹窗和菜单。
use super::*;

pub(super) fn option_color(color: OptionColor) -> u32 {
    match color {
        OptionColor::Gray => 0x64748b,
        OptionColor::Red => 0xef4444,
        OptionColor::Orange => 0xf97316,
        OptionColor::Yellow => 0xca8a04,
        OptionColor::Green => 0x16a34a,
        OptionColor::Blue => 0x3b82f6,
        OptionColor::Purple => 0xa855f7,
        OptionColor::Pink => 0xec4899,
    }
}

fn pill(list: &mut DrawList, r: Rect, label: &str, color: OptionColor, p: &Palette) -> f32 {
    let w = (text::measure(label, TextStyle::Table) + 18.0)
        .min(r.width())
        .max(0.0);
    let color = option_color(color);
    let vertical = ((r.height() - TextStyle::Table.line_height()).max(0.0) * 0.4).max(0.0);
    let rect = Rect::new(r.left, r.top + vertical, r.left + w, r.bottom - vertical);
    list.rounded_rect(rect, 6.0, theme::mix(color, p.background, 0.18));
    list.text(
        Rect::new(rect.left + 8.0, rect.top, rect.right - 6.0, rect.bottom),
        text::ellipsize(label, TextStyle::Table, (w - 14.0).max(0.0)),
        TextStyle::Table,
        theme::mix(color, p.foreground, 0.78),
    );
    w
}

pub(super) fn cell(list: &mut DrawList, r: Rect, field: &BaseField, value: &Value, p: &Palette) {
    list.push_clip(r);
    let padding = table_cell_padding().min(r.width().max(0.0) / 2.0);
    let inner = Rect::new(r.left + padding, r.top, r.right - padding, r.bottom);
    match field.field_type {
        FieldType::Reference | FieldType::Document => {
            let urls = value.as_array().map(Vec::as_slice).unwrap_or_default();
            let mut x = inner.left;
            for url in urls.iter().filter_map(Value::as_str) {
                if x >= inner.right {
                    break;
                }
                x += pill(
                    list,
                    Rect::new(x, r.top, inner.right, r.bottom),
                    &format!("↗ {}", format_base_reference(url)),
                    OptionColor::Blue,
                    p,
                ) + 5.0;
            }
        }
        FieldType::SingleSelect | FieldType::MultiSelect => {
            let mut x = inner.left;
            for option in &field.options {
                if value.as_str() == Some(&option.id)
                    || value
                        .as_array()
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some(&option.id)))
                {
                    if x >= inner.right {
                        break;
                    }
                    x += pill(
                        list,
                        Rect::new(x, r.top, inner.right, r.bottom),
                        &option.label,
                        option.color,
                        p,
                    ) + 5.0;
                }
            }
        }
        FieldType::Checkbox => {
            let size = TextStyle::Table.font_size().clamp(16.0, 24.0);
            let top = r.top + (r.height() - size).max(0.0) / 2.0;
            let rect = Rect::new(inner.left, top, inner.left + size, top + size);
            list.rounded_border(rect, 4.0, p.border);
            if value.as_bool() == Some(true) {
                list.rounded_rect(rect, 4.0, p.accent);
                list.text(rect, "✓", TextStyle::Table, p.accent_foreground);
            }
        }
        FieldType::Progress => {
            if let Some(n) = value.as_f64() {
                // 数值放在一个小输入框里，与可拖动的轨道分开。
                // 绘制、命中和拖动共用 progress_regions 的位置。
                let (bar, input) = super::progress_regions(r);
                list.rounded_rect(bar, 4.0, p.surface_muted);
                list.rounded_rect(
                    Rect::new(
                        bar.left,
                        bar.top,
                        bar.left + bar.width() * (n / 100.0) as f32,
                        bar.bottom,
                    ),
                    4.0,
                    p.accent,
                );
                list.rounded_rect(input, 4.0, p.surface_elevated);
                list.rounded_border(input, 4.0, p.border);
                list.text(input, format!("{n:.0}%"), TextStyle::Table, p.muted);
            }
        }
        _ => {
            list.text(
                inner,
                text::ellipsize(
                    &format_cell_value(field, value),
                    TextStyle::Table,
                    inner.width().max(0.0),
                ),
                TextStyle::Table,
                if field.field_type == FieldType::Url {
                    p.accent
                } else {
                    p.foreground
                },
            );
        }
    }
    list.pop_clip();
}

pub fn paint(list: &mut DrawList, area: Rect, s: &State, l: &Layout, p: &Palette) {
    list.push_clip(area);
    // 两个编辑器共用宿主绘制的文档背景/卡片配置。
    // 这里若整面填充不透明底色，会盖掉宿主设置的不透明度。
    list.hline(area.left, area.right, area.top + 44.0, p.border);
    list.hline(area.left, area.right, area.top + 85.0, p.border);
    if l.body.top > area.top + 85.5 {
        list.hline(area.left, area.right, l.body.top, p.border);
    }
    for (r, hit) in &l.entries {
        if *hit == Hit::ToggleEditing
            && l.detail.is_some_and(|panel| {
                r.left >= panel.left
                    && r.right <= panel.right
                    && r.top >= panel.top
                    && r.bottom <= panel.bottom
            })
        {
            continue;
        }
        let label = match *hit {
            Hit::TablesMenu => Some("数据表 ▾".into()),
            Hit::ViewsMenu => Some("视图 ▾".into()),
            Hit::Table(i) => Some(s.document.tables[i].name.clone()),
            Hit::View(i) => Some(format!(
                "{} {}",
                if s.table().views[i].view_type == ViewType::Grid {
                    "▦"
                } else {
                    "▥"
                },
                s.table().views[i].name
            )),
            Hit::NewTable | Hit::NewView => Some("＋".into()),
            Hit::NewRecord => Some("＋ 记录".into()),
            Hit::NewField => Some("＋ 字段".into()),
            Hit::NewTemporaryDocument => Some("＋ 临时文档".into()),
            Hit::Filter => Some("筛选".into()),
            Hit::Columns => Some("显示列".into()),
            Hit::Freeze => Some("冻结".into()),
            Hit::ExportXlsx => Some("导出".into()),
            Hit::Automations => Some("自动化".into()),
            Hit::RowHeight => Some("行高".into()),
            Hit::Sort => Some("排序".into()),
            Hit::Group => Some("分组".into()),
            Hit::ToggleEditing => Some(if s.editing { "查看" } else { "编辑" }.into()),
            Hit::ClearLocation => Some("返回视图".into()),
            _ => None,
        };
        if let Some(label) = label {
            let active = matches!(hit,Hit::Table(i)if *i==s.table_index())
                || matches!(hit,Hit::View(i)if *i==s.view_index())
                || matches!(hit,Hit::Filter if !s.view().filters.is_empty())
                || matches!(hit,Hit::Columns if !s.view().hidden_field_ids.is_empty())
                || matches!(hit,Hit::Freeze if s.view().frozen_rows > 0 || s.view().frozen_columns > 0)
                || matches!(hit,Hit::Sort if !s.view().sorts.is_empty());
            let primary = *hit == Hit::NewRecord;
            if *hit == Hit::ToggleEditing {
                list.rounded_rect(
                    *r,
                    6.0,
                    if s.editing {
                        p.surface_elevated
                    } else {
                        p.background
                    },
                );
                list.rounded_border(*r, 6.0, p.border);
            } else if primary {
                list.rounded_rect(*r, 6.0, theme::mix(p.accent, p.background, 0.09));
            } else if active || s.hover == Some(*hit) {
                list.rounded_rect(*r, 6.0, p.surface_muted);
            }
            list.text(
                Rect::new(r.left + 8.0, r.top, r.right - 5.0, r.bottom),
                label,
                TextStyle::Caption,
                if active || primary || *hit == Hit::ToggleEditing && s.editing {
                    p.accent
                } else {
                    p.foreground
                },
            );
        }
    }
    list.push_clip(l.body);
    let t = s.table();
    if let Some(grid) = &l.grid {
        grid::paint(list, s, l, grid, p);
    } else {
        list.rect(l.body, p.surface_muted);
        let groups = s.groups();
        let fields = s.visible_fields();
        let h = 38.0 + fields.len() as f32 * 34.0;
        for (g, (option, rows)) in groups.iter().enumerate() {
            let left = l.body.left + 12.0 + g as f32 * 280.0 - s.scroll_x.min(l.max_x);
            if left + 268.0 < l.body.left || left > l.body.right {
                continue;
            }
            list.push_clip(Rect::new(
                left,
                l.body.top + 42.0,
                left + 268.0,
                l.body.bottom,
            ));
            for (pos, &r) in rows.iter().enumerate() {
                let y = l.body.top + 48.0 + pos as f32 * (h + 12.0) - s.scroll_y.min(l.max_y);
                if y + h < l.body.top + 42.0 {
                    continue;
                }
                if y > l.body.bottom {
                    break;
                }
                let card = Rect::new(left, y, left + 268.0, y + h);
                let radius = board_card_radius();
                list.rounded_rect(card, radius, p.background);
                list.rounded_border(card, radius, p.border);
                list.text(
                    Rect::new(left + 10.0, y, left + 255.0, y + 32.0),
                    "记录 ···",
                    TextStyle::Caption,
                    p.muted,
                );
                for (position, &f) in fields.iter().enumerate() {
                    let field = &t.fields[f];
                    let cy = y + 34.0 + position as f32 * 34.0;
                    list.text(
                        Rect::new(left + 10.0, cy, left + 90.0, cy + 34.0),
                        text::ellipsize(&field.name, TextStyle::Tiny, 78.0),
                        TextStyle::Tiny,
                        p.muted,
                    );
                    cell(
                        list,
                        Rect::new(left + 91.0, cy, left + 260.0, cy + 34.0),
                        field,
                        t.records[r].values.get(&field.id).unwrap_or(&Value::Null),
                        p,
                    );
                }
            }
            list.pop_clip();
            let head = Rect::new(left, l.body.top, left + 268.0, l.body.top + 40.0);
            list.rect(head, p.surface_muted);
            if let Some(o) = option {
                pill(
                    list,
                    Rect::new(left + 4.0, head.top, left + 218.0, head.bottom),
                    &o.label,
                    o.color,
                    p,
                );
            } else {
                list.text(
                    Rect::new(left + 8.0, head.top, left + 210.0, head.bottom),
                    "未分组",
                    TextStyle::Caption,
                    p.muted,
                );
            }
            list.text(
                Rect::new(left + 231.0, head.top, left + 264.0, head.bottom),
                rows.len().to_string(),
                TextStyle::Caption,
                p.muted,
            );
        }
        if groups.is_empty() {
            list.text(
                Rect::new(
                    l.body.left + 24.0,
                    l.body.top + 24.0,
                    l.body.right - 24.0,
                    l.body.top + 70.0,
                ),
                "添加下拉菜单字段并通过「分组」选择，即可按彩色标签展示看板",
                TextStyle::Label,
                p.muted,
            );
        }
    }
    list.pop_clip();
    s.scrollbars.paint(list, &l.scrollbars, p);
    if !s.error.is_empty() {
        list.hline(area.left, area.right, area.bottom - 32.0, p.border);
        list.text(
            Rect::new(
                area.left + 12.0,
                area.bottom - 32.0,
                area.right - 12.0,
                area.bottom,
            ),
            &s.error,
            TextStyle::Tiny,
            p.danger,
        );
    }
    if s.popup.is_none() && s.detail.is_none() {
        if let Some(Hit::Cell(r, f)) = s.hover {
            if let Some(cell_rect) = l.rect_of(Hit::Cell(r, f)) {
                if let (Some(record), Some(field)) =
                    (s.table().records.get(r), s.table().fields.get(f))
                {
                    let value = record.values.get(&field.id).unwrap_or(&Value::Null);
                    let label = format_cell_value(field, value);
                    if !label.is_empty() {
                        let max_wrap_width = 400.0;
                        let table_line_height = TextStyle::Table.line_height();
                        let mut lines = Vec::new();
                        for hard_line in label.split('\n') {
                            let hard_line = hard_line.strip_suffix('\r').unwrap_or(hard_line);
                            for runs in text::wrap_source(
                                hard_line,
                                TextStyle::Table,
                                max_wrap_width - 24.0,
                            ) {
                                let line_text =
                                    runs.iter().map(|run| run.text.as_str()).collect::<String>();
                                lines.push(line_text);
                            }
                        }
                        if lines.is_empty() {
                            lines.push(String::new());
                        }

                        let max_lines = 20;
                        let truncated = lines.len() > max_lines;
                        if truncated {
                            lines.truncate(max_lines);
                            if let Some(last) = lines.last_mut() {
                                last.push_str("...");
                            }
                        }

                        let text_width = lines
                            .iter()
                            .map(|l| text::measure(l, TextStyle::Table))
                            .fold(0.0, f32::max)
                            + 24.0;
                        let text_width = text_width.clamp(60.0, max_wrap_width);
                        let text_height = (lines.len() as f32 * table_line_height + 8.0).max(28.0);

                        let left = (cell_rect.left + 16.0).clamp(
                            area.left + 4.0,
                            (area.right - text_width - 4.0).max(area.left + 4.0),
                        );
                        let top = if cell_rect.bottom + text_height + 4.0 <= area.bottom {
                            cell_rect.bottom + 4.0
                        } else if cell_rect.top - text_height - 4.0 >= area.top {
                            cell_rect.top - text_height - 4.0
                        } else {
                            area.top + 4.0
                        };

                        let tooltip_rect =
                            Rect::new(left, top, left + text_width, top + text_height);
                        list.rounded_rect(tooltip_rect, 6.0, p.surface_elevated);
                        list.rounded_border(tooltip_rect, 6.0, p.border);

                        let mut line_y = tooltip_rect.top + 4.0;
                        for line in lines {
                            list.text(
                                Rect::new(
                                    tooltip_rect.left + 12.0,
                                    line_y,
                                    tooltip_rect.right - 12.0,
                                    line_y + table_line_height,
                                ),
                                &line,
                                TextStyle::Table,
                                p.foreground,
                            );
                            line_y += table_line_height;
                        }
                    }
                }
            }
        }
    }
    if s.detail.is_none() {
        paint_popup(list, s, l, p);
    }
    list.pop_clip();
}

pub fn paint_modal(list: &mut DrawList, viewport: Rect, s: &State, p: &Palette) {
    if s.detail.is_none() {
        return;
    }
    let l = layout(s, viewport);
    list.rect_alpha(viewport, 0x000000, 0.28);
    paint_detail(list, s, &l, p);
    paint_popup(list, s, &l, p);
}

fn paint_popup(list: &mut DrawList, s: &State, l: &Layout, p: &Palette) {
    if matches!(s.popup, Some(Popup::DateTime(..))) {
        let (Some(menu), Some(picker)) = (l.menu, s.date_time_picker) else {
            return;
        };
        list.rounded_rect(menu, 10.0, p.surface_elevated);
        list.rounded_border(menu, 10.0, p.border);
        list.text(
            Rect::new(
                menu.left + 14.0,
                menu.top + 10.0,
                menu.right - 14.0,
                menu.top + 30.0,
            ),
            "选择时间点",
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(
                menu.left + 14.0,
                menu.top + 32.0,
                menu.right - 14.0,
                menu.top + 52.0,
            ),
            format!("{} · 默认当前时刻", picker.display()),
            TextStyle::Caption,
            p.muted,
        );
        for (part, label, label_rect, minus, value, plus) in date_time_picker_controls(menu) {
            list.text(label_rect, label, TextStyle::Caption, p.muted);
            list.rounded_rect(value, 4.0, p.surface);
            list.rounded_border(value, 4.0, p.border);
            list.text(
                value,
                date_time_part_value(picker, part),
                TextStyle::Caption,
                p.foreground,
            );
            for (rect, text_value, hit) in [
                (minus, "−", Hit::DateTimeAdjust(part, -1)),
                (plus, "+", Hit::DateTimeAdjust(part, 1)),
            ] {
                if s.hover == Some(hit) {
                    list.rounded_rect(rect, 4.0, p.surface_muted);
                }
                list.rounded_border(rect, 4.0, p.border);
                list.text(rect, text_value, TextStyle::Caption, p.foreground);
            }
        }
        let confirm = Rect::new(
            menu.left + 14.0,
            menu.bottom - 38.0,
            menu.right - 88.0,
            menu.bottom - 10.0,
        );
        let clear = Rect::new(
            menu.right - 80.0,
            menu.bottom - 38.0,
            menu.right - 14.0,
            menu.bottom - 10.0,
        );
        list.glass_button(confirm, 6.0, p, s.hover == Some(Hit::DateTimeConfirm));
        list.text(confirm, "确定", TextStyle::Caption, p.button_foreground());
        if s.hover == Some(Hit::DateTimeClear) {
            list.rounded_rect(clear, 6.0, p.surface_muted);
        }
        list.rounded_border(clear, 6.0, p.border);
        list.text(clear, "清除", TextStyle::Caption, p.muted);
        return;
    }
    if let Some(menu) = l.menu {
        list.rounded_rect(menu, 8.0, p.surface_elevated);
        list.rounded_border(menu, 8.0, p.border);
        list.push_clip(Rect::new(
            menu.left,
            menu.top + 6.0,
            menu.right,
            menu.bottom - 6.0,
        ));
        for (i, item) in s.menu_items().iter().enumerate() {
            let y = menu.top + 6.0 + i as f32 * 34.0 - s.popup_scroll.min(l.menu_max);
            if y + 34.0 < menu.top || y > menu.bottom {
                continue;
            }
            let rect = Rect::new(menu.left + 8.0, y, menu.right - 8.0, y + 34.0);
            if item.checked || s.hover == Some(Hit::Menu(i)) {
                list.rounded_rect(rect, 4.0, p.surface_muted);
            }
            let label = if matches!(item.choice, Choice::ToggleColumn(_)) {
                format!("{} {}", if item.checked { "☑" } else { "☐" }, item.label)
            } else if item.checked {
                format!("✓ {}", item.label)
            } else {
                item.label.clone()
            };
            if let Some(color) = item.color {
                pill(
                    list,
                    Rect::new(rect.left + 4.0, rect.top, rect.right - 4.0, rect.bottom),
                    &label,
                    color,
                    p,
                );
            } else {
                list.text(
                    Rect::new(rect.left + 8.0, rect.top, rect.right - 8.0, rect.bottom),
                    text::ellipsize(&label, TextStyle::Caption, rect.width() - 16.0),
                    TextStyle::Caption,
                    p.foreground,
                );
            }
        }
        list.pop_clip();
    }
}
