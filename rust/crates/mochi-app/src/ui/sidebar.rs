//! 行尺寸取工作区设置；绘制只接收 Row，不查询 Shell。

use std::path::{Path, PathBuf};

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};
use crate::shell::Row;

/// `fileTreeItemSize`。
pub fn row_height() -> f32 {
    setting_number("sidebar.fileTreeItemSize", 34.0, 20.0, 80.0)
        .max(TextStyle::Label.line_height())
        .max(TextStyle::Caption.line_height())
}
/// `fileTreeItemSpacing`。
#[cfg(test)]
pub const ROW_GAP: f32 = 3.0;
pub fn row_gap() -> f32 {
    setting_number("sidebar.fileTreeItemSpacing", 3.0, 0.0, 32.0)
}
/// `fileTreeIndent`。
#[cfg(test)]
pub const INDENT: f32 = 12.0;
fn indent() -> f32 {
    setting_number("sidebar.fileTreeIndent", 12.0, 0.0, 48.0)
}
fn horizontal_inset() -> f32 {
    setting_number("sidebar.fileTreeHorizontalInset", ROW_MARGIN, 0.0, 32.0)
}
fn icon_size() -> f32 {
    setting_number("sidebar.fileTreeIconSize", ICON, 10.0, 24.0)
}
fn content_gap() -> f32 {
    setting_number("sidebar.fileTreeContentGap", GAP, 0.0, 24.0)
}
fn row_radius() -> f32 {
    setting_number("sidebar.fileTreeItemRadius", ROW_RADIUS, 0.0, 20.0)
}
fn search_height() -> f32 {
    setting_number("sidebar.searchHeight", SEARCH_HEIGHT, 24.0, 56.0)
        .max(TextStyle::Label.line_height())
}
fn header_height() -> f32 {
    HEADER_HEIGHT - SEARCH_HEIGHT + search_height()
}
fn setting_number(key: &str, fallback: f32, min: f32, max: f32) -> f32 {
    let value = super::settings_values::number(key, fallback);
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}
fn show_file_icons() -> bool {
    super::settings_values::boolean("sidebar.showFileIcons", true)
}
/// 图标与 chevron：`round(34 * 0.46) = 16`。
const ICON: f32 = 16.0;
/// 行内间距 `gap-2`。
const GAP: f32 = 8.0;
/// 行的水平外边距 `mx-2`。
const ROW_MARGIN: f32 = 8.0;
/// 行圆角 `rounded-lg`。
const ROW_RADIUS: f32 = 8.0;
/// 默认头部高度；可调搜索框高度同时改变树内容区起点。
pub const HEADER_HEIGHT: f32 = 12.0 + 22.0 + 8.0 + 34.0 + 8.0 + 12.0 + 1.0;
const SEARCH_HEIGHT: f32 = 34.0;

/// 正在进行的内联编辑（新建文件/文件夹、重命名）。
#[derive(Debug, Clone)]
pub struct Editing {
    pub kind: EditKind,
    pub field: TextField,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKind {
    /// 在 `parent` 下新建文件。`parent` 是根时输入框出现在列表最顶上。
    NewFile {
        parent: PathBuf,
    },
    NewFolder {
        parent: PathBuf,
    },
    /// 重命名第 `row` 行（下标指向 [`SidebarModel::rows`]）。
    Rename {
        row: usize,
    },
}

impl Editing {
    pub fn new(kind: EditKind, initial: &str) -> Self {
        let mut field = TextField::new(match kind {
            EditKind::NewFolder { .. } => "文件夹名",
            _ => "文件名",
        })
        .with_text(initial);
        field.style = TextStyle::Label;
        // 重命名文件时只选中扩展名之前的部分（TSX 的 setSelectionRange(0, ext)）
        if let Some(ext) = initial.rfind('.').filter(|i| *i > 0) {
            if matches!(kind, EditKind::Rename { .. }) {
                field.buffer.set_cursor(0, false);
                field.buffer.set_cursor(ext, true);
            }
        } else {
            field.select_all();
        }
        Editing {
            kind,
            field,
            error: String::new(),
        }
    }

    fn is_folder(&self) -> bool {
        matches!(self.kind, EditKind::NewFolder { .. })
    }
}

pub fn default_document_extension() -> &'static str {
    if super::settings_values::boolean("editor.simpleDocumentMode", true) {
        ".md"
    } else {
        ".mc"
    }
}

/// 新文档未指定扩展名时，使用当前文档模式的默认格式。
pub fn finalize_name(raw: &str, is_folder: bool) -> Result<String, &'static str> {
    finalize_name_with_extension(raw, is_folder, default_document_extension())
}

/// 重命名由调用方传入原扩展名，避免新建偏好改变已有文件的格式。
pub fn finalize_name_with_extension(
    raw: &str,
    is_folder: bool,
    default_extension: &str,
) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("名称不能为空");
    }
    if trimmed
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
    {
        return Err("名称包含非法字符");
    }
    let has_extension = trimmed
        .rsplit_once('.')
        .map(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
        .unwrap_or(false);
    Ok(if !is_folder && !has_extension {
        format!("{trimmed}{default_extension}")
    } else {
        trimmed.to_owned()
    })
}

/// 侧栏要画什么，全部由外部投影进来。
pub struct SidebarModel<'a> {
    pub icons: Option<&'a std::collections::HashMap<std::path::PathBuf, String>>,
    /// 头部标题：当前库名（或类型名）。
    pub title: &'a str,
    pub rows: &'a [Row],
    pub loading_paths: Vec<&'a Path>,
    /// 当前激活标签的文件路径——那一行加粗、底色 surface-muted。
    pub active_path: Option<&'a Path>,
    /// 选中状态与编辑器当前打开的文档无关。
    pub selected_row: Option<usize>,
    pub hover_row: Option<usize>,
    pub search: &'a TextField,
    pub search_focused: bool,
    pub all_expanded: bool,
    pub editing: Option<&'a Editing>,
    /// 正在拖放时的预览落点。实际移动由 App 在松开鼠标后执行。
    pub drop_indicator: Option<TreeDropIndicator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeDropIndicator {
    Before(usize),
    After(usize),
    Into(usize),
}

/// 点到了侧栏的什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarHit {
    /// 目录选择器（标题）。
    Title,
    ExpandAll,
    Refresh,
    Search,
    SearchClear,
    /// 某一行（下标指向 `rows`）。
    Row(usize),
    /// 某一行的 chevron。
    RowChevron(usize),
    /// 内联编辑框。
    Editor,
    /// 列表空白处。
    Blank,
}

/// 一次布局。绘制和命中测试共用。
#[derive(Debug, Clone, Default)]
pub struct SidebarLayout {
    pub entries: Vec<(Rect, SidebarHit)>,
    /// 文件树的可滚动区。
    pub content: Rect,
    pub content_height: f32,
    /// 每一行的 (行下标, 层级, 该层是否还有下一个同级) ——画连接线要它。
    rows: Vec<RowPlacement>,
    editor_rect: Option<(Rect, usize)>,
}

#[derive(Debug, Clone)]
struct RowPlacement {
    index: usize,
    path: PathBuf,
    rect: Rect,
    /// 每个祖先层级是否还有后续同级（`ancestorHasNext`），最后一项是本行自己。
    has_next: Vec<bool>,
}

impl SidebarLayout {
    pub fn hit(&self, x: f32, y: f32) -> Option<SidebarHit> {
        if let Some((_, h)) = self.entries.iter().rev().find(|(r, h)| {
            r.contains(x, y)
                && (!matches!(
                    h,
                    SidebarHit::Row(_) | SidebarHit::RowChevron(_) | SidebarHit::Editor
                ) || self.content.contains(x, y))
        }) {
            return Some(*h);
        }
        if self.content.contains(x, y) {
            return Some(SidebarHit::Blank);
        }
        None
    }

    pub fn rect_of(&self, hit: SidebarHit) -> Option<Rect> {
        self.entries
            .iter()
            .find(|(_, h)| *h == hit)
            .map(|(r, _)| *r)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.content.height()).max(0.0)
    }

    /// 路径取自实际绘制的画面，因此指针事件之间即使监视器重建了树，
    /// 同一行索引也不会突然指向另一个文件。
    pub fn row_path(&self, index: usize) -> Option<&Path> {
        self.rows
            .iter()
            .find(|row| row.index == index)
            .map(|row| row.path.as_path())
    }

    pub fn drop_row(&self, x: f32, y: f32) -> Option<(usize, Rect)> {
        if !self.content.contains(x, y) {
            return None;
        }
        self.rows
            .iter()
            .find(|row| {
                y >= row.rect.top - row_gap() / 2.0 && y <= row.rect.bottom + row_gap() / 2.0
            })
            .map(|row| (row.index, row.rect))
    }

    /// 内联编辑框里文字的起点 x（IME 定位与点击定位要用）。
    pub fn editor_text_left(&self) -> Option<f32> {
        self.editor_rect.map(|(r, level)| {
            r.left
                + 8.0
                + level as f32 * indent()
                + icon_size()
                + content_gap()
                + icon_size()
                + content_gap()
                + 4.0
        })
    }
}

/// 搜索过滤：匹配文件和所有祖先目录都保留，树的深度与连接线保持一致。
pub fn visible_rows(rows: &[Row], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    let hide = super::settings_values::boolean("sidebar.hideDotFiles", false);
    let hidden_patterns = super::settings_values::text(
        "sidebar.hiddenTreePatterns",
        "*.blocks-backup,*.blocks-restore-backup,*.backup,assets,图片,.annotations,*.documents",
    );
    let hidden_patterns: Vec<String> = hidden_patterns
        .split(',')
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut hidden_depth = None;
    let shown: Vec<bool> = rows
        .iter()
        .map(|r| {
            if hidden_depth.is_some_and(|d| r.depth <= d) {
                hidden_depth = None;
            }
            let lower_name = r.name.to_lowercase();
            let hidden_by_pattern = hidden_patterns
                .iter()
                .any(|pattern| wildcard_matches(pattern, &lower_name));
            if (hide && r.name.starts_with('.')) || hidden_by_pattern {
                if r.is_dir {
                    hidden_depth = Some(r.depth);
                }
                return false;
            }
            hidden_depth.is_none()
        })
        .collect();
    let matches = |r: &Row| r.name.to_lowercase().contains(&q);
    (0..rows.len())
        .filter(|&i| {
            if !shown[i] {
                return false;
            }
            if q.is_empty() {
                return true;
            }
            let r = &rows[i];
            if matches(r) {
                return true;
            }
            if !r.has_children {
                return false;
            }
            // 深层文件命中时也保留全部祖先；缺失祖先会破坏绘制时的层级栈。
            rows[i + 1..]
                .iter()
                .enumerate()
                .take_while(|(_, c)| c.depth > r.depth)
                .any(|(offset, c)| shown[i + 1 + offset] && matches(c))
        })
        .collect()
}

/// 轻量名称通配：`*` 匹配任意字符，规则大小写不敏感（调用方已转小写）。
/// 不使用正则，避免把用户填写的 `[`、`\\` 等普通文件名变成无效规则。
fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let mut rest = name;
    let mut first = true;
    for part in pattern.split('*') {
        if part.is_empty() {
            continue;
        }
        let Some(index) = rest.find(part) else {
            return false;
        };
        if first && !pattern.starts_with('*') && index != 0 {
            return false;
        }
        rest = &rest[index + part.len()..];
        first = false;
    }
    pattern.ends_with('*') || rest.is_empty()
}

/// 反向遍历一次即可替代对每一行逐个扫描后代项。搜索结果和隐藏
/// 文件夹使用与布局相同的筛选顺序，因此引导线会在正确位置结束。
fn next_siblings(rows: &[Row], visible: &[usize]) -> Vec<bool> {
    let mut next = vec![false; visible.len()];
    let mut stack: Vec<usize> = Vec::new();
    for position in (0..visible.len()).rev() {
        let depth = rows[visible[position]].depth;
        while let Some(&other) = stack.last() {
            if rows[visible[other]].depth < depth {
                break;
            }
            stack.pop();
            if rows[visible[other]].depth == depth {
                next[position] = true;
            }
        }
        stack.push(position);
    }
    next
}

pub fn layout(model: &SidebarModel, area: Rect, scroll: f32) -> SidebarLayout {
    let mut out = SidebarLayout::default();
    if area.is_empty() {
        return out;
    }
    // ── 头部 ──
    let top = area.top + 12.0;
    let right = area.right - 12.0;
    let left = area.left + 12.0;
    out.entries.push((
        Rect::new(right - 22.0, top, right, top + 22.0),
        SidebarHit::Refresh,
    ));
    out.entries.push((
        Rect::new(
            right - 22.0 - 4.0 - 22.0,
            top,
            right - 22.0 - 4.0,
            top + 22.0,
        ),
        SidebarHit::ExpandAll,
    ));
    out.entries.push((
        Rect::new(left, top, right - 48.0 - 4.0, top + 22.0),
        SidebarHit::Title,
    ));
    let search_top = top + 22.0 + 8.0;
    let search = Rect::new(left, search_top, right, search_top + search_height());
    out.entries.push((search, SidebarHit::Search));
    if !model.search.is_empty() {
        out.entries.push((
            Rect::new(
                search.right - 8.0 - 16.0,
                search.top + 9.0,
                search.right - 8.0,
                search.bottom - 9.0,
            ),
            SidebarHit::SearchClear,
        ));
    }

    // ── 文件树 ──
    let content = Rect::new(
        area.left,
        area.top + header_height(),
        area.right,
        area.bottom,
    );
    out.content = content;
    let visible = visible_rows(model.rows, model.search.text());
    let siblings = next_siblings(model.rows, &visible);
    let height = row_height().max(1.0);
    let gap = row_gap().max(0.0);
    let level_indent = indent();
    let in_view = |r: Rect| r.bottom > content.top && r.top < content.bottom;
    let mut y = content.top - scroll + setting_number("sidebar.paddingTop", 0.0, 0.0, 80.0);
    let row_margin = horizontal_inset();
    let row_left = content.left + row_margin;
    let row_right = content.right - row_margin;

    // 根级新建：输入框在最顶上
    if let Some(e) = model.editing {
        if let EditKind::NewFile { parent } | EditKind::NewFolder { parent } = &e.kind {
            if parent.as_os_str().is_empty() {
                let r = Rect::new(row_left, y, row_right, y + height);
                if in_view(r) {
                    out.entries.push((r, SidebarHit::Editor));
                    out.editor_rect = Some((r, 0));
                }
                y += height + gap;
            }
        }
    }

    // 祖先栈：每个已进入的层级「是否还有下一个同级」
    let mut ancestors: Vec<bool> = Vec::new();
    for (position, &i) in visible.iter().enumerate() {
        let row = &model.rows[i];
        ancestors.truncate(row.depth);
        let mine = siblings[position];

        let renaming = matches!(model.editing, Some(Editing { kind: EditKind::Rename { row: r }, .. }) if *r == i);
        let r = Rect::new(row_left, y, row_right, y + height);
        if in_view(r) {
            if renaming {
                out.entries.push((r, SidebarHit::Editor));
                out.editor_rect = Some((r, row.depth));
            } else {
                out.entries.push((r, SidebarHit::Row(i)));
                if row.has_children {
                    let content_left = r.left + 8.0 + row.depth as f32 * level_indent;
                    out.entries.push((
                        Rect::new(content_left, r.top, content_left + icon_size(), r.bottom),
                        SidebarHit::RowChevron(i),
                    ));
                }
                let mut has_next = ancestors.clone();
                has_next.push(mine);
                out.rows.push(RowPlacement {
                    index: i,
                    path: row.path.clone(),
                    rect: r,
                    has_next,
                });
            }
        }
        y += height + gap;

        // 目录下新建：输入框紧跟目录行（不管它展开没展开——TSX 特意去掉了 isExpanded 检查）
        if let Some(e) = model.editing {
            if let EditKind::NewFile { parent } | EditKind::NewFolder { parent } = &e.kind {
                if row.is_dir && parent == &row.path {
                    let er = Rect::new(row_left, y, row_right, y + height);
                    if in_view(er) {
                        out.entries.push((er, SidebarHit::Editor));
                        out.editor_rect = Some((er, row.depth + 1));
                    }
                    y += height + gap;
                }
            }
        }
        if row.has_children {
            ancestors.push(mine);
        }
    }
    out.content_height =
        y + scroll - content.top + setting_number("sidebar.paddingBottom", 0.0, 0.0, 80.0);

    out
}

/// 文件名去掉扩展名（`showFileExtensions: false`）。目录永远显示全名。
pub fn display_name(row: &Row) -> String {
    if super::settings_values::boolean("sidebar.showFileExtensions", false) {
        return row.name.clone();
    }
    if row.is_dir {
        return row.name.clone();
    }
    match row.name.rfind('.') {
        Some(i) if i > 0 => row.name[..i].to_owned(),
        _ => row.name.clone(),
    }
}

pub fn paint(
    list: &mut DrawList,
    area: Rect,
    model: &SidebarModel,
    lay: &SidebarLayout,
    p: &Palette,
) {
    paint_custom(list, area, model, lay, p, None);
}
pub struct Appearance<'a> {
    pub row_palette: &'a dyn Fn(&Row) -> (Palette, Option<u32>),
    pub show_icons: bool,
    pub show_extensions: bool,
}
pub fn paint_custom(
    list: &mut DrawList,
    area: Rect,
    model: &SidebarModel,
    lay: &SidebarLayout,
    p: &Palette,
    appearance: Option<&Appearance<'_>>,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);

    // ── 头部 ──
    let header = Rect::new(area.left, area.top, area.right, area.top + header_height());
    list.border_bottom(header, p.border);
    if let Some(r) = lay.rect_of(SidebarHit::Title) {
        // DirectoryPickerMenu：目录名 + 小箭头
        let label = text::ellipsize(model.title, TextStyle::Label, (r.width() - 20.0).max(10.0));
        let w = text::measure(&label, TextStyle::Label);
        list.text(r, label, TextStyle::Label, p.muted);
        list.icon_centered(
            Rect::new(r.left + w + 4.0, r.top, r.left + w + 18.0, r.bottom),
            Icon::CHEVRON_DOWN,
            14.0,
            p.muted,
        );
    }
    if let Some(r) = lay.rect_of(SidebarHit::ExpandAll) {
        let icon = if model.all_expanded {
            Icon::CHEVRONS_UP_DOWN
        } else {
            Icon::CHEVRONS_DOWN_UP
        };
        list.icon_centered(r, icon, 14.0, p.muted);
    }
    if let Some(r) = lay.rect_of(SidebarHit::Refresh) {
        list.icon_centered(r, Icon::ROTATE_CW, 14.0, p.muted);
    }
    if let Some(r) = lay.rect_of(SidebarHit::Search) {
        let mut f = model.search.clone();
        f.style = TextStyle::Label;
        f.paint(list, r, model.search_focused, p, FieldLook::search(p));
        if let Some(x) = lay.rect_of(SidebarHit::SearchClear) {
            list.icon_centered(x, Icon::X, 12.0, p.muted);
        }
    }

    // ── 文件树 ──
    list.push_clip(lay.content);
    for placement in &lay.rows {
        let row = &model.rows[placement.index];
        let r = placement.rect;
        let (row_palette, background) =
            appearance.map_or_else(|| (p.clone(), None), |a| (a.row_palette)(row));
        let p = &row_palette;
        if let Some(bg) = background {
            list.rounded_rect(r, row_radius(), bg);
        }
        let display_icons = appearance.map_or_else(show_file_icons, |a| a.show_icons);
        let is_active = model.active_path.map(|a| a == row.path).unwrap_or(false);
        let is_hover = model.hover_row == Some(placement.index);
        if is_active || is_hover || model.selected_row == Some(placement.index) {
            list.rounded_rect(r, row_radius(), p.surface_muted);
        }

        let level = row.depth;
        let content_left = r.left + 8.0 + level as f32 * indent();

        // 连接线（showFileTreeGuides）：祖先的竖线、本层的竖线与横线
        if level > 0 && super::settings_values::boolean("sidebar.showTreeGuides", true) {
            let guide = super::guides::Style::read(true, p.border);
            for (a, has_next) in placement.has_next[..level].iter().enumerate() {
                if *has_next {
                    let x = r.left + 8.0 + (a as f32 + 1.0) * indent() - indent() / 2.0;
                    guide.line(list, x, r.top, r.bottom, true);
                }
            }
            let branch_x = r.left + 8.0 + level as f32 * indent() - indent() / 2.0;
            let mid = (r.top + r.bottom) / 2.0;
            let bottom = if placement.has_next[level] {
                r.bottom
            } else {
                mid
            };
            guide.line(list, branch_x, r.top, bottom, true);
            let expandable = row.has_children;
            let target = if expandable {
                content_left
            } else if display_icons {
                content_left + icon_size() + content_gap()
            } else {
                content_left
            };
            let end_gap = if expandable { guide.gap } else { 0.0 };
            let width = (target - branch_x - end_gap).max(0.0);
            guide.line(list, branch_x, mid, branch_x + width, false);
        }

        // chevron（可展开才画，否则留同宽空位）
        if row.has_children {
            let icon = if row.expanded {
                Icon::CHEVRON_DOWN
            } else {
                Icon::CHEVRON_RIGHT
            };
            list.icon_centered(
                Rect::new(content_left, r.top, content_left + icon_size(), r.bottom),
                icon,
                icon_size(),
                p.muted,
            );
        }
        let icon_left = content_left + icon_size() + content_gap();
        let (icon, color) = if row.is_dir {
            (
                if row.expanded {
                    Icon::FOLDER_OPEN
                } else {
                    Icon::FOLDER
                },
                p.muted,
            )
        } else if row.name.ends_with(".link.json") {
            (Icon::LINK2, p.accent)
        } else if row
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mcb"))
        {
            (Icon::TABLE, p.accent)
        } else {
            (Icon::FILE, p.muted)
        };
        if display_icons {
            let icon_rect = Rect::new(icon_left, r.top, icon_left + icon_size(), r.bottom);
            if let Some(custom) = model.icons.and_then(|icons| icons.get(&row.path)) {
                if let Some(named) = super::icons::named(custom) {
                    list.icon_centered(icon_rect, named, icon_size(), color);
                } else {
                    list.text_aligned(icon_rect, custom, TextStyle::Body, color, Align::Center);
                }
            } else {
                list.icon_centered(icon_rect, icon, icon_size(), color);
            }
        }

        // 名字。激活文件加粗（font-semibold）
        let text_left = if display_icons {
            icon_left + icon_size() + content_gap()
        } else {
            icon_left
        };
        let mut text_right = r.right - 8.0;
        if row.expanded && model.loading_paths.contains(&row.path.as_path()) {
            let label = "正在加载…";
            let width = text::measure(label, TextStyle::Caption);
            list.text_aligned(
                Rect::new(text_right - width, r.top, text_right, r.bottom),
                label,
                TextStyle::Caption,
                p.muted,
                Align::Trailing,
            );
            text_right -= width + content_gap();
        } else if row.has_children && is_hover {
            // 悬停时右侧显示子项数（group-hover:opacity-100）
            let count = child_count(model.rows, placement.index).to_string();
            let w = text::measure(&count, TextStyle::Caption);
            list.text_aligned(
                Rect::new(text_right - w, r.top, text_right, r.bottom),
                count,
                TextStyle::Caption,
                p.muted,
                Align::Trailing,
            );
            text_right -= w + content_gap();
        }
        let name = text::ellipsize(
            &appearance.map_or_else(
                || display_name(row),
                |a| {
                    if a.show_extensions || row.is_dir {
                        row.name.clone()
                    } else {
                        Path::new(&row.name)
                            .file_stem()
                            .map_or_else(|| row.name.clone(), |s| s.to_string_lossy().into_owned())
                    }
                },
            ),
            TextStyle::Label,
            (text_right - text_left).max(10.0),
        );
        let emphasis = if is_active {
            Emphasis::Bold
        } else {
            Emphasis::None
        };
        paint_highlighted(
            list,
            Rect::new(text_left, r.top, text_right, r.bottom),
            &name,
            model.search.text(),
            emphasis,
            p,
        );
    }

    // 拖放反馈只覆盖目标行：插入同级时是一条细线，放入目录时高亮整行。
    // 这样不会把「移动到文件夹」误读成「与文件夹同级排列」。
    if let Some(indicator) = model.drop_indicator {
        let (index, into, after) = match indicator {
            TreeDropIndicator::Before(index) => (index, false, false),
            TreeDropIndicator::After(index) => (index, false, true),
            TreeDropIndicator::Into(index) => (index, true, false),
        };
        if let Some(r) = lay.rect_of(SidebarHit::Row(index)) {
            if into {
                list.rounded_border(r, row_radius(), p.accent);
            } else {
                let y = if after { r.bottom } else { r.top };
                list.rect(
                    Rect::new(r.left + 4.0, y - 1.0, r.right - 4.0, y + 1.0),
                    p.accent,
                );
            }
        }
    }

    // 内联编辑框
    if let (Some(e), Some((r, level))) = (model.editing, lay.editor_rect) {
        list.rect(r, p.background);
        let content_left = r.left + 8.0 + level as f32 * indent();
        let icon_left = content_left + icon_size() + content_gap();
        if show_file_icons() {
            let icon = if e.is_folder()
                || matches!(e.kind, EditKind::Rename { row } if model.rows.get(row).map(|x| x.is_dir).unwrap_or(false))
            {
                Icon::FOLDER
            } else {
                Icon::FILE
            };
            list.icon_centered(
                Rect::new(icon_left, r.top, icon_left + icon_size(), r.bottom),
                icon,
                icon_size(),
                p.muted,
            );
        }
        let field_rect = Rect::new(
            if show_file_icons() {
                icon_left + icon_size() + content_gap()
            } else {
                icon_left
            },
            r.top + 4.0,
            r.right - 8.0,
            r.bottom - 4.0,
        );
        let mut f = e.field.clone();
        f.paint(list, field_rect, true, p, FieldLook::inline_accent(p));
        if !e.error.is_empty() {
            list.text(
                Rect::new(field_rect.left, r.bottom, r.right, r.bottom + 16.0),
                e.error.clone(),
                TextStyle::Caption,
                p.danger,
            );
        }
    }
    list.pop_clip();
    list.pop_clip();
}

/// 直接子项数（TSX 的 `node.children.length`）。
fn child_count(rows: &[Row], i: usize) -> usize {
    let depth = rows[i].depth;
    rows[i + 1..]
        .iter()
        .take_while(|r| r.depth > depth)
        .filter(|r| r.depth == depth + 1)
        .count()
}

/// 把名字里匹配搜索词的部分画上 accent/20 的底（`<mark>`）。
fn paint_highlighted(
    list: &mut DrawList,
    rect: Rect,
    name: &str,
    query: &str,
    emphasis: Emphasis,
    p: &Palette,
) {
    let style = TextStyle::Label;
    if query.is_empty() {
        list.text_run(rect, name, style, p.foreground, Align::Leading, emphasis);
        return;
    }
    let lower = name.to_lowercase();
    let q = query.to_lowercase();
    let mut x = rect.left;
    let mut pos = 0;
    // 大小写不敏感匹配后按原文切片：小写化可能改变字节长度，这里按字符走
    let name_chars: Vec<(usize, char)> = name.char_indices().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let q_chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    let mut segments: Vec<(usize, usize, bool)> = Vec::new();
    while i < lower_chars.len() {
        if lower_chars[i..].starts_with(&q_chars) && name_chars.len() == lower_chars.len() {
            let start = name_chars[i].0;
            let end = name_chars
                .get(i + q_chars.len())
                .map(|(b, _)| *b)
                .unwrap_or(name.len());
            if start > pos {
                segments.push((pos, start, false));
            }
            segments.push((start, end, true));
            pos = end;
            i += q_chars.len();
        } else {
            i += 1;
        }
    }
    if pos < name.len() {
        segments.push((pos, name.len(), false));
    }
    let pad = (rect.height() - style.line_height()) / 2.0;
    for (a, b, hit) in segments {
        let piece = &name[a..b];
        let w = text::measure(piece, style);
        if hit {
            list.rect(
                Rect::new(x, rect.top + pad, x + w, rect.bottom - pad),
                theme::mix(p.accent, p.surface, 0.20),
            );
        }
        list.text_run(
            Rect::new(x, rect.top, x + w + 2.0, rect.bottom),
            piece,
            style,
            p.foreground,
            Align::Leading,
            emphasis,
        );
        x += w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;
    #[path = "sidebar_performance_tests.rs"]
    mod performance;

    fn row(name: &str, depth: usize, is_dir: bool, expanded: bool) -> Row {
        Row {
            name: name.to_owned(),
            path: PathBuf::from(name),
            depth,
            is_dir,
            is_mapped_folder: false,
            expanded,
            has_children: is_dir,
        }
    }

    fn tree() -> Vec<Row> {
        vec![
            row("网络", 0, true, true),
            row("TCP.md", 1, false, false),
            row("UDP.md", 1, false, false),
            row("操作系统.md", 0, false, false),
            row("索引.md", 0, false, false),
        ]
    }

    const AREA: Rect = Rect {
        left: 220.0,
        top: 32.0,
        right: 480.0,
        bottom: 800.0,
    };

    fn model<'a>(
        rows: &'a [Row],
        search: &'a TextField,
        editing: Option<&'a Editing>,
    ) -> SidebarModel<'a> {
        SidebarModel {
            icons: None,
            title: "计算机通识",
            rows,
            loading_paths: Vec::new(),
            active_path: None,
            selected_row: None,
            hover_row: None,
            search,
            search_focused: false,
            all_expanded: false,
            editing,
            drop_indicator: None,
        }
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn partially_scrolled_tree_row_cannot_capture_header_clicks() {
        let rows = tree();
        let search = TextField::new("");
        let lay = layout(&model(&rows, &search, None), AREA, 20.0);
        let top = lay.rect_of(SidebarHit::Row(0)).unwrap();
        assert!(top.top < lay.content.top);
        assert!(!matches!(
            lay.hit(top.left + 90.0, lay.content.top - 1.0),
            Some(SidebarHit::Row(_) | SidebarHit::RowChevron(_))
        ));
    }

    #[test]
    fn rows_are_34_tall_with_a_3px_gap_below_the_97px_header() {
        let rows = tree();
        let search = TextField::new("");
        let lay = layout(&model(&rows, &search, None), AREA, 0.0);
        let r0 = lay.rect_of(SidebarHit::Row(0)).unwrap();
        let r1 = lay.rect_of(SidebarHit::Row(1)).unwrap();
        assert_eq!(r0.top, AREA.top + HEADER_HEIGHT);
        assert_eq!(HEADER_HEIGHT, 97.0);
        assert_eq!(r0.height(), 34.0);
        assert_eq!(r1.top - r0.bottom, 3.0);
        // mx-2
        assert_eq!(r0.left, AREA.left + 8.0);
        assert_eq!(r0.right, AREA.right - 8.0);
    }

    #[test]
    fn the_chevron_hit_zone_is_indented_by_level_and_only_exists_for_expandable_rows() {
        let rows = tree();
        let search = TextField::new("");
        let lay = layout(&model(&rows, &search, None), AREA, 0.0);
        let c0 = lay.rect_of(SidebarHit::RowChevron(0)).unwrap();
        assert_eq!(c0.left, AREA.left + 8.0 + 8.0);
        assert_eq!(c0.width(), 16.0);
        assert!(
            lay.rect_of(SidebarHit::RowChevron(1)).is_none(),
            "文件没有 chevron"
        );
        // 点 chevron 命中 RowChevron 而不是 Row
        assert_eq!(
            lay.hit(c0.left + 4.0, c0.top + 4.0),
            Some(SidebarHit::RowChevron(0))
        );
        assert_eq!(
            lay.hit(c0.right + 40.0, c0.top + 4.0),
            Some(SidebarHit::Row(0))
        );
    }

    #[test]
    fn file_extensions_are_hidden_but_directory_names_are_kept_whole() {
        assert_eq!(display_name(&row("TCP.md", 0, false, false)), "TCP");
        assert_eq!(display_name(&row("v1.2", 0, true, false)), "v1.2");
        assert_eq!(
            display_name(&row(".gitignore", 0, false, false)),
            ".gitignore",
            "点开头不算扩展名"
        );
    }

    #[test]
    fn tree_hidden_patterns_cover_internal_backups_and_literal_unicode_names() {
        assert!(wildcard_matches(
            "*.blocks-backup",
            "墨池-食用说明.mc.blocks-backup"
        ));
        assert!(wildcard_matches("*.backup", "草稿.md.backup"));
        assert!(wildcard_matches("图片", "图片"));
        assert!(!wildcard_matches("*.blocks-backup", "墨池-食用说明.mc"));
        assert!(!wildcard_matches("assets", "my-assets"));
        assert!(wildcard_matches("*.documents", "资料表.documents"));
    }

    #[test]
    fn table_temporary_document_folders_are_hidden_with_the_default_rules() {
        let rows = vec![
            row("资料表.documents", 0, true, true),
            row("临时内容.md", 1, false, false),
            row("资料表.mcb", 0, false, false),
        ];
        assert_eq!(visible_rows(&rows, ""), vec![2]);
    }

    #[test]
    fn the_active_file_is_bold_on_a_muted_surface_and_the_header_paints_its_tools() {
        let rows = tree();
        let search = TextField::new("搜索文件...");
        let mut m = model(&rows, &search, None);
        m.active_path = Some(Path::new("TCP.md"));
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &m, &lay, &p);

        let active_rect = lay.rect_of(SidebarHit::Row(1)).unwrap();
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::RoundedRect { rect, color, .. } if *rect == active_rect && *color == p.surface_muted)));
        assert!(list.cmds().iter().any(
            |c| matches!(c, DrawCmd::Text { text, emphasis: Emphasis::Bold, .. } if text == "TCP")
        ));
        assert!(list.cmds().iter().any(
            |c| matches!(c, DrawCmd::Text { text, emphasis: Emphasis::None, .. } if text == "UDP")
        ));
        let t = texts(&list);
        assert!(t.contains(&"计算机通识".to_owned()));
        assert!(t.contains(&"搜索文件...".to_owned()), "{t:?}");
        let icons: Vec<Icon> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Icon { icon, .. } => Some(*icon),
                _ => None,
            })
            .collect();
        assert!(
            icons.contains(&Icon::ROTATE_CW)
                && icons.contains(&Icon::CHEVRONS_DOWN_UP)
                && icons.contains(&Icon::SEARCH)
        );
        assert!(
            icons.contains(&Icon::FOLDER_OPEN),
            "展开的目录用 FolderOpen"
        );
        assert!(list.finish().is_ok());
    }

    #[test]
    fn guide_lines_are_drawn_for_nested_rows_only() {
        let rows = tree();
        let search = TextField::new("");
        let m = model(&rows, &search, None);
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &m, &lay, &p);
        let r1 = lay.rect_of(SidebarHit::Row(1)).unwrap();
        let branch_x = r1.left + 8.0 + INDENT - INDENT / 2.0;
        // TCP 还有下一个同级 UDP：竖线通到底
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Rect { rect, color } if *color == p.border && rect.left == branch_x && rect.top == r1.top && rect.bottom == r1.bottom)));
        let r2 = lay.rect_of(SidebarHit::Row(2)).unwrap();
        let mid = (r2.top + r2.bottom) / 2.0;
        // UDP 是最后一个：竖线只到一半
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Rect { rect, color } if *color == p.border && rect.left == branch_x && rect.top == r2.top && rect.bottom == mid)));
    }

    #[test]
    fn searching_keeps_matching_files_and_directories_with_a_matching_direct_child() {
        let rows = tree();
        assert_eq!(
            visible_rows(&rows, "tcp"),
            vec![0, 1],
            "网络 因为直接子项 TCP 匹配而保留"
        );
        assert_eq!(visible_rows(&rows, "索引"), vec![4]);
        assert_eq!(visible_rows(&rows, ""), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn searching_nested_documents_preserves_the_ancestor_chain_for_painting() {
        let rows = vec![
            row("知识库", 0, true, true),
            row("学习", 1, true, true),
            row("网络", 2, true, true),
            row("TCP.md", 3, false, false),
            row("UDP.md", 3, false, false),
            row("其他", 1, true, true),
            row("笔记.md", 2, false, false),
        ];
        assert_eq!(visible_rows(&rows, "tcp"), vec![0, 1, 2, 3]);
        let search = TextField::new("").with_text("tcp");
        let m = model(&rows, &search, None);
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        paint(&mut list, AREA, &m, &lay, theme::tokens().palette(false));
        assert!(lay.rect_of(SidebarHit::Row(3)).is_some());
        assert!(lay.rect_of(SidebarHit::Row(4)).is_none());
        assert!(!list.is_empty());
    }

    #[test]
    fn the_search_clear_button_appears_only_with_text() {
        let rows = tree();
        let empty = TextField::new("搜索文件...");
        assert!(layout(&model(&rows, &empty, None), AREA, 0.0)
            .rect_of(SidebarHit::SearchClear)
            .is_none());
        let typed = TextField::new("").with_text("tcp");
        assert!(layout(&model(&rows, &typed, None), AREA, 0.0)
            .rect_of(SidebarHit::SearchClear)
            .is_some());
    }

    #[test]
    fn a_new_file_editor_appears_right_after_its_parent_directory_one_level_deeper() {
        let rows = tree();
        let search = TextField::new("");
        let e = Editing::new(
            EditKind::NewFile {
                parent: PathBuf::from("网络"),
            },
            "",
        );
        let lay = layout(&model(&rows, &search, Some(&e)), AREA, 0.0);
        let dir = lay.rect_of(SidebarHit::Row(0)).unwrap();
        let editor = lay.rect_of(SidebarHit::Editor).unwrap();
        assert_eq!(editor.top, dir.bottom + ROW_GAP);
        // 后续行整体下移一行
        let tcp = lay.rect_of(SidebarHit::Row(1)).unwrap();
        assert_eq!(tcp.top, editor.bottom + ROW_GAP);
        assert_eq!(lay.editor_rect.unwrap().1, 1, "层级 = 父目录 + 1");
    }

    #[test]
    fn a_root_level_new_folder_editor_sits_at_the_very_top() {
        let rows = tree();
        let search = TextField::new("");
        let e = Editing::new(
            EditKind::NewFolder {
                parent: PathBuf::new(),
            },
            "",
        );
        let lay = layout(&model(&rows, &search, Some(&e)), AREA, 0.0);
        let editor = lay.rect_of(SidebarHit::Editor).unwrap();
        assert_eq!(editor.top, AREA.top + HEADER_HEIGHT);
        assert_eq!(
            lay.rect_of(SidebarHit::Row(0)).unwrap().top,
            editor.bottom + ROW_GAP
        );
    }

    #[test]
    fn renaming_replaces_the_row_with_the_editor_and_preselects_the_stem() {
        let rows = tree();
        let search = TextField::new("");
        let e = Editing::new(EditKind::Rename { row: 1 }, "TCP.md");
        assert_eq!(e.field.buffer.selection(), (0, 3), "只选中扩展名之前的部分");
        let lay = layout(&model(&rows, &search, Some(&e)), AREA, 0.0);
        assert!(lay.rect_of(SidebarHit::Row(1)).is_none());
        let editor = lay.rect_of(SidebarHit::Editor).unwrap();
        assert_eq!(
            editor.top,
            lay.rect_of(SidebarHit::Row(0)).unwrap().bottom + ROW_GAP
        );
    }

    #[test]
    fn document_names_respect_the_mode_and_explicit_extensions() {
        super::super::settings_values::reset();
        assert_eq!(finalize_name("笔记", false), Ok("笔记.md".to_owned()));
        assert_eq!(
            finalize_name_with_extension("笔记", false, ".mc"),
            Ok("笔记.mc".to_owned())
        );
        assert_eq!(finalize_name("笔记.md", false), Ok("笔记.md".to_owned()));
        assert_eq!(finalize_name("笔记.mc", false), Ok("笔记.mc".to_owned()));
        assert_eq!(finalize_name("数据.mcb", false), Ok("数据.mcb".to_owned()));
        assert_eq!(
            finalize_name_with_extension("无扩展名文件", false, ""),
            Ok("无扩展名文件".to_owned())
        );
        assert_eq!(finalize_name("文件夹", true), Ok("文件夹".to_owned()));
        assert_eq!(finalize_name("  ", false), Err("名称不能为空"));
        assert_eq!(finalize_name("a/b", false), Err("名称包含非法字符"));
    }

    #[test]
    fn rows_scrolled_out_of_the_content_area_are_not_hittable_and_blank_space_is() {
        let rows = tree();
        let search = TextField::new("");
        let short = Rect::new(220.0, 32.0, 480.0, 32.0 + HEADER_HEIGHT + 60.0);
        let lay = layout(&model(&rows, &search, None), short, 0.0);
        assert!(lay.rect_of(SidebarHit::Row(3)).is_none());
        assert!(lay.max_scroll() > 0.0);
        let tall = layout(&model(&rows, &search, None), AREA, 0.0);
        assert_eq!(tall.hit(300.0, 700.0), Some(SidebarHit::Blank));
        // 标题行在 p-3 之后：32 + 12 .. 32 + 34
        assert_eq!(tall.hit(300.0, 50.0), Some(SidebarHit::Title));
        assert_eq!(tall.hit(300.0, 40.0), None, "p-3 的内边距不属于任何控件");
    }

    #[test]
    fn a_zero_sized_panel_paints_nothing() {
        let rows = tree();
        let search = TextField::new("");
        let m = model(&rows, &search, None);
        let lay = layout(&m, Rect::ZERO, 0.0);
        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::ZERO,
            &m,
            &lay,
            theme::tokens().palette(false),
        );
        assert!(list.is_empty());
    }

    #[test]
    fn search_matches_are_marked_inside_the_name() {
        let rows = tree();
        let search = TextField::new("").with_text("tc");
        let m = model(&rows, &search, None);
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &m, &lay, &p);
        let mark = theme::mix(p.accent, p.surface, 0.20);
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == mark)));
        // 名字被切成「TC」+「P」两段画
        let t = texts(&list);
        assert!(
            t.contains(&"TC".to_owned()) && t.contains(&"P".to_owned()),
            "{t:?}"
        );
    }
}
