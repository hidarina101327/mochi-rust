//! 表格只读预览，行高/列宽/预览上限对应 SpreadsheetViewer.tsx。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::Palette,
};
use mochi_core::office::Workbook;
#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub data: Option<Workbook>,
    pub loading: bool,
    pub error: String,
    pub scroll_y: f32,
    pub scroll_x: f32,
    pub tab_start: usize,
    pub requested: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Sheet(usize),
    PreviousTabs,
    NextTabs,
    Reload,
    OpenExternal,
    ShowInFolder,
    Body,
}
#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub body: Rect,
    pub max_y: f32,
    pub max_x: f32,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }
}
pub fn column_name(mut index: usize) -> String {
    let mut s = Vec::new();
    index += 1;
    while index > 0 {
        index -= 1;
        s.push((b'A' + (index % 26) as u8) as char);
        index /= 26;
    }
    s.iter().rev().collect()
}
pub fn layout(s: &State, area: Rect) -> Layout {
    let mut l = Layout {
        body: Rect::new(area.left, area.top + 41.0, area.right, area.bottom - 36.0),
        ..Default::default()
    };
    l.entries.push((l.body, Hit::Body));
    for (i, h) in [Hit::OpenExternal, Hit::ShowInFolder, Hit::Reload]
        .into_iter()
        .enumerate()
    {
        let x = area.right - 12.0 - i as f32 * 36.0;
        l.entries
            .push((Rect::new(x - 32.0, area.top + 4.0, x, area.top + 36.0), h));
    }
    if let Some(data) = &s.data {
        l.max_y = (data.sheet.rows.len() as f32 * 30.0 + 30.0 - l.body.height()).max(0.0);
        l.max_x =
            (data.sheet.column_count.min(120) as f32 * 160.0 + 56.0 - l.body.width()).max(0.0);
        let mut x = area.left + 32.0;
        for (i, name) in data.names.iter().enumerate().skip(s.tab_start) {
            let w = (text::measure(name, TextStyle::Caption) + 24.0).clamp(70.0, 180.0);
            if x + w > area.right - 32.0 {
                break;
            }
            l.entries.push((
                Rect::new(x, area.bottom - 32.0, x + w, area.bottom),
                Hit::Sheet(i),
            ));
            x += w;
        }
    }
    l.entries.push((
        Rect::new(area.left, area.bottom - 32.0, area.left + 30.0, area.bottom),
        Hit::PreviousTabs,
    ));
    l.entries.push((
        Rect::new(
            area.right - 30.0,
            area.bottom - 32.0,
            area.right,
            area.bottom,
        ),
        Hit::NextTabs,
    ));
    l
}
pub fn paint(list: &mut DrawList, area: Rect, s: &State, l: &Layout, name: &str, p: &Palette) {
    list.push_clip(area);
    list.rect(area, p.background);
    list.text(
        Rect::new(
            area.left + 12.0,
            area.top,
            area.right - 124.0,
            area.top + 40.0,
        ),
        name,
        TextStyle::Label,
        p.foreground,
    );
    list.hline(area.left, area.right, area.top + 40.0, p.border);
    for (r, h) in &l.entries {
        let icon = match h {
            Hit::Reload => Some(Icon::REFRESH_CW),
            Hit::OpenExternal => Some(Icon::EXTERNAL_LINK),
            Hit::ShowInFolder => Some(Icon::FOLDER_OPEN),
            Hit::PreviousTabs => Some(Icon::CHEVRON_LEFT),
            Hit::NextTabs => Some(Icon::CHEVRON_RIGHT),
            _ => None,
        };
        if let Some(icon) = icon {
            list.icon_centered(*r, icon, 16.0, p.muted);
        }
        if let (Hit::Sheet(i), Some(data)) = (h, &s.data) {
            if data.names[*i] == data.sheet.name {
                list.rounded_rect(*r, 4.0, p.surface_muted);
            }
            list.text(
                Rect::new(r.left + 12.0, r.top, r.right - 12.0, r.bottom),
                &data.names[*i],
                TextStyle::Caption,
                p.foreground,
            );
        }
    }
    list.push_clip(l.body);
    if s.loading || !s.error.is_empty() {
        list.text(
            Rect::new(
                l.body.left + 24.0,
                l.body.top + 24.0,
                l.body.right - 24.0,
                l.body.top + 72.0,
            ),
            if s.error.is_empty() {
                "正在加载表格…"
            } else {
                &s.error
            },
            TextStyle::Label,
            p.muted,
        );
    } else if let Some(data) = &s.data {
        let cols = data.sheet.column_count.min(120);
        let first = (s.scroll_y / 30.0) as usize;
        let end = (first + (l.body.height() / 30.0).ceil() as usize + 2).min(data.sheet.rows.len());
        let first_col = (s.scroll_x / 160.0) as usize;
        let end_col = (first_col + (l.body.width() / 160.0).ceil() as usize + 1).min(cols);
        let content = Rect::new(
            l.body.left + 56.0,
            l.body.top + 30.0,
            l.body.right,
            l.body.bottom,
        );
        list.push_clip(content);
        for row in first..end {
            let y = l.body.top + 30.0 + row as f32 * 30.0 - s.scroll_y;
            for col in first_col..end_col {
                let x = l.body.left + 56.0 + col as f32 * 160.0 - s.scroll_x;
                let r = Rect::new(x, y, x + 160.0, y + 30.0);
                list.rounded_border(r, 0.0, p.border);
                let value = data.sheet.rows[row]
                    .get(col)
                    .map(|s| s.as_str())
                    .unwrap_or("");
                list.text(
                    Rect::new(x + 8.0, y, x + 152.0, y + 30.0),
                    text::ellipsize(value, TextStyle::Caption, 144.0),
                    TextStyle::Caption,
                    p.foreground,
                );
            }
        }
        list.pop_clip();
        list.rect(
            Rect::new(l.body.left, l.body.top, l.body.right, l.body.top + 30.0),
            p.surface_muted,
        );
        list.push_clip(Rect::new(
            l.body.left + 56.0,
            l.body.top,
            l.body.right,
            l.body.top + 30.0,
        ));
        for col in first_col..end_col {
            let x = l.body.left + 56.0 + col as f32 * 160.0 - s.scroll_x;
            list.text(
                Rect::new(x + 8.0, l.body.top, x + 160.0, l.body.top + 30.0),
                column_name(col),
                TextStyle::Caption,
                p.muted,
            );
        }
        list.pop_clip();
        list.rect(
            Rect::new(
                l.body.left,
                l.body.top + 30.0,
                l.body.left + 56.0,
                l.body.bottom,
            ),
            p.surface_muted,
        );
        for row in first..end {
            let y = l.body.top + 30.0 + row as f32 * 30.0 - s.scroll_y;
            list.text(
                Rect::new(l.body.left + 8.0, y, l.body.left + 52.0, y + 30.0),
                (row + 1).to_string(),
                TextStyle::Caption,
                p.muted,
            );
        }
        if data.sheet.rows.is_empty() {
            list.text(content, "工作表为空", TextStyle::Label, p.muted);
        }
    }
    list.pop_clip();
    list.pop_clip();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excel_column_labels_cross_z_and_zz() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }
    #[test]
    fn huge_sheet_draws_only_visible_cells() {
        let data = Workbook {
            names: vec!["表".into()],
            sheet: mochi_core::office::Sheet {
                name: "表".into(),
                rows: vec![vec!["x".into(); 120]; 5000].into(),
                row_count: 5000,
                column_count: 120,
            },
        };
        let s = State {
            data: Some(data),
            ..Default::default()
        };
        let area = Rect::new(0.0, 0.0, 800.0, 700.0);
        let l = layout(&s, area);
        let mut d = DrawList::new();
        paint(
            &mut d,
            area,
            &s,
            &l,
            "a.xlsx",
            super::super::theme::tokens().palette(false),
        );
        assert!(d.cmds().len() < 1000);
        assert!(d.finish().is_ok());
    }
}
