//! 筛选数据表中可见字段、记录、分组和搜索结果。
use super::*;

impl State {
    pub fn visible_fields(&self) -> Vec<usize> {
        if self.located.is_some() {
            (0..self.table().fields.len()).collect()
        } else {
            visible_field_indices(self.table(), self.view())
        }
    }

    pub(super) fn visible_records(&self) -> Vec<usize> {
        let t = self.table();
        if self.located.is_some() {
            return (0..t.records.len()).collect();
        }
        let indices = t
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect::<std::collections::HashMap<_, _>>();
        get_view_records(t, self.view(), &self.query)
            .iter()
            .filter_map(|r| indices.get(r.id.as_str()).copied())
            .collect()
    }

    pub(super) fn groups(&self) -> Vec<(Option<&BaseOption>, Vec<usize>)> {
        let rows = self.visible_records();
        let Some(field) = self
            .view()
            .group_by
            .as_ref()
            .and_then(|id| self.table().fields.iter().find(|f| &f.id == id))
        else {
            return vec![];
        };
        let mut groups = field
            .options
            .iter()
            .map(|o| (Some(o), Vec::new()))
            .collect::<Vec<_>>();
        groups.push((None, Vec::new()));
        for r in rows {
            let value = self.table().records[r]
                .values
                .get(&field.id)
                .and_then(Value::as_str);
            let i = field
                .options
                .iter()
                .position(|o| Some(o.id.as_str()) == value)
                .unwrap_or(field.options.len());
            groups[i].1.push(r);
        }
        groups
    }

    pub fn grid(&self) -> bool {
        self.located.is_some() || self.view().view_type == ViewType::Grid
    }

    pub(super) fn action_row(&self) -> bool {
        self.editing || self.view().view_type == ViewType::Board || self.located.is_some()
    }

    pub fn search_cells(&self, query: &str) -> Vec<(usize, usize, usize)> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for (table_index, table) in self.document.tables.iter().enumerate() {
            for (record_index, record) in table.records.iter().enumerate() {
                let mut field_index = table.fields.iter().position(|field| {
                    format_cell_value(field, record.values.get(&field.id).unwrap_or(&Value::Null))
                        .to_lowercase()
                        .contains(&needle)
                });
                if field_index.is_none() && table.name.to_lowercase().contains(&needle) {
                    field_index = Some(0);
                }
                if let Some(field_index) = field_index {
                    hits.push((table_index, record_index, field_index));
                    if hits.len() >= 100 {
                        return hits;
                    }
                }
            }
        }
        hits
    }

    pub fn focus_cell(&mut self, table: usize, record: usize, field: usize, area: Rect) {
        let Some(target) = self.document.tables.get(table) else {
            return;
        };
        let field = field.min(target.fields.len().saturating_sub(1));
        let record = record.min(target.records.len().saturating_sub(1));
        let Some(row) = target.records.get(record) else {
            return;
        };
        let location = BaseLocation {
            table_id: target.id.clone(),
            record_id: Some(row.id.clone()),
            field_id: target.fields.get(field).map(|field| field.id.clone()),
        };
        if self.reveal(location).is_err() {
            return;
        }
        self.detail = None;
        let l = layout(self, area);
        if let Some(grid) = &l.grid {
            let frozen_width = grid.frozen_x - l.body.left - grid::GUTTER;
            let frozen_height = grid.frozen_y - l.body.top - grid::HEADER;
            let x: f32 = self.table().fields.iter().take(field).map(width).sum();
            self.scroll_x = (x - frozen_width).clamp(0.0, l.max_x);
            self.scroll_y =
                (record as f32 * grid_row_height(self) - frozen_height).clamp(0.0, l.max_y);
        }
    }

    /// 给单元格右键菜单用的人类可读、稳定的上下文。带上完整记录，
    /// 相邻字段往往能消歧一个含义模糊的短标签。
    pub fn cell_ai_context(&self, record: usize, field: usize) -> Option<AiCellContext> {
        let table = self.table();
        let record_data = table.records.get(record)?;
        let target = table.fields.get(field)?;
        let value = format_cell_value(
            target,
            record_data.values.get(&target.id).unwrap_or(&Value::Null),
        );
        let fields = table
            .fields
            .iter()
            .map(|field| {
                format!(
                    "{}：{}",
                    field.name,
                    format_cell_value(
                        field,
                        record_data.values.get(&field.id).unwrap_or(&Value::Null)
                    )
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        Some(AiCellContext {
            // 小片标题里也保留目标值：点击发送前，用户可以核对
            // 排队交给 AI 的到底是哪个单元格。
            title: format!("{} · {}：{}", table.name, target.name, value),
            content: format!(
                "多维表格「{}」中的单元格。字段：{}；值：{}。该记录：{}",
                table.name, target.name, value, fields,
            ),
        })
    }

    pub fn reveal(&mut self, location: BaseLocation) -> Result<(), String> {
        let table_index = self
            .document
            .tables
            .iter()
            .position(|table| table.id == location.table_id)
            .ok_or("引用的数据表已被删除")?;
        let table = &self.document.tables[table_index];
        let record = location
            .record_id
            .as_ref()
            .map(|id| {
                table
                    .records
                    .iter()
                    .position(|record| &record.id == id)
                    .ok_or("引用的记录已被删除")
            })
            .transpose()?;
        let field = location
            .field_id
            .as_ref()
            .map(|id| {
                table
                    .fields
                    .iter()
                    .position(|field| &field.id == id)
                    .ok_or("引用的字段已被删除")
            })
            .transpose()?;
        let scroll_x = table
            .fields
            .iter()
            .take(field.unwrap_or(0))
            .map(width)
            .sum();
        self.document.active_table_id = Some(table.id.clone());
        self.selected = record.map(|record| (record, field.unwrap_or(0)));
        self.scroll_x = scroll_x;
        self.scroll_y = record.unwrap_or(0) as f32 * 38.0;
        self.detail = record;
        self.detail_scroll = record
            .zip(field)
            .map(|(record, field)| {
                self.table()
                    .fields
                    .iter()
                    .take(field)
                    .map(|field| detail_height(self, record, field))
                    .sum()
            })
            .unwrap_or(0.0);
        self.popup = None;
        self.located = Some(location);
        Ok(())
    }
}
