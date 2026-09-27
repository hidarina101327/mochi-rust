//! 独立的整理器编辑器，不依赖组合画布。
use super::*;
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Edges,
    theme::Palette,
    widgets::FieldLook,
};
fn button_columns(area: Rect) -> usize {
    (area.width() / 76.0).floor().max(1.0).min(7.0) as usize
}
fn header_height(area: Rect) -> f32 {
    160.0 + (7usize.div_ceil(button_columns(area)) - 1) as f32 * 34.0
}
fn tiles(state: &State, area: Rect) -> Vec<(Rect, usize)> {
    let Some(page) = state.selected_page_ref() else {
        return vec![];
    };
    let columns = page.presentation.columns.max(1) as usize;
    let height = page.presentation.grid_height.max(48) as f32;
    page.studio
        .nodes
        .iter()
        .enumerate()
        .map(|(i, _)| {
            (
                Rect::from_size(
                    area.left + (i % columns) as f32 * area.width() / columns as f32,
                    area.top + header_height(area) + (i / columns) as f32 * height
                        - state.options_scroll,
                    area.width() / columns as f32,
                    height,
                ),
                i,
            )
        })
        .collect()
}
pub(super) fn height(state: &State, area: Rect) -> f32 {
    state.selected_page_ref().map_or(header_height(area), |p| {
        header_height(area)
            + p.studio
                .nodes
                .len()
                .div_ceil(p.presentation.columns.max(1) as usize) as f32
                * p.presentation.grid_height.max(48) as f32
    })
}
pub(super) fn controls(state: &State, area: Rect) -> Vec<(Rect, Hit)> {
    let mut hits = tiles(state, area)
        .into_iter()
        .map(|(r, i)| (r, Hit::ShortcutSelect(i)))
        .collect::<Vec<_>>();
    let top = area.top - state.options_scroll;
    if let Some(i) = state.studio_editor.selected.filter(|&i| {
        state
            .selected_page_ref()
            .is_some_and(|p| i < p.studio.nodes.len())
    }) {
        hits.extend([
            (
                Rect::new(area.left + 50.0, top + 5.0, area.right - 4.0, top + 37.0),
                Hit::EditPreference(Field::ShortcutName(i)),
            ),
            (
                Rect::new(area.left + 50.0, top + 46.0, area.right - 4.0, top + 78.0),
                Hit::EditPreference(Field::ShortcutTarget(i)),
            ),
        ]);
        for (n, h) in [
            Hit::ShortcutMove(-1),
            Hit::ShortcutMove(1),
            Hit::ShortcutDelete,
            Hit::ItemColor(false, Some(i)),
            Hit::ItemColor(true, Some(i)),
            Hit::ResetItemColor(false, Some(i)),
            Hit::ResetItemColor(true, Some(i)),
        ]
        .into_iter()
        .enumerate()
        {
            hits.push((
                Rect::from_size(
                    area.left + (n % button_columns(area)) as f32 * 76.0,
                    top + 90.0 + (n / button_columns(area)) as f32 * 34.0,
                    70.0,
                    30.0,
                ),
                h,
            ));
        }
    }
    hits
}
pub(super) fn paint(list: &mut DrawList, state: &mut State, area: Rect, p: &Palette) {
    let Some(page) = state.selected_page_ref().cloned() else {
        return;
    };
    list.push_clip(area);
    let top = area.top - state.options_scroll;
    let controls = controls(state, area);
    if state.studio_editor.selected.is_none() {
        list.text(
            Rect::new(area.left + 4.0, top + 20.0, area.right - 4.0, top + 62.0),
            "选择一个图标，编辑名称、路径与颜色",
            TextStyle::Label,
            p.foreground,
        );
    }
    list.text(
        Rect::new(
            area.left + 4.0,
            top + header_height(area) - 34.0,
            area.right - 4.0,
            top + header_height(area) - 10.0,
        ),
        "拖入浮窗添加 · 双击打开 · 长按拖动排序 · 删除仅移除快捷引用",
        TextStyle::Caption,
        p.muted,
    );
    for (r, hit) in controls {
        match hit {
            Hit::ShortcutSelect(i) => {
                let n = &page.studio.nodes[i];
                let r = r.inset(Edges::xy(4.0, 4.0));
                let style = page
                    .item_styles
                    .get(&format!("node:{}", n.id))
                    .cloned()
                    .unwrap_or_default()
                    .over(&mochi_core::desktop_cards::ItemStyle {
                        foreground: page.presentation.item_foreground,
                        background: page.presentation.item_background,
                    });
                let bg = style.background.unwrap_or(p.surface);
                let fg = style.foreground.unwrap_or_else(|| {
                    if style.background.is_some() {
                        if ((bg >> 16) & 255) * 299 + ((bg >> 8) & 255) * 587 + (bg & 255) * 114
                            < 145000
                        {
                            0xf0f2f4
                        } else {
                            0x252b32
                        }
                    } else {
                        p.foreground
                    }
                });
                if style.background.is_some() {
                    list.rounded_rect(r, 7.0, bg);
                }
                if state.studio_editor.selected == Some(i) || state.hover == Some(hit) {
                    list.rounded_rect(r, 7.0, crate::ui::theme::mix(fg, bg, 0.06));
                }
                if page.presentation.grid_lines || state.studio_editor.selected == Some(i) {
                    list.rounded_border(r, 7.0, crate::ui::theme::mix(fg, bg, 0.20));
                }
                let y = r.top + (r.height() - 60.0).max(0.0) / 2.0;
                if page.presentation.show_icons {
                    if let Some(path) =
                        crate::desktop_window::shortcut_icon::path(&state.icon_workspace, &n.target)
                    {
                        list.image(
                            Rect::from_size((r.left + r.right) / 2.0 - 15.0, y, 30.0, 30.0),
                            path,
                            &n.title,
                        );
                    } else {
                        list.icon_centered(
                            Rect::from_size(r.left, y, r.width(), 30.0),
                            Icon::FILE_TEXT,
                            26.0,
                            fg,
                        );
                    }
                }
                if page.presentation.show_names {
                    list.text_aligned(
                        Rect::new(
                            r.left + 5.0,
                            if page.presentation.show_icons {
                                y + 34.0
                            } else {
                                r.top
                            },
                            r.right - 5.0,
                            if page.presentation.show_icons {
                                y + 60.0
                            } else {
                                r.bottom
                            },
                        ),
                        crate::ui::text::ellipsize(&n.title, TextStyle::Label, r.width() - 10.0),
                        TextStyle::Label,
                        fg,
                        Align::Center,
                    );
                }
            }
            Hit::EditPreference(field) => {
                let (label, value) = match field {
                    Field::ShortcutName(i) => ("名称", page.studio.nodes[i].title.as_str()),
                    Field::ShortcutTarget(i) => ("路径", page.studio.nodes[i].target.as_str()),
                    _ => continue,
                };
                list.text(
                    Rect::new(area.left, r.top, r.left - 6.0, r.bottom),
                    label,
                    TextStyle::Caption,
                    p.muted,
                );
                if state.focus_field == Some(field) {
                    state
                        .preference_text
                        .paint(list, r, true, p, FieldLook::dialog(p));
                } else {
                    list.rounded_border(r, 6.0, p.border);
                    list.text(
                        Rect::new(r.left + 8.0, r.top, r.right - 8.0, r.bottom),
                        crate::ui::text::ellipsize(value, TextStyle::Label, r.width() - 16.0),
                        TextStyle::Label,
                        p.foreground,
                    );
                }
            }
            _ => {
                let label = match hit {
                    Hit::ShortcutMove(-1) => "前移",
                    Hit::ShortcutMove(_) => "后移",
                    Hit::ShortcutDelete => "删除",
                    Hit::ItemColor(false, _) => "文字色",
                    Hit::ItemColor(true, _) => "背景色",
                    Hit::ResetItemColor(false, _) => "恢复文字",
                    Hit::ResetItemColor(true, _) => "恢复底色",
                    _ => "",
                };
                if state.hover == Some(hit) {
                    list.rounded_rect(r, 5.0, p.surface_muted);
                }
                list.rounded_border(r, 5.0, p.border);
                list.text_aligned(r, label, TextStyle::Caption, p.foreground, Align::Center);
            }
        }
    }
    list.pop_clip();
}
