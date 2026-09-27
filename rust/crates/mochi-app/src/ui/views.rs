//! 小记复用普通文档标签，此模块只确保小记文件存在。

use std::path::{Path, PathBuf};

use chrono::{Local, TimeZone};
use mochi_core::capture::{derive_title, CaptureItem};
use mochi_core::domain::FileNode;

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

/// 标题区域随 Display 字号扩展，默认仍为 36px 标题 + 20px 留白。
fn header_height() -> f32 {
    TextStyle::Display.line_height().max(36.0) + 20.0
}

/// `formatCapturedAt` / `formatModifiedTime`：相对时间。
pub fn relative_time(ms: i64, now_ms: i64) -> String {
    let diff = now_ms - ms;
    if diff < 60_000 {
        return "刚刚".into();
    }
    if diff < 3_600_000 {
        return format!("{} 分钟前", diff / 60_000);
    }
    if diff < 86_400_000 {
        return format!("{} 小时前", diff / 3_600_000);
    }
    if diff < 7 * 86_400_000 {
        return format!("{} 天前", diff / 86_400_000);
    }
    match (
        Local.timestamp_millis_opt(ms).single(),
        Local.timestamp_millis_opt(now_ms).single(),
    ) {
        (Some(t), Some(now)) => {
            if chrono::Datelike::year(&t) == chrono::Datelike::year(&now) {
                t.format("%m/%d %H:%M").to_string()
            } else {
                t.format("%Y/%m/%d %H:%M").to_string()
            }
        }
        _ => "未知时间".into(),
    }
}

/// 头部的绘制，两个视图共用。返回内容区起点 y。
fn paint_header(list: &mut DrawList, col: Rect, title: &str, p: &Palette) -> f32 {
    let y = col.top;
    let title_height = TextStyle::Display.line_height().max(36.0);
    list.text_run(
        Rect::new(col.left, y, col.right, y + title_height),
        title,
        TextStyle::Display,
        p.foreground,
        Align::Leading,
        Emphasis::Bold,
    );
    y + header_height()
}

/// 内容列：`mx-auto max-w-[N] px-6`。
fn column(area: Rect, max_w: f32) -> Rect {
    let w = (area.width() - 48.0).min(max_w);
    let left = ((area.left + area.right) / 2.0 - w / 2.0).round();
    Rect::new(left, area.top + 24.0, left + w, area.bottom)
}

// =====================================================================
// 收件箱
// =====================================================================

pub mod inbox {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Tab {
        Inbox,
        Archived,
    }

    pub struct State {
        pub items: Vec<CaptureItem>,
        pub tab: Tab,
        pub editing: Option<(String, TextField)>,
        /// 归档目标库（下标进 `libraries`）。
        pub target_library: usize,
        pub libraries: Vec<(String, PathBuf)>,
        pub scroll: f32,
        pub error: String,
        pub loaded: bool,
        pub hover: Option<Hit>,
    }

    impl Default for State {
        fn default() -> Self {
            State {
                items: Vec::new(),
                tab: Tab::Inbox,
                editing: None,
                target_library: 0,
                libraries: Vec::new(),
                scroll: 0.0,
                error: String::new(),
                loaded: false,
                hover: None,
            }
        }
    }

    impl State {
        pub fn visible(&self) -> Vec<&CaptureItem> {
            let want = if self.tab == Tab::Inbox {
                "inbox"
            } else {
                "archived"
            };
            let mut v: Vec<&CaptureItem> = self.items.iter().filter(|i| i.status == want).collect();
            // 新的在上
            v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            v
        }
        pub fn start_edit(&mut self, item: &CaptureItem) {
            let mut f = TextField::new("").with_text(&item.content);
            f.style = TextStyle::Label;
            self.editing = Some((item.id.clone(), f));
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        Refresh,
        Capture,
        TabInbox,
        TabArchived,
        TargetLibrary,
        /// 行内按钮（下标进 `visible()`）。
        Archive(usize),
        ToTask(usize),
        Edit(usize),
        Delete(usize),
        SaveEdit(usize),
        CancelEdit(usize),
        OpenArchived(usize),
        EditField,
        Blank,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Layout {
        pub entries: Vec<(Rect, Hit)>,
        pub rows: Vec<(usize, Rect)>,
        pub card: Rect,
        pub content_height: f32,
        pub viewport: Rect,
    }

    impl Layout {
        pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
            if !self.viewport.contains(x, y) {
                return None;
            }
            if let Some((_, h)) = self.entries.iter().rev().find(|(r, _)| r.contains(x, y)) {
                return Some(*h);
            }
            self.viewport.contains(x, y).then_some(Hit::Blank)
        }
        pub fn rect_of(&self, h: Hit) -> Option<Rect> {
            self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
        }
        pub fn max_scroll(&self) -> f32 {
            (self.content_height - self.viewport.height()).max(0.0)
        }
    }

    fn row_height(item: &CaptureItem, editing: bool) -> f32 {
        if editing {
            let lines = item.content.lines().count().clamp(3, 10) as f32;
            return 12.0 + lines * 20.0 + 16.0 + 8.0 + 16.0 + 12.0;
        }
        let lines = item.content.lines().count().clamp(1, 12) as f32;
        12.0 + lines * 22.0 + 4.0 + 16.0 + 12.0 + if item.web_clip.is_some() { 22.0 } else { 0.0 }
    }

    pub fn layout(s: &State, area: Rect) -> Layout {
        let mut out = Layout::default();
        if area.is_empty() {
            return out;
        }
        out.viewport = area;
        let col = super::super::workspace_ui::column(area, 1120.0);
        let top = col.top - s.scroll;
        let compact = col.width() < 540.0;
        let actions_top = top + if compact { 78.0 } else { 0.0 };
        // 头部右侧两个按钮 h-9
        let cap_w = 16.0 + 8.0 + text::measure("捕获", TextStyle::Label) + 24.0;
        let ref_w = 16.0 + 8.0 + text::measure("刷新", TextStyle::Label) + 24.0;
        out.entries.push((
            Rect::new(
                col.right - cap_w,
                actions_top,
                col.right,
                actions_top + 36.0,
            ),
            Hit::Capture,
        ));
        out.entries.push((
            Rect::new(
                col.right - cap_w - 8.0 - ref_w,
                actions_top,
                col.right - cap_w - 8.0,
                actions_top + 36.0,
            ),
            Hit::Refresh,
        ));
        // 分段按钮
        let mut y = top + if compact { 130.0 } else { 88.0 };
        let seg_a = text::measure("待整理", TextStyle::Label) + 24.0;
        let seg_b = text::measure("已归档", TextStyle::Label) + 24.0;
        out.entries.push((
            Rect::new(col.left + 2.0, y + 2.0, col.left + 2.0 + seg_a, y + 34.0),
            Hit::TabInbox,
        ));
        out.entries.push((
            Rect::new(
                col.left + 2.0 + seg_a,
                y + 2.0,
                col.left + 2.0 + seg_a + seg_b,
                y + 34.0,
            ),
            Hit::TabArchived,
        ));
        if s.tab == Tab::Inbox && !s.libraries.is_empty() {
            let name = s
                .libraries
                .get(s.target_library)
                .map(|l| l.0.as_str())
                .unwrap_or("");
            let w =
                (text::measure(name, TextStyle::Label) + 44.0).min((col.width() - 68.0).min(240.0));
            if col.width() < seg_a + seg_b + w + 88.0 {
                y += 44.0;
            }
            out.entries.push((
                Rect::new(col.right - w, y, col.right, y + 36.0),
                Hit::TargetLibrary,
            ));
        }
        y += 36.0 + 16.0;
        if !s.error.is_empty() {
            y += 40.0 + 12.0;
        }
        let card_top = y;
        let visible = s.visible();
        let mut cy = card_top;
        if visible.is_empty() {
            cy += 56.0 + 48.0 + 12.0 + 20.0 + 8.0 + 20.0 + 56.0;
        }
        for (i, item) in visible.iter().enumerate() {
            let editing = s
                .editing
                .as_ref()
                .map(|(id, _)| id == &item.id)
                .unwrap_or(false);
            let h = row_height(item, editing);
            let r = Rect::new(col.left, cy, col.right, cy + h);
            out.rows.push((i, r));
            let mut bx = r.right - 16.0;
            let mut button = |hit: Hit, out: &mut Layout| {
                out.entries
                    .push((Rect::new(bx - 32.0, r.top + 8.0, bx, r.top + 40.0), hit));
                bx -= 36.0;
            };
            if editing {
                button(Hit::CancelEdit(i), &mut out);
                button(Hit::SaveEdit(i), &mut out);
                out.entries.push((
                    Rect::new(
                        r.left + 16.0,
                        r.top + 12.0,
                        r.right - 16.0 - 72.0 - 8.0,
                        r.bottom - 12.0 - 8.0 - 16.0,
                    ),
                    Hit::EditField,
                ));
            } else {
                button(Hit::Delete(i), &mut out);
                if s.tab == Tab::Inbox {
                    button(Hit::Edit(i), &mut out);
                    button(Hit::ToTask(i), &mut out);
                    button(Hit::Archive(i), &mut out);
                }
                if item.archived_path.is_some() || item.web_clip.is_some() {
                    let label_w = 14.0
                        + 4.0
                        + text::measure(&derive_title(&item.content), TextStyle::Caption);
                    let meta_x = r.left
                        + 16.0
                        + text::measure(
                            &relative_time(item.created_at, chrono::Utc::now().timestamp_millis()),
                            TextStyle::Caption,
                        )
                        + 8.0
                        + 8.0
                        + 8.0;
                    out.entries.push((
                        Rect::new(
                            meta_x,
                            r.bottom - 12.0 - 16.0,
                            (meta_x + label_w).min(r.right - 16.0),
                            r.bottom - 12.0,
                        ),
                        Hit::OpenArchived(i),
                    ));
                }
            }
            cy += h + 1.0;
        }
        out.card = Rect::new(col.left, card_top, col.right, cy.max(card_top + 1.0));
        out.content_height = cy + s.scroll - area.top + 24.0;
        out.entries.retain(|(r, h)| match h {
            Hit::Refresh | Hit::Capture | Hit::TabInbox | Hit::TabArchived | Hit::TargetLibrary => {
                true
            }
            _ => r.bottom > area.top && r.top < area.bottom,
        });
        out.rows
            .retain(|(_, r)| r.bottom > area.top && r.top < area.bottom);
        out
    }

    pub fn paint(
        list: &mut DrawList,
        area: Rect,
        s: &mut State,
        lay: &Layout,
        focused: bool,
        now_ms: i64,
        p: &Palette,
    ) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        list.rect(area, p.area_main_default);
        let col = super::super::workspace_ui::column(area, 1120.0);
        let top = col.top - s.scroll;
        paint_header(
            list,
            Rect::new(col.left, top, col.right, col.bottom),
            "收件箱",
            p,
        );
        list.text(
            Rect::new(col.left, top + 44.0, col.right, top + 64.0),
            format!(
                "{} 条待整理 · 先留住想法，再为它找到位置。",
                s.items.iter().filter(|i| i.status == "inbox").count()
            ),
            TextStyle::Caption,
            p.muted,
        );
        if let Some(r) = lay.rect_of(Hit::Refresh) {
            if s.hover == Some(Hit::Refresh) {
                list.rounded_rect(r, 6.0, p.surface_muted);
            }
            list.rounded_border(r, 6.0, p.border);
            list.icon_centered(
                Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
                Icon::REFRESH_CW,
                16.0,
                p.muted,
            );
            list.text(
                Rect::new(r.left + 36.0, r.top, r.right, r.bottom),
                "刷新",
                TextStyle::Label,
                p.muted,
            );
        }
        if let Some(r) = lay.rect_of(Hit::Capture) {
            list.rounded_rect(r, 7.0, p.foreground);
            list.icon_centered(
                Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
                Icon::PLUS,
                16.0,
                p.surface,
            );
            list.text_run(
                Rect::new(r.left + 36.0, r.top, r.right, r.bottom),
                "捕获",
                TextStyle::Label,
                p.surface,
                Align::Leading,
                Emphasis::Bold,
            );
        }
        if let (Some(a), Some(b)) = (lay.rect_of(Hit::TabInbox), lay.rect_of(Hit::TabArchived)) {
            for (r, label, tab) in [(a, "待整理", Tab::Inbox), (b, "已归档", Tab::Archived)] {
                let active = s.tab == tab;
                super::super::workspace_ui::tab(
                    list,
                    r,
                    label,
                    active,
                    s.hover
                        == Some(if tab == Tab::Inbox {
                            Hit::TabInbox
                        } else {
                            Hit::TabArchived
                        }),
                    p,
                );
            }
        }
        if let Some(r) = lay.rect_of(Hit::TargetLibrary) {
            let name = s
                .libraries
                .get(s.target_library)
                .map(|l| l.0.clone())
                .unwrap_or_default();
            list.text_aligned(
                Rect::new(r.left - 60.0, r.top, r.left - 8.0, r.bottom),
                "归档到",
                TextStyle::Label,
                p.muted,
                Align::Trailing,
            );
            list.rounded_rect(r, 6.0, p.surface);
            list.rounded_border(r, 6.0, p.border);
            list.text(
                Rect::new(r.left + 8.0, r.top, r.right - 24.0, r.bottom),
                text::ellipsize(&name, TextStyle::Label, (r.width() - 36.0).max(0.0)),
                TextStyle::Label,
                p.foreground,
            );
            list.icon_centered(
                Rect::new(r.right - 22.0, r.top, r.right - 8.0, r.bottom),
                Icon::CHEVRON_DOWN,
                14.0,
                p.muted,
            );
        }
        if !s.error.is_empty() {
            let r = Rect::new(
                col.left,
                lay.card.top - 12.0 - 40.0,
                col.right,
                lay.card.top - 12.0,
            );
            list.rounded_rect(r, 6.0, theme::mix(p.danger, p.surface, 0.08));
            list.rounded_border(r, 6.0, theme::mix(p.danger, p.surface, 0.25));
            list.text(
                Rect::new(r.left + 12.0, r.top, r.right - 12.0, r.bottom),
                text::ellipsize(&s.error, TextStyle::Label, r.width() - 24.0),
                TextStyle::Label,
                p.danger,
            );
        }
        // 卡片
        let card = lay.card;
        list.hline(card.left, card.right, card.top, p.border);
        // 拷一份：下面画编辑框要 `&mut s.editing`，不能同时借着 `s.items`
        let visible: Vec<CaptureItem> = s.visible().into_iter().cloned().collect();
        if visible.is_empty() {
            let cy = card.top + 56.0;
            let cx = (card.left + card.right) / 2.0;
            list.rounded_rect(
                Rect::new(cx - 24.0, cy, cx + 24.0, cy + 48.0),
                10.0,
                p.surface_muted,
            );
            list.icon_centered(
                Rect::new(cx - 24.0, cy, cx + 24.0, cy + 48.0),
                Icon::INBOX,
                20.0,
                p.muted,
            );
            let (title, desc) = if s.tab == Tab::Inbox {
                ("收件箱已清空", "想到什么先记下来，不用先想好放在哪儿。")
            } else {
                (
                    "还没有归档过的条目",
                    "分拣过的想法会留在这里，方便回头查证它去了哪篇笔记。",
                )
            };
            list.text_run(
                Rect::new(card.left, cy + 60.0, card.right, cy + 80.0),
                title,
                TextStyle::Label,
                p.foreground,
                Align::Center,
                Emphasis::Bold,
            );
            list.text_aligned(
                Rect::new(card.left, cy + 88.0, card.right, cy + 108.0),
                desc,
                TextStyle::Label,
                p.muted,
                Align::Center,
            );
        }
        for (i, r) in &lay.rows {
            let item = &visible[*i];
            if *i > 0 {
                list.hline(card.left + 1.0, card.right - 1.0, r.top - 1.0, p.border);
            }
            let editing = s
                .editing
                .as_ref()
                .map(|(id, _)| id == &item.id)
                .unwrap_or(false);
            let text_right = r.right
                - 16.0
                - if editing {
                    72.0
                } else if s.tab == Tab::Inbox {
                    144.0
                } else {
                    36.0
                };
            if editing {
                if let (Some(fr), Some((_, field))) =
                    (lay.rect_of(Hit::EditField), s.editing.as_mut())
                {
                    list.rounded_rect(fr, 6.0, p.background);
                    list.rounded_border(fr, 6.0, if focused { p.accent } else { p.border });
                    field.paint(
                        list,
                        Rect::new(fr.left + 8.0, fr.top + 4.0, fr.right - 8.0, fr.top + 32.0),
                        focused,
                        p,
                        FieldLook::bare(),
                    );
                }
            } else {
                let mut ly = r.top + 12.0 + if item.web_clip.is_some() { 22.0 } else { 0.0 };
                for line in item.content.lines().take(12) {
                    list.text(
                        Rect::new(r.left + 16.0, ly, text_right, ly + 22.0),
                        text::ellipsize(line, TextStyle::Label, text_right - r.left - 16.0),
                        TextStyle::Label,
                        p.foreground,
                    );
                    ly += 22.0;
                }
            }
            // 元信息行
            if let Some(meta) = item.web_clip.as_ref().filter(|_| !editing) {
                let label = format!("网页 · {} · {}", meta.format.to_uppercase(), meta.url);
                list.text(
                    Rect::new(r.left + 16.0, r.top + 12.0, text_right, r.top + 28.0),
                    text::ellipsize(
                        &label,
                        TextStyle::Caption,
                        (text_right - r.left - 16.0).max(0.0),
                    ),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            let meta_y = r.bottom - 12.0 - 16.0;
            let when = relative_time(item.created_at, now_ms);
            let mut mx = r.left + 16.0;
            list.text(
                Rect::new(mx, meta_y, r.right, meta_y + 16.0),
                when.clone(),
                TextStyle::Caption,
                p.muted,
            );
            mx += text::measure(&when, TextStyle::Caption) + 8.0;
            if item.source == "quick-capture" {
                list.text(
                    Rect::new(mx, meta_y, r.right, meta_y + 16.0),
                    "· 快速捕获",
                    TextStyle::Caption,
                    p.muted,
                );
                mx += text::measure("· 快速捕获", TextStyle::Caption) + 8.0;
            }
            if let Some(a) = lay.rect_of(Hit::OpenArchived(*i)) {
                list.text(
                    Rect::new(mx, meta_y, mx + 8.0, meta_y + 16.0),
                    "·",
                    TextStyle::Caption,
                    p.muted,
                );
                list.icon_centered(
                    Rect::new(a.left, a.top, a.left + 14.0, a.bottom),
                    Icon::ARCHIVE_RESTORE,
                    14.0,
                    p.accent,
                );
                list.text(
                    Rect::new(a.left + 18.0, a.top, a.right, a.bottom),
                    derive_title(&item.content),
                    TextStyle::Caption,
                    p.accent,
                );
            }
            let icon_for = |h: Hit| match h {
                Hit::Archive(_) => Some((Icon::ARCHIVE, p.muted)),
                Hit::ToTask(_) => Some((Icon::CHECK_SQUARE, p.muted)),
                Hit::Edit(_) => Some((Icon::PEN_LINE, p.muted)),
                Hit::Delete(_) => Some((Icon::TRASH2, p.muted)),
                Hit::SaveEdit(_) => Some((Icon::CHECK, p.muted)),
                Hit::CancelEdit(_) => Some((Icon::X, p.muted)),
                _ => None,
            };
            for (br, h) in lay
                .entries
                .iter()
                .filter(|(br, _)| br.top >= r.top && br.bottom <= r.bottom)
            {
                if let Some((icon, color)) = icon_for(*h) {
                    if s.hover == Some(*h) {
                        list.rounded_rect(*br, 6.0, p.surface_muted);
                    }
                    list.icon_centered(*br, icon, 16.0, color);
                }
            }
        }
        list.pop_clip();
    }
}

// =====================================================================
// 最近
// =====================================================================

pub mod recent {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Doc {
        pub path: PathBuf,
        pub name: String,
        pub title: String,
        pub parent: String,
        pub library: Option<String>,
        pub mtime_ms: i64,
        pub size: u64,
        pub type_label: &'static str,
    }

    pub struct State {
        pub docs: Vec<Doc>,
        pub query: TextField,
        pub scroll: f32,
        pub loaded: bool,
        pub error: String,
    }

    impl Default for State {
        fn default() -> Self {
            let mut query = TextField::new("搜索文件名、路径或知识库");
            query.style = TextStyle::Label;
            State {
                docs: Vec::new(),
                query,
                scroll: 0.0,
                loaded: false,
                error: String::new(),
            }
        }
    }

    impl State {
        pub fn filtered(&self) -> Vec<&Doc> {
            let q = self.query.text().trim().to_lowercase();
            if q.is_empty() {
                return self.docs.iter().collect();
            }
            self.docs
                .iter()
                .filter(|d| {
                    d.title.to_lowercase().contains(&q)
                        || d.name.to_lowercase().contains(&q)
                        || d.path
                            .to_string_lossy()
                            .replace('\\', "/")
                            .to_lowercase()
                            .contains(&q)
                        || d.library
                            .as_deref()
                            .map(|l| l.to_lowercase().contains(&q))
                            .unwrap_or(false)
                })
                .collect()
        }
    }

    /// `detectFileType` 的文档类别 + `getFileTypeInfo().description`。
    pub fn type_label(name: &str) -> Option<&'static str> {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        Some(match ext.as_str() {
            "md" | "mc" | "mdx" | "markdown" => "Markdown 文档",
            "txt" => "纯文本",
            "pdf" => "PDF 文档",
            "doc" | "docx" => "Word 文档",
            "html" | "htm" => "HTML 页面",
            _ if name.ends_with(".link.json") => "链接",
            _ => return None,
        })
    }

    /// `formatFileSize`。
    pub fn format_size(bytes: u64) -> String {
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
        }
    }

    /// 只收集属于可见知识库的文档并按修改时间倒序。
    pub fn collect(
        nodes: &[FileNode],
        workspace: &Path,
        libraries: &[(String, PathBuf)],
    ) -> Vec<Doc> {
        fn walk(
            nodes: &[FileNode],
            workspace: &Path,
            libraries: &[(String, PathBuf)],
            out: &mut Vec<Doc>,
        ) {
            for n in nodes {
                if let Some(children) = &n.children {
                    walk(children, workspace, libraries, out);
                }
                if n.is_directory() {
                    continue;
                }
                let Some(type_label) = type_label(&n.name) else {
                    continue;
                };
                let path = PathBuf::from(&n.path);
                let norm = n.path.replace('\\', "/").to_lowercase();
                let library = libraries
                    .iter()
                    .find(|(_, lp)| {
                        let lp = lp.to_string_lossy().replace('\\', "/").to_lowercase();
                        let lp = lp.trim_end_matches('/');
                        norm == lp || norm.starts_with(&format!("{lp}/"))
                    })
                    .map(|(name, _)| name.clone());
                // 库名缺失意味着该文档不在知识库目录树中（例如内置 Agent Skills）。
                if library.is_none() {
                    continue;
                }
                let parent_full = path
                    .parent()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let root = workspace.to_string_lossy().replace('\\', "/");
                let parent = parent_full
                    .strip_prefix(&format!("{root}/"))
                    .map(str::to_owned)
                    .unwrap_or(parent_full);
                let title = match n.name.rfind('.') {
                    Some(i) if i > 0 => n.name[..i].to_owned(),
                    _ => n.name.clone(),
                };
                out.push(Doc {
                    path,
                    name: n.name.clone(),
                    title,
                    parent,
                    library,
                    mtime_ms: n
                        .mtime
                        .as_deref()
                        .and_then(mochi_core::jstime::try_parse_millis)
                        .unwrap_or(0),
                    size: n.size.unwrap_or(0).max(0) as u64,
                    type_label,
                });
            }
        }
        let mut out = Vec::new();
        walk(nodes, workspace, libraries, &mut out);
        out.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms));
        out
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        Refresh,
        Query,
        Doc(usize),
        Blank,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Layout {
        pub entries: Vec<(Rect, Hit)>,
        pub card: Rect,
        pub content_height: f32,
        pub viewport: Rect,
    }

    impl Layout {
        pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
            if let Some((_, h)) = self.entries.iter().rev().find(|(r, _)| r.contains(x, y)) {
                return Some(*h);
            }
            self.viewport.contains(x, y).then_some(Hit::Blank)
        }
        pub fn rect_of(&self, h: Hit) -> Option<Rect> {
            self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
        }
        pub fn max_scroll(&self) -> f32 {
            (self.content_height - self.viewport.height()).max(0.0)
        }
    }

    const ROW_H: f32 = 12.0 + 36.0 + 12.0;

    pub fn layout(s: &State, area: Rect) -> Layout {
        let mut out = Layout::default();
        if area.is_empty() {
            return out;
        }
        out.viewport = area;
        let col = column(area, 1120.0);
        let top = col.top - s.scroll;
        let ref_w = 16.0 + 8.0 + text::measure("刷新", TextStyle::Label) + 24.0;
        out.entries.push((
            Rect::new(
                col.right - ref_w,
                top + header_height() - 20.0 - 36.0,
                col.right,
                top + header_height() - 20.0,
            ),
            Hit::Refresh,
        ));
        let mut y = top + header_height();
        out.entries
            .push((Rect::new(col.left, y, col.right, y + 40.0), Hit::Query));
        y += 40.0 + 16.0;
        let card_top = y;
        y += 44.0 + 1.0; // 卡片头「文档记录」
        let docs = s.filtered();
        if docs.is_empty() {
            y += 60.0;
        }
        for i in 0..docs.len() {
            out.entries
                .push((Rect::new(col.left, y, col.right, y + ROW_H), Hit::Doc(i)));
            y += ROW_H + 1.0;
        }
        out.card = Rect::new(col.left, card_top, col.right, y);
        out.content_height = y + s.scroll - area.top + 24.0;
        out.entries.retain(|(r, h)| match h {
            Hit::Doc(_) => r.bottom > area.top && r.top < area.bottom,
            _ => true,
        });
        out
    }

    pub fn paint(
        list: &mut DrawList,
        area: Rect,
        s: &mut State,
        lay: &Layout,
        focused: bool,
        now_ms: i64,
        p: &Palette,
    ) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        list.rect(area, p.area_main_default);
        let col = column(area, 1120.0);
        let top = col.top - s.scroll;
        paint_header(
            list,
            Rect::new(col.left, top, col.right, col.bottom),
            "最近",
            p,
        );
        if let Some(r) = lay.rect_of(Hit::Refresh) {
            list.rounded_rect(r, 6.0, p.surface);
            list.rounded_border(r, 6.0, p.border);
            list.icon_centered(
                Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
                Icon::REFRESH_CW,
                16.0,
                p.muted,
            );
            list.text(
                Rect::new(r.left + 36.0, r.top, r.right, r.bottom),
                "刷新",
                TextStyle::Label,
                p.muted,
            );
        }
        if let Some(r) = lay.rect_of(Hit::Query) {
            list.rounded_rect(r, 8.0, p.surface);
            list.rounded_border(r, 8.0, if focused { p.accent } else { p.border });
            list.icon_centered(
                Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
                Icon::SEARCH,
                16.0,
                p.muted,
            );
            s.query.paint(
                list,
                Rect::new(r.left + 40.0, r.top, r.right - 12.0, r.bottom),
                focused,
                p,
                FieldLook::bare(),
            );
        }
        let card = lay.card;
        list.rounded_rect(card, 8.0, p.surface);
        list.rounded_border(card, 8.0, p.border);
        let docs = s.filtered();
        list.text_run(
            Rect::new(
                card.left + 16.0,
                card.top,
                card.right - 16.0,
                card.top + 44.0,
            ),
            "文档记录",
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        list.text_aligned(
            Rect::new(
                card.left + 16.0,
                card.top,
                card.right - 16.0,
                card.top + 44.0,
            ),
            format!("{} 个", docs.len()),
            TextStyle::Caption,
            p.muted,
            Align::Trailing,
        );
        list.hline(card.left + 1.0, card.right - 1.0, card.top + 44.0, p.border);
        if !s.error.is_empty() {
            list.text(
                Rect::new(
                    card.left + 16.0,
                    card.top + 45.0,
                    card.right - 16.0,
                    card.top + 105.0,
                ),
                s.error.clone(),
                TextStyle::Label,
                0xEF4444,
            );
        } else if docs.is_empty() {
            let msg = if s.query.text().trim().is_empty() {
                "还没有可展示的文档。"
            } else {
                "没有匹配的文档。"
            };
            list.text(
                Rect::new(
                    card.left + 16.0,
                    card.top + 45.0,
                    card.right - 16.0,
                    card.top + 105.0,
                ),
                msg,
                TextStyle::Label,
                p.muted,
            );
        }
        for (r, h) in &lay.entries {
            let Hit::Doc(i) = h else { continue };
            let d = docs[*i];
            if *i > 0 {
                list.hline(card.left + 1.0, card.right - 1.0, r.top - 1.0, p.border);
            }
            let icon_box = Rect::new(r.left + 16.0, r.top + 12.0, r.left + 52.0, r.top + 48.0);
            list.rounded_rect(icon_box, 6.0, theme::mix(p.accent, p.surface, 0.10));
            list.icon_centered(icon_box, Icon::FILE_TEXT, 16.0, p.accent);
            let text_left = r.left + 64.0;
            let right_w = 160.0;
            let text_right = r.right - 16.0 - right_w;
            list.text_run(
                Rect::new(text_left, r.top + 10.0, text_right, r.top + 30.0),
                text::ellipsize(&d.title, TextStyle::Label, text_right - text_left),
                TextStyle::Label,
                p.foreground,
                Align::Leading,
                Emphasis::Bold,
            );
            let mut sub = String::new();
            if let Some(lib) = &d.library {
                sub.push_str(lib);
                sub.push_str(" · ");
            }
            sub.push_str(&d.parent);
            list.icon_centered(
                Rect::new(text_left, r.top + 32.0, text_left + 14.0, r.top + 48.0),
                Icon::FOLDER_OPEN,
                14.0,
                p.muted,
            );
            list.text(
                Rect::new(text_left + 18.0, r.top + 32.0, text_right, r.top + 48.0),
                text::ellipsize(&sub, TextStyle::Caption, text_right - text_left - 18.0),
                TextStyle::Caption,
                p.muted,
            );
            // 右侧：时间 + 类型/大小
            list.text_aligned(
                Rect::new(text_right, r.top + 10.0, r.right - 16.0, r.top + 30.0),
                relative_time(d.mtime_ms, now_ms),
                TextStyle::Label,
                p.foreground,
                Align::Trailing,
            );
            list.text_aligned(
                Rect::new(text_right, r.top + 32.0, r.right - 16.0, r.top + 48.0),
                format!("{} · {}", d.type_label, format_size(d.size)),
                TextStyle::Caption,
                p.muted,
                Align::Trailing,
            );
        }
        list.pop_clip();
    }
}

// =====================================================================
// 收藏
// =====================================================================

/// 收藏摘要与最近视图共用文档资料结构，但数据来源独立：收藏路径可能指向图片、
/// 表格或多维表格，而 `recent::collect` 只收集少数笔记类型。
pub mod favorites {
    use super::recent;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::time::UNIX_EPOCH;

    #[derive(Default)]
    pub struct State {
        pub docs: Vec<recent::Doc>,
        pub loaded: bool,
        pub error: String,
    }

    /// `open_file` 可交给文本编辑器或文件查看器处理的文件，都允许出现在收藏里。
    /// 未知后缀也保留为「文件」：查看器会给出自己的不支持提示，收藏页不应丢掉用户
    /// 明确收藏的路径。
    pub fn type_label(name: &str) -> &'static str {
        let lower = name.to_lowercase();
        if lower.ends_with(".link.json") {
            return "链接";
        }
        match lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("") {
            "mcb" => "多维表格",
            "exam" => "练习",
            "md" | "markdown" | "mc" | "mdx" => "Markdown 文档",
            "txt" => "纯文本",
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "ico" => "图片",
            "pdf" => "PDF 文档",
            "doc" | "docx" => "Word 文档",
            "xls" | "xlsx" => "Excel 表格",
            "ppt" | "pptx" => "演示文稿",
            "html" | "htm" => "HTML 页面",
            "js" | "jsx" | "ts" | "tsx" | "py" | "rb" | "java" | "c" | "cpp" | "cs" | "go"
            | "rs" | "php" | "css" | "scss" | "json" | "xml" | "yaml" | "yml" | "sh" | "bash"
            | "sql" | "swift" | "kt" | "r" => "代码文件",
            _ => "文件",
        }
    }

    fn normalized(path: &Path) -> String {
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    }

    fn resolved(path: &Path, workspace: &Path) -> PathBuf {
        // `Path::is_absolute` 使用当前宿主系统的路径规则。测试和导入的数据中
        // 可能包含 Windows 盘符路径，即使代码正在另一种操作系统上运行。
        let raw = path.to_string_lossy();
        let windows_absolute = raw.len() >= 3
            && raw.as_bytes().get(1) == Some(&b':')
            && matches!(raw.as_bytes().get(2), Some(b'/') | Some(b'\\'));
        if path.is_absolute() || windows_absolute {
            path.to_path_buf()
        } else {
            workspace.join(path)
        }
    }

    fn relative_parent(path: &Path, workspace: &Path) -> String {
        let parent = path.parent().map(normalized).unwrap_or_default();
        let root = normalized(workspace);
        parent
            .strip_prefix(&root)
            .map(|rest| rest.trim_matches('/').to_owned())
            .unwrap_or(parent)
    }

    fn modified_ms(metadata: &std::fs::Metadata) -> i64 {
        metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }

    /// 从持久化收藏路径读取当前仍存在的文件。
    ///
    /// 这里故意不回写收藏存储：文件暂时不存在时只隐藏这一行，等用户修复路径
    /// 或外部同步完成后仍可再次显示。返回值沿用最近页的 `Doc`，便于打开逻辑复用。
    pub fn collect(
        paths: &[PathBuf],
        workspace: &Path,
        libraries: &[(String, PathBuf)],
    ) -> Vec<recent::Doc> {
        let mut seen = HashSet::new();
        let mut docs = Vec::new();
        for path in paths {
            let path = resolved(path, workspace);
            if !seen.insert(normalized(&path)) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let title = match name.rfind('.') {
                Some(index) if index > 0 => name[..index].to_owned(),
                _ => name.clone(),
            };
            let path_key = normalized(&path);
            let library = libraries
                .iter()
                .find(|(_, library_path)| {
                    let library_key = normalized(library_path);
                    path_key == library_key || path_key.starts_with(&format!("{library_key}/"))
                })
                .map(|(name, _)| name.clone());
            docs.push(recent::Doc {
                parent: relative_parent(&path, workspace),
                name: name.clone(),
                title,
                library,
                mtime_ms: modified_ms(&metadata),
                size: metadata.len(),
                type_label: type_label(&name),
                path,
            });
        }
        docs.sort_by(|a, b| {
            b.mtime_ms
                .cmp(&a.mtime_ms)
                .then_with(|| a.path.cmp(&b.path))
        });
        docs
    }
}

// =====================================================================
// 小记
// =====================================================================

pub mod quick_note {
    use super::*;
    use std::io::Write;

    /// 小记文件：`<工作区>/小记.md`，不存在就建（`# 小记\n\n`）。
    pub fn ensure(workspace: &Path) -> std::io::Result<PathBuf> {
        let markdown = workspace.join("小记.md");
        // 带富文本标记的保存可能把 Markdown 升级为 `.mc`。优先保留仍在
        // 工作区中的 Markdown；只有它不存在时才复用编译格式，避免再次
        // 进小记时凭空创建一份空的 `小记.md`。
        if markdown.is_file() {
            return Ok(markdown);
        }
        let compiled = workspace.join("小记.mc");
        if compiled.is_file() {
            return Ok(compiled);
        }

        // 调用 `exists` 后直接 `write`，并发创建时可能会截断另一实例刚写入的内容。
        // create_new 把「创建」变成一次原子操作；撞到并发创建者时复用其文件。
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&markdown)
        {
            Ok(mut file) => {
                file.write_all("# 小记\n\n".as_bytes())?;
                file.flush()?;
                Ok(markdown)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if markdown.is_file() {
                    Ok(markdown)
                } else if compiled.is_file() {
                    Ok(compiled)
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const AREA: Rect = Rect {
        left: 220.0,
        top: 32.0,
        right: 1200.0,
        bottom: 800.0,
    };

    fn texts(list: &DrawList) -> Vec<String> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn capture(id: &str, content: &str, status: &str, created: i64) -> CaptureItem {
        CaptureItem {
            id: id.into(),
            content: content.into(),
            created_at: created,
            updated_at: created,
            source: "quick-capture".into(),
            status: status.into(),
            archived_path: None,
            archived_at: None,
            web_clip: None,
        }
    }

    #[test]
    fn relative_time_matches_the_electron_thresholds() {
        let now = 1_756_800_000_000;
        assert_eq!(relative_time(now - 30_000, now), "刚刚");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5 分钟前");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3 小时前");
        assert_eq!(relative_time(now - 2 * 86_400_000, now), "2 天前");
        assert!(relative_time(now - 30 * 86_400_000, now).contains('/'));
    }

    #[test]
    fn the_inbox_tab_shows_only_inbox_items_newest_first_with_four_actions() {
        let mut s = inbox::State::default();
        s.items = vec![
            capture("a", "早的", "inbox", 1),
            capture("b", "晚的\n第二行", "inbox", 2),
            capture("c", "归档了", "archived", 3),
        ];
        s.libraries = vec![(
            "计算机通识".into(),
            PathBuf::from("D:/ws/知识库/计算机通识"),
        )];
        let v = s.visible();
        assert_eq!(
            v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        let lay = inbox::layout(&s, AREA);
        assert_eq!(lay.rows.len(), 2);
        for h in [
            inbox::Hit::Archive(0),
            inbox::Hit::ToTask(0),
            inbox::Hit::Edit(0),
            inbox::Hit::Delete(0),
        ] {
            assert!(lay.rect_of(h).is_some(), "{h:?}");
        }
        assert!(lay.rect_of(inbox::Hit::TargetLibrary).is_some());
        let mut list = DrawList::new();
        inbox::paint(
            &mut list,
            AREA,
            &mut s,
            &lay,
            false,
            10_000,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(
            t.contains(&"收件箱".to_owned())
                && t.contains(&"晚的".to_owned())
                && t.contains(&"第二行".to_owned())
        );
        assert!(t.contains(&"计算机通识".to_owned()));
        assert!(list.finish().is_ok());

        s.tab = inbox::Tab::Archived;
        let lay = inbox::layout(&s, AREA);
        assert_eq!(lay.rows.len(), 1);
        assert!(
            lay.rect_of(inbox::Hit::Archive(0)).is_none(),
            "已归档页没有归档按钮"
        );
        assert!(lay.rect_of(inbox::Hit::Delete(0)).is_some());
    }

    #[test]
    fn editing_replaces_the_row_content_with_a_field_and_swaps_the_buttons() {
        let mut s = inbox::State::default();
        s.items = vec![capture("a", "原文", "inbox", 1)];
        s.start_edit(&s.items[0].clone());
        let lay = inbox::layout(&s, AREA);
        assert!(lay.rect_of(inbox::Hit::EditField).is_some());
        assert!(
            lay.rect_of(inbox::Hit::SaveEdit(0)).is_some()
                && lay.rect_of(inbox::Hit::CancelEdit(0)).is_some()
        );
        assert!(lay.rect_of(inbox::Hit::Edit(0)).is_none());
        assert_eq!(s.editing.as_ref().unwrap().1.text(), "原文");
    }

    #[test]
    fn an_empty_inbox_says_so_and_the_empty_state_differs_per_tab() {
        let mut s = inbox::State::default();
        let lay = inbox::layout(&s, AREA);
        let mut list = DrawList::new();
        inbox::paint(
            &mut list,
            AREA,
            &mut s,
            &lay,
            false,
            0,
            theme::tokens().palette(false),
        );
        assert!(texts(&list).contains(&"收件箱已清空".to_owned()));
        s.tab = inbox::Tab::Archived;
        let lay = inbox::layout(&s, AREA);
        let mut list = DrawList::new();
        inbox::paint(
            &mut list,
            AREA,
            &mut s,
            &lay,
            false,
            0,
            theme::tokens().palette(false),
        );
        assert!(texts(&list).contains(&"还没有归档过的条目".to_owned()));
    }

    fn node(name: &str, path: &str, dir: bool, mtime: &str, children: Vec<FileNode>) -> FileNode {
        FileNode {
            id: path.into(),
            name: name.into(),
            path: path.into(),
            kind: if dir {
                "directory".into()
            } else {
                "file".into()
            },
            children: dir.then_some(children),
            mtime: (!dir).then(|| mtime.to_owned()),
            size: (!dir).then_some(2048),
            ..Default::default()
        }
    }

    #[test]
    fn recent_collects_documents_newest_first_and_labels_their_library() {
        let ws = Path::new("D:/ws");
        let libs = vec![(
            "计算机通识".to_owned(),
            PathBuf::from("D:/ws/知识库/计算机通识"),
        )];
        let tree = vec![node(
            "知识库",
            "D:/ws/知识库",
            true,
            "",
            vec![node(
                "计算机通识",
                "D:/ws/知识库/计算机通识",
                true,
                "",
                vec![
                    node(
                        "旧.md",
                        "D:/ws/知识库/计算机通识/旧.md",
                        false,
                        "2026-01-01T00:00:00.000Z",
                        vec![],
                    ),
                    node(
                        "新.md",
                        "D:/ws/知识库/计算机通识/新.md",
                        false,
                        "2026-09-01T00:00:00.000Z",
                        vec![],
                    ),
                    node(
                        "图.png",
                        "D:/ws/知识库/计算机通识/图.png",
                        false,
                        "2026-09-02T00:00:00.000Z",
                        vec![],
                    ),
                ],
            )],
        )];
        let docs = recent::collect(&tree, ws, &libs);
        assert_eq!(
            docs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["新.md", "旧.md"],
            "图片不是文档；新的在前"
        );
        assert_eq!(docs[0].library.as_deref(), Some("计算机通识"));
        assert_eq!(docs[0].parent, "知识库/计算机通识");
        assert_eq!(docs[0].title, "新");
        assert_eq!(recent::format_size(2048), "2.0 KB");
        assert_eq!(recent::type_label("a.pdf"), Some("PDF 文档"));
        assert_eq!(recent::type_label("a.link.json"), Some("链接"));

        let mut s = recent::State::default();
        s.docs = docs;
        let lay = recent::layout(&s, AREA);
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, recent::Hit::Doc(_)))
                .count(),
            2
        );
        s.query.set_text("旧");
        assert_eq!(s.filtered().len(), 1);
        let lay = recent::layout(&s, AREA);
        let mut list = DrawList::new();
        recent::paint(
            &mut list,
            AREA,
            &mut s,
            &lay,
            false,
            1_756_800_000_000,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        // 当前页头已是「最近」；结果数验证实际可操作的行，而非已移除的计数文案。
        assert!(t.contains(&"最近".to_owned()) && t.contains(&"旧".to_owned()));
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, recent::Hit::Doc(_)))
                .count(),
            1
        );
        assert!(list.finish().is_ok());
    }

    #[test]
    fn recent_excludes_documents_outside_visible_libraries() {
        let ws = Path::new("D:/ws");
        // 重新加载最近项目时，会略过隐藏的 ai-prompts 资料库。
        let libs = vec![
            ("学习".to_owned(), PathBuf::from("D:/ws/知识库/学习/")),
            ("工作".to_owned(), PathBuf::from("D:/ws/知识库/工作")),
        ];
        let tree = [
            ("笔记.md", "D:/ws/知识库/学习/课程/笔记.md"),
            ("计划.mc", "d:\\ws\\知识库\\工作\\计划.mc"),
            ("SKILL.md", "D:/ws/Agent配置/Skills/example/SKILL.md"),
            ("agent.md", "D:/ws/AI Prompts/Agents/agent.md"),
            ("日记.md", "D:/ws/日记/日记.md"),
            ("根目录.md", "D:/ws/根目录.md"),
            ("旁支.md", "D:/ws/知识库/学习备份/旁支.md"),
        ]
        .map(|(name, path)| node(name, path, false, "2026-09-01T00:00:00.000Z", vec![]));

        let docs = recent::collect(&tree, ws, &libs);
        assert_eq!(
            docs.iter().map(|doc| doc.name.as_str()).collect::<Vec<_>>(),
            vec!["笔记.md", "计划.mc"]
        );
        assert!(recent::collect(&tree, ws, &[]).is_empty());
    }

    #[test]
    fn the_quick_note_file_is_created_once_with_the_heading_template() {
        let ws = std::env::temp_dir().join(format!(
            "mochi-quicknote-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(8)
        ));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        let path = quick_note::ensure(&ws).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# 小记\n\n");
        std::fs::write(&path, "改过").unwrap();
        quick_note::ensure(&ws).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "改过",
            "已存在的不能被覆盖"
        );
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn quick_note_reuses_compiled_file_after_markdown_upgrade() {
        let ws = std::env::temp_dir().join(format!(
            "mochi-quicknote-mc-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(8)
        ));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        let markdown = ws.join("小记.md");
        let compiled = ws.join("小记.mc");
        std::fs::write(&compiled, "<span style=\"color: red\">保留的小记</span>").unwrap();
        let path = quick_note::ensure(&ws).unwrap();
        assert_eq!(path, compiled);
        assert!(!markdown.exists(), "已有 .mc 时不能重新创建空的 .md");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "<span style=\"color: red\">保留的小记</span>"
        );

        // 如果两个路径都存在，已有 Markdown 仍是首选，保持历史行为稳定。
        std::fs::write(&markdown, "# Markdown 优先").unwrap();
        assert_eq!(quick_note::ensure(&ws).unwrap(), markdown);
        let _ = std::fs::remove_dir_all(&ws);
    }
}
