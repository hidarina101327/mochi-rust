//! 为拖到侧栏外的文件显示目标目录树。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    theme::Palette,
};
use std::path::{Path, PathBuf};

mod painting;
#[cfg(test)]
mod tests;

const ROW_HEIGHT: f32 = 32.0;

fn large_title_extra() -> f32 {
    (TextStyle::Large.line_height() - 28.0).max(0.0)
}

pub struct Folder {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub expanded: bool,
    pub expandable: bool,
}

impl Folder {
    pub fn new(path: PathBuf, name: String, depth: usize) -> Self {
        Self {
            path,
            name,
            depth,
            expanded: false,
            expandable: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Select(usize),
    Toggle(usize),
    Cancel,
    Confirm,
    Inside,
    Outside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Cancel,
    Confirm,
}

pub struct State {
    pub folders: Vec<Folder>,
    pub selected: Option<PathBuf>,
    pub summary: String,
    pub error: String,
    pub scroll: f32,
    pub hover: Option<Hit>,
    pub focus: Focus,
}

pub struct Layout {
    pub panel: Rect,
    pub tree: Rect,
    pub close: Rect,
    pub cancel: Rect,
    pub confirm: Rect,
    pub max_scroll: f32,
}

impl State {
    pub fn new(folders: Vec<Folder>, paths: &[PathBuf], preferred: Option<&Path>) -> Self {
        let selected = preferred
            .and_then(|path| folders.iter().find(|f| f.path == path))
            .or_else(|| folders.first())
            .map(|f| f.path.clone());
        let names = paths
            .iter()
            .take(3)
            .map(|p| p.file_name().unwrap_or(p.as_os_str()).to_string_lossy())
            .collect::<Vec<_>>()
            .join("、");
        Self {
            folders,
            selected,
            summary: format!(
                "共 {} 项 · {names}{}",
                paths.len(),
                if paths.len() > 3 { " 等" } else { "" }
            ),
            error: String::new(),
            scroll: 0.0,
            hover: None,
            focus: Focus::Tree,
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.folders
            .iter()
            .position(|f| Some(&f.path) == self.selected.as_ref())
    }

    pub fn destination_label(&self) -> String {
        let Some(selected) = self.selected.as_ref() else {
            return "尚未选择目录".into();
        };
        let root = self
            .folders
            .iter()
            .filter(|folder| folder.depth == 0 && selected.starts_with(&folder.path))
            .max_by_key(|folder| folder.path.as_os_str().len());
        if let Some(root) = root {
            let relative = selected.strip_prefix(&root.path).unwrap();
            if relative.as_os_str().is_empty() {
                return root.name.clone();
            }
            return format!(
                "{} / {}",
                root.name,
                relative.to_string_lossy().replace('\\', " / ")
            );
        }
        selected.display().to_string()
    }

    pub fn select(&mut self, index: usize, viewport: Rect) {
        if let Some(folder) = self.folders.get(index) {
            self.selected = Some(folder.path.clone());
            self.error.clear();
            self.focus = Focus::Tree;
            let layout = self.layout(viewport);
            let top = index as f32 * ROW_HEIGHT;
            if top < self.scroll {
                self.scroll = top;
            }
            if top + ROW_HEIGHT > self.scroll + layout.tree.height() {
                self.scroll = top + ROW_HEIGHT - layout.tree.height();
            }
            self.scroll = self.scroll.clamp(0.0, layout.max_scroll);
        }
    }

    pub fn collapse(&mut self, index: usize, viewport: Rect) {
        let depth = self.folders[index].depth;
        let end = self
            .folders
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, folder)| folder.depth <= depth)
            .map_or(self.folders.len(), |(i, _)| i);
        if self.selected_index().is_some_and(|i| i > index && i < end) {
            self.selected = Some(self.folders[index].path.clone());
        }
        self.folders.drain(index + 1..end);
        self.folders[index].expanded = false;
        self.scroll = self.scroll.min(self.layout(viewport).max_scroll);
        self.hover = None;
    }

    pub fn layout(&self, viewport: Rect) -> Layout {
        let title_extra = large_title_extra();
        let width = (viewport.width() - 32.0).clamp(0.0, 600.0);
        let height = (viewport.height() - 32.0).clamp(0.0, 620.0 + title_extra);
        let panel = Rect::from_size(
            viewport.left + (viewport.width() - width) / 2.0,
            viewport.top + (viewport.height() - height) / 2.0,
            width,
            height,
        );
        let tree = Rect::new(
            panel.left + 24.0,
            panel.top + 116.0 + title_extra,
            panel.right - 24.0,
            (panel.bottom - 146.0).max(panel.top + 116.0 + title_extra),
        );
        Layout {
            panel,
            tree,
            close: Rect::from_size(panel.right - 52.0, panel.top + 16.0, 32.0, 32.0),
            cancel: Rect::from_size(panel.right - 244.0, panel.bottom - 58.0, 88.0, 34.0),
            confirm: Rect::from_size(panel.right - 144.0, panel.bottom - 58.0, 120.0, 34.0),
            max_scroll: (self.folders.len() as f32 * ROW_HEIGHT - tree.height()).max(0.0),
        }
    }

    pub fn row_rect(&self, layout: &Layout, index: usize) -> Rect {
        Rect::from_size(
            layout.tree.left,
            layout.tree.top + index as f32 * ROW_HEIGHT - self.scroll.clamp(0.0, layout.max_scroll),
            layout.tree.width(),
            ROW_HEIGHT,
        )
    }

    pub fn toggle_rect(&self, layout: &Layout, index: usize) -> Rect {
        let row = self.row_rect(layout, index);
        // 即使窗口较窄，也要让层级很深的文件夹名称保持可读。
        let indent = (self.folders[index].depth as f32 * 18.0).min(row.width() * 0.4);
        Rect::from_size(row.left + 6.0 + indent, row.top + 6.0, 20.0, 20.0)
    }

    pub fn hit(&self, viewport: Rect, x: f32, y: f32) -> Hit {
        let layout = self.layout(viewport);
        if !layout.panel.contains(x, y) {
            return Hit::Outside;
        }
        if layout.close.contains(x, y) || layout.cancel.contains(x, y) {
            return Hit::Cancel;
        }
        if layout.confirm.contains(x, y) && self.selected.is_some() {
            return Hit::Confirm;
        }
        if layout.tree.contains(x, y) {
            let index = ((y - layout.tree.top + self.scroll.clamp(0.0, layout.max_scroll))
                / ROW_HEIGHT) as usize;
            if let Some(folder) = self.folders.get(index) {
                if folder.expandable && self.toggle_rect(&layout, index).contains(x, y) {
                    return Hit::Toggle(index);
                }
                return Hit::Select(index);
            }
        }
        Hit::Inside
    }

    pub fn key(&mut self, key: u16, shift: bool, viewport: Rect) -> Option<Hit> {
        if key == 0x1b {
            return Some(Hit::Cancel);
        }
        if key == 0x09 {
            self.focus = match (self.focus, shift) {
                (Focus::Tree, false) | (Focus::Confirm, true) => Focus::Cancel,
                (Focus::Cancel, false) | (Focus::Tree, true) => Focus::Confirm,
                _ => Focus::Tree,
            };
            return None;
        }
        if matches!(key, 0x0d | 0x20) && self.focus != Focus::Tree {
            return Some(if self.focus == Focus::Cancel {
                Hit::Cancel
            } else {
                Hit::Confirm
            });
        }
        if self.focus != Focus::Tree {
            return None;
        }
        let index = self.selected_index()?;
        match key {
            0x0d => return Some(Hit::Confirm),
            0x20 => return Some(Hit::Toggle(index)),
            0x26 => self.select(index.saturating_sub(1), viewport),
            0x28 => self.select((index + 1).min(self.folders.len() - 1), viewport),
            0x24 => self.select(0, viewport),
            0x23 => self.select(self.folders.len() - 1, viewport),
            0x21 | 0x22 => {
                let page = (self.layout(viewport).tree.height() / ROW_HEIGHT).max(1.0) as usize;
                let next = if key == 0x21 {
                    index.saturating_sub(page)
                } else {
                    (index + page).min(self.folders.len() - 1)
                };
                self.select(next, viewport);
            }
            0x27 if !self.folders[index].expanded => return Some(Hit::Toggle(index)),
            0x27 if index + 1 < self.folders.len()
                && self.folders[index + 1].depth > self.folders[index].depth =>
            {
                self.select(index + 1, viewport)
            }
            0x25 if self.folders[index].expanded => return Some(Hit::Toggle(index)),
            0x25 => {
                if let Some(parent) = (0..index)
                    .rev()
                    .find(|i| self.folders[*i].depth < self.folders[index].depth)
                {
                    self.select(parent, viewport);
                }
            }
            _ => {}
        }
        None
    }
}
