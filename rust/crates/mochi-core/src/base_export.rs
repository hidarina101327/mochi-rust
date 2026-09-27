//! 将数据表视图导出为 XLSX 文件。
use crate::base::*;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use rust_xlsxwriter::{Format, Workbook, Worksheet};
use serde_json::Value;

pub fn export_view_xlsx(
    table: &BaseTable,
    view: &BaseView,
    rows: &[usize],
    fields: &[usize],
) -> Result<Vec<u8>> {
    if fields.is_empty() {
        bail!("请至少显示一列后再导出")
    }
    if fields.len() > 16_384 || rows.len() > 1_048_575 {
        bail!("数据超出 XLSX 的行列上限，请先筛选后导出")
    }
    if fields.iter().any(|&i| i >= table.fields.len())
        || rows.iter().any(|&i| i >= table.records.len())
    {
        bail!("导出视图包含失效的行列引用")
    }
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    let title: String = view
        .name
        .chars()
        .map(|c| {
            if "[]:*?/\\".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .take(31)
        .collect();
    let title = title.trim_matches('\'');
    sheet.set_name(
        if title.is_empty() || title.eq_ignore_ascii_case("History") {
            "数据表"
        } else {
            title
        },
    )?;
    let header = Format::new()
        .set_bold()
        .set_background_color(0xEEF1F4)
        .set_font_color(0x202832);
    let number = Format::new().set_num_format("0.###############");
    let progress = Format::new().set_num_format("0.0%");
    let date = Format::new().set_num_format("yyyy-mm-dd");
    let datetime = Format::new().set_num_format("yyyy-mm-dd hh:mm");
    sheet.set_row_height(0, 26)?;
    sheet.set_freeze_panes(
        (1 + view.frozen_rows.min(rows.len() as u32)).min(1_048_575),
        view.frozen_columns.min(fields.len() as u32).min(16_383) as u16,
    )?;
    sheet.set_default_row_height(view.row_height.unwrap_or(38.0).clamp(28.0, 96.0) * 0.75);
    for (column, &field_index) in fields.iter().enumerate() {
        let field = &table.fields[field_index];
        let column = column as u16;
        sheet.set_column_width_pixels(
            column,
            field.width.unwrap_or(180.0).clamp(100.0, 600.0) as u32,
        )?;
        sheet.write_string_with_format(0, column, &field.name, &header)?;
        for (row, &record_index) in rows.iter().enumerate() {
            let value = table.records[record_index]
                .values
                .get(&field.id)
                .unwrap_or(&Value::Null);
            if value.is_null() {
                continue;
            }
            let row = row as u32 + 1;
            let context = || format!("第 {row} 条记录，字段「{}」", field.name);
            match field.field_type {
                FieldType::Number | FieldType::Progress if value.is_number() => {
                    let raw = value.to_string();
                    // Excel 只保留 15 位有效数字。
                    if (value.is_i64() || value.is_u64()) && raw.trim_start_matches('-').len() > 15
                    {
                        sheet
                            .write_string(row, column, &raw)
                            .with_context(context)?;
                    } else {
                        let n = value.as_f64().context("无效数字")?;
                        sheet
                            .write_number_with_format(
                                row,
                                column,
                                if field.field_type == FieldType::Progress {
                                    n / 100.0
                                } else {
                                    n
                                },
                                if field.field_type == FieldType::Progress {
                                    &progress
                                } else {
                                    &number
                                },
                            )
                            .with_context(context)?;
                    }
                }
                FieldType::Checkbox if value.is_boolean() => {
                    sheet
                        .write_boolean(row, column, value.as_bool().unwrap())
                        .with_context(context)?;
                }
                FieldType::Date if value.is_string() => {
                    if let Ok(parsed) =
                        NaiveDate::parse_from_str(value.as_str().unwrap(), "%Y-%m-%d")
                    {
                        sheet
                            .write_datetime_with_format(row, column, parsed, &date)
                            .with_context(context)?;
                    } else {
                        write_text(sheet, row, column, field, value).with_context(context)?;
                    }
                }
                FieldType::DateTime if value.is_string() => {
                    let text = value.as_str().unwrap();
                    let parsed = DateTime::parse_from_rfc3339(text)
                        .map(|d| d.with_timezone(&Local).naive_local())
                        .ok()
                        .or_else(|| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").ok())
                        .or_else(|| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M").ok());
                    if let Some(parsed) = parsed {
                        sheet
                            .write_datetime_with_format(row, column, parsed, &datetime)
                            .with_context(context)?;
                    } else {
                        write_text(sheet, row, column, field, value).with_context(context)?;
                    }
                }
                _ => {
                    write_text(sheet, row, column, field, value).with_context(context)?;
                }
            }
        }
    }
    if !rows.is_empty() {
        sheet.autofilter(0, 0, rows.len() as u32, fields.len() as u16 - 1)?;
    }
    Ok(book.save_to_buffer()?)
}

fn write_text(
    sheet: &mut Worksheet,
    row: u32,
    column: u16,
    field: &BaseField,
    value: &Value,
) -> Result<()> {
    let text = if matches!(field.field_type, FieldType::Reference | FieldType::Document) {
        value
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    } else {
        format_cell_value(field, value)
    };
    // 显式按字符串写出，避免源数据开头是 '=' 或 '+' 时被 Excel 当成公式。
    sheet.write_string(row, column, text)?;
    Ok(())
}

#[cfg(test)]
mod tests;
