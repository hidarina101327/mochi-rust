//! 面板数据由 App 注入，不直接访问磁盘。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{Local, TimeZone};
use mochi_core::ai::agent_inbox::InboxEntry;
use mochi_core::ai::document_mounts::{AiDocumentMount, MountScope};
use mochi_core::git::GitCommitInfo;
use mochi_core::sidecars::DocumentComment;

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

fn basename(path: Option<&Path>) -> String {
    path.and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "未选择文档".to_owned())
}

/// `toLocaleString('zh-CN', {month, day, hour, minute})` → `09/02 14:05`。
pub fn format_time(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%m/%d %H:%M").to_string(),
        None => String::new(),
    }
}

/// 面板通用头部：`px-4 py-3 border-b`，图标 17 + 标题 text-sm semibold，
/// 下一行 text-xs muted 副标题，右侧 X。返回头部高度。
fn paint_header(
    list: &mut DrawList,
    area: Rect,
    icon: Icon,
    title: &str,
    subtitle: &str,
    p: &Palette,
) -> f32 {
    let h = 12.0 + 20.0 + 2.0 + 16.0 + 12.0 + 1.0;
    let head = Rect::new(area.left, area.top, area.right, area.top + h);
    list.icon_centered(
        Rect::new(
            head.left + 16.0,
            head.top + 12.0,
            head.left + 33.0,
            head.top + 32.0,
        ),
        icon,
        17.0,
        p.foreground,
    );
    list.text_run(
        Rect::new(
            head.left + 41.0,
            head.top + 12.0,
            head.right - 48.0,
            head.top + 32.0,
        ),
        title,
        TextStyle::Label,
        p.foreground,
        Align::Leading,
        Emphasis::Bold,
    );
    list.text(
        Rect::new(
            head.left + 16.0,
            head.top + 34.0,
            head.right - 48.0,
            head.top + 50.0,
        ),
        text::ellipsize(subtitle, TextStyle::Caption, head.width() - 64.0),
        TextStyle::Caption,
        p.muted,
    );
    list.icon_centered(close_rect(area), Icon::X, 16.0, p.muted);
    list.border_bottom(head, p.border);
    h
}

fn close_rect(area: Rect) -> Rect {
    Rect::new(
        area.right - 16.0 - 24.0,
        area.top + 12.0,
        area.right - 16.0,
        area.top + 36.0,
    )
}

fn centered_hint(list: &mut DrawList, area: Rect, y: f32, text: &str, p: &Palette) {
    list.text_aligned(
        Rect::new(area.left + 16.0, y, area.right - 16.0, y + 20.0),
        text,
        TextStyle::Label,
        p.muted,
        Align::Center,
    );
}

// =====================================================================
// 版本历史
// =====================================================================

pub mod version {
    use super::*;

    pub struct State {
        pub source: Option<PathBuf>,
        pub commits: Vec<GitCommitInfo>,
        /// 用户「删除」的版本只是隐藏（TSX 存 localStorage），不动仓库。
        pub deleted: HashSet<String>,
        pub message: TextField,
        pub loading: bool,
        pub error: String,
        pub scroll: f32,
        pub enabled: bool,
    }

    impl Default for State {
        fn default() -> Self {
            let mut message = TextField::new("提交信息，可留空");
            message.style = TextStyle::Label;
            State {
                source: None,
                commits: Vec::new(),
                deleted: HashSet::new(),
                message,
                loading: false,
                error: String::new(),
                scroll: 0.0,
                enabled: true,
            }
        }
    }

    impl State {
        /// 可见版本：旧 → 新（TSX：`[...commits].reverse()`），第 1 版最旧。
        pub fn versions(&self) -> Vec<&GitCommitInfo> {
            self.commits
                .iter()
                .rev()
                .filter(|c| !self.deleted.contains(&c.oid))
                .collect()
        }
    }

    /// `cleanMessage`：去掉 `auto:` / `manual:` 这类前缀。
    pub fn clean_message(m: &str) -> String {
        let lower = m.to_lowercase();
        for prefix in [
            "auto:",
            "ai-before:",
            "ai-after:",
            "manual:",
            "restore:",
            "rename:",
            "delete:",
            "import:",
        ] {
            if lower.starts_with(prefix) {
                return m[prefix.len()..].trim().to_owned();
            }
        }
        m.trim().to_owned()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        Close,
        Message,
        Record,
        Refresh,
        Open(usize),
        Rollback(usize),
        Delete(usize),
    }

    #[derive(Debug, Clone, Default)]
    pub struct Layout {
        pub entries: Vec<(Rect, Hit)>,
        pub list: Rect,
        pub content_height: f32,
    }

    impl Layout {
        pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
            self.entries
                .iter()
                .rev()
                .find(|(r, _)| r.contains(x, y))
                .map(|(_, h)| *h)
        }
        pub fn rect_of(&self, h: Hit) -> Option<Rect> {
            self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
        }
        pub fn max_scroll(&self) -> f32 {
            (self.content_height - self.list.height()).max(0.0)
        }
    }

    const HEADER: f32 = 63.0;
    const ROW: f32 = 8.0 + 20.0 + 2.0 + 16.0 + 8.0; // p-2 + 两行

    pub fn layout(s: &State, area: Rect) -> Layout {
        let mut out = Layout::default();
        if area.is_empty() {
            return out;
        }
        out.entries.push((close_rect(area), Hit::Close));
        let mut y = area.top + HEADER;
        if s.source.is_none() {
            return out;
        }
        if !s.enabled {
            y += 12.0 + 12.0 + 32.0 + 12.0; // m-3 提示框
        }
        // 提交信息区 p-3：textarea min-h-16(64) + mt-2 + 按钮 28 + p-3
        y += 12.0;
        out.entries.push((
            Rect::new(area.left + 12.0, y, area.right - 12.0, y + 64.0),
            Hit::Message,
        ));
        y += 64.0 + 8.0;
        let record_w = 13.0 + 4.0 + text::measure("记录版本", TextStyle::Caption) + 20.0;
        out.entries.push((
            Rect::new(area.left + 12.0, y, area.left + 12.0 + record_w, y + 28.0),
            Hit::Record,
        ));
        let refresh_w = text::measure("刷新", TextStyle::Caption) + 20.0;
        out.entries.push((
            Rect::new(
                area.left + 12.0 + record_w + 8.0,
                y,
                area.left + 12.0 + record_w + 8.0 + refresh_w,
                y + 28.0,
            ),
            Hit::Refresh,
        ));
        y += 28.0 + 12.0 + 1.0;
        if !s.error.is_empty() {
            y += 32.0 + 1.0;
        }
        let list = Rect::new(area.left, y, area.right, area.bottom);
        out.list = list;
        let versions = s.versions();
        let mut ly = list.top + 8.0 - s.scroll;
        for i in 0..versions.len() {
            let r = Rect::new(list.left + 8.0, ly, list.right - 8.0, ly + ROW);
            out.entries.push((r, Hit::Open(i)));
            out.entries.push((
                Rect::new(
                    r.right - 8.0 - 28.0 - 4.0 - 28.0,
                    r.top + 8.0,
                    r.right - 8.0 - 28.0 - 4.0,
                    r.top + 36.0,
                ),
                Hit::Rollback(i),
            ));
            out.entries.push((
                Rect::new(
                    r.right - 8.0 - 28.0,
                    r.top + 8.0,
                    r.right - 8.0,
                    r.top + 36.0,
                ),
                Hit::Delete(i),
            ));
            ly += ROW + 4.0;
        }
        out.content_height = versions.len() as f32 * (ROW + 4.0) + 16.0;
        out.entries.retain(|(r, h)| match h {
            Hit::Open(_) | Hit::Rollback(_) | Hit::Delete(_) => {
                r.bottom > list.top && r.top < list.bottom
            }
            _ => true,
        });
        out
    }

    pub fn paint(
        list: &mut DrawList,
        area: Rect,
        s: &State,
        lay: &Layout,
        focused: bool,
        p: &Palette,
    ) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        let subtitle = basename(s.source.as_deref());
        paint_header(list, area, Icon::HISTORY, "版本历史", &subtitle, p);
        if s.source.is_none() {
            centered_hint(
                list,
                area,
                area.top + HEADER + 32.0,
                "打开文档后查看版本历史",
                p,
            );
            list.pop_clip();
            return;
        }
        let mut y = area.top + HEADER;
        if !s.enabled {
            let r = Rect::new(
                area.left + 12.0,
                y + 12.0,
                area.right - 12.0,
                y + 12.0 + 44.0,
            );
            list.rounded_rect(r, 4.0, 0xFFFBEB);
            list.rounded_border(r, 4.0, 0xFDE68A);
            list.icon_centered(
                Rect::new(r.left + 12.0, r.top, r.left + 27.0, r.bottom),
                Icon::ALERT_TRIANGLE,
                15.0,
                0x92400E,
            );
            list.text(
                Rect::new(r.left + 35.0, r.top, r.right - 12.0, r.bottom),
                "版本历史已关闭。仍可查看已有版本，但不会记录新版本。",
                TextStyle::Caption,
                0x92400E,
            );
            y += 12.0 + 44.0 + 12.0;
        }
        if let Some(r) = lay.rect_of(Hit::Message) {
            let mut f = s.message.clone();
            f.paint(
                list,
                Rect::new(r.left, r.top, r.right, r.top + 36.0),
                focused,
                p,
                FieldLook::dialog(p),
            );
            // textarea 比单行高：把剩余部分补成同色底
            list.rounded_rect(
                Rect::new(r.left + 1.0, r.top + 20.0, r.right - 1.0, r.bottom - 1.0),
                0.0,
                p.background,
            );
            list.rounded_border(r, 4.0, if focused { p.accent } else { p.border });
        }
        if let Some(r) = lay.rect_of(Hit::Record) {
            let start = list.cmds().len();
            list.glass_button(r, 4.0, p, false);
            list.icon_centered(
                Rect::new(r.left + 10.0, r.top, r.left + 23.0, r.bottom),
                Icon::SAVE,
                13.0,
                p.button_foreground(),
            );
            list.text(
                Rect::new(r.left + 27.0, r.top, r.right, r.bottom),
                "记录版本",
                TextStyle::Caption,
                p.button_foreground(),
            );
            if !s.enabled {
                list.fade_since(start, r, 0.45);
            }
        }
        if let Some(r) = lay.rect_of(Hit::Refresh) {
            list.rounded_border(r, 4.0, p.border);
            list.text_aligned(r, "刷新", TextStyle::Caption, p.foreground, Align::Center);
        }
        let _ = y;
        let list_rect = lay.list;
        list.hline(area.left, area.right, list_rect.top - 1.0, p.border);
        if !s.error.is_empty() {
            let r = Rect::new(
                area.left,
                list_rect.top - 33.0,
                area.right,
                list_rect.top - 1.0,
            );
            list.rect(r, 0xFEF2F2);
            list.text(
                Rect::new(r.left + 12.0, r.top, r.right - 12.0, r.bottom),
                text::ellipsize(&s.error, TextStyle::Caption, r.width() - 24.0),
                TextStyle::Caption,
                0xB91C1C,
            );
        }
        list.push_clip(list_rect);
        let versions = s.versions();
        if s.loading {
            centered_hint(list, area, list_rect.top + 32.0, "加载版本中...", p);
        } else if versions.is_empty() {
            centered_hint(list, area, list_rect.top + 32.0, "当前文档暂无版本", p);
        } else {
            for (i, c) in versions.iter().enumerate() {
                let Some(r) = lay.rect_of(Hit::Open(i)) else {
                    continue;
                };
                list.icon_centered(
                    Rect::new(r.left + 8.0, r.top + 8.0, r.left + 24.0, r.top + 28.0),
                    Icon::FILE_CLOCK,
                    16.0,
                    p.muted,
                );
                let title = clean_message(&c.message);
                let title = if title.is_empty() {
                    format!("第{}版", i + 1)
                } else {
                    title
                };
                let text_right = r.right - 8.0 - 28.0 - 4.0 - 28.0 - 8.0;
                list.text_run(
                    Rect::new(r.left + 40.0, r.top + 8.0, text_right, r.top + 28.0),
                    format!("第{}版", i + 1),
                    TextStyle::Label,
                    p.foreground,
                    Align::Leading,
                    Emphasis::Bold,
                );
                list.text(
                    Rect::new(r.left + 40.0, r.top + 30.0, text_right, r.top + 46.0),
                    text::ellipsize(&title, TextStyle::Caption, text_right - r.left - 40.0),
                    TextStyle::Caption,
                    p.muted,
                );
                if let Some(b) = lay.rect_of(Hit::Rollback(i)) {
                    list.icon_centered(b, Icon::ROTATE_CCW, 14.0, p.muted);
                }
                if let Some(b) = lay.rect_of(Hit::Delete(i)) {
                    list.icon_centered(b, Icon::TRASH2, 14.0, p.muted);
                }
            }
        }
        list.pop_clip();
        list.pop_clip();
    }
}

// =====================================================================
// 番茄钟
// =====================================================================

pub mod pomodoro {
    use super::*;
    use std::time::Instant;

    pub const SECONDS: u64 = 25 * 60;

    pub struct State {
        pub running: bool,
        /// 运行时的结束时刻。
        pub end_at: Option<Instant>,
        /// 暂停时剩下的秒数。
        pub remaining: u64,
    }

    impl Default for State {
        fn default() -> Self {
            State {
                running: false,
                end_at: None,
                remaining: SECONDS,
            }
        }
    }

    impl State {
        pub fn remaining_now(&self) -> u64 {
            match (self.running, self.end_at) {
                (true, Some(end)) => end
                    .saturating_duration_since(Instant::now())
                    .as_secs_f64()
                    .ceil() as u64,
                _ => self.remaining,
            }
        }
        pub fn is_complete(&self) -> bool {
            !self.running && self.remaining == 0
        }
        pub fn start(&mut self) {
            if self.remaining == 0 {
                self.remaining = SECONDS;
            }
            self.end_at = Some(Instant::now() + std::time::Duration::from_secs(self.remaining));
            self.running = true;
        }
        pub fn pause(&mut self) {
            self.remaining = self.remaining_now();
            self.running = false;
            self.end_at = None;
        }
        pub fn reset(&mut self) {
            self.running = false;
            self.end_at = None;
            self.remaining = SECONDS;
        }
        /// 计时器每 250ms 调一次。到点了就自动暂停并返回 `true`（该记一条 focus_session）。
        pub fn tick(&mut self) -> bool {
            if self.running && self.remaining_now() == 0 {
                self.pause();
                return true;
            }
            false
        }
    }

    pub fn format_duration(secs: u64) -> String {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        Toggle,
        Reset,
    }

    /// 内容竖直居中在面板里（`min-h-full justify-center`）。
    pub fn layout(area: Rect) -> Vec<(Rect, Hit)> {
        if area.is_empty() {
            return Vec::new();
        }
        let block_h = 56.0 + 20.0 + 28.0 + 4.0 + 20.0 + 32.0 + 60.0 + 36.0 + 40.0 + 24.0 + 16.0;
        let top = ((area.top + area.bottom) / 2.0 - block_h / 2.0).max(area.top + 20.0);
        let buttons_y = top + 56.0 + 20.0 + 28.0 + 4.0 + 20.0 + 32.0 + 60.0 + 36.0;
        let cx = (area.left + area.right) / 2.0;
        // 主按钮 min-w-28(112) h-10 + gap-3 + 方按钮 10×10
        let total = 112.0 + 12.0 + 40.0;
        let left = cx - total / 2.0;
        vec![
            (
                Rect::new(left, buttons_y, left + 112.0, buttons_y + 40.0),
                Hit::Toggle,
            ),
            (
                Rect::new(left + 124.0, buttons_y, left + 164.0, buttons_y + 40.0),
                Hit::Reset,
            ),
        ]
    }

    pub fn paint(list: &mut DrawList, area: Rect, s: &State, lay: &[(Rect, Hit)], p: &Palette) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        let Some((toggle, _)) = lay.iter().find(|(_, h)| *h == Hit::Toggle) else {
            list.pop_clip();
            return;
        };
        let cx = (area.left + area.right) / 2.0;
        let top = toggle.top - (56.0 + 20.0 + 28.0 + 4.0 + 20.0 + 32.0 + 60.0 + 36.0);
        // 图标圆
        list.rounded_rect(
            Rect::new(cx - 28.0, top, cx + 28.0, top + 56.0),
            28.0,
            theme::mix(p.accent, p.area_assistant_default, 0.10),
        );
        list.icon_centered(
            Rect::new(cx - 28.0, top, cx + 28.0, top + 56.0),
            Icon::TIMER,
            28.0,
            p.accent,
        );
        let mut y = top + 56.0 + 20.0;
        list.text_aligned(
            Rect::new(area.left, y, area.right, y + 28.0),
            "番茄钟",
            TextStyle::Large,
            p.foreground,
            Align::Center,
        );
        y += 28.0 + 4.0;
        list.text_aligned(
            Rect::new(area.left, y, area.right, y + 20.0),
            "专注 25 分钟",
            TextStyle::Label,
            p.muted,
            Align::Center,
        );
        y += 20.0 + 32.0;
        let secs = if s.is_complete() {
            0
        } else {
            s.remaining_now()
        };
        list.text_run(
            Rect::new(area.left, y, area.right, y + 60.0),
            format_duration(secs),
            TextStyle::Clock,
            p.foreground,
            Align::Center,
            Emphasis::Bold,
        );
        y += 60.0;
        if s.is_complete() {
            list.text_aligned(
                Rect::new(area.left, y + 12.0, area.right, y + 32.0),
                "本次专注已完成",
                TextStyle::Label,
                p.accent,
                Align::Center,
            );
        }
        for (r, h) in lay {
            match h {
                Hit::Toggle => {
                    list.glass_button(*r, 6.0, p, false);
                    let label = if s.running {
                        "暂停"
                    } else if s.is_complete() {
                        "重新开始"
                    } else {
                        "开始"
                    };
                    let icon = if s.running { Icon::PAUSE } else { Icon::PLAY };
                    let w = 16.0 + 8.0 + text::measure(label, TextStyle::Label);
                    let x = (r.left + r.right) / 2.0 - w / 2.0;
                    list.icon_centered(
                        Rect::new(x, r.top, x + 16.0, r.bottom),
                        icon,
                        16.0,
                        p.button_foreground(),
                    );
                    list.text_run(
                        Rect::new(x + 24.0, r.top, r.right, r.bottom),
                        label,
                        TextStyle::Label,
                        p.button_foreground(),
                        Align::Leading,
                        Emphasis::Bold,
                    );
                }
                Hit::Reset => {
                    list.rounded_border(*r, 6.0, p.border);
                    list.icon_centered(*r, Icon::ROTATE_CCW, 16.0, p.muted);
                }
            }
        }
        list.text_aligned(
            Rect::new(
                area.left,
                toggle.bottom + 24.0,
                area.right,
                toggle.bottom + 40.0,
            ),
            "计时会在关闭或切换侧栏后继续。",
            TextStyle::Caption,
            p.muted,
            Align::Center,
        );
        list.pop_clip();
    }
}

// =====================================================================
// 评论
// =====================================================================

pub mod comments {
    use super::*;

    pub struct Pending {
        pub parent_id: Option<String>,
        pub anchor: Option<mochi_core::sidecars::CommentAnchor>,
        pub quote: String,
        pub field: TextField,
    }

    pub struct State {
        pub source: Option<PathBuf>,
        pub comments: Vec<DocumentComment>,
        pub author: String,
        pub pending: Option<Pending>,
        pub scroll: f32,
    }

    impl Default for State {
        fn default() -> Self {
            State {
                source: None,
                comments: Vec::new(),
                author: "我".into(),
                pending: None,
                scroll: 0.0,
            }
        }
    }

    impl State {
        pub fn scroll_to(&mut self, id: &str) {
            let mut flat = Vec::new();
            for root in self.roots() {
                if let Some(index) = self.comments.iter().position(|c| c.id == root.id) {
                    flat.push((index, 0));
                }
                flatten(self, &root.id, 1, &mut flat);
            }
            if let Some(index) = flat.iter().position(|(i, _)| self.comments[*i].id == id) {
                self.scroll = index as f32 * CARD;
            }
        }
        pub fn roots(&self) -> Vec<&DocumentComment> {
            self.comments
                .iter()
                .filter(|c| c.parent_id.is_none())
                .collect()
        }
        pub fn children(&self, id: &str) -> Vec<&DocumentComment> {
            self.comments
                .iter()
                .filter(|c| c.parent_id.as_deref() == Some(id))
                .collect()
        }
        pub fn start_compose(&mut self, parent_id: Option<String>, quote: String) {
            let mut field = TextField::new("支持 Markdown，可用 Ctrl+Enter 发送");
            field.style = TextStyle::Label;
            self.pending = Some(Pending {
                parent_id,
                quote,
                field,
                anchor: None,
            });
        }
        /// 删除一条及其全部回复（`deleteCommentThread`）。
        pub fn delete_thread(&mut self, id: &str) {
            let mut doomed: HashSet<String> = [id.to_owned()].into_iter().collect();
            loop {
                let before = doomed.len();
                for c in &self.comments {
                    if let Some(pid) = &c.parent_id {
                        if doomed.contains(pid) {
                            doomed.insert(c.id.clone());
                        }
                    }
                }
                if doomed.len() == before {
                    break;
                }
            }
            self.comments.retain(|c| !doomed.contains(&c.id));
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        /// 「全文评论」按钮。
        Compose,
        /// 第 i 条（按扁平顺序）的「回复」。
        Reply(usize),
        Delete(usize),
        ComposeField,
        Send,
        Cancel,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Layout {
        pub entries: Vec<(Rect, Hit)>,
        /// 扁平化的评论：(下标进 `comments`, 层级)
        pub rows: Vec<(usize, usize, Rect)>,
        pub compose: Option<Rect>,
        pub content_height: f32,
        pub list: Rect,
    }

    impl Layout {
        pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
            self.entries
                .iter()
                .rev()
                .find(|(r, _)| r.contains(x, y))
                .map(|(_, h)| *h)
        }
        pub fn rect_of(&self, h: Hit) -> Option<Rect> {
            self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
        }
        pub fn max_scroll(&self) -> f32 {
            (self.content_height - self.list.height()).max(0.0)
        }
    }

    const CARD: f32 = 116.0; // TSX 的 cursor += 116
    const COMPOSE_H: f32 = 12.0 + 16.0 + 8.0 + 80.0 + 32.0 + 12.0;

    fn flatten(s: &State, id: &str, depth: usize, out: &mut Vec<(usize, usize)>) {
        for c in s.children(id) {
            let idx = s.comments.iter().position(|x| x.id == c.id).unwrap_or(0);
            out.push((idx, depth));
            flatten(s, &c.id, depth + 1, out);
        }
    }

    pub fn layout(s: &State, area: Rect) -> Layout {
        let mut out = Layout::default();
        if area.is_empty() || s.source.is_none() {
            return out;
        }
        let compose_h = if s.pending.is_some() {
            COMPOSE_H + 12.0
        } else {
            0.0
        };
        let list = Rect::new(area.left, area.top, area.right, area.bottom - compose_h);
        out.list = list;
        // 头部 p-3：一行 20 + mb-3
        let y0 = list.top + 12.0;
        let compose_w = text::measure("全文评论", TextStyle::Caption) + 16.0;
        let author_w = 13.0 + 4.0 + 64.0;
        out.entries.push((
            Rect::new(
                list.right - 12.0 - author_w - 8.0 - compose_w,
                y0,
                list.right - 12.0 - author_w - 8.0,
                y0 + 20.0,
            ),
            Hit::Compose,
        ));
        let mut y = y0 + 20.0 + 12.0 - s.scroll;
        let mut flat: Vec<(usize, usize)> = Vec::new();
        for root in s.roots() {
            let idx = s.comments.iter().position(|x| x.id == root.id).unwrap_or(0);
            flat.push((idx, 0));
            flatten(s, &root.id, 1, &mut flat);
        }
        for (i, (idx, depth)) in flat.iter().enumerate() {
            let indent = *depth as f32 * 28.0;
            let r = Rect::new(
                list.left + 12.0 + indent,
                y,
                list.right - 12.0,
                y + CARD - 12.0,
            );
            out.rows.push((*idx, *depth, r));
            out.entries.push((
                Rect::new(
                    r.right - 8.0 - 24.0 - 4.0 - 24.0,
                    r.top + 8.0,
                    r.right - 8.0 - 24.0 - 4.0,
                    r.top + 32.0,
                ),
                Hit::Reply(i),
            ));
            out.entries.push((
                Rect::new(
                    r.right - 8.0 - 24.0,
                    r.top + 8.0,
                    r.right - 8.0,
                    r.top + 32.0,
                ),
                Hit::Delete(i),
            ));
            y += CARD;
        }
        out.content_height = (y + s.scroll - list.top).max(0.0) + 12.0;
        out.entries.retain(|(r, h)| match h {
            Hit::Reply(_) | Hit::Delete(_) => r.bottom > list.top + 44.0 && r.top < list.bottom,
            _ => true,
        });
        out.rows
            .retain(|(_, _, r)| r.bottom > list.top && r.top < list.bottom);
        if s.pending.is_some() {
            let c = Rect::new(
                area.left + 12.0,
                area.bottom - compose_h,
                area.right - 12.0,
                area.bottom - 12.0,
            );
            out.compose = Some(c);
            out.entries.push((
                Rect::new(
                    c.left + 12.0,
                    c.top + 12.0 + 16.0 + 8.0,
                    c.right - 12.0,
                    c.top + 12.0 + 16.0 + 8.0 + 80.0,
                ),
                Hit::ComposeField,
            ));
            let send_w = 13.0 + 4.0 + text::measure("发送", TextStyle::Caption) + 20.0;
            out.entries.push((
                Rect::new(
                    c.right - 12.0 - send_w,
                    c.bottom - 12.0 - 28.0,
                    c.right - 12.0,
                    c.bottom - 12.0,
                ),
                Hit::Send,
            ));
            let cancel_w = text::measure("取消", TextStyle::Caption) + 16.0;
            out.entries.push((
                Rect::new(
                    c.right - 12.0 - send_w - 8.0 - cancel_w,
                    c.bottom - 12.0 - 28.0,
                    c.right - 12.0 - send_w - 8.0,
                    c.bottom - 12.0,
                ),
                Hit::Cancel,
            ));
        }
        out
    }

    pub fn paint(
        list: &mut DrawList,
        area: Rect,
        s: &State,
        lay: &Layout,
        focused: bool,
        p: &Palette,
    ) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        if s.source.is_none() {
            list.text(
                Rect::new(
                    area.left + 20.0,
                    area.top + 20.0,
                    area.right - 20.0,
                    area.top + 40.0,
                ),
                "打开文档后可查看评论。",
                TextStyle::Label,
                p.muted,
            );
            list.pop_clip();
            return;
        }
        let lr = lay.list;
        let y0 = lr.top + 12.0;
        list.icon_centered(
            Rect::new(lr.left + 12.0, y0, lr.left + 28.0, y0 + 20.0),
            Icon::MESSAGE_SQUARE,
            16.0,
            p.foreground,
        );
        list.text_run(
            Rect::new(lr.left + 36.0, y0, lr.left + 80.0, y0 + 20.0),
            "评论",
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        list.text(
            Rect::new(lr.left + 76.0, y0, lr.left + 120.0, y0 + 20.0),
            s.roots().len().to_string(),
            TextStyle::Caption,
            p.muted,
        );
        if let Some(r) = lay.rect_of(Hit::Compose) {
            list.text_aligned(r, "全文评论", TextStyle::Caption, p.accent, Align::Center);
            list.icon_centered(
                Rect::new(r.right + 8.0, r.top, r.right + 21.0, r.bottom),
                Icon::USER_ROUND,
                13.0,
                p.muted,
            );
            list.text(
                Rect::new(r.right + 25.0, r.top, lr.right - 12.0, r.bottom),
                s.author.clone(),
                TextStyle::Caption,
                p.muted,
            );
        }
        list.push_clip(Rect::new(lr.left, lr.top + 44.0, lr.right, lr.bottom));
        if s.roots().is_empty() {
            let r = Rect::new(
                lr.left + 12.0,
                lr.top + 44.0,
                lr.right - 12.0,
                lr.top + 44.0 + 68.0,
            );
            list.rounded_border(r, 8.0, p.border);
            list.text_aligned(
                Rect::new(r.left + 24.0, r.top + 24.0, r.right - 24.0, r.top + 44.0),
                "选中文字或悬停段落，选择“评论”开始讨论。",
                TextStyle::Label,
                p.muted,
                Align::Center,
            );
        }
        for (i, (idx, depth, r)) in lay.rows.iter().enumerate() {
            let c = &s.comments[*idx];
            if *depth > 0 {
                list.rect(
                    Rect::new(r.left - 8.0, r.top, r.left - 7.0, r.bottom),
                    p.border,
                );
            }
            list.rounded_rect(*r, 8.0, p.surface);
            list.rounded_border(*r, 8.0, p.border);
            list.text_run(
                Rect::new(r.left + 12.0, r.top + 8.0, r.right - 72.0, r.top + 26.0),
                text::ellipsize(&c.author, TextStyle::Caption, 100.0),
                TextStyle::Caption,
                p.foreground,
                Align::Leading,
                Emphasis::Bold,
            );
            let mut when = c
                .created_at
                .get(5..16)
                .unwrap_or(&c.created_at)
                .replace('T', " ");
            if c.resolved {
                when.push_str(" · 已解决");
            }
            list.text(
                Rect::new(
                    r.left + 12.0 + text::measure(&c.author, TextStyle::Caption) + 8.0,
                    r.top + 8.0,
                    r.right - 72.0,
                    r.top + 26.0,
                ),
                when,
                TextStyle::Caption,
                p.muted,
            );
            if let Some(anchor) = &c.anchor {
                if !anchor.selected_text.is_empty() {
                    list.rect(
                        Rect::new(r.left + 12.0, r.top + 30.0, r.left + 14.0, r.top + 46.0),
                        p.accent,
                    );
                    list.text(
                        Rect::new(r.left + 20.0, r.top + 30.0, r.right - 12.0, r.top + 46.0),
                        text::ellipsize(
                            &anchor.selected_text,
                            TextStyle::Caption,
                            r.width() - 32.0,
                        ),
                        TextStyle::Caption,
                        p.muted,
                    );
                }
            }
            list.text(
                Rect::new(r.left + 12.0, r.top + 52.0, r.right - 12.0, r.top + 72.0),
                text::ellipsize(&c.content, TextStyle::Label, r.width() - 24.0),
                TextStyle::Label,
                p.foreground,
            );
            if let Some(b) = lay.rect_of(Hit::Reply(i)) {
                list.icon_centered(b, Icon::REPLY, 14.0, p.muted);
            }
            if let Some(b) = lay.rect_of(Hit::Delete(i)) {
                list.icon_centered(b, Icon::TRASH2, 14.0, p.muted);
            }
        }
        list.pop_clip();
        if let (Some(pending), Some(c)) = (&s.pending, lay.compose) {
            list.rounded_rect(c, 8.0, p.surface);
            list.rounded_border(c, 8.0, theme::mix(p.accent, p.surface, 0.4));
            list.rect(
                Rect::new(c.left + 12.0, c.top + 12.0, c.left + 14.0, c.top + 28.0),
                p.accent,
            );
            let quote = if pending.parent_id.is_some() {
                "回复评论".to_owned()
            } else if pending.quote.is_empty() {
                "全文评论".to_owned()
            } else {
                pending.quote.clone()
            };
            list.text(
                Rect::new(c.left + 20.0, c.top + 12.0, c.right - 12.0, c.top + 28.0),
                text::ellipsize(&quote, TextStyle::Caption, c.width() - 32.0),
                TextStyle::Caption,
                p.muted,
            );
            if let Some(f) = lay.rect_of(Hit::ComposeField) {
                let mut field = pending.field.clone();
                field.paint_multiline_with_look(list, f, focused, p, FieldLook::bare());
            }
            if let Some(b) = lay.rect_of(Hit::Cancel) {
                list.text_aligned(b, "取消", TextStyle::Caption, p.muted, Align::Center);
            }
            if let Some(b) = lay.rect_of(Hit::Send) {
                let enabled = !pending.field.text().trim().is_empty();
                let start = list.cmds().len();
                list.glass_button(b, 4.0, p, false);
                list.icon_centered(
                    Rect::new(b.left + 10.0, b.top, b.left + 23.0, b.bottom),
                    Icon::SEND,
                    13.0,
                    p.button_foreground(),
                );
                list.text(
                    Rect::new(b.left + 27.0, b.top, b.right, b.bottom),
                    "发送",
                    TextStyle::Caption,
                    p.button_foreground(),
                );
                if !enabled {
                    list.fade_since(start, b, 0.45);
                }
            }
        }
        list.pop_clip();
    }
}

// =====================================================================
// 文档挂载
// =====================================================================

pub mod mounts {
    use super::*;

    #[derive(Default)]
    pub struct State {
        pub source: Option<PathBuf>,
        pub mounts: Vec<AiDocumentMount>,
        pub scroll: f32,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        Open(usize),
        Delete(usize),
    }

    #[derive(Debug, Clone, Default)]
    pub struct Layout {
        pub entries: Vec<(Rect, Hit)>,
        /// 分组标题的位置：(y, 标题)。
        pub headings: Vec<(f32, &'static str)>,
        pub list: Rect,
        pub content_height: f32,
    }

    impl Layout {
        pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
            self.entries
                .iter()
                .rev()
                .find(|(r, _)| r.contains(x, y))
                .map(|(_, h)| *h)
        }
        pub fn rect_of(&self, h: Hit) -> Option<Rect> {
            self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
        }
        pub fn max_scroll(&self) -> f32 {
            (self.content_height - self.list.height()).max(0.0)
        }
    }

    const HEADER: f32 = 12.0 + 20.0 + 16.0 + 12.0 + 1.0;
    const ITEM: f32 = 10.0 + 18.0 + 4.0 + 16.0 + 4.0 + 14.0 + 10.0;

    /// 排序：会话级在前，消息级在后（TSX 分两组渲染）。返回 mounts 的下标序列与分组边界。
    pub fn ordered(s: &State) -> (Vec<usize>, Vec<usize>) {
        let session: Vec<usize> = s
            .mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| m.scope() == MountScope::Session)
            .map(|(i, _)| i)
            .collect();
        let message: Vec<usize> = s
            .mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| m.scope() != MountScope::Session)
            .map(|(i, _)| i)
            .collect();
        (session, message)
    }

    pub fn layout(s: &State, area: Rect) -> Layout {
        let mut out = Layout::default();
        if area.is_empty() {
            return out;
        }
        let list = Rect::new(area.left, area.top + HEADER, area.right, area.bottom);
        out.list = list;
        let mut y = list.top + 12.0 - s.scroll;
        let (session, message) = ordered(s);
        for (title, group) in [("会话挂载", &session), ("对话挂载", &message)] {
            if group.is_empty() {
                continue;
            }
            out.headings.push((y, title));
            y += 24.0;
            for &i in group {
                let r = Rect::new(list.left + 12.0, y, list.right - 12.0, y + ITEM);
                out.entries.push((r, Hit::Open(i)));
                out.entries.push((
                    Rect::new(
                        r.right - 8.0 - 24.0,
                        r.top + 8.0,
                        r.right - 8.0,
                        r.top + 32.0,
                    ),
                    Hit::Delete(i),
                ));
                y += ITEM + 8.0;
            }
            y += 8.0;
        }
        out.content_height = (y + s.scroll - list.top).max(0.0);
        out.entries
            .retain(|(r, _)| r.bottom > list.top && r.top < list.bottom);
        out
    }

    pub fn paint(list: &mut DrawList, area: Rect, s: &State, lay: &Layout, p: &Palette) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        let head = Rect::new(area.left, area.top, area.right, area.top + HEADER);
        list.text_run(
            Rect::new(
                head.left + 16.0,
                head.top + 12.0,
                head.right - 48.0,
                head.top + 32.0,
            ),
            "文档挂载",
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        let sub = s
            .source
            .as_deref()
            .map(|p| basename(Some(p)))
            .unwrap_or_else(|| "当前没有打开文档".to_owned());
        list.text(
            Rect::new(
                head.left + 16.0,
                head.top + 32.0,
                head.right - 48.0,
                head.top + 48.0,
            ),
            sub,
            TextStyle::Caption,
            p.muted,
        );
        list.icon_centered(
            Rect::new(
                head.right - 32.0,
                head.top + 12.0,
                head.right - 16.0,
                head.top + 32.0,
            ),
            Icon::LINK2,
            16.0,
            p.muted,
        );
        list.border_bottom(head, p.border);
        let lr = lay.list;
        list.push_clip(lr);
        if s.source.is_none() {
            centered_hint(
                list,
                area,
                lr.top + 32.0,
                "打开一个文档后查看挂载的会话和对话",
                p,
            );
        } else if s.mounts.is_empty() {
            centered_hint(
                list,
                area,
                lr.top + 32.0,
                "该文档还没有挂载 AI 会话或对话",
                p,
            );
        } else {
            for (y, title) in &lay.headings {
                list.text(
                    Rect::new(lr.left + 12.0, *y, lr.right - 12.0, *y + 20.0),
                    *title,
                    TextStyle::Caption,
                    p.muted,
                );
            }
            for (r, h) in &lay.entries {
                let Hit::Open(i) = h else { continue };
                let m = &s.mounts[*i];
                list.rounded_rect(*r, 8.0, p.surface);
                list.rounded_border(*r, 8.0, p.border);
                let is_session = m.scope() == MountScope::Session;
                list.icon_centered(
                    Rect::new(r.left + 12.0, r.top + 10.0, r.left + 26.0, r.top + 28.0),
                    if is_session {
                        Icon::MESSAGE_SQUARE
                    } else {
                        Icon::EXTERNAL_LINK
                    },
                    14.0,
                    p.foreground,
                );
                let title = if m.session_title().is_empty() {
                    "AI 会话"
                } else {
                    m.session_title()
                };
                list.text_run(
                    Rect::new(r.left + 32.0, r.top + 10.0, r.right - 40.0, r.top + 28.0),
                    text::ellipsize(title, TextStyle::Label, r.width() - 80.0),
                    TextStyle::Label,
                    p.foreground,
                    Align::Leading,
                    Emphasis::Bold,
                );
                let snippet = if m.snippet().is_empty() {
                    if is_session {
                        "整个会话"
                    } else {
                        "未命名问答"
                    }
                } else {
                    m.snippet()
                };
                list.text(
                    Rect::new(r.left + 12.0, r.top + 32.0, r.right - 12.0, r.top + 48.0),
                    text::ellipsize(snippet, TextStyle::Caption, r.width() - 24.0),
                    TextStyle::Caption,
                    p.muted,
                );
                list.text(
                    Rect::new(r.left + 12.0, r.top + 52.0, r.right - 12.0, r.top + 66.0),
                    format_time(m.created_at()),
                    TextStyle::Caption,
                    p.muted,
                );
                if let Some(b) = lay.rect_of(Hit::Delete(*i)) {
                    list.icon_centered(b, Icon::TRASH2, 14.0, p.muted);
                }
            }
        }
        list.pop_clip();
        list.pop_clip();
    }
}

// =====================================================================
// 待批准操作
// =====================================================================

pub mod inbox {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct State {
        pub entries: Vec<InboxEntry>,
        pub scroll: f32,
        pub loading: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Hit {
        Refresh,
        ApproveAll,
        RejectAll,
        Open(usize),
        Approve(usize),
        Reject(usize),
    }

    /// 审批收件箱里的一行紧凑条目。文件内容提案按完整路径分组；
    /// 语义不同的命令、导出和文件系统动作刻意各占一行。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Group {
        pub key: String,
        pub path: String,
        pub name: String,
        pub indices: Vec<usize>,
    }

    impl Group {
        pub fn len(&self) -> usize {
            self.indices.len()
        }

        pub fn is_empty(&self) -> bool {
            self.indices.is_empty()
        }
    }

    fn normalized_path(path: &str) -> String {
        let normalized = path.replace('\\', "/");
        let normalized = normalized.trim_end_matches('/');
        if cfg!(windows) {
            normalized.to_lowercase()
        } else {
            normalized.to_owned()
        }
    }

    fn path_name(path: &str) -> String {
        let normalized = path.replace('\\', "/");
        normalized
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(path)
            .to_owned()
    }

    fn groupable_kind(kind: &str) -> bool {
        matches!(kind, "write" | "overwrite" | "block-edit")
    }

    fn group_type_label(s: &State, group: &Group) -> &'static str {
        let kind = group
            .indices
            .first()
            .and_then(|index| s.entries.get(*index))
            .map(InboxEntry::kind)
            .unwrap_or_default();
        match kind {
            "write" | "overwrite" | "block-edit" => "文档",
            "native-shell-command" => "命令",
            "native-document-export" => "导出",
            _ => "文件",
        }
    }

    /// 按「首次出现」稳定分组。返回的下标始终指回 `State::entries`，
    /// 命令处理因此能保留每个审批真实的收件箱身份，
    /// 对原始条目执行应用/拒绝。
    pub fn groups(s: &State) -> Vec<Group> {
        let mut result: Vec<Group> = Vec::new();
        let mut positions = HashMap::<String, usize>::new();

        for (index, entry) in s.entries.iter().enumerate() {
            let independent = !groupable_kind(entry.kind());
            let path = entry.path().to_owned();
            let key = if independent || path.trim().is_empty() {
                format!("{}:{}", entry.kind(), entry.id())
            } else {
                format!("file:{}", normalized_path(&path))
            };
            if let Some(position) = positions.get(&key).copied() {
                result[position].indices.push(index);
                continue;
            }

            let name = if independent {
                if entry.title().is_empty() {
                    entry.kind().to_owned()
                } else {
                    entry.title().to_owned()
                }
            } else {
                let file = path_name(&path);
                if file.is_empty() {
                    if entry.title().is_empty() {
                        entry.kind().to_owned()
                    } else {
                        entry.title().to_owned()
                    }
                } else {
                    file
                }
            };
            positions.insert(key.clone(), result.len());
            result.push(Group {
                key,
                path,
                name,
                indices: vec![index],
            });
        }
        result
    }

    /// 把这些行描述为「分组投影」的调用方用的别名。
    pub fn grouped(s: &State) -> Vec<Group> {
        groups(s)
    }

    #[derive(Debug, Clone, Default)]
    pub struct Layout {
        pub entries: Vec<(Rect, Hit)>,
        /// 可见行，形式为 `(分组下标, 行矩形)`。
        pub cards: Vec<(usize, Rect)>,
        pub list: Rect,
        pub content_height: f32,
    }

    impl Layout {
        pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
            self.entries
                .iter()
                .rev()
                .find(|(r, _)| r.contains(x, y))
                .map(|(_, h)| *h)
        }
        pub fn rect_of(&self, h: Hit) -> Option<Rect> {
            self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
        }
        pub fn max_scroll(&self) -> f32 {
            (self.content_height - self.list.height()).max(0.0)
        }
    }

    const HEADER: f32 = 12.0 + 20.0 + 16.0 + 12.0 + 1.0;
    /// 一行紧凑审批条目：48 DIP，含上下呼吸空间。
    pub const ROW: f32 = 48.0;

    fn header_buttons(area: Rect) -> (Rect, Rect, Rect) {
        let refresh = Rect::new(
            area.right - 16.0 - 24.0,
            area.top + 12.0,
            area.right - 16.0,
            area.top + 36.0,
        );
        let approve_w = text::measure("全部通过", TextStyle::Caption) + 16.0;
        let reject_w = text::measure("全部拒绝", TextStyle::Caption) + 16.0;
        let approve = Rect::new(
            refresh.left - 8.0 - approve_w,
            area.top + 10.0,
            refresh.left - 8.0,
            area.top + 38.0,
        );
        let reject = Rect::new(
            approve.left - 4.0 - reject_w,
            area.top + 10.0,
            approve.left - 4.0,
            area.top + 38.0,
        );
        (reject, approve, refresh)
    }

    pub fn layout(s: &State, area: Rect) -> Layout {
        let mut out = Layout::default();
        if area.is_empty() {
            return out;
        }
        let (reject_all, approve_all, refresh) = header_buttons(area);
        out.entries.push((reject_all, Hit::RejectAll));
        out.entries.push((approve_all, Hit::ApproveAll));
        out.entries.push((refresh, Hit::Refresh));
        let list = Rect::new(area.left, area.top + HEADER, area.right, area.bottom);
        out.list = list;
        let mut y = list.top - s.scroll;
        for (group_index, _group) in groups(s).iter().enumerate() {
            let r = Rect::new(list.left, y, list.right, y + ROW);
            out.cards.push((group_index, r));
            let reject = Rect::new(
                r.right - 12.0 - 28.0,
                r.top + 10.0,
                r.right - 12.0,
                r.top + 38.0,
            );
            let approve = Rect::new(
                reject.left - 4.0 - 28.0,
                r.top + 10.0,
                reject.left - 4.0,
                r.top + 38.0,
            );
            let open = Rect::new(
                r.left + 8.0,
                r.top + 4.0,
                approve.left - 8.0,
                r.bottom - 4.0,
            );
            out.entries.push((open, Hit::Open(group_index)));
            out.entries.push((approve, Hit::Approve(group_index)));
            out.entries.push((reject, Hit::Reject(group_index)));
            y += ROW;
        }
        out.content_height = (y + s.scroll - list.top).max(0.0);
        out.entries.retain(|(r, h)| match h {
            Hit::Refresh | Hit::ApproveAll | Hit::RejectAll => true,
            _ => r.bottom > list.top && r.top < list.bottom,
        });
        out.cards
            .retain(|(_, r)| r.bottom > list.top && r.top < list.bottom);
        out
    }

    pub fn paint(list: &mut DrawList, area: Rect, s: &State, lay: &Layout, p: &Palette) {
        if area.is_empty() {
            return;
        }
        list.push_clip(area);
        let head = Rect::new(area.left, area.top, area.right, area.top + HEADER);
        let (reject_all, approve_all, _refresh) = header_buttons(area);
        list.icon_centered(
            Rect::new(
                head.left + 16.0,
                head.top + 12.0,
                head.left + 32.0,
                head.top + 32.0,
            ),
            Icon::INBOX,
            16.0,
            p.foreground,
        );
        list.text_run(
            Rect::new(
                head.left + 40.0,
                head.top + 12.0,
                head.right - 180.0,
                head.top + 32.0,
            ),
            "待批准操作",
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        list.text(
            Rect::new(
                head.left + 16.0,
                head.top + 32.0,
                reject_all.left - 8.0,
                head.top + 48.0,
            ),
            format!("{} 条待处理 · {} 组", s.entries.len(), groups(s).len()),
            TextStyle::Caption,
            p.muted,
        );
        list.rounded_border(reject_all, 4.0, p.border);
        list.text_aligned(
            reject_all,
            "全部拒绝",
            TextStyle::Caption,
            p.foreground,
            Align::Center,
        );
        list.glass_button(approve_all, 4.0, p, false);
        list.text_aligned(
            approve_all,
            "全部通过",
            TextStyle::Caption,
            p.button_foreground(),
            Align::Center,
        );
        if let Some(r) = lay.rect_of(Hit::Refresh) {
            list.icon_centered(r, Icon::REFRESH_CW, 14.0, p.muted);
        }
        list.border_bottom(head, p.border);
        let lr = lay.list;
        list.push_clip(lr);
        if s.loading {
            centered_hint(list, area, lr.top + 32.0, "加载中...", p);
        } else if s.entries.is_empty() {
            centered_hint(list, area, lr.top + 32.0, "没有待批准的操作", p);
        }
        let grouped = groups(s);
        for (group_index, r) in &lay.cards {
            let Some(group) = grouped.get(*group_index) else {
                continue;
            };
            list.hline(r.left, r.right, r.bottom - 1.0, p.border);
            list.icon_centered(
                Rect::new(r.left + 12.0, r.top + 14.0, r.left + 28.0, r.top + 34.0),
                Icon::FILE_TEXT,
                16.0,
                p.muted,
            );
            let label = if group.len() > 1 {
                format!("{} · {} 项", group.name, group.len())
            } else {
                group.name.clone()
            };
            let type_label = group_type_label(s, group);
            let type_width = text::measure(type_label, TextStyle::Caption) + 4.0;
            let type_rect = Rect::new(
                r.left + 36.0,
                r.top + 4.0,
                r.left + 36.0 + type_width,
                r.bottom - 4.0,
            );
            list.text_aligned(
                type_rect,
                type_label,
                TextStyle::Caption,
                p.muted,
                Align::Leading,
            );
            let name_rect = Rect::new(
                type_rect.right + 8.0,
                r.top + 4.0,
                r.right - 88.0,
                r.bottom - 4.0,
            );
            list.text_run(
                name_rect,
                text::ellipsize(&label, TextStyle::Label, name_rect.width()),
                TextStyle::Label,
                p.accent,
                Align::Leading,
                Emphasis::Bold,
            );
            if let Some(b) = lay.rect_of(Hit::Approve(*group_index)) {
                list.rounded_rect(b, 4.0, theme::mix(p.accent, p.surface, 0.10));
                list.icon_centered(b, Icon::CHECK, 15.0, p.accent);
            }
            if let Some(b) = lay.rect_of(Hit::Reject(*group_index)) {
                list.rounded_rect(b, 4.0, theme::mix(p.danger, p.surface, 0.10));
                list.icon_centered(b, Icon::X, 15.0, p.danger);
            }
        }
        list.pop_clip();
        list.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const AREA: Rect = Rect {
        left: 780.0,
        top: 72.0,
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

    fn commit(oid: &str, message: &str) -> GitCommitInfo {
        GitCommitInfo {
            oid: oid.into(),
            short_oid: oid[..oid.len().min(8)].into(),
            message: message.into(),
            kind: "auto".into(),
            is_key_node: false,
            author_name: "me".into(),
            timestamp_ms: 0,
        }
    }

    #[test]
    fn versions_are_numbered_oldest_first_and_hidden_ones_are_skipped() {
        let mut s = version::State::default();
        s.source = Some(PathBuf::from("a.md"));
        s.commits = vec![
            commit("c3", "auto: 第三次"),
            commit("c2", "manual: 手动"),
            commit("c1", "auto: 第一次"),
        ];
        s.deleted.insert("c2".into());
        let v = s.versions();
        assert_eq!(
            v.iter().map(|c| c.oid.as_str()).collect::<Vec<_>>(),
            vec!["c1", "c3"]
        );
        assert_eq!(version::clean_message("auto: 第三次"), "第三次");
        assert_eq!(version::clean_message("随便写"), "随便写");

        let lay = version::layout(&s, AREA);
        assert!(lay.rect_of(version::Hit::Open(1)).is_some());
        assert!(lay.rect_of(version::Hit::Open(2)).is_none());
        let open = lay.rect_of(version::Hit::Open(0)).unwrap();
        let del = lay.rect_of(version::Hit::Delete(0)).unwrap();
        assert_eq!(
            lay.hit(del.left + 2.0, del.top + 2.0),
            Some(version::Hit::Delete(0)),
            "删除按钮要赢过整行"
        );
        assert_eq!(
            lay.hit(open.left + 10.0, open.top + 10.0),
            Some(version::Hit::Open(0))
        );

        let mut list = DrawList::new();
        version::paint(
            &mut list,
            AREA,
            &s,
            &lay,
            false,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(t.contains(&"第1版".to_owned()) && t.contains(&"第2版".to_owned()));
        assert!(t.contains(&"第一次".to_owned()));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_version_panel_without_a_document_shows_only_the_hint() {
        let s = version::State::default();
        let lay = version::layout(&s, AREA);
        assert!(lay.rect_of(version::Hit::Record).is_none());
        let mut list = DrawList::new();
        version::paint(
            &mut list,
            AREA,
            &s,
            &lay,
            false,
            theme::tokens().palette(false),
        );
        assert!(texts(&list).contains(&"打开文档后查看版本历史".to_owned()));
    }

    #[test]
    fn the_pomodoro_counts_down_pauses_and_resets() {
        let mut s = pomodoro::State::default();
        assert_eq!(pomodoro::format_duration(s.remaining_now()), "25:00");
        s.start();
        assert!(s.running);
        assert!(
            s.remaining_now() <= pomodoro::SECONDS && s.remaining_now() >= pomodoro::SECONDS - 1
        );
        s.pause();
        assert!(!s.running);
        s.remaining = 0;
        assert!(s.is_complete());
        s.start();
        let restarted = s.remaining_now();
        assert!(
            (pomodoro::SECONDS - 1..=pomodoro::SECONDS).contains(&restarted),
            "完成后再开始要从 25 分钟重来，实际 {restarted}"
        );
        s.reset();
        assert_eq!(s.remaining, pomodoro::SECONDS);
        assert_eq!(pomodoro::format_duration(65), "01:05");

        let lay = pomodoro::layout(AREA);
        let (toggle, _) = lay
            .iter()
            .find(|(_, h)| *h == pomodoro::Hit::Toggle)
            .unwrap();
        assert_eq!(toggle.width(), 112.0);
        let mut list = DrawList::new();
        pomodoro::paint(&mut list, AREA, &s, &lay, theme::tokens().palette(false));
        let t = texts(&list);
        assert!(t.contains(&"25:00".to_owned()) && t.contains(&"开始".to_owned()));
        assert!(list.finish().is_ok());
    }

    fn comment(id: &str, parent: Option<&str>, content: &str) -> DocumentComment {
        DocumentComment {
            resolved: false,
            id: id.into(),
            parent_id: parent.map(str::to_owned),
            target_type: "document".into(),
            author: "我".into(),
            content: content.into(),
            created_at: "2026-09-02T10:00:00.000Z".into(),
            updated_at: None,
            anchor: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn comment_threads_are_flattened_with_indentation_and_deleted_recursively() {
        let mut s = comments::State::default();
        s.source = Some(PathBuf::from("a.md"));
        s.comments = vec![
            comment("r1", None, "根一"),
            comment("c1", Some("r1"), "回复一"),
            comment("c2", Some("c1"), "回复的回复"),
            comment("r2", None, "根二"),
        ];
        let lay = comments::layout(&s, AREA);
        assert_eq!(lay.rows.len(), 4);
        assert_eq!(
            lay.rows.iter().map(|(_, d, _)| *d).collect::<Vec<_>>(),
            vec![0, 1, 2, 0]
        );
        assert_eq!(lay.rows[1].2.left - lay.rows[0].2.left, 28.0);
        assert!(lay.rect_of(comments::Hit::Compose).is_some());
        assert!(lay.compose.is_none());

        s.delete_thread("r1");
        assert_eq!(
            s.comments.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["r2"]
        );

        s.start_compose(None, String::new());
        let lay = comments::layout(&s, AREA);
        assert!(lay.compose.is_some());
        assert!(lay.rect_of(comments::Hit::Send).is_some());
        let mut list = DrawList::new();
        comments::paint(
            &mut list,
            AREA,
            &s,
            &lay,
            true,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(t.contains(&"全文评论".to_owned()) && t.contains(&"发送".to_owned()));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn mounts_are_grouped_session_first() {
        let mk = |scope: &str, title: &str| {
            let v = serde_json::json!({ "id": format!("m-{title}"), "scope": scope, "sessionId": "s", "sessionTitle": title, "snippet": "", "createdAt": 1756800000000i64 });
            AiDocumentMount::from_map(v.as_object().unwrap().clone())
        };
        let mut s = mounts::State::default();
        s.source = Some(PathBuf::from("a.md"));
        s.mounts = vec![mk("message", "问答"), mk("session", "整段")];
        let (session, message) = mounts::ordered(&s);
        assert_eq!((session, message), (vec![1], vec![0]));
        let lay = mounts::layout(&s, AREA);
        assert_eq!(
            lay.headings.iter().map(|(_, t)| *t).collect::<Vec<_>>(),
            vec!["会话挂载", "对话挂载"]
        );
        let mut list = DrawList::new();
        mounts::paint(&mut list, AREA, &s, &lay, theme::tokens().palette(false));
        let t = texts(&list);
        assert!(t.contains(&"整段".to_owned()) && t.contains(&"整个会话".to_owned()));
        assert!(list.finish().is_ok());
    }

    fn inbox_entry(id: &str, kind: &str, path: &str, title: &str) -> InboxEntry {
        let value = serde_json::json!({
            "id": id,
            "createdAt": 1756800000000i64,
            "taskId": null,
            "taskName": "夜间整理",
            "operation": {
                "id": format!("op-{id}"),
                "kind": kind,
                "path": path,
                "title": title,
                "summary": "待处理修改",
                "reason": "permission-suggest",
                "status": "pending"
            }
        });
        InboxEntry::from_value(&value).unwrap()
    }

    #[test]
    fn inbox_entries_for_same_document_are_one_compact_group() {
        let mut s = inbox::State::default();
        s.entries = (0..45)
            .map(|i| {
                inbox_entry(
                    &format!("i{i}"),
                    "overwrite",
                    r"D:\mochi\知识库\Code\算法\算法学习.mcb",
                    "算法学习.mcb",
                )
            })
            .collect();

        let grouped = inbox::groups(&s);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].indices.len(), 45);
        assert_eq!(grouped[0].name, "算法学习.mcb");

        let lay = inbox::layout(&s, AREA);
        assert_eq!(lay.cards.len(), 1, "同一完整路径只占一行");
        assert_eq!(lay.cards[0].1.height(), 48.0);
        let open = lay.rect_of(inbox::Hit::Open(0)).unwrap();
        let approve = lay.rect_of(inbox::Hit::Approve(0)).unwrap();
        let reject = lay.rect_of(inbox::Hit::Reject(0)).unwrap();
        assert_eq!(
            lay.hit(open.left + 4.0, open.top + 20.0),
            Some(inbox::Hit::Open(0))
        );
        assert_eq!(
            lay.hit(approve.left + 4.0, approve.top + 4.0),
            Some(inbox::Hit::Approve(0))
        );
        assert_eq!(
            lay.hit(reject.left + 4.0, reject.top + 4.0),
            Some(inbox::Hit::Reject(0))
        );

        let mut list = DrawList::new();
        inbox::paint(&mut list, AREA, &s, &lay, theme::tokens().palette(false));
        let t = texts(&list);
        assert!(t.contains(&"文档".to_owned()));
        assert!(t.contains(&"算法学习.mcb · 45 项".to_owned()));
        assert!(t.contains(&"全部通过".to_owned()) && t.contains(&"全部拒绝".to_owned()));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn inbox_header_batch_actions_and_independent_request_groups_are_hit() {
        let mut s = inbox::State::default();
        s.entries = vec![
            inbox_entry("file-1", "write", "D:/ws/a.md", "a.md"),
            inbox_entry("file-2", "overwrite", r"d:\ws\A.md", "a.md"),
            inbox_entry("shell-1", "native-shell-command", "D:/ws", "运行命令"),
            inbox_entry("shell-2", "native-shell-command", "D:/ws", "运行命令"),
            inbox_entry(
                "export-1",
                "native-document-export",
                "D:/ws/a.pdf",
                "导出 PDF",
            ),
            inbox_entry("delete-1", "delete-file", "D:/ws/a.md", "删除 a.md"),
            inbox_entry("rename-1", "rename", "D:/ws/a.md", "重命名 a.md"),
        ];
        let grouped = inbox::grouped(&s);
        assert_eq!(
            grouped.len(),
            6,
            "内容修改按路径聚合，shell/export/delete/rename 各自独立"
        );
        assert_eq!(grouped[0].indices, vec![0, 1]);
        assert_eq!(grouped[1].indices, vec![2]);
        assert_eq!(grouped[2].indices, vec![3]);
        assert_eq!(grouped[3].indices, vec![4]);
        assert_eq!(grouped[4].indices, vec![5]);
        assert_eq!(grouped[5].indices, vec![6]);

        let lay = inbox::layout(&s, AREA);
        let approve_all = lay.rect_of(inbox::Hit::ApproveAll).unwrap();
        let reject_all = lay.rect_of(inbox::Hit::RejectAll).unwrap();
        let refresh = lay.rect_of(inbox::Hit::Refresh).unwrap();
        assert_eq!(
            lay.hit(approve_all.left + 4.0, approve_all.top + 4.0),
            Some(inbox::Hit::ApproveAll)
        );
        assert_eq!(
            lay.hit(reject_all.left + 4.0, reject_all.top + 4.0),
            Some(inbox::Hit::RejectAll)
        );
        assert_eq!(
            lay.hit(refresh.left + 4.0, refresh.top + 4.0),
            Some(inbox::Hit::Refresh)
        );
        assert!(lay.rect_of(inbox::Hit::Open(0)).is_some());
        assert!(lay.rect_of(inbox::Hit::Open(1)).is_some());
    }

    #[test]
    fn inbox_same_names_in_different_directories_stay_separate() {
        let mut s = inbox::State::default();
        s.entries = vec![
            inbox_entry("a", "overwrite", "D:/ws/one/note.md", "note.md"),
            inbox_entry("b", "overwrite", "D:/ws/two/note.md", "note.md"),
        ];
        let grouped = inbox::groups(&s);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].path, "D:/ws/one/note.md");
        assert_eq!(grouped[1].path, "D:/ws/two/note.md");
    }

    #[test]
    fn empty_states_say_something_and_zero_areas_paint_nothing() {
        let p = theme::tokens().palette(false);
        let s = inbox::State::default();
        let mut list = DrawList::new();
        inbox::paint(&mut list, AREA, &s, &inbox::layout(&s, AREA), p);
        assert!(texts(&list).contains(&"没有待批准的操作".to_owned()));
        let mut list = DrawList::new();
        inbox::paint(&mut list, Rect::ZERO, &s, &inbox::layout(&s, Rect::ZERO), p);
        assert!(list.is_empty());
        let mut list = DrawList::new();
        let m = mounts::State::default();
        mounts::paint(&mut list, AREA, &m, &mounts::layout(&m, AREA), p);
        assert!(texts(&list).contains(&"打开一个文档后查看挂载的会话和对话".to_owned()));
    }
}
