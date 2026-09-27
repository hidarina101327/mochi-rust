//! 根据当前资料库和视图配置读取数据表界面状态。
use super::*;

impl State {
    pub fn parse(raw: String) -> anyhow::Result<Self> {
        Ok(Self {
            document: parse_base_document(&raw)?,
            saved_raw: raw,
            dirty: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
            query: String::new(),
            error: String::new(),
            popup: None,
            popup_anchor: None,
            date_time_picker: None,
            popup_scroll: 0.0,
            // 新表格默认进入编辑模式，除非共享设置要求视图模式。
            // 这样原生端与 Web 端的默认值保持一致。
            editing: crate::ui::settings_values::text("tables.baseDefaultMode", "edit") != "view",
            selected: None,
            detail: None,
            detail_scroll: 0.0,
            located: None,
            hover: None,
            reference_candidates: Vec::new(),
            reference_query: String::new(),
            action: None,
            drag: None,
            scrollbars: Interaction::default(),
        })
    }

    pub fn table_index(&self) -> usize {
        self.document
            .active_table_id
            .as_ref()
            .and_then(|id| self.document.tables.iter().position(|t| &t.id == id))
            .unwrap_or(0)
    }

    pub fn table(&self) -> &BaseTable {
        &self.document.tables[self.table_index()]
    }

    pub(super) fn table_mut(&mut self) -> &mut BaseTable {
        let i = self.table_index();
        &mut self.document.tables[i]
    }

    pub fn view_index(&self) -> usize {
        let t = self.table();
        t.active_view_id
            .as_ref()
            .and_then(|id| t.views.iter().position(|v| &v.id == id))
            .unwrap_or(0)
    }

    pub fn view(&self) -> &BaseView {
        &self.table().views[self.view_index()]
    }

    pub(super) fn view_mut(&mut self) -> &mut BaseView {
        let i = self.view_index();
        &mut self.table_mut().views[i]
    }

    pub(super) fn changed(&mut self) {
        self.dirty = true;
        self.error.clear();
    }

    /// 在单元格相邻位置插入一个空记录或文本字段。
    ///
    /// 行/列的插入只在编辑模式可用，避免右键菜单绕过只读视图。
    pub fn insert_next_to_cell(
        &mut self,
        record: usize,
        field: usize,
        direction: InsertDirection,
    ) -> bool {
        if !self.editing
            || record >= self.table().records.len()
            || field >= self.table().fields.len()
        {
            return false;
        }
        match direction {
            InsertDirection::Above | InsertDirection::Below => {
                let index = record + usize::from(direction == InsertDirection::Below);
                self.table_mut().records.insert(
                    index,
                    BaseRecord {
                        id: create_id("rec"),
                        ..Default::default()
                    },
                );
                self.selected = Some((index, field));
            }
            InsertDirection::Left | InsertDirection::Right => {
                let used = self
                    .table()
                    .fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<std::collections::HashSet<_>>();
                let name = (1usize..)
                    .map(|index| {
                        if index == 1 {
                            "新字段".to_owned()
                        } else {
                            format!("新字段 {index}")
                        }
                    })
                    .find(|name| !used.contains(name.as_str()))
                    .unwrap_or_else(|| "新字段".into());
                let index = field + usize::from(direction == InsertDirection::Right);
                self.table_mut()
                    .fields
                    .insert(index, create_base_field(FieldType::Text, &name));
                self.selected = Some((record, index));
            }
        }
        self.popup = None;
        self.changed();
        true
    }

    pub(super) fn show(&mut self, popup: Popup) {
        if let Some(anchor) = popup_geometry::origin(popup) {
            self.popup_anchor = Some(anchor);
        } else if self.popup.is_none() {
            self.popup_anchor = None;
        }
        self.popup = Some(popup);
        self.date_time_picker = matches!(popup, Popup::DateTime(..)).then(DateTimePicker::now);
        self.popup_scroll = 0.0;
    }

    pub fn close_popup(&mut self) -> bool {
        if self.popup.take().is_some() {
            self.date_time_picker = None;
            true
        } else {
            self.detail.take().is_some()
        }
    }

    pub fn close_menu(&mut self) {
        self.popup = None;
        self.date_time_picker = None;
    }

    /// 用共享对象选择器确认的 URL 替换引用单元格的内容。宿主只提供
    /// 已持久化的 Mochi 链接；这里再校验一遍，防止过期或跨字段的对话框
    /// 把任意 JSON 写进表格。顺序保持不变，重复链接折叠为首次出现，
    /// 保证保存结果确定。
    pub fn set_reference_urls(
        &mut self,
        record: usize,
        field: usize,
        urls: impl IntoIterator<Item = String>,
    ) -> Result<(), String> {
        let field_kind = self
            .table()
            .fields
            .get(field)
            .ok_or_else(|| "字段不存在".to_owned())?
            .field_type;
        if !matches!(field_kind, FieldType::Reference | FieldType::Document) {
            return Err("该字段不是对象引用字段".into());
        }
        let field_id = self.table().fields[field].id.clone();
        if record >= self.table().records.len() {
            return Err("记录不存在".into());
        }
        let mut selected = Vec::new();
        for url in urls {
            let reference = ObjectReference::parse(&url)
                .ok_or_else(|| "包含无效的 Mochi 引用链接".to_owned())?;
            if field_kind == FieldType::Document && !reference.is_document() {
                return Err("文档字段只能关联文档对象".into());
            }
            if !selected.iter().any(|value: &String| value == &url) {
                selected.push(url);
            }
        }
        self.table_mut().records[record]
            .values
            .insert(field_id, json!(selected));
        self.popup = None;
        self.changed();
        Ok(())
    }
}
