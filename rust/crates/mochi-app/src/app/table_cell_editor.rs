//! 处理数据表单元格编辑的开始和提交。
use super::*;

impl App {
    pub(super) fn begin_table_cell(&mut self, cell: table_edit::Cell, x: f32) {
        let Some(path) = self.active_file_path() else {
            return;
        };
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        let Some(value) = buffer.text().get(cell.range.clone()) else {
            return;
        };
        let original = value.to_owned();
        let mut field = TextField::new("");
        field.style = TextStyle::Table;
        field.set_text(value);
        let mut edit = table_edit::Editing {
            path,
            cell,
            original,
            field,
        };
        edit.click(x, edit.cell.rect.top + 12.0, false);
        self.table_editing = Some(edit);
        self.focus = Focus::TableCell;
        self.editor_engaged = false;
    }

    pub(super) fn commit_table_cell(&mut self) -> bool {
        let Some(edit) = self.table_editing.take() else {
            return true;
        };
        if self.active_file_path().as_deref() != Some(edit.path.as_path()) {
            self.table_editing = Some(edit);
            self.state.status_text = "单元格所属文档已切换".into();
            return false;
        }
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return false;
        };
        if buffer.text().get(edit.cell.range.clone()) != Some(edit.original.as_str()) {
            self.table_editing = Some(edit);
            self.state.status_text = "单元格源码已变化，取消编辑后重试".into();
            return false;
        }
        let value = table_edit::encode(edit.field.text());
        let (a, b) = edit.field.buffer.selection();
        let a = table_edit::encode(&edit.field.text()[..a]).len();
        let b = table_edit::encode(&edit.field.text()[..b]).len();
        if value != edit.original {
            buffer.replace_range_select(edit.cell.range.clone(), &value, a..b);
        } else {
            buffer.set_cursor(edit.cell.range.start + a, false);
            buffer.set_cursor(edit.cell.range.start + b, true);
        }
        self.focus = Focus::Main;
        self.editor_engaged = false;
        self.after_doc_edit(false);
        true
    }
}
