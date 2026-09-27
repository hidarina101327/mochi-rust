//! 绘制桌面卡片、标题栏和卡片编辑界面。
use super::{
    model::{fields_for, route_label, CardSizePreset, ModuleKind, PageExt, ROUTES},
    DeleteTarget, Hit, Layout, State,
};
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::{self, Palette},
    widgets::FieldLook,
};

pub fn paint(list: &mut DrawList, state: &mut State, layout: &Layout, viewport: Rect, p: &Palette) {
    let mut calm = *p;
    calm.accent = p.foreground;
    calm.accent_hover = p.muted;
    calm.accent_foreground = p.surface;
    let p = &calm;
    if viewport.is_empty() {
        return;
    }
    if !state.embedded {
        list.rect_alpha(viewport, 0x000000, 0.38);
    }
    let r = layout.frame;
    if !state.embedded {
        list.rounded_rect_alpha(
            Rect::new(r.left - 2.0, r.top + 4.0, r.right + 2.0, r.bottom + 8.0),
            10.0,
            0x000000,
            0.12,
        );
        list.rounded_rect(r, 12.0, p.surface);
        list.rounded_border(r, 12.0, p.border);
    } else {
        list.rect(r, p.area_main_default);
    }
    list.push_clip(r);

    paint_header(list, state, layout, p);
    paint_cards(list, state, layout, p);
    list.push_clip(layout.right_body);
    paint_editor(list, state, layout, p);
    list.pop_clip();
    paint_footer(list, state, layout, p);

    if let Some(target) = state.confirm_delete {
        paint_delete_confirmation(list, state, target, layout, p);
    } else if state.confirm_cancel {
        paint_cancel_confirmation(list, state, layout, p);
    }
    list.pop_clip();

    if !state.confirm_cancel && state.confirm_delete.is_none() {
        state.scrollbar.paint(list, &layout.bars(state), p);
        if state.embedded {
            paint_tooltip(list, state, layout, p);
        }
    }
}

fn paint_header(list: &mut DrawList, state: &State, layout: &Layout, p: &Palette) {
    let r = layout.frame;
    list.text(
        Rect::new(r.left + 22.0, r.top + 12.0, r.left + 260.0, r.top + 39.0),
        "桌面卡片",
        if state.embedded {
            TextStyle::Heading2
        } else {
            TextStyle::Large
        },
        p.foreground,
    );
    if state.embedded {
        list.hline(r.left, r.right, layout.header.bottom, p.border);
        for (rect, hit) in &layout.controls {
            match hit {
                Hit::NewCard if rect.top < layout.header.bottom => button(
                    list,
                    *rect,
                    Some(Icon::PLUS),
                    "新建",
                    state.hover == Some(*hit),
                    p,
                    true,
                ),
                Hit::Import | Hit::ExportAll => button(
                    list,
                    *rect,
                    Some(if *hit == Hit::Import {
                        Icon::UPLOAD
                    } else {
                        Icon::DOWNLOAD
                    }),
                    "",
                    state.hover == Some(*hit),
                    p,
                    false,
                ),
                _ => {}
            }
        }
        return;
    }
    let toolbar_y = r.top + 19.0;
    let width = r.width();
    let right = r.right - 54.0;
    let import_w = if width >= 640.0 { 62.0 } else { 32.0 };
    let export_w = if width >= 640.0 { 84.0 } else { 32.0 };
    let new_w = if width >= 640.0 { 78.0 } else { 32.0 };
    let new_rect = Rect::from_size(
        right - import_w - export_w - new_w - 16.0,
        toolbar_y,
        new_w,
        30.0,
    );
    let import_rect = Rect::from_size(right - import_w - export_w - 8.0, toolbar_y, import_w, 30.0);
    let export_rect = Rect::from_size(right - export_w, toolbar_y, export_w, 30.0);
    button(
        list,
        new_rect,
        Some(Icon::PLUS),
        if width >= 640.0 { "新建" } else { "" },
        state.hover == Some(Hit::NewCard),
        p,
        true,
    );
    button(
        list,
        import_rect,
        Some(Icon::UPLOAD),
        if width >= 640.0 { "导入" } else { "" },
        state.hover == Some(Hit::Import),
        p,
        false,
    );
    button(
        list,
        export_rect,
        Some(Icon::DOWNLOAD),
        if width >= 640.0 { "布局导出" } else { "" },
        state.hover == Some(Hit::ExportAll),
        p,
        false,
    );
    list.icon_centered(
        Rect::from_size(r.right - 42.0, r.top + 16.0, 26.0, 26.0),
        Icon::X,
        16.0,
        p.muted,
    );
    if state.focused == Some(Hit::Close) {
        list.rounded_border(
            Rect::from_size(r.right - 42.0, r.top + 16.0, 26.0, 26.0),
            5.0,
            p.accent,
        );
    }
}

fn paint_cards(list: &mut DrawList, state: &State, layout: &Layout, p: &Palette) {
    let pane = layout.cards_pane;
    list.rect(pane, theme::mix(p.surface_muted, p.surface, 0.18));
    list.vline(pane.right, pane.top, pane.bottom, p.border);
    list.text(
        Rect::new(
            pane.left + 14.0,
            pane.top + 14.0,
            pane.right - 14.0,
            pane.top + 38.0,
        ),
        "我的卡片",
        TextStyle::Title,
        p.foreground,
    );
    if !state.embedded {
        // 新增按钮刻意画成标题旁的小图标（正常宽度下的紧凑形态）；
        // 完整的命中矩形保持不变，对键盘和读屏器友好。
        let add_icon = Rect::from_size(pane.right - 38.0, pane.top + 18.0, 24.0, 24.0);
        list.rounded_rect(add_icon, 5.0, p.surface);
        list.rounded_border(
            add_icon,
            5.0,
            if state.hover == Some(Hit::NewCard) {
                p.accent
            } else {
                p.border
            },
        );
        list.icon_centered(add_icon, Icon::PLUS, 15.0, p.accent);
    }

    if state.config.cards.is_empty() {
        let center = Rect::new(
            pane.left + 16.0,
            pane.top + 120.0,
            pane.right - 16.0,
            pane.bottom - 110.0,
        );
        list.icon_centered(
            Rect::from_size(center.left, center.top, center.width(), 30.0),
            Icon::LAYOUT_GRID,
            26.0,
            p.muted,
        );
        list.text_aligned(
            Rect::new(
                center.left,
                center.top + 42.0,
                center.right,
                center.top + 66.0,
            ),
            "还没有桌面卡片",
            TextStyle::Label,
            p.foreground,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(
                center.left,
                center.top + 68.0,
                center.right,
                center.top + 92.0,
            ),
            "新建一张卡片，",
            TextStyle::Caption,
            p.muted,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(
                center.left,
                center.top + 88.0,
                center.right,
                center.top + 112.0,
            ),
            "选择想关注的模块。",
            TextStyle::Caption,
            p.muted,
            Align::Center,
        );
        return;
    }

    list.push_clip(layout.cards_body);
    for (row, index) in &layout.card_rows {
        let Some(card) = state.config.cards.get(*index) else {
            continue;
        };
        let selected = state.selected_card == Some(*index);
        let hovered_card = is_hovering_card(state, *index);
        let active = selected || hovered_card;
        if active {
            list.rounded_rect(*row, 6.0, if selected { p.surface } else { p.surface });
        }
        list.rounded_border(
            *row,
            6.0,
            if selected {
                p.border
            } else if hovered_card {
                p.border
            } else {
                theme::mix(p.border, p.surface, 0.72)
            },
        );
        if selected {
            list.rounded_rect(
                Rect::from_size(row.left, row.top + 12.0, 2.0, row.height() - 24.0),
                1.0,
                p.accent,
            );
        }
        let icon_rect = Rect::from_size(row.left + 10.0, row.top + 12.0, 24.0, 24.0);
        list.rounded_rect(icon_rect, 6.0, p.surface_muted);
        list.icon_centered(
            icon_rect,
            Icon::LAYOUT_GRID,
            14.0,
            if selected { p.accent } else { p.muted },
        );
        let geometry = super::geometry::card_row_layout(*row, *index);
        list.text(
            geometry.title,
            text::ellipsize(
                &card.title,
                TextStyle::Label,
                geometry.title.width().max(1.0),
            ),
            TextStyle::Label,
            p.foreground,
        );
        let status = format!(
            "{} 个分页 · {}",
            card.pages.len(),
            if card.enabled {
                "显示中"
            } else {
                "已隐藏"
            }
        );
        list.text(
            geometry.status,
            text::ellipsize(
                &status,
                TextStyle::Caption,
                geometry.status.width().max(1.0),
            ),
            TextStyle::Caption,
            p.muted,
        );

        // 锁定/可见性图标始终显示（行的右上角）。
        for (rect, hit) in &geometry.controls {
            let icon = match hit {
                Hit::CardLock(_) => {
                    if card.locked {
                        Icon::PIN
                    } else {
                        Icon::PENCIL
                    }
                }
                Hit::CardVisible(_) => {
                    if card.enabled {
                        Icon::EYE
                    } else {
                        Icon::CIRCLE
                    }
                }
                _ => continue, // 操作按钮在下方处理
            };
            let btn_hovered = state.hover == Some(*hit);
            if btn_hovered {
                list.rounded_rect(*rect, 4.0, p.surface_muted);
            }
            let color = if matches!(hit, Hit::CardVisible(_)) && card.enabled {
                p.accent
            } else {
                p.muted
            };
            list.icon_centered(*rect, icon, 13.0, color);
        }

        // 键盘聚焦时也显示工具条，保证这些操作始终可被发现。
        if state.card_toolbar_visible(*index) {
            let toolbar_bg = Rect::new(
                row.left + 2.0,
                row.bottom - 26.0,
                row.right - 2.0,
                row.bottom - 2.0,
            );
            list.rounded_rect_alpha(
                Rect::new(
                    toolbar_bg.left,
                    toolbar_bg.top + 2.0,
                    toolbar_bg.right,
                    toolbar_bg.bottom + 2.0,
                ),
                6.0,
                0x000000,
                0.08,
            );
            list.rounded_rect(toolbar_bg, 6.0, p.surface);
            list.rounded_border(toolbar_bg, 6.0, p.border);
            for (rect, hit) in &geometry.controls {
                let icon = match hit {
                    Hit::CardLocate(_) => Icon::CROSSHAIR,
                    Hit::CardExport(_) => Icon::DOWNLOAD,
                    Hit::CardDuplicate(_) => Icon::COPY,
                    Hit::CardUp(_) => Icon::ARROW_UP,
                    Hit::CardDown(_) => Icon::ARROW_DOWN,
                    Hit::CardDelete(_) => Icon::TRASH2,
                    _ => continue,
                };
                let btn_hovered = state.hover == Some(*hit) || state.focused == Some(*hit);
                if btn_hovered {
                    list.rounded_rect(*rect, 4.0, p.surface_muted);
                }
                let color = if btn_hovered && matches!(hit, Hit::CardDelete(_)) {
                    p.danger
                } else {
                    p.muted
                };
                list.icon_centered(*rect, icon, 12.0, color);
            }
        }
    }
    list.pop_clip();
    let hide = Rect::from_size(
        pane.left + 12.0,
        pane.bottom - 42.0,
        pane.width() - 24.0,
        30.0,
    );
    list.rounded_rect(hide, 5.0, p.surface);
    list.rounded_border(
        hide,
        5.0,
        if state.hover == Some(Hit::HideAll) {
            p.accent
        } else {
            p.border
        },
    );
    list.icon_centered(
        Rect::from_size(hide.left + 8.0, hide.top + 5.0, 20.0, 20.0),
        Icon::EYE,
        13.0,
        p.muted,
    );
    list.text(
        Rect::new(hide.left + 34.0, hide.top, hide.right - 8.0, hide.bottom),
        "全部隐藏",
        TextStyle::Caption,
        p.muted,
    );
}

fn paint_editor(list: &mut DrawList, state: &mut State, layout: &Layout, p: &Palette) {
    let body = layout.right_body;
    if state.creating.is_some() {
        list.text(
            Rect::new(body.left, body.top + 4.0, body.right, body.top + 34.0),
            "选择一个模块",
            TextStyle::Large,
            p.foreground,
        );
        paint_templates(list, state, layout, p);
        return;
    }
    if state.selected_card_ref().is_none() {
        list.text(
            Rect::new(body.left, body.top + 30.0, body.right, body.top + 62.0),
            "选择一张卡片开始编辑",
            TextStyle::Large,
            p.foreground,
        );
        list.text(
            Rect::new(body.left, body.top + 68.0, body.right, body.top + 102.0),
            "你可以为一张卡片添加多个分页，每个分页使用自己的模块模板。",
            TextStyle::Caption,
            p.muted,
        );
        return;
    }
    let Some(card) = state.selected_card_ref() else {
        return;
    };
    let card_width = card.width;
    let card_height = card.height;
    let card_enabled = card.enabled;
    let card_locked = card.locked;
    let pages = card.pages.clone();
    state.card_name.paint(
        list,
        layout.card_name,
        state.focus_field == Some(super::Field::CardName),
        p,
        FieldLook::dialog(p),
    );
    for (i, label) in ["外观", "分页", "设计工作台"]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < 2 || super::studio::enabled(state))
    {
        let r = Rect::from_size(body.left + i as f32 * 88.0, body.top + 46.0, 80.0, 28.0);
        tab_surface(list, r, state.editor_tab == i as u8, p);
        list.text_aligned(
            r,
            *label,
            TextStyle::Label,
            if state.editor_tab == i as u8 {
                p.foreground
            } else {
                p.muted
            },
            Align::Center,
        );
    }
    for (i, preset) in CardSizePreset::ALL
        .iter()
        .copied()
        .enumerate()
        .filter(|_| layout.appearance_open)
    {
        let rect = super::geometry::size_preset_rect(body, i);
        let selected = preset.matches(card_width, card_height);
        tab_surface(list, rect, selected, p);
        list.text_aligned(
            rect,
            if rect.width() >= 94.0 {
                size_label(preset)
            } else {
                match preset {
                    CardSizePreset::Compact => "紧凑",
                    CardSizePreset::Medium => "标准",
                    CardSizePreset::Spacious => "宽松",
                }
            },
            TextStyle::Caption,
            if selected { p.accent } else { p.muted },
            Align::Center,
        );
    }
    let visible = Rect::from_size(
        layout.card_name.right + 10.0,
        layout.card_name.top,
        30.0,
        30.0,
    );
    let locked = Rect::from_size(
        layout.card_name.right + 46.0,
        layout.card_name.top,
        30.0,
        30.0,
    );
    icon_toggle(
        list,
        visible,
        if card_enabled {
            Icon::EYE
        } else {
            Icon::CIRCLE
        },
        card_enabled,
        p,
    );
    icon_toggle(
        list,
        locked,
        if card_locked { Icon::PIN } else { Icon::PENCIL },
        card_locked,
        p,
    );

    if state.editor_tab == 2 {
        super::studio::paint(list, state, body, p);
        return;
    }
    if layout.appearance_open {
        super::appearance::paint(
            list,
            state,
            Rect::new(body.left, body.top + 40.0, body.right, body.bottom),
            p,
        );
        list.push_clip(layout.options_view);
        for (rect, index) in &layout.option_rows {
            super::preferences::paint(list, state, *rect, *index, p);
        }
        list.pop_clip();
        return;
    }
    let section_top = 86.0;

    if !state.embedded {
        section_header(
            list,
            Rect::new(
                body.left,
                body.top + section_top,
                body.right,
                body.top + section_top + 20.0,
            ),
            "分页",
            Some("双击标签重命名"),
            p,
        );
    }

    list.push_clip(Rect::new(
        layout.page_tabs.left,
        layout.page_tabs.top,
        layout.page_tabs.right - 102.0,
        layout.page_tabs.bottom,
    ));
    if state
        .selected_card_ref()
        .is_some_and(|c| c.pages.is_empty())
    {
        list.text(layout.page_tabs, "暂无分页", TextStyle::Caption, p.muted);
    }
    for (rect, index) in &layout.page_rows {
        let Some(page) = pages.get(*index) else {
            continue;
        };
        let selected = state.selected_page == Some(*index);
        tab_surface(list, *rect, selected, p);
        list.text(
            Rect::new(rect.left + 10.0, rect.top, rect.right - 30.0, rect.bottom),
            text::ellipsize(
                &page.title,
                TextStyle::Caption,
                (rect.width() - 30.0).max(1.0),
            ),
            TextStyle::Caption,
            if selected { p.accent } else { p.foreground },
        );
        list.icon_centered(
            Rect::from_size(rect.right - 25.0, rect.top + 7.0, 18.0, 18.0),
            Icon::X,
            11.0,
            p.muted,
        );
    }
    list.pop_clip();
    for (dx, icon) in [(98.0, Icon::CHEVRON_LEFT), (66.0, Icon::CHEVRON_RIGHT)] {
        list.icon_centered(
            Rect::from_size(
                layout.page_tabs.right - dx,
                layout.page_tabs.top + 2.0,
                28.0,
                32.0,
            ),
            icon,
            14.0,
            p.muted,
        );
    }
    let add = Rect::from_size(
        layout.page_tabs.right - 34.0,
        layout.page_tabs.top + 2.0,
        30.0,
        32.0,
    );
    list.rounded_rect(add, 5.0, p.surface);
    list.rounded_border(
        add,
        5.0,
        if state.hover == Some(Hit::AddPage) {
            p.accent
        } else {
            p.border
        },
    );
    list.icon_centered(add, Icon::PLUS, 14.0, p.accent);
    if state.focus_field == Some(super::Field::PageName) {
        state
            .page_name
            .paint(list, layout.page_name, true, p, FieldLook::dialog(p));
    }
    paint_options(list, state, layout, p);
}

fn paint_templates(list: &mut DrawList, state: &State, layout: &Layout, p: &Palette) {
    list.push_clip(layout.modules_body);
    for (rect, module) in &layout.module_rows {
        let selected = state
            .selected_page_ref()
            .is_some_and(|page| page.module == *module);
        let hot = state.hover == Some(Hit::Module(*module));
        if state.embedded {
            list.rounded_rect(*rect, 8.0, if hot { p.surface_muted } else { p.surface });
            list.rounded_border(*rect, 8.0, if hot { p.muted } else { p.border });
            list.icon_centered(
                Rect::from_size(rect.left + 14.0, rect.top + 20.0, 28.0, 28.0),
                module_icon(*module),
                20.0,
                p.muted,
            );
            list.text(
                Rect::new(rect.left + 52.0, rect.top, rect.right - 10.0, rect.bottom),
                module.label(),
                TextStyle::Label,
                p.foreground,
            );
            continue;
        }
        list.rounded_rect(*rect, 10.0, if hot { p.surface_muted } else { p.surface });
        list.rounded_border(*rect, 10.0, if hot { p.muted } else { p.border });
        list.icon_centered(
            Rect::from_size(rect.left + 8.0, rect.top + 8.0, 22.0, 22.0),
            module_icon(*module),
            15.0,
            if selected { p.accent } else { p.muted },
        );
        list.text(
            Rect::new(
                rect.left + 36.0,
                rect.top + 5.0,
                rect.right - 6.0,
                rect.top + 24.0,
            ),
            module.label(),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 36.0,
                rect.top + 24.0,
                rect.right - 6.0,
                rect.bottom - 3.0,
            ),
            text::ellipsize(
                module.description(),
                TextStyle::Caption,
                (rect.width() - 42.0).max(1.0),
            ),
            TextStyle::Caption,
            p.muted,
        );
    }
    list.pop_clip();
}

fn paint_options(list: &mut DrawList, state: &mut State, layout: &Layout, p: &Palette) {
    let body = layout.options_body;
    let Some(page) = state.selected_page_ref().cloned() else {
        list.text(body, "先添加一个分页", TextStyle::Caption, p.muted);
        return;
    };
    list.push_clip(body);
    for (i, (mode, label)) in [(false, "展示内容"), (true, "显示设置")]
        .into_iter()
        .enumerate()
    {
        let r = Rect::from_size(body.left + i as f32 * 88.0, body.top, 80.0, 28.0);
        tab_surface(list, r, state.preferences_mode == mode, p);
        list.text_aligned(
            r,
            label,
            TextStyle::Label,
            if state.preferences_mode == mode {
                p.foreground
            } else {
                p.muted
            },
            Align::Center,
        );
    }
    list.text_aligned(
        Rect::new(
            body.left + 184.0,
            body.top,
            body.right - 8.0,
            body.top + 28.0,
        ),
        page.module.label(),
        TextStyle::Caption,
        p.muted,
        Align::Trailing,
    );
    if super::model::interactive(page.module) && !state.preferences_mode {
        let help=match page.module {ModuleKind::Ai=>"直接在卡片底部输入并发送消息。\n右上角切换对话历史，回复区可以滚动。",ModuleKind::Pomodoro=>"在卡片中直接开始、暂停或重置计时。\n计时状态与墨池的番茄钟同步。",ModuleKind::Shortcuts=>"将文件或桌面图标拖入卡片，自动保存原路径。\n双击打开快捷方式；在设计工作台中调整组件。",_=>"在「设计工作台」添加组件、拖动布局、设置参数与事件。\n保存后更新桌面卡片，也可导出为可复用模板。"};
        list.text(
            Rect::new(
                body.left + 8.0,
                body.top + 44.0,
                body.right - 8.0,
                body.top + 122.0,
            ),
            help,
            TextStyle::Label,
            p.muted,
        );
        list.pop_clip();
        return;
    }
    if page.module == ModuleKind::Shortcuts && !state.preferences_mode {
        super::shortcuts::paint(list, state, layout.options_view, p);
        list.pop_clip();
        return;
    }
    if super::utility::enabled(page.module) && !state.preferences_mode {
        super::utility::paint(list, state, layout.options_view, p);
        list.pop_clip();
        return;
    }
    if page.module == ModuleKind::Folder && !state.preferences_mode {
        super::folder::paint(list, state, layout.options_view, p);
        list.pop_clip();
        return;
    }
    list.push_clip(layout.options_view);
    for (rect, index) in &layout.option_rows {
        if state.preferences_mode {
            super::preferences::paint(list, state, *rect, *index, p);
            continue;
        }
        let fields = fields_for(page.module);
        let Some(field) = fields.get(*index) else {
            super::preferences::paint(list, state, *rect, *index - fields.len(), p);
            continue;
        };
        let selected = page.selected(&field.key);
        list.rounded_rect(
            *rect,
            5.0,
            if state.hover == Some(Hit::Option(*index)) {
                p.surface_muted
            } else {
                p.surface
            },
        );
        list.rounded_border(*rect, 5.0, p.border);
        let box_ = Rect::from_size(rect.left + 9.0, rect.top + 13.0, 18.0, 18.0);
        list.rounded_rect(box_, 4.0, if selected { p.accent } else { p.surface });
        list.rounded_border(box_, 4.0, if selected { p.accent } else { p.muted });
        if selected {
            list.icon_centered(box_, Icon::CHECK, 13.0, p.accent_foreground);
        }
        list.text(
            Rect::new(
                rect.left + 36.0,
                rect.top + 5.0,
                rect.right - 8.0,
                rect.top + 25.0,
            ),
            &field.label,
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(
                rect.left + 36.0,
                rect.top + 25.0,
                rect.right - 8.0,
                rect.bottom - 3.0,
            ),
            text::ellipsize(
                &field.description,
                TextStyle::Caption,
                (rect.width() - 44.0).max(1.0),
            ),
            TextStyle::Caption,
            p.muted,
        );
    }
    list.pop_clip();
    if page.module == ModuleKind::Shortcuts || super::model::interactive(page.module) {
        list.pop_clip();
        return;
    }
    list.hline(body.left, body.right, layout.options_footer_top, p.border);
    let limit_top = layout.options_footer_top + 8.0;
    let limit = Rect::from_size(body.left, limit_top, body.width(), 32.0);
    list.rounded_rect(limit, 5.0, p.surface);
    list.rounded_border(limit, 5.0, p.border);
    list.text(
        Rect::new(
            limit.left + 10.0,
            limit.top,
            limit.right - 70.0,
            limit.bottom,
        ),
        if page.limit == 0 {
            "显示数量：全部".into()
        } else {
            format!("显示数量：{}", page.limit)
        },
        TextStyle::Caption,
        p.muted,
    );
    list.text_aligned(
        Rect::from_size(limit.right - 114.0, limit.top + 4.0, 48.0, 24.0),
        "全部",
        TextStyle::Caption,
        p.muted,
        Align::Center,
    );
    if state.focus_field == Some(super::Field::Limit) {
        state.preference_text.paint(
            list,
            Rect::new(
                limit.left + 4.0,
                limit.top + 2.0,
                limit.right - 120.0,
                limit.bottom - 2.0,
            ),
            true,
            p,
            FieldLook::dialog(p),
        );
    }
    let minus = Rect::from_size(limit.right - 60.0, limit.top + 4.0, 24.0, 24.0);
    let plus = Rect::from_size(limit.right - 32.0, limit.top + 4.0, 24.0, 24.0);
    list.rounded_rect(minus, 4.0, p.surface_muted);
    list.rounded_rect(plus, 4.0, p.surface_muted);
    list.icon_centered(minus, Icon::MINUS, 12.0, p.muted);
    list.icon_centered(plus, Icon::PLUS, 12.0, p.muted);
    let source_top = limit_top + 40.0;
    if page.module == ModuleKind::Folder {
        list.text(
            Rect::from_size(body.left, source_top, body.width(), 30.0),
            "双击或 Enter 打开；右键可整理、导入和管理文件",
            TextStyle::Caption,
            p.muted,
        );
        list.pop_clip();
        return;
    }
    if page.has_source() {
        let source = Rect::from_size(body.left, source_top, body.width(), 30.0);
        list.rounded_rect(source, 5.0, p.surface);
        list.rounded_border(
            source,
            5.0,
            if state.hover == Some(Hit::Source) {
                p.accent
            } else {
                p.border
            },
        );
        list.icon_centered(
            Rect::from_size(source.left + 6.0, source.top + 5.0, 20.0, 20.0),
            Icon::FOLDER_OPEN,
            13.0,
            p.muted,
        );
        let summary = if page.sources.is_empty() {
            page.source
                .clone()
                .unwrap_or("全部内容 · 选择知识库、文件夹或文件…".into())
        } else {
            format!(
                "{} 个来源 · {}",
                page.sources.len(),
                page.sources.join("、")
            )
        };
        let label = summary.as_str();
        list.icon_centered(
            Rect::from_size(source.right - 30.0, source.top, 30.0, 30.0),
            Icon::X,
            12.0,
            p.muted,
        );
        list.text(
            Rect::new(
                source.left + 32.0,
                source.top,
                source.right - 38.0,
                source.bottom,
            ),
            text::ellipsize(label, TextStyle::Caption, (source.width() - 42.0).max(1.0)),
            TextStyle::Caption,
            if page.source.is_some() {
                p.foreground
            } else {
                p.muted
            },
        );
    }
    let route_top = source_top + if page.has_source() { 52.0 } else { 12.0 };
    list.text(
        Rect::new(body.left, route_top - 18.0, body.right, route_top),
        "点击行为",
        TextStyle::Caption,
        p.muted,
    );
    let segment = Rect::from_size(body.left, route_top, body.width().min(320.0), 28.0);
    list.rounded_rect(segment, 14.0, p.surface_muted);
    for route in ROUTES.iter().copied() {
        let Some((rect, _)) = layout
            .controls
            .iter()
            .find(|(_, hit)| *hit == Hit::Route(route))
        else {
            continue;
        };
        let inner = Rect::new(
            rect.left + 2.0,
            rect.top + 2.0,
            rect.right - 2.0,
            rect.bottom - 2.0,
        );
        let selected = page.interaction == route;
        if selected || state.hover == Some(Hit::Route(route)) {
            list.rounded_rect(inner, 12.0, p.surface);
        }
        if selected {
            list.rounded_border(inner, 12.0, theme::mix(p.accent, p.border, 0.5));
        }
        list.text_aligned(
            *rect,
            route_label(route),
            TextStyle::Caption,
            if selected { p.accent } else { p.muted },
            Align::Center,
        );
    }
    list.pop_clip();
}

fn paint_footer(list: &mut DrawList, state: &State, layout: &Layout, p: &Palette) {
    let r = layout.footer;
    list.hline(r.left, r.right, r.top, p.border);
    if state.embedded {
        let message = state
            .error
            .as_deref()
            .unwrap_or(if state.dirty { "未保存" } else { "" });
        list.text(
            Rect::new(r.left + 20.0, r.top, r.right - 170.0, r.bottom),
            text::ellipsize(message, TextStyle::Caption, (r.width() - 190.0).max(1.0)),
            TextStyle::Caption,
            if state.error.is_some() {
                p.danger
            } else {
                p.muted
            },
        );
        button(
            list,
            Rect::from_size(r.right - 150.0, r.top + 12.0, 30.0, 30.0),
            Some(Icon::ROTATE_CCW),
            "",
            state.hover == Some(Hit::Cancel),
            p,
            false,
        );
        button(
            list,
            Rect::from_size(r.right - 108.0, r.top + 12.0, 88.0, 30.0),
            Some(Icon::SAVE),
            "保存",
            state.hover == Some(Hit::SaveKeepOpen),
            p,
            true,
        );
        return;
    }
    let message = if let Some(error) = &state.error {
        error.as_str()
    } else if state.dirty {
        "有未保存的修改"
    } else {
        "保存后可继续编辑，应用后关闭"
    };
    list.text(
        Rect::new(r.left + 22.0, r.top + 15.0, r.right - 320.0, r.top + 38.0),
        text::ellipsize(message, TextStyle::Caption, (r.width() - 342.0).max(1.0)),
        TextStyle::Caption,
        if state.error.is_some() {
            p.danger
        } else {
            p.muted
        },
    );
    let cancel = Rect::from_size(r.right - 292.0, r.top + 13.0, 82.0, 28.0);
    button(
        list,
        Rect::from_size(r.right - 198.0, r.top + 13.0, 82.0, 28.0),
        Some(Icon::SAVE),
        "保存",
        state.hover == Some(Hit::SaveKeepOpen),
        p,
        false,
    );
    let save = Rect::from_size(r.right - 104.0, r.top + 13.0, 82.0, 28.0);
    list.text_aligned(
        cancel,
        "取消",
        TextStyle::Caption,
        if state.hover == Some(Hit::Cancel) || state.focused == Some(Hit::Cancel) {
            p.foreground
        } else {
            p.muted
        },
        Align::Center,
    );
    button(
        list,
        save,
        Some(Icon::SAVE),
        "应用",
        state.hover == Some(Hit::Save),
        p,
        true,
    );
}

fn paint_delete_confirmation(
    list: &mut DrawList,
    state: &State,
    target: DeleteTarget,
    layout: &Layout,
    p: &Palette,
) {
    list.rect_alpha(layout.frame, 0x000000, 0.28);
    let (dialog, cancel, confirm) = super::geometry::confirmation_rects(layout.frame);
    list.rounded_rect(dialog, 12.0, p.surface);
    list.rounded_border(dialog, 12.0, p.border);
    let title = match target {
        DeleteTarget::Card(_) => "删除这张卡片？",
        DeleteTarget::Page(_) => "删除这个分页？",
    };
    list.text(
        Rect::new(
            dialog.left + 24.0,
            dialog.top + 22.0,
            dialog.right - 24.0,
            dialog.top + 52.0,
        ),
        title,
        TextStyle::Large,
        p.foreground,
    );
    list.text(
        Rect::new(
            dialog.left + 24.0,
            dialog.top + 64.0,
            dialog.right - 24.0,
            dialog.top + 94.0,
        ),
        "保存或应用后生效，原始内容不会被删除。",
        TextStyle::Label,
        p.muted,
    );
    button(
        list,
        cancel,
        None,
        "保留",
        state.hover == Some(Hit::CancelDelete) || state.focused == Some(Hit::CancelDelete),
        p,
        false,
    );
    button(
        list,
        confirm,
        None,
        "删除",
        state.hover == Some(Hit::ConfirmDelete) || state.focused == Some(Hit::ConfirmDelete),
        p,
        false,
    );
}

fn paint_cancel_confirmation(list: &mut DrawList, state: &State, layout: &Layout, p: &Palette) {
    list.rect_alpha(layout.frame, 0x000000, 0.28);
    let (dialog, cancel, confirm) = super::geometry::confirmation_rects(layout.frame);
    list.rounded_rect_alpha(
        Rect::new(
            dialog.left - 3.0,
            dialog.top + 4.0,
            dialog.right + 3.0,
            dialog.bottom + 8.0,
        ),
        12.0,
        0x000000,
        0.15,
    );
    list.rounded_rect(dialog, 12.0, p.surface);
    list.rounded_border(dialog, 12.0, p.border);
    list.text(
        Rect::new(
            dialog.left + 24.0,
            dialog.top + 22.0,
            dialog.right - 24.0,
            dialog.top + 52.0,
        ),
        "放弃尚未保存的修改？",
        TextStyle::Large,
        p.foreground,
    );
    list.text(
        Rect::new(
            dialog.left + 24.0,
            dialog.top + 62.0,
            dialog.right - 24.0,
            dialog.top + 92.0,
        ),
        "本次调整还没有应用，放弃后无法恢复。",
        TextStyle::Label,
        p.muted,
    );
    button(
        list,
        cancel,
        None,
        "继续编辑",
        state.hover == Some(Hit::DismissCancel),
        p,
        true,
    );
    button(
        list,
        confirm,
        None,
        "放弃修改",
        state.hover == Some(Hit::ConfirmCancel),
        p,
        false,
    );
    if let Some(hit) = state.focused {
        if matches!(hit, Hit::DismissCancel | Hit::ConfirmCancel) {
            let r = if hit == Hit::DismissCancel {
                cancel
            } else {
                confirm
            };
            list.rounded_border(
                Rect::new(r.left - 3.0, r.top - 3.0, r.right + 3.0, r.bottom + 3.0),
                7.0,
                p.foreground,
            );
        }
    }
}

fn is_hovering_card(state: &State, index: usize) -> bool {
    state.hovered_card == Some(index)
}

fn section_header(list: &mut DrawList, rect: Rect, label: &str, detail: Option<&str>, p: &Palette) {
    list.text(
        Rect::new(rect.left + 8.0, rect.top, rect.right - 8.0, rect.bottom),
        label,
        TextStyle::Label,
        p.foreground,
    );
    if let Some(detail) = detail {
        if rect.width() >= 240.0 {
            list.text_aligned(
                Rect::new(rect.left + 90.0, rect.top, rect.right - 8.0, rect.bottom),
                detail,
                TextStyle::Caption,
                p.muted,
                Align::Trailing,
            );
        }
    }
}

fn button(
    list: &mut DrawList,
    rect: Rect,
    icon: Option<Icon>,
    label: &str,
    active: bool,
    p: &Palette,
    primary: bool,
) {
    let fill = if primary {
        if active {
            p.accent_hover
        } else {
            p.accent
        }
    } else if active {
        p.surface_muted
    } else {
        p.surface
    };
    let color = if primary {
        p.button_foreground()
    } else {
        p.foreground
    };
    if primary {
        list.rounded_rect(rect, 8.0, fill);
    } else {
        list.rounded_rect(rect, 5.0, fill);
        list.rounded_border(rect, 5.0, if active { p.accent } else { p.border });
    }
    let left = if icon.is_some() {
        rect.left + 8.0
    } else {
        rect.left
    };
    if let Some(icon) = icon {
        list.icon_centered(
            Rect::from_size(
                rect.left + 6.0,
                rect.top + (rect.height() - 20.0) / 2.0,
                20.0,
                20.0,
            ),
            icon,
            14.0,
            color,
        );
    }
    if !label.is_empty() {
        let text_rect = Rect::new(
            left + if icon.is_some() { 22.0 } else { 0.0 },
            rect.top,
            rect.right - 6.0,
            rect.bottom,
        );
        list.text_aligned(
            text_rect,
            label,
            TextStyle::Caption,
            color,
            if icon.is_some() {
                Align::Leading
            } else {
                Align::Center
            },
        );
    }
}

fn icon_toggle(list: &mut DrawList, rect: Rect, icon: Icon, selected: bool, p: &Palette) {
    list.rounded_rect(rect, 5.0, if selected { p.surface } else { p.surface });
    list.rounded_border(rect, 5.0, if selected { p.accent } else { p.border });
    list.icon_centered(rect, icon, 15.0, if selected { p.accent } else { p.muted });
}

fn size_label(preset: CardSizePreset) -> &'static str {
    match preset {
        CardSizePreset::Compact => "紧凑 400×400",
        CardSizePreset::Medium => "标准 480×480",
        CardSizePreset::Spacious => "宽松 560×600",
    }
}

fn module_icon(module: ModuleKind) -> Icon {
    match module {
        ModuleKind::Home => Icon::HOME,
        ModuleKind::Schedule => Icon::CALENDAR_DAYS,
        ModuleKind::Inbox => Icon::INBOX,
        ModuleKind::QuickNote => Icon::PENCIL_LINE,
        ModuleKind::Recent => Icon::HISTORY,
        ModuleKind::Favorites => Icon::STAR,
        ModuleKind::Knowledge => Icon::BOOK_OPEN,
        ModuleKind::Document => Icon::FILE_TEXT,
        ModuleKind::Base => Icon::TABLE,
        ModuleKind::Canvas => Icon::LAYOUT_GRID,
        ModuleKind::QuickNav => Icon::LINK2,
        ModuleKind::English => Icon::BOOK_OPEN,
        ModuleKind::Exam => Icon::FILE_TEXT,
        ModuleKind::Ai => Icon::BOT,
        ModuleKind::Templates => Icon::FILE_PLUS,
        ModuleKind::Automations => Icon::GIT_BRANCH,
        ModuleKind::Pomodoro => Icon::TIMER,
        ModuleKind::AgentConfig => Icon::SETTINGS2,
        ModuleKind::Custom | ModuleKind::Shortcuts => Icon::LAYOUT_GRID,
        ModuleKind::Clock => Icon::CLOCK,
        ModuleKind::Folder => Icon::FOLDER_OPEN,
        ModuleKind::Weather => Icon::SUN,
        ModuleKind::Music => Icon::PLAY,
        ModuleKind::Search => Icon::SEARCH,
    }
}

fn tab_surface(list: &mut DrawList, rect: Rect, selected: bool, p: &Palette) {
    if selected {
        list.rounded_rect(rect, 7.0, p.surface_muted);
        list.rounded_border(rect, 7.0, p.border);
    }
}

fn paint_tooltip(list: &mut DrawList, state: &State, layout: &Layout, p: &Palette) {
    let Some(hit) = state.hover.or(state.focused) else {
        return;
    };
    let label = match hit {
        Hit::AddPage => "添加分页",
        Hit::Import => "导入布局",
        Hit::ExportAll => "导出布局",
        Hit::Cancel => "放弃未保存的修改",
        Hit::Module(module) => module.description(),
        Hit::Card(i) => state
            .config
            .cards
            .get(i)
            .map(|c| c.title.as_str())
            .unwrap_or(""),
        _ => return,
    };
    let rect = layout
        .controls
        .iter()
        .find(|(_, h)| *h == hit)
        .map(|(r, _)| *r)
        .or_else(|| {
            layout
                .module_rows
                .iter()
                .find(|(_, m)| hit == Hit::Module(*m))
                .map(|(r, _)| *r)
        });
    let rect = rect.or_else(|| {
        layout
            .card_rows
            .iter()
            .find(|(_, i)| hit == Hit::Card(*i))
            .map(|(r, _)| *r)
    });
    let Some(rect) = rect else {
        return;
    };
    let width = (label.chars().count() as f32 * 12.0 + 20.0).min(layout.frame.width() - 16.0);
    let left = rect
        .left
        .min(layout.frame.right - width - 8.0)
        .max(layout.frame.left + 8.0);
    let top = if rect.bottom + 36.0 > layout.frame.bottom {
        rect.top - 34.0
    } else {
        rect.bottom + 6.0
    };
    let tip = Rect::from_size(left, top, width, 28.0);
    list.rounded_rect(tip, 6.0, p.surface);
    list.rounded_border(tip, 6.0, p.border);
    list.text_aligned(
        tip,
        text::ellipsize(label, TextStyle::Caption, width - 12.0),
        TextStyle::Caption,
        p.foreground,
        Align::Center,
    );
}
