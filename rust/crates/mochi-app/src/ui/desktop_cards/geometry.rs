//! 计算桌面卡片行、模块和设置项的尺寸与位置。
use super::{
    model::{ModuleKind, PageExt, ROUTES},
    CardSizePreset, Hit, State,
};
use crate::ui::layout::{Edges, Rect};

pub const CARD_ROW_HEIGHT: f32 = 82.0;
pub const MODULE_ROW_HEIGHT: f32 = 76.0;
pub const OPTION_ROW_HEIGHT: f32 = 52.0;

pub(super) fn size_preset_rect(body: Rect, index: usize) -> Rect {
    let width = ((body.width() - 72.0) / 3.0).clamp(1.0, 100.0);
    Rect::from_size(
        body.left + 60.0 + index as f32 * (width + 6.0),
        body.top + 86.0,
        width,
        24.0,
    )
}

/// 每张侧栏卡片的绘制与命中测试共用的几何契约。
pub(super) struct CardRowLayout {
    pub title: Rect,
    pub status: Rect,
    pub controls: Vec<(Rect, Hit)>,
}
pub(super) fn card_row_layout(row: Rect, index: usize) -> CardRowLayout {
    let title = Rect::new(
        row.left + 46.0,
        row.top + 8.0,
        row.right - 64.0,
        row.top + 28.0,
    );
    let status = Rect::new(
        row.left + 46.0,
        row.top + 32.0,
        row.right - 10.0,
        row.top + 48.0,
    );
    let mut controls = vec![
        (
            Rect::from_size(row.right - 58.0, row.top + 8.0, 24.0, 24.0),
            Hit::CardLock(index),
        ),
        (
            Rect::from_size(row.right - 30.0, row.top + 8.0, 24.0, 24.0),
            Hit::CardVisible(index),
        ),
    ];
    // 操作按钮在悬停时以覆盖层形式渲染在行的底部。
    let cell = (row.width() - 16.0) / 6.0;
    let size = 22.0_f32.min(cell - 2.0);
    for (i, hit) in [
        Hit::CardLocate(index),
        Hit::CardExport(index),
        Hit::CardDuplicate(index),
        Hit::CardUp(index),
        Hit::CardDown(index),
        Hit::CardDelete(index),
    ]
    .into_iter()
    .enumerate()
    {
        controls.push((
            Rect::from_size(
                row.left + 8.0 + i as f32 * cell + (cell - size) / 2.0,
                row.bottom - 26.0,
                size,
                22.0,
            ),
            hit,
        ));
    }
    CardRowLayout {
        title,
        status,
        controls,
    }
}

#[derive(Debug, Clone)]
pub struct Layout {
    toolbar_cards: Vec<usize>,
    pub modal: bool,
    pub appearance_open: bool,
    pub viewport: Rect,
    pub frame: Rect,
    pub header: Rect,
    pub cards_pane: Rect,
    pub cards_body: Rect,
    pub right_pane: Rect,
    pub right_body: Rect,
    pub studio_canvas: Rect,
    pub card_name: Rect,
    pub page_name: Rect,
    pub page_tabs: Rect,
    pub modules_body: Rect,
    pub options_body: Rect,
    pub options_view: Rect,
    pub options_footer_top: f32,
    pub footer: Rect,
    pub controls: Vec<(Rect, Hit)>,
    pub card_rows: Vec<(Rect, usize)>,
    pub page_rows: Vec<(Rect, usize)>,
    pub module_rows: Vec<(Rect, ModuleKind)>,
    pub option_rows: Vec<(Rect, usize)>,
    pub cards_max_scroll: f32,
    pub modules_max_scroll: f32,
    pub options_max_scroll: f32,
    pub pages_max_scroll: f32,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        // 控件按视觉顺序追加。显式控件优先于整行的宽泛命中框
        // （删除/可见性按钮都在卡片行内部）。
        if let Some((_, hit)) = self
            .controls
            .iter()
            .rev()
            .find(|(r, hit)| r.contains(x, y) && self.control_visible(*hit, x, y))
        {
            // 滑轨命中需要把点击位置换算成百分比。
            return Some(match hit {
                Hit::OpacityTrack(_) => {
                    Hit::OpacityTrack(super::appearance::track_hit_value(self.right_body, 0, x))
                }
                Hit::FontSizeTrack(_) => {
                    Hit::FontSizeTrack(super::appearance::track_hit_value(self.right_body, 1, x))
                }
                other => *other,
            });
        }
        if self.modal {
            return None;
        }
        if self.cards_body.contains(x, y) {
            if let Some((_, index)) = self.card_rows.iter().find(|(r, _)| r.contains(x, y)) {
                return Some(Hit::Card(*index));
            }
        }
        if self.page_tabs.contains(x, y) && x < self.page_tabs.right - 102.0 {
            if let Some((_, index)) = self.page_rows.iter().find(|(r, _)| r.contains(x, y)) {
                return Some(Hit::Page(*index));
            }
        }
        if self.modules_body.contains(x, y) {
            if let Some((_, module)) = self.module_rows.iter().find(|(r, _)| r.contains(x, y)) {
                return Some(Hit::Module(*module));
            }
        }
        (self.viewport.contains(x, y) && !self.frame.contains(x, y)).then_some(Hit::Backdrop)
    }

    fn control_visible(&self, hit: Hit, x: f32, y: f32) -> bool {
        match hit {
            Hit::Card(_) => self.cards_body.contains(x, y),
            Hit::CardDelete(i)
            | Hit::CardUp(i)
            | Hit::CardDown(i)
            | Hit::CardExport(i)
            | Hit::CardLocate(i)
            | Hit::CardDuplicate(i) => {
                self.toolbar_cards.contains(&i)
                    && self.cards_body.contains(x, y)
                    && self
                        .card_rows
                        .iter()
                        .any(|(row, index)| *index == i && row.contains(x, y))
            }
            Hit::CardVisible(_) | Hit::CardLock(_) => {
                self.cards_body.contains(x, y) || self.right_body.contains(x, y)
            }
            Hit::Page(_) | Hit::PageDelete(_) | Hit::PageName => {
                self.page_tabs.contains(x, y) && x < self.page_tabs.right - 102.0
            }
            Hit::Module(_) => self.modules_body.contains(x, y),
            Hit::Studio(super::studio::Command::Select(_) | super::studio::Command::Resize(_)) => {
                self.studio_canvas.contains(x, y)
            }
            Hit::Studio(super::studio::Command::DismissPicker) => self.right_body.contains(x, y),
            Hit::Studio(_) if x >= super::studio::inspector(self.right_body).left => {
                let r = super::studio::inspector(self.right_body);
                r.contains(x, y) && y >= r.top + 32.0
            }
            Hit::EditPreference(super::Field::Studio(_)) => {
                let r = super::studio::inspector(self.right_body);
                r.contains(x, y) && y >= r.top + 32.0
            }
            Hit::ShortcutSelect(_)
            | Hit::ShortcutDelete
            | Hit::ShortcutMove(_)
            | Hit::ItemColor(..)
            | Hit::ResetItemColor(..)
            | Hit::EditPreference(
                super::Field::ShortcutName(_)
                | super::Field::ShortcutTarget(_)
                | super::Field::FolderPath
                | super::Field::FolderFilter
                | super::Field::Utility,
            )
            | Hit::FolderChoose
            | Hit::FolderSort
            | Hit::FolderToggleHidden
            | Hit::Option(_)
            | Hit::Preference(..)
            | Hit::AddGroup
            | Hit::RemoveGroup(_) => self.options_view.contains(x, y),
            Hit::EditPreference(super::Field::GroupName(_) | super::Field::GroupRule(_)) => {
                self.options_view.contains(x, y)
            }
            Hit::Opacity(_)
            | Hit::OpacityTrack(_)
            | Hit::FontSize(_)
            | Hit::FontSizeTrack(_)
            | Hit::FontColor(_)
            | Hit::FontCustom
            | Hit::CardSize(_)
            | Hit::CardName => self.right_body.contains(x, y),
            Hit::LimitUp | Hit::LimitDown | Hit::Source | Hit::Route(_) => {
                self.options_body.contains(x, y)
            }
            _ => self.frame.contains(x, y),
        }
    }

    pub fn bars(
        &self,
        state: &State,
    ) -> Vec<(super::ScrollTarget, crate::ui::overlay_scrollbar::Bar)> {
        state.bars(self)
    }
}

pub fn layout(state: &State, viewport: Rect) -> Layout {
    let width = 1080.0_f32.min((viewport.width() - 16.0).max(1.0));
    let height = 800.0_f32.min((viewport.height() - 16.0).max(1.0));
    let frame = if state.embedded {
        viewport
    } else {
        Rect::from_size(
            (viewport.left + viewport.right - width) / 2.0,
            (viewport.top + viewport.bottom - height) / 2.0,
            width,
            height,
        )
    };
    let width = frame.width();
    let header = Rect::new(
        frame.left,
        frame.top,
        frame.right,
        (frame.top + 60.0).min(frame.bottom),
    );
    let footer = Rect::new(
        frame.left,
        (frame.bottom - 54.0).max(header.bottom),
        frame.right,
        frame.bottom,
    );
    let cards_width = if state.embedded {
        if width < 610.0 {
            180.0
        } else {
            212.0
        }
    } else if width < 610.0 {
        188.0
    } else {
        244.0
    };
    let cards_pane = Rect::new(
        frame.left,
        header.bottom,
        (frame.left + cards_width).min(frame.right),
        footer.top,
    );
    let right_pane = Rect::new(cards_pane.right, header.bottom, frame.right, footer.top);
    let cards_body = cards_pane.inset(Edges {
        left: 12.0,
        top: 50.0,
        right: 10.0,
        bottom: 54.0,
    });
    let right_body = right_pane.inset(Edges {
        left: 18.0,
        top: 12.0,
        right: 18.0,
        bottom: 12.0,
    });

    let mut controls = Vec::new();
    controls.push((
        Rect::from_size(frame.right - 42.0, frame.top + 16.0, 26.0, 26.0),
        Hit::Close,
    ));
    // 头部操作按钮在 880px 宽时保留文字标签，面板变窄时收缩为图标大小的
    // 命中框。绘制用的是同一批矩形。
    let toolbar_y = frame.top + 19.0;
    let toolbar_right = frame.right - 54.0;
    let import_w = if width >= 640.0 { 62.0 } else { 32.0 };
    let export_w = if width >= 640.0 { 84.0 } else { 32.0 };
    let new_w = if width >= 640.0 { 78.0 } else { 32.0 };
    controls.push((
        Rect::from_size(
            toolbar_right - import_w - export_w - new_w - 16.0,
            toolbar_y,
            new_w,
            30.0,
        ),
        Hit::NewCard,
    ));
    controls.push((
        Rect::from_size(
            toolbar_right - import_w - export_w - 8.0,
            toolbar_y,
            import_w,
            30.0,
        ),
        Hit::Import,
    ));
    controls.push((
        Rect::from_size(toolbar_right - export_w, toolbar_y, export_w, 30.0),
        Hit::ExportAll,
    ));

    let mut card_rows = Vec::new();
    let card_content_height = state.config.cards.len() as f32 * CARD_ROW_HEIGHT;
    let cards_max_scroll = (card_content_height - cards_body.height()).max(0.0);
    let cards_scroll = state.cards_scroll.clamp(0.0, cards_max_scroll);
    for i in 0..state.config.cards.len() {
        let row = Rect::from_size(
            cards_body.left,
            cards_body.top + i as f32 * CARD_ROW_HEIGHT - cards_scroll,
            cards_body.width(),
            CARD_ROW_HEIGHT - 8.0,
        );
        card_rows.push((row, i));
        controls.push((row, Hit::Card(i)));
        controls.extend(card_row_layout(row, i).controls);
    }
    controls.push((
        Rect::from_size(cards_pane.right - 38.0, cards_pane.top + 18.0, 24.0, 24.0),
        Hit::NewCard,
    ));
    controls.push((
        Rect::from_size(
            cards_pane.left + 12.0,
            cards_pane.bottom - 42.0,
            cards_pane.width() - 24.0,
            30.0,
        ),
        Hit::HideAll,
    ));

    let appearance_open = state.editor_tab == 0;
    let section_top = if state.embedded { 62.0 } else { 86.0 };
    if state.selected_card.is_some() && state.creating.is_none() {
        for i in 0..if super::studio::enabled(state) { 3 } else { 2 } {
            controls.push((
                Rect::from_size(
                    right_body.left + i as f32 * 88.0,
                    right_body.top + 46.0,
                    80.0,
                    28.0,
                ),
                Hit::EditorTab(i),
            ));
        }
    }
    let mut page_rows = Vec::new();
    let page_tabs = Rect::new(
        right_body.left,
        right_body.top + section_top + 24.0,
        right_body.right,
        (right_body.top + section_top + 58.0).min(right_body.bottom),
    );
    let page_count = state.selected_card_ref().map_or(0, |c| c.pages.len());
    let page_cell = if page_count == 0 {
        0.0
    } else {
        ((page_tabs.width() - 102.0) / page_count as f32).clamp(78.0, 142.0)
    };
    let pages_content_width = page_count as f32 * page_cell;
    let pages_max_scroll = (pages_content_width - (page_tabs.width() - 102.0)).max(0.0);
    let page_scroll = state.page_scroll.clamp(0.0, pages_max_scroll);
    for i in 0..page_count {
        let rect = Rect::from_size(
            page_tabs.left + i as f32 * page_cell - page_scroll,
            page_tabs.top + 2.0,
            (page_cell - 4.0).max(1.0),
            32.0,
        );
        page_rows.push((rect, i));
        controls.push((rect, Hit::Page(i)));
        controls.push((
            Rect::from_size(rect.right - 25.0, rect.top + 7.0, 18.0, 18.0),
            Hit::PageDelete(i),
        ));
    }
    controls.push((
        Rect::from_size(page_tabs.right - 34.0, page_tabs.top + 2.0, 30.0, 32.0),
        Hit::AddPage,
    ));

    if let Some(i) = state.selected_page {
        controls.push((
            Rect::from_size(page_tabs.right - 98.0, page_tabs.top + 2.0, 28.0, 32.0),
            if pages_max_scroll > 0.0 {
                Hit::PageScroll(-1)
            } else {
                Hit::PageUp(i)
            },
        ));
        controls.push((
            Rect::from_size(page_tabs.right - 66.0, page_tabs.top + 2.0, 28.0, 32.0),
            if pages_max_scroll > 0.0 {
                Hit::PageScroll(1)
            } else {
                Hit::PageDown(i)
            },
        ));
    }
    let card_name = Rect::new(
        right_body.left,
        right_body.top + 8.0,
        (right_body.left + (right_body.width() * 0.52).max(120.0)).min(right_body.right - 80.0),
        right_body.top + 42.0,
    );
    if state.selected_card.is_some() {
        controls.push((card_name, Hit::CardName));
    }

    let module_top = if state.creating.is_some() {
        right_body.top + 54.0
    } else {
        right_body.top
            + if appearance_open {
                300.0
            } else {
                section_top + 68.0
            }
    };
    let modules_body = if state.creating.is_some() {
        Rect::new(
            right_body.left,
            module_top,
            right_body.right,
            right_body.bottom,
        )
    } else {
        Rect::default()
    };
    let mut options_body = Rect::new(
        right_body.left,
        module_top,
        right_body.right,
        right_body.bottom,
    );
    let page_name = page_rows
        .iter()
        .find(|(_, i)| Some(*i) == state.selected_page)
        .map(|(r, _)| Rect::new(r.left, r.top, r.right - 25.0, r.bottom))
        .unwrap_or_default();
    if state.focus_field == Some(super::Field::PageName) {
        controls.push((page_name, Hit::PageName));
    }
    if appearance_open && state.creating.is_none() && state.selected_card.is_some() {
        controls.extend(super::appearance::controls(Rect::new(
            right_body.left,
            right_body.top + 40.0,
            right_body.right,
            right_body.bottom,
        )));
    }

    let modules = ModuleKind::ALL;
    let module_cols = if state.embedded && modules_body.width() >= 850.0 {
        4
    } else if modules_body.width() >= 650.0 {
        3
    } else if modules_body.width() >= 300.0 {
        2
    } else {
        1
    };
    let module_cell_w =
        ((modules_body.width() - (module_cols as f32 - 1.0) * 6.0) / module_cols as f32).max(1.0);
    let module_content_height = modules.len().div_ceil(module_cols) as f32 * MODULE_ROW_HEIGHT;
    let modules_max_scroll = (module_content_height - modules_body.height()).max(0.0);
    let modules_scroll = state.modules_scroll.clamp(0.0, modules_max_scroll);
    let mut module_rows = Vec::new();
    for (i, module) in modules
        .iter()
        .copied()
        .enumerate()
        .filter(|_| state.creating.is_some())
    {
        let row = Rect::from_size(
            modules_body.left + (i % module_cols) as f32 * (module_cell_w + 6.0),
            modules_body.top + (i / module_cols) as f32 * MODULE_ROW_HEIGHT - modules_scroll,
            module_cell_w,
            MODULE_ROW_HEIGHT - 6.0,
        );
        module_rows.push((row, module));
        controls.push((row, Hit::Module(module)));
    }

    for (i, mode) in [false, true].into_iter().enumerate() {
        controls.push((
            Rect::from_size(
                options_body.left + i as f32 * 88.0,
                options_body.top,
                80.0,
                28.0,
            ),
            Hit::PreferencesTab(mode),
        ));
    }
    let field_count = state.selected_page_ref().map_or(0, |p| {
        if !appearance_open && !state.preferences_mode && super::model::interactive(p.module) {
            0
        } else if state.preferences_mode || appearance_open {
            super::preferences::count(state)
        } else {
            super::model::fields_for(p.module).len()
        }
    });
    let has_source = state.selected_page_ref().is_some_and(|p| p.has_source());
    // 选项列表滚动时，行数上限、来源选择器和点击行为这几项要保持可见。
    // 它们是用户在这个面板上做的最终决定，不应被过长的模板盖住。
    let options_fixed_height = if appearance_open {
        0.0
    } else if has_source {
        146.0
    } else {
        106.0
    };
    let options_footer_top = (options_body.bottom - options_fixed_height).max(options_body.top);
    let mut options_view = Rect::new(
        options_body.left,
        (options_body.top + 34.0).min(options_footer_top),
        options_body.right,
        options_footer_top,
    );
    let option_columns = if !state.preferences_mode
        && !appearance_open
        && options_body.width() >= 520.0
        && state
            .selected_page_ref()
            .is_some_and(|p| p.module == ModuleKind::Home)
    {
        2
    } else {
        1
    };
    let mut options_max_scroll = (field_count.div_ceil(option_columns) as f32 * OPTION_ROW_HEIGHT
        - options_view.height())
    .max(0.0);
    let options_scroll = state.options_scroll.clamp(0.0, options_max_scroll);
    let mut option_rows = Vec::new();
    let option_top = options_view.top - options_scroll;
    for i in 0..field_count {
        let row = Rect::from_size(
            options_body.left
                + (i % option_columns) as f32 * (options_body.width() + 10.0)
                    / option_columns as f32,
            option_top + (i / option_columns) as f32 * OPTION_ROW_HEIGHT,
            (options_body.width() - 10.0 * (option_columns - 1) as f32) / option_columns as f32,
            OPTION_ROW_HEIGHT - 6.0,
        );
        option_rows.push((row, i));
        let option_count = if state.preferences_mode || appearance_open {
            0
        } else {
            state
                .selected_page_ref()
                .map_or(0, |p| super::model::fields_for(p.module).len())
        };
        if i < option_count {
            controls.push((row, Hit::Option(i)));
        } else {
            controls.extend(super::preferences::controls(state, row, i - option_count));
        }
    }
    let limit_top = options_footer_top + 8.0;
    controls.push((
        Rect::from_size(options_body.left, limit_top, options_body.width(), 32.0),
        Hit::EditPreference(super::Field::Limit),
    ));
    controls.push((
        Rect::from_size(options_body.right - 32.0, limit_top + 4.0, 24.0, 24.0),
        Hit::LimitUp,
    ));
    controls.push((
        Rect::from_size(options_body.right - 60.0, limit_top + 4.0, 24.0, 24.0),
        Hit::LimitDown,
    ));
    controls.push((
        Rect::from_size(options_body.right - 114.0, limit_top + 4.0, 48.0, 24.0),
        Hit::LimitAll,
    ));
    let source_top = limit_top + 40.0;
    if has_source {
        controls.push((
            Rect::from_size(
                options_body.left,
                source_top,
                options_body.width() - 34.0,
                30.0,
            ),
            Hit::Source,
        ));
    }
    if has_source {
        controls.push((
            Rect::from_size(options_body.right - 30.0, source_top, 30.0, 30.0),
            Hit::ClearSources,
        ));
    }
    let route_top = source_top + if has_source { 52.0 } else { 12.0 };
    let route_w = (options_body.width().min(320.0) / 2.0).max(1.0);
    for (i, route) in ROUTES.iter().copied().enumerate() {
        controls.push((
            Rect::from_size(
                options_body.left + i as f32 * route_w,
                route_top,
                route_w,
                28.0,
            ),
            Hit::Route(route),
        ));
    }

    for preset in CardSizePreset::ALL {
        // 尺寸选项的布局在 painting.rs 里相对选中卡片的头部排布；
        // 命中矩形也放在同一处。
        let index = CardSizePreset::ALL
            .iter()
            .position(|p| *p == preset)
            .unwrap_or(0);
        if state.selected_card.is_some() {
            controls.push((size_preset_rect(right_body, index), Hit::CardSize(preset)));
        }
    }
    if let Some(i) = state.selected_card {
        controls.push((
            Rect::from_size(card_name.right + 10.0, card_name.top, 30.0, 30.0),
            Hit::CardVisible(i),
        ));
        controls.push((
            Rect::from_size(card_name.right + 46.0, card_name.top, 30.0, 30.0),
            Hit::CardLock(i),
        ));
    }
    controls.push((
        Rect::from_size(frame.right - 292.0, footer.top + 13.0, 82.0, 28.0),
        Hit::Cancel,
    ));
    controls.push((
        Rect::from_size(frame.right - 104.0, footer.top + 13.0, 82.0, 28.0),
        Hit::Save,
    ));
    controls.push((
        Rect::from_size(frame.right - 198.0, footer.top + 13.0, 82.0, 28.0),
        Hit::SaveKeepOpen,
    ));
    if appearance_open {
        controls.retain(|(_, h)| {
            !matches!(
                h,
                Hit::Page(_)
                    | Hit::PageDelete(_)
                    | Hit::PageUp(_)
                    | Hit::PageDown(_)
                    | Hit::PageName
                    | Hit::AddPage
                    | Hit::PreferencesTab(_)
                    | Hit::Option(_)
                    | Hit::LimitUp
                    | Hit::LimitDown
                    | Hit::LimitAll
                    | Hit::EditPreference(super::Field::Limit)
                    | Hit::Source
                    | Hit::ClearSources
                    | Hit::Route(_)
            )
        });
        page_rows.clear();
    } else {
        controls.retain(|(_, h)| !matches!(h, Hit::CardSize(_)));
    }
    if state.editor_tab == 1
        && state
            .selected_page_ref()
            .is_some_and(|p| super::model::interactive(p.module))
    {
        controls.retain(|(_, h)| {
            !matches!(
                h,
                Hit::Option(_)
                    | Hit::LimitUp
                    | Hit::LimitDown
                    | Hit::LimitAll
                    | Hit::EditPreference(super::Field::Limit)
                    | Hit::Source
                    | Hit::ClearSources
                    | Hit::Route(_)
            )
        });
        if !state.preferences_mode {
            option_rows.clear();
        }
    }
    if state.editor_tab == 1
        && state.creating.is_none()
        && state
            .selected_page_ref()
            .is_some_and(|p| p.module == ModuleKind::Shortcuts)
    {
        controls.retain(|(_, h)| {
            !matches!(
                h,
                Hit::LimitUp
                    | Hit::LimitDown
                    | Hit::LimitAll
                    | Hit::EditPreference(super::Field::Limit)
                    | Hit::Source
                    | Hit::Route(_)
                    | Hit::Option(_)
            )
        });
        options_view.bottom = options_body.bottom;
        options_max_scroll =
            (field_count as f32 * OPTION_ROW_HEIGHT - options_view.height()).max(0.0);
        if !state.preferences_mode {
            option_rows.clear();
            options_max_scroll =
                (super::shortcuts::height(state, options_view) - options_view.height()).max(0.0);
            controls.extend(super::shortcuts::controls(state, options_view));
        }
    }
    if state.editor_tab == 1
        && state.creating.is_none()
        && !state.preferences_mode
        && state
            .selected_page_ref()
            .is_some_and(|p| p.module == ModuleKind::Folder)
    {
        controls.retain(|(_, h)| {
            !matches!(
                h,
                Hit::LimitUp
                    | Hit::LimitDown
                    | Hit::LimitAll
                    | Hit::EditPreference(super::Field::Limit)
                    | Hit::Source
                    | Hit::ClearSources
                    | Hit::Route(_)
                    | Hit::Option(_)
            )
        });
        options_view.bottom = options_body.bottom;
        option_rows.clear();
        options_max_scroll =
            (super::folder::height(state, options_view) - options_view.height()).max(0.0);
        controls.extend(super::folder::controls(state, options_view));
    }
    if state
        .selected_page_ref()
        .is_some_and(|page| page.module == ModuleKind::Folder)
    {
        controls.retain(|(_, hit)| !matches!(hit, Hit::Route(_)));
    }
    if state.editor_tab == 1
        && state.creating.is_none()
        && !state.preferences_mode
        && state
            .selected_page_ref()
            .is_some_and(|p| super::utility::enabled(p.module))
    {
        controls.retain(|(_, h)| {
            !matches!(
                h,
                Hit::LimitUp
                    | Hit::LimitDown
                    | Hit::LimitAll
                    | Hit::EditPreference(super::Field::Limit)
                    | Hit::Source
                    | Hit::ClearSources
                    | Hit::Route(_)
                    | Hit::Option(_)
            )
        });
        options_view.bottom = options_body.bottom;
        option_rows.clear();
        options_max_scroll = 0.0;
        controls.extend(super::utility::controls(state, options_view));
    }
    if state.editor_tab == 2 && state.creating.is_none() {
        controls.retain(|(_, h)| {
            matches!(
                h,
                Hit::EditorTab(_)
                    | Hit::CardName
                    | Hit::Card(_)
                    | Hit::CardVisible(_)
                    | Hit::CardLock(_)
                    | Hit::CardDelete(_)
                    | Hit::CardLocate(_)
                    | Hit::CardExport(_)
                    | Hit::CardDuplicate(_)
                    | Hit::CardUp(_)
                    | Hit::CardDown(_)
                    | Hit::NewCard
                    | Hit::Import
                    | Hit::ExportAll
                    | Hit::HideAll
                    | Hit::Close
                    | Hit::Save
                    | Hit::SaveKeepOpen
                    | Hit::Cancel
            )
        });
        controls.extend(super::studio::controls(state, right_body));
        page_rows.clear();
        option_rows.clear();
        options_body = super::studio::inspector(right_body);
        options_view = options_body;
        options_max_scroll =
            (super::studio::content_height(state, right_body) - options_view.height()).max(0.0);
    }
    if state.embedded {
        controls.retain(|(_, h)| {
            !matches!(
                h,
                Hit::Close
                    | Hit::NewCard
                    | Hit::Import
                    | Hit::ExportAll
                    | Hit::Save
                    | Hit::SaveKeepOpen
                    | Hit::Cancel
            )
        });
        controls.extend([
            (
                Rect::from_size(frame.right - 172.0, frame.top + 16.0, 76.0, 30.0),
                Hit::NewCard,
            ),
            (
                Rect::from_size(frame.right - 88.0, frame.top + 16.0, 30.0, 30.0),
                Hit::Import,
            ),
            (
                Rect::from_size(frame.right - 50.0, frame.top + 16.0, 30.0, 30.0),
                Hit::ExportAll,
            ),
            (
                Rect::from_size(footer.right - 150.0, footer.top + 12.0, 30.0, 30.0),
                Hit::Cancel,
            ),
            (
                Rect::from_size(footer.right - 108.0, footer.top + 12.0, 88.0, 30.0),
                Hit::SaveKeepOpen,
            ),
        ]);
    }
    if state.confirm_delete.is_some() {
        controls.clear();
        let (_, cancel, confirm) = confirmation_rects(frame);
        controls.push((cancel, Hit::CancelDelete));
        controls.push((confirm, Hit::ConfirmDelete));
    }
    if state.confirm_cancel {
        controls.clear();
        let (_, cancel, confirm) = confirmation_rects(frame);
        controls.push((cancel, Hit::DismissCancel));
        controls.push((confirm, Hit::ConfirmCancel));
    }

    if state.creating.is_some() {
        controls.retain(|(_, h)| {
            matches!(
                h,
                Hit::NewCard
                    | Hit::Close
                    | Hit::Import
                    | Hit::ExportAll
                    | Hit::Card(_)
                    | Hit::HideAll
                    | Hit::Cancel
                    | Hit::Save
                    | Hit::SaveKeepOpen
                    | Hit::Module(_)
                    | Hit::ConfirmDelete
                    | Hit::CancelDelete
                    | Hit::ConfirmCancel
                    | Hit::DismissCancel
            )
        });
        page_rows.clear();
        option_rows.clear();
    }
    Layout {
        studio_canvas: super::studio::canvas(state, right_body),
        modal: state.confirm_cancel || state.confirm_delete.is_some(),
        appearance_open,
        toolbar_cards: (0..state.config.cards.len())
            .filter(|i| state.card_toolbar_visible(*i))
            .collect(),
        viewport,
        frame,
        header,
        cards_pane,
        cards_body,
        right_pane,
        right_body,
        card_name,
        page_name,
        page_tabs,
        modules_body,
        options_body,
        options_view,
        options_footer_top,
        footer,
        controls,
        card_rows,
        page_rows,
        module_rows,
        option_rows,
        cards_max_scroll,
        modules_max_scroll,
        options_max_scroll,
        pages_max_scroll,
    }
}

/// 确认弹窗的按钮与输入目标共用这份几何，保证绘制和命中完全一致。
pub(super) fn confirmation_rects(frame: Rect) -> (Rect, Rect, Rect) {
    let width = 440.0_f32.min(frame.width() - 32.0);
    let r = Rect::from_size(
        (frame.left + frame.right - width) / 2.0,
        (frame.top + frame.bottom - 184.0) / 2.0,
        width,
        184.0,
    );
    (
        r,
        Rect::from_size(r.right - 236.0, r.bottom - 56.0, 100.0, 34.0),
        Rect::from_size(r.right - 124.0, r.bottom - 56.0, 100.0, 34.0),
    )
}
