//! 与 Electron 插入控件对应的 10 × 10 表格选择器。
use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    theme::{self, Palette},
};
#[derive(Clone, Debug)]
pub struct Picker {
    pub rect: Rect,
    pub rows: usize,
    pub cols: usize,
}
impl Picker {
    pub fn open(x: f32, y: f32, viewport: Rect) -> Self {
        let width = 224.0;
        let height = 282.0;
        let x = x.min(viewport.right - width - 8.0).max(viewport.left + 8.0);
        let y = y
            .min(viewport.bottom - height - 8.0)
            .max(viewport.top + 8.0);
        Self {
            rect: Rect::from_size(x, y, width, height),
            rows: 3,
            cols: 3,
        }
    }
    pub fn cell_rect(&self, row: usize, col: usize) -> Rect {
        Rect::from_size(
            self.rect.left + 12.0 + (col - 1) as f32 * 20.0,
            self.rect.top + 38.0 + (row - 1) as f32 * 20.0,
            16.0,
            16.0,
        )
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if Rect::new(
            self.rect.left + 12.0,
            self.rect.bottom - 38.0,
            self.rect.right - 12.0,
            self.rect.bottom - 10.0,
        )
        .contains(x, y)
        {
            return Some((3, 3));
        }
        (1..=10)
            .flat_map(|row| (1..=10).map(move |col| (row, col)))
            .find(|(row, col)| self.cell_rect(*row, *col).contains(x, y))
    }
    pub fn hover(&mut self, x: f32, y: f32) -> bool {
        if let Some((rows, cols)) = self.hit(x, y) {
            let changed = self.rows != rows || self.cols != cols;
            self.rows = rows;
            self.cols = cols;
            changed
        } else {
            false
        }
    }
    pub fn paint(&self, list: &mut DrawList, p: &Palette) {
        list.rounded_rect(self.rect, 10.0, p.surface);
        list.rounded_border(self.rect, 10.0, p.border);
        list.text(
            Rect::new(
                self.rect.left + 12.0,
                self.rect.top + 6.0,
                self.rect.right - 12.0,
                self.rect.top + 32.0,
            ),
            format!("插入表格       {} × {}", self.rows, self.cols),
            TextStyle::Label,
            p.foreground,
        );
        for row in 1..=10 {
            for col in 1..=10 {
                let active = row <= self.rows && col <= self.cols;
                let rect = self.cell_rect(row, col);
                list.rounded_rect(
                    rect,
                    3.0,
                    if active {
                        theme::mix(p.accent, p.surface, 0.25)
                    } else {
                        p.background
                    },
                );
                list.rounded_border(rect, 3.0, if active { p.accent } else { p.border });
            }
        }
        let quick = Rect::new(
            self.rect.left + 12.0,
            self.rect.bottom - 38.0,
            self.rect.right - 12.0,
            self.rect.bottom - 10.0,
        );
        list.rounded_border(quick, 5.0, p.border);
        list.text(quick, "快速插入 3 × 3", TextStyle::Label, p.foreground);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_visible_grid_cell_inserts_its_displayed_dimensions() {
        let mut picker = Picker::open(890.0, 590.0, Rect::new(0.0, 0.0, 900.0, 600.0));
        assert!(picker.rect.right <= 900.0 && picker.rect.bottom <= 600.0);
        for row in 1..=10 {
            for col in 1..=10 {
                let cell = picker.cell_rect(row, col);
                assert_eq!(
                    picker.hit(cell.left + 8.0, cell.top + 8.0),
                    Some((row, col))
                );
                picker.hover(cell.left + 8.0, cell.top + 8.0);
                assert_eq!((picker.rows, picker.cols), (row, col));
            }
        }
    }
}
