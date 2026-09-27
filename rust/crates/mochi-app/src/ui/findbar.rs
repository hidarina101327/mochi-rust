//! 匹配源码而非渲染文本；忽略大小写，命中不重叠。

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text;
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

pub const MAX_WIDTH: f32 = 420.0;
const RIGHT: f32 = 20.0;
const TOP: f32 = 56.0;
const RADIUS: f32 = 12.0;
const HEADER: f32 = 40.0;
const PAD: f32 = 12.0;
const ROW: f32 = 36.0;
const ROW_GAP: f32 = 8.0;

#[derive(Debug)]
pub struct State {
    pub query: TextField,
    pub replacement: TextField,
    pub replace_mode: bool,
    /// 源码里的匹配（字节范围）。
    pub matches: Vec<(usize, usize)>,
    pub active: Option<usize>,
    pub hover: Option<Hit>,
}

impl State {
    pub fn new(replace_mode: bool) -> State {
        let mut query = TextField::new("查找当前文档");
        query.style = TextStyle::Label;
        let mut replacement = TextField::new("替换为");
        replacement.style = TextStyle::Label;
        State {
            query,
            replacement,
            replace_mode,
            matches: Vec::new(),
            active: None,
            hover: None,
        }
    }

    /// 重新匹配。`keep_active` 为真时尽量保住当前序号（文档变了但查询没变）。
    pub fn refresh(&mut self, text: &str, keep_active: bool) {
        self.matches = find_matches(text, self.query.text());
        self.active = match (keep_active, self.active) {
            (true, Some(i)) if i < self.matches.len() => Some(i),
            _ if self.matches.is_empty() => None,
            (true, _) => None,
            (false, _) => Some(0),
        };
    }

    /// 上一个 / 下一个。返回新的活动匹配。
    pub fn navigate(&mut self, direction: i32) -> Option<(usize, usize)> {
        if self.matches.is_empty() {
            self.active = None;
            return None;
        }
        let n = self.matches.len() as i32;
        let next = match self.active {
            None => {
                if direction > 0 {
                    0
                } else {
                    n - 1
                }
            }
            Some(i) => (i as i32 + direction).rem_euclid(n),
        };
        self.active = Some(next as usize);
        Some(self.matches[next as usize])
    }

    pub fn counter_text(&self) -> String {
        if self.matches.is_empty() {
            "0/0".to_owned()
        } else {
            format!(
                "{}/{}",
                self.active.map(|i| i + 1).unwrap_or(0),
                self.matches.len()
            )
        }
    }

    /// 表格查找没有源码字节范围，只用占位匹配驱动计数器与上/下一个。
    pub fn set_match_count(&mut self, count: usize) {
        self.matches = (0..count).map(|i| (i, i + 1)).collect();
        if self.matches.is_empty() || self.active.is_none_or(|i| i >= count) {
            self.active = None;
        }
    }
}

/// 不区分大小写地找出全部匹配（字节范围，不重叠）。
///
/// 小写化可能改变字节长度（`İ` → `i̇`），所以不能在小写串上直接算偏移——
/// 逐字符小写化并记下每个小写字节对应的原始偏移，找到之后映射回去。
pub fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    let needle: String = query.trim().chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut lowered = String::with_capacity(text.len());
    let mut origin: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        for lc in c.to_lowercase() {
            let before = lowered.len();
            lowered.push(lc);
            for _ in before..lowered.len() {
                origin.push(i);
            }
        }
    }
    origin.push(text.len());
    let mut out = Vec::new();
    let mut from = 0;
    while from + needle.len() <= lowered.len() {
        let Some(pos) = lowered[from..].find(&needle) else {
            break;
        };
        let start = from + pos;
        let end = start + needle.len();
        out.push((origin[start], origin[end]));
        from = end.max(start + 1);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    TabFind,
    TabReplace,
    Close,
    Query,
    Prev,
    Next,
    Replacement,
    ReplaceOne,
    ReplaceAll,
    /// 卡片内空白：吞掉点击，别落到下面的文档上。
    Inside,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub card: Rect,
    pub entries: Vec<(Rect, Hit)>,
    pub query_rect: Rect,
    pub replacement_rect: Rect,
    pub counter: Rect,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }

    /// 输入框文字起点（含左内边距）。点击定位与 IME 候选窗定位要它。
    pub fn query_text_left(&self) -> f32 {
        self.query_rect.left + 4.0
    }

    pub fn replacement_text_left(&self) -> f32 {
        self.replacement_rect.left + 8.0
    }
}

/// `editor` 是整个编辑区（含工具栏）。
pub fn layout(editor: Rect, replace_mode: bool) -> Layout {
    layout_ex(editor, replace_mode, false)
}

/// `find_only` 用于多维表格：不提供替换页签，卡片稍稍下移以免挡住视图工具。
pub fn layout_ex(editor: Rect, replace_mode: bool, find_only: bool) -> Layout {
    let width = MAX_WIDTH.min(editor.width() - 24.0).max(200.0);
    let right = editor.right - RIGHT;
    let left = right - width;
    let top = editor.top + if find_only { 96.0 } else { TOP };
    let replace_mode = replace_mode && !find_only;
    let body_h = PAD + ROW + if replace_mode { ROW_GAP + ROW } else { 0.0 } + PAD;
    let card = Rect::new(left, top, right, top + HEADER + 1.0 + body_h);
    let mut lay = Layout {
        card,
        ..Layout::default()
    };
    lay.entries.push((card, Hit::Inside));

    // 头部页签：文字宽 + 下划线；「查找」右边距 mr-5
    let header = Rect::new(card.left, card.top, card.right, card.top + HEADER);
    let find_w = text::measure("查找", TextStyle::Label);
    let replace_w = text::measure("替换", TextStyle::Label);
    let find_tab = Rect::new(
        header.left + PAD,
        header.top,
        header.left + PAD + find_w,
        header.bottom,
    );
    lay.entries.push((find_tab, Hit::TabFind));
    if !find_only {
        let replace_tab = Rect::new(
            find_tab.right + 20.0,
            header.top,
            find_tab.right + 20.0 + replace_w,
            header.bottom,
        );
        lay.entries.push((replace_tab, Hit::TabReplace));
    }
    let close = Rect::new(
        header.right - PAD - 28.0,
        header.top + (HEADER - 28.0) / 2.0,
        header.right - PAD,
        header.top + (HEADER + 28.0) / 2.0,
    );
    lay.entries.push((close, Hit::Close));

    let row_top = header.bottom + 1.0 + PAD;
    let row = Rect::new(card.left + PAD, row_top, card.right - PAD, row_top + ROW);
    lay.query_rect = row;
    let next = Rect::new(
        row.right - 8.0 - 27.0,
        row.top + (ROW - 27.0) / 2.0,
        row.right - 8.0,
        row.top + (ROW + 27.0) / 2.0,
    );
    let prev = Rect::new(
        next.left - 4.0 - 27.0,
        next.top,
        next.left - 4.0,
        next.bottom,
    );
    lay.counter = Rect::new(prev.left - 4.0 - 48.0, row.top, prev.left - 4.0, row.bottom);
    lay.entries.push((
        Rect::new(row.left, row.top, lay.counter.left, row.bottom),
        Hit::Query,
    ));
    lay.entries.push((prev, Hit::Prev));
    lay.entries.push((next, Hit::Next));

    if replace_mode {
        let r_top = row.bottom + ROW_GAP;
        let all_w = text::measure("全部", TextStyle::Label) + 24.0;
        let one_w = text::measure("替换", TextStyle::Label) + 24.0 + 2.0;
        let all = Rect::new(
            card.right - PAD - all_w,
            r_top,
            card.right - PAD,
            r_top + ROW,
        );
        let one = Rect::new(all.left - 8.0 - one_w, r_top, all.left - 8.0, r_top + ROW);
        let field = Rect::new(card.left + PAD, r_top, one.left - 8.0, r_top + ROW);
        lay.replacement_rect = field;
        lay.entries.push((field, Hit::Replacement));
        lay.entries.push((one, Hit::ReplaceOne));
        lay.entries.push((all, Hit::ReplaceAll));
    }
    lay
}

/// 谁拿着键盘焦点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    None,
    Query,
    Replacement,
}

pub fn paint(
    list: &mut DrawList,
    lay: &Layout,
    state: &mut State,
    focus: FocusTarget,
    p: &Palette,
) {
    let card = lay.card;
    // shadow-lg：两层偏移的暗色兑色
    list.rounded_rect(
        Rect::new(
            card.left + 2.0,
            card.top + 6.0,
            card.right + 2.0,
            card.bottom + 6.0,
        ),
        RADIUS,
        theme::mix(p.foreground, p.area_main_default, 0.05),
    );
    list.rounded_rect(
        Rect::new(card.left, card.top + 2.0, card.right, card.bottom + 2.0),
        RADIUS,
        theme::mix(p.foreground, p.area_main_default, 0.05),
    );
    list.rounded_rect(card, RADIUS, p.surface_elevated);
    list.rounded_border(card, RADIUS, p.border);

    let header_bottom = card.top + HEADER;
    list.hline(card.left, card.right, header_bottom, p.border);
    for (r, hit) in &lay.entries {
        match hit {
            Hit::TabFind | Hit::TabReplace => {
                let active = (*hit == Hit::TabFind) != state.replace_mode;
                let color = if active { p.foreground } else { p.muted };
                list.text(
                    *r,
                    if *hit == Hit::TabFind {
                        "查找"
                    } else {
                        "替换"
                    }
                    .to_owned(),
                    TextStyle::Label,
                    color,
                );
                if active {
                    list.rect(
                        Rect::new(r.left, r.bottom - 2.0, r.right, r.bottom),
                        p.accent,
                    );
                }
            }
            Hit::Close => {
                let color = if state.hover == Some(Hit::Close) {
                    list.rounded_rect(*r, 4.0, p.surface_muted);
                    p.foreground
                } else {
                    p.muted
                };
                list.icon_centered(*r, Icon::X, 16.0, color);
            }
            _ => {}
        }
    }

    let row = lay.query_rect;
    list.rounded_rect(row, 8.0, p.background);
    list.rounded_border(row, 8.0, p.border);
    list.icon_centered(
        Rect::new(row.left + 8.0, row.top, row.left + 8.0 + 15.0, row.bottom),
        Icon::SEARCH,
        15.0,
        p.muted,
    );
    let field = Rect::new(row.left + 8.0 + 15.0, row.top, lay.counter.left, row.bottom);
    state.query.paint(
        list,
        field,
        focus == FocusTarget::Query,
        p,
        FieldLook {
            padding_left: 4.0,
            padding_right: 4.0,
            ..FieldLook::bare()
        },
    );
    list.text_aligned(
        lay.counter,
        state.counter_text(),
        TextStyle::Caption,
        p.muted,
        Align::Center,
    );
    let disabled = state.matches.is_empty();
    for (r, hit) in &lay.entries {
        let icon = match hit {
            Hit::Prev => Icon::CHEVRON_UP,
            Hit::Next => Icon::CHEVRON_DOWN,
            _ => continue,
        };
        let color = if disabled {
            theme::mix(p.muted, p.background, 0.35)
        } else if state.hover == Some(*hit) {
            list.rounded_rect(*r, 4.0, p.surface_muted);
            p.muted
        } else {
            p.muted
        };
        list.icon_centered(*r, icon, 15.0, color);
    }

    if state.replace_mode {
        let field = lay.replacement_rect;
        list.rounded_rect(field, 8.0, p.background);
        list.rounded_border(field, 8.0, p.border);
        list.icon_centered(
            Rect::new(
                field.left + 8.0,
                field.top,
                field.left + 8.0 + 15.0,
                field.bottom,
            ),
            Icon::REPLACE,
            15.0,
            p.muted,
        );
        let inner = Rect::new(
            field.left + 8.0 + 15.0,
            field.top,
            field.right,
            field.bottom,
        );
        state.replacement.paint(
            list,
            inner,
            focus == FocusTarget::Replacement,
            p,
            FieldLook {
                padding_left: 8.0,
                padding_right: 8.0,
                ..FieldLook::bare()
            },
        );
        for (r, hit) in &lay.entries {
            match hit {
                Hit::ReplaceOne => {
                    if state.hover == Some(Hit::ReplaceOne) && !disabled {
                        list.rounded_rect(*r, 8.0, p.surface_muted);
                    }
                    list.rounded_border(*r, 8.0, p.border);
                    let color = if disabled {
                        theme::mix(p.foreground, p.surface_elevated, 0.4)
                    } else {
                        p.foreground
                    };
                    list.text_aligned(
                        *r,
                        "替换".to_owned(),
                        TextStyle::Label,
                        color,
                        Align::Center,
                    );
                }
                Hit::ReplaceAll => {
                    let start = list.cmds().len();
                    list.glass_button(
                        *r,
                        8.0,
                        p,
                        !disabled && state.hover == Some(Hit::ReplaceAll),
                    );
                    list.text_aligned(
                        *r,
                        "全部".to_owned(),
                        TextStyle::Label,
                        p.button_foreground(),
                        Align::Center,
                    );
                    if disabled {
                        list.fade_since(start, *r, 0.45);
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_is_case_insensitive_trimmed_and_non_overlapping() {
        assert_eq!(
            find_matches("Hello hello HELLO", " hello "),
            vec![(0, 5), (6, 11), (12, 17)]
        );
        assert_eq!(find_matches("aaaa", "aa"), vec![(0, 2), (2, 4)], "不重叠");
        assert!(find_matches("abc", "   ").is_empty());
        // 中文与多字节：返回的是源码字节范围
        let t = "前缀 测试 后缀";
        assert_eq!(
            find_matches(t, "测试"),
            vec![(t.find("测试").unwrap(), t.find("测试").unwrap() + 6)]
        );
    }

    #[test]
    fn lowercasing_that_changes_byte_length_still_maps_back_to_source_offsets() {
        // `İ` 小写成两个码点（i + 组合点），小写串比原串长
        let t = "xİy abc";
        let m = find_matches(t, "abc");
        assert_eq!(m, vec![(t.find("abc").unwrap(), t.len())]);
    }

    #[test]
    fn navigation_wraps_and_the_counter_reads_like_the_tsx() {
        let mut s = State::new(false);
        s.query.set_text("a");
        s.refresh("a a a", false);
        assert_eq!(s.matches.len(), 3);
        assert_eq!(s.active, Some(0), "有结果时先选中第一个");
        assert_eq!(s.counter_text(), "1/3");
        s.navigate(-1);
        assert_eq!(s.active, Some(2), "从头往前绕到末尾");
        s.navigate(1);
        assert_eq!(s.active, Some(0));
        s.query.set_text("zzz");
        s.refresh("a a a", false);
        assert_eq!(s.counter_text(), "0/0");
        assert_eq!(s.navigate(1), None);
    }

    #[test]
    fn refresh_after_an_edit_keeps_the_index_when_still_valid() {
        let mut s = State::new(false);
        s.query.set_text("a");
        s.refresh("a a a", false);
        s.navigate(1);
        assert_eq!(s.active, Some(1));
        s.refresh("a a", true);
        assert_eq!(s.active, Some(1));
        s.refresh("a", true);
        assert_eq!(s.active, None, "序号越界就清掉（TSX 同样置 -1）");
    }

    #[test]
    fn the_card_hangs_20px_from_the_right_and_56px_from_the_top_capped_at_420() {
        let editor = Rect::new(400.0, 68.0, 1200.0, 800.0);
        let lay = layout(editor, false);
        assert_eq!(lay.card.right, 1180.0);
        assert_eq!(lay.card.top, 124.0);
        assert_eq!(lay.card.width(), 420.0);
        // 窄编辑区：100% - 24
        let narrow = layout(Rect::new(0.0, 0.0, 300.0, 600.0), false);
        assert_eq!(narrow.card.width(), 276.0);
        // 替换模式多一行
        let tall = layout(editor, true);
        assert_eq!(tall.card.height(), lay.card.height() + ROW_GAP + ROW);
        assert!(tall.entries.iter().any(|(_, h)| *h == Hit::ReplaceAll));
        assert!(!lay.entries.iter().any(|(_, h)| *h == Hit::ReplaceAll));
    }

    #[test]
    fn find_only_layout_hides_replace_and_sits_lower() {
        let editor = Rect::new(400.0, 68.0, 1200.0, 800.0);
        let lay = layout_ex(editor, true, true);
        assert_eq!(lay.card.top, 164.0);
        assert!(!lay
            .entries
            .iter()
            .any(|(_, h)| matches!(h, Hit::TabReplace | Hit::ReplaceOne | Hit::ReplaceAll)));
        let mut s = State::new(false);
        s.set_match_count(3);
        assert_eq!(s.counter_text(), "0/3");
        assert_eq!(s.navigate(1), Some((0, 1)));
        assert_eq!(s.counter_text(), "1/3");
    }

    #[test]
    fn hits_prefer_controls_over_the_card_body() {
        let lay = layout(Rect::new(400.0, 68.0, 1200.0, 800.0), true);
        let (close, _) = lay.entries.iter().find(|(_, h)| *h == Hit::Close).unwrap();
        assert_eq!(
            lay.hit(
                (close.left + close.right) / 2.0,
                (close.top + close.bottom) / 2.0
            ),
            Some(Hit::Close)
        );
        assert_eq!(
            lay.hit(lay.card.left + 2.0, lay.card.bottom - 2.0),
            Some(Hit::Inside)
        );
        assert_eq!(lay.hit(lay.card.left - 5.0, lay.card.top), None);
    }

    #[test]
    fn painting_shows_tabs_counter_and_replace_buttons() {
        use crate::ui::draw::DrawCmd;
        let lay = layout(Rect::new(400.0, 68.0, 1200.0, 800.0), true);
        let mut s = State::new(true);
        s.query.set_text("x");
        s.refresh("x y x", false);
        let mut list = DrawList::new();
        paint(
            &mut list,
            &lay,
            &mut s,
            FocusTarget::Query,
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
        for expected in ["查找", "替换", "1/2", "全部"] {
            assert!(
                texts.iter().any(|t| t == expected),
                "缺 {expected}: {texts:?}"
            );
        }
        assert!(list.finish().is_ok());
    }
}
