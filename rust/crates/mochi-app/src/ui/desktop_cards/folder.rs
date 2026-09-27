//! Configuration controls for a mapped local folder page.
use super::*;
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Edges,
    theme::Palette,
    widgets::FieldLook,
};
use mochi_core::desktop_cards::folder::FolderSort;

const INPUT_HEIGHT: f32 = 32.0;
const OPTION_HEIGHT: f32 = 46.0;

fn option_columns(area: Rect, count: usize) -> usize {
    if count > 1 && area.width() >= 520.0 {
        2
    } else {
        1
    }
}

fn path_rect(area: Rect) -> Rect {
    Rect::new(
        area.left,
        area.top + 20.0,
        area.right - 78.0,
        area.top + 20.0 + INPUT_HEIGHT,
    )
}

fn choose_rect(area: Rect) -> Rect {
    Rect::new(
        area.right - 72.0,
        area.top + 20.0,
        area.right,
        area.top + 20.0 + INPUT_HEIGHT,
    )
}

fn filter_rect(area: Rect) -> Rect {
    Rect::new(
        area.left,
        area.top + 77.0,
        area.right,
        area.top + 77.0 + INPUT_HEIGHT,
    )
}

fn sort_rect(area: Rect) -> Rect {
    Rect::new(
        area.left,
        area.top + 137.0,
        area.left + (area.width() * 0.5).min(170.0),
        area.top + 169.0,
    )
}

fn hidden_rect(area: Rect) -> Rect {
    Rect::new(
        area.left + area.width() * 0.5 + 8.0,
        area.top + 137.0,
        area.right,
        area.top + 169.0,
    )
}

fn options_top(area: Rect) -> f32 {
    area.top + 197.0
}

fn option_rect(area: Rect, index: usize, count: usize) -> Rect {
    let columns = option_columns(area, count);
    let width = (area.width() - (columns as f32 - 1.0) * 8.0) / columns as f32;
    let top = options_top(area) - 2.0 + (index / columns) as f32 * OPTION_HEIGHT;
    Rect::from_size(
        area.left + (index % columns) as f32 * (width + 8.0),
        top,
        width,
        40.0,
    )
}

fn options(_state: &State) -> Vec<mochi_core::desktop_cards::ModuleOption> {
    model::fields_for(ModuleKind::Folder)
}

pub(super) fn height(state: &State, area: Rect) -> f32 {
    let fields = options(state);
    let columns = option_columns(area, fields.len());
    options_top(area) - area.top + fields.len().div_ceil(columns) as f32 * OPTION_HEIGHT + 6.0
}

pub(super) fn controls(state: &State, area: Rect) -> Vec<(Rect, Hit)> {
    let top = area.top - state.options_scroll;
    let area = Rect::new(area.left, top, area.right, top + area.height());
    let mut controls = vec![
        (path_rect(area), Hit::EditPreference(Field::FolderPath)),
        (choose_rect(area), Hit::FolderChoose),
        (filter_rect(area), Hit::EditPreference(Field::FolderFilter)),
        (sort_rect(area), Hit::FolderSort),
        (hidden_rect(area), Hit::FolderToggleHidden),
    ];
    let fields = options(state);
    controls.extend(
        fields
            .iter()
            .enumerate()
            .map(|(index, _)| (option_rect(area, index, fields.len()), Hit::Option(index))),
    );
    controls
}

pub(super) fn paint(list: &mut DrawList, state: &mut State, area: Rect, p: &Palette) {
    let Some(page) = state.selected_page_ref().cloned() else {
        return;
    };
    let clip = area;
    let top = area.top - state.options_scroll;
    let area = Rect::new(area.left, top, area.right, top + area.height());
    list.push_clip(clip);

    list.text(
        Rect::new(area.left, area.top, area.right, area.top + 18.0),
        "文件夹路径",
        TextStyle::Caption,
        p.muted,
    );
    paint_input(
        list,
        state,
        path_rect(area),
        Field::FolderPath,
        &page.folder.path,
        "尚未选择文件夹",
        p,
    );
    paint_button(
        list,
        choose_rect(area),
        "选择…",
        state.hover == Some(Hit::FolderChoose),
        p,
    );

    list.text(
        Rect::new(area.left, area.top + 57.0, area.right, area.top + 75.0),
        "名称包含",
        TextStyle::Caption,
        p.muted,
    );
    paint_input(
        list,
        state,
        filter_rect(area),
        Field::FolderFilter,
        &page.folder.filter,
        "例如：.md 或 会议",
        p,
    );

    list.text(
        Rect::new(area.left, area.top + 116.0, area.right, area.top + 137.0),
        "排序与可见性",
        TextStyle::Caption,
        p.muted,
    );
    let sort = match page.folder.sort {
        FolderSort::Name => "名称",
        FolderSort::Modified => "修改时间",
        FolderSort::Type => "类型",
        FolderSort::Manual => "手动顺序",
    };
    paint_button(
        list,
        sort_rect(area),
        &format!("排序：{sort}"),
        state.hover == Some(Hit::FolderSort),
        p,
    );
    paint_toggle(
        list,
        hidden_rect(area),
        page.folder.show_hidden,
        "显示隐藏项",
        state.hover == Some(Hit::FolderToggleHidden),
        p,
    );

    let fields = options(state);
    if !fields.is_empty() {
        list.text(
            Rect::new(area.left, area.top + 174.0, area.right, area.top + 194.0),
            "收纳内容",
            TextStyle::Caption,
            p.muted,
        );
    }
    for (index, field) in fields.iter().enumerate() {
        let rect = option_rect(area, index, fields.len());
        let selected = page.selected(&field.key);
        list.rounded_rect(
            rect,
            5.0,
            if state.hover == Some(Hit::Option(index)) {
                p.surface_muted
            } else {
                p.surface
            },
        );
        list.rounded_border(rect, 5.0, p.border);
        let box_ = Rect::from_size(rect.left + 9.0, rect.top + 11.0, 18.0, 18.0);
        list.rounded_rect(box_, 4.0, if selected { p.accent } else { p.surface });
        list.rounded_border(box_, 4.0, if selected { p.accent } else { p.muted });
        if selected {
            list.icon_centered(box_, Icon::CHECK, 13.0, p.accent_foreground);
        }
        list.text(
            Rect::new(
                rect.left + 36.0,
                rect.top + 4.0,
                rect.right - 8.0,
                rect.top + 22.0,
            ),
            &field.label,
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 36.0,
                rect.top + 22.0,
                rect.right - 8.0,
                rect.bottom - 2.0,
            ),
            crate::ui::text::ellipsize(&field.description, TextStyle::Caption, rect.width() - 44.0),
            TextStyle::Caption,
            p.muted,
        );
    }
    list.pop_clip();
}

pub(super) fn paint_input(
    list: &mut DrawList,
    state: &mut State,
    rect: Rect,
    field: Field,
    value: &str,
    placeholder: &str,
    p: &Palette,
) {
    if state.focus_field == Some(field) {
        state
            .preference_text
            .paint(list, rect, true, p, FieldLook::dialog(p));
    } else {
        list.rounded_border(rect, 6.0, p.border);
        let value = if value.is_empty() { placeholder } else { value };
        list.text(
            rect.inset(Edges::xy(8.0, 0.0)),
            crate::ui::text::ellipsize(value, TextStyle::Label, rect.width() - 16.0),
            TextStyle::Label,
            if value == placeholder {
                p.muted
            } else {
                p.foreground
            },
        );
    }
}

fn paint_button(list: &mut DrawList, rect: Rect, label: &str, hovered: bool, p: &Palette) {
    if hovered {
        list.rounded_rect(rect, 6.0, p.surface_muted);
    }
    list.rounded_border(rect, 6.0, p.border);
    list.text_aligned(rect, label, TextStyle::Caption, p.foreground, Align::Center);
}

fn paint_toggle(
    list: &mut DrawList,
    rect: Rect,
    checked: bool,
    label: &str,
    hovered: bool,
    p: &Palette,
) {
    if hovered {
        list.rounded_rect(rect, 6.0, p.surface_muted);
    }
    let box_ = Rect::from_size(rect.left + 2.0, rect.top + 7.0, 18.0, 18.0);
    list.rounded_rect(box_, 4.0, if checked { p.accent } else { p.surface });
    list.rounded_border(box_, 4.0, if checked { p.accent } else { p.muted });
    if checked {
        list.icon_centered(box_, Icon::CHECK, 13.0, p.accent_foreground);
    }
    list.text(
        Rect::new(box_.right + 6.0, rect.top, rect.right, rect.bottom),
        label,
        TextStyle::Caption,
        p.foreground,
    );
}
