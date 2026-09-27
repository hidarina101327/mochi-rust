//! 工作区共用的窗口框架和全局主按钮样式。
use super::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::{self, Palette},
};

pub fn column(area: Rect, max_width: f32) -> Rect {
    let pad = if area.width() < 600.0 { 20.0 } else { 32.0 };
    let width = (area.width() - pad * 2.0).max(0.0).min(max_width);
    let left = area.left + (area.width() - width) / 2.0;
    Rect::new(left, area.top + 28.0, left + width, area.bottom - 24.0)
}

pub fn button(
    list: &mut DrawList,
    r: Rect,
    label: &str,
    icon: Option<Icon>,
    primary: bool,
    hovered: bool,
    p: &Palette,
) {
    let color = if primary {
        p.button_foreground()
    } else {
        p.foreground
    };
    if primary {
        list.glass_button(r, 7.0, p, hovered);
    } else {
        if hovered {
            list.rounded_rect(r, 7.0, p.surface_muted);
        }
        list.rounded_border(r, 7.0, p.border);
    }
    let label_area = if let Some(icon) = icon {
        list.icon_centered(
            Rect::new(r.left + 10.0, r.top, r.left + 28.0, r.bottom),
            icon,
            16.0,
            color,
        );
        Rect::new(r.left + 34.0, r.top, r.right - 10.0, r.bottom)
    } else {
        r
    };
    list.text_aligned(label_area, label, TextStyle::Label, color, Align::Center);
}

pub fn tab(list: &mut DrawList, r: Rect, label: &str, active: bool, hovered: bool, p: &Palette) {
    if active || hovered {
        list.rounded_rect(
            r,
            6.0,
            theme::mix(
                p.surface_muted,
                p.area_main_default,
                if active { 0.85 } else { 0.5 },
            ),
        );
    }
    list.text_aligned(
        r,
        text::ellipsize(label, TextStyle::Label, (r.width() - 16.0).max(0.0)),
        TextStyle::Label,
        if active { p.foreground } else { p.muted },
        Align::Center,
    );
    if active {
        list.hline(r.left + 10.0, r.right - 10.0, r.bottom - 1.0, p.foreground);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{agenda, agent_config, settings, template_picker, templates, views};

    #[test]
    fn settings_search_and_close_have_separate_hit_targets_at_every_width() {
        for width in [300.0, 400.0, 559.0, 560.0, 800.0, 1120.0] {
            let area = Rect::from_size(31.0, 47.0, width, 760.0);
            for tab in ["general", "customization", "version-history", "ai"] {
                let lay = settings::content_layout(area, tab, None, 0.0);
                let close = settings::close_rect(area);
                assert!(
                    lay.search_rect.intersect(&close).is_empty(),
                    "{width} {tab}"
                );
                assert!(lay.search_rect.left >= area.left + 24.0);
                assert!(
                    lay.search_rect.right <= close.left - 12.0
                        || lay.search_rect.top >= close.bottom + 8.0
                );
                assert!(lay.body.top >= lay.search_rect.bottom + 8.0);
            }
        }
    }

    #[test]
    fn picker_keyboard_scrolls_selection_into_view_and_keeps_footer_safe() {
        let area = Rect::from_size(10.0, 20.0, 380.0, 540.0);
        let items = (0..40)
            .map(|i| mochi_core::templates::Template {
                name: format!("模板{i}"),
                group: "学习".into(),
                path: format!("{i}.md").into(),
            })
            .collect();
        let mut picker = template_picker::Picker::new(Default::default(), items);
        for _ in 0..41 {
            picker.navigate(area, false);
            let lay = picker.layout(area);
            let r = lay
                .rows
                .iter()
                .find(|(_, h)| Some(*h) == picker.focused)
                .unwrap()
                .0;
            assert!(r.top >= lay.content.top - 0.1 && r.bottom <= lay.content.bottom + 0.1);
            assert_eq!(
                picker.hit(area, r.left + 4.0, r.top + 4.0),
                picker.focused.unwrap()
            );
        }
        picker.navigate(area, false);
        assert_eq!(picker.focused, Some(template_picker::Hit::Cancel));
        let lay = picker.layout(area);
        assert_eq!(
            picker.hit(area, lay.close.left + 8.0, lay.close.top + 8.0),
            template_picker::Hit::Cancel
        );
        picker.navigate(area, false);
        assert_eq!(picker.focused, Some(template_picker::Hit::Blank));
        assert_eq!(picker.scroll, 0.0);
    }

    #[test]
    fn agenda_toolbar_actions_do_not_overlap_when_narrow() {
        use agenda::{Hit, View};
        let palette = crate::ui::theme::configured_palette(false);
        let mut state = agenda::State {
            data: Some(Default::default()),
            ..Default::default()
        };
        for width in [320.0, 420.0, 620.0, 960.0, 1440.0] {
            for view in View::ALL {
                state.set_view(view);
                for todo_month in [false, true] {
                    state.todo_month = todo_month;
                    let area = Rect::from_size(0.0, 0.0, width, 900.0);
                    let mut list = crate::ui::draw::DrawList::default();
                    let lay = agenda::paint(&mut list, area, &mut state, None, &palette);
                    let toolbar: Vec<_> = lay
                        .entries
                        .iter()
                        .filter(|(r, h)| {
                            r.bottom <= area.top + 52.0
                                && !matches!(
                                    h,
                                    Hit::Blank | Hit::Grid | Hit::DismissToast | Hit::Undo
                                )
                        })
                        .collect();
                    for (i, (r, h)) in toolbar.iter().enumerate() {
                        assert!(
                            r.left >= area.left && r.right <= area.right,
                            "{width} {view:?}: {h:?}"
                        );
                        for (other, oh) in toolbar.iter().skip(i + 1) {
                            assert!(
                                r.intersect(other).is_empty(),
                                "{width} {view:?}: {h:?} overlaps {oh:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn inbox_library_picker_never_overlaps_tabs_or_capture() {
        let state = views::inbox::State {
            libraries: vec![("一个很长很长的知识库名称".into(), Default::default())],
            ..Default::default()
        };
        for width in [320.0, 420.0, 600.0, 1100.0] {
            let area = Rect::from_size(0.0, 0.0, width, 900.0);
            let lay = views::inbox::layout(&state, area);
            let controls: Vec<_> = lay
                .entries
                .iter()
                .filter(|(_, h)| {
                    matches!(
                        h,
                        views::inbox::Hit::Refresh
                            | views::inbox::Hit::Capture
                            | views::inbox::Hit::TabInbox
                            | views::inbox::Hit::TabArchived
                            | views::inbox::Hit::TargetLibrary
                    )
                })
                .collect();
            for (i, (r, h)) in controls.iter().enumerate() {
                for (other, oh) in controls.iter().skip(i + 1) {
                    assert!(r.intersect(other).is_empty(), "{width}: {h:?} {oh:?}");
                }
            }
        }
    }

    #[test]
    fn agent_fields_stack_and_scrolled_sections_cannot_steal_header_clicks() {
        let data = agent_config::Data {
            sections: vec![vec![agent_config::Card {
                name: "研究员".into(),
                description: "整理资料".into(),
                source_path: String::new(),
                read_only: false,
                fields: (0..6)
                    .map(|i| agent_config::Field::Text {
                        label: format!("字段{i}"),
                        value: "值".into(),
                    })
                    .collect(),
            }]],
        };
        let area = Rect::from_size(0.0, 0.0, 420.0, 800.0);
        let lay = agent_config::layout(area, &data, 0, 0.0);
        assert!(lay
            .fields
            .windows(2)
            .all(|pair| pair[0].2.bottom < pair[1].2.top));
        let scrolled = agent_config::layout(area, &data, 0, 300.0);
        let r = scrolled
            .entries
            .iter()
            .find(|(_, h)| *h == agent_config::Hit::New)
            .unwrap()
            .0;
        assert_eq!(
            scrolled.hit(r.left + 4.0, r.top + 4.0),
            Some(agent_config::Hit::New)
        );
    }

    #[test]
    fn refreshed_surfaces_paint_in_both_themes_without_accent_slab() {
        for dark in [false, true] {
            let p = theme::tokens().palette(dark);
            let area = Rect::from_size(0.0, 0.0, 1000.0, 800.0);
            let mut list = DrawList::new();
            button(
                &mut list,
                Rect::from_size(0.0, 0.0, 120.0, 36.0),
                "新建",
                None,
                true,
                false,
                p,
            );
            let picker = template_picker::Picker::new(Default::default(), vec![]);
            picker.paint(&mut list, area, p);
            let state = templates::State::default();
            templates::paint(&mut list, area, &state, &templates::layout(&state, area), p);
            assert!(!list.cmds().iter().any(|cmd| matches!(cmd, super::super::draw::DrawCmd::RoundedRect { color, rect, .. } if *color == p.accent && rect.width() > 200.0)));
            assert!(list.finish().is_ok());
        }
    }
}
