//! 命令与快速打开的排序规则保持与 CommandPalette.tsx、QuickOpen.tsx 一致。

use std::path::{Path, PathBuf};

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

pub const MAX_WIDTH: f32 = 672.0;
fn max_width() -> f32 {
    super::settings_values::number("command.maxWidth", MAX_WIDTH)
}
const RADIUS: f32 = 8.0;
fn radius() -> f32 {
    super::settings_values::number("command.radius", RADIUS)
}
const HEADER_H: f32 = 16.0 + 38.0 + 16.0 + 1.0; // p-4 + 输入框(py-2 + 20 + 边框) + p-4 + border-b
const FOOTER_H: f32 = 8.0 + 16.0 + 8.0 + 1.0;
const GROUP_H: f32 = 8.0 + 16.0 + 8.0;
const ROW_PAD_Y: f32 = 12.0;
fn row_pad_y() -> f32 {
    super::settings_values::number("command.rowPaddingY", ROW_PAD_Y)
}
const ROW_PAD_X: f32 = 16.0;
fn row_pad_x() -> f32 {
    super::settings_values::number("command.rowPaddingX", ROW_PAD_X)
}
const EMPTY_H: f32 = 32.0 + 20.0 + 32.0; // p-8

/// 命令面板里能执行的事。对应 `createDefaultCommands` 的 handlers。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAction {
    NewFile,
    NewFolder,
    NewLinkFile,
    NewBase,
    ToggleSidebar,
    ToggleSourceMode,
    SplitRight,
    CloseSplit,
    QuickOpen,
    GlobalSearch,
    ToggleAiPanel,
    VersionHistory,
    Settings,
}

pub struct Command {
    pub label: &'static str,
    pub description: Option<&'static str>,
    pub icon: Icon,
    pub shortcut: Option<&'static str>,
    pub category: &'static str,
    pub action: CommandAction,
}

/// 与 `createDefaultCommands` 同序同文案。
pub const COMMANDS: &[Command] = &[
    Command {
        label: "新建笔记",
        description: Some("创建一篇新的 Markdown 笔记"),
        icon: Icon::FILE_TEXT,
        shortcut: Some("Ctrl+N"),
        category: "文件",
        action: CommandAction::NewFile,
    },
    Command {
        label: "新建文件夹",
        description: Some("在当前位置创建文件夹"),
        icon: Icon::FOLDER_PLUS,
        shortcut: None,
        category: "文件",
        action: CommandAction::NewFolder,
    },
    Command {
        label: "新建链接文件",
        description: Some("创建一个指向外部资源的链接文件"),
        icon: Icon::LINK,
        shortcut: None,
        category: "文件",
        action: CommandAction::NewLinkFile,
    },
    Command {
        label: "新建多维表格",
        description: Some("创建包含字段、彩色标签和视图的数据文件"),
        icon: Icon::TABLE,
        shortcut: None,
        category: "文件",
        action: CommandAction::NewBase,
    },
    Command {
        label: "显示 / 隐藏侧边栏",
        description: None,
        icon: Icon::PANEL_LEFT_CLOSE,
        shortcut: None,
        category: "视图",
        action: CommandAction::ToggleSidebar,
    },
    Command {
        label: "切换源码模式",
        description: Some("在所见即所得与 Markdown 源码之间切换"),
        icon: Icon::CODE,
        shortcut: None,
        category: "视图",
        action: CommandAction::ToggleSourceMode,
    },
    Command {
        label: "向右拆分编辑器",
        description: Some("在右侧并排编辑当前文档"),
        icon: Icon::PANEL_LEFT_CLOSE,
        shortcut: None,
        category: "视图",
        action: CommandAction::SplitRight,
    },
    Command {
        label: "关闭右侧拆分",
        description: Some("保留左侧文档和所有已打开的标签"),
        icon: Icon::X,
        shortcut: None,
        category: "视图",
        action: CommandAction::CloseSplit,
    },
    Command {
        label: "快速打开文件",
        description: Some("按文件名快速跳转"),
        icon: Icon::SEARCH,
        shortcut: Some("Ctrl+P"),
        category: "导航",
        action: CommandAction::QuickOpen,
    },
    Command {
        label: "全局搜索",
        description: Some("搜索所有笔记的内容"),
        icon: Icon::SEARCH,
        shortcut: Some("Ctrl+Shift+F"),
        category: "导航",
        action: CommandAction::GlobalSearch,
    },
    Command {
        label: "显示 / 隐藏 AI 助手",
        description: None,
        icon: Icon::SPARKLES,
        shortcut: Some("Ctrl+J"),
        category: "AI",
        action: CommandAction::ToggleAiPanel,
    },
    Command {
        label: "打开版本历史",
        description: Some("查看当前文件的历史版本"),
        icon: Icon::HISTORY,
        shortcut: None,
        category: "版本控制",
        action: CommandAction::VersionHistory,
    },
    Command {
        label: "打开设置",
        description: Some("调整应用的偏好配置"),
        icon: Icon::SETTINGS,
        shortcut: None,
        category: "设置",
        action: CommandAction::Settings,
    },
];

/// 命令过滤：返回 `COMMANDS` 的下标，按分数降序（空查询保持原序）。
pub fn filter_commands(query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..COMMANDS.len()).collect();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(usize, i32)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let label = c.label.to_lowercase();
            let desc = c.description.unwrap_or("").to_lowercase();
            let cat = c.category.to_lowercase();
            let mut score = 0;
            for w in &words {
                if label.contains(w) {
                    score += if label.starts_with(w) { 10 } else { 5 };
                }
                if desc.contains(w) {
                    score += 2;
                }
                if cat.contains(w) {
                    score += 1;
                }
                if is_subsequence(w, &label) {
                    score += 3;
                }
            }
            (score > 0).then_some((i, score))
        })
        .collect();
    // 稳定排序：同分保持原序（JS 的 sort 也是稳定的）
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.into_iter().map(|(i, _)| i).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// 一个可快速打开的文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatFile {
    pub path: PathBuf,
    pub name: String,
    /// 相对工作区（或库）的 POSIX 路径。与文件名相同则不另起一行显示。
    pub relative: String,
    pub is_link: bool,
}

/// 遍历工作区收集文件。跳过隐藏目录与 `.mochi`、`.git`、`node_modules`。
pub fn collect_files(root: &Path) -> Vec<FlatFile> {
    let mut out = Vec::new();
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<FlatFile>, depth: usize) {
        if depth > 16 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if ft.is_dir() {
                walk(&e.path(), &rel, out, depth + 1);
            } else {
                let is_link = name.to_lowercase().ends_with(".mclink")
                    || name.to_lowercase().ends_with(".url");
                out.push(FlatFile {
                    path: e.path(),
                    name,
                    relative: rel,
                    is_link,
                });
            }
        }
    }
    walk(root, "", &mut out, 0);
    out
}

/// 文件过滤（QuickOpen 的打分）。空查询：`recent` 里前 10 个能在 `files` 里找到的（找不到就按路径临时造一个）。
pub fn filter_files(files: &[FlatFile], recent: &[PathBuf], query: &str) -> Vec<FlatFile> {
    if query.trim().is_empty() {
        return recent
            .iter()
            .take(10)
            .map(|p| {
                files
                    .iter()
                    .find(|f| &f.path == p)
                    .cloned()
                    .unwrap_or_else(|| FlatFile {
                        path: p.clone(),
                        name: p
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        relative: p.to_string_lossy().into_owned(),
                        is_link: false,
                    })
            })
            .collect();
    }
    let q = query.to_lowercase();
    let mut scored: Vec<(&FlatFile, i32)> = files
        .iter()
        .filter_map(|f| score_file(f, &q).map(|s| (f, s)))
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored
        .into_iter()
        .take(50)
        .map(|(f, _)| f.clone())
        .collect()
}

/// TSX 的逐字符打分。`indexOf(char, lastIndex + 1)` 在名字里找、找不到再到路径里找——
/// 两个字符串共用同一个 `lastIndex`，照抄（这是它的行为，不是笔误）。按字符而非字节。
fn score_file(f: &FlatFile, q: &str) -> Option<i32> {
    let name: Vec<char> = f.name.to_lowercase().chars().collect();
    let path: Vec<char> = f.relative.to_lowercase().chars().collect();
    let mut score = 0;
    let mut last: i64 = -1;
    let mut consecutive = 0;
    for c in q.chars() {
        let from = (last + 1) as usize;
        let find = |hay: &[char]| {
            hay.iter()
                .skip(from)
                .position(|h| *h == c)
                .map(|i| i + from)
        };
        let index = find(&name).or_else(|| find(&path))?;
        if index as i64 == last + 1 {
            consecutive += 1;
            score += 5 + consecutive;
        } else {
            consecutive = 0;
            score += 1;
        }
        if index == 0 || matches!(path.get(index - 1), Some('/') | Some('-')) {
            score += 10;
        }
        last = index as i64;
    }
    Some(score)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Commands,
    Files,
}

pub struct State {
    pub mode: Mode,
    pub query: TextField,
    pub selected: usize,
    pub scroll: f32,
    /// 快速打开：工作区全部文件（打开时扫一次）与最近打开（标签页路径）。
    pub files: Vec<FlatFile>,
    pub recent: Vec<PathBuf>,
}

impl State {
    pub fn new(mode: Mode) -> State {
        let mut query = TextField::new(match mode {
            Mode::Commands => "输入命令名称进行搜索…",
            Mode::Files => "按文件名快速打开…",
        });
        query.style = TextStyle::Label;
        State {
            mode,
            query,
            selected: 0,
            scroll: 0.0,
            files: Vec::new(),
            recent: Vec::new(),
        }
    }

    /// 当前可选项个数。
    pub fn item_count(&self) -> usize {
        match self.mode {
            Mode::Commands => filter_commands(self.query.text()).len(),
            Mode::Files => filter_files(&self.files, &self.recent, self.query.text()).len(),
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        let n = self.item_count();
        if n == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as i32 + delta).clamp(0, n as i32 - 1) as usize;
    }

    /// 查询变了：选中回到第一项（TSX 的 `useEffect(() => setSelectedIndex(0), [filtered])`）。
    pub fn query_changed(&mut self) {
        self.selected = 0;
        self.scroll = 0.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Backdrop,
    Inside,
    Input,
    Row(usize),
}

/// 列表里的一行：分组标题或条目。
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Group(String),
    Command(usize),
    File(usize),
    Empty(String),
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub card: Rect,
    pub input: Rect,
    pub list: Rect,
    pub footer: Rect,
    /// (矩形, 行) —— 已按滚动偏移。
    pub rows: Vec<(Rect, RowKind)>,
    pub entries: Vec<(Rect, Hit)>,
    pub content_height: f32,
    /// 这次排版对应的命令下标 / 文件列表，画的时候要用。
    pub commands: Vec<usize>,
    pub files: Vec<FlatFile>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Hit {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
            .unwrap_or(Hit::Backdrop)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.list.height()).max(0.0)
    }
    /// 第 `i` 个条目的矩形（未滚动时的内容坐标）。让选中项滚进视野要用。
    pub fn item_rect(&self, i: usize) -> Option<Rect> {
        self.rows
            .iter()
            .find(|(_, k)| matches!(k, RowKind::Command(j) | RowKind::File(j) if *j == i))
            .map(|(r, _)| *r)
    }
}

fn row_height(has_description: bool) -> f32 {
    row_pad_y() + 20.0 + if has_description { 16.0 } else { 0.0 } + row_pad_y()
}

pub fn layout(viewport: Rect, state: &State) -> Layout {
    let mut lay = Layout::default();
    if viewport.is_empty() {
        return lay;
    }
    let width = max_width().min(viewport.width() - 32.0).max(240.0);
    let left = viewport.left + (viewport.width() - width) / 2.0;
    let top = viewport.top + viewport.height() * 0.2;
    let max_list = match state.mode {
        Mode::Commands => 500.0,
        Mode::Files => 400.0,
    };
    let query = state.query.text();
    let showing_groups = query.trim().is_empty();

    // 先算内容
    let mut rows: Vec<(f32, RowKind)> = Vec::new(); // (高度, 行)
    match state.mode {
        Mode::Commands => {
            lay.commands = filter_commands(query);
            if lay.commands.is_empty() {
                rows.push((EMPTY_H, RowKind::Empty("没有找到匹配的命令".to_owned())));
            } else {
                let mut last_cat: Option<&str> = None;
                for &ci in &lay.commands {
                    let c = &COMMANDS[ci];
                    if showing_groups && last_cat != Some(c.category) {
                        rows.push((GROUP_H, RowKind::Group(c.category.to_owned())));
                        last_cat = Some(c.category);
                    }
                    rows.push((row_height(c.description.is_some()), RowKind::Command(ci)));
                }
            }
        }
        Mode::Files => {
            lay.files = filter_files(&state.files, &state.recent, query);
            if lay.files.is_empty() {
                let msg = if query.trim().is_empty() {
                    "还没有最近打开的文件"
                } else {
                    "没有找到匹配的文件"
                };
                rows.push((EMPTY_H, RowKind::Empty(msg.to_owned())));
            } else {
                if showing_groups {
                    rows.push((GROUP_H, RowKind::Group("最近打开".to_owned())));
                }
                for (i, f) in lay.files.iter().enumerate() {
                    rows.push((row_height(f.relative != f.name), RowKind::File(i)));
                }
            }
        }
    }
    let content_height: f32 = rows.iter().map(|(h, _)| *h).sum();
    let list_h = content_height.min(max_list);
    let card = Rect::new(left, top, left + width, top + HEADER_H + list_h + FOOTER_H);
    lay.card = card;
    lay.input = Rect::new(
        card.left + 16.0,
        card.top + 16.0,
        card.right - 16.0,
        card.top + 16.0 + 38.0,
    );
    lay.list = Rect::new(
        card.left,
        card.top + HEADER_H,
        card.right,
        card.top + HEADER_H + list_h,
    );
    lay.footer = Rect::new(card.left, lay.list.bottom, card.right, card.bottom);
    lay.content_height = content_height;
    lay.entries.push((card, Hit::Inside));
    lay.entries.push((lay.input, Hit::Input));

    let mut y = lay.list.top - state.scroll;
    let mut item_index = 0usize;
    for (h, kind) in rows {
        let r = Rect::new(card.left, y, card.right, y + h);
        if matches!(kind, RowKind::Command(_) | RowKind::File(_)) {
            // 只有落在列表可视区里的才可点
            if r.bottom > lay.list.top && r.top < lay.list.bottom {
                let clipped = Rect::new(
                    r.left,
                    r.top.max(lay.list.top),
                    r.right,
                    r.bottom.min(lay.list.bottom),
                );
                lay.entries.push((clipped, Hit::Row(item_index)));
            }
            item_index += 1;
        }
        lay.rows.push((r, kind));
        y += h;
    }
    lay
}

pub fn paint(list: &mut DrawList, viewport: Rect, lay: &Layout, state: &mut State, p: &Palette) {
    // bg-black/50
    list.rect_alpha(viewport, 0x000000, 0.5);
    let card = lay.card;
    // shadow-2xl
    list.rounded_rect(
        Rect::new(card.left, card.top + 12.0, card.right, card.bottom + 12.0),
        radius() + 4.0,
        theme::mix(0x000000, p.background, 0.10),
    );
    list.rounded_rect(card, radius(), p.surface);
    list.hline(card.left, card.right, card.top + HEADER_H - 1.0, p.border);
    let look = FieldLook {
        radius: 4.0,
        background: Some(p.background),
        border: Some(p.border),
        focus_ring: Some(p.accent),
        padding_left: 12.0,
        padding_right: 12.0,
        leading_icon: None,
    };
    state.query.paint(list, lay.input, true, p, look);

    // 列表
    list.push_clip(lay.list);
    let mut item_index = 0usize;
    for (r, kind) in &lay.rows {
        if r.bottom < lay.list.top || r.top > lay.list.bottom {
            if matches!(kind, RowKind::Command(_) | RowKind::File(_)) {
                item_index += 1;
            }
            continue;
        }
        match kind {
            RowKind::Group(title) => {
                list.rect(*r, p.background);
                let text_rect = Rect::new(
                    r.left + row_pad_x(),
                    r.top + 8.0,
                    r.right - row_pad_x(),
                    r.bottom - 8.0,
                );
                if state.mode == Mode::Files {
                    list.icon_centered(
                        Rect::new(
                            text_rect.left,
                            text_rect.top,
                            text_rect.left + 12.0,
                            text_rect.bottom,
                        ),
                        Icon::CLOCK,
                        12.0,
                        p.muted,
                    );
                    list.text_run(
                        Rect::new(
                            text_rect.left + 20.0,
                            text_rect.top,
                            text_rect.right,
                            text_rect.bottom,
                        ),
                        title.clone(),
                        TextStyle::Caption,
                        p.muted,
                        Align::Leading,
                        Emphasis::Bold,
                    );
                } else {
                    list.text_run(
                        text_rect,
                        title.clone(),
                        TextStyle::Caption,
                        p.muted,
                        Align::Leading,
                        Emphasis::Bold,
                    );
                }
            }
            RowKind::Empty(msg) => {
                list.text_aligned(*r, msg.clone(), TextStyle::Body, p.muted, Align::Center);
            }
            RowKind::Command(ci) => {
                let selected = item_index == state.selected;
                paint_row_bg(list, *r, selected, p);
                let c = &COMMANDS[*ci];
                let icon_rect = Rect::new(
                    r.left + row_pad_x(),
                    r.top,
                    r.left + row_pad_x() + 18.0,
                    r.bottom,
                );
                list.icon_centered(icon_rect, c.icon, 18.0, p.foreground);
                let text_left = icon_rect.right + 12.0;
                let mut right = r.right - row_pad_x();
                if let Some(sc) = c.shortcut {
                    let w = text::measure(sc, TextStyle::Mono) + 16.0;
                    let badge = Rect::new(
                        right - w,
                        r.top + (r.height() - 24.0) / 2.0,
                        right,
                        r.top + (r.height() + 24.0) / 2.0,
                    );
                    list.rounded_rect(badge, 4.0, p.background);
                    list.text_aligned(
                        badge,
                        sc.to_owned(),
                        TextStyle::Mono,
                        p.muted,
                        Align::Center,
                    );
                    right = badge.left - 12.0;
                }
                list.text_run(
                    Rect::new(
                        text_left,
                        r.top + row_pad_y(),
                        right,
                        r.top + row_pad_y() + 20.0,
                    ),
                    c.label,
                    TextStyle::Label,
                    p.foreground,
                    Align::Leading,
                    Emphasis::Bold,
                );
                if let Some(d) = c.description {
                    list.text(
                        Rect::new(
                            text_left,
                            r.top + row_pad_y() + 20.0,
                            right,
                            r.top + row_pad_y() + 36.0,
                        ),
                        d,
                        TextStyle::Caption,
                        p.muted,
                    );
                }
                item_index += 1;
            }
            RowKind::File(fi) => {
                let selected = item_index == state.selected;
                paint_row_bg(list, *r, selected, p);
                let f = &lay.files[*fi];
                let icon_rect = Rect::new(
                    r.left + row_pad_x(),
                    r.top,
                    r.left + row_pad_x() + 16.0,
                    r.bottom,
                );
                let (icon, color) = if f.is_link {
                    (Icon::LINK, p.accent)
                } else if f.name.to_lowercase().ends_with(".md") {
                    (Icon::FILE, p.foreground)
                } else {
                    (Icon::FOLDER_OPEN, p.muted)
                };
                list.icon_centered(icon_rect, icon, 16.0, color);
                let text_left = icon_rect.right + 12.0;
                let mut right = r.right - row_pad_x();
                if selected {
                    let hint = "Enter 打开";
                    let w = text::measure(hint, TextStyle::Caption);
                    list.text(
                        Rect::new(right - w, r.top, right, r.bottom),
                        hint,
                        TextStyle::Caption,
                        p.muted,
                    );
                    right -= w + 12.0;
                }
                list.push_clip(Rect::new(text_left, r.top, right, r.bottom));
                list.text_run(
                    Rect::new(
                        text_left,
                        r.top + row_pad_y(),
                        right,
                        r.top + row_pad_y() + 20.0,
                    ),
                    f.name.clone(),
                    TextStyle::Label,
                    p.foreground,
                    Align::Leading,
                    Emphasis::Bold,
                );
                if f.relative != f.name {
                    list.text(
                        Rect::new(
                            text_left,
                            r.top + row_pad_y() + 20.0,
                            right,
                            r.top + row_pad_y() + 36.0,
                        ),
                        f.relative.clone(),
                        TextStyle::Caption,
                        p.muted,
                    );
                }
                list.pop_clip();
                item_index += 1;
            }
        }
    }
    list.pop_clip();

    // 页脚
    list.rect(lay.footer, p.background);
    list.hline(card.left, card.right, lay.footer.top, p.border);
    let f = Rect::new(
        lay.footer.left + row_pad_x(),
        lay.footer.top + 8.0,
        lay.footer.right - row_pad_x(),
        lay.footer.bottom - 8.0,
    );
    list.text(f, "↑↓ 选择", TextStyle::Caption, p.muted);
    let right_text = match state.mode {
        Mode::Commands => "Enter 执行 · Esc 关闭",
        Mode::Files => "Enter 打开 · Esc 关闭",
    };
    list.text_aligned(f, right_text, TextStyle::Caption, p.muted, Align::Trailing);
}

fn paint_row_bg(list: &mut DrawList, r: Rect, selected: bool, p: &Palette) {
    if selected {
        list.rect(r, theme::mix(p.accent, p.surface, 0.10));
        list.rect(Rect::new(r.left, r.top, r.left + 2.0, r.bottom), p.accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    fn file(rel: &str) -> FlatFile {
        let name = rel.rsplit('/').next().unwrap().to_owned();
        FlatFile {
            path: PathBuf::from(format!("D:/ws/{rel}")),
            name,
            relative: rel.to_owned(),
            is_link: false,
        }
    }

    #[test]
    fn command_filtering_scores_like_the_tsx() {
        let all = filter_commands("");
        assert_eq!(all.len(), COMMANDS.len());
        let hits = filter_commands("新建");
        // 三个「新建…」都命中，且前缀命中的在前；「打开设置」不含「新建」但描述/分类也不含 → 淘汰
        assert!(hits.len() >= 3);
        assert!(hits
            .iter()
            .take(3)
            .all(|i| COMMANDS[*i].label.starts_with("新建")));
        assert!(!hits.contains(&(COMMANDS.len() - 1)));
        // 描述命中也算
        let by_desc = filter_commands("偏好");
        assert_eq!(by_desc, vec![COMMANDS.len() - 1]);
        assert!(filter_commands("zzzz").is_empty());
    }

    #[test]
    fn file_filtering_prefers_consecutive_and_boundary_matches_and_caps_at_50() {
        let files = vec![
            file("知识库/操作系统.md"),
            file("刷题库/LeetCode/两数之和.md"),
            file("a-b/xab.md"),
        ];
        let r = filter_files(&files, &[], "两数");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "两数之和.md");
        // `ab` 在 `a-b/xab.md`：文件名里 a、b 连续 → 高分；顺序仍是稳定的
        let r = filter_files(&files, &[], "ab");
        assert_eq!(r[0].relative, "a-b/xab.md");
        // 50 上限
        let many: Vec<FlatFile> = (0..80).map(|i| file(&format!("d/note{i}.md"))).collect();
        assert_eq!(filter_files(&many, &[], "note").len(), 50);
        // 空查询显示最近打开（能对上的用表里的，对不上的按路径临时造）
        let recent = vec![
            PathBuf::from("D:/ws/a-b/xab.md"),
            PathBuf::from("D:/else/gone.md"),
        ];
        let r = filter_files(&files, &recent, "");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].relative, "a-b/xab.md");
        assert_eq!(r[1].name, "gone.md");
    }

    #[test]
    fn the_card_is_672_wide_centered_and_20vh_from_the_top() {
        let state = State::new(Mode::Commands);
        let vp = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let lay = layout(vp, &state);
        assert_eq!(lay.card.width(), max_width());
        assert_eq!(lay.card.left, (1200.0 - max_width()) / 2.0);
        assert_eq!(lay.card.top, 160.0);
        // 空查询：分组标题 + 全部命令
        let groups = lay
            .rows
            .iter()
            .filter(|(_, k)| matches!(k, RowKind::Group(_)))
            .count();
        assert_eq!(groups, 6, "文件/视图/导航/AI/版本控制/设置");
        assert!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, Hit::Row(_)))
                .count()
                <= COMMANDS.len()
        );
        assert!(lay.list.height() <= 500.0);
        assert_eq!(lay.hit(vp.left + 1.0, vp.top + 1.0), Hit::Backdrop);
        let (r0, _) = lay.entries.iter().find(|(_, h)| *h == Hit::Row(0)).unwrap();
        assert_eq!(lay.hit(r0.left + 10.0, r0.top + 5.0), Hit::Row(0));
    }

    #[test]
    fn selection_moves_within_bounds_and_resets_on_query_change() {
        let mut s = State::new(Mode::Commands);
        s.move_selection(-1);
        assert_eq!(s.selected, 0);
        s.move_selection(3);
        assert_eq!(s.selected, 3);
        s.move_selection(100);
        assert_eq!(s.selected, COMMANDS.len() - 1);
        s.query.set_text("zz");
        s.query_changed();
        assert_eq!(s.selected, 0);
        s.move_selection(1);
        assert_eq!(s.selected, 0, "没有结果时停在 0");
    }

    #[test]
    fn painting_shows_labels_shortcuts_groups_and_footer() {
        let mut state = State::new(Mode::Commands);
        let vp = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let lay = layout(vp, &state);
        let mut list = DrawList::new();
        paint(
            &mut list,
            vp,
            &lay,
            &mut state,
            theme::tokens().palette(false),
        );
        let texts: Vec<String> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        for expected in [
            "文件",
            "新建笔记",
            "Ctrl+N",
            "↑↓ 选择",
            "Enter 执行 · Esc 关闭",
        ] {
            assert!(texts.iter().any(|t| t == expected), "缺 {expected}");
        }
        assert!(list.finish().is_ok());

        // 某个分类里新增的命令可能把靠后的快捷键挤到首屏之外。滚动到该行
        // 后验证其渲染。
        let search = COMMANDS
            .iter()
            .position(|c| c.shortcut == Some("Ctrl+Shift+F"))
            .unwrap();
        state.scroll =
            (lay.item_rect(search).unwrap().top - lay.list.top).clamp(0.0, lay.max_scroll());
        let scrolled = layout(vp, &state);
        let mut scrolled_list = DrawList::new();
        paint(
            &mut scrolled_list,
            vp,
            &scrolled,
            &mut state,
            theme::tokens().palette(false),
        );
        assert!(scrolled_list
            .cmds()
            .iter()
            .any(|cmd| matches!(cmd, DrawCmd::Text { text, .. } if text == "Ctrl+Shift+F")));
        assert!(scrolled_list.finish().is_ok());

        let mut files = State::new(Mode::Files);
        files.files = vec![file("知识库/操作系统.md")];
        files.recent = vec![PathBuf::from("D:/ws/知识库/操作系统.md")];
        let lay = layout(vp, &files);
        let mut list = DrawList::new();
        paint(
            &mut list,
            vp,
            &lay,
            &mut files,
            theme::tokens().palette(false),
        );
        let texts: Vec<String> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        for expected in [
            "最近打开",
            "操作系统.md",
            "知识库/操作系统.md",
            "Enter 打开",
            "Enter 打开 · Esc 关闭",
        ] {
            assert!(
                texts.iter().any(|t| t == expected),
                "缺 {expected}: {texts:?}"
            );
        }
        assert!(list.finish().is_ok());
    }
}
