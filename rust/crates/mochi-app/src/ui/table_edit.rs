//! 单元格编辑仅替换源码内容区，保留管道、空白和 CRLF。
use super::{layout::Rect, widgets::TextField};
use std::{ops::Range, path::PathBuf};
#[derive(Clone, Debug)]
pub struct Cell {
    pub range: Range<usize>,
    pub rect: Rect,
}
pub struct Editing {
    pub path: PathBuf,
    pub cell: Cell,
    pub original: String,
    pub field: TextField,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RowBefore,
    RowAfter,
    DeleteRow,
    ColumnBefore,
    ColumnAfter,
    DeleteColumn,
    DeleteTable,
    ParagraphAfter,
}
pub const ACTIONS: &[(&str, Action)] = &[
    ("上方插入行", Action::RowBefore),
    ("下方插入行", Action::RowAfter),
    ("删除行", Action::DeleteRow),
    ("左侧插入列", Action::ColumnBefore),
    ("右侧插入列", Action::ColumnAfter),
    ("删除列", Action::DeleteColumn),
    ("下方插入空行", Action::ParagraphAfter),
    ("删除表格", Action::DeleteTable),
];

pub fn insert(buffer: &mut super::editor::TextBuffer, rows: usize, cols: usize) {
    let row = vec![String::new(); cols.clamp(1, 10)];
    let mut data = vec![row; rows.clamp(1, 10)];
    for (col, value) in data[0].iter_mut().enumerate() {
        *value = format!("列 {}", col + 1);
    }
    let newline = if buffer.text().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let table = serialize(&data, &vec!["---".into(); cols.clamp(1, 10)], newline);
    super::format::insert_block(buffer, &table);
}

fn serialize(rows: &[Vec<String>], separators: &[String], newline: &str) -> String {
    let render = |row: &[String]| format!("| {} |", row.join(" | "));
    let mut lines = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        lines.push(render(row));
        if i == 0 {
            lines.push(render(separators));
        }
    }
    lines.join(newline)
}

pub fn apply(buffer: &mut super::editor::TextBuffer, at: usize, action: Action) -> bool {
    let parsed = super::document::parse_ranged(buffer.text());
    let Some(block) = parsed.blocks.iter().find(|b| {
        b.start <= at && at <= b.end && matches!(b.block, super::document::Block::Table { .. })
    }) else {
        return false;
    };
    let raw = &buffer.text()[block.start..block.end];
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let mut rows = Vec::new();
    let mut separators = Vec::new();
    let mut offset = block.start;
    let mut row = 0;
    let mut col = 0;
    for (i, line) in raw.split_inclusive('\n').enumerate() {
        let ranges = ranges(line, offset);
        if i != 1 && offset <= at && at <= offset + line.len() {
            row = rows.len();
            col = ranges
                .iter()
                .position(|r| r.start <= at && at <= r.end)
                .unwrap_or(0);
        }
        let values = ranges
            .into_iter()
            .map(|r| buffer.text()[r].to_owned())
            .collect::<Vec<_>>();
        if i == 1 {
            separators = values;
        } else {
            rows.push(values);
        }
        offset += line.len();
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    for cells in &mut rows {
        cells.resize(cols, String::new());
    }
    separators.resize(cols, "---".into());
    if action == Action::ParagraphAfter {
        buffer.replace_range_select(
            block.end..block.end,
            &format!("{newline}{newline}"),
            newline.len()..newline.len(),
        );
        return true;
    }
    match action {
        Action::RowBefore | Action::RowAfter => {
            rows.insert(
                row + usize::from(action == Action::RowAfter),
                vec![String::new(); cols],
            );
        }
        Action::DeleteRow => {
            rows.remove(row);
        }
        Action::ColumnBefore | Action::ColumnAfter => {
            let col = col + usize::from(action == Action::ColumnAfter);
            for cells in &mut rows {
                cells.insert(col, String::new());
            }
            separators.insert(col, "---".into());
        }
        Action::DeleteColumn => {
            for cells in &mut rows {
                cells.remove(col);
            }
            separators.remove(col);
        }
        Action::DeleteTable => rows.clear(),
        Action::ParagraphAfter => unreachable!(),
    }
    let value = if rows.is_empty() || rows[0].is_empty() {
        String::new()
    } else {
        serialize(&rows, &separators, newline)
    };
    buffer.replace_range_select(block.start..block.end, &value, 0..0);
    true
}
impl Editing {
    fn origin(&self) -> (f32, f32) {
        let padding = super::editor_preferences::current().table_padding;
        (
            self.cell.rect.left + padding + 4.0,
            self.cell.rect.top + padding,
        )
    }
    fn layout(&self) -> super::live::LiveLayout {
        let shown = self.field.buffer.display_text().0;
        let width = (self.cell.rect.right
            - self.origin().0
            - super::editor_preferences::current().table_padding
            - 4.0)
            .max(20.0);
        super::live::inline_layout(&shown, width, super::draw::TextStyle::Table)
    }
    pub fn click(&mut self, x: f32, y: f32, extend: bool) {
        let origin = self.origin();
        let layout = self.layout();
        if let Some(at) = layout.hit(self.field.text(), x - origin.0, y - origin.1) {
            self.field.buffer.set_cursor(at, extend);
        }
    }
    pub fn key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        let layout = self.layout();
        let b = &mut self.field.buffer;
        match key {
            0x5a | 0x59 if ctrl => {
                if key == 0x59 || shift {
                    b.redo();
                } else {
                    b.undo();
                }
            }
            0x25 | 0x27 => layout.move_horizontal(b, key == 0x27, shift),
            0x24 | 0x23 => {
                let (a, z) = layout.line_bounds(b.cursor());
                b.set_cursor(if key == 0x24 { a } else { z }, shift);
            }
            8 | 0x2e => super::rich::delete(b, key == 8, &layout.parsed),
            0x26 | 0x28 => {
                let x = layout.locate(b.cursor()).map(|(_, x)| x).unwrap_or(0.0);
                let at =
                    layout.move_vertical(b.text(), b.cursor(), if key == 0x26 { -1 } else { 1 }, x);
                b.set_cursor(at, shift);
            }
            0x41 if ctrl => b.select_all(),
            _ => {
                return !matches!(
                    self.field.key(key, shift, ctrl),
                    super::widgets::FieldKey::Ignored
                )
            }
        }
        true
    }
    pub fn selected_text(&self) -> String {
        self.layout().selected_text(&self.field.buffer)
    }
    pub fn caret(&self) -> Option<Rect> {
        let r = self.layout().caret(self.field.buffer.display_cursor())?;
        let (x, y) = self.origin();
        Some(Rect::new(x + r.left, y + r.top, x + r.right, y + r.bottom))
    }
    pub fn paint(
        &self,
        list: &mut super::draw::DrawList,
        focused: bool,
        p: &super::theme::Palette,
    ) {
        let rect = self.cell.rect;
        let (x, y) = self.origin();
        let layout = self.layout();
        list.push_clip(rect);
        list.rect(rect, p.area_main_default);
        list.rounded_border(rect, 0.0, p.accent);
        let area = Rect::new(
            x - super::editor_preferences::current().padding_left,
            y,
            rect.right,
            rect.bottom,
        );
        super::document::paint_in(list, area, &layout.layout, 0.0, None, p);
        super::live::paint_overlay(
            list,
            area,
            &layout,
            &self.field.buffer,
            0.0,
            focused,
            focused,
            p,
        );
        list.pop_clip();
    }
}
pub fn ranges(line: &str, offset: usize) -> Vec<Range<usize>> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut slashes = 0;
    for (i, c) in line.char_indices() {
        if c == '|' && slashes % 2 == 0 {
            pieces.push(start..i);
            start = i + 1;
        }
        if c == '\\' {
            slashes += 1;
        } else {
            slashes = 0;
        }
    }
    pieces.push(start..line.len());
    if line.trim_start().starts_with('|') && !pieces.is_empty() {
        pieces.remove(0);
    }
    if line.trim_end().ends_with('|')
        && pieces
            .last()
            .is_some_and(|r| line[r.clone()].trim().is_empty())
    {
        pieces.pop();
    }
    pieces
        .into_iter()
        .map(|r| {
            let text = &line[r.clone()];
            let a = r.start + text.len() - text.trim_start().len();
            let b = r.end - (text.len() - text.trim_end().len());
            offset + a..offset + b.max(a)
        })
        .collect()
}
pub fn encode(value: &str) -> String {
    let mut out = String::new();
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\n' {
            out.push_str("<br>");
            slashes = 0;
            continue;
        }
        if ch == '\r' {
            continue;
        }
        if ch == '|' && slashes % 2 == 0 {
            out.push('\\');
        }
        out.push(ch);
        if ch == '\\' {
            slashes += 1
        } else {
            slashes = 0
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structural_actions_preserve_surroundings_cells_alignment_and_undo() {
        for newline in ["\n", "\r\n"] {
            let source = format!("# 前文{newline}{newline}| 甲 | 乙 |{newline}| :--- | ---: |{newline}| 中\\|文 | **粗体** |{newline}{newline}后文");
            let at = source.find("**粗体**").unwrap();
            for (_, action) in ACTIONS {
                let mut buffer = super::super::editor::TextBuffer::new(&source);
                assert!(apply(&mut buffer, at, *action));
                assert!(buffer
                    .text()
                    .starts_with(&format!("# 前文{newline}{newline}")));
                assert!(buffer.text().ends_with(&format!("{newline}{newline}后文")));
                if matches!(
                    action,
                    Action::RowBefore
                        | Action::RowAfter
                        | Action::ColumnBefore
                        | Action::ColumnAfter
                ) {
                    assert!(buffer.text().contains("中\\|文"));
                    assert!(buffer.text().contains("**粗体**"));
                    assert!(buffer.text().contains(":---") && buffer.text().contains("---:"));
                }
                assert!(buffer.undo());
                assert_eq!(buffer.text(), source);
            }
        }
    }
    #[test]
    fn picker_dimensions_create_the_requested_table() {
        for (rows, cols) in [(1, 1), (3, 3), (10, 10)] {
            let mut buffer = super::super::editor::TextBuffer::new("");
            insert(&mut buffer, rows, cols);
            let parsed = super::super::document::parse_ranged(buffer.text());
            let super::super::document::Block::Table { rows: cells, .. } = &parsed.blocks[0].block
            else {
                panic!("table missing")
            };
            assert_eq!(cells.len(), rows);
            assert!(cells.iter().all(|r| r.len() == cols));
            buffer.undo();
            assert_eq!(buffer.text(), "");
        }
    }
    #[test]
    fn cell_editing_uses_visible_text_and_keeps_markdown_pairs() {
        let mut field = TextField::new("");
        field.set_text("**中文**");
        field.buffer.set_cursor(2, false);
        let mut edit = Editing {
            path: PathBuf::from("test.md"),
            cell: Cell {
                range: 0..10,
                rect: Rect::new(0.0, 0.0, 200.0, 45.0),
            },
            original: field.text().into(),
            field,
        };
        edit.key(0x27, true, false);
        assert_eq!(edit.selected_text(), "中");
        assert!(edit.caret().is_some());
        edit.key(8, false, false);
        assert_eq!(edit.field.text(), "**文**");
        edit.field.buffer.undo();
        assert_eq!(edit.field.text(), "**中文**");
    }
    #[test]
    fn chinese_escaped_pipes_and_crlf_keep_exact_ranges() {
        let s = "  | 中文 | a\\|b | 尾巴 |\r\n";
        let r = ranges(s, 0);
        assert_eq!(
            r.iter().map(|r| &s[r.clone()]).collect::<Vec<_>>(),
            ["中文", "a\\|b", "尾巴"]
        );
        let mut next = s.to_owned();
        next.replace_range(r[1].clone(), &encode("新|值"));
        assert_eq!(next, "  | 中文 | 新\\|值 | 尾巴 |\r\n");
    }
    #[test]
    fn encoding_does_not_double_escape_existing_pipe() {
        assert_eq!(encode("a\\|b"), "a\\|b");
        assert_eq!(encode("a\nb"), "a<br>b");
    }
}
