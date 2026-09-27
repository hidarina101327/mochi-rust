//! 焦点和 IME 由 App 路由，各页面共用 TextField。

use super::draw::{Align, DrawList, TextStyle};
use super::editor::TextBuffer;
use super::icons::Icon;
use super::layout::{Edges, Rect};
use super::text;
use super::theme::{self, Palette};

const MULTILINE_PAD_X: f32 = 12.0;
const MULTILINE_PAD_Y: f32 = 8.0;

/// 单行输入框。
///
/// 内容放在 [`TextBuffer`] 里——光标/选区/IME 组合串的逻辑与编辑器共用，
/// 输入框只多两件事：横向滚动让光标可见、回车/Esc 交给调用方决定。
#[derive(Debug, Clone)]
pub struct TextField {
    scroll_y: f32,
    pub buffer: TextBuffer,
    pub placeholder: String,
    /// 横向滚动（像素）。文本比框宽时保证光标在视野里。
    scroll_x: f32,
    pub style: TextStyle,
    /// LaTeX 字段用公式对话框的多行编辑器和实时预览。
    pub(super) formula: bool,
    /// 跨多次垂直移动时保持首选列。
    vertical_goal: Option<(usize, u64, f32)>,
    manual_scroll: Option<(usize, u64)>,
}

/// 输入框上一次按键的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKey {
    /// 吃掉了，内容或光标变了。
    Edited,
    /// 回车。调用方决定是「确认」还是「提交搜索」。
    Submit,
    /// Esc。
    Cancel,
    /// 不是输入框认识的键。
    Ignored,
}

impl TextField {
    pub fn new(placeholder: impl Into<String>) -> Self {
        TextField {
            scroll_y: 0.0,
            buffer: TextBuffer::new(""),
            placeholder: placeholder.into(),
            scroll_x: 0.0,
            style: TextStyle::Body,
            formula: false,
            vertical_goal: None,
            manual_scroll: None,
        }
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.buffer = TextBuffer::new(text);
        self.buffer.set_cursor(text.len(), false);
        self
    }

    pub fn formula(text: &str) -> Self {
        let mut field = Self::new("输入 LaTeX 公式").with_text(text);
        field.formula = true;
        field.style = TextStyle::Mono;
        field
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.text().is_empty() && self.buffer.composition().is_none()
    }

    pub fn clear(&mut self) {
        self.vertical_goal = None;
        self.manual_scroll = None;
        self.buffer = TextBuffer::new("");
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
    }

    pub fn set_text(&mut self, text: &str) {
        self.vertical_goal = None;
        self.manual_scroll = None;
        self.buffer = TextBuffer::new(text);
        self.buffer.set_cursor(text.len(), false);
        self.scroll_y = 0.0;
    }

    pub fn select_all(&mut self) {
        self.vertical_goal = None;
        self.buffer.select_all();
    }

    /// 普通字符。控制字符一律拒收——回车走 [`Self::key`]。
    pub fn char(&mut self, ch: char) -> bool {
        self.vertical_goal = None;
        if ch.is_control() {
            return false;
        }
        self.buffer.insert(&ch.to_string());
        true
    }

    /// 按键。`key` 是 Win32 虚拟键码；这里只认导航/删除/回车/Esc/全选。
    pub fn key(&mut self, key: u16, shift: bool, ctrl: bool) -> FieldKey {
        self.vertical_goal = None;
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT,
            VK_UP,
        };
        match key {
            k if k == VK_RETURN.0 => FieldKey::Submit,
            k if k == VK_ESCAPE.0 => FieldKey::Cancel,
            k if k == VK_LEFT.0 => {
                self.buffer.move_left(shift);
                FieldKey::Edited
            }
            k if k == VK_RIGHT.0 => {
                self.buffer.move_right(shift);
                FieldKey::Edited
            }
            k if k == VK_HOME.0 || k == VK_UP.0 => {
                self.buffer.set_cursor(0, shift);
                FieldKey::Edited
            }
            k if k == VK_END.0 || k == VK_DOWN.0 => {
                let end = self.buffer.text().len();
                self.buffer.set_cursor(end, shift);
                FieldKey::Edited
            }
            k if k == VK_BACK.0 => {
                self.buffer.delete_backward();
                FieldKey::Edited
            }
            k if k == VK_DELETE.0 => {
                self.buffer.delete_forward();
                FieldKey::Edited
            }
            _ if ctrl && key == b'A' as u16 => {
                self.buffer.select_all();
                FieldKey::Edited
            }
            _ => FieldKey::Ignored,
        }
    }

    /// 点击定位光标。`x` 是相对文字起点（已扣掉内边距）的横坐标。
    pub fn click(&mut self, x: f32, extend: bool) {
        self.vertical_goal = None;
        let (shown, _) = self.buffer.display_text();
        let offset = offset_at_x(&shown, self.style, x + self.scroll_x);
        let source = self.buffer.source_offset_from_display(offset);
        self.buffer.set_cursor(source, extend);
    }

    /// 光标相对文字起点的横坐标（已扣掉滚动）。IME 候选窗定位要它。
    pub fn caret_x(&self) -> f32 {
        let (shown, _) = self.buffer.display_text();
        let cursor = self.buffer.display_cursor();
        text::caret_x(&shown, cursor, self.style) - self.scroll_x
    }

    fn masked(&self) -> Self {
        let source = self.text();
        let mut masked = TextField::new(&self.placeholder);
        masked.style = self.style;
        masked.set_text(&"•".repeat(source.chars().count()));
        let (a, b) = self.buffer.selection();
        let cursor = self.buffer.cursor();
        let anchor = if cursor == a { b } else { a };
        masked
            .buffer
            .set_cursor(source[..anchor].chars().count() * '•'.len_utf8(), false);
        masked
            .buffer
            .set_cursor(source[..cursor].chars().count() * '•'.len_utf8(), true);
        masked.scroll_x = self.scroll_x;
        masked
    }

    pub fn click_masked(&mut self, x: f32, extend: bool) {
        let mut masked = self.masked();
        masked.click(x, false);
        let count = masked.text()[..masked.buffer.cursor()].chars().count();
        let offset = self
            .text()
            .char_indices()
            .nth(count)
            .map(|(i, _)| i)
            .unwrap_or(self.text().len());
        self.buffer.set_cursor(offset, extend);
    }

    pub fn paint_masked(
        &mut self,
        list: &mut DrawList,
        rect: Rect,
        focused: bool,
        p: &Palette,
        look: FieldLook,
    ) {
        let mut masked = self.masked();
        masked.paint(list, rect, focused, p, look);
        self.scroll_x = masked.scroll_x;
    }

    /// 在不可变的呈现环节之前同步单行字段的滚动。
    /// 之后指针命中测试和 IME 就能使用与绘制的光标相同的滚动值。
    pub fn sync_singleline_scroll(&mut self, width: f32) {
        let width = width.max(1.0);
        let (shown, _) = self.buffer.display_text();
        let caret_abs = text::caret_x(&shown, self.buffer.display_cursor(), self.style);
        if caret_abs - self.scroll_x > width - 2.0 {
            self.scroll_x = caret_abs - width + 2.0;
        } else if caret_abs - self.scroll_x < 0.0 {
            self.scroll_x = caret_abs;
        }
        let total = text::measure(&shown, self.style);
        self.scroll_x = self.scroll_x.clamp(0.0, (total - width + 2.0).max(0.0));
    }

    fn multiline_text_layout(&self, width: f32) -> super::editor::EditorLayout {
        let (shown, _) = self.buffer.display_text();
        super::editor::layout(
            &shown,
            self.style,
            (width - MULTILINE_PAD_X * 2.0).max(40.0),
        )
    }

    /// 整个字段宽度下的自然高度，含文本内边距。
    /// 自动增高的输入框必须用与绘制相同的视口来测量。
    pub fn multiline_height(&self, width: f32) -> f32 {
        self.multiline_text_layout(width).height + MULTILINE_PAD_Y * 2.0
    }

    /// `rect` 是整个框（含边框与内边距）。
    fn multiline_layout(&self, rect: Rect) -> (Rect, super::editor::EditorLayout) {
        let editor_padding = super::editor::EDITOR_PADDING;
        let area = Rect::new(
            rect.left - (editor_padding - MULTILINE_PAD_X),
            rect.top - (editor_padding - MULTILINE_PAD_Y),
            rect.right + (editor_padding - MULTILINE_PAD_X),
            rect.bottom + (editor_padding - MULTILINE_PAD_Y),
        );
        (area, self.multiline_text_layout(rect.width()))
    }
    pub fn multiline_caret(&self, rect: Rect) -> Rect {
        let (area, lay) = self.multiline_layout(rect);
        super::editor::caret_rect(area, &lay, &self.buffer, self.style, self.scroll_y)
    }
    pub fn multiline_click(&mut self, rect: Rect, x: f32, y: f32) {
        self.multiline_select(rect, x, y, false);
    }
    pub fn multiline_select(&mut self, rect: Rect, x: f32, y: f32, extend: bool) {
        self.vertical_goal = None;
        let (_, lay) = self.multiline_layout(rect);
        let line = lay.line_at(y - rect.top - MULTILINE_PAD_Y + self.scroll_y, self.style);
        let offset = lay.offset_at(line, x - rect.left - MULTILINE_PAD_X, self.style);
        let offset = self.buffer.source_offset_from_display(offset);
        self.buffer.set_cursor(offset, extend);
    }
    pub fn multiline_key(&mut self, rect: Rect, key: u16, shift: bool, ctrl: bool) -> FieldKey {
        if key == 13 && !ctrl {
            self.vertical_goal = None;
            self.buffer.insert("\n");
            return FieldKey::Edited;
        }
        let (_, lay) = self.multiline_layout(rect);
        if key == 38 || key == 40 {
            let cursor = self.buffer.cursor();
            let revision = self.buffer.revision();
            let x = self
                .vertical_goal
                .filter(|(at, rev, _)| *at == cursor && *rev == revision)
                .map_or_else(|| lay.locate(cursor, self.style).1, |(_, _, x)| x);
            let next = lay.move_vertical(
                self.buffer.cursor(),
                if key == 38 { -1 } else { 1 },
                x,
                self.style,
            );
            self.buffer.set_cursor(next, shift);
            self.vertical_goal = Some((next, revision, x));
            return FieldKey::Edited;
        }
        if !ctrl && (key == 36 || key == 35) {
            self.vertical_goal = None;
            let (i, _) = lay.locate(self.buffer.cursor(), self.style);
            let (a, b) = lay.line_bounds(i);
            self.buffer.set_cursor(if key == 36 { a } else { b }, shift);
            return FieldKey::Edited;
        }
        self.key(key, shift, ctrl)
    }
    pub fn paint_multiline(&mut self, list: &mut DrawList, rect: Rect, focused: bool, p: &Palette) {
        self.paint_multiline_with_look(list, rect, focused, p, FieldLook::dialog(p));
    }
    pub fn sync_multiline_scroll(&mut self, rect: Rect) {
        let (area, lay) = self.multiline_layout(rect);
        let caret = super::editor::caret_rect(area, &lay, &self.buffer, self.style, self.scroll_y);
        if caret.top < rect.top + MULTILINE_PAD_Y {
            self.scroll_y -= (rect.top + MULTILINE_PAD_Y) - caret.top;
        } else if caret.bottom > rect.bottom - MULTILINE_PAD_Y {
            self.scroll_y += caret.bottom - (rect.bottom - MULTILINE_PAD_Y);
        }
        self.scroll_y = self.scroll_y.clamp(
            0.0,
            (lay.height - rect.height() + MULTILINE_PAD_Y * 2.0).max(0.0),
        );
    }
    pub fn scroll_multiline(&mut self, rect: Rect, step: f32) {
        let (_, layout) = self.multiline_layout(rect);
        self.scroll_y = (self.scroll_y - step).clamp(
            0.0,
            (layout.height - rect.height() + MULTILINE_PAD_Y * 2.0).max(0.0),
        );
        self.manual_scroll = Some((self.buffer.cursor(), self.buffer.revision()));
    }
    pub fn paint_multiline_with_look(
        &mut self,
        list: &mut DrawList,
        rect: Rect,
        focused: bool,
        p: &Palette,
        look: FieldLook,
    ) {
        if focused
            && (self.manual_scroll != Some((self.buffer.cursor(), self.buffer.revision()))
                || self.buffer.composition().is_some())
        {
            self.manual_scroll = None;
            self.sync_multiline_scroll(rect);
        }
        let (area, lay) = self.multiline_layout(rect);
        if let Some(bg) = look.background {
            list.rounded_rect(rect, look.radius, bg);
        }
        if let Some(border) = look.border {
            list.rounded_border(rect, look.radius, if focused { p.accent } else { border });
        }
        list.push_clip(rect);
        if self.is_empty() && !focused {
            list.text(
                Rect::new(
                    rect.left + MULTILINE_PAD_X,
                    rect.top + MULTILINE_PAD_Y,
                    rect.right - MULTILINE_PAD_X,
                    rect.top + MULTILINE_PAD_Y + self.style.line_height(),
                ),
                &self.placeholder,
                self.style,
                p.muted,
            );
        }
        super::editor::paint(
            list,
            area,
            &self.buffer,
            &lay,
            self.style,
            self.scroll_y,
            focused,
            p,
        );
        list.pop_clip();
    }

    ///
    /// 样式对应 Sidebar.tsx 的搜索框：`bg-background border border-border rounded`
    /// + 聚焦时 `ring-1`。`focused` 决定画不画光标与聚焦环。
    pub fn paint(
        &mut self,
        list: &mut DrawList,
        rect: Rect,
        focused: bool,
        p: &Palette,
        look: FieldLook,
    ) {
        if rect.is_empty() {
            return;
        }
        if let Some(bg) = look.background {
            list.rounded_rect(rect, look.radius, bg);
        }
        if let Some(border) = look.border {
            list.rounded_border(
                rect,
                look.radius,
                if focused {
                    look.focus_ring.unwrap_or(border)
                } else {
                    border
                },
            );
        }

        let inner = rect.inset(Edges {
            left: look.padding_left,
            top: 0.0,
            right: look.padding_right,
            bottom: 0.0,
        });
        if inner.is_empty() {
            return;
        }
        if let Some(icon) = look.leading_icon {
            // 图标在左内边距里居中
            let box_rect = Rect::new(
                rect.left + (look.padding_left - 16.0) / 2.0,
                rect.top,
                rect.left + (look.padding_left + 16.0) / 2.0,
                rect.bottom,
            );
            list.icon_centered(box_rect, icon, 16.0, p.muted);
        }

        // 先让光标可见，再画——顺序反了会画出上一帧的滚动位置
        self.sync_singleline_scroll(inner.width());
        let (shown, comp) = self.buffer.display_text();
        let total = text::measure(&shown, self.style);

        list.push_clip(inner);
        let text_left = inner.left - self.scroll_x;
        if shown.is_empty() {
            list.text(
                Rect::new(inner.left, inner.top, inner.right, inner.bottom),
                self.placeholder.clone(),
                self.style,
                p.muted,
            );
        } else {
            // 选区高亮
            // IME 组合串会在绘制时临时替换选区，显示文本的长度因此可能
            // 小于原始选区。此时不能再按提交文本的选区去切片，否则会越界
            // panic（例如全选英文名称后输入中文）。组合串自身会单独绘制。
            if focused && self.buffer.has_selection() && self.buffer.composition().is_none() {
                let (a, b) = self.buffer.selection();
                let x0 = text_left + text::caret_x(&shown, a, self.style);
                let x1 = text_left + text::caret_x(&shown, b, self.style);
                let pad = (inner.height() - self.style.line_height()) / 2.0;
                list.rect(
                    Rect::new(x0, inner.top + pad, x1, inner.bottom - pad),
                    theme::mix(p.accent, p.background, 0.25),
                );
            }
            list.text(
                Rect::new(text_left, inner.top, text_left + total + 4.0, inner.bottom),
                shown.clone(),
                self.style,
                p.foreground,
            );
            // 组合串下划线
            if let Some((a, b)) = comp {
                let x0 = text_left + text::caret_x(&shown, a, self.style);
                let x1 = text_left + text::caret_x(&shown, b, self.style);
                let pad = (inner.height() - self.style.line_height()) / 2.0;
                list.hline(x0, x1, inner.bottom - pad - 1.0, p.foreground);
            }
        }
        if focused {
            let x = inner.left + self.caret_x();
            let pad = (inner.height() - self.style.line_height()) / 2.0;
            list.caret(
                Rect::new(x, inner.top + pad, x + 1.0, inner.bottom - pad),
                p.foreground,
            );
        }
        list.pop_clip();
    }
}

/// 输入框外观。不同位置的输入框长得不一样（侧栏搜索框有边框和放大镜，
/// 导航轨的新建库输入框是石板蓝细边），差异全收在这里。
#[derive(Debug, Clone, Copy)]
pub struct FieldLook {
    pub radius: f32,
    pub background: Option<u32>,
    pub border: Option<u32>,
    pub focus_ring: Option<u32>,
    pub padding_left: f32,
    pub padding_right: f32,
    pub leading_icon: Option<Icon>,
}

impl FieldLook {
    /// `px-1 py-0.5 bg-surface border border-[#3C5A78] rounded`（GlobalNavigation 新建库）。
    pub fn inline(p: &Palette) -> Self {
        FieldLook {
            radius: 4.0,
            background: Some(p.surface),
            border: Some(p.accent),
            focus_ring: Some(p.accent),
            padding_left: 4.0,
            padding_right: 4.0,
            leading_icon: None,
        }
    }

    /// `px-1 py-0.5 bg-surface border border-accent rounded`（FileTreeNode 的 InlineEditor）。
    pub fn inline_accent(p: &Palette) -> Self {
        FieldLook {
            radius: 4.0,
            background: Some(p.surface),
            border: Some(p.accent),
            focus_ring: Some(p.accent),
            padding_left: 4.0,
            padding_right: 4.0,
            leading_icon: None,
        }
    }

    /// `px-3 py-2 text-sm border border-border rounded bg-background`（对话框里的输入框）。
    pub fn dialog(p: &Palette) -> Self {
        FieldLook {
            radius: 4.0,
            background: Some(p.background),
            border: Some(p.border),
            focus_ring: Some(p.accent),
            padding_left: 12.0,
            padding_right: 12.0,
            leading_icon: None,
        }
    }

    /// `pl-8 pr-8 py-1.5 bg-background border border-border rounded` + 放大镜（Sidebar 搜索框）。
    pub fn search(p: &Palette) -> Self {
        FieldLook {
            radius: 4.0,
            background: Some(p.background),
            border: Some(p.border),
            focus_ring: Some(p.accent),
            padding_left: 32.0,
            padding_right: 32.0,
            leading_icon: Some(Icon::SEARCH),
        }
    }

    /// 无边框、透明底（对话框里的大输入框）。
    pub fn bare() -> Self {
        FieldLook {
            radius: 0.0,
            background: None,
            border: None,
            focus_ring: None,
            padding_left: 0.0,
            padding_right: 0.0,
            leading_icon: None,
        }
    }
}

/// 一段文本里横坐标 `x` 落在哪个字节偏移上（就近原则：点在字的右半边算下一个）。
pub fn offset_at_x(s: &str, style: TextStyle, x: f32) -> usize {
    if let Some(shape) = super::measurement::shape(s, style, text::Emphasis::None) {
        return shape.hit(x).min(s.len());
    }
    let mut acc = 0.0;
    for (i, ch) in s.char_indices() {
        let w = style.advance_em(ch) * style.font_size();
        if x < acc + w / 2.0 {
            return i;
        }
        acc += w;
    }
    s.len()
}

// ---------- 弹出菜单 ----------

/// 菜单里的一项。
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem<A: Clone> {
    pub label: String,
    pub icon: Option<Icon>,
    /// 画在尾部的可选键盘加速键。
    pub shortcut: Option<String>,
    pub action: A,
    /// 危险操作（删除）用 danger 色。
    pub danger: bool,
    /// 项之上画一条分隔线。
    pub separator_before: bool,
    pub disabled: bool,
    /// 次要的非交互文案，比如「按 Esc 关闭」的提示。
    pub hint: bool,
}

impl<A: Clone> MenuItem<A> {
    pub fn new(label: impl Into<String>, action: A) -> Self {
        MenuItem {
            label: label.into(),
            icon: None,
            shortcut: None,
            action,
            danger: false,
            separator_before: false,
            disabled: false,
            hint: false,
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn separated(mut self) -> Self {
        self.separator_before = true;
        self
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.disabled = v;
        self
    }
    pub fn shortcut(mut self, value: impl Into<String>) -> Self {
        self.shortcut = Some(value.into());
        self
    }
    pub fn hint(mut self) -> Self {
        self.disabled = true;
        self.hint = true;
        self
    }
}

/// Radix ContextMenu.Content 的尺寸：`min-w-[200px] p-1 rounded-lg`，
/// 项 `px-3 py-2 text-sm`（20 行高 + 16 = 36），分隔线 `my-1 h-px`。
pub const MENU_MIN_WIDTH: f32 = 200.0;
pub const MENU_PADDING: f32 = 4.0;
pub const MENU_ITEM_HEIGHT: f32 = 36.0;
pub const MENU_SEPARATOR_HEIGHT: f32 = 9.0;
pub const MENU_RADIUS: f32 = 8.0;
const MENU_SEARCH_HEIGHT: f32 = 44.0;
const MENU_MAX_LIST_HEIGHT: f32 = MENU_ITEM_HEIGHT * 10.0;

/// 打开着的弹出菜单。位置在打开时定死（右键点哪儿就在哪儿），
/// 超出视口时向内翻折——Radix 的 `collisionPadding` 行为。
#[derive(Debug, Clone)]
pub struct Menu<A: Clone> {
    pub items: Vec<MenuItem<A>>,
    pub rect: Rect,
    pub hover: Option<usize>,
    /// 子菜单保持挂在触发行上。放在同一浮层里，子菜单打开时
    /// 父菜单才依然可见。
    pub submenu: Option<Box<Menu<A>>>,
    scroll: f32,
    scrollbar: super::overlay_scrollbar::Interaction<()>,
    /// 长列表选择器（如代码语言）用的可选「输入即筛选」字段。
    pub search: Option<TextField>,
}

fn menu_min_width() -> f32 {
    super::settings_values::number("menu.minWidth", MENU_MIN_WIDTH)
}
fn menu_padding() -> f32 {
    super::settings_values::number("menu.padding", MENU_PADDING)
}
fn menu_item_height() -> f32 {
    super::settings_values::number("menu.itemHeight", MENU_ITEM_HEIGHT)
        .max(TextStyle::Body.line_height() + 8.0)
}
fn menu_separator_height() -> f32 {
    super::settings_values::number("menu.separatorHeight", MENU_SEPARATOR_HEIGHT)
}
fn menu_radius() -> f32 {
    super::settings_values::number("menu.radius", MENU_RADIUS)
}

impl<A: Clone> Menu<A> {
    pub fn open_at(items: Vec<MenuItem<A>>, x: f32, y: f32, viewport: Rect) -> Self {
        Self::open(items, x, y, viewport, None)
    }

    fn open(
        items: Vec<MenuItem<A>>,
        x: f32,
        y: f32,
        viewport: Rect,
        search: Option<TextField>,
    ) -> Self {
        let width = items
            .iter()
            .map(|i| {
                text::measure(&i.label, TextStyle::Body)
                    + 12.0 * 2.0
                    + menu_padding() * 2.0
                    + 12.0 // 预留滚动条的区域，不要遮住标签文字。
                    + if i.icon.is_some() { 24.0 } else { 0.0 }
                    + i.shortcut
                        .as_deref()
                        .map(|shortcut| text::measure(shortcut, TextStyle::Caption) + 28.0)
                        .unwrap_or(0.0)
            })
            .fold(menu_min_width(), f32::max)
            .min(viewport.width().max(0.0));
        let height = menu_padding() * 2.0
            + items
                .iter()
                .map(|i| {
                    menu_item_height()
                        + if i.separator_before {
                            menu_separator_height()
                        } else {
                            0.0
                        }
                })
                .sum::<f32>();
        let header = if search.is_some() {
            MENU_SEARCH_HEIGHT
        } else {
            0.0
        };
        let height = (height.min(
            menu_item_height() * super::settings_values::number("menu.visibleRows", 10.0).floor()
                + menu_padding() * 2.0,
        ) + header)
            .min((viewport.height() - 16.0).max(0.0));
        let left = x.clamp(viewport.left, (viewport.right - width).max(viewport.left));
        let top = y.clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
        Menu {
            items,
            rect: Rect::from_size(left, top, width, height),
            hover: None,
            submenu: None,
            scroll: 0.0,
            scrollbar: Default::default(),
            search,
        }
    }

    /// 让下拉框贴着它的控件。优先放控件下方；下方空间不足时翻到上方，
    /// 超长列表可滚动。
    pub fn open_anchored(items: Vec<MenuItem<A>>, anchor: Rect, viewport: Rect) -> Self {
        Self::open_anchored_with_search(items, anchor, viewport, None)
    }

    pub fn open_searchable_anchored(
        items: Vec<MenuItem<A>>,
        anchor: Rect,
        viewport: Rect,
        placeholder: &str,
    ) -> Self {
        Self::open_anchored_with_search(items, anchor, viewport, Some(TextField::new(placeholder)))
    }

    fn open_anchored_with_search(
        items: Vec<MenuItem<A>>,
        anchor: Rect,
        viewport: Rect,
        search: Option<TextField>,
    ) -> Self {
        let gap = 4.0;
        let pad_x = 8.0_f32.min(viewport.width().max(0.0) / 2.0);
        let pad_y = 8.0_f32.min(viewport.height().max(0.0) / 2.0);
        let bounds = Rect::new(
            viewport.left + pad_x,
            viewport.top + pad_y,
            viewport.right - pad_x,
            viewport.bottom - pad_y,
        );
        let mut menu = Self::open(items, anchor.left, anchor.bottom + gap, viewport, search);
        let below = (bounds.bottom - anchor.bottom - gap).max(0.0);
        let above = (anchor.top - gap - bounds.top).max(0.0);
        let opens_above = menu.rect.height() > below && above > below;
        let height = menu
            .rect
            .height()
            .min(if opens_above { above } else { below })
            .min(bounds.height().max(0.0));
        let width = menu.rect.width().min(bounds.width().max(0.0));
        let left = anchor
            .left
            .clamp(bounds.left, (bounds.right - width).max(bounds.left));
        let top = if opens_above {
            anchor.top - gap - height
        } else {
            anchor.bottom + gap
        };
        menu.rect = Rect::from_size(
            left,
            top.clamp(bounds.top, (bounds.bottom - height).max(bounds.top)),
            width,
            height,
        );
        menu
    }

    pub fn open_searchable(
        items: Vec<MenuItem<A>>,
        x: f32,
        y: f32,
        viewport: Rect,
        placeholder: &str,
    ) -> Self {
        Self::open(items, x, y, viewport, Some(TextField::new(placeholder)))
    }

    pub fn search_rect(&self) -> Option<Rect> {
        self.search.as_ref().map(|_| {
            Rect::new(
                self.rect.left + 8.0,
                self.rect.top + 8.0,
                self.rect.right - 8.0,
                self.rect.top + 42.0,
            )
            .intersect(&self.rect)
        })
    }

    fn list_rect(&self) -> Rect {
        let header = if self.search.is_some() {
            MENU_SEARCH_HEIGHT
        } else {
            0.0
        };
        let top = (self.rect.top + menu_padding() + header).min(self.rect.bottom);
        Rect::new(
            (self.rect.left + menu_padding()).min(self.rect.right),
            top,
            (self.rect.right - menu_padding()).max(self.rect.left),
            (self.rect.bottom - menu_padding()).max(top),
        )
    }

    fn max_scroll(&self) -> f32 {
        let height = self
            .visible_indices()
            .into_iter()
            .map(|i| {
                menu_item_height()
                    + if self.items[i].separator_before {
                        menu_separator_height()
                    } else {
                        0.0
                    }
            })
            .sum::<f32>();
        (height - self.list_rect().height()).max(0.0)
    }

    pub fn constrain_to_viewport(&mut self, viewport: Rect) {
        let width = self.rect.width().min(viewport.width().max(0.0));
        let height = self.rect.height().min(viewport.height().max(0.0));
        let left = self
            .rect
            .left
            .clamp(viewport.left, (viewport.right - width).max(viewport.left));
        let top = self
            .rect
            .top
            .clamp(viewport.top, (viewport.bottom - height).max(viewport.top));
        self.rect = Rect::from_size(left, top, width, height);
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        if let Some(submenu) = self.submenu.as_deref_mut() {
            submenu.constrain_to_viewport(viewport);
        }
    }

    pub fn search_mut(&mut self) -> Option<&mut TextField> {
        if let Some(submenu) = self.submenu.as_deref_mut() {
            return submenu.search_mut();
        }
        self.search.as_mut()
    }

    pub fn search_hit(&self, x: f32, y: f32) -> bool {
        self.submenu
            .as_ref()
            .is_some_and(|child| child.search_hit(x, y))
            || self.search_rect().is_some_and(|rect| rect.contains(x, y))
    }

    pub fn search_changed(&mut self) {
        if let Some(submenu) = self.submenu.as_deref_mut() {
            submenu.search_changed();
        } else {
            self.scroll = 0.0;
            self.hover = None;
            self.scrollbar.end();
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        let query = self
            .search
            .as_ref()
            .map(|field| field.text().trim().to_lowercase())
            .unwrap_or_default();
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (query.is_empty() || item.label.to_lowercase().contains(&query)).then_some(index)
            })
            .collect()
    }

    /// 每一项的矩形，按顺序。分隔线占的高度在前一项与该项之间。
    fn item_rects(&self) -> Vec<(usize, Rect)> {
        let mut y = self.list_rect().top - self.scroll.min(self.max_scroll());
        let mut out = Vec::with_capacity(self.items.len());
        for index in self.visible_indices() {
            let item = &self.items[index];
            if item.separator_before {
                y += menu_separator_height();
            }
            out.push((
                index,
                Rect::new(
                    self.rect.left + menu_padding(),
                    y,
                    self.rect.right - menu_padding(),
                    y + menu_item_height(),
                ),
            ));
            y += menu_item_height();
        }
        out
    }

    pub fn item_rect(&self, index: usize) -> Option<Rect> {
        self.item_rects()
            .into_iter()
            .find_map(|(item, rect)| (item == index).then_some(rect))
    }

    /// 返回指针下方最深层的菜单项。深度 0 是当前菜单。
    pub fn hit_path(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if let Some((depth, index)) = self
            .submenu
            .as_ref()
            .and_then(|submenu| submenu.hit_path(x, y))
        {
            return Some((depth + 1, index));
        }
        self.hit(x, y).map(|index| (0, index))
    }

    fn contains_tree(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y)
            || self
                .submenu
                .as_ref()
                .is_some_and(|submenu| submenu.contains_tree(x, y))
    }

    /// 在本菜单树的某个条目上打开子菜单。子菜单优先靠右边缘，
    /// 会与视口相撞时翻到左边。
    pub fn open_submenu_at(
        &mut self,
        depth: usize,
        index: usize,
        items: Vec<MenuItem<A>>,
        viewport: Rect,
    ) -> bool {
        if depth > 0 {
            return self
                .submenu
                .as_deref_mut()
                .is_some_and(|submenu| submenu.open_submenu_at(depth - 1, index, items, viewport));
        }
        let Some(anchor) = self.item_rect(index) else {
            return false;
        };
        let gap = 4.0;
        let probe = Self::open_at(items.clone(), anchor.right + gap, anchor.top, viewport);
        let x = if probe.rect.right > viewport.right - 8.0 {
            (anchor.left - gap - probe.rect.width()).max(viewport.left + 8.0)
        } else {
            anchor.right + gap
        };
        self.submenu = Some(Box::new(Self::open_at(items, x, anchor.top, viewport)));
        true
    }

    pub fn open_searchable_submenu_at(
        &mut self,
        depth: usize,
        index: usize,
        items: Vec<MenuItem<A>>,
        viewport: Rect,
        placeholder: &str,
    ) -> bool {
        if depth > 0 {
            return self.submenu.as_deref_mut().is_some_and(|submenu| {
                submenu.open_searchable_submenu_at(depth - 1, index, items, viewport, placeholder)
            });
        }
        let Some(anchor) = self.item_rect(index) else {
            return false;
        };
        let gap = 4.0;
        let probe = Self::open_searchable(
            items.clone(),
            anchor.right + gap,
            anchor.top,
            viewport,
            placeholder,
        );
        let x = if probe.rect.right > viewport.right - 8.0 {
            (anchor.left - gap - probe.rect.width()).max(viewport.left + 8.0)
        } else {
            anchor.right + gap
        };
        self.submenu = Some(Box::new(Self::open_searchable(
            items,
            x,
            anchor.top,
            viewport,
            placeholder,
        )));
        true
    }

    /// 点/悬停在哪一项上。禁用项不响应。
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if !self.list_rect().contains(x, y)
            || self
                .scrollbar_bar()
                .is_some_and(|bar| bar.hotzone.contains(x, y))
        {
            return None;
        }
        self.item_rects()
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|(i, _)| *i)
            .filter(|i| !self.items[*i].disabled)
    }

    /// 更新悬停。返回是否变了（要不要重画）。
    pub fn set_hover(&mut self, x: f32, y: f32) -> bool {
        if let Some(submenu) = self.submenu.as_deref_mut() {
            if submenu.contains_tree(x, y) {
                return submenu.set_hover(x, y);
            }
        }
        let next = self.hit(x, y);
        let changed = next != self.hover;
        self.hover = next;
        changed
    }
    pub fn scroll_by(&mut self, delta: f32) {
        self.scroll = (self.scroll - delta).clamp(0.0, self.max_scroll());
        self.hover = None;
        self.submenu = None;
    }

    /// 把滚轮输入路由给指针真正所在的菜单，包括子菜单。
    pub fn scroll_at(&mut self, x: f32, y: f32, delta: f32) -> bool {
        if self
            .submenu
            .as_deref_mut()
            .is_some_and(|child| child.scroll_at(x, y, delta))
        {
            return true;
        }
        if !self.rect.contains(x, y) {
            return false;
        }
        self.scroll_by(delta);
        true
    }

    fn scrollbar_bar(&self) -> Option<super::overlay_scrollbar::Bar> {
        super::overlay_scrollbar::Bar::new(
            self.list_rect(),
            super::overlay_scrollbar::Axis::Vertical,
            self.max_scroll(),
            self.scroll,
            false,
        )
    }

    pub fn begin_scrollbar_drag(&mut self, x: f32, y: f32) -> bool {
        if self
            .submenu
            .as_deref_mut()
            .is_some_and(|child| child.begin_scrollbar_drag(x, y))
        {
            return true;
        }
        let Some(bar) = self.scrollbar_bar() else {
            return false;
        };
        let Some(((), offset)) = self.scrollbar.begin(&[((), bar)], x, y) else {
            return false;
        };
        self.scroll = offset;
        self.hover = None;
        self.submenu = None;
        true
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        if let Some(child) = self.submenu.as_deref_mut() {
            if child.is_scrollbar_dragging() || child.contains_tree(x, y) {
                return child.pointer_move(x, y);
            }
        }
        let bars = self
            .scrollbar_bar()
            .map(|bar| vec![((), bar)])
            .unwrap_or_default();
        if let Some(((), offset)) = self.scrollbar.drag_to(&bars, x, y) {
            self.scroll = offset;
            return true;
        }
        self.scrollbar.pointer(&bars, x, y) | self.set_hover(x, y)
    }

    pub fn end_scrollbar_drag(&mut self) -> bool {
        self.scrollbar.end()
            | self
                .submenu
                .as_deref_mut()
                .is_some_and(Self::end_scrollbar_drag)
    }

    pub fn is_scrollbar_dragging(&self) -> bool {
        self.scrollbar.dragging()
            || self
                .submenu
                .as_ref()
                .is_some_and(|child| child.is_scrollbar_dragging())
    }

    pub fn move_selection(&mut self, delta: i32) {
        if let Some(submenu) = self.submenu.as_deref_mut() {
            submenu.move_selection(delta);
            return;
        }
        let enabled = self
            .visible_indices()
            .into_iter()
            .filter(|i| !self.items[*i].disabled)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return;
        }
        let next = match self
            .hover
            .and_then(|i| enabled.iter().position(|e| *e == i))
        {
            Some(i) => (i as i32 + delta).rem_euclid(enabled.len() as i32) as usize,
            None => {
                if delta > 0 {
                    0
                } else {
                    enabled.len() - 1
                }
            }
        };
        self.hover = Some(enabled[next]);
        let r = self
            .item_rects()
            .into_iter()
            .find(|(i, _)| *i == enabled[next])
            .unwrap()
            .1;
        let top = self.list_rect().top;
        if r.top < top {
            self.scroll -= top - r.top;
        } else if r.bottom > self.rect.bottom - menu_padding() {
            self.scroll += r.bottom - (self.rect.bottom - menu_padding());
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    pub fn paint(&self, list: &mut DrawList, p: &Palette) {
        // 阴影：两层半透明感的偏移矩形。没有真阴影，用边框色兑一层暗
        let shadow = Rect::new(
            self.rect.left + 1.0,
            self.rect.top + 2.0,
            self.rect.right + 1.0,
            self.rect.bottom + 2.0,
        );
        list.rounded_rect(
            shadow,
            menu_radius(),
            theme::mix(p.foreground, p.background, 0.06),
        );
        list.rounded_rect(self.rect, menu_radius(), p.surface);
        list.rounded_border(self.rect, menu_radius(), p.border);
        list.push_clip(self.rect);

        if let (Some(field), Some(search)) = (&self.search, self.search_rect()) {
            let mut field = field.clone();
            field.paint(list, search, true, p, FieldLook::dialog(p));
        }

        let body = self.list_rect();
        list.push_clip(body);
        let bar = self.scrollbar_bar();
        let rects = self.item_rects();
        for (i, r) in &rects {
            if r.bottom <= body.top || r.top >= body.bottom {
                continue;
            }
            let item = &self.items[*i];
            if item.separator_before {
                list.hline(
                    r.left,
                    r.right,
                    r.top - menu_separator_height() / 2.0,
                    p.border,
                );
            }
            if self.hover == Some(*i) && !item.disabled {
                list.rounded_rect(*r, 4.0, p.background);
            }
            let color = if item.disabled || item.hint {
                p.muted
            } else if item.danger {
                p.danger
            } else {
                p.foreground
            };
            let mut text_left = r.left + 12.0;
            if let Some(icon) = item.icon {
                list.icon_centered(
                    Rect::new(text_left, r.top, text_left + 16.0, r.bottom),
                    icon,
                    16.0,
                    color,
                );
                text_left += 24.0;
            }
            let text_right = r.right - 12.0 - if bar.is_some() { 12.0 } else { 0.0 };
            let label_right = text_right
                - item
                    .shortcut
                    .as_deref()
                    .map(|s| text::measure(s, TextStyle::Caption) + 20.0)
                    .unwrap_or(0.0);
            let label_style = if item.hint {
                TextStyle::Caption
            } else {
                TextStyle::Body
            };
            list.text_aligned(
                Rect::new(text_left, r.top, label_right.max(text_left), r.bottom),
                text::ellipsize(&item.label, label_style, (label_right - text_left).max(0.0)),
                label_style,
                color,
                Align::Leading,
            );
            if let Some(shortcut) = &item.shortcut {
                list.text_aligned(
                    Rect::new(text_left, r.top, text_right.max(text_left), r.bottom),
                    shortcut.clone(),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
            }
        }
        list.pop_clip();
        if let Some(bar) = bar {
            list.rounded_rect_alpha(bar.thumb, 3.0, p.foreground, 0.2);
            self.scrollbar.paint(list, &[((), bar)], p);
        }
        list.pop_clip();
        if let Some(submenu) = &self.submenu {
            submenu.paint(list, p);
        }
    }
}

// ---------- 模态对话框 ----------

/// 对话框按钮的样子。对应 dialogs.tsx 里的三种按钮类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// 独占一行、放在操作按钮上方的小型可选偏好。
    Checkbox(bool),
    /// `text-foreground hover:bg-background`。
    Ghost,
    /// `bg-accent text-white` / `bg-[#3C5A78] text-white`。
    Primary,
    /// `border border-red-500/40 text-red-600`。
    Danger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DialogButton<A: Clone> {
    pub label: String,
    pub kind: ButtonKind,
    pub action: A,
}

/// `BaseDialog`：`w-[400px] p-6 rounded-lg`，标题 `text-lg font-semibold mb-2`，
/// 描述 `text-sm text-muted mb-4`，右上角关闭按钮。
///
/// 内容区只有两种：一个输入框（新建文件/文件夹），或一段带警告图标的说明
/// （删除确认）。按钮靠右 `gap-2`，`px-4 py-2 text-sm` → 36 高。
#[derive(Debug, Clone)]
pub struct Dialog<A: Clone> {
    pub title: String,
    pub description: String,
    pub field: Option<TextField>,
    pub error: String,
    /// 普通警告说明，或可滚动的 Markdown 更新日志。
    pub note: Option<DialogNote>,
    pub buttons: Vec<DialogButton<A>>,
    /// 点空白/×/Esc 时的动作。
    pub dismiss: A,
    pub hover: Option<usize>,
}

pub const DIALOG_WIDTH: f32 = 400.0;

#[derive(Debug, Clone)]
pub enum DialogNote {
    Plain(String),
    Markdown { source: String, scroll: f32 },
}

impl From<String> for DialogNote {
    fn from(source: String) -> Self {
        Self::Plain(source)
    }
}

impl From<&str> for DialogNote {
    fn from(source: &str) -> Self {
        Self::Plain(source.into())
    }
}

impl std::ops::Deref for DialogNote {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Self::Plain(source) | Self::Markdown { source, .. } => source,
        }
    }
}
const DIALOG_PADDING: f32 = 24.0;
const BUTTON_HEIGHT: f32 = 36.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogHit {
    Close,
    Field,
    Button(usize),
    /// 对话框内部但不在任何控件上。
    Inside,
    /// 遮罩层。
    Outside,
}

impl<A: Clone> Dialog<A> {
    fn markdown_layout(&self) -> Option<std::rc::Rc<super::ai_markdown::Layout>> {
        match self.note.as_ref()? {
            DialogNote::Markdown { source, .. } => Some(super::ai_markdown::layout(
                source,
                DIALOG_WIDTH - DIALOG_PADDING * 2.0 - 32.0,
                true,
            )),
            _ => None,
        }
    }

    fn note_height(&self, viewport: Rect) -> f32 {
        if let Some(layout) = self.markdown_layout() {
            (layout.height + 24.0)
                .min(480.0)
                .min((viewport.height() - 220.0).max(48.0))
        } else {
            self.note_lines().len().max(1) as f32 * 20.0 + 24.0
        }
    }

    pub fn scroll_note(&mut self, viewport: Rect, x: f32, y: f32, delta: f32) {
        let Some(layout) = self.markdown_layout() else {
            return;
        };
        let Some(rect) = self.parts(viewport).2 else {
            return;
        };
        if rect.contains(x, y) {
            if let Some(DialogNote::Markdown { scroll, .. }) = self.note.as_mut() {
                *scroll =
                    (*scroll - delta).clamp(0.0, (layout.height - rect.height() + 24.0).max(0.0));
            }
        }
    }

    pub fn note_link_at(&self, viewport: Rect, x: f32, y: f32) -> Option<String> {
        let layout = self.markdown_layout()?;
        let rect = self.parts(viewport).2?;
        let clip = Rect::new(
            rect.left + 12.0,
            rect.top + 12.0,
            rect.right - 20.0,
            rect.bottom - 12.0,
        );
        if !clip.contains(x, y) {
            return None;
        }
        let Some(DialogNote::Markdown { scroll, .. }) = &self.note else {
            return None;
        };
        let scroll = scroll.clamp(0.0, (layout.height - clip.height()).max(0.0));
        layout
            .link_at(x - clip.left, y - clip.top + scroll, None)
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
    }

    pub fn is_math_editor(&self) -> bool {
        self.field.as_ref().is_some_and(|field| field.formula)
    }

    fn note_lines(&self) -> Vec<String> {
        self.note
            .as_deref()
            .map(|note| {
                note.split('\n')
                    .flat_map(|line| {
                        text::wrap_runs(
                            &[text::Run::plain(line)],
                            TextStyle::Label,
                            DIALOG_WIDTH - DIALOG_PADDING * 2.0 - 56.0,
                        )
                        .into_iter()
                        .map(|runs| runs.into_iter().map(|r| r.text).collect::<String>())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 内容高度包含标题、描述、可选输入框及错误、说明框和按钮，并计入各段间距。
    pub fn rect(&self, viewport: Rect) -> Rect {
        if self.is_math_editor() {
            return super::math_editor::layout(viewport).dialog;
        }
        let mut h = DIALOG_PADDING * 2.0 + TextStyle::Large.line_height().max(28.0) + 8.0;
        if !self.description.is_empty() {
            h += 20.0 + 16.0;
        }
        if self.field.is_some() {
            h += 38.0 + 16.0;
            if !self.error.is_empty() {
                h += 18.0;
            }
        }
        if self.note.is_some() {
            h += self.note_height(viewport) + 20.0;
        }
        h += BUTTON_HEIGHT;
        h += self
            .buttons
            .iter()
            .filter(|b| matches!(b.kind, ButtonKind::Checkbox(_)))
            .count() as f32
            * 28.0;
        let left = ((viewport.left + viewport.right) / 2.0 - DIALOG_WIDTH / 2.0).round();
        let top = ((viewport.top + viewport.bottom) / 2.0 - h / 2.0).round();
        Rect::from_size(left, top, DIALOG_WIDTH, h)
    }

    pub(crate) fn parts(
        &self,
        viewport: Rect,
    ) -> (Rect, Option<Rect>, Option<Rect>, Vec<Rect>, Rect) {
        if self.is_math_editor() {
            let layout = super::math_editor::layout(viewport);
            let mut x = layout.dialog.right - DIALOG_PADDING;
            let mut buttons = self
                .buttons
                .iter()
                .rev()
                .map(|button| {
                    let width = text::measure(&button.label, TextStyle::Label) + 32.0;
                    let rect = Rect::from_size(x - width, layout.footer, width, BUTTON_HEIGHT);
                    x -= width + 8.0;
                    rect
                })
                .collect::<Vec<_>>();
            buttons.reverse();
            return (
                layout.title,
                Some(layout.editor),
                None,
                buttons,
                layout.close,
            );
        }
        let r = self.rect(viewport);
        let mut y = r.top + DIALOG_PADDING;
        let title = Rect::new(
            r.left + DIALOG_PADDING,
            y,
            r.right - DIALOG_PADDING - 32.0,
            y + TextStyle::Large.line_height().max(28.0),
        );
        y += title.height() + 8.0;
        if !self.description.is_empty() {
            y += 20.0 + 16.0;
        }
        let field = self.field.as_ref().map(|_| {
            let f = Rect::new(
                r.left + DIALOG_PADDING,
                y,
                r.right - DIALOG_PADDING,
                y + 38.0,
            );
            y += 38.0 + 16.0;
            if !self.error.is_empty() {
                y += 18.0;
            }
            f
        });
        let note = self.note.as_ref().map(|_| {
            let height = self.note_height(viewport);
            let n = Rect::new(
                r.left + DIALOG_PADDING,
                y,
                r.right - DIALOG_PADDING,
                y + height,
            );
            y += height + 20.0;
            n
        });
        // 按钮从右往左排
        let mut buttons = Vec::new();
        let checkboxes = self
            .buttons
            .iter()
            .filter(|b| matches!(b.kind, ButtonKind::Checkbox(_)))
            .count();
        let mut checkbox_y = y + checkboxes as f32 * 28.0;
        y = checkbox_y;
        let mut x = r.right - DIALOG_PADDING;
        for b in self.buttons.iter().rev() {
            if matches!(b.kind, ButtonKind::Checkbox(_)) {
                checkbox_y -= 28.0;
                buttons.push(Rect::new(
                    r.left + DIALOG_PADDING,
                    checkbox_y,
                    r.right - DIALOG_PADDING,
                    checkbox_y + 24.0,
                ));
                continue;
            }
            let w = text::measure(&b.label, TextStyle::Label) + 32.0;
            buttons.push(Rect::new(x - w, y, x, y + BUTTON_HEIGHT));
            x -= w + 8.0;
        }
        buttons.reverse();
        let close = Rect::new(
            r.right - 16.0 - 24.0,
            r.top + 16.0,
            r.right - 16.0,
            r.top + 16.0 + 24.0,
        );
        (title, field, note, buttons, close)
    }

    pub fn hit(&self, viewport: Rect, x: f32, y: f32) -> DialogHit {
        let r = self.rect(viewport);
        if !r.contains(x, y) {
            return DialogHit::Outside;
        }
        let (_, field, _, buttons, close) = self.parts(viewport);
        if close.contains(x, y) {
            return DialogHit::Close;
        }
        if field.map(|f| f.contains(x, y)).unwrap_or(false) {
            return DialogHit::Field;
        }
        if let Some(i) = buttons.iter().position(|b| b.contains(x, y)) {
            return DialogHit::Button(i);
        }
        DialogHit::Inside
    }

    pub fn set_hover(&mut self, viewport: Rect, x: f32, y: f32) -> bool {
        let next = match self.hit(viewport, x, y) {
            DialogHit::Button(i) => Some(i),
            _ => None,
        };
        let changed = next != self.hover;
        self.hover = next;
        changed
    }

    /// 输入框文字起点（IME 定位）。
    pub fn field_text_left(&self, viewport: Rect) -> Option<f32> {
        self.parts(viewport).1.map(|f| f.left + 12.0)
    }

    pub fn field_rect(&self, viewport: Rect) -> Option<Rect> {
        self.parts(viewport).1
    }

    pub fn paint(&mut self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        if viewport.is_empty() {
            return;
        }
        // 遮罩 bg-black/50
        list.rect_alpha(viewport, 0x000000, 0.5);
        let r = self.rect(viewport);
        list.rounded_rect(r, 8.0, p.surface);
        list.rounded_border(r, 8.0, p.border);
        let (title, field, note, buttons, close) = self.parts(viewport);
        list.text(title, self.title.clone(), TextStyle::Large, p.foreground);
        list.icon_centered(close, Icon::X, 16.0, p.muted);
        if !self.description.is_empty() {
            let d = Rect::new(
                title.left,
                title.bottom + 8.0,
                r.right - DIALOG_PADDING,
                title.bottom + 8.0 + 20.0,
            );
            list.text(
                d,
                text::ellipsize(&self.description, TextStyle::Label, d.width()),
                TextStyle::Label,
                p.muted,
            );
        }
        let math_editor = self.is_math_editor();
        if let (Some(f), Some(fr)) = (self.field.as_mut(), field) {
            if math_editor {
                super::math_editor::paint(list, viewport, f, p);
            } else {
                f.style = TextStyle::Label;
                f.paint(list, fr, true, p, FieldLook::dialog(p));
            }
            if !self.error.is_empty() {
                list.text(
                    Rect::new(fr.left, fr.bottom + 4.0, fr.right, fr.bottom + 4.0 + 16.0),
                    self.error.clone(),
                    TextStyle::Caption,
                    p.danger,
                );
            }
        }
        if let (Some(_), Some(nr)) = (&self.note, note) {
            list.rounded_rect(nr, 8.0, p.background);
            list.rounded_border(nr, 8.0, p.border);
            if let (Some(layout), Some(DialogNote::Markdown { scroll, .. })) =
                (self.markdown_layout(), &self.note)
            {
                let clip = Rect::new(
                    nr.left + 12.0,
                    nr.top + 12.0,
                    nr.right - 20.0,
                    nr.bottom - 12.0,
                );
                let max_scroll = (layout.height - clip.height()).max(0.0);
                let scroll = scroll.clamp(0.0, max_scroll);
                layout.paint_scrolled(
                    list,
                    (clip.left, clip.top - scroll),
                    clip,
                    p,
                    None,
                    false,
                    None,
                    None,
                );
                if max_scroll > 0.0 {
                    let thumb_height = (clip.height() * clip.height() / layout.height)
                        .max(16.0)
                        .min(clip.height());
                    let top = clip.top + (clip.height() - thumb_height) * scroll / max_scroll;
                    list.rounded_rect(
                        Rect::from_size(nr.right - 8.0, top, 3.0, thumb_height),
                        1.5,
                        p.muted,
                    );
                }
            } else {
                // AlertTriangle 图标使用 amber-500 色。
                list.icon_centered(
                    Rect::new(nr.left + 12.0, nr.top + 12.0, nr.left + 32.0, nr.top + 32.0),
                    Icon::ALERT_TRIANGLE,
                    20.0,
                    0xF59E0B,
                );
                for (i, line) in self.note_lines().into_iter().enumerate() {
                    let top = nr.top + 12.0 + i as f32 * 20.0;
                    list.text(
                        Rect::new(nr.left + 44.0, top, nr.right - 12.0, top + 20.0),
                        line,
                        TextStyle::Label,
                        p.muted,
                    );
                }
            }
        }
        for (i, (b, br)) in self.buttons.iter().zip(&buttons).enumerate() {
            let hover = self.hover == Some(i);
            match b.kind {
                ButtonKind::Checkbox(checked) => {
                    let box_rect = Rect::from_size(br.left, br.top + 4.0, 14.0, 14.0);
                    list.rounded_border(box_rect, 3.0, if checked { p.accent } else { p.border });
                    if checked {
                        list.icon_centered(box_rect, Icon::CHECK, 12.0, p.accent);
                    }
                    list.text(
                        Rect::new(br.left + 22.0, br.top, br.right, br.bottom),
                        &b.label,
                        TextStyle::Caption,
                        p.muted,
                    );
                }
                ButtonKind::Ghost => {
                    if hover {
                        list.rounded_rect(*br, 8.0, p.background);
                    }
                    list.text_aligned(
                        *br,
                        b.label.clone(),
                        TextStyle::Label,
                        p.foreground,
                        Align::Center,
                    );
                }
                ButtonKind::Primary => {
                    list.glass_button(*br, 8.0, p, hover);
                    list.text_aligned(
                        *br,
                        b.label.clone(),
                        TextStyle::Label,
                        p.button_foreground(),
                        Align::Center,
                    );
                }
                ButtonKind::Danger => {
                    if hover {
                        list.rounded_rect(*br, 8.0, theme::mix(p.danger, p.surface, 0.10));
                    }
                    list.rounded_border(*br, 8.0, theme::mix(p.danger, p.surface, 0.40));
                    list.text_aligned(
                        *br,
                        b.label.clone(),
                        TextStyle::Label,
                        p.danger,
                        Align::Center,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_wheel_keeps_viewport_until_the_caret_or_text_changes() {
        let mut field = TextField::new("").with_text(&"中文参数\n".repeat(80));
        let area = Rect::from_size(0., 0., 320., 200.);
        let mut list = DrawList::new();
        let p = theme::tokens().palette(false);
        field.paint_multiline(&mut list, area, true, p);
        let bottom = field.scroll_y;
        assert!(bottom > 0.);
        field.scroll_multiline(area, 160.);
        let scrolled = field.scroll_y;
        assert!(scrolled < bottom);
        field.paint_multiline(&mut list, area, true, p);
        assert_eq!(field.scroll_y, scrolled);
        field.char('新');
        field.paint_multiline(&mut list, area, true, p);
        assert!(field.scroll_y > scrolled);
    }

    #[test]
    fn singleline_arrows_move_to_edges_and_extend_utf8_selection() {
        let mut field = TextField::new("").with_text("文件😀名称");
        field.buffer.set_cursor("文件".len(), false);
        assert_eq!(field.key(40, true, false), FieldKey::Edited);
        assert_eq!(field.buffer.cursor(), field.text().len());
        assert_eq!(field.buffer.selected_text(), "😀名称");
        assert_eq!(field.key(38, false, false), FieldKey::Edited);
        assert_eq!(field.buffer.cursor(), 0);
        assert!(!field.buffer.has_selection());
    }

    #[test]
    fn multiline_arrows_keep_column_and_reach_document_boundaries() {
        let source = "abcdefgh\nx\nabcdefgh";
        let mut field = TextField::new("").with_text(source);
        let area = Rect::from_size(0.0, 0.0, 600.0, 220.0);
        field.buffer.set_cursor(6, false);
        field.multiline_key(area, 40, false, false);
        assert_eq!(field.buffer.cursor(), 10);
        field.multiline_key(area, 40, false, false);
        assert_eq!(
            field.buffer.cursor(),
            17,
            "keep the original column after a short line"
        );
        field.multiline_key(area, 40, true, false);
        assert_eq!(field.buffer.cursor(), source.len());
        assert_eq!(field.buffer.selected_text(), "gh");
        field.multiline_key(area, 36, false, true);
        assert_eq!(field.buffer.cursor(), 0);
        field.buffer.set_cursor(4, false);
        field.multiline_key(area, 38, false, false);
        assert_eq!(field.buffer.cursor(), 0);
    }

    #[test]
    fn click_selection_and_ime_caret_share_the_complete_shaped_line() {
        use super::super::measurement::{Backend, Cluster, Shape};
        use std::rc::Rc;
        struct Shaping;
        impl Backend for Shaping {
            fn shape(&self, value: &str, _: TextStyle, _: text::Emphasis) -> Option<Rc<Shape>> {
                (value == "Wi中").then(|| {
                    Rc::new(Shape {
                        width: 51.0,
                        clusters: vec![
                            Cluster {
                                start: 0,
                                end: 1,
                                leading: 0.0,
                                trailing: 30.0,
                                advance: 30.0,
                            },
                            Cluster {
                                start: 1,
                                end: 2,
                                leading: 30.0,
                                trailing: 33.0,
                                advance: 3.0,
                            },
                            Cluster {
                                start: 2,
                                end: 5,
                                leading: 33.0,
                                trailing: 51.0,
                                advance: 18.0,
                            },
                        ],
                    })
                })
            }
        }
        let backend: Rc<dyn Backend> = Rc::new(Shaping);
        super::super::measurement::register(backend.clone());
        let mut field = TextField::new("").with_text("Wi中");
        field.buffer.set_cursor(1, false);
        assert_eq!(field.caret_x(), 30.0);
        field.click(32.5, false);
        assert_eq!(field.buffer.cursor(), 2);
        let mut list = DrawList::new();
        let rect = Rect::from_size(100.0, 50.0, 200.0, 38.0);
        field.paint(&mut list, rect, true, &p(), FieldLook::dialog(&p()));
        let caret = list.caret_rect().unwrap();
        assert_eq!(caret.left, 100.0 + 12.0 + 33.0);
        assert_eq!(caret.height(), field.style.line_height());
        assert!(list.set_caret_visible(false));
        assert_eq!(
            list.caret_rect(),
            Some(caret),
            "IME does not jump when caret blinks"
        );
    }

    #[test]
    fn long_menu_scrolls_and_keyboard_skips_disabled_rows() {
        let items = (0..40)
            .map(|i| MenuItem::new(format!("项目 {i}"), i).disabled(i == 1))
            .collect();
        let area = Rect::new(0.0, 0.0, 600.0, 400.0);
        let mut menu = Menu::open_at(items, 10.0, 10.0, area);
        assert!(menu.rect.bottom <= area.bottom);
        menu.move_selection(1);
        assert_eq!(menu.hover, Some(0));
        menu.move_selection(1);
        assert_eq!(menu.hover, Some(2));
        for _ in 0..35 {
            menu.move_selection(1);
        }
        let i = menu.hover.unwrap();
        let (_, r) = menu
            .item_rects()
            .into_iter()
            .find(|(index, _)| *index == i)
            .unwrap();
        assert!(r.top >= menu.rect.top);
        assert!(r.bottom <= menu.rect.bottom);
        assert_eq!(menu.hit(r.left + 1.0, r.top + 1.0), Some(i));
        assert!(menu.hit(r.left, menu.rect.bottom + 1.0).is_none());
    }
    #[test]
    fn searchable_menu_filters_without_losing_original_actions() {
        let items = vec![
            MenuItem::new("javascript", 1u8),
            MenuItem::new("rust", 2u8),
            MenuItem::new("ruby", 3u8),
        ];
        let mut menu = Menu::open_searchable(items, 10.0, 10.0, VIEW, "搜索语言");
        menu.search.as_mut().unwrap().char('r');
        menu.search.as_mut().unwrap().char('u');
        menu.search.as_mut().unwrap().char('s');
        let rows = menu.item_rects();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 1);
        assert_eq!(menu.hit(rows[0].1.left + 2.0, rows[0].1.top + 2.0), Some(1));
    }

    fn many_menu_items() -> Vec<MenuItem<usize>> {
        (0..100)
            .map(|i| MenuItem::new(format!("提供商或节点变量 {i}"), i))
            .collect()
    }

    #[test]
    fn searchable_dropdown_fits_small_viewports_and_keeps_a_scrollable_list() {
        for (width, height) in [(360.0, 240.0), (800.0, 600.0), (1280.0, 900.0)] {
            let viewport = Rect::from_size(20.0, 30.0, width, height);
            let anchor = Rect::from_size(viewport.right - 80.0, viewport.bottom - 60.0, 72.0, 32.0);
            let mut menu =
                Menu::open_searchable_anchored(many_menu_items(), anchor, viewport, "搜索提供商");
            assert!(menu.rect.left >= viewport.left && menu.rect.top >= viewport.top);
            assert!(menu.rect.right <= viewport.right && menu.rect.bottom <= viewport.bottom);
            assert!(menu.rect.bottom <= anchor.top && menu.list_rect().height() > 0.0);
            assert!(
                menu.rect.height()
                    <= MENU_MAX_LIST_HEIGHT + MENU_SEARCH_HEIGHT + MENU_PADDING * 2.0
            );
            assert!(menu.scrollbar_bar().is_some());
            menu.scroll_by(-100_000.0);
            let body = menu.list_rect();
            assert_eq!(menu.hit(body.left + 20.0, body.bottom - 1.0), Some(99));
        }
        // 最小宽度不能让窄窗口或最小化窗口溢出。
        for viewport in [
            Rect::from_size(10.0, 20.0, 100.0, 80.0),
            Rect::from_size(10.0, 20.0, 0.0, 0.0),
        ] {
            let menu = Menu::open_searchable(many_menu_items(), -500.0, -500.0, viewport, "搜索");
            assert!(menu.rect.left >= viewport.left && menu.rect.top >= viewport.top);
            assert!(menu.rect.right <= viewport.right && menu.rect.bottom <= viewport.bottom);
            let mut list = DrawList::new();
            menu.paint(&mut list, &p());
            assert!(list.finish().is_ok());
        }
    }

    #[test]
    fn scrolled_rows_cannot_paint_or_receive_clicks_over_the_search_field() {
        let mut menu = Menu::open_searchable(many_menu_items(), 20.0, 20.0, VIEW, "搜索提供商");
        menu.scroll_by(-270.0);
        let search = menu.search_rect().unwrap();
        assert_eq!(menu.hit(search.left + 20.0, search.top + 10.0), None);
        let mut list = DrawList::new();
        menu.paint(&mut list, &p());
        let mut clips = vec![];
        let mut rows = 0;
        for cmd in list.cmds() {
            match cmd {
                DrawCmd::PushClip { rect } => clips.push(*rect),
                DrawCmd::PopClip => {
                    clips.pop();
                }
                DrawCmd::Text { text, .. } if text.starts_with("提供商或节点变量") => {
                    rows += 1;
                    assert_eq!(clips.last(), Some(&menu.list_rect()));
                }
                _ => {}
            }
        }
        assert!(rows > 0 && rows <= 11, "only visible rows should be drawn");
        assert!(list.finish().is_ok());
    }

    #[test]
    fn filtering_a_scrolled_menu_resets_offset_and_empty_results_cannot_select() {
        let mut menu = Menu::open_searchable(many_menu_items(), 20.0, 20.0, VIEW, "搜索");
        menu.scroll_by(-100_000.0);
        menu.search_mut().unwrap().set_text("变量 2");
        menu.search_changed();
        assert_eq!(menu.scroll, 0.0);
        menu.move_selection(1);
        assert_eq!(menu.hover, Some(2));
        let row = menu.item_rect(2).unwrap();
        assert_eq!(menu.hit(row.left + 10.0, row.top + 10.0), Some(2));
        menu.search_mut().unwrap().set_text("没有匹配项");
        menu.search_changed();
        menu.move_selection(1);
        assert!(menu.hover.is_none());
        assert_eq!(menu.max_scroll(), 0.0);
        assert!(menu.scrollbar_bar().is_none());
        assert!(menu
            .hit(menu.rect.left + 20.0, menu.list_rect().top + 10.0)
            .is_none());
    }

    #[test]
    fn submenu_wheel_and_keyboard_only_change_the_child() {
        let mut menu = Menu::open_at(vec![MenuItem::new("更多", 0usize)], 100.0, 100.0, VIEW);
        menu.open_searchable_submenu_at(0, 0, many_menu_items(), VIEW, "搜索");
        let child_rect = menu.submenu.as_ref().unwrap().rect;
        assert!(menu.scroll_at(child_rect.left + 20.0, child_rect.bottom - 10.0, -10_000.0));
        assert_eq!(menu.scroll, 0.0);
        assert!(menu.submenu.as_ref().unwrap().scroll > 0.0);
        menu.move_selection(-1);
        assert_eq!(menu.submenu.as_ref().unwrap().hover, Some(99));
        menu.search_mut().unwrap().set_text("变量 0");
        menu.search_changed();
        menu.move_selection(1);
        assert_eq!(menu.submenu.as_ref().unwrap().hover, Some(0));
        assert!(menu.search.is_none());
    }

    #[test]
    fn menu_scrollbar_drag_reaches_last_item_without_activating_a_row() {
        let mut menu = Menu::open_searchable(many_menu_items(), 20.0, 20.0, VIEW, "搜索");
        let bar = menu.scrollbar_bar().unwrap();
        let x = bar.thumb.left + 1.0;
        let y = bar.thumb.top + 1.0;
        assert_eq!(menu.hit(x, y), None);
        assert!(menu.begin_scrollbar_drag(x, y));
        assert!(menu.is_scrollbar_dragging());
        assert!(menu.pointer_move(x, VIEW.bottom + 200.0));
        assert_eq!(menu.scroll, menu.max_scroll());
        assert!(menu.end_scrollbar_drag());
        assert!(!menu.is_scrollbar_dragging());
        let row = menu.item_rect(99).unwrap();
        assert_eq!(menu.hit(row.left + 10.0, row.bottom - 1.0), Some(99));
        menu.constrain_to_viewport(Rect::from_size(0.0, 0.0, 160.0, 180.0));
        assert!(menu.rect.right <= 160.0 && menu.rect.bottom <= 180.0);
        menu.move_selection(-1);
        let last = menu.item_rect(99).unwrap();
        assert!(last.top >= menu.list_rect().top && last.bottom <= menu.list_rect().bottom);
    }

    #[test]
    fn submenu_stays_attached_to_its_parent_row_and_keeps_parent_visible() {
        let mut menu = Menu::open_at(
            vec![MenuItem::new("切换格式", 1u8), MenuItem::new("删除", 2u8)],
            100.0,
            100.0,
            VIEW,
        );
        let parent_row = menu.item_rect(0).unwrap();
        assert!(menu.open_submenu_at(0, 0, vec![MenuItem::new("加粗", 3u8)], VIEW,));
        let child = menu.submenu.as_ref().unwrap();
        assert_eq!(child.rect.left, parent_row.right + 4.0);
        assert_eq!(child.rect.top, parent_row.top);
        assert!(menu
            .rect
            .contains(parent_row.left + 1.0, parent_row.top + 1.0));
        let child_row = child.item_rect(0).unwrap();
        assert_eq!(
            menu.hit_path(child_row.left + 1.0, child_row.top + 1.0),
            Some((1, 0))
        );

        let edge_parent = Menu::open_at(vec![MenuItem::new("父项", 1u8)], 980.0, 100.0, VIEW);
        let mut edge_menu = edge_parent;
        assert!(edge_menu.open_submenu_at(0, 0, vec![MenuItem::new("子项", 2u8)], VIEW));
        assert!(edge_menu.submenu.as_ref().unwrap().rect.right <= VIEW.right);
        assert!(edge_menu.submenu.as_ref().unwrap().rect.left < edge_menu.rect.left);

        let mut searchable_parent =
            Menu::open_at(vec![MenuItem::new("图标选择", 1u8)], 100.0, 100.0, VIEW);
        assert!(searchable_parent.open_searchable_submenu_at(
            0,
            0,
            vec![MenuItem::new("Smile", 2u8)],
            VIEW,
            "搜索图标…",
        ));
        let searchable_child = searchable_parent.submenu.as_ref().unwrap();
        assert!(searchable_child.search.is_some());
        assert_eq!(
            searchable_child.rect.left,
            searchable_parent.item_rect(0).unwrap().right + 4.0
        );
    }
    use crate::ui::draw::DrawCmd;

    const VIEW: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 1200.0,
        bottom: 800.0,
    };

    #[test]
    fn markdown_note_renders_and_scrolls_without_moving_dialog_buttons() {
        let mut d: Dialog<u8> = Dialog {
            title: "发现新版本".into(),
            description: "v1.1.0".into(),
            field: None,
            error: String::new(),
            note: Some(DialogNote::Markdown {
                source: format!(
                    "## 更新日志\n\n- **桌面卡片**\n\n{}\n\n末尾内容",
                    "说明段落。\n\n".repeat(100)
                ),
                scroll: 0.0,
            }),
            buttons: vec![DialogButton {
                label: "下载并安装".into(),
                kind: ButtonKind::Primary,
                action: 1,
            }],
            dismiss: 0,
            hover: None,
        };
        let layout = d.markdown_layout().unwrap();
        let rendered = layout
            .selectable_text(None)
            .into_iter()
            .map(|run| run.text)
            .collect::<String>();
        assert!(rendered.contains("更新日志"));
        assert!(rendered.contains("桌面卡片"));
        assert!(rendered.contains("末尾内容"));
        assert!(!rendered.contains("##"));
        assert!(!rendered.contains("**"));
        let viewport = Rect::from_size(0.0, 0.0, 800.0, 600.0);
        let rect = d.rect(viewport);
        assert!(rect.top >= viewport.top && rect.bottom <= viewport.bottom);
        let (_, _, note, buttons, _) = d.parts(viewport);
        let note = note.unwrap();
        assert!(buttons[0].top >= note.bottom);
        assert!(buttons[0].bottom < viewport.bottom);
        d.scroll_note(viewport, note.left + 20.0, note.top + 20.0, -100_000.0);
        match d.note.as_ref().unwrap() {
            DialogNote::Markdown { scroll, .. } => {
                assert!((*scroll - (layout.height - note.height() + 24.0)).abs() < 0.1)
            }
            _ => panic!("expected Markdown"),
        }
        assert_eq!(d.parts(viewport).3, buttons);
        d.scroll_note(viewport, note.left + 20.0, note.top + 20.0, 100_000.0);
        assert!(matches!(
            d.note,
            Some(DialogNote::Markdown { scroll: 0.0, .. })
        ));
    }

    #[test]
    fn multiline_warning_is_drawn_completely_and_pushes_buttons_down() {
        let mut d: Dialog<u8> = Dialog {
            title: "冲突".into(),
            description: String::new(),
            field: None,
            error: String::new(),
            note: Some("第一行".into()),
            buttons: vec![],
            dismiss: 0,
            hover: None,
        };
        let before = d.rect(VIEW).height();
        d.note = Some("第一行\n用本地：覆盖磁盘；副本：保留两者。".into());
        assert_eq!(d.rect(VIEW).height(), before + 20.0);
        let mut list = DrawList::new();
        d.paint(&mut list, VIEW, theme::tokens().palette(false));
        assert!(list.cmds().iter().any(
            |c| matches!(c,DrawCmd::Text{text,..} if text=="用本地：覆盖磁盘；副本：保留两者。")
        ));
    }

    #[test]
    fn a_dialog_is_400_wide_and_centered_with_buttons_flush_right() {
        let mut d: Dialog<u8> = Dialog {
            title: "删除文件".into(),
            description: "是否将 “a.md” 移动到系统回收站？".into(),
            field: None,
            error: String::new(),
            note: Some("移动到系统回收站后仍可恢复。".into()),
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: 0,
                },
                DialogButton {
                    label: "永久删除".into(),
                    kind: ButtonKind::Danger,
                    action: 1,
                },
                DialogButton {
                    label: "移到系统回收站".into(),
                    kind: ButtonKind::Primary,
                    action: 2,
                },
            ],
            dismiss: 0,
            hover: None,
        };
        let r = d.rect(VIEW);
        assert_eq!(r.width(), 400.0);
        assert_eq!((r.left + r.right) / 2.0, 600.0);
        let (_, _, _, buttons, _) = d.parts(VIEW);
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[2].right, r.right - 24.0);
        assert!(buttons[0].right < buttons[1].left && buttons[1].right < buttons[2].left);
        assert_eq!(
            d.hit(VIEW, buttons[1].left + 2.0, buttons[1].top + 2.0),
            DialogHit::Button(1)
        );
        assert_eq!(d.hit(VIEW, 5.0, 5.0), DialogHit::Outside);

        let mut list = DrawList::new();
        d.paint(&mut list, VIEW, theme::tokens().palette(false));
        let p = theme::tokens().palette(false);
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { text, color, .. } if text == "永久删除" && *color == p.danger)));
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Icon { icon, .. } if *icon == Icon::ALERT_TRIANGLE)));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn a_dialog_with_a_field_grows_when_an_error_is_shown() {
        let mut d: Dialog<u8> = Dialog {
            title: "新建文件".into(),
            description: String::new(),
            field: Some(TextField::new("文件名（自动添加 .mc）")),
            error: String::new(),
            note: None,
            buttons: vec![DialogButton {
                label: "创建".into(),
                kind: ButtonKind::Primary,
                action: 1,
            }],
            dismiss: 0,
            hover: None,
        };
        let h = d.rect(VIEW).height();
        d.error = "文件名不能为空".into();
        assert_eq!(d.rect(VIEW).height() - h, 18.0);
        let f = d.field_rect(VIEW).unwrap();
        assert_eq!(d.hit(VIEW, f.left + 5.0, f.top + 5.0), DialogHit::Field);
    }

    fn p() -> Palette {
        *theme::tokens().palette(false)
    }

    #[test]
    fn typing_and_backspace_round_trip() {
        let mut f = TextField::new("库名称");
        assert!(f.is_empty());
        f.char('a');
        f.char('中');
        assert_eq!(f.text(), "a中");
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_BACK;
        assert_eq!(f.key(VK_BACK.0, false, false), FieldKey::Edited);
        assert_eq!(f.text(), "a");
    }

    #[test]
    fn control_characters_are_rejected_but_enter_and_escape_are_reported() {
        let mut f = TextField::new("");
        assert!(!f.char('\u{8}'));
        assert!(!f.char('\r'));
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
        assert_eq!(f.key(VK_RETURN.0, false, false), FieldKey::Submit);
        assert_eq!(f.key(VK_ESCAPE.0, false, false), FieldKey::Cancel);
        assert_eq!(f.text(), "");
    }

    #[test]
    fn an_empty_field_paints_its_placeholder_in_muted() {
        let mut f = TextField::new("库名称");
        let mut list = DrawList::new();
        f.paint(
            &mut list,
            Rect::new(0.0, 0.0, 200.0, 28.0),
            false,
            &p(),
            FieldLook::inline(&p()),
        );
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { text, color, .. } if text == "库名称" && *color == p().muted)));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn a_focused_field_paints_a_caret_and_an_unfocused_one_does_not() {
        let mut f = TextField::new("").with_text("abc");
        let rect = Rect::new(0.0, 0.0, 200.0, 28.0);
        let caret_cmds = |focused: bool| {
            let mut list = DrawList::new();
            let mut f = f.clone();
            f.paint(&mut list, rect, focused, &p(), FieldLook::bare());
            list.cmds()
                .iter()
                .filter(|c| matches!(c, DrawCmd::Caret { rect, .. } if rect.width() == 1.0))
                .count()
        };
        assert_eq!(caret_cmds(true), 1);
        assert_eq!(caret_cmds(false), 0);
        f.clear();
    }

    #[test]
    fn painting_ime_composition_over_a_selection_does_not_panic() {
        let mut f = TextField::new("").with_text("Mochi User");
        f.select_all();
        f.buffer.set_composition("中", "中".len());

        let mut list = DrawList::new();
        f.paint(
            &mut list,
            Rect::new(0.0, 0.0, 200.0, 28.0),
            true,
            &p(),
            FieldLook::bare(),
        );

        assert!(list.finish().is_ok());
    }

    #[test]
    fn long_text_scrolls_so_the_caret_stays_visible() {
        let mut f = TextField::new("").with_text(&"x".repeat(200));
        let mut list = DrawList::new();
        let rect = Rect::new(0.0, 0.0, 100.0, 28.0);
        f.paint(&mut list, rect, true, &p(), FieldLook::bare());
        // 光标在末尾，滚动量必须让光标落在框内
        let caret = f.caret_x();
        assert!(caret <= rect.width() && caret >= 0.0, "caret_x = {caret}");
        assert!(f.scroll_x > 0.0);
    }

    #[test]
    fn clicking_maps_x_to_the_nearest_boundary() {
        let s = "abcd";
        let w = TextStyle::Body.advance_em('a') * TextStyle::Body.font_size();
        assert_eq!(offset_at_x(s, TextStyle::Body, 0.0), 0);
        // 点在第一个字的右半边算到第二个字前
        assert_eq!(offset_at_x(s, TextStyle::Body, w * 0.75), 1);
        assert_eq!(offset_at_x(s, TextStyle::Body, w * 10.0), 4);
    }

    #[test]
    fn a_menu_opens_at_the_pointer_and_folds_back_inside_the_viewport() {
        let items = vec![
            MenuItem::new("复制为 Mochi 链接", 1u8),
            MenuItem::new("删除", 2u8).danger().separated(),
        ];
        let m = Menu::open_at(items.clone(), 100.0, 100.0, VIEW);
        assert_eq!((m.rect.left, m.rect.top), (100.0, 100.0));
        assert!(m.rect.width() >= MENU_MIN_WIDTH);

        // 贴着右下角打开要翻折进来
        let m = Menu::open_at(items, 1190.0, 790.0, VIEW);
        assert!(m.rect.right <= VIEW.right);
        assert!(m.rect.bottom <= VIEW.bottom);
    }

    #[test]
    fn dropdown_stays_attached_to_its_control_and_scrolls_when_space_is_limited() {
        let items = (0..40)
            .map(|i| MenuItem::new(format!("助手 {i}"), i))
            .collect();
        let anchor = Rect::new(900.0, 680.0, 1120.0, 716.0);
        let mut menu = Menu::open_anchored(items, anchor, VIEW);
        assert_eq!(menu.rect.bottom, anchor.top - 4.0);
        assert!(menu.rect.top >= VIEW.top + 8.0);
        assert!(menu.rect.right <= VIEW.right - 8.0);
        let first = menu.hit(menu.rect.left + 30.0, menu.rect.top + MENU_PADDING + 10.0);
        assert_eq!(first, Some(0));
        menu.scroll_by(-10000.0);
        let last = menu.hit(
            menu.rect.left + 30.0,
            menu.rect.bottom - MENU_PADDING - 10.0,
        );
        assert_eq!(last, Some(39));
        let small = Menu::open_anchored(
            vec![MenuItem::new("挂载文档", 0)],
            Rect::new(20.0, 20.0, 240.0, 56.0),
            VIEW,
        );
        assert_eq!(small.rect.top, 60.0);
    }

    #[test]
    fn menu_hit_testing_skips_the_separator_and_disabled_items() {
        let items = vec![
            MenuItem::new("一", 1u8),
            MenuItem::new("二", 2u8).separated(),
            MenuItem::new("三", 3u8).disabled(true),
        ];
        let m = Menu::open_at(items, 0.0, 0.0, VIEW);
        let first_mid = MENU_PADDING + MENU_ITEM_HEIGHT / 2.0;
        assert_eq!(m.hit(50.0, first_mid), Some(0));
        // 分隔线那几个像素不属于任何项
        assert_eq!(m.hit(50.0, MENU_PADDING + MENU_ITEM_HEIGHT + 2.0), None);
        assert_eq!(
            m.hit(
                50.0,
                MENU_PADDING + MENU_ITEM_HEIGHT + MENU_SEPARATOR_HEIGHT + 2.0
            ),
            Some(1)
        );
        let third = MENU_PADDING + MENU_ITEM_HEIGHT * 2.0 + MENU_SEPARATOR_HEIGHT + 2.0;
        assert_eq!(m.hit(50.0, third), None, "禁用项不可点");
    }

    #[test]
    fn a_danger_item_is_painted_in_the_danger_color() {
        let items = vec![MenuItem::new("删除", 1u8).danger()];
        let m = Menu::open_at(items, 0.0, 0.0, VIEW);
        let mut list = DrawList::new();
        m.paint(&mut list, &p());
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { text, color, .. } if text == "删除" && *color == p().danger)));
    }
}
