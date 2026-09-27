//! 搜索命中的 span 使用 UTF-16 偏移，切 Rust 字符串前先换算。

use std::collections::HashSet;

use mochi_core::search::{SearchFileGroup, SearchOptions, SearchQueryResult, SearchResultLine};

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text;
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

/// 文件类型预设，照抄 `FILE_TYPE_PRESETS`。
pub const FILE_TYPE_PRESETS: &[(&str, &str, &[&str])] = &[
    ("note", "笔记", &[".md", ".mc", ".mdx", ".exam"]),
    ("text", "纯文本", &[".txt"]),
    (
        "code",
        "代码",
        &[".ts", ".tsx", ".js", ".jsx", ".css", ".html", ".xml"],
    ),
    ("data", "数据", &[".json", ".yml", ".yaml"]),
];

/// 输入停顿多久才真正搜。
pub const DEBOUNCE_MS: u32 = 220;

pub const MAX_WIDTH: f32 = 768.0;
const RADIUS: f32 = 12.0;
const FILE_ROW: f32 = 32.0;
const MATCH_ROW: f32 = 32.0;
const INPUT_ROW: f32 = 40.0;
const OPTION_ROW: f32 = 28.0 + 10.0;
const FOOTER: f32 = 33.0;

/// 列表里的一行：文件组头，或组内一条命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    File(usize),
    Match(usize, usize),
}

/// 搜索覆盖层的全部状态。
pub struct SearchState {
    pub query: TextField,
    pub options: SearchOptions,
    pub filters_open: bool,
    pub collapsed: HashSet<String>,
    pub selected: usize,
    pub scroll: f32,
    pub result: Option<SearchQueryResult>,
    pub searching: bool,
    pub history: Vec<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        let mut query = TextField::new("搜索所有笔记内容…");
        query.style = TextStyle::Search;
        SearchState {
            query,
            options: SearchOptions::default(),
            filters_open: false,
            collapsed: HashSet::new(),
            selected: 0,
            scroll: 0.0,
            result: None,
            searching: false,
            history: Vec::new(),
        }
    }
}

impl SearchState {
    pub fn trimmed_query(&self) -> String {
        self.query.text().trim().to_owned()
    }

    /// 展平成行：文件头 + 未折叠组的命中。
    pub fn rows(&self) -> Vec<RowKind> {
        let mut out = Vec::new();
        let Some(r) = &self.result else { return out };
        for (gi, g) in r.groups.iter().enumerate() {
            out.push(RowKind::File(gi));
            if self.collapsed.contains(&g.path) {
                continue;
            }
            for mi in 0..g.matches.len() {
                out.push(RowKind::Match(gi, mi));
            }
        }
        out
    }

    pub fn has_results(&self) -> bool {
        self.result
            .as_ref()
            .map(|r| !r.groups.is_empty())
            .unwrap_or(false)
    }

    /// 换词：展开状态与选中项重置（TSX：`useEffect(..., [trimmedQuery])`）。
    pub fn on_query_changed(&mut self) {
        self.collapsed.clear();
        self.selected = 0;
        self.scroll = 0.0;
    }

    pub fn apply_result(&mut self, result: SearchQueryResult) {
        self.result = Some(result);
        self.searching = false;
        let n = self.rows().len();
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }

    pub fn clear_results(&mut self) {
        self.result = None;
        self.searching = false;
    }

    /// 某个预设是否整组都在筛选里。
    pub fn preset_active(&self, exts: &[&str]) -> bool {
        exts.iter()
            .all(|e| self.options.extensions.iter().any(|x| x == e))
    }

    pub fn toggle_preset(&mut self, exts: &[&str]) {
        if self.preset_active(exts) {
            self.options
                .extensions
                .retain(|x| !exts.contains(&x.as_str()));
        } else {
            for e in exts {
                if !self.options.extensions.iter().any(|x| x == e) {
                    self.options.extensions.push((*e).to_owned());
                }
            }
        }
    }

    pub fn add_to_history(&mut self, q: &str) {
        if q.is_empty() {
            return;
        }
        self.history.retain(|h| h != q);
        self.history.insert(0, q.to_owned());
        self.history.truncate(20);
    }

    /// 选中项对应的命中（文件头取组内第一条）。
    pub fn result_at(&self, row: RowKind) -> Option<&SearchResultLine> {
        let r = self.result.as_ref()?;
        match row {
            RowKind::File(g) => r.groups.get(g)?.matches.first(),
            RowKind::Match(g, m) => r.groups.get(g)?.matches.get(m),
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        let n = self.rows().len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).clamp(0, n as i32 - 1) as usize;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchHit {
    Backdrop,
    Input,
    Close,
    CaseSensitive,
    WholeWord,
    Regex,
    Phrase,
    Filters,
    /// 筛选药丸；`None` 是「全部」。
    Preset(Option<usize>),
    Row(usize),
    /// 文件头左侧的折叠箭头。
    Collapse(usize),
    History(usize),
    ClearHistory,
    /// 内容盒里但不在任何控件上。
    Inside,
}

#[derive(Debug, Clone, Default)]
pub struct SearchLayout {
    pub panel: Rect,
    pub entries: Vec<(Rect, SearchHit)>,
    pub list: Rect,
    pub content_height: f32,
}

impl SearchLayout {
    pub fn hit(&self, x: f32, y: f32) -> SearchHit {
        if let Some((_, h)) = self.entries.iter().rev().find(|(r, _)| r.contains(x, y)) {
            return *h;
        }
        if self.panel.contains(x, y) {
            SearchHit::Inside
        } else {
            SearchHit::Backdrop
        }
    }

    pub fn rect_of(&self, hit: SearchHit) -> Option<Rect> {
        self.entries
            .iter()
            .find(|(_, h)| *h == hit)
            .map(|(r, _)| *r)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.list.height()).max(0.0)
    }
}

pub fn layout(s: &SearchState, viewport: Rect) -> SearchLayout {
    let mut out = SearchLayout::default();
    if viewport.is_empty() {
        return out;
    }
    let width = MAX_WIDTH.min(viewport.width() - 32.0).max(200.0);
    let left = ((viewport.left + viewport.right) / 2.0 - width / 2.0).round();
    let top = (viewport.top + viewport.height() * 0.12).round();
    let max_h = (viewport.height() * 0.72).round();

    // ── 头部 ──
    let mut y = top + 12.0; // pt-3
    let px = left + 16.0;
    let pr = left + width - 16.0;
    let input_row = Rect::new(px, y, pr, y + INPUT_ROW);
    out.entries.push((
        Rect::new(pr - 28.0, y + 6.0, pr, y + 34.0),
        SearchHit::Close,
    ));
    out.entries.push((
        Rect::new(px + 17.0 + 8.0, y, pr - 28.0 - 8.0 - 24.0, y + INPUT_ROW),
        SearchHit::Input,
    ));
    let _ = input_row;
    y += INPUT_ROW;
    // 选项行
    let mut x = px;
    for hit in [
        SearchHit::CaseSensitive,
        SearchHit::WholeWord,
        SearchHit::Regex,
        SearchHit::Phrase,
    ] {
        out.entries.push((Rect::new(x, y, x + 28.0, y + 28.0), hit));
        x += 28.0 + 4.0;
    }
    x += 4.0 + 1.0 + 4.0; // mx-1 竖线
    let filter_w = 13.0
        + 6.0
        + text::measure("文件类型", TextStyle::Caption)
        + 16.0
        + if s.options.extensions.is_empty() {
            0.0
        } else {
            text::measure(
                &format!("·{}", s.options.extensions.len()),
                TextStyle::Caption,
            ) + 4.0
        };
    out.entries
        .push((Rect::new(x, y, x + filter_w, y + 28.0), SearchHit::Filters));
    y += OPTION_ROW;
    if s.filters_open {
        y += 1.0 + 10.0;
        let mut fx = px;
        let pill = |label: &str| text::measure(label, TextStyle::Caption) + 20.0;
        let w = pill("全部");
        out.entries
            .push((Rect::new(fx, y, fx + w, y + 24.0), SearchHit::Preset(None)));
        fx += w + 6.0;
        for (i, (_, label, _)) in FILE_TYPE_PRESETS.iter().enumerate() {
            let w = pill(label);
            out.entries.push((
                Rect::new(fx, y, fx + w, y + 24.0),
                SearchHit::Preset(Some(i)),
            ));
            fx += w + 6.0;
        }
        y += 24.0 + 10.0;
    }
    y += 1.0; // border-b
    let header_bottom = y;

    // ── 列表 ──
    let rows = s.rows();
    let query = s.trimmed_query();
    let content_height = if query.is_empty() {
        if s.history.is_empty() {
            40.0 + 16.0 + 20.0 + 8.0 + 16.0 * 2.0 + 56.0 * 2.0
        } else {
            8.0 + 28.0 + (s.history.len().min(8) as f32) * 36.0 + 8.0
        }
    } else if rows.is_empty() {
        56.0 * 2.0 + 20.0 + 8.0 + 16.0
    } else {
        rows.iter()
            .map(|r| match r {
                RowKind::File(_) => FILE_ROW,
                RowKind::Match(..) => MATCH_ROW,
            })
            .sum()
    };
    let footer = if s.has_results() { FOOTER } else { 0.0 };
    let list_h = content_height
        .min(max_h - (header_bottom - top) - footer)
        .max(0.0);
    let list = Rect::new(left, header_bottom, left + width, header_bottom + list_h);
    out.list = list;
    out.content_height = content_height;
    let scroll = s.scroll.min(out.max_scroll()).max(0.0);

    let mut ly = list.top - scroll;
    if query.is_empty() && !s.history.is_empty() {
        ly += 8.0;
        out.entries.push((
            Rect::new(
                list.right - 16.0 - 52.0,
                ly + 4.0,
                list.right - 16.0,
                ly + 24.0,
            ),
            SearchHit::ClearHistory,
        ));
        ly += 28.0;
        for i in 0..s.history.len().min(8) {
            out.entries.push((
                Rect::new(list.left, ly, list.right, ly + 36.0),
                SearchHit::History(i),
            ));
            ly += 36.0;
        }
    } else {
        for (i, row) in rows.iter().enumerate() {
            let h = match row {
                RowKind::File(_) => FILE_ROW,
                RowKind::Match(..) => MATCH_ROW,
            };
            let r = Rect::new(list.left, ly, list.right, ly + h);
            // 行先、箭头后：命中测试从后往前找，箭头要赢过它所在的行
            out.entries.push((r, SearchHit::Row(i)));
            if let RowKind::File(_) = row {
                out.entries.push((
                    Rect::new(
                        list.left + 12.0,
                        ly + 6.0,
                        list.left + 12.0 + 18.0,
                        ly + 26.0,
                    ),
                    SearchHit::Collapse(i),
                ));
            }
            ly += h;
        }
    }
    // 滚出列表的行不参与命中
    out.entries.retain(|(r, h)| match h {
        SearchHit::Row(_)
        | SearchHit::Collapse(_)
        | SearchHit::History(_)
        | SearchHit::ClearHistory => r.bottom > list.top && r.top < list.bottom,
        _ => true,
    });
    out.panel = Rect::new(left, top, left + width, list.bottom + footer);
    out
}

/// UTF-16 偏移 → 字节偏移（span 与 JS 一致按码元计）。
fn byte_at_utf16(s: &str, units: u32) -> usize {
    super::editor::byte_offset_from_utf16(s, units as usize)
}

pub fn paint(
    list: &mut DrawList,
    s: &mut SearchState,
    lay: &SearchLayout,
    viewport: Rect,
    p: &Palette,
) {
    if viewport.is_empty() || lay.panel.is_empty() {
        return;
    }
    // 遮罩 bg-black/40
    list.rect_alpha(viewport, 0x000000, 0.4);
    let panel = lay.panel;
    list.rounded_rect(panel, RADIUS, p.surface_elevated);
    list.rounded_border(panel, RADIUS, p.border);

    // ── 输入行 ──
    let input = lay.rect_of(SearchHit::Input).unwrap_or(Rect::ZERO);
    list.icon_centered(
        Rect::new(
            panel.left + 16.0,
            input.top,
            panel.left + 16.0 + 17.0,
            input.bottom,
        ),
        Icon::SEARCH,
        17.0,
        p.muted,
    );
    s.query.paint(list, input, true, p, FieldLook::bare());
    if s.searching {
        list.icon_centered(
            Rect::new(
                input.right + 8.0,
                input.top,
                input.right + 24.0,
                input.bottom,
            ),
            Icon::LOADER2,
            16.0,
            p.accent,
        );
    }
    if let Some(r) = lay.rect_of(SearchHit::Close) {
        list.icon_centered(r, Icon::X, 16.0, p.muted);
    }

    // ── 选项行 ──
    let toggles = [
        (
            SearchHit::CaseSensitive,
            Icon::CASE_SENSITIVE,
            s.options.case_sensitive,
        ),
        (SearchHit::WholeWord, Icon::WHOLE_WORD, s.options.whole_word),
        (SearchHit::Regex, Icon::REGEX, s.options.use_regex),
        (SearchHit::Phrase, Icon::QUOTE, s.options.match_phrase),
    ];
    for (hit, icon, active) in toggles {
        let Some(r) = lay.rect_of(hit) else { continue };
        if active {
            list.rounded_rect(r, 6.0, theme::mix(p.accent, p.surface_elevated, 0.16));
            list.rounded_border(r, 6.0, theme::mix(p.accent, p.surface_elevated, 0.55));
        }
        list.icon_centered(r, icon, 15.0, if active { p.accent } else { p.muted });
    }
    if let Some(r) = lay.rect_of(SearchHit::Filters) {
        // 竖线 mx-1 h-4
        list.rect(
            Rect::new(r.left - 5.0, r.top + 6.0, r.left - 4.0, r.top + 22.0),
            p.border,
        );
        let active = s.filters_open || !s.options.extensions.is_empty();
        if active {
            list.rounded_rect(r, 6.0, theme::mix(p.accent, p.surface_elevated, 0.16));
        }
        let color = if active { p.accent } else { p.muted };
        list.icon_centered(
            Rect::new(r.left + 8.0, r.top, r.left + 21.0, r.bottom),
            Icon::SLIDERS_HORIZONTAL,
            13.0,
            color,
        );
        let mut label = "文件类型".to_owned();
        if !s.options.extensions.is_empty() {
            label.push_str(&format!(" ·{}", s.options.extensions.len()));
        }
        list.text(
            Rect::new(r.left + 27.0, r.top, r.right, r.bottom),
            label,
            TextStyle::Caption,
            color,
        );
        // 统计（右侧）
        if let Some(res) = &s.result {
            if !res.groups.is_empty() {
                let st = &res.stats;
                let mut t = format!("{} 个文件 · {} 处匹配", st.total_files, st.total_matches);
                if st.truncated {
                    t.push_str("（仅显示最相关的部分）");
                }
                t.push_str(&format!(" · {}ms", st.duration_ms));
                list.text_aligned(
                    Rect::new(r.right + 8.0, r.top, panel.right - 16.0, r.bottom),
                    t,
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
            }
        }
    }
    // ── 筛选行 ──
    if s.filters_open {
        if let Some(first) = lay.rect_of(SearchHit::Preset(None)) {
            list.hline(
                panel.left + 16.0,
                panel.right - 16.0,
                first.top - 10.0 - 1.0,
                p.border,
            );
        }
        let paint_pill = |list: &mut DrawList, r: Rect, label: &str, active: bool| {
            if active {
                list.rounded_rect(r, 12.0, theme::mix(p.accent, p.surface_elevated, 0.14));
                list.rounded_border(r, 12.0, theme::mix(p.accent, p.surface_elevated, 0.55));
            } else {
                list.rounded_border(r, 12.0, p.border);
            }
            list.text_aligned(
                r,
                label,
                TextStyle::Caption,
                if active { p.accent } else { p.muted },
                Align::Center,
            );
        };
        if let Some(r) = lay.rect_of(SearchHit::Preset(None)) {
            paint_pill(list, r, "全部", s.options.extensions.is_empty());
        }
        for (i, (_, label, exts)) in FILE_TYPE_PRESETS.iter().enumerate() {
            if let Some(r) = lay.rect_of(SearchHit::Preset(Some(i))) {
                paint_pill(list, r, label, s.preset_active(exts));
            }
        }
    }
    list.hline(panel.left, panel.right, lay.list.top - 1.0, p.border);

    // ── 列表 ──
    let lr = lay.list;
    list.push_clip(lr);
    let query = s.trimmed_query();
    let rows = s.rows();
    let selected_bg = theme::mix(p.accent, p.surface_elevated, 0.14);
    if query.is_empty() {
        if s.history.is_empty() {
            let cy = lr.top + 56.0;
            list.icon_centered(
                Rect::new(lr.left, cy, lr.right, cy + 40.0),
                Icon::SEARCH,
                40.0,
                p.border,
            );
            list.text_aligned(
                Rect::new(lr.left, cy + 56.0, lr.right, cy + 76.0),
                "输入关键词，搜索工作区内的所有笔记",
                TextStyle::Label,
                p.foreground,
                Align::Center,
            );
            list.text_aligned(
                Rect::new(lr.left, cy + 84.0, lr.right, cy + 102.0),
                "多个词以空格分隔时，只返回同时包含它们的笔记",
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
            list.text_aligned(
                Rect::new(lr.left, cy + 102.0, lr.right, cy + 120.0),
                "用引号包裹可以搜索完整短语，例如「\"机器学习\"」",
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
        } else {
            if let Some(clear) = lay.rect_of(SearchHit::ClearHistory) {
                let head = Rect::new(
                    lr.left + 16.0,
                    clear.top - 4.0,
                    lr.right - 16.0,
                    clear.bottom + 4.0,
                );
                list.icon_centered(
                    Rect::new(head.left, head.top, head.left + 12.0, head.bottom),
                    Icon::HISTORY,
                    12.0,
                    p.muted,
                );
                list.text(
                    Rect::new(head.left + 18.0, head.top, head.right, head.bottom),
                    "最近搜索",
                    TextStyle::Caption,
                    p.muted,
                );
                list.icon_centered(
                    Rect::new(clear.left + 4.0, clear.top, clear.left + 15.0, clear.bottom),
                    Icon::TRASH2,
                    11.0,
                    p.muted,
                );
                list.text(
                    Rect::new(clear.left + 19.0, clear.top, clear.right, clear.bottom),
                    "清空",
                    TextStyle::Caption,
                    p.muted,
                );
            }
            for i in 0..s.history.len().min(8) {
                let Some(r) = lay.rect_of(SearchHit::History(i)) else {
                    continue;
                };
                list.icon_centered(
                    Rect::new(r.left + 16.0, r.top, r.left + 29.0, r.bottom),
                    Icon::SEARCH,
                    13.0,
                    p.muted,
                );
                list.text(
                    Rect::new(r.left + 39.0, r.top, r.right - 16.0, r.bottom),
                    text::ellipsize(&s.history[i], TextStyle::Label, r.width() - 60.0),
                    TextStyle::Label,
                    p.foreground,
                );
            }
        }
    } else if s.searching && !s.has_results() {
        let cy = lr.top + 56.0;
        list.icon_centered(
            Rect::new(lr.left, cy, lr.right, cy + 32.0),
            Icon::LOADER2,
            32.0,
            p.accent,
        );
        list.text_aligned(
            Rect::new(lr.left, cy + 48.0, lr.right, cy + 68.0),
            "正在搜索…",
            TextStyle::Label,
            p.muted,
            Align::Center,
        );
    } else if !s.has_results() {
        let cy = lr.top + 56.0;
        match s.result.as_ref().and_then(|r| r.error.clone()) {
            Some(err) => list.text_aligned(
                Rect::new(lr.left + 32.0, cy, lr.right - 32.0, cy + 20.0),
                err,
                TextStyle::Label,
                0xE5484D,
                Align::Center,
            ),
            None => {
                list.text_aligned(
                    Rect::new(lr.left, cy, lr.right, cy + 20.0),
                    format!("没有找到与「{query}」匹配的内容"),
                    TextStyle::Label,
                    p.foreground,
                    Align::Center,
                );
                list.text_aligned(
                    Rect::new(lr.left, cy + 28.0, lr.right, cy + 46.0),
                    "试试更换关键词，或调整上方的匹配选项与文件类型",
                    TextStyle::Caption,
                    p.muted,
                    Align::Center,
                );
            }
        }
    } else if let Some(res) = &s.result {
        for (i, row) in rows.iter().enumerate() {
            let Some(r) = lay.rect_of(SearchHit::Row(i)) else {
                continue;
            };
            let selected = s.selected == i;
            if selected {
                list.rect(r, selected_bg);
            }
            match *row {
                RowKind::File(gi) => paint_file_row(
                    list,
                    r,
                    &res.groups[gi],
                    s.collapsed.contains(&res.groups[gi].path),
                    p,
                ),
                RowKind::Match(gi, mi) => paint_match_row(list, r, &res.groups[gi].matches[mi], p),
            }
        }
    }
    list.pop_clip();

    // ── 底栏 ──
    if s.has_results() {
        let f = Rect::new(panel.left, lr.bottom, panel.right, panel.bottom);
        list.rect(f, p.surface);
        list.hline(f.left, f.right, f.top, p.border);
        list.text(
            Rect::new(f.left + 16.0, f.top, f.right, f.bottom),
            "↑↓ 选择 · 点击文件名左侧箭头可折叠",
            TextStyle::Caption,
            p.muted,
        );
        list.text_aligned(
            Rect::new(f.left, f.top, f.right - 16.0, f.bottom),
            "Enter 打开 · Esc 关闭",
            TextStyle::Caption,
            p.muted,
            Align::Trailing,
        );
    }
}

fn directory_of(rel: &str) -> &str {
    match rel.rfind('/') {
        Some(i) => &rel[..i],
        None => "",
    }
}

fn paint_file_row(list: &mut DrawList, r: Rect, g: &SearchFileGroup, collapsed: bool, p: &Palette) {
    let mut x = r.left + 12.0;
    list.icon_centered(
        Rect::new(x, r.top, x + 18.0, r.bottom),
        if collapsed {
            Icon::CHEVRON_RIGHT
        } else {
            Icon::CHEVRON_DOWN
        },
        14.0,
        p.muted,
    );
    x += 18.0 + 6.0;
    let (icon, color) = if g.path.ends_with(".link.json") {
        (Icon::LINK, p.accent)
    } else {
        (Icon::FILE_TEXT, p.muted)
    };
    list.icon_centered(Rect::new(x, r.top, x + 14.0, r.bottom), icon, 14.0, color);
    x += 14.0 + 6.0;
    // 计数药丸（右）
    let count = g.match_count.to_string();
    let cw = text::measure(&count, TextStyle::Caption) + 12.0;
    let pill = Rect::new(
        r.right - 12.0 - cw,
        r.top + 8.0,
        r.right - 12.0,
        r.bottom - 8.0,
    );
    list.rounded_rect(pill, pill.height() / 2.0, p.surface_muted);
    list.text_aligned(pill, count, TextStyle::Caption, p.muted, Align::Center);
    // 标题（命中标题时 accent 半粗）
    let title_color = if g.title_matched {
        p.accent
    } else {
        p.foreground
    };
    let title_w = text::measure(&g.title, TextStyle::Body).min(pill.left - 8.0 - x);
    list.text_run(
        Rect::new(x, r.top, x + title_w + 2.0, r.bottom),
        text::ellipsize(&g.title, TextStyle::Body, title_w),
        TextStyle::Body,
        title_color,
        Align::Leading,
        super::text::Emphasis::Bold,
    );
    x += title_w + 6.0;
    let dir = directory_of(&g.rel_path);
    if !dir.is_empty() && pill.left - 8.0 > x + 20.0 {
        list.text(
            Rect::new(x, r.top, pill.left - 8.0, r.bottom),
            text::ellipsize(dir, TextStyle::Caption, pill.left - 8.0 - x),
            TextStyle::Caption,
            p.muted,
        );
    }
}

fn paint_match_row(list: &mut DrawList, r: Rect, m: &SearchResultLine, p: &Palette) {
    // pl-9 = 36；行号 w-9 右对齐
    let num = Rect::new(r.left + 36.0, r.top, r.left + 36.0 + 36.0, r.bottom);
    list.text_aligned(
        num,
        if m.base_location.is_some() {
            "▦".into()
        } else {
            m.line.to_string()
        },
        TextStyle::Caption,
        p.muted,
        Align::Trailing,
    );
    let mut x = num.right + 10.0;
    let right = r.right - 16.0;
    let mark = theme::mix(p.accent, p.surface_elevated, 0.34);
    let style = TextStyle::Body;
    let pad = (r.height() - style.line_height()) / 2.0;
    // 按 span 切成普通/高亮交替片段
    let spans: Vec<(u32, u32)> = if m.spans.is_empty() {
        vec![(m.match_start, m.match_end)]
    } else {
        m.spans.iter().map(|s| (s.start, s.end)).collect()
    };
    let content = &m.content;
    let mut cursor = 0usize;
    let mut pieces: Vec<(usize, usize, bool)> = Vec::new();
    for (a, b) in spans {
        let a = byte_at_utf16(content, a).max(cursor);
        let b = byte_at_utf16(content, b).min(content.len());
        if b <= a {
            continue;
        }
        if a > cursor {
            pieces.push((cursor, a, false));
        }
        pieces.push((a, b, true));
        cursor = b;
    }
    if cursor < content.len() {
        pieces.push((cursor, content.len(), false));
    }
    for (a, b, hit) in pieces {
        if x >= right {
            break;
        }
        let piece = text::ellipsize(&content[a..b], style, right - x);
        let w = text::measure(&piece, style);
        if hit {
            list.rounded_rect(
                Rect::new(x - 1.0, r.top + pad, x + w + 1.0, r.bottom - pad),
                2.0,
                mark,
            );
        }
        list.text_run(
            Rect::new(x, r.top, x + w + 2.0, r.bottom),
            piece,
            style,
            p.foreground,
            Align::Leading,
            if hit {
                super::text::Emphasis::Bold
            } else {
                super::text::Emphasis::None
            },
        );
        x += w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;
    use mochi_core::search::{SearchMatchSpan, SearchStats};

    const VIEW: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 1200.0,
        bottom: 800.0,
    };

    fn result() -> SearchQueryResult {
        let m = |line: u32, content: &str, s: u32, e: u32| SearchResultLine {
            id: format!("m{line}"),
            path: "D:/ws/a.md".into(),
            line,
            content: content.into(),
            score: 1.0,
            match_start: s,
            match_end: e,
            spans: vec![SearchMatchSpan { start: s, end: e }],
            match_text: content[..0].into(),
            occurrence: 0,
            base_location: None,
        };
        SearchQueryResult {
            groups: vec![
                SearchFileGroup {
                    path: "D:/ws/a.md".into(),
                    title: "操作系统".into(),
                    rel_path: "知识库/计算机通识/a.md".into(),
                    score: 2.0,
                    match_count: 2,
                    title_matched: false,
                    mtime_ms: 0,
                    matches: vec![m(3, "进程与线程", 0, 2), m(9, "线程共享进程资源", 4, 6)],
                },
                SearchFileGroup {
                    path: "D:/ws/b.md".into(),
                    title: "进程".into(),
                    rel_path: "b.md".into(),
                    score: 1.0,
                    match_count: 1,
                    title_matched: true,
                    mtime_ms: 0,
                    matches: vec![m(1, "# 进程", 2, 4)],
                },
            ],
            stats: SearchStats {
                total_matches: 3,
                total_files: 2,
                truncated: false,
                duration_ms: 7,
            },
            error: None,
        }
    }

    fn with_results() -> SearchState {
        let mut s = SearchState::default();
        s.query.set_text("进程");
        s.apply_result(result());
        s
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
    fn rows_flatten_groups_and_skip_collapsed_ones() {
        let mut s = with_results();
        assert_eq!(
            s.rows(),
            vec![
                RowKind::File(0),
                RowKind::Match(0, 0),
                RowKind::Match(0, 1),
                RowKind::File(1),
                RowKind::Match(1, 0)
            ]
        );
        s.collapsed.insert("D:/ws/a.md".into());
        assert_eq!(
            s.rows(),
            vec![RowKind::File(0), RowKind::File(1), RowKind::Match(1, 0)]
        );
    }

    #[test]
    fn the_panel_is_at_most_768_wide_and_starts_at_12_percent_of_the_viewport() {
        let s = with_results();
        let lay = layout(&s, VIEW);
        assert_eq!(lay.panel.width(), 768.0);
        assert_eq!(lay.panel.top, 96.0);
        assert_eq!((lay.panel.left + lay.panel.right) / 2.0, 600.0);
        assert!(lay.panel.height() <= 800.0 * 0.72 + 1.0);
    }

    #[test]
    fn file_rows_have_a_collapse_arrow_that_hits_separately() {
        let s = with_results();
        let lay = layout(&s, VIEW);
        let arrow = lay.rect_of(SearchHit::Collapse(0)).unwrap();
        assert_eq!(
            lay.hit(arrow.left + 3.0, arrow.top + 3.0),
            SearchHit::Collapse(0)
        );
        let row = lay.rect_of(SearchHit::Row(0)).unwrap();
        assert_eq!(lay.hit(row.left + 200.0, row.top + 3.0), SearchHit::Row(0));
        assert!(
            lay.rect_of(SearchHit::Collapse(1)).is_none(),
            "命中行没有箭头"
        );
        assert_eq!(row.height(), 32.0);
    }

    #[test]
    fn the_selected_row_is_tinted_and_matches_are_marked() {
        let mut s = with_results();
        s.selected = 1;
        let lay = layout(&s, VIEW);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, &mut s, &lay, VIEW, &p);
        let row = lay.rect_of(SearchHit::Row(1)).unwrap();
        let tint = theme::mix(p.accent, p.surface_elevated, 0.14);
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Rect { rect, color } if *rect == row && *color == tint)));
        let mark = theme::mix(p.accent, p.surface_elevated, 0.34);
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::RoundedRect { color, .. } if *color == mark)));
        let t = texts(&list);
        // UTF-16 span (0,2) 切出「进程」
        assert!(t.contains(&"进程".to_owned()));
        assert!(t.contains(&"操作系统".to_owned()));
        assert!(t.iter().any(|x| x.contains("2 个文件 · 3 处匹配")), "{t:?}");
        assert!(t.contains(&"Enter 打开 · Esc 关闭".to_owned()));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn an_empty_query_with_no_history_shows_the_hint_and_no_footer() {
        let mut s = SearchState::default();
        let lay = layout(&s, VIEW);
        let mut list = DrawList::new();
        paint(
            &mut list,
            &mut s,
            &lay,
            VIEW,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(t.contains(&"输入关键词，搜索工作区内的所有笔记".to_owned()));
        assert!(!t.contains(&"Enter 打开 · Esc 关闭".to_owned()));
    }

    #[test]
    fn history_rows_appear_when_the_query_is_empty() {
        let mut s = SearchState::default();
        s.add_to_history("进程");
        s.add_to_history("线程");
        s.add_to_history("进程");
        assert_eq!(
            s.history,
            vec!["进程", "线程"],
            "重复的词提到最前，不重复出现"
        );
        let lay = layout(&s, VIEW);
        assert!(lay.rect_of(SearchHit::History(0)).is_some());
        assert!(lay.rect_of(SearchHit::ClearHistory).is_some());
    }

    #[test]
    fn preset_toggling_adds_and_removes_the_whole_group() {
        let mut s = SearchState::default();
        let exts = FILE_TYPE_PRESETS[0].2;
        s.toggle_preset(exts);
        assert!(s.preset_active(exts));
        assert_eq!(s.options.extensions.len(), 4);
        s.toggle_preset(exts);
        assert!(s.options.extensions.is_empty());
    }

    #[test]
    fn selection_is_clamped_to_the_row_count() {
        let mut s = with_results();
        s.move_selection(100);
        assert_eq!(s.selected, 4);
        s.move_selection(-100);
        assert_eq!(s.selected, 0);
        // 结果变少时选中项跟着夹
        s.selected = 4;
        s.collapsed.insert("D:/ws/a.md".into());
        s.apply_result(result());
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn clicking_outside_the_panel_is_the_backdrop() {
        let s = with_results();
        let lay = layout(&s, VIEW);
        assert_eq!(lay.hit(5.0, 5.0), SearchHit::Backdrop);
        assert_eq!(lay.hit(600.0, lay.panel.top + 5.0), SearchHit::Inside);
    }
}
