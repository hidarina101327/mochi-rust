//! 生成数据表中的记录和字段菜单项。
use super::*;

impl State {
    pub(super) fn menu_items(&self) -> Vec<MenuItem> {
        let mut items = Vec::new();
        let t = self.table();
        match self.popup {
            Some(Popup::Table) => {
                items.push(MenuItem::new("＋ 新建数据表", Choice::Edit(Edit::NewTable)));
                items.push(MenuItem::new("重命名数据表", Choice::Edit(Edit::TableName)));
                if self.document.tables.len() > 1 {
                    items.push(MenuItem::new(
                        "删除数据表…",
                        Choice::Edit(Edit::DeleteTable),
                    ));
                }
                for (i, table) in self.document.tables.iter().enumerate() {
                    let mut item = MenuItem::new(&table.name, Choice::Table(i));
                    item.checked = i == self.table_index();
                    items.push(item);
                }
            }
            Some(Popup::View) => {
                items.push(MenuItem::new("显示列…", Choice::Popup(Popup::Columns)));
                if self.grid() {
                    items.push(MenuItem::new("冻结行列…", Choice::Popup(Popup::Freeze)));
                    items.push(MenuItem::new("行高…", Choice::Popup(Popup::RowHeight)));
                }
                items.push(MenuItem::new("导出当前视图为 XLSX…", Choice::ExportXlsx));
                items.push(MenuItem::new("＋ 新建视图", Choice::Popup(Popup::NewView)));
                items.push(MenuItem::new("重命名视图", Choice::Edit(Edit::ViewName)));
                if t.views.len() > 1 {
                    items.push(MenuItem::new("删除视图…", Choice::Edit(Edit::DeleteView)));
                }
                for (i, view) in t.views.iter().enumerate() {
                    let mut item = MenuItem::new(&view.name, Choice::View(i));
                    item.checked = i == self.view_index();
                    items.push(item);
                }
            }
            Some(Popup::NewField) => {
                for (kind, name) in TYPES {
                    items.push(MenuItem::new(name, Choice::Edit(Edit::NewField(kind))));
                }
            }
            Some(Popup::NewView) => {
                items.push(MenuItem::new(
                    "表格视图",
                    Choice::Edit(Edit::NewView(ViewType::Grid)),
                ));
                items.push(MenuItem::new(
                    "看板视图",
                    Choice::Edit(Edit::NewView(ViewType::Board)),
                ));
            }
            Some(Popup::Field(f)) => {
                items.push(MenuItem::new(
                    "重命名字段",
                    Choice::Edit(Edit::FieldName(f)),
                ));
                if matches!(
                    t.fields[f].field_type,
                    FieldType::SingleSelect | FieldType::MultiSelect
                ) {
                    items.push(MenuItem::new(
                        "管理选项与颜色",
                        Choice::Popup(Popup::Options(f)),
                    ));
                }
                for (kind, name) in TYPES {
                    let mut item =
                        MenuItem::new(format!("字段类型 · {name}"), Choice::SetType(f, kind));
                    item.checked = t.fields[f].field_type == kind;
                    items.push(item);
                }
                if t.fields.len() > 1 {
                    items.push(MenuItem::new(
                        "删除字段…",
                        Choice::Edit(Edit::DeleteField(f)),
                    ));
                }
            }
            Some(Popup::Options(f)) => {
                items.push(MenuItem::new(
                    "＋ 添加选项",
                    Choice::Edit(Edit::NewOption(f)),
                ));
                for (o, option) in t.fields[f].options.iter().enumerate() {
                    let mut item = MenuItem::new(&option.label, Choice::Popup(Popup::Option(f, o)));
                    item.color = Some(option.color);
                    items.push(item);
                }
            }
            Some(Popup::Option(f, o)) => {
                items.push(MenuItem::new(
                    "修改标签",
                    Choice::Edit(Edit::OptionName(f, o)),
                ));
                items.push(MenuItem::new(
                    "设置气泡颜色",
                    Choice::Popup(Popup::Color(f, o)),
                ));
                items.push(MenuItem::new(
                    "删除选项…",
                    Choice::Edit(Edit::DeleteOption(f, o)),
                ));
            }
            Some(Popup::Color(f, o)) => {
                for (color, name) in COLORS {
                    let mut item = MenuItem::new(name, Choice::SetColor(f, o, color));
                    item.color = Some(color);
                    item.checked = t.fields[f].options[o].color == color;
                    items.push(item);
                }
            }
            Some(Popup::Select(r, f)) => {
                let field = &t.fields[f];
                let value = t.records[r].values.get(&field.id).unwrap_or(&Value::Null);
                items.push(MenuItem::new("清除内容", Choice::ClearCell(r, f)));
                for (o, option) in field.options.iter().enumerate() {
                    let mut item = MenuItem::new(&option.label, Choice::ToggleOption(r, f, o));
                    item.color = Some(option.color);
                    item.checked = value.as_str() == Some(&option.id)
                        || value
                            .as_array()
                            .is_some_and(|v| v.iter().any(|v| v.as_str() == Some(&option.id)));
                    items.push(item);
                }
                items.push(MenuItem::new(
                    "＋ 添加选项",
                    Choice::Edit(Edit::NewOption(f)),
                ));
                items.push(MenuItem::new(
                    "管理选项与颜色",
                    Choice::Popup(Popup::Options(f)),
                ));
            }
            Some(Popup::DateTime(..)) => {}
            Some(Popup::Record(r)) => {
                items.push(MenuItem::new("查看记录详情", Choice::ShowRecord(r)));
                items.push(MenuItem::new("复制记录链接", Choice::CopyRecord(r)));
                items.push(MenuItem::new("复制记录", Choice::DuplicateRecord(r)));
                items.push(MenuItem::new(
                    "删除记录…",
                    Choice::Edit(Edit::DeleteRecord(r)),
                ));
            }
            Some(Popup::Reference(r, f)) => {
                items.push(MenuItem::new(
                    if self.reference_query.is_empty() {
                        "查找笔记、资料或日程…".into()
                    } else {
                        format!("查找 · {}", self.reference_query)
                    },
                    Choice::Edit(Edit::ReferenceSearch(r, f)),
                ));
                items.push(MenuItem::new(
                    "粘贴 Mochi 链接…",
                    Choice::Edit(Edit::ReferenceLink(r, f)),
                ));
                let values = t
                    .records
                    .get(r)
                    .and_then(|record| record.values.get(&t.fields[f].id))
                    .and_then(Value::as_array);
                let query = self.reference_query.to_lowercase();
                for (index, candidate) in self.reference_candidates.iter().enumerate() {
                    if !query.is_empty() && !candidate.label.to_lowercase().contains(&query) {
                        continue;
                    }
                    let mut item =
                        MenuItem::new(&candidate.label, Choice::ToggleReference(r, f, index));
                    item.checked = values.is_some_and(|urls| {
                        urls.iter().any(|url| url.as_str() == Some(&candidate.url))
                    });
                    items.push(item);
                }
            }
            Some(Popup::Sort) => {
                items.push(MenuItem::new("清除排序", Choice::ClearSort));
                for (f, field) in t.fields.iter().enumerate() {
                    items.push(MenuItem::new(
                        format!("{} · 升序", field.name),
                        Choice::Sort(f, SortDirection::Asc),
                    ));
                    items.push(MenuItem::new(
                        format!("{} · 降序", field.name),
                        Choice::Sort(f, SortDirection::Desc),
                    ));
                }
            }
            Some(Popup::RowHeight) => {
                let current = grid_row_height(self);
                for (height, label) in GRID_ROW_HEIGHTS {
                    let mut item = MenuItem::new(
                        format!("{label} · {} px", height as u32),
                        Choice::SetRowHeight(height),
                    );
                    item.checked = (current - height).abs() < 0.5;
                    items.push(item);
                }
            }
            Some(Popup::Columns) => {
                items.push(MenuItem::new("显示全部列", Choice::ShowAllColumns));
                for (f, field) in t.fields.iter().enumerate() {
                    let mut item = MenuItem::new(&field.name, Choice::ToggleColumn(f));
                    item.checked = !self.view().hidden_field_ids.contains(&field.id);
                    items.push(item);
                }
            }
            Some(Popup::Freeze) => {
                items.push(MenuItem::new(
                    format!("冻结前 {} 列…", self.view().frozen_columns),
                    Choice::Edit(Edit::FrozenColumns),
                ));
                items.push(MenuItem::new(
                    format!("冻结前 {} 行…", self.view().frozen_rows),
                    Choice::Edit(Edit::FrozenRows),
                ));
                items.push(MenuItem::new("取消全部冻结", Choice::ClearFreeze));
            }
            Some(Popup::Filter) => {
                items.push(MenuItem::new("清除全部筛选", Choice::ClearFilter));
                for (f, field) in t.fields.iter().enumerate() {
                    items.push(MenuItem::new(
                        &field.name,
                        Choice::Popup(Popup::FilterOperator(f)),
                    ));
                }
            }
            Some(Popup::FilterOperator(f)) => {
                for (operator, name) in OPERATORS {
                    items.push(MenuItem::new(
                        name,
                        if matches!(
                            operator,
                            FilterOperator::IsEmpty | FilterOperator::IsNotEmpty
                        ) {
                            Choice::EmptyFilter(f, operator)
                        } else {
                            Choice::Edit(Edit::Filter(f, operator))
                        },
                    ));
                }
            }
            Some(Popup::Group) => {
                for (f, field) in t.fields.iter().enumerate() {
                    if field.field_type == FieldType::SingleSelect {
                        let mut item = MenuItem::new(&field.name, Choice::Group(f));
                        item.checked = self.view().group_by.as_deref() == Some(&field.id);
                        items.push(item);
                    }
                }
            }
            None => {}
        }
        if !self.editing {
            items.retain(|item| {
                matches!(
                    item.choice,
                    Choice::Table(_)
                        | Choice::ToggleColumn(_)
                        | Choice::ShowAllColumns
                        | Choice::ClearFreeze
                        | Choice::ExportXlsx
                        | Choice::Popup(Popup::Columns | Popup::Freeze | Popup::RowHeight)
                        | Choice::Edit(Edit::FrozenRows | Edit::FrozenColumns)
                        | Choice::SetRowHeight(_)
                        | Choice::View(_)
                        | Choice::Sort(..)
                        | Choice::ClearSort
                        | Choice::EmptyFilter(..)
                        | Choice::ClearFilter
                        | Choice::Group(_)
                        | Choice::Popup(Popup::FilterOperator(_))
                        | Choice::Edit(Edit::Filter(..))
                        | Choice::ShowRecord(_)
                        | Choice::CopyRecord(_)
                )
            });
        }
        items
    }
}
