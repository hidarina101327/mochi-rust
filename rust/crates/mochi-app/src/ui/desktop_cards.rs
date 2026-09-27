//! 桌面卡片管理器。
//!
//! 这个模块只描述管理器的交互和绘制，不负责文件对话框、窗口创建或把
//! 配置写回磁盘。`State` 持有一份草稿，宿主在收到 [`Action::Save`] 后再
//! 提交配置；这样取消编辑不会改动正在运行的桌面窗口。

mod appearance;
mod folder;
mod geometry;
mod model;
mod painting;
mod preferences;
mod shortcuts;
mod studio;
mod utility;

#[cfg(test)]
mod tests;

use crate::ui::{
    layout::Rect,
    overlay_scrollbar::{Axis, Bar, Interaction as ScrollInteraction},
    widgets::{FieldKey, TextField},
};

pub use geometry::Layout;
pub use model::{CardSizePreset, DesktopCard, DesktopConfig, DesktopPage, ModuleKind, PageRoute};

/// 管理器动作。涉及文件系统或窗口的动作都交给 App，UI 不在绘制线程中
/// 打开对话框，也不直接创建桌面窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    PickItemColor {
        background: bool,
        item: Option<String>,
    },
    Save(DesktopConfig),
    SaveKeepOpen(DesktopConfig),
    PickFontColor,
    PickBackgroundColor,
    PickStudioColor(bool),
    Cancel,
    Import,
    ExportAll,
    ExportCard(String),
    ChooseSource {
        card_id: String,
        page_id: String,
    },
    ChooseFolder {
        card_id: String,
        page_id: String,
    },
    LocateCard(String),
    OpenModule {
        card_id: String,
        page_id: String,
        module: ModuleKind,
    },
}

/// 可命中的语义区域。命中区域由 geometry 统一生成，避免绘制坐标和交互
/// 坐标在紧凑窗口中逐渐漂移。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    ShortcutSelect(usize),
    ShortcutDelete,
    ShortcutMove(i8),
    FolderChoose,
    FolderSort,
    FolderToggleHidden,
    ItemColor(bool, Option<usize>),
    ResetItemColor(bool, Option<usize>),
    Backdrop,
    Close,
    NewCard,
    Import,
    ExportAll,
    HideAll,
    Card(usize),
    CardDelete(usize),
    CardUp(usize),
    CardDown(usize),
    CardExport(usize),
    CardLocate(usize),
    CardDuplicate(usize),
    CardVisible(usize),
    CardLock(usize),
    CardSize(CardSizePreset),
    CardName,
    Opacity(i8),
    /// 不透明度滑轨上的点击位置（归一化到 0.0 – 1.0）。
    OpacityTrack(f32),
    FontSize(i8),
    /// 字号滑轨上的点击位置（归一化到 0.0 – 1.0）。
    FontSizeTrack(f32),
    FontColor(Option<u32>),
    FontCustom,
    EditorTab(u8),
    Studio(studio::Command),
    BackgroundColor(Option<u32>),
    BackgroundCustom,
    Spacing(u8),
    ToggleDetails,
    ToggleBorder,
    ToggleSeparators,
    Page(usize),
    AddPage,
    PageDelete(usize),
    PageUp(usize),
    PageDown(usize),
    PageName,
    Module(ModuleKind),
    Option(usize),
    PreferencesTab(bool),
    LimitDown,
    LimitAll,
    EditPreference(Field),
    Preference(preferences::Setting, i8),
    AddGroup,
    RemoveGroup(usize),
    LimitUp,
    Source,
    ClearSources,
    PageScroll(i8),
    Route(PageRoute),
    Save,
    SaveKeepOpen,
    Cancel,
    ConfirmDelete,
    CancelDelete,
    ConfirmCancel,
    DismissCancel,
}

/// 文本输入焦点。`focused_field_mut` 允许 App 将剪贴板和 IME 直接路由到
/// 管理器；文本变更后调用 [`State::commit_focused_field`] 同步到草稿。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Utility,
    ShortcutName(usize),
    ShortcutTarget(usize),
    FolderPath,
    FolderFilter,
    Studio(studio::Property),
    CardName,
    PageName,
    GroupName(usize),
    GroupRule(usize),
    Limit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FocusTarget {
    Hit(Hit),
    Field(Field),
}

/// 管理器交互状态。
#[derive(Debug)]
pub struct State {
    pub icon_workspace: std::path::PathBuf,
    /// 当前编辑中的配置草稿。
    pub config: DesktopConfig,
    pub creating: Option<bool>, // true：新建卡片；false：新建页面
    /// 被选中的卡片和分页均为草稿中的下标；对外动作使用稳定 id。
    pub selected_card: Option<usize>,
    pub selected_page: Option<usize>,
    pub hover: Option<Hit>,
    hovered_card: Option<usize>,
    pub focused: Option<Hit>,
    pub focus_field: Option<Field>,
    pub card_name: TextField,
    pub page_name: TextField,
    pub preference_text: TextField,
    field_dragging: bool,
    slider_drag: Option<usize>,
    pub cards_scroll: f32,
    pub modules_scroll: f32,
    pub options_scroll: f32,
    pub preferences_mode: bool,
    pub editor_tab: u8,
    pub studio_editor: studio::Editor,
    pub page_scroll: f32,
    pub embedded: bool,
    pub scrollbar: ScrollInteraction<ScrollTarget>,
    pub confirm_delete: Option<DeleteTarget>,
    pub confirm_cancel: bool,
    pub error: Option<String>,
    pub dirty: bool,
    next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    Cards,
    Modules,
    Options,
    Pages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteTarget {
    Card(usize),
    Page(usize),
}

impl State {
    /// Apply a folder pick result to the matching page in the editor draft.
    /// A canceled picker never calls this method, so it leaves the draft alone.
    pub fn set_folder_source(&mut self, card_id: &str, page_id: &str, path: String) {
        if path.chars().count() > mochi_core::desktop_cards::folder::MAX_FOLDER_PATH_CHARS {
            self.error = Some("文件夹路径过长，请选择更短的路径".into());
            return;
        }
        let Some(page) = self
            .config
            .cards
            .iter_mut()
            .find(|card| card.id == card_id)
            .and_then(|card| card.pages.iter_mut().find(|page| page.id == page_id))
        else {
            return;
        };
        if page.module != ModuleKind::Folder {
            return;
        }
        if page.folder.path == path {
            return;
        }
        page.folder.path = path.clone();
        page.folder.subfolder.clear();
        page.folder.manual_order.clear();
        page.folder.stacks.clear();
        page.folder.directory_views.clear();
        self.dirty = true;
        if self.focus_field == Some(Field::FolderPath)
            && self
                .selected_card_ref()
                .is_some_and(|card| card.id == card_id)
            && self
                .selected_page_ref()
                .is_some_and(|page| page.id == page_id)
        {
            self.preference_text.set_text(&path);
        }
    }

    pub fn studio_color(&self, background: bool) -> Option<u32> {
        let node = self
            .selected_page_ref()?
            .studio
            .nodes
            .get(self.studio_editor.selected?)?;
        if background {
            node.background
        } else {
            node.foreground
        }
    }
    pub fn set_studio_color(&mut self, background: bool, color: u32) {
        studio::checkpoint(self);
        studio::set(
            self,
            if background {
                studio::Property::Background
            } else {
                studio::Property::Foreground
            },
            &format!("#{color:06X}"),
        );
    }
    pub fn new(config: &DesktopConfig) -> Self {
        let mut state = Self {
            icon_workspace: Default::default(),
            config: config.clone(),
            creating: config.cards.is_empty().then_some(true),
            selected_card: (!config.cards.is_empty()).then_some(0),
            selected_page: config.cards.first().and_then(|c| {
                c.pages
                    .iter()
                    .position(|page| page.id == c.active_page)
                    .or_else(|| (!c.pages.is_empty()).then_some(0))
            }),
            hover: None,
            hovered_card: None,
            focused: None,
            focus_field: None,
            card_name: TextField::new("卡片名称"),
            page_name: TextField::new("分页名称"),
            preference_text: TextField::new("输入内容"),
            field_dragging: false,
            slider_drag: None,
            cards_scroll: 0.0,
            modules_scroll: 0.0,
            options_scroll: 0.0,
            preferences_mode: false,
            editor_tab: 1,
            studio_editor: Default::default(),
            page_scroll: 0.0,
            embedded: false,
            scrollbar: ScrollInteraction::default(),
            confirm_delete: None,
            confirm_cancel: false,
            error: None,
            dirty: false,
            next_id: 1,
        };
        state.config.migrate_templates();
        state.next_id = state.max_id() + 1;
        state.sync_editors();
        state
    }

    pub fn is_open(&self) -> bool {
        true
    }

    fn card_toolbar_visible(&self, index: usize) -> bool {
        self.creating.is_none()
            && (self.hovered_card == Some(index)
                || matches!(self.focused,
                Some(Hit::CardDelete(i) | Hit::CardUp(i) | Hit::CardDown(i)
                    | Hit::CardExport(i) | Hit::CardLocate(i) | Hit::CardDuplicate(i)) if i == index))
    }

    pub fn selected_card_ref(&self) -> Option<&DesktopCard> {
        self.selected_card.and_then(|i| self.config.cards.get(i))
    }

    pub fn selected_card_mut(&mut self) -> Option<&mut DesktopCard> {
        self.selected_card
            .and_then(|i| self.config.cards.get_mut(i))
    }

    pub fn selected_page_ref(&self) -> Option<&DesktopPage> {
        self.selected_card_ref()
            .and_then(|c| self.selected_page.and_then(|i| c.pages.get(i)))
    }

    pub fn selected_page_mut(&mut self) -> Option<&mut DesktopPage> {
        let page = self.selected_page?;
        self.selected_card_mut()?.pages.get_mut(page)
    }

    pub fn focused_field_mut(&mut self) -> Option<&mut TextField> {
        match self.focus_field {
            Some(Field::CardName) => Some(&mut self.card_name),
            Some(Field::PageName) => Some(&mut self.page_name),
            Some(_) => Some(&mut self.preference_text),
            None => None,
        }
    }

    /// 与宿主的 IME/剪贴板路由配合。调用后会限制名称长度并保留草稿状态。
    pub fn commit_focused_field(&mut self) -> bool {
        let field = self.focus_field;
        let text = field.and_then(|f| match f {
            Field::CardName => Some(self.card_name.text().to_owned()),
            Field::PageName => Some(self.page_name.text().to_owned()),
            _ => Some(self.preference_text.text().to_owned()),
        });
        let Some(text) = text else { return false };
        let text = if matches!(field, Some(Field::Studio(_))) {
            text.chars().take(4096).collect()
        } else if matches!(field, Some(Field::GroupRule(_))) {
            text.chars().take(1024).collect()
        } else if matches!(field, Some(Field::FolderPath)) {
            text.chars()
                .take(mochi_core::desktop_cards::folder::MAX_FOLDER_PATH_CHARS)
                .collect()
        } else if matches!(field, Some(Field::Utility)) {
            text.chars().take(512).collect()
        } else if matches!(field, Some(Field::FolderFilter)) {
            text.chars()
                .take(mochi_core::desktop_cards::folder::MAX_FILTER_CHARS)
                .collect()
        } else {
            bounded_name(&text)
        };
        match field {
            Some(Field::Studio(prop)) => {
                studio::set(self, prop, &text);
            }
            Some(Field::CardName) => {
                if let Some(card) = self.selected_card_mut() {
                    if card.title != text {
                        card.title = text;
                        self.dirty = true;
                        return true;
                    }
                }
            }
            Some(Field::PageName) => {
                if let Some(page) = self.selected_page_mut() {
                    if page.title != text {
                        page.title = text;
                        self.dirty = true;
                        return true;
                    }
                }
            }
            Some(Field::GroupName(i)) | Some(Field::GroupRule(i)) => {
                if let Some(page) = self.selected_page_mut() {
                    if let Some(group) = page.presentation.groups.get_mut(i) {
                        let value = if matches!(field, Some(Field::GroupName(_))) {
                            &mut group.name
                        } else {
                            &mut group.rule
                        };
                        if *value != text {
                            *value = text;
                            self.dirty = true;
                            return true;
                        }
                    }
                }
            }
            Some(Field::ShortcutName(i)) | Some(Field::ShortcutTarget(i)) => {
                if let Some(node) = self
                    .selected_page_mut()
                    .and_then(|p| p.studio.nodes.get_mut(i))
                {
                    if matches!(field, Some(Field::ShortcutName(_))) {
                        node.title = text;
                    } else {
                        node.target = text;
                    }
                    self.dirty = true;
                }
            }
            Some(Field::Utility) => {
                if let Some(page) = self.selected_page_mut() {
                    let value = match page.module {
                        ModuleKind::Weather => &mut page.utility.location,
                        ModuleKind::Search => &mut page.utility.query,
                        _ => &mut page.utility.media_source,
                    };
                    if *value != text {
                        *value = text;
                        self.dirty = true;
                        return true;
                    }
                }
            }
            Some(Field::FolderPath) | Some(Field::FolderFilter) => {
                if let Some(folder) = self.selected_page_mut().map(|page| &mut page.folder) {
                    let value = if matches!(field, Some(Field::FolderPath)) {
                        &mut folder.path
                    } else {
                        &mut folder.filter
                    };
                    if *value != text {
                        *value = text;
                        if matches!(field, Some(Field::FolderPath)) {
                            folder.subfolder.clear();
                            folder.manual_order.clear();
                            folder.stacks.clear();
                            folder.directory_views.clear();
                        }
                        self.dirty = true;
                        return true;
                    }
                }
            }
            Some(Field::Limit) => {
                if let Ok(limit) = text.parse::<usize>() {
                    if let Some(page) = self.selected_page_mut() {
                        page.limit = limit;
                        self.dirty = true;
                    }
                }
            }
            None => {}
        }
        false
    }

    /// App 把字符/按键直接路由给字段后，同步各字段的缓冲区。
    pub fn sync_fields(&mut self) {
        self.commit_focused_field();
    }

    pub fn caret_rect(&self, layout: &Layout) -> Option<Rect> {
        match self.focus_field {
            Some(Field::CardName) => Some(self.card_name_caret(layout.card_name)),
            Some(Field::PageName) => Some(self.page_name_caret(layout.page_name)),
            Some(field) => layout
                .controls
                .iter()
                .find(|(_, h)| *h == Hit::EditPreference(field))
                .map(|(r, _)| {
                    Rect::from_size(
                        r.left + 10.0 + self.preference_text.caret_x(),
                        r.top + 6.0,
                        1.0,
                        r.height() - 12.0,
                    )
                }),
            None => None,
        }
    }

    fn card_name_caret(&self, rect: Rect) -> Rect {
        let inner = rect.inset(crate::ui::layout::Edges::xy(10.0, 0.0));
        let x = inner.left + self.card_name.caret_x();
        Rect::new(x, inner.top + 7.0, x + 1.0, inner.bottom - 7.0)
    }

    fn page_name_caret(&self, rect: Rect) -> Rect {
        let inner = rect.inset(crate::ui::layout::Edges::xy(10.0, 0.0));
        let x = inner.left + self.page_name.caret_x();
        Rect::new(x, inner.top + 6.0, x + 1.0, inner.bottom - 6.0)
    }

    pub fn workspace(config: &DesktopConfig) -> Self {
        let mut state = Self::new(config);
        state.embedded = true;
        state
    }

    pub fn layout(&self, viewport: Rect) -> Layout {
        geometry::layout(self, viewport)
    }

    pub fn reveal_selected_page(&mut self, viewport: Rect) {
        let layout = self.layout(viewport);
        let Some((rect, _)) = layout
            .page_rows
            .iter()
            .find(|(_, i)| Some(*i) == self.selected_page)
        else {
            return;
        };
        let right = layout.page_tabs.right - 102.0;
        let delta = if rect.left < layout.page_tabs.left {
            rect.left - layout.page_tabs.left
        } else {
            (rect.right - right).max(0.0)
        };
        self.page_scroll = (self.page_scroll.clamp(0.0, layout.pages_max_scroll) + delta)
            .clamp(0.0, layout.pages_max_scroll);
    }

    pub fn paint(
        &mut self,
        list: &mut crate::ui::draw::DrawList,
        layout: &Layout,
        viewport: Rect,
        p: &crate::ui::theme::Palette,
    ) {
        painting::paint(list, self, layout, viewport, p)
    }

    pub fn double_click(&mut self, layout: &Layout, x: f32, y: f32) {
        if self.confirm_cancel || self.confirm_delete.is_some() || self.creating.is_some() {
            return;
        }
        if let Some(Hit::Page(i)) = layout.hit(x, y) {
            self.select_page(i);
            self.focus_field = Some(Field::PageName);
            self.page_name.buffer.select_all();
        } else if let Some(f) = self.focused_field_mut() {
            f.buffer.select_word();
        }
    }

    pub fn click(&mut self, layout: &Layout, x: f32, y: f32) -> Option<Action> {
        if self.confirm_cancel || self.confirm_delete.is_some() {
            return layout.hit(x, y).and_then(|hit| self.activate(hit));
        }
        let bars = self.bars(layout);
        if self.scrollbar.dragging() {
            if let Some((target, offset)) = self.scrollbar.drag_to(&bars, x, y) {
                self.set_scroll(target, offset, layout);
            }
            self.scrollbar.end();
            return None;
        }
        if let Some((target, offset)) = self.scrollbar.begin(&bars, x, y) {
            self.set_scroll(target, offset, layout);
            return None;
        }
        let hit = layout.hit(x, y)?;
        self.focused = Some(match hit {
            Hit::OpacityTrack(_) | Hit::Opacity(_) => Hit::OpacityTrack(0.0),
            Hit::FontSizeTrack(_) | Hit::FontSize(_) => Hit::FontSizeTrack(0.0),
            Hit::CardDelete(i)
            | Hit::CardUp(i)
            | Hit::CardDown(i)
            | Hit::CardExport(i)
            | Hit::CardLocate(i)
            | Hit::CardDuplicate(i) => Hit::Card(i),
            _ => hit,
        });
        match hit {
            Hit::CardName => {
                self.activate(hit);
                self.card_name
                    .click(x - layout.card_name.left - 10.0, false);
                return None;
            }
            Hit::PageName => {
                self.activate(hit);
                self.page_name
                    .click(x - layout.page_name.left - 10.0, false);
                return None;
            }
            _ => {}
        }
        let selection = (self.selected_card, self.selected_page, self.creating);
        let action = self.activate(hit);
        if selection != (self.selected_card, self.selected_page, self.creating) {
            self.reveal_selected_page(layout.viewport);
        }
        action
    }

    pub fn pointer(&mut self, layout: &Layout, x: f32, y: f32) -> bool {
        if studio::drag(self, layout, x, y) {
            return true;
        }
        if let Some(row) = self.slider_drag {
            let value = appearance::track_hit_value(layout.right_body, row, x);
            self.activate(if row == 0 {
                Hit::OpacityTrack(value)
            } else {
                Hit::FontSizeTrack(value)
            });
            return true;
        }
        if self.field_dragging {
            match self.focus_field {
                Some(Field::CardName) => {
                    self.card_name.click(x - layout.card_name.left - 10.0, true);
                    return true;
                }
                Some(Field::PageName) => {
                    self.page_name.click(x - layout.page_name.left - 10.0, true);
                    return true;
                }
                Some(field) => {
                    if let Some((r, _)) = layout
                        .controls
                        .iter()
                        .find(|(_, h)| *h == Hit::EditPreference(field))
                    {
                        self.preference_text.click(x - r.left - 10.0, true);
                        return true;
                    }
                }
                None => {}
            }
        }
        let bars = self.bars(layout);
        if self.scrollbar.dragging() {
            if let Some((target, offset)) = self.scrollbar.drag_to(&bars, x, y) {
                self.set_scroll(target, offset, layout);
                return true;
            }
        }
        let bar_changed = self.scrollbar.pointer(&bars, x, y);
        let hovered_card = layout
            .card_rows
            .iter()
            .find(|(row, _)| layout.cards_body.contains(x, y) && row.contains(x, y))
            .map(|(_, index)| *index);
        let card_changed = self.hovered_card != hovered_card;
        self.hovered_card = hovered_card;
        let next = layout.hit(x, y).map(|hit| match hit {
            Hit::OpacityTrack(_) => Hit::OpacityTrack(0.0),
            Hit::FontSizeTrack(_) => Hit::FontSizeTrack(0.0),
            _ => hit,
        });
        if next == self.hover {
            return bar_changed || card_changed;
        }
        self.hover = next;
        true
    }

    /// 宿主的鼠标处理可在按下时调用这里，让滑块拖动在第一次指针移动
    /// 落在原滑块之外时也能正常工作。
    pub fn pointer_down(&mut self, layout: &Layout, x: f32, y: f32) -> bool {
        if self.confirm_cancel || self.confirm_delete.is_some() {
            return false;
        }
        if studio::begin(self, layout, x, y) {
            return true;
        }
        if let Some(hit @ (Hit::OpacityTrack(_) | Hit::FontSizeTrack(_))) = layout.hit(x, y) {
            self.slider_drag = Some(if matches!(hit, Hit::OpacityTrack(_)) {
                0
            } else {
                1
            });
            self.activate(hit);
            return true;
        }
        if let Some(Hit::EditPreference(field)) = layout.hit(x, y) {
            if self.focus_field != Some(field) {
                self.activate(Hit::EditPreference(field));
            }
            if let Some((r, _)) = layout
                .controls
                .iter()
                .find(|(_, h)| *h == Hit::EditPreference(field))
            {
                self.preference_text.click(x - r.left - 10.0, false);
                self.field_dragging = true;
                return true;
            }
        }
        if self.selected_card.is_some() && layout.card_name.contains(x, y) {
            self.activate(Hit::CardName);
            self.card_name
                .click(x - layout.card_name.left - 10.0, false);
            self.field_dragging = true;
            return true;
        }
        if self.focus_field == Some(Field::PageName) && layout.page_name.contains(x, y) {
            self.activate(Hit::PageName);
            self.page_name
                .click(x - layout.page_name.left - 10.0, false);
            self.field_dragging = true;
            return true;
        }
        let bars = self.bars(layout);
        let Some((target, offset)) = self.scrollbar.begin(&bars, x, y) else {
            return false;
        };
        self.set_scroll(target, offset, layout);
        true
    }

    pub fn pointer_up(&mut self) -> bool {
        if self.studio_editor.drag.take().is_some() {
            return true;
        }
        let changed = self.scrollbar.end();
        self.field_dragging = false;
        self.slider_drag = None;
        changed
    }

    pub fn dragging(&self) -> bool {
        self.studio_editor.drag.is_some()
            || self.slider_drag.is_some()
            || self.field_dragging
            || self.scrollbar.dragging()
    }

    pub fn wheel(&mut self, layout: &Layout, x: f32, y: f32, pixels: f32) -> bool {
        if self.editor_tab == 2 && studio::scroll_picker(self, layout.right_body, pixels) {
            return true;
        }
        if self.confirm_cancel || self.confirm_delete.is_some() {
            return false;
        }
        if let Some(hit) = layout.hit(x, y) {
            let delta = if pixels > 0.0 { 1 } else { -1 };
            match hit {
                Hit::Preference(
                    setting @ (preferences::Setting::TabsRatio
                    | preferences::Setting::Padding
                    | preferences::Setting::HeadingSize
                    | preferences::Setting::Columns
                    | preferences::Setting::Rows
                    | preferences::Setting::GridHeight),
                    _,
                ) => {
                    preferences::change(self, setting, delta);
                    return true;
                }
                Hit::OpacityTrack(_) => {
                    self.activate(Hit::Opacity(delta));
                    return true;
                }
                Hit::FontSizeTrack(_) => {
                    self.activate(Hit::FontSize(delta));
                    return true;
                }
                _ => {}
            }
        }
        let amount = if pixels.abs() > 1.0 {
            pixels
        } else {
            pixels * 36.0
        };
        if layout.cards_body.contains(x, y) {
            self.cards_scroll = (self.cards_scroll - amount).clamp(0.0, layout.cards_max_scroll);
            return true;
        }
        if layout.modules_body.contains(x, y) {
            self.modules_scroll =
                (self.modules_scroll - amount).clamp(0.0, layout.modules_max_scroll);
            return true;
        }
        if layout.options_body.contains(x, y) {
            self.options_scroll =
                (self.options_scroll - amount).clamp(0.0, layout.options_max_scroll);
            return true;
        }
        if layout.page_tabs.contains(x, y) {
            self.page_scroll = (self.page_scroll - amount).clamp(0.0, layout.pages_max_scroll);
            return true;
        }
        false
    }

    fn set_scroll(&mut self, target: ScrollTarget, offset: f32, layout: &Layout) {
        match target {
            ScrollTarget::Cards => self.cards_scroll = offset.clamp(0.0, layout.cards_max_scroll),
            ScrollTarget::Modules => {
                self.modules_scroll = offset.clamp(0.0, layout.modules_max_scroll)
            }
            ScrollTarget::Options => {
                self.options_scroll = offset.clamp(0.0, layout.options_max_scroll)
            }
            ScrollTarget::Pages => self.page_scroll = offset.clamp(0.0, layout.pages_max_scroll),
        }
    }

    pub fn key(&mut self, key: u16, shift: bool, ctrl: bool) -> Option<Action> {
        if self.studio_editor.picker.is_some() {
            studio::picker_key(self, key, shift);
            return None;
        }
        if self.confirm_delete.is_some() {
            return match key {
                27 => self.activate(Hit::CancelDelete),
                9 => {
                    self.move_focus(shift);
                    None
                }
                13 | 32 => self.activate(
                    self.focused
                        .filter(|h| matches!(h, Hit::CancelDelete | Hit::ConfirmDelete))
                        .unwrap_or(Hit::CancelDelete),
                ),
                _ => None,
            };
        }
        if self.confirm_cancel {
            return match key {
                0x1b => self.activate(Hit::DismissCancel),
                0x09 => {
                    self.move_focus(shift);
                    None
                }
                0x0d | 0x20 => self.activate(
                    self.focused
                        .filter(|h| matches!(h, Hit::DismissCancel | Hit::ConfirmCancel))
                        .unwrap_or(Hit::DismissCancel),
                ),
                _ => None,
            };
        }
        self.commit_focused_field();
        if self.editor_tab == 2 && self.focus_field.is_none() {
            let c = match (key, ctrl, shift) {
                (90, true, false) => Some(studio::Command::Undo),
                (90, true, true) | (89, true, _) => Some(studio::Command::Redo),
                (68, true, _) => Some(studio::Command::Duplicate),
                (46, false, _) => Some(studio::Command::Delete),
                _ => None,
            };
            if let Some(c) = c {
                studio::command(self, c);
                return None;
            }
        }
        if self.focus_field.is_none() && !self.confirm_cancel && self.confirm_delete.is_none() {
            let delta = match key {
                0x25 | 0x28 => Some(-1),
                0x26 | 0x27 => Some(1),
                _ => None,
            };
            if let Some(delta) = delta {
                match self.focused {
                    Some(Hit::OpacityTrack(_)) => return self.activate(Hit::Opacity(delta)),
                    Some(Hit::FontSizeTrack(_)) => return self.activate(Hit::FontSize(delta)),
                    _ => {}
                }
            }
        }
        match key {
            0x1b => {
                if self.creating.take().is_some() {
                    return None;
                }
                if self.confirm_delete.is_some() {
                    self.confirm_delete = None;
                    return None;
                }
                if self.focus_field.is_some() {
                    self.focus_field = None;
                    return None;
                }
                if self.confirm_cancel {
                    self.confirm_cancel = false;
                    self.focused = Some(Hit::Cancel);
                    return None;
                }
                if self.dirty {
                    self.confirm_cancel = true;
                    self.focused = Some(Hit::DismissCancel);
                    return None;
                }
                return Some(Action::Cancel);
            }
            0x09 => {
                self.move_focus(shift);
                return None;
            }
            0x0d => {
                if self.focus_field.is_some() {
                    let _ = self
                        .focused_field_mut()
                        .map(|field| field.key(key, shift, ctrl));
                    self.commit_focused_field();
                    self.focus_field = None;
                    return None;
                }
                if let Some(target) = self.focused {
                    if matches!(target, Hit::OpacityTrack(_) | Hit::FontSizeTrack(_)) {
                        return None;
                    }
                    return self.activate(target);
                }
                return None;
            }
            0x2e => {
                if self.focus_field.is_some() {
                    let _ = self
                        .focused_field_mut()
                        .map(|field| field.key(key, shift, ctrl));
                    self.commit_focused_field();
                    return None;
                }
                if let Some(Hit::Card(i)) = self.focused {
                    self.confirm_delete = Some(DeleteTarget::Card(i));
                    self.focused = Some(Hit::CancelDelete);
                } else if let Some(Hit::Page(i)) = self.focused {
                    self.confirm_delete = Some(DeleteTarget::Page(i));
                    self.focused = Some(Hit::CancelDelete);
                }
                return None;
            }
            _ => {}
        }
        if ctrl && key == b'S' as u16 {
            return self.activate(Hit::SaveKeepOpen);
        }
        if self.focus_field.is_some() {
            let result = self
                .focused_field_mut()
                .map(|f| f.key(key, shift, ctrl))
                .unwrap_or(FieldKey::Ignored);
            if matches!(result, FieldKey::Cancel) {
                self.focus_field = None;
            }
            self.commit_focused_field();
            return None;
        }
        match key {
            0x71 => {
                if self.selected_page.is_some() {
                    self.focus_field = Some(Field::PageName);
                    self.page_name.buffer.select_all();
                }
            }
            0x25 => self.move_focus(true),
            0x27 => self.move_focus(false),
            0x26 => self.move_focus(true),
            0x28 => self.move_focus(false),
            k if ctrl && k == b'D' as u16 => match self.focused {
                Some(Hit::Card(index)) => self.duplicate_card(index),
                Some(Hit::Page(index)) => self.duplicate_page(index),
                _ => {}
            },
            k if ctrl && k == b'S' as u16 => return self.activate(Hit::SaveKeepOpen),
            _ => {}
        }
        None
    }

    pub fn char(&mut self, character: char) -> bool {
        let changed = self
            .focused_field_mut()
            .map(|f| f.char(character))
            .unwrap_or(false);
        if changed {
            self.commit_focused_field();
        }
        changed
    }

    fn activate(&mut self, hit: Hit) -> Option<Action> {
        if matches!(hit, Hit::Save | Hit::SaveKeepOpen)
            && self.focus_field == Some(Field::Limit)
            && self.preference_text.text().parse::<usize>().is_err()
        {
            self.error = Some("显示数量请输入正整数，或输入 0 表示全部".into());
            return None;
        }
        if !matches!(hit, Hit::CardName | Hit::PageName | Hit::EditPreference(_)) {
            self.commit_focused_field();
            self.focus_field = None;
        }
        if self.confirm_delete.is_some() {
            return match hit {
                Hit::ConfirmDelete => {
                    self.delete_confirmed();
                    None
                }
                Hit::CancelDelete => {
                    self.confirm_delete = None;
                    None
                }
                _ => None,
            };
        }
        if self.confirm_cancel {
            return match hit {
                Hit::ConfirmCancel => Some(Action::Cancel),
                Hit::DismissCancel => {
                    self.confirm_cancel = false;
                    None
                }
                _ => None,
            };
        }
        match hit {
            Hit::ShortcutSelect(i) => {
                self.studio_editor.selected = Some(i);
                None
            }
            Hit::ShortcutDelete => {
                if let Some(i) = self.studio_editor.selected {
                    if let Some(p) = self.selected_page_mut() {
                        if i < p.studio.nodes.len() {
                            let n = p.studio.nodes.remove(i);
                            p.item_styles.remove(&format!("node:{}", n.id));
                            self.dirty = true;
                        }
                    }
                }
                self.studio_editor.selected = None;
                None
            }
            Hit::ShortcutMove(delta) => {
                if let Some(i) = self.studio_editor.selected {
                    if let Some(p) = self.selected_page_mut() {
                        let to = (i as i32 + delta as i32)
                            .clamp(0, p.studio.nodes.len().saturating_sub(1) as i32)
                            as usize;
                        p.studio.nodes.swap(i, to);
                        self.studio_editor.selected = Some(to);
                        self.dirty = true;
                    }
                }
                None
            }
            Hit::ItemColor(background, index) => Some(Action::PickItemColor {
                background,
                item: index.and_then(|i| {
                    self.selected_page_ref()?
                        .studio
                        .nodes
                        .get(i)
                        .map(|n| format!("node:{}", n.id))
                }),
            }),
            Hit::ResetItemColor(background, index) => {
                if let Some(p) = self.selected_page_mut() {
                    if let Some(i) = index {
                        if let Some(n) = p.studio.nodes.get(i) {
                            let style = p.item_styles.entry(format!("node:{}", n.id)).or_default();
                            if background {
                                style.background = None
                            } else {
                                style.foreground = None
                            }
                        }
                    } else if background {
                        p.presentation.item_background = None
                    } else {
                        p.presentation.item_foreground = None
                    }
                    self.dirty = true;
                }
                None
            }

            Hit::Backdrop | Hit::Close => {
                if self.dirty {
                    self.confirm_cancel = true;
                    self.focused = Some(Hit::DismissCancel);
                    None
                } else {
                    Some(Action::Cancel)
                }
            }
            Hit::Cancel => {
                if self.dirty {
                    self.confirm_cancel = true;
                    self.focused = Some(Hit::DismissCancel);
                    None
                } else {
                    Some(Action::Cancel)
                }
            }
            Hit::NewCard => {
                self.focus_field = None;
                self.creating = Some(true);
                self.modules_scroll = 0.0;
                None
            }
            Hit::Import => Some(Action::Import),
            Hit::ExportAll => Some(Action::ExportAll),
            Hit::HideAll => {
                let mut changed = false;
                for card in &mut self.config.cards {
                    changed |= card.enabled;
                    card.enabled = false;
                }
                self.dirty |= changed;
                None
            }
            Hit::Card(i) => {
                self.creating = None;
                self.focus_field = None;
                self.select_card(i);
                None
            }
            Hit::CardDelete(i) => {
                self.confirm_delete = Some(DeleteTarget::Card(i));
                self.focused = Some(Hit::CancelDelete);
                None
            }
            Hit::CardUp(i) => {
                self.move_card(i, -1);
                None
            }
            Hit::CardDown(i) => {
                self.move_card(i, 1);
                None
            }
            Hit::CardExport(i) => self
                .config
                .cards
                .get(i)
                .map(|c| Action::ExportCard(c.id.clone())),
            Hit::CardLocate(i) => self
                .config
                .cards
                .get(i)
                .map(|c| Action::LocateCard(c.id.clone())),
            Hit::CardDuplicate(i) => {
                self.duplicate_card(i);
                None
            }
            Hit::CardVisible(i) => {
                if let Some(card) = self.config.cards.get_mut(i) {
                    card.enabled = !card.enabled;
                    self.dirty = true;
                }
                None
            }
            Hit::CardLock(i) => {
                if let Some(card) = self.config.cards.get_mut(i) {
                    card.locked = !card.locked;
                    self.dirty = true;
                }
                None
            }
            Hit::CardSize(preset) => {
                if let Some(card) = self.selected_card_mut() {
                    let (width, height) = preset.size();
                    card.width = width;
                    card.height = height;
                    self.dirty = true;
                }
                None
            }
            Hit::CardName => {
                self.focus_field = Some(Field::CardName);
                self.focused = Some(hit);
                None
            }
            Hit::Page(i) => {
                self.select_page(i);
                None
            }
            Hit::AddPage => {
                self.focus_field = None;
                self.creating = Some(false);
                self.modules_scroll = 0.0;
                None
            }
            Hit::PageDelete(i) => {
                self.confirm_delete = Some(DeleteTarget::Page(i));
                self.focused = Some(Hit::CancelDelete);
                None
            }
            Hit::PageUp(i) => {
                self.move_page(i, -1);
                None
            }
            Hit::PageDown(i) => {
                self.move_page(i, 1);
                None
            }
            Hit::PageName => {
                self.focus_field = Some(Field::PageName);
                self.focused = Some(hit);
                None
            }
            Hit::Module(module) => {
                if let Some(new_card) = self.creating.take() {
                    if new_card {
                        if self.config.cards.len() >= mochi_core::desktop_cards::MAX_CARDS {
                            self.error = Some("最多支持 12 张卡片".into());
                            return None;
                        }
                        self.add_card();
                    } else {
                        if self.selected_card_ref().is_none_or(|c| {
                            c.pages.len() >= mochi_core::desktop_cards::MAX_PAGES_PER_CARD
                        }) {
                            self.error = Some("每张卡片最多支持 8 个分页".into());
                            return None;
                        }
                        self.add_page();
                    }
                    if let Some(page) = self.selected_page_mut() {
                        let id = page.id.clone();
                        *page = DesktopPage::new(module);
                        page.id = id;
                    }
                    self.sync_editors();
                    self.studio_editor = Default::default();
                    self.editor_tab = if studio::enabled(self) { 2 } else { 1 };
                    self.dirty = true;
                }
                None
            }
            Hit::Opacity(delta) => {
                if let Some(c) = self.selected_card_mut() {
                    let value = (c.appearance.opacity as i16 + delta as i16).clamp(35, 100) as u8;
                    let changed = c.appearance.opacity != value;
                    c.appearance.opacity = value;
                    self.dirty |= changed;
                }
                None
            }
            Hit::OpacityTrack(pct) => {
                if pct.is_finite() {
                    if let Some(c) = self.selected_card_ref() {
                        let value = (35.0 + pct.clamp(0.0, 1.0) * 65.0).round() as i16;
                        return self
                            .activate(Hit::Opacity((value - c.appearance.opacity as i16) as i8));
                    }
                }
                None
            }
            Hit::FontSize(delta) => {
                if let Some(c) = self.selected_card_mut() {
                    let value = (c.appearance.font_size as i16 + delta as i16).clamp(8, 72) as u8;
                    let changed = c.appearance.font_size != value;
                    c.appearance.font_size = value;
                    self.dirty |= changed;
                }
                None
            }
            Hit::FontSizeTrack(pct) => {
                if pct.is_finite() {
                    if let Some(c) = self.selected_card_ref() {
                        let value = (8.0 + pct.clamp(0.0, 1.0) * 64.0).round() as i16;
                        return self.activate(Hit::FontSize(
                            (value - c.appearance.font_size as i16) as i8,
                        ));
                    }
                }
                None
            }
            Hit::Studio(command) => {
                if let studio::Command::Color(background) = command {
                    return Some(Action::PickStudioColor(background));
                }
                self.commit_focused_field();
                self.focus_field = None;
                studio::command(self, command);
                None
            }
            Hit::EditorTab(tab) => {
                self.commit_focused_field();
                self.focus_field = None;
                self.editor_tab = tab;
                self.options_scroll = 0.0;
                None
            }
            Hit::BackgroundCustom => Some(Action::PickBackgroundColor),
            Hit::BackgroundColor(color) => {
                if let Some(c) = self.selected_card_mut() {
                    c.appearance.background_color = color;
                    self.dirty = true;
                }
                None
            }
            Hit::Spacing(value) => {
                if let Some(c) = self.selected_card_mut() {
                    c.appearance.spacing = value.min(2);
                    c.appearance.row_padding = [0, 4, 12][value.min(2) as usize];
                    self.dirty = true;
                }
                None
            }
            Hit::ToggleDetails | Hit::ToggleBorder | Hit::ToggleSeparators => {
                if let Some(c) = self.selected_card_mut() {
                    let value = match hit {
                        Hit::ToggleDetails => &mut c.appearance.show_details,
                        Hit::ToggleBorder => &mut c.appearance.show_border,
                        _ => &mut c.appearance.show_separators,
                    };
                    *value = !*value;
                    self.dirty = true;
                }
                None
            }
            Hit::FontCustom => Some(Action::PickFontColor),
            Hit::FontColor(color) => {
                if let Some(c) = self.selected_card_mut() {
                    c.appearance.font_color = color;
                    self.dirty = true;
                }
                None
            }
            Hit::PreferencesTab(value) => {
                self.preferences_mode = value;
                self.options_scroll = 0.0;
                None
            }
            Hit::Option(i) => {
                if let Some(page) = self.selected_page_mut() {
                    let options = model::fields_for(page.module);
                    if let Some(option) = options.get(i) {
                        if let Some(position) =
                            page.options.iter().position(|key| key == &option.key)
                        {
                            page.options.remove(position);
                        } else {
                            page.options.push(option.key.clone());
                        }
                        self.dirty = true;
                    }
                }
                None
            }
            Hit::Preference(setting, delta) => {
                preferences::change(self, setting, delta);
                None
            }
            Hit::EditPreference(field) => {
                self.commit_focused_field();
                if let Field::Studio(prop) = field {
                    studio::checkpoint(self);
                    let text = studio::value(self, prop);
                    self.preference_text.set_text(&text);
                    self.preference_text.buffer.select_all();
                    self.focus_field = Some(field);
                    return None;
                }
                let text = self
                    .selected_page_ref()
                    .map(|page| match field {
                        Field::GroupName(i) => page
                            .presentation
                            .groups
                            .get(i)
                            .map(|g| g.name.clone())
                            .unwrap_or_default(),
                        Field::GroupRule(i) => page
                            .presentation
                            .groups
                            .get(i)
                            .map(|g| g.rule.clone())
                            .unwrap_or_default(),
                        Field::ShortcutName(i) => page
                            .studio
                            .nodes
                            .get(i)
                            .map(|n| n.title.clone())
                            .unwrap_or_default(),
                        Field::ShortcutTarget(i) => page
                            .studio
                            .nodes
                            .get(i)
                            .map(|n| n.target.clone())
                            .unwrap_or_default(),
                        Field::FolderPath => page.folder.path.clone(),
                        Field::FolderFilter => page.folder.filter.clone(),
                        Field::Utility => match page.module {
                            ModuleKind::Weather => page.utility.location.clone(),
                            ModuleKind::Search => page.utility.query.clone(),
                            _ => page.utility.media_source.clone(),
                        },
                        Field::Limit => page.limit.to_string(),
                        _ => String::new(),
                    })
                    .unwrap_or_default();
                self.preference_text = TextField::new(match field {
                    Field::FolderPath => "粘贴或输入本地文件夹路径",
                    Field::FolderFilter => "输入名称过滤规则",
                    _ => "输入内容",
                })
                .with_text(&text);
                self.preference_text.buffer.select_all();
                self.focus_field = Some(field);
                None
            }
            Hit::AddGroup => {
                if let Some(page) = self.selected_page_mut() {
                    if page.presentation.groups.len() < 32 {
                        page.presentation
                            .groups
                            .push(mochi_core::desktop_cards::ScheduleGroup {
                                name: "新分组".into(),
                                rule: "优先级 = 一般".into(),
                            });
                        self.dirty = true;
                    }
                }
                None
            }
            Hit::RemoveGroup(index) => {
                if let Some(page) = self.selected_page_mut() {
                    if index < page.presentation.groups.len() {
                        page.presentation.groups.remove(index);
                        self.dirty = true;
                    }
                }
                None
            }
            Hit::LimitAll => {
                if let Some(page) = self.selected_page_mut() {
                    page.limit = 0;
                    self.dirty = true;
                }
                None
            }
            Hit::LimitDown => {
                if let Some(page) = self.selected_page_mut() {
                    page.limit = page.limit.saturating_sub(1).max(1);
                    self.dirty = true;
                }
                None
            }
            Hit::LimitUp => {
                if let Some(page) = self.selected_page_mut() {
                    page.limit = page.limit.saturating_add(1);
                    self.dirty = true;
                }
                None
            }
            Hit::ClearSources => {
                if let Some(page) = self.selected_page_mut() {
                    page.sources.clear();
                    page.source = None;
                    self.dirty = true;
                }
                None
            }
            Hit::PageScroll(delta) => {
                self.page_scroll = (self.page_scroll + delta as f32 * 200.0).max(0.0);
                None
            }
            Hit::Source => {
                let card_id = self.selected_card_ref()?.id.clone();
                let page_id = self.selected_page_ref()?.id.clone();
                Some(Action::ChooseSource { card_id, page_id })
            }
            Hit::FolderChoose => {
                let card_id = self.selected_card_ref()?.id.clone();
                let page_id = self.selected_page_ref()?.id.clone();
                Some(Action::ChooseFolder { card_id, page_id })
            }
            Hit::FolderSort => {
                if let Some(page) = self.selected_page_mut() {
                    page.folder.sort = match page.folder.sort {
                        mochi_core::desktop_cards::folder::FolderSort::Name => {
                            mochi_core::desktop_cards::folder::FolderSort::Modified
                        }
                        mochi_core::desktop_cards::folder::FolderSort::Modified => {
                            mochi_core::desktop_cards::folder::FolderSort::Type
                        }
                        mochi_core::desktop_cards::folder::FolderSort::Type => {
                            mochi_core::desktop_cards::folder::FolderSort::Manual
                        }
                        mochi_core::desktop_cards::folder::FolderSort::Manual => {
                            mochi_core::desktop_cards::folder::FolderSort::Name
                        }
                    };
                    self.dirty = true;
                }
                None
            }
            Hit::FolderToggleHidden => {
                if let Some(page) = self.selected_page_mut() {
                    page.folder.show_hidden = !page.folder.show_hidden;
                    self.dirty = true;
                }
                None
            }
            Hit::Route(route) => {
                if let Some(page) = self.selected_page_mut() {
                    page.interaction = route;
                    self.dirty = true;
                }
                None
            }
            Hit::Save => self.save_action(),
            Hit::SaveKeepOpen => self.save_action().map(|a| match a {
                Action::Save(c) => Action::SaveKeepOpen(c),
                other => other,
            }),
            Hit::ConfirmDelete | Hit::CancelDelete | Hit::ConfirmCancel | Hit::DismissCancel => {
                None
            }
        }
    }

    fn save_action(&mut self) -> Option<Action> {
        self.commit_focused_field();
        if let Some(error) = self.validate() {
            self.error = Some(error);
            return None;
        }
        self.error = None;
        Some(Action::Save(self.config.clone()))
    }

    fn validate(&self) -> Option<String> {
        for card in &self.config.cards {
            if card.title.trim().is_empty() {
                return Some("请为每张卡片填写名称".into());
            }
            if card.pages.is_empty() {
                return Some(format!("「{}」至少需要一个分页", card.title));
            }
            for page in &card.pages {
                if page.title.trim().is_empty() {
                    return Some(format!("「{}」有分页尚未命名", card.title));
                }
            }
        }
        None
    }

    fn add_card(&mut self) {
        let id = self.new_unique_id("card");
        let mut card = DesktopCard::new(
            format!("桌面卡片 {}", self.config.cards.len() + 1),
            ModuleKind::Home,
        );
        card.id = id;
        self.config.cards.push(card);
        self.selected_card = Some(self.config.cards.len() - 1);
        self.selected_page = Some(0);
        self.dirty = true;
        self.reset_scrolls();
        self.sync_editors();
    }

    fn add_page(&mut self) {
        let Some(card_index) = self.selected_card else {
            return;
        };
        let id = self.new_unique_id("page");
        let number = self
            .config
            .cards
            .get(card_index)
            .map(|c| c.pages.len() + 1)
            .unwrap_or(1);
        let mut page = DesktopPage::new(ModuleKind::Recent);
        page.id = id;
        page.title = format!("分页 {number}");
        if let Some(card) = self.config.cards.get_mut(card_index) {
            card.pages.push(page);
            self.selected_page = Some(card.pages.len() - 1);
            if let Some(page) = card.pages.last() {
                card.active_page = page.id.clone();
            }
            self.dirty = true;
        }
        self.sync_editors();
    }

    fn duplicate_card(&mut self, index: usize) {
        if self.config.cards.len() >= mochi_core::desktop_cards::MAX_CARDS {
            self.error = Some(format!(
                "最多只能保留 {} 张卡片",
                mochi_core::desktop_cards::MAX_CARDS
            ));
            return;
        }
        let Some(original) = self.config.cards.get(index).cloned() else {
            return;
        };
        let mut copy = original.clone();
        copy.id = self.new_unique_id("card");
        copy.title = bounded_name(&format!("{} · 副本", original.title));
        copy.x = copy.x.saturating_add(24);
        copy.y = copy.y.saturating_add(24);
        let mut ids = std::collections::BTreeMap::new();
        for page in &mut copy.pages {
            let old = page.id.clone();
            page.id = self.new_unique_id("page");
            ids.insert(old, page.id.clone());
        }
        mochi_core::desktop_cards::remap_page_events(&mut copy, &ids);
        copy.active_page = copy
            .pages
            .first()
            .map(|page| page.id.clone())
            .unwrap_or_default();
        let insert_at = (index + 1).min(self.config.cards.len());
        self.config.cards.insert(insert_at, copy);
        self.selected_card = Some(insert_at);
        self.selected_page = self
            .config
            .cards
            .get(insert_at)
            .and_then(|card| (!card.pages.is_empty()).then_some(0));
        self.dirty = true;
        self.error = None;
        self.reset_scrolls();
        self.sync_editors();
    }

    fn duplicate_page(&mut self, index: usize) {
        let Some(card_index) = self.selected_card else {
            return;
        };
        let Some((page_count, original)) = self
            .config
            .cards
            .get(card_index)
            .map(|card| (card.pages.len(), card.pages.get(index).cloned()))
        else {
            return;
        };
        if page_count >= mochi_core::desktop_cards::MAX_PAGES_PER_CARD {
            self.error = Some(format!(
                "每张卡片最多只能有 {} 个分页",
                mochi_core::desktop_cards::MAX_PAGES_PER_CARD
            ));
            return;
        }
        let Some(original) = original else {
            return;
        };
        let mut copy = original;
        copy.id = self.new_unique_id("page");
        copy.title = bounded_name(&format!("{} · 副本", copy.title));
        let insert_at = (index + 1).min(page_count);
        if let Some(card) = self.config.cards.get_mut(card_index) {
            card.pages.insert(insert_at, copy);
            card.active_page = card.pages[insert_at].id.clone();
        }
        self.selected_page = Some(insert_at);
        self.dirty = true;
        self.error = None;
        self.sync_editors();
    }

    fn delete_confirmed(&mut self) {
        match self.confirm_delete.take() {
            Some(DeleteTarget::Card(i)) if i < self.config.cards.len() => {
                self.config.cards.remove(i);
                self.selected_card = if self.config.cards.is_empty() {
                    None
                } else {
                    Some(i.min(self.config.cards.len() - 1))
                };
                self.selected_page = self.selected_card_ref().and_then(|c| {
                    c.pages
                        .iter()
                        .position(|page| page.id == c.active_page)
                        .or_else(|| (!c.pages.is_empty()).then_some(0))
                });
                self.dirty = true;
                self.sync_editors();
            }
            Some(DeleteTarget::Page(i)) => {
                let Some(card_index) = self.selected_card else {
                    return;
                };
                let Some(card) = self.config.cards.get_mut(card_index) else {
                    return;
                };
                if card.pages.len() > 1 && i < card.pages.len() {
                    card.pages.remove(i);
                    self.selected_page = Some(i.min(card.pages.len() - 1));
                    if let Some(page) = self.selected_page.and_then(|index| card.pages.get(index)) {
                        card.active_page = page.id.clone();
                    }
                    self.dirty = true;
                    self.sync_editors();
                } else {
                    self.error = Some("每张卡片至少保留一个分页".into());
                }
            }
            _ => {}
        }
    }

    pub fn select_card(&mut self, index: usize) {
        self.studio_editor = Default::default();
        if self.editor_tab == 2 {
            self.editor_tab = 1;
        }
        self.creating = None;
        self.focus_field = None;
        if index >= self.config.cards.len() {
            return;
        }
        self.commit_focused_field();
        self.selected_card = Some(index);
        self.selected_page = self.config.cards[index]
            .pages
            .iter()
            .position(|page| page.id == self.config.cards[index].active_page)
            .or_else(|| (!self.config.cards[index].pages.is_empty()).then_some(0));
        self.focus_field = None;
        self.reset_scrolls();
        self.sync_editors();
    }

    pub fn select_page(&mut self, index: usize) {
        self.studio_editor = Default::default();
        if self.editor_tab == 2 {
            self.editor_tab = 1;
        }
        if self
            .selected_card_ref()
            .is_some_and(|card| index < card.pages.len())
        {
            self.commit_focused_field();
            self.selected_page = Some(index);
            if let Some(card) = self.selected_card_mut() {
                if let Some(page) = card.pages.get(index) {
                    card.active_page = page.id.clone();
                }
            }
            self.focus_field = None;
            self.sync_editors();
        }
    }

    fn move_card(&mut self, index: usize, delta: i32) {
        let Some(next) = checked_move(index, delta, self.config.cards.len()) else {
            return;
        };
        self.config.cards.swap(index, next);
        self.selected_card = Some(next);
        self.dirty = true;
        self.sync_editors();
    }

    fn move_page(&mut self, index: usize, delta: i32) {
        let Some(card_index) = self.selected_card else {
            return;
        };
        let Some(card) = self.config.cards.get_mut(card_index) else {
            return;
        };
        let Some(next) = checked_move(index, delta, card.pages.len()) else {
            return;
        };
        card.pages.swap(index, next);
        self.selected_page = Some(next);
        if let Some(page) = card.pages.get(next) {
            card.active_page = page.id.clone();
        }
        self.dirty = true;
        self.sync_editors();
    }

    fn move_focus(&mut self, backwards: bool) {
        self.commit_focused_field();
        let order = self.focus_order();
        if order.is_empty() {
            self.focused = None;
            return;
        }
        let current = self.focus_target();
        let index =
            current.and_then(|target| order.iter().position(|candidate| *candidate == target));
        let next = match (index, backwards) {
            (Some(i), true) => (i + order.len() - 1) % order.len(),
            (Some(i), false) => (i + 1) % order.len(),
            (None, true) => order.len() - 1,
            (None, false) => 0,
        };
        self.set_focus_target(order[next]);
    }

    fn focus_order(&self) -> Vec<FocusTarget> {
        if self.confirm_cancel {
            return vec![
                FocusTarget::Hit(Hit::DismissCancel),
                FocusTarget::Hit(Hit::ConfirmCancel),
            ];
        }
        if self.confirm_delete.is_some() {
            return vec![
                FocusTarget::Hit(Hit::CancelDelete),
                FocusTarget::Hit(Hit::ConfirmDelete),
            ];
        }
        let mut order = vec![
            FocusTarget::Hit(Hit::NewCard),
            FocusTarget::Hit(Hit::Import),
            FocusTarget::Hit(Hit::ExportAll),
        ];
        for i in 0..self.config.cards.len() {
            order.push(FocusTarget::Hit(Hit::Card(i)));
            order.push(FocusTarget::Hit(Hit::CardDuplicate(i)));
        }
        if self.creating.is_some() {
            for module in ModuleKind::ALL {
                order.push(FocusTarget::Hit(Hit::Module(*module)));
            }
        } else if self.selected_card.is_some() {
            order.push(FocusTarget::Field(Field::CardName));
            order.extend([
                FocusTarget::Hit(Hit::EditorTab(0)),
                FocusTarget::Hit(Hit::EditorTab(1)),
            ]);
            if studio::enabled(self) {
                order.push(FocusTarget::Hit(Hit::EditorTab(2)));
            }
            let controls = if self.editor_tab == 2 {
                studio::controls(self, Rect::from_size(0.0, 0.0, 800.0, 700.0))
            } else {
                self.layout(Rect::from_size(0.0, 0.0, 1200.0, 900.0))
                    .controls
            };
            for (_, hit) in controls {
                if let Hit::EditPreference(field) = hit {
                    order.push(FocusTarget::Field(field));
                    continue;
                }
                if matches!(
                    hit,
                    Hit::OpacityTrack(_)
                        | Hit::FontSizeTrack(_)
                        | Hit::FontColor(_)
                        | Hit::FontCustom
                        | Hit::BackgroundColor(_)
                        | Hit::BackgroundCustom
                        | Hit::Spacing(_)
                        | Hit::ToggleDetails
                        | Hit::ToggleBorder
                        | Hit::ToggleSeparators
                        | Hit::Page(_)
                        | Hit::AddPage
                        | Hit::Option(_)
                        | Hit::PreferencesTab(_)
                        | Hit::Preference(..)
                        | Hit::FolderChoose
                        | Hit::FolderSort
                        | Hit::FolderToggleHidden
                        | Hit::AddGroup
                        | Hit::RemoveGroup(_)
                        | Hit::Studio(_)
                        | Hit::Route(_)
                        | Hit::LimitAll
                        | Hit::LimitUp
                        | Hit::LimitDown
                        | Hit::Source
                ) {
                    order.push(FocusTarget::Hit(hit));
                }
            }
        }

        order.push(FocusTarget::Hit(Hit::SaveKeepOpen));
        order.push(FocusTarget::Hit(Hit::Save));
        order.push(FocusTarget::Hit(Hit::Cancel));
        order
    }

    fn focus_target(&self) -> Option<FocusTarget> {
        self.focus_field
            .map(FocusTarget::Field)
            .or_else(|| self.focused.map(FocusTarget::Hit))
    }

    fn set_focus_target(&mut self, target: FocusTarget) {
        match target {
            FocusTarget::Field(field) => {
                let hit = match field {
                    Field::CardName => Hit::CardName,
                    Field::PageName => Hit::PageName,
                    other => Hit::EditPreference(other),
                };
                if !matches!(field, Field::CardName | Field::PageName) {
                    self.activate(hit);
                } else {
                    self.focus_field = Some(field);
                    self.sync_editors();
                }
                self.focused = Some(hit);
            }
            FocusTarget::Hit(hit) => {
                self.focus_field = None;
                self.focused = Some(hit);
            }
        }
    }

    fn sync_editors(&mut self) {
        if let Some(title) = self.selected_card_ref().map(|card| card.title.clone()) {
            if self.card_name.text() != title {
                self.card_name.set_text(&title);
            }
        } else {
            self.card_name.clear();
        }
        if let Some(title) = self.selected_page_ref().map(|page| page.title.clone()) {
            if self.page_name.text() != title {
                self.page_name.set_text(&title);
            }
        } else {
            self.page_name.clear();
        }
    }

    fn reset_scrolls(&mut self) {
        self.cards_scroll = 0.0;
        self.modules_scroll = 0.0;
        self.options_scroll = 0.0;
        self.page_scroll = 0.0;
        self.scrollbar = ScrollInteraction::default();
    }

    fn max_id(&self) -> u64 {
        self.config
            .cards
            .iter()
            .flat_map(|c| std::iter::once(&c.id).chain(c.pages.iter().map(|p| &p.id)))
            .filter_map(|id| id.rsplit('-').next()?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
    }

    fn new_id(&mut self, prefix: &str) -> String {
        let id = format!("{prefix}-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn new_unique_id(&mut self, prefix: &str) -> String {
        loop {
            let id = self.new_id(prefix);
            let used = self
                .config
                .cards
                .iter()
                .any(|card| card.id == id || card.pages.iter().any(|page| page.id == id));
            if !used {
                return id;
            }
        }
    }

    pub fn bars(&self, layout: &Layout) -> Vec<(ScrollTarget, Bar)> {
        let mut bars = Vec::new();
        if let Some(bar) = Bar::new(
            layout.cards_body,
            Axis::Vertical,
            layout.cards_max_scroll,
            self.cards_scroll,
            false,
        ) {
            bars.push((ScrollTarget::Cards, bar));
        }
        if let Some(bar) = Bar::new(
            layout.modules_body,
            Axis::Vertical,
            layout.modules_max_scroll,
            self.modules_scroll,
            false,
        ) {
            bars.push((ScrollTarget::Modules, bar));
        }
        if let Some(bar) = Bar::new(
            layout.options_view,
            Axis::Vertical,
            layout.options_max_scroll,
            self.options_scroll,
            false,
        ) {
            bars.push((ScrollTarget::Options, bar));
        }
        bars
    }
}

fn checked_move(index: usize, delta: i32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let next = index as i32 + delta;
    (next >= 0 && next < len as i32).then_some(next as usize)
}

fn bounded_name(s: &str) -> String {
    let mut chars = s.trim().chars();
    let mut out: String = chars.by_ref().take(64).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}
