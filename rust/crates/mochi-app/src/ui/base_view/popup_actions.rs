//! 处理数据表弹出菜单的滚动和选项选择。
use super::*;

impl State {
    pub fn scroll(&mut self, area: Rect, notches: f32, horizontal: bool) {
        let l = layout(self, area);
        if self.popup.is_some() {
            self.popup_scroll = (self.popup_scroll - notches * 96.0).clamp(0.0, l.menu_max);
        } else if self.detail.is_some() && !horizontal {
            self.detail_scroll = (self.detail_scroll - notches * 96.0).clamp(0.0, l.detail_max);
        } else if horizontal {
            self.scroll_x = (self.scroll_x - notches * 120.0).clamp(0.0, l.max_x);
        } else {
            self.scroll_y = (self.scroll_y - notches * 96.0).clamp(0.0, l.max_y);
        }
    }

    pub fn activate(&mut self, hit: Hit) -> Option<Prompt> {
        if !self.editing
            && matches!(
                hit,
                Hit::NewTable
                    | Hit::NewView
                    | Hit::NewField
                    | Hit::NewRecord
                    | Hit::NewTemporaryDocument
                    | Hit::Field(_)
                    | Hit::RemoveReference(..)
                    | Hit::AddReference(..)
            )
        {
            return None;
        }
        match hit {
            Hit::Columns => self.show(Popup::Columns),
            Hit::Freeze => self.show(Popup::Freeze),
            Hit::ExportXlsx => self.action = Some(Action::ExportXlsx),
            Hit::Automations => {
                self.action = Some(Action::Automations);
            }
            Hit::Body | Hit::Dismiss => {
                self.popup = None;
                self.date_time_picker = None;
            }
            Hit::Table(i) => {
                if let Some(t) = self.document.tables.get(i) {
                    self.document.active_table_id = Some(t.id.clone());
                    self.scroll_x = 0.0;
                    self.scroll_y = 0.0;
                    self.popup = None;
                    self.detail = None;
                    self.selected = None;
                    self.located = None;
                    if self.editing {
                        self.changed();
                    }
                }
            }
            Hit::View(i) => {
                if let Some(v) = self.table().views.get(i) {
                    let id = v.id.clone();
                    self.table_mut().active_view_id = Some(id);
                    self.scroll_x = 0.0;
                    self.scroll_y = 0.0;
                    self.popup = None;
                    if self.editing {
                        self.changed();
                    }
                }
            }
            Hit::TablesMenu => self.show(Popup::Table),
            Hit::ViewsMenu => self.show(Popup::View),
            Hit::NewTable => return Some(self.prompt(Edit::NewTable)),
            Hit::NewView => self.show(Popup::NewView),
            Hit::NewField => self.show(Popup::NewField),
            Hit::NewRecord => {
                self.table_mut().records.push(BaseRecord {
                    id: create_id("rec"),
                    ..Default::default()
                });
                self.changed();
            }
            Hit::NewTemporaryDocument => {
                self.action = Some(Action::CreateTemporaryDocument);
            }
            Hit::Field(i) => self.show(Popup::Field(i)),
            Hit::Record(i) => {
                self.detail = Some(i);
                self.detail_scroll = 0.0;
                self.popup = None;
            }
            Hit::RecordMenu(i) => self.show(Popup::Record(i)),
            Hit::Cell(r, f) => {
                let field = self.table().fields.get(f)?;
                let kind = field.field_type;
                let id = field.id.clone();
                self.selected = Some((r, f));
                if !self.editing {
                    if matches!(kind, FieldType::Reference | FieldType::Document) {
                        let urls = self
                            .table()
                            .records
                            .get(r)?
                            .values
                            .get(&id)
                            .and_then(Value::as_array);
                        if let Some(url) = urls
                            .filter(|urls| urls.len() == 1)
                            .and_then(|urls| urls[0].as_str())
                        {
                            self.action = Some(Action::Open(url.into()));
                        } else {
                            self.detail = Some(r);
                            self.detail_scroll = 0.0;
                        }
                    } else if kind == FieldType::Url {
                        if let Some(url) = self
                            .table()
                            .records
                            .get(r)?
                            .values
                            .get(&id)
                            .and_then(Value::as_str)
                        {
                            self.action = Some(Action::Open(url.into()));
                        }
                    }
                    return None;
                }
                if kind == FieldType::Checkbox {
                    let record = self.table_mut().records.get_mut(r)?;
                    let checked = record
                        .values
                        .get(&id)
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    record.values.insert(id, json!(!checked));
                    self.changed();
                } else if matches!(kind, FieldType::SingleSelect | FieldType::MultiSelect) {
                    self.show(Popup::Select(r, f));
                } else if kind == FieldType::DateTime {
                    // 不要落到通用文本对话框：日期选择器始终提供
                    // 确认过的当前时间戳。
                    self.show(Popup::DateTime(r, f));
                } else if matches!(kind, FieldType::Reference | FieldType::Document) {
                    self.action = Some(Action::LoadReferences(r, f));
                    self.reference_query.clear();
                    self.show(Popup::Reference(r, f));
                } else {
                    return Some(self.prompt(Edit::Cell(r, f)));
                }
            }
            Hit::Search => return Some(self.prompt(Edit::Search)),
            Hit::ProgressInput(r, f) => return Some(self.prompt(Edit::Cell(r, f))),
            Hit::Sort => self.show(Popup::Sort),
            Hit::RowHeight if self.grid() => self.show(Popup::RowHeight),
            Hit::RowHeight => {}
            Hit::Filter => self.show(Popup::Filter),
            Hit::Group => self.show(Popup::Group),
            Hit::Save => {}
            Hit::ToggleEditing => {
                self.editing = !self.editing;
                self.popup = None;
            }
            Hit::ClearLocation => {
                self.located = None;
                self.detail = None;
                self.selected = None;
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
            }
            Hit::DateTimeAdjust(part, delta) => {
                if let Some(picker) = self.date_time_picker.as_mut() {
                    picker.adjust(part, delta);
                }
            }
            Hit::DateTimeConfirm => {
                let (record, field) = match self.popup {
                    Some(Popup::DateTime(record, field)) => (record, field),
                    _ => return None,
                };
                let Some(value) = self.date_time_picker.map(DateTimePicker::as_rfc3339) else {
                    return None;
                };
                return self.choose(Choice::SetDateTime(record, field, value));
            }
            Hit::DateTimeClear => {
                let (record, field) = match self.popup {
                    Some(Popup::DateTime(record, field)) => (record, field),
                    _ => return None,
                };
                return self.choose(Choice::ClearCell(record, field));
            }
            Hit::Resize(_) | Hit::ScrollTrack | Hit::ScrollThumb | Hit::DetailBody => {}
            Hit::CloseDetail | Hit::DetailBackdrop => {
                self.detail = None;
                self.popup = None;
            }
            Hit::CopyRecord(r) => {
                self.action = Some(Action::CopyRecord(
                    r,
                    self.selected
                        .filter(|(selected, _)| *selected == r)
                        .map(|(_, field)| field),
                ))
            }
            Hit::AddReference(r, f) => {
                self.action = Some(Action::LoadReferences(r, f));
                self.reference_query.clear();
                self.show(Popup::Reference(r, f));
            }
            Hit::OpenReference(r, f, i) => {
                let id = &self.table().fields.get(f)?.id;
                let url = self
                    .table()
                    .records
                    .get(r)?
                    .values
                    .get(id)?
                    .as_array()?
                    .get(i)?
                    .as_str()?
                    .to_owned();
                self.action = Some(Action::Open(url));
            }
            Hit::RemoveReference(r, f, i) => {
                let id = self.table().fields.get(f)?.id.clone();
                if let Some(urls) = self
                    .table_mut()
                    .records
                    .get_mut(r)?
                    .values
                    .get_mut(&id)
                    .and_then(Value::as_array_mut)
                {
                    if i < urls.len() {
                        urls.remove(i);
                        self.changed();
                    }
                }
            }
            Hit::Menu(i) => {
                let item = self.menu_items().get(i)?.choice.clone();
                return self.choose(item);
            }
        }
        None
    }

    pub(super) fn choose(&mut self, choice: Choice) -> Option<Prompt> {
        match choice {
            Choice::ExportXlsx => {
                self.popup = None;
                self.action = Some(Action::ExportXlsx);
            }
            Choice::ToggleColumn(field) => {
                let id = self.table().fields.get(field)?.id.clone();
                if self.view().hidden_field_ids.contains(&id) {
                    self.view_mut()
                        .hidden_field_ids
                        .retain(|hidden| hidden != &id);
                } else {
                    if visible_field_indices(self.table(), self.view()).len() <= 1 {
                        self.error = "至少保留一列；隐藏列不会删除数据".into();
                        return None;
                    }
                    self.view_mut().hidden_field_ids.push(id);
                }
                self.located = None;
                self.selected = None;
                self.scroll_x = 0.0;
                self.changed();
            }
            Choice::ShowAllColumns => {
                self.view_mut().hidden_field_ids.clear();
                self.changed();
            }
            Choice::ClearFreeze => {
                self.view_mut().frozen_rows = 0;
                self.view_mut().frozen_columns = 0;
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
                self.popup = None;
                self.changed();
            }
            Choice::Table(i) => return self.activate(Hit::Table(i)),
            Choice::View(i) => return self.activate(Hit::View(i)),
            Choice::Popup(p) => {
                let anchor = self.popup_anchor;
                self.show(p);
                if anchor.is_some() {
                    self.popup_anchor = anchor;
                }
            }
            Choice::Edit(edit) => {
                self.popup = None;
                return Some(self.prompt(edit));
            }
            Choice::SetType(f, kind) => {
                let field = self.table().fields.get(f)?;
                if field.field_type == kind {
                    return None;
                }
                let id = field.id.clone();
                let mut preview = self.table().clone();
                match convert_base_field_type(&mut preview, &id, kind) {
                    Ok(cleared) => {
                        self.popup = None;
                        if cleared > 0 {
                            return Some(Prompt {
                                edit: Edit::FieldType(f, kind),
                                title: "更改字段类型".into(),
                                description: format!(
                                    "将清空 {cleared} 个无法转换为「{}」的值。",
                                    type_name(kind)
                                ),
                                value: None,
                            });
                        }
                        *self.table_mut() = preview;
                        self.changed();
                    }
                    Err(error) => self.error = format!("字段类型转换失败：{error}"),
                }
            }
            Choice::SetColor(f, o, color) => {
                self.table_mut()
                    .fields
                    .get_mut(f)?
                    .options
                    .get_mut(o)?
                    .color = color;
                self.changed();
                self.show(Popup::Options(f));
            }
            Choice::ToggleOption(r, f, o) => {
                let field = self.table().fields.get(f)?;
                let id = field.id.clone();
                let option = field.options.get(o)?.id.clone();
                let multi = field.field_type == FieldType::MultiSelect;
                let record = self.table_mut().records.get_mut(r)?;
                if multi {
                    let mut values = record
                        .values
                        .get(&id)
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    if let Some(i) = values.iter().position(|v| v.as_str() == Some(&option)) {
                        values.remove(i);
                    } else {
                        values.push(json!(option));
                    }
                    record.values.insert(id, json!(values));
                } else {
                    record.values.insert(id, json!(option));
                    self.popup = None;
                }
                self.changed();
            }
            Choice::SetDateTime(r, f, value) => {
                let id = self.table().fields.get(f)?.id.clone();
                self.table_mut()
                    .records
                    .get_mut(r)?
                    .values
                    .insert(id, json!(value));
                self.popup = None;
                self.date_time_picker = None;
                self.changed();
            }
            Choice::ClearCell(r, f) => {
                let id = self.table().fields.get(f)?.id.clone();
                self.table_mut().records.get_mut(r)?.values.remove(&id);
                self.popup = None;
                self.date_time_picker = None;
                self.changed();
            }
            Choice::Sort(f, direction) => {
                let id = self.table().fields.get(f)?.id.clone();
                self.view_mut().sorts = vec![BaseSort {
                    field_id: id,
                    direction,
                    ..Default::default()
                }];
                self.popup = None;
                self.changed();
            }
            Choice::ClearSort => {
                self.view_mut().sorts.clear();
                self.popup = None;
                self.changed();
            }
            Choice::SetRowHeight(height) => {
                self.view_mut().row_height =
                    (height != DEFAULT_GRID_ROW_HEIGHT).then_some(height as f64);
                self.popup = None;
                self.scroll_y = 0.0;
                self.changed();
            }
            Choice::EmptyFilter(f, operator) => {
                let id = self.table().fields.get(f)?.id.clone();
                self.view_mut().filters.push(BaseFilter {
                    field_id: id,
                    operator,
                    value: None,
                    ..Default::default()
                });
                self.popup = None;
                self.changed();
            }
            Choice::ClearFilter => {
                self.view_mut().filters.clear();
                self.popup = None;
                self.changed();
            }
            Choice::Group(f) => {
                let id = self.table().fields.get(f)?.id.clone();
                self.view_mut().group_by = Some(id);
                self.popup = None;
                self.scroll_x = 0.0;
                self.scroll_y = 0.0;
                self.changed();
            }
            Choice::DuplicateRecord(r) => {
                let mut record = self.table().records.get(r)?.clone();
                record.id = create_id("rec");
                self.table_mut().records.push(record);
                self.popup = None;
                self.changed();
            }
            Choice::CopyRecord(r) => self.action = Some(Action::CopyRecord(r, None)),
            Choice::ShowRecord(r) => {
                self.detail = Some(r);
                self.detail_scroll = 0.0;
                self.popup = None;
            }
            Choice::ToggleReference(r, f, index) => {
                let url = self.reference_candidates.get(index)?.url.clone();
                let id = self.table().fields.get(f)?.id.clone();
                let record = self.table_mut().records.get_mut(r)?;
                let mut urls = record
                    .values
                    .get(&id)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(index) = urls.iter().position(|value| value.as_str() == Some(&url)) {
                    urls.remove(index);
                } else {
                    urls.push(json!(url));
                }
                record.values.insert(id, json!(urls));
                self.changed();
            }
        }
        None
    }
}
