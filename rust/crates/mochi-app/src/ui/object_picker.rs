//! 只返回对象选择结果，不打开系统对话框，也不执行挂载或写入。

use mochi_core::object_reference::{ObjectCandidate, ObjectKind, ObjectReference, ObjectScope};

use super::draw::{DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text;
use super::theme::{self, Palette};
use super::widgets::{FieldKey, FieldLook, TextField};

mod refresh;
pub(crate) use refresh::reconcile_candidates;

pub const MAX_RESULTS: usize = 100;
const PANEL_MAX_WIDTH: f32 = 680.0;
const PANEL_MAX_HEIGHT: f32 = 680.0;
const PANEL_RADIUS: f32 = 12.0;
const HEADER_HEIGHT: f32 = 132.0;
const SCOPE_HEIGHT: f32 = 34.0;
const SELECTED_HEIGHT: f32 = 48.0;
const ROW_HEIGHT: f32 = 48.0;
const FOOTER_HEIGHT: f32 = 54.0;
const PADDING: f32 = 18.0;

/// 选择器里的一个命中目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Backdrop,
    Inside,
    Query,
    Close,
    Scope(ObjectScope),
    Selected(usize),
    Candidate(usize),
    Cancel,
    Confirm,
}

/// 绘制、鼠标输入与测试共用的几何。
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub panel: Rect,
    pub title: Rect,
    pub query: Rect,
    pub scopes: Vec<(Rect, ObjectScope)>,
    pub selected: Rect,
    pub list: Rect,
    pub footer: Rect,
    pub entries: Vec<(Rect, Hit)>,
    pub content_height: f32,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Hit {
        self.entries
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, hit)| *hit)
            .unwrap_or_else(|| {
                if self.panel.contains(x, y) {
                    Hit::Inside
                } else {
                    Hit::Backdrop
                }
            })
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.list.height()).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyResult {
    Edited,
    Moved,
    Submit,
    Cancel,
    Ignored,
}

/// 可搜索、可多选的选择器状态。
#[derive(Debug, Clone)]
pub struct State {
    pub title: String,
    pub description: String,
    pub query: TextField,
    pub candidates: Vec<ObjectCandidate>,
    pub scope: ObjectScope,
    /// 把行和粘贴的链接限定为工作区文档对象。AI 文件挂载和 Document
    /// 字段会置起这个标志；通用引用字段保持关闭，
    /// 日程和 AI 对象才能继续被选。
    pub documents_only: bool,
    /// URL 按插入顺序保存，确保多维表格单元格的差异保持稳定。
    pub selected: Vec<String>,
    pub scroll: f32,
    pub selected_row: usize,
    pub hover: Option<Hit>,
}

impl State {
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        candidates: Vec<ObjectCandidate>,
        selected: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut query = TextField::new("搜索名称、路径，或粘贴 Mochi 链接…");
        query.style = TextStyle::Label;
        let mut state = Self {
            title: title.into(),
            description: description.into(),
            query,
            candidates,
            scope: ObjectScope::All,
            documents_only: false,
            selected: Vec::new(),
            scroll: 0.0,
            selected_row: 0,
            hover: None,
        };
        for url in selected {
            state.select_url(url);
        }
        state
    }

    pub fn for_references(
        candidates: Vec<ObjectCandidate>,
        selected: impl IntoIterator<Item = String>,
    ) -> Self {
        Self::new(
            "关联 Mochi 对象",
            "可搜索并多选文档、表格记录、日程与 AI 对话。",
            candidates,
            selected,
        )
    }

    pub fn for_ai_mount(
        candidates: Vec<ObjectCandidate>,
        selected: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut state = Self::new(
            "挂载文档",
            "选择工作区中的文档或文档块，挂载到当前 AI 会话。",
            candidates,
            selected,
        );
        state.set_documents_only(true);
        state
    }

    pub fn set_documents_only(&mut self, enabled: bool) {
        self.documents_only = enabled;
        if enabled {
            self.scope = ObjectScope::Files;
            self.selected.retain(|url| {
                ObjectReference::parse(url).is_some_and(|reference| reference.is_document())
            });
        }
        self.query_changed();
    }

    fn accepts_kind(&self, kind: ObjectKind) -> bool {
        (!self.documents_only
            || matches!(
                kind,
                ObjectKind::Document | ObjectKind::Directory | ObjectKind::Block
            ))
            && self.scope.accepts(kind)
    }

    fn accepts_reference(&self, reference: &ObjectReference) -> bool {
        self.accepts_kind(reference.kind)
    }

    fn select_url(&mut self, url: String) {
        if ObjectReference::parse(&url).is_some_and(|reference| self.accepts_reference(&reference))
            && !self.selected.iter().any(|item| item == &url)
        {
            self.selected.push(url);
        }
    }

    pub fn selected_urls(&self) -> Vec<String> {
        self.selected.clone()
    }

    pub fn is_selected(&self, url: &str) -> bool {
        self.selected.iter().any(|item| item == url)
    }

    pub fn toggle_url(&mut self, url: &str) {
        if let Some(index) = self.selected.iter().position(|item| item == url) {
            self.selected.remove(index);
        } else if ObjectReference::parse(url)
            .is_some_and(|reference| self.accepts_reference(&reference))
        {
            self.selected.push(url.to_owned());
        }
    }

    pub fn set_scope(&mut self, scope: ObjectScope) {
        if self.scope != scope {
            self.scope = scope;
            self.scroll = 0.0;
            self.selected_row = 0;
            self.hover = None;
        }
    }

    pub fn query_changed(&mut self) {
        self.scroll = 0.0;
        self.selected_row = 0;
        self.hover = None;
    }

    fn matching_candidates(&self) -> Vec<(usize, ObjectCandidate)> {
        let query = self.query.text().trim();
        let mut rows = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| self.accepts_kind(candidate.kind) && candidate.matches(query))
            .map(|(index, candidate)| (index, candidate.clone()))
            .collect::<Vec<_>>();
        if let Some(reference) = ObjectReference::parse(query) {
            if self.accepts_reference(&reference)
                && !rows.iter().any(|(_, candidate)| candidate.url == query)
            {
                rows.insert(
                    0,
                    (
                        usize::MAX,
                        ObjectCandidate::new(
                            query,
                            reference.display_label(),
                            reference.kind.label(),
                            reference.kind,
                        ),
                    ),
                );
            }
        }
        rows
    }

    /// 搜索候选，并把合法的粘贴 Mochi 链接只收录一次。UI 会限制渲染
    /// 行数，`filtered_candidate_count` 暴露完整的匹配数，
    /// 底栏才能说明还有更多结果。
    pub fn filtered_candidates(&self) -> Vec<(usize, ObjectCandidate)> {
        let mut rows = self.matching_candidates();
        rows.truncate(MAX_RESULTS);
        rows
    }

    pub fn filtered_candidate_count(&self) -> usize {
        self.matching_candidates().len()
    }

    pub fn visible_count(&self) -> usize {
        self.filtered_candidates().len()
    }

    pub fn toggle_row(&mut self, row: usize) -> bool {
        let Some((_, candidate)) = self.filtered_candidates().get(row).cloned() else {
            return false;
        };
        let url = candidate.url;
        self.toggle_url(&url);
        self.selected_row = row.min(self.visible_count().saturating_sub(1));
        true
    }

    pub fn remove_selected(&mut self, index: usize) -> bool {
        if index >= self.selected.len() {
            return false;
        }
        self.selected.remove(index);
        true
    }

    pub fn move_selection(&mut self, delta: i32) -> bool {
        let count = self.visible_count();
        if count == 0 {
            self.selected_row = 0;
            return false;
        }
        let next = (self.selected_row as i32 + delta).clamp(0, count as i32 - 1) as usize;
        let changed = next != self.selected_row;
        self.selected_row = next;
        changed
    }

    pub fn key(&mut self, key: u16, shift: bool, ctrl: bool) -> KeyResult {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
        match key {
            k if k == VK_DOWN.0 => {
                self.move_selection(1);
                KeyResult::Moved
            }
            k if k == VK_UP.0 => {
                self.move_selection(-1);
                KeyResult::Moved
            }
            k if k == VK_RETURN.0 => {
                if ctrl {
                    KeyResult::Submit
                } else {
                    if self.visible_count() > 0 {
                        self.toggle_row(self.selected_row);
                    }
                    KeyResult::Edited
                }
            }
            k if k == VK_ESCAPE.0 => KeyResult::Cancel,
            _ => match self.query.key(key, shift, ctrl) {
                FieldKey::Edited => {
                    self.query_changed();
                    KeyResult::Edited
                }
                FieldKey::Submit => KeyResult::Submit,
                FieldKey::Cancel => KeyResult::Cancel,
                FieldKey::Ignored => KeyResult::Ignored,
            },
        }
    }

    pub fn char(&mut self, character: char) -> bool {
        let changed = self.query.char(character);
        if changed {
            self.query_changed();
        }
        changed
    }

    pub fn click(&mut self, layout: &Layout, x: f32, y: f32) -> PickerAction {
        match layout.hit(x, y) {
            Hit::Backdrop | Hit::Close | Hit::Cancel => PickerAction::Cancel,
            Hit::Confirm => PickerAction::Confirm(self.selected_urls()),
            Hit::Scope(scope) => {
                self.set_scope(scope);
                PickerAction::Changed
            }
            Hit::Selected(index) => {
                self.remove_selected(index);
                PickerAction::Changed
            }
            Hit::Candidate(index) => {
                if self.toggle_row(index) {
                    PickerAction::Changed
                } else {
                    PickerAction::None
                }
            }
            Hit::Query | Hit::Inside => PickerAction::FocusQuery,
        }
    }

    pub fn hover(&mut self, layout: &Layout, x: f32, y: f32) -> bool {
        let next = match layout.hit(x, y) {
            Hit::Backdrop | Hit::Inside | Hit::Query => None,
            hit => Some(hit),
        };
        let changed = next != self.hover;
        self.hover = next;
        changed
    }

    pub fn scroll_by(&mut self, layout: &Layout, notches: f32) {
        self.scroll = (self.scroll - notches * ROW_HEIGHT * 3.0).clamp(0.0, layout.max_scroll());
        self.hover = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    None,
    Changed,
    FocusQuery,
    Cancel,
    Confirm(Vec<String>),
}

pub fn layout(state: &State, viewport: Rect) -> Layout {
    let mut layout = Layout::default();
    if viewport.is_empty() {
        return layout;
    }
    // 紧凑窗口下，桌面最小尺寸必须让位于实际视口；
    // 否则面板会超出命中测试表面，关闭/确认按钮点不到。
    let width = PANEL_MAX_WIDTH.min((viewport.width() - 16.0).max(1.0));
    let height = PANEL_MAX_HEIGHT.min((viewport.height() - 16.0).max(1.0));
    let left = ((viewport.left + viewport.right - width) / 2.0).round();
    let top = ((viewport.top + viewport.bottom - height) / 2.0).round();
    let panel = Rect::new(left, top, left + width, top + height);
    layout.panel = panel;
    // 背景遮罩在命中栈中必须低于所有控件。条目按从后向前检查，
    // 把遮罩放在最后会吞掉本该给面板的点击，对话框就像卡死了。
    layout.entries.push((viewport, Hit::Backdrop));
    layout.entries.push((panel, Hit::Inside));
    layout.title = Rect::new(
        panel.left + PADDING,
        panel.top + 14.0,
        panel.right - 58.0,
        panel.top + 42.0,
    );
    layout.entries.push((
        Rect::new(
            panel.right - 42.0,
            panel.top + 12.0,
            panel.right - 14.0,
            panel.top + 40.0,
        ),
        Hit::Close,
    ));

    layout.query = Rect::new(
        panel.left + PADDING,
        panel.top + 64.0,
        panel.right - PADDING,
        panel.top + 102.0,
    );
    layout.entries.push((layout.query, Hit::Query));
    let mut y = panel.top + HEADER_HEIGHT;
    let scopes_width = panel.width() - PADDING * 2.0;
    let scopes: &[ObjectScope] = if state.documents_only {
        &[ObjectScope::Files]
    } else {
        &ObjectScope::ALL
    };
    let scope_width = (scopes_width / scopes.len() as f32).max(70.0);
    let mut x = panel.left + PADDING;
    for &scope in scopes {
        let rect = Rect::new(
            x,
            y,
            (x + scope_width).min(panel.right - PADDING),
            y + SCOPE_HEIGHT,
        );
        layout.scopes.push((rect, scope));
        layout.entries.push((rect, Hit::Scope(scope)));
        x += scope_width;
        if x >= panel.right - PADDING {
            break;
        }
    }
    y += SCOPE_HEIGHT + 8.0;

    if !state.selected.is_empty() {
        layout.selected = Rect::new(
            panel.left + PADDING,
            y,
            panel.right - PADDING,
            y + SELECTED_HEIGHT,
        );
        for (index, rect, _) in selected_chips(state, layout.selected) {
            layout.entries.push((rect, Hit::Selected(index)));
        }
        y += SELECTED_HEIGHT + 4.0;
    }

    let footer_top = panel.bottom - FOOTER_HEIGHT;
    layout.list = Rect::new(
        panel.left + PADDING,
        y,
        panel.right - PADDING,
        (footer_top - 1.0).max(y),
    );
    layout.footer = Rect::new(panel.left, footer_top, panel.right, panel.bottom);
    let rows = state.filtered_candidates().len();
    layout.content_height = rows as f32 * ROW_HEIGHT;
    for row in 0..rows {
        let top = layout.list.top + row as f32 * ROW_HEIGHT - state.scroll;
        let bottom = top + ROW_HEIGHT;
        if bottom <= layout.list.top || top >= layout.list.bottom {
            continue;
        }
        let clipped_top = top.max(layout.list.top);
        let clipped_bottom = bottom.min(layout.list.bottom);
        if clipped_bottom > clipped_top {
            layout.entries.push((
                Rect::new(
                    layout.list.left,
                    clipped_top,
                    layout.list.right,
                    clipped_bottom,
                ),
                Hit::Candidate(row),
            ));
        }
    }
    let cancel = Rect::new(
        panel.right - 190.0,
        footer_top + 10.0,
        panel.right - 104.0,
        panel.bottom - 10.0,
    );
    let confirm = Rect::new(
        panel.right - 96.0,
        footer_top + 10.0,
        panel.right - PADDING,
        panel.bottom - 10.0,
    );
    layout.entries.push((cancel, Hit::Cancel));
    layout.entries.push((confirm, Hit::Confirm));
    layout
}

fn kind_icon(kind: ObjectKind) -> Icon {
    match kind {
        ObjectKind::Document | ObjectKind::Block => Icon::FILE_TEXT,
        ObjectKind::PdfAnnotation => Icon::FILE,
        ObjectKind::Directory => Icon::FOLDER,
        ObjectKind::Record => Icon::TABLE,
        ObjectKind::Task | ObjectKind::Event | ObjectKind::Project => Icon::CALENDAR_DAYS,
        ObjectKind::AiSession | ObjectKind::AiMessage => Icon::SPARKLES,
    }
}

fn kind_color(kind: ObjectKind, p: &Palette) -> u32 {
    match kind {
        ObjectKind::Document | ObjectKind::Directory | ObjectKind::Block => p.accent,
        ObjectKind::PdfAnnotation => 0xdc2626,
        ObjectKind::Record => 0x8b5cf6,
        ObjectKind::Task | ObjectKind::Event | ObjectKind::Project => 0xf59e0b,
        ObjectKind::AiSession | ObjectKind::AiMessage => 0x22c55e,
    }
}

fn selected_chips(state: &State, area: Rect) -> Vec<(usize, Rect, String)> {
    let mut chips = Vec::new();
    let mut x = area.left + 8.0;
    for (index, url) in state.selected.iter().enumerate() {
        if x + 28.0 > area.right {
            break;
        }
        let label = ObjectReference::parse(url)
            .map(|reference| reference.display_label())
            .unwrap_or_else(|| url.clone());
        let width = (text::measure(&label, TextStyle::Tiny) + 26.0)
            .max(28.0)
            .min(area.right - x);
        chips.push((
            index,
            Rect::new(x, area.top + 8.0, x + width, area.bottom - 8.0),
            label,
        ));
        x += width + 5.0;
    }
    chips
}

pub fn paint(state: &mut State, list: &mut DrawList, viewport: Rect, p: &Palette) {
    let layout = layout(state, viewport);
    if layout.panel.is_empty() {
        return;
    }
    list.rect_alpha(viewport, 0x000000, 0.48);
    let shadow = Rect::new(
        layout.panel.left + 2.0,
        layout.panel.top + 3.0,
        layout.panel.right + 2.0,
        layout.panel.bottom + 3.0,
    );
    list.rounded_rect_alpha(shadow, PANEL_RADIUS, 0x000000, 0.22);
    list.rounded_rect(layout.panel, PANEL_RADIUS, p.surface_elevated);
    list.rounded_border(layout.panel, PANEL_RADIUS, p.border);
    list.text(layout.title, &state.title, TextStyle::Large, p.foreground);
    list.icon_centered(
        Rect::new(
            layout.panel.right - 44.0,
            layout.panel.top + 12.0,
            layout.panel.right - 14.0,
            layout.panel.top + 40.0,
        ),
        Icon::X,
        17.0,
        p.muted,
    );
    if !state.description.is_empty() {
        list.text(
            Rect::new(
                layout.panel.left + PADDING,
                layout.panel.top + 44.0,
                layout.panel.right - PADDING,
                layout.panel.top + 60.0,
            ),
            text::ellipsize(
                &state.description,
                TextStyle::Tiny,
                layout.panel.width() - PADDING * 2.0,
            ),
            TextStyle::Tiny,
            p.muted,
        );
    }
    state
        .query
        .paint(list, layout.query, true, p, FieldLook::dialog(p));
    for (rect, scope) in &layout.scopes {
        let active = *scope == state.scope;
        if active || state.hover == Some(Hit::Scope(*scope)) {
            list.rounded_rect(
                *rect,
                6.0,
                if active {
                    theme::mix(p.accent, p.surface_elevated, 0.16)
                } else {
                    p.surface_muted
                },
            );
        }
        list.text(
            Rect::new(rect.left + 8.0, rect.top, rect.right - 8.0, rect.bottom),
            scope.label(),
            TextStyle::Caption,
            if active { p.accent } else { p.muted },
        );
    }
    if !layout.selected.is_empty() {
        list.rounded_rect(layout.selected, 6.0, p.surface_muted);
        for (_, chip, label) in selected_chips(state, layout.selected) {
            let width = chip.width();
            list.rounded_rect(chip, 5.0, theme::mix(p.accent, p.background, 0.18));
            list.text(
                Rect::new(chip.left + 7.0, chip.top, chip.right - 18.0, chip.bottom),
                text::ellipsize(&label, TextStyle::Tiny, (width - 24.0).max(4.0)),
                TextStyle::Tiny,
                p.accent,
            );
            list.text(
                Rect::new(chip.right - 15.0, chip.top, chip.right - 4.0, chip.bottom),
                "×",
                TextStyle::Tiny,
                p.muted,
            );
        }
    }
    list.push_clip(layout.list);
    for (row, (_, candidate)) in state.filtered_candidates().iter().enumerate() {
        let top = layout.list.top + row as f32 * ROW_HEIGHT - state.scroll;
        let rect = Rect::new(layout.list.left, top, layout.list.right, top + ROW_HEIGHT);
        if rect.bottom <= layout.list.top || rect.top >= layout.list.bottom {
            continue;
        }
        let selected = state.is_selected(&candidate.url);
        if selected || state.hover == Some(Hit::Candidate(row)) {
            list.rounded_rect(
                rect.inset(super::layout::Edges {
                    left: 2.0,
                    top: 2.0,
                    right: 2.0,
                    bottom: 2.0,
                }),
                6.0,
                if selected {
                    theme::mix(p.accent, p.surface_elevated, 0.10)
                } else {
                    p.surface_muted
                },
            );
        }
        let color = kind_color(candidate.kind, p);
        list.icon_centered(
            Rect::new(
                rect.left + 10.0,
                rect.top + 8.0,
                rect.left + 30.0,
                rect.bottom - 8.0,
            ),
            kind_icon(candidate.kind),
            16.0,
            color,
        );
        list.text(
            Rect::new(
                rect.left + 40.0,
                rect.top + 5.0,
                rect.right - 50.0,
                rect.top + 24.0,
            ),
            text::ellipsize(&candidate.title, TextStyle::Caption, rect.width() - 94.0),
            TextStyle::Caption,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 40.0,
                rect.top + 25.0,
                rect.right - 50.0,
                rect.bottom - 4.0,
            ),
            text::ellipsize(
                &format!("{} · {}", candidate.kind.label(), candidate.description),
                TextStyle::Tiny,
                rect.width() - 94.0,
            ),
            TextStyle::Tiny,
            p.muted,
        );
        let check = Rect::new(
            rect.right - 32.0,
            rect.top + 14.0,
            rect.right - 14.0,
            rect.top + 32.0,
        );
        if selected {
            list.rounded_rect(check, 5.0, color);
            list.text(check, "✓", TextStyle::Tiny, p.accent_foreground);
        } else {
            list.rounded_border(check, 5.0, p.border);
        }
    }
    list.pop_clip();
    if state.filtered_candidates().is_empty() {
        list.text(
            Rect::new(
                layout.list.left + 12.0,
                layout.list.top + 26.0,
                layout.list.right - 12.0,
                layout.list.top + 56.0,
            ),
            if state.query.text().trim().is_empty() {
                "没有可选择的对象"
            } else {
                "没有找到匹配对象"
            },
            TextStyle::Caption,
            p.muted,
        );
    }
    list.hline(
        layout.footer.left,
        layout.footer.right,
        layout.footer.top,
        p.border,
    );
    let total = state.filtered_candidate_count();
    let shown = state.filtered_candidates().len();
    let footer_label = if total > shown {
        format!(
            "已选择 {} 项 · 显示 {shown}/{total}，继续搜索缩小结果 · Enter 选择 · Ctrl+Enter 确定",
            state.selected.len(),
        )
    } else {
        format!(
            "已选择 {} 项 · 共 {total} 个 · Enter 选择 · Ctrl+Enter 确定",
            state.selected.len(),
        )
    };
    list.text(
        Rect::new(
            layout.footer.left + PADDING,
            layout.footer.top,
            layout.footer.right - 210.0,
            layout.footer.bottom,
        ),
        text::ellipsize(
            &footer_label,
            TextStyle::Tiny,
            (layout.footer.width() - 228.0).max(0.0),
        ),
        TextStyle::Tiny,
        p.muted,
    );
    if state.hover == Some(Hit::Cancel) {
        list.rounded_rect(
            Rect::new(
                layout.footer.right - 190.0,
                layout.footer.top + 10.0,
                layout.footer.right - 104.0,
                layout.footer.bottom - 10.0,
            ),
            6.0,
            p.surface_muted,
        );
    }
    if state.hover == Some(Hit::Confirm) {
        list.rounded_rect(
            Rect::new(
                layout.footer.right - 96.0,
                layout.footer.top + 10.0,
                layout.footer.right - PADDING,
                layout.footer.bottom - 10.0,
            ),
            6.0,
            theme::mix(p.accent, p.surface_elevated, 0.84),
        );
    }
    list.text(
        Rect::new(
            layout.footer.right - 184.0,
            layout.footer.top + 10.0,
            layout.footer.right - 110.0,
            layout.footer.bottom - 10.0,
        ),
        "取消",
        TextStyle::Caption,
        p.foreground,
    );
    list.glass_button(
        Rect::new(
            layout.footer.right - 96.0,
            layout.footer.top + 10.0,
            layout.footer.right - PADDING,
            layout.footer.bottom - 10.0,
        ),
        6.0,
        p,
        false,
    );
    list.text(
        Rect::new(
            layout.footer.right - 88.0,
            layout.footer.top + 10.0,
            layout.footer.right - PADDING - 8.0,
            layout.footer.bottom - 10.0,
        ),
        "确定",
        TextStyle::Caption,
        p.button_foreground(),
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn selected_chip_remove_hits_follow_the_painted_text_width() {
        use super::*;
        let mut state = State::new(
            "选择对象",
            "",
            vec![],
            [
                "mochi://open?path=notes%2Fa.md&label=较长的中文文档名称".to_owned(),
                "mochi://open?path=notes%2Fb.md&label=second".to_owned(),
            ],
        );
        let viewport = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
        let lay = layout(&state, viewport);
        let chips = selected_chips(&state, lay.selected);
        let second = chips[1].1;
        assert_eq!(
            lay.hit(second.right - 9.0, second.top + 10.0),
            Hit::Selected(1)
        );
        state.click(&lay, second.right - 9.0, second.top + 10.0);
        assert_eq!(state.selected.len(), 1);
        let lay = layout(&state, viewport);
        let first = selected_chips(&state, lay.selected)[0].1;
        state.click(&lay, first.right - 9.0, first.top + 10.0);
        assert!(state.selected.is_empty());
    }
    use super::*;
    use mochi_core::object_reference::{
        build_block_reference, build_document_reference, ObjectKind,
    };
    use std::path::Path;

    fn picker() -> State {
        let workspace = Path::new("D:/workspace");
        let a = build_document_reference(
            Path::new("D:/workspace/a.md"),
            ObjectKind::Document,
            Some(workspace),
            Some("甲"),
        );
        let b = build_document_reference(
            Path::new("D:/workspace/b.md"),
            ObjectKind::Document,
            Some(workspace),
            Some("乙"),
        );
        State::for_references(
            vec![
                ObjectCandidate::new(a.clone(), "甲", "a.md", ObjectKind::Document),
                ObjectCandidate::new(b, "乙", "b.md", ObjectKind::Document),
            ],
            [a],
        )
    }

    #[test]
    fn geometry_keeps_controls_inside_panel_and_outside_hits_are_backdrop() {
        let state = picker();
        let viewport = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let layout = layout(&state, viewport);
        assert!(layout.panel.left > viewport.left);
        assert!(layout.panel.right < viewport.right);
        assert!(layout.panel.top > viewport.top);
        assert!(layout.panel.bottom < viewport.bottom);
        assert_eq!(layout.hit(1.0, 1.0), Hit::Backdrop);
        assert_eq!(
            layout.hit(layout.query.left + 2.0, layout.query.top + 2.0),
            Hit::Query
        );
        assert!(matches!(
            layout.hit(layout.panel.left + 4.0, layout.panel.top + 4.0),
            Hit::Inside | Hit::Close
        ));
        assert!(layout
            .entries
            .iter()
            .all(|(rect, _)| rect.left >= viewport.left && rect.right <= viewport.right));
    }

    #[test]
    fn filtering_and_multiselect_preserve_url_order() {
        let mut state = picker();
        assert_eq!(state.selected.len(), 1);
        assert_eq!(state.visible_count(), 2);
        state.toggle_row(1);
        assert_eq!(state.selected.len(), 2);
        assert!(state.selected[0].contains("a.md"));
        state.query.char('乙');
        assert_eq!(state.visible_count(), 1);
        state.toggle_row(0);
        assert_eq!(state.selected.len(), 1);
    }

    #[test]
    fn pasted_reference_is_a_selectable_row() {
        let mut state = picker();
        state.query.set_text("mochi://open?path=extra.md");
        state.query_changed();
        assert_eq!(state.visible_count(), 1);
        state.toggle_row(0);
        assert!(state.is_selected("mochi://open?path=extra.md"));
        assert!(state.is_selected("mochi://open?path=a.md&label=%E7%94%B2"));
    }

    #[test]
    fn scope_switch_excludes_unrelated_kinds() {
        let mut state = picker();
        state.candidates.push(ObjectCandidate::new(
            "mochi://open?path=schedule&kind=task&item=t",
            "任务",
            "日程任务",
            ObjectKind::Task,
        ));
        state.set_scope(ObjectScope::Schedule);
        assert_eq!(state.visible_count(), 1);
        state.set_scope(ObjectScope::Files);
        assert_eq!(state.visible_count(), 2);
    }

    #[test]
    fn document_mode_rejects_non_document_rows_and_enter_only_toggles() {
        let document = "mochi://open?path=notes.md".to_owned();
        let task = "mochi://open?path=schedule&kind=task&item=t".to_owned();
        let mut state = State::for_ai_mount(
            vec![
                ObjectCandidate::new(document.clone(), "文档", "notes.md", ObjectKind::Document),
                ObjectCandidate::new(task, "任务", "日程", ObjectKind::Task),
            ],
            [],
        );
        assert_eq!(state.scope, ObjectScope::Files);
        assert!(state.documents_only);
        assert_eq!(state.visible_count(), 1);
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        assert_eq!(state.key(VK_RETURN.0, false, false), KeyResult::Edited);
        assert_eq!(state.selected_urls(), vec![document]);
        assert_eq!(state.key(VK_RETURN.0, false, true), KeyResult::Submit);
    }

    #[test]
    fn files_scope_and_document_mode_include_persisted_block_rows() {
        let block = build_block_reference(
            Path::new("D:/workspace/notes.md"),
            "block_00000000-0000-4000-8000-000000000001",
            Some(Path::new("D:/workspace")),
            Some("块标题"),
        )
        .unwrap();
        let mut state = State::for_references(
            vec![ObjectCandidate::new(
                block.clone(),
                "块标题",
                "notes.md · block_00000000-0000-4000-8000-000000000001",
                ObjectKind::Block,
            )],
            [],
        );
        state.set_scope(ObjectScope::Files);
        assert_eq!(state.visible_count(), 1);
        state.set_documents_only(true);
        assert_eq!(state.visible_count(), 1);
        state.toggle_row(0);
        assert_eq!(state.selected_urls(), vec![block]);
    }

    #[test]
    fn compact_viewport_keeps_panel_inside_hit_surface() {
        let state = picker();
        let viewport = Rect::new(0.0, 0.0, 280.0, 250.0);
        let layout = layout(&state, viewport);
        assert!(layout.panel.left >= viewport.left);
        assert!(layout.panel.right <= viewport.right);
        assert!(layout.panel.top >= viewport.top);
        assert!(layout.panel.bottom <= viewport.bottom);
    }
}
