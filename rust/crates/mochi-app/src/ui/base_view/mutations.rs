//! 提交并应用数据表记录、字段和视图变更。
use super::*;

impl State {
    pub(super) fn prompt(&self, edit: Edit) -> Prompt {
        let t = self.table();
        let (title, description, value) = match edit {
            Edit::FrozenRows | Edit::FrozenColumns => {
                let rows = edit == Edit::FrozenRows;
                (
                    if rows { "冻结前 N 行" } else { "冻结前 N 列" }.into(),
                    if rows {
                        "输入非负整数，0 取消。表头始终固定，N 指排序/筛选后的前 N 条记录。窗口较小时保留一行滚动空间，放大后恢复。"
                    } else {
                        "输入非负整数，0 取消。按当前显示列从左计数。窗口较小时保留滚动空间，放大后恢复。"
                    }.into(),
                    Some(if rows { self.view().frozen_rows } else { self.view().frozen_columns }.to_string()),
                )
            }
            Edit::NewTable => (
                "新建数据表".into(),
                "输入数据表名称".into(),
                Some(String::new()),
            ),
            Edit::TableName => ("重命名数据表".into(), String::new(), Some(t.name.clone())),
            Edit::NewField(kind) => (
                format!("添加{}字段", type_name(kind)),
                "输入字段名称".into(),
                Some(String::new()),
            ),
            Edit::FieldName(f) => (
                "重命名字段".into(),
                String::new(),
                Some(t.fields[f].name.clone()),
            ),
            Edit::FieldType(f, kind) => (
                "更改字段类型".into(),
                format!("将「{}」改为「{}」", t.fields[f].name, type_name(kind)),
                None,
            ),
            Edit::NewOption(_) => (
                "添加选项".into(),
                "输入标签内容，之后可以设置颜色".into(),
                Some(String::new()),
            ),
            Edit::OptionName(f, o) => (
                "修改选项标签".into(),
                String::new(),
                Some(t.fields[f].options[o].label.clone()),
            ),
            Edit::NewView(kind) => (
                if kind == ViewType::Grid {
                    "新建表格视图"
                } else {
                    "新建看板视图"
                }
                .into(),
                "输入视图名称".into(),
                Some(String::new()),
            ),
            Edit::ViewName => (
                "重命名视图".into(),
                String::new(),
                Some(self.view().name.clone()),
            ),
            Edit::Cell(r, f) => {
                let field = &t.fields[f];
                let value = t.records[r].values.get(&field.id).unwrap_or(&Value::Null);
                let hint = match field.field_type {
                    FieldType::Number => "请输入数字，留空清除",
                    FieldType::Date => "日期格式 YYYY-MM-DD，留空清除",
                    FieldType::DateTime => "格式：YYYY-MM-DDTHH:mm（留空清除）",
                    FieldType::DateRange => "格式：YYYY-MM-DDTHH:mm 至 YYYY-MM-DDTHH:mm",
                    FieldType::Progress => "请输入 0 到 100 的进度，留空清除",
                    FieldType::Url => "请输入完整链接，留空清除",
                    _ => "输入内容，留空清除",
                };
                (
                    field.name.clone(),
                    hint.into(),
                    Some(if value.is_null() {
                        String::new()
                    } else if field.field_type == FieldType::DateRange {
                        format_cell_value(field, value)
                    } else if let Some(s) = value.as_str() {
                        s.into()
                    } else {
                        value.to_string()
                    }),
                )
            }
            Edit::Search => (
                "搜索记录".into(),
                "在当前视图的所有字段中搜索，留空显示全部".into(),
                Some(self.query.clone()),
            ),
            Edit::ReferenceSearch(..) => (
                "查找引用".into(),
                "按笔记、资料、记录或日程名称筛选".into(),
                Some(self.reference_query.clone()),
            ),
            Edit::ReferenceLink(..) => (
                "粘贴 Mochi 链接".into(),
                "插入已复制的笔记、记录或日程链接".into(),
                Some(String::new()),
            ),
            Edit::Filter(f, op) => (
                format!("筛选 · {}", t.fields[f].name),
                format!(
                    "{}：输入匹配内容",
                    OPERATORS
                        .iter()
                        .find(|(o, _)| *o == op)
                        .map(|(_, n)| *n)
                        .unwrap_or("包含")
                ),
                Some(String::new()),
            ),
            Edit::DeleteTable => (
                "删除数据表".into(),
                format!("删除「{}」及其 {} 条记录？", t.name, t.records.len()),
                None,
            ),
            Edit::DeleteView => (
                "删除视图".into(),
                format!("删除「{}」？记录将保留在数据表中。", self.view().name),
                None,
            ),
            Edit::DeleteField(f) => (
                "删除字段".into(),
                format!("删除「{}」及该字段的全部内容？", t.fields[f].name),
                None,
            ),
            Edit::DeleteRecord(_) => ("删除记录".into(), "删除这条记录及其字段内容？".into(), None),
            Edit::DeleteOption(f, o) => (
                "删除选项".into(),
                format!(
                    "删除「{}」并清除记录中的该标签？",
                    t.fields[f].options[o].label
                ),
                None,
            ),
        };
        Prompt {
            edit,
            title,
            description,
            value,
        }
    }

    pub fn submit(&mut self, edit: Edit, text: &str) -> Result<(), String> {
        // 改动实时状态前先校验候选文档，包括类型相关的值。
        let t = self.table();
        let valid = match edit {
            Edit::FieldName(f)
            | Edit::FieldType(f, _)
            | Edit::NewOption(f)
            | Edit::DeleteField(f)
            | Edit::Filter(f, _) => f < t.fields.len(),
            Edit::OptionName(f, o) | Edit::DeleteOption(f, o) => {
                t.fields.get(f).is_some_and(|field| o < field.options.len())
            }
            Edit::Cell(r, f) | Edit::ReferenceSearch(r, f) | Edit::ReferenceLink(r, f) => {
                r < t.records.len() && f < t.fields.len()
            }
            Edit::DeleteRecord(r) => r < t.records.len(),
            _ => true,
        };
        if !valid {
            return Err("数据表已变化，请重新选择要编辑的内容".into());
        }
        let mut candidate = self.clone();
        candidate.apply(edit, text)?;
        serialize_base_document(&candidate.document).map_err(|e| e.to_string())?;
        candidate.changed();
        if matches!(edit, Edit::Search | Edit::ReferenceSearch(..)) {
            candidate.dirty = self.dirty;
        }
        *self = candidate;
        Ok(())
    }

    pub(super) fn apply(&mut self, edit: Edit, text: &str) -> Result<(), String> {
        let name = text.trim();
        if !matches!(
            edit,
            Edit::Cell(..)
                | Edit::Search
                | Edit::Filter(..)
                | Edit::DeleteTable
                | Edit::DeleteView
                | Edit::DeleteField(..)
                | Edit::DeleteRecord(..)
                | Edit::DeleteOption(..)
                | Edit::FieldType(..)
                | Edit::ReferenceSearch(..)
                | Edit::ReferenceLink(..)
        ) && name.is_empty()
        {
            return Err("名称不能为空".into());
        }
        match edit {
            Edit::FrozenRows | Edit::FrozenColumns => {
                let count = name
                    .parse::<u32>()
                    .map_err(|_| "请输入非负整数，0 表示取消冻结")?;
                let max = if edit == Edit::FrozenRows {
                    self.visible_records().len()
                } else {
                    self.visible_fields().len()
                };
                if count as usize > max {
                    return Err(format!("当前视图最多可冻结 {max} 项"));
                }
                if edit == Edit::FrozenRows {
                    self.view_mut().frozen_rows = count;
                } else {
                    self.view_mut().frozen_columns = count;
                }
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
            }
            Edit::NewTable => {
                let t = create_base_table(name);
                self.document.active_table_id = Some(t.id.clone());
                self.document.tables.push(t);
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
            }
            Edit::TableName => self.table_mut().name = name.into(),
            Edit::NewField(kind) => self.table_mut().fields.push(create_base_field(kind, name)),
            Edit::FieldName(f) => self.table_mut().fields[f].name = name.into(),
            Edit::FieldType(f, kind) => {
                let id = self.table().fields[f].id.clone();
                convert_base_field_type(self.table_mut(), &id, kind)
                    .map_err(|error| error.to_string())?;
            }
            Edit::NewOption(f) => self.table_mut().fields[f].options.push(BaseOption {
                id: create_id("opt"),
                label: name.into(),
                color: OptionColor::Blue,
                ..Default::default()
            }),
            Edit::OptionName(f, o) => self.table_mut().fields[f].options[o].label = name.into(),
            Edit::NewView(kind) => {
                let mut view = create_base_view(name, kind);
                if kind == ViewType::Board {
                    view.group_by = self
                        .table()
                        .fields
                        .iter()
                        .find(|f| f.field_type == FieldType::SingleSelect)
                        .map(|f| f.id.clone());
                }
                let id = view.id.clone();
                self.table_mut().views.push(view);
                self.table_mut().active_view_id = Some(id);
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
            }
            Edit::ViewName => self.view_mut().name = name.into(),
            Edit::Cell(r, f) => {
                let field = &self.table().fields[f];
                let id = field.id.clone();
                let value = if name.is_empty() {
                    Value::Null
                } else {
                    match field.field_type {
                        FieldType::DateTime => json!(normalize_date_time_input(name)
                            .ok_or("请输入有效的 YYYY-MM-DDTHH:mm 时间点")?),
                        FieldType::DateRange => normalize_date_range_input(name)
                            .ok_or("请输入有效的开始和结束时间，开始不得晚于结束")?,
                        FieldType::Number | FieldType::Progress => {
                            let n = name.parse::<f64>().map_err(|_| "请输入有效数字")?;
                            if !n.is_finite() {
                                return Err("请输入有限数字".into());
                            }
                            if field.field_type == FieldType::Progress
                                && !(0.0..=100.0).contains(&n)
                            {
                                return Err("进度必须在 0 到 100 之间".into());
                            }
                            json!(n)
                        }
                        _ => json!(text),
                    }
                };
                self.table_mut().records[r].values.insert(id, value);
            }
            Edit::Search => {
                self.query = text.into();
                self.scroll_y = 0.0;
            }
            Edit::ReferenceSearch(..) => self.reference_query = text.into(),
            Edit::ReferenceLink(r, f) => {
                if !is_valid_base_reference(name) {
                    return Err("请输入有效的 Mochi 引用链接".into());
                }
                let id = self.table().fields[f].id.clone();
                let record = &mut self.table_mut().records[r];
                let mut urls = record
                    .values
                    .get(&id)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if !urls.iter().any(|url| url.as_str() == Some(name)) {
                    urls.push(json!(name));
                }
                record.values.insert(id, json!(urls));
            }
            Edit::Filter(f, operator) => {
                let field = &self.table().fields[f];
                let id = field.id.clone();
                let value = match field.field_type {
                    FieldType::DateTime if operator != FilterOperator::Contains => {
                        json!(normalize_date_time_input(name).ok_or("请输入有效的时间点")?)
                    }
                    FieldType::DateRange if operator != FilterOperator::Contains => {
                        normalize_date_range_input(name).ok_or("请输入有效的开始和结束时间")?
                    }
                    FieldType::Number | FieldType::Progress
                        if operator != FilterOperator::Contains =>
                    {
                        let number = name.parse::<f64>().map_err(|_| "请输入有效数字")?;
                        if !number.is_finite() {
                            return Err("请输入有限数字".into());
                        }
                        json!(number)
                    }
                    FieldType::Checkbox if operator != FilterOperator::Contains => {
                        json!(match name {
                            "true" | "1" | "是" => true,
                            "false" | "0" | "否" => false,
                            _ => return Err("请输入 true 或 false".into()),
                        })
                    }
                    FieldType::SingleSelect | FieldType::MultiSelect
                        if operator != FilterOperator::Contains =>
                    {
                        let option = field
                            .options
                            .iter()
                            .find(|o| o.label == name || o.id == name)
                            .ok_or("请输入已有选项的标签")?;
                        json!(option.id)
                    }
                    _ => json!(text),
                };
                self.view_mut().filters.push(BaseFilter {
                    field_id: id,
                    operator,
                    value: Some(value),
                    ..Default::default()
                });
                self.scroll_y = 0.0;
            }
            Edit::DeleteTable => {
                if self.document.tables.len() <= 1 {
                    return Err("至少保留一个数据表".into());
                }
                let i = self.table_index();
                self.document.tables.remove(i);
                self.document.active_table_id = Some(self.document.tables[0].id.clone());
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
            }
            Edit::DeleteView => {
                if self.table().views.len() <= 1 {
                    return Err("至少保留一个视图".into());
                }
                let i = self.view_index();
                self.table_mut().views.remove(i);
                let id = self.table().views[0].id.clone();
                self.table_mut().active_view_id = Some(id);
            }
            Edit::DeleteField(f) => {
                if self.table().fields.len() <= 1 {
                    return Err("至少保留一个字段".into());
                }
                let table = self.table_mut();
                let id = table.fields.remove(f).id;
                for r in &mut table.records {
                    r.values.remove(&id);
                }
                for v in &mut table.views {
                    v.hidden_field_ids.retain(|hidden| hidden != &id);
                    if table
                        .fields
                        .iter()
                        .all(|field| v.hidden_field_ids.contains(&field.id))
                    {
                        v.hidden_field_ids.clear();
                    }
                    v.sorts.retain(|s| s.field_id != id);
                    v.filters.retain(|s| s.field_id != id);
                    if v.group_by.as_deref() == Some(&id) {
                        v.group_by = None;
                    }
                }
            }
            Edit::DeleteRecord(r) => {
                self.table_mut().records.remove(r);
            }
            Edit::DeleteOption(f, o) => {
                let table = self.table_mut();
                let field = &mut table.fields[f];
                let id = field.options.remove(o).id;
                for r in &mut table.records {
                    if let Some(v) = r.values.get_mut(&field.id) {
                        if v.as_str() == Some(&id) {
                            *v = Value::Null;
                        } else if let Some(values) = v.as_array_mut() {
                            values.retain(|v| v.as_str() != Some(&id));
                        }
                    }
                }
                for view in &mut table.views {
                    view.filters.retain(|filter| {
                        filter.field_id != field.id
                            || !filter.value.as_ref().is_some_and(|v| {
                                v.as_str() == Some(&id)
                                    || v.as_array().is_some_and(|values| {
                                        values.iter().any(|value| value.as_str() == Some(&id))
                                    })
                            })
                    });
                }
            }
        }
        self.popup = match edit {
            Edit::ReferenceSearch(r, f) | Edit::ReferenceLink(r, f) => Some(Popup::Reference(r, f)),
            _ => None,
        };
        Ok(())
    }
}
