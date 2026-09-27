use super::*;
use crate::ui::{draw::DrawList, layout::Rect, theme};
use mochi_core::desktop_cards::{Card, Module};

#[test]
fn sidebar_card_actions_do_not_overlap_text_or_each_other() {
    let mut state = State::new(&config_with_cards(3));
    for (width, height) in [(1200.0, 800.0), (760.0, 600.0), (520.0, 420.0)] {
        let layout = state.layout(Rect::from_size(0.0, 0.0, width, height));
        for (row, index) in &layout.card_rows {
            let geometry = super::geometry::card_row_layout(*row, *index);
            let separate = |a: Rect, b: Rect| {
                a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top
            };
            for (i, (rect, hit)) in geometry.controls.iter().enumerate() {
                assert!(separate(*rect, geometry.title), "title overlaps {hit:?}");
                assert!(separate(*rect, geometry.status), "status overlaps {hit:?}");
                assert!(
                    rect.left >= row.left
                        && rect.right <= row.right
                        && rect.top >= row.top
                        && rect.bottom <= row.bottom
                );
                for (other, _) in geometry.controls.iter().skip(i + 1) {
                    assert!(separate(*rect, *other));
                }
                if layout.cards_body.contains(rect.left + 1.0, rect.top + 1.0) {
                    state.pointer(&layout, rect.left + 1.0, rect.top + 1.0);
                    let hovered = state.layout(layout.viewport);
                    assert_eq!(hovered.hit(rect.left + 1.0, rect.top + 1.0), Some(*hit));
                }
            }
        }
    }
}

fn config_with_cards(count: usize) -> DesktopConfig {
    let mut config = DesktopConfig::default();
    for index in 0..count {
        config
            .cards
            .push(Card::new(format!("卡片 {}", index + 1), Module::Home));
    }
    config
}

#[test]
fn desktop_utility_inputs_commit_to_their_own_pages() {
    let mut config = DesktopConfig::default();
    let mut card = Card::new("桌面工具", Module::Weather);
    card.pages
        .push(mochi_core::desktop_cards::Page::new(Module::Search));
    config.cards.push(card);
    let mut state = State::new(&config);
    state.activate(Hit::EditPreference(Field::Utility));
    state.preference_text.set_text("深圳");
    state.activate(Hit::Page(1));
    state.activate(Hit::EditPreference(Field::Utility));
    state.preference_text.set_text("项目计划");
    state.activate(Hit::Page(0));
    assert_eq!(state.config.cards[0].pages[0].utility.location, "深圳");
    assert_eq!(state.config.cards[0].pages[1].utility.query, "项目计划");
    assert!(state.config.cards[0].pages[0].utility.query.is_empty());
    assert!(config.cards[0].pages[0].utility.location.is_empty());
}

#[test]
fn desktop_utility_input_is_visible_and_hit_testable() {
    for module in [Module::Weather, Module::Search] {
        let mut config = DesktopConfig::default();
        config.cards.push(Card::new(module.label(), module));
        let state = State::new(&config);
        for (width, height) in [(820.0, 640.0), (560.0, 520.0)] {
            let layout = state.layout(Rect::from_size(0.0, 0.0, width, height));
            let (rect, _) = layout
                .controls
                .iter()
                .find(|(_, hit)| *hit == Hit::EditPreference(Field::Utility))
                .unwrap();
            assert_eq!(
                layout.hit(rect.left + 2.0, rect.top + 2.0),
                Some(Hit::EditPreference(Field::Utility))
            );
            assert!(layout
                .options_view
                .contains(rect.right - 1.0, rect.bottom - 1.0));
        }
    }
}

#[test]
fn manager_layout_stays_inside_small_viewports() {
    let state = State::new(&config_with_cards(3));
    for (width, height) in [
        (1280.0, 800.0),
        (760.0, 600.0),
        (520.0, 420.0),
        (320.0, 260.0),
    ] {
        let viewport = Rect::from_size(0.0, 0.0, width, height);
        let layout = state.layout(viewport);
        assert!(layout.frame.left >= viewport.left);
        assert!(layout.frame.top >= viewport.top);
        assert!(layout.frame.right <= viewport.right + 0.01);
        assert!(layout.frame.bottom <= viewport.bottom + 0.01);
        assert!(layout.cards_body.left >= layout.cards_pane.left);
        assert!(layout.options_body.right <= layout.right_pane.right + 0.01);
    }
}

#[test]
fn header_toolbar_controls_do_not_overlap_or_escape_the_frame() {
    let state = State::new(&config_with_cards(1));
    for viewport in [
        Rect::from_size(0.0, 0.0, 900.0, 700.0),
        Rect::from_size(0.0, 0.0, 600.0, 500.0),
    ] {
        let layout = state.layout(viewport);
        let header_hits = [Hit::NewCard, Hit::Import, Hit::ExportAll, Hit::Close];
        let mut rects = Vec::new();
        for hit in header_hits {
            let rect = layout
                .controls
                .iter()
                .find(|(rect, candidate)| *candidate == hit && rect.top < layout.header.bottom)
                .map(|(rect, _)| *rect)
                .expect("header control");
            assert!(layout.frame.contains(rect.left, rect.top));
            assert!(layout.frame.contains(rect.right - 0.01, rect.bottom - 0.01));
            rects.push(rect);
        }
        for (index, left) in rects.iter().enumerate() {
            for right in rects.iter().skip(index + 1) {
                assert!(
                    left.right <= right.left || right.right <= left.left,
                    "header controls overlap: {left:?} and {right:?}"
                );
            }
        }
    }
}

#[test]
fn page_delete_confirmation_is_clickable_where_it_is_painted() {
    let mut state = State::new(&config_with_cards(1));
    state.activate(Hit::AddPage);
    state.activate(Hit::Module(Module::Recent));
    state.activate(Hit::PageDelete(1));
    let layout = state.layout(Rect::from_size(0.0, 0.0, 760.0, 600.0));
    let (_, _, confirm) = geometry::confirmation_rects(layout.frame);
    let x = confirm.left + 12.0;
    let y = confirm.top + 12.0;
    assert_eq!(layout.hit(x, y), Some(Hit::ConfirmDelete));
    let action = state.click(&layout, x, y);
    assert!(action.is_none());
    assert_eq!(state.config.cards[0].pages.len(), 1);
}

#[test]
fn manager_supports_new_cards_pages_and_active_page_reordering() {
    let mut state = State::new(&DesktopConfig::default());
    state.activate(Hit::NewCard);
    assert!(state.config.cards.is_empty());
    state.activate(Hit::Module(Module::Home));
    assert_eq!(state.config.cards.len(), 1);
    assert_eq!(state.selected_card, Some(0));
    state.activate(Hit::AddPage);
    state.activate(Hit::Module(Module::Recent));
    assert_eq!(state.config.cards[0].pages.len(), 2);
    let second = state.config.cards[0].pages[1].id.clone();
    assert_eq!(state.config.cards[0].active_page, second);
    state.activate(Hit::Page(0));
    assert_eq!(
        state.config.cards[0].active_page,
        state.config.cards[0].pages[0].id
    );
    state.activate(Hit::PageDown(0));
    assert_eq!(state.selected_page, Some(1));
    assert_eq!(
        state.config.cards[0].active_page,
        state.config.cards[0].pages[1].id
    );
}

#[test]
fn module_options_are_per_page_and_source_is_delegated() {
    let mut state = State::new(&config_with_cards(1));
    let before = state.config.cards[0].pages[0].options.clone();
    state.activate(Hit::NewCard);
    let action = state.activate(Hit::Module(Module::Document));
    assert_eq!(state.config.cards[1].pages[0].module, Module::Document);
    assert_ne!(before, state.config.cards[1].pages[0].options);
    assert!(action.is_none());
    let options = Module::Document.options();
    let first = options.first().unwrap().key.clone();
    let was_enabled = state.config.cards[1].pages[0].selected(&first);
    state.activate(Hit::Option(0));
    assert_eq!(
        state.config.cards[1].pages[0].selected(&first),
        !was_enabled
    );
    assert!(matches!(
        state.activate(Hit::Source),
        Some(Action::ChooseSource { .. })
    ));
}

#[test]
fn existing_page_template_is_immutable() {
    let mut state = State::new(&config_with_cards(1));
    let before = state.selected_page_ref().unwrap().clone();
    state.activate(Hit::Module(Module::Document));
    assert_eq!(state.selected_page_ref(), Some(&before));
    let layout = state.layout(Rect::from_size(0.0, 0.0, 1200.0, 800.0));
    assert!(layout.module_rows.is_empty());
}

#[test]
fn duplicate_card_and_page_make_fresh_ids_without_losing_templates() {
    let mut state = State::new(&config_with_cards(1));
    let card_id = state.config.cards[0].id.clone();
    let page_id = state.config.cards[0].pages[0].id.clone();
    state.activate(Hit::CardDuplicate(0));
    assert_eq!(state.config.cards.len(), 2);
    assert_ne!(state.config.cards[1].id, card_id);
    assert_eq!(state.config.cards[1].pages[0].module, Module::Home);
    state.focused = Some(Hit::Page(0));
    state.key(b'D' as u16, false, true);
    assert_eq!(state.config.cards[1].pages.len(), 2);
    assert_ne!(state.config.cards[1].pages[1].id, page_id);
    assert!(state.config.cards[1].pages[1].title.contains("副本"));
}

#[test]
fn delete_requires_confirmation_and_cancel_warns_when_dirty() {
    let mut state = State::new(&config_with_cards(2));
    state.activate(Hit::CardDelete(0));
    assert_eq!(state.confirm_delete, Some(DeleteTarget::Card(0)));
    state.activate(Hit::CancelDelete);
    assert!(state.confirm_delete.is_none());
    state.activate(Hit::CardVisible(0));
    assert!(state.dirty);
    assert_eq!(state.key(0x1b, false, false), None);
    assert!(state.confirm_cancel);
    assert_eq!(state.activate(Hit::DismissCancel), None);
    assert!(!state.confirm_cancel);
    assert!(matches!(state.activate(Hit::Cancel), None));
    assert!(state.confirm_cancel);
    assert_eq!(state.activate(Hit::ConfirmCancel), Some(Action::Cancel));
}

#[test]
fn field_delete_and_enter_are_routed_to_text_field() {
    let mut state = State::new(&config_with_cards(1));
    let layout = state.layout(Rect::from_size(0.0, 0.0, 900.0, 700.0));
    state.click(
        &layout,
        layout.card_name.left + 12.0,
        layout.card_name.top + 12.0,
    );
    state.char('X');
    let after_char = state.card_name.text().to_owned();
    state.key(0x2e, false, false);
    assert!(state.card_name.text().len() < after_char.len());
    let after_delete = state.card_name.text().to_owned();
    state.key(0x08, false, false);
    assert!(state.card_name.text().len() <= after_delete.len());
    state.key(0x0d, false, false);
    assert!(state.focus_field.is_none());
}

#[test]
fn paint_is_valid_in_both_themes_and_empty_state_has_a_cta() {
    for dark in [false, true] {
        let mut state = State::new(&DesktopConfig::default());
        let viewport = Rect::from_size(0.0, 0.0, 760.0, 600.0);
        let layout = state.layout(viewport);
        let palette = theme::configured_palette(dark);
        let mut list = DrawList::new();
        state.paint(&mut list, &layout, viewport, &palette);
        assert!(list.finish().is_ok());
        assert!(!list.is_empty());
    }
}

#[test]
fn blank_inside_manager_does_not_dismiss_and_tab_renames_inline() {
    let mut state = State::new(&config_with_cards(1));
    let layout = state.layout(Rect::from_size(0.0, 0.0, 1200.0, 900.0));
    assert_eq!(
        layout.hit(layout.right_body.left + 10.0, layout.right_body.top + 78.0),
        None
    );
    assert_eq!(
        state.click(
            &layout,
            layout.right_body.left + 10.0,
            layout.right_body.top + 78.0
        ),
        None
    );
    assert_eq!(layout.hit(1.0, 1.0), Some(Hit::Backdrop));
    let r = layout.page_rows[0].0;
    state.double_click(&layout, r.left + 12.0, r.top + 10.0);
    assert_eq!(state.focus_field, Some(Field::PageName));
    state.char('新');
    state.char('页');
    state.key(13, false, false);
    assert_eq!(state.config.cards[0].pages[0].title, "新页");
    assert_eq!(state.focus_field, None);
}
#[test]
fn appearance_limits_and_draft_cancel_preserve_original() {
    let original = config_with_cards(1);
    let mut state = State::new(&original);
    for _ in 0..40 {
        state.activate(Hit::Opacity(-5));
        state.activate(Hit::FontSize(2));
    }
    assert_eq!(state.config.cards[0].appearance.opacity, 35);
    assert_eq!(state.config.cards[0].appearance.font_size, 72);
    state.activate(Hit::FontColor(Some(0xffffff)));
    assert_eq!(original.cards[0].appearance.font_color, None);
    assert!(state.config.validate().is_ok());
}

#[test]
fn track_clicks_map_to_values_and_keyboard_steps_without_resetting() {
    let original = config_with_cards(1);
    let mut state = State::new(&original);
    state.editor_tab = 0;
    let layout = state.layout(Rect::from_size(0.0, 0.0, 760.0, 600.0));
    for (row, expected) in [(0, [35, 68, 100]), (1, [8, 40, 72])] {
        let b = layout.right_body;
        let track = appearance::track_rect(Rect::new(b.left, b.top + 40.0, b.right, b.bottom), row);
        for (fraction, value) in [0.0, 0.5, 0.99999].into_iter().zip(expected) {
            state.click(
                &layout,
                track.left + track.width() * fraction,
                track.top + 9.0,
            );
            let appearance = &state.config.cards[0].appearance;
            assert_eq!(
                if row == 0 {
                    appearance.opacity
                } else {
                    appearance.font_size
                },
                value
            );
        }
        state.key(0x25, false, false);
        let value = if row == 0 {
            state.config.cards[0].appearance.opacity
        } else {
            state.config.cards[0].appearance.font_size
        };
        assert_eq!(value, expected[2] - 1);
        state.key(0x0d, false, false);
        assert_eq!(
            if row == 0 {
                state.config.cards[0].appearance.opacity
            } else {
                state.config.cards[0].appearance.font_size
            },
            value
        );
        state.key(0x09, false, false);
        assert_eq!(
            state.focused,
            Some(if row == 0 {
                Hit::FontSizeTrack(0.0)
            } else {
                Hit::FontColor(None)
            })
        );
    }
    assert_ne!(state.config, original);
    state.dirty = false;
    state.activate(Hit::OpacityTrack(f32::NAN));
    assert!(!state.dirty);
    state.activate(Hit::OpacityTrack(-1.0));
    assert_eq!(state.config.cards[0].appearance.opacity, 35);
    state.dirty = false;
    state.activate(Hit::Opacity(-1));
    assert!(!state.dirty);
}

#[test]
fn hidden_card_actions_cannot_fire_and_toolbar_stays_open_across_its_buttons() {
    let mut state = State::new(&config_with_cards(2));
    let viewport = Rect::from_size(0.0, 0.0, 760.0, 600.0);
    let layout = state.layout(viewport);
    let row = layout.card_rows[0].0;
    let geometry = geometry::card_row_layout(row, 0);
    let actions: Vec<_> = geometry.controls.iter().skip(2).copied().collect();
    for (rect, _) in &actions {
        assert_eq!(
            layout.hit(rect.left + 1.0, rect.top + 1.0),
            Some(Hit::Card(0))
        );
    }
    state.pointer(&layout, row.left + 10.0, row.top + 10.0);
    for (rect, hit) in &actions {
        let layout = state.layout(viewport);
        state.pointer(&layout, rect.left + 1.0, rect.top + 1.0);
        assert!(state.card_toolbar_visible(0));
        assert_eq!(layout.hit(rect.left + 1.0, rect.top + 1.0), Some(*hit));
    }
    let layout = state.layout(viewport);
    state.pointer(
        &layout,
        layout.right_body.left + 10.0,
        layout.right_body.top + 10.0,
    );
    let layout = state.layout(viewport);
    assert!(!state.card_toolbar_visible(0));
    let (rect, _) = actions[5];
    state.click(&layout, rect.left + 1.0, rect.top + 1.0);
    assert_eq!(state.confirm_delete, None);
    state.set_focus_target(FocusTarget::Hit(Hit::CardDuplicate(0)));
    assert!(state.card_toolbar_visible(0));
}

#[test]
fn appearance_and_fixed_sections_do_not_overlap_click_targets() {
    for (width, height) in [(1200.0, 800.0), (760.0, 600.0), (520.0, 420.0)] {
        let mut state = State::new(&config_with_cards(1));
        state.editor_tab = 0;
        let layout = state.layout(Rect::from_size(0.0, 0.0, width, height));
        let b = layout.right_body;
        let controls = appearance::controls(Rect::new(b.left, b.top + 40.0, b.right, b.bottom));
        for (index, (rect, hit)) in controls.iter().enumerate() {
            assert!(layout.page_rows.is_empty());
            assert!(rect.right <= layout.right_body.right);
            for (other, other_hit) in controls.iter().skip(index + 1) {
                assert!(
                    rect.right <= other.left
                        || other.right <= rect.left
                        || rect.bottom <= other.top
                        || other.bottom <= rect.top,
                    "{hit:?} overlaps {other_hit:?}"
                );
            }
        }
        assert!(
            layout.options_view.top >= layout.options_body.top + 26.0
                || layout.options_view.is_empty()
        );
        for (rect, hit) in &layout.controls {
            if matches!(hit, Hit::Route(_) | Hit::Save | Hit::Cancel) {
                assert_eq!(rect.height(), 28.0);
            }
        }
    }
}

/// 可选的原生渲染测试夹具；只写 PNG，不创建任何应用窗口。
#[test]
#[ignore = "visual review: cargo test -p mochi-app desktop_cards::tests::render_manager_review -- --ignored"]
fn render_manager_review() {
    use windows::Win32::{
        Foundation::HWND,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
    };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .unwrap();
    let _com = Com;
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/desktop-verification/ui-refactor");
    std::fs::create_dir_all(&output).unwrap();
    for (name, width, height, dark, hover, module) in [
        (
            "organizer-editor",
            1200,
            800,
            false,
            false,
            Module::Shortcuts,
        ),
        (
            "organizer-settings",
            1200,
            800,
            false,
            false,
            Module::Shortcuts,
        ),
        (
            "organizer-narrow",
            760,
            600,
            false,
            false,
            Module::Shortcuts,
        ),
        (
            "document-sources",
            1200,
            800,
            false,
            false,
            Module::Document,
        ),
        ("studio", 1200, 800, false, false, Module::Clock),
        ("studio-picker", 1200, 800, false, false, Module::Clock),
        ("studio-dark", 1200, 800, true, false, Module::Clock),
        ("studio-narrow", 760, 600, false, false, Module::Clock),
        ("templates", 1200, 800, false, false, Module::Home),
        ("home-choices", 1200, 800, false, false, Module::Home),
        ("appearance", 1200, 800, false, false, Module::Home),
        ("light", 1200, 800, false, false, Module::Schedule),
        ("delete", 1200, 800, false, false, Module::Home),
        ("empty", 1200, 800, false, false, Module::Home),
        ("preferences", 1200, 800, false, false, Module::Home),
        ("rules", 1200, 800, false, false, Module::Schedule),
        ("dark", 1200, 800, true, false, Module::Schedule),
        ("hover", 1200, 800, false, true, Module::Schedule),
        ("discard", 1200, 800, false, false, Module::Schedule),
        ("custom", 1200, 800, false, false, Module::Inbox),
        ("narrow", 760, 600, false, true, Module::Schedule),
        ("narrow-dark", 760, 600, true, true, Module::Document),
    ] {
        let mut config = config_with_cards(3);
        config.cards[0].title = "今日工作台".into();
        config.cards[0].pages = vec![DesktopPage::new(module), DesktopPage::new(Module::Inbox)];
        config.cards[0].active_page = config.cards[0].pages[0].id.clone();
        config.cards[1].title = "阅读与收藏".into();
        config.cards[2].title = "学习进度".into();
        if name == "custom" {
            config.cards[0].appearance.background_color = Some(0xd6e5ee);
        }
        if name == "empty" {
            config.cards.clear();
        }
        let mut state = State::new(&config);
        if name.starts_with("organizer") {
            let page = state.selected_page_mut().unwrap();
            page.studio.add_shortcuts(&[
                "C:/Windows/notepad.exe".into(),
                "C:/Windows/explorer.exe".into(),
                "C:/Windows/System32/calc.exe".into(),
            ]);
            page.presentation.columns = 3;
            page.presentation.grid_lines = true;
            state.studio_editor.selected = Some(0);
            state.editor_tab = 1;
            state.preferences_mode = name == "organizer-settings";
        }
        if name == "document-sources" {
            let page = state.selected_page_mut().unwrap();
            page.sources = vec!["知识库/设计".into(), "知识库/工作/计划.md".into()];
            page.presentation.show_modified = true;
            state.preferences_mode = true;
        }
        if name.starts_with("studio") {
            state.editor_tab = 2;
            state.studio_editor.selected = Some(0);
        }
        if name == "studio-picker" {
            state.activate(Hit::Studio(studio::Command::EventAdd));
            state.activate(Hit::Studio(studio::Command::Trigger));
        }
        if name == "templates" {
            state.activate(Hit::AddPage);
        }
        if name == "appearance" {
            state.editor_tab = 0;
        }
        if name == "delete" {
            state.activate(Hit::CardDelete(0));
        }
        if name == "preferences" || name == "rules" {
            state.preferences_mode = true;
            state.editor_tab = 1;
        }
        if name == "rules" {
            state
                .selected_page_mut()
                .unwrap()
                .presentation
                .schedule_view = 2;
            state.options_scroll = 400.0;
        }
        if name == "discard" {
            state.dirty = true;
            state.activate(Hit::Cancel);
        }
        let viewport = Rect::from_size(0.0, 0.0, width as f32, height as f32);
        let layout = state.layout(viewport);
        if hover {
            let (rect, _) = geometry::card_row_layout(layout.card_rows[0].0, 0).controls[2];
            state.pointer(&layout, rect.left + 10.0, rect.top + 10.0);
            let layout = state.layout(viewport);
            state.pointer(&layout, rect.left + 10.0, rect.top + 10.0);
        }
        let layout = state.layout(viewport);
        let palette = theme::configured_palette(dark);
        let mut list = DrawList::new();
        state.paint(&mut list, &layout, viewport, &palette);
        list.finish().unwrap();
        let mut renderer = crate::gfx::Renderer::new().unwrap();
        let snapshot = renderer
            .prepare_snapshot(width * 3 / 2, height * 3 / 2, 144.0)
            .unwrap();
        renderer
            .present(HWND::default(), palette.surface, &list)
            .unwrap();
        renderer
            .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn discard_modal_traps_input_and_escape_keeps_draft() {
    let mut state = State::new(&config_with_cards(1));
    state.activate(Hit::BackgroundColor(Some(0xd6e5ee)));
    state.activate(Hit::ToggleDetails);
    let draft = state.config.clone();
    state.activate(Hit::Cancel);
    let layout = state.layout(Rect::from_size(0.0, 0.0, 1200.0, 800.0));
    assert_eq!(
        layout.hit(layout.card_name.left + 2.0, layout.card_name.top + 2.0),
        None
    );
    assert!(!state.pointer_down(
        &layout,
        layout.card_name.left + 2.0,
        layout.card_name.top + 2.0
    ));
    assert_eq!(state.focused, Some(Hit::DismissCancel));
    state.key(9, false, false);
    assert_eq!(state.focused, Some(Hit::ConfirmCancel));
    state.key(9, false, false);
    assert_eq!(state.focused, Some(Hit::DismissCancel));
    assert_eq!(state.key(27, false, false), None);
    assert!(!state.confirm_cancel);
    assert_eq!(state.config, draft);
    state.activate(Hit::Cancel);
    let layout = state.layout(Rect::from_size(0.0, 0.0, 1200.0, 800.0));
    let (_, _, confirm) = geometry::confirmation_rects(layout.frame);
    assert_eq!(
        state.click(&layout, confirm.left + 10.0, confirm.top + 10.0),
        Some(Action::Cancel)
    );
}

#[test]
fn short_manager_switches_appearance_and_pages_tabs() {
    let mut state = State::new(&config_with_cards(1));
    let viewport = Rect::from_size(0.0, 0.0, 760.0, 600.0);
    let layout = state.layout(viewport);
    assert!(!layout.appearance_open);
    assert!(layout.options_view.height() >= 100.0);
    let toggle = layout
        .controls
        .iter()
        .find(|(_, h)| *h == Hit::EditorTab(0))
        .unwrap()
        .0;
    state.click(&layout, toggle.left + 4.0, toggle.top + 4.0);
    let expanded = state.layout(viewport);
    assert!(expanded.appearance_open);
    assert!(expanded
        .controls
        .iter()
        .any(|(_, h)| *h == Hit::BackgroundCustom));
    state.activate(Hit::EditorTab(1));
    assert!(!state.layout(viewport).appearance_open);
    assert!(!state.dirty);
}

#[test]
fn desktop_sliders_drag_outside_track_and_wheel_changes_values() {
    let mut state = State::new(&config_with_cards(1));
    state.editor_tab = 0;
    let layout = state.layout(Rect::from_size(0.0, 0.0, 1200.0, 800.0));
    for row in 0..2 {
        let b = layout.right_body;
        let r = appearance::track_rect(Rect::new(b.left, b.top + 40.0, b.right, b.bottom), row);
        assert!(state.pointer_down(&layout, r.left + 10.0, r.top + 10.0));
        assert!(state.dragging());
        state.pointer(&layout, r.right + 100.0, r.top - 30.0);
        let a = &state.config.cards[0].appearance;
        assert_eq!(
            if row == 0 { a.opacity } else { a.font_size },
            if row == 0 { 100 } else { 72 }
        );
        state.pointer_up();
        assert!(!state.dragging());
        state.wheel(&layout, r.left + 20.0, r.top + 10.0, -120.0);
        let a = &state.config.cards[0].appearance;
        assert_eq!(
            if row == 0 { a.opacity } else { a.font_size },
            if row == 0 { 99 } else { 71 }
        );
    }
}

#[test]
fn desktop_preferences_do_not_toggle_content_and_save_keeps_editor_open() {
    let mut state = State::new(&config_with_cards(1));
    state.editor_tab = 1;
    state.activate(Hit::PreferencesTab(true));
    let viewport = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
    let layout = state.layout(viewport);
    let options = state.config.cards[0].pages[0].options.clone();
    state.click(
        &layout,
        layout.options_view.left + 10.0,
        layout.options_view.top + 20.0,
    );
    assert_eq!(state.config.cards[0].pages[0].options, options);
    state.activate(Hit::Preference(preferences::Setting::Padding, -1));
    assert!(matches!(
        state.activate(Hit::SaveKeepOpen),
        Some(Action::SaveKeepOpen(_))
    ));
    assert!(matches!(
        state.key(b'S' as u16, false, true),
        Some(Action::SaveKeepOpen(_))
    ));
    state.activate(Hit::LimitAll);
    assert_eq!(state.selected_page_ref().unwrap().limit, 0);
    state.focused = Some(Hit::Card(0));
    state.key(0x2e, false, false);
    assert!(state.confirm_delete.is_some());
    let draft = state.config.clone();
    let layout = state.layout(viewport);
    assert!(layout.modal);
    assert!(!state.wheel(
        &layout,
        layout.options_view.left + 1.0,
        layout.options_view.top + 1.0,
        -100.0
    ));
    state.key(9, false, false);
    assert_eq!(state.focused, Some(Hit::ConfirmDelete));
    state.key(27, false, false);
    assert_eq!(state.config, draft);
}

#[test]
fn desktop_studio_drag_resize_undo_and_event_roundtrip() {
    use mochi_core::desktop_cards::studio::{Kind, Trigger};
    let mut config = DesktopConfig::default();
    config.cards.push(Card::new("设计", Module::Custom));
    let mut s = State::new(&config);
    s.activate(Hit::EditorTab(2));
    s.activate(Hit::Studio(studio::Command::Add(Kind::Clock)));
    let layout = s.layout(Rect::from_size(0.0, 0.0, 1200.0, 900.0));
    let r = studio::canvas(&s, layout.right_body);
    let n = s.selected_page_ref().unwrap().studio.nodes[0].clone();
    let nr = crate::desktop_window::widgets::node_rect(&n, r);
    assert!(s.pointer_down(&layout, nr.left + 12.0, nr.top + 12.0));
    assert!(s.dragging());
    s.pointer(&layout, nr.left + 40.0, nr.top + 48.0);
    s.pointer_up();
    assert_ne!(s.selected_page_ref().unwrap().studio.nodes[0].y, n.y);
    s.key(90, false, true);
    assert_eq!(s.selected_page_ref().unwrap().studio.nodes[0].y, n.y);
    s.activate(Hit::Studio(studio::Command::Select(0)));
    s.activate(Hit::Studio(studio::Command::EventAdd));
    s.activate(Hit::Studio(studio::Command::Trigger));
    assert_eq!(
        s.selected_page_ref().unwrap().studio.nodes[0].events[0].trigger,
        Trigger::Click
    );
    s.activate(Hit::Studio(studio::Command::PickTrigger(
        Trigger::DoubleClick,
    )));
    assert_eq!(
        s.selected_page_ref().unwrap().studio.nodes[0].events[0].trigger,
        Trigger::DoubleClick
    );
    let encoded = serde_json::to_string(&s.config).unwrap();
    let imported = DesktopConfig::import_json(&encoded).unwrap();
    assert_eq!(imported.cards[0].pages, s.config.cards[0].pages);
    assert!(!imported.cards[0].enabled);
    s.activate(Hit::Studio(studio::Command::Add(Kind::AiChat)));
    s.activate(Hit::Studio(studio::Command::Add(Kind::AiChat)));
    s.activate(Hit::Studio(studio::Command::Duplicate));
    assert_eq!(
        s.selected_page_ref()
            .unwrap()
            .studio
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::AiChat)
            .count(),
        1
    );
}
#[test]
fn desktop_editor_tabs_never_hit_hidden_page_controls() {
    let mut s = State::new(&config_with_cards(1));
    s.activate(Hit::EditorTab(0));
    let l = s.layout(Rect::from_size(0.0, 0.0, 1200.0, 800.0));
    assert!(!l
        .controls
        .iter()
        .any(|(_, h)| matches!(h, Hit::Page(_) | Hit::AddPage | Hit::PreferencesTab(_))));
    s.activate(Hit::EditorTab(1));
    let l = s.layout(l.viewport);
    assert!(!l.controls.iter().any(|(_, h)| matches!(
        h,
        Hit::FontSizeTrack(_) | Hit::OpacityTrack(_) | Hit::CardSize(_)
    )));
}

#[test]
fn desktop_preference_hover_paints_value_above_background_and_templates_exclude_english() {
    use crate::ui::{draw::DrawCmd, theme};
    let mut cfg = config_with_cards(1);
    cfg.cards[0].pages[0] = DesktopPage::new(Module::Schedule);
    let mut s = State::new(&cfg);
    s.preferences_mode = true;
    s.hover = Some(Hit::Preference(preferences::Setting::ScheduleView, 1));
    let mut list = crate::ui::draw::DrawList::new();
    preferences::paint(
        &mut list,
        &mut s,
        Rect::from_size(0.0, 0.0, 600.0, 52.0),
        0,
        &theme::configured_palette(false),
    );
    let value = list
        .cmds()
        .iter()
        .position(|c| matches!(c,DrawCmd::Text{text,..} if text=="日期列表"))
        .unwrap();
    let background = list
        .cmds()
        .iter()
        .position(|c| matches!(c, DrawCmd::RoundedRect { .. }))
        .unwrap();
    assert!(value > background);
    assert!(!Module::ALL.contains(&Module::English));
}
#[test]
fn desktop_studio_selectors_and_color_controls_edit_selected_component() {
    use mochi_core::desktop_cards::studio::{Kind, Trigger};
    let mut cfg = DesktopConfig::default();
    cfg.cards.push(Card::new("设计", Module::Custom));
    let mut s = State::new(&cfg);
    s.editor_tab = 2;
    s.activate(Hit::Studio(studio::Command::Add(Kind::AnalogClock)));
    assert_eq!(
        s.activate(Hit::Studio(studio::Command::Color(true))),
        Some(Action::PickStudioColor(true))
    );
    s.set_studio_color(true, 0x123456);
    assert_eq!(s.studio_color(true), Some(0x123456));
    s.activate(Hit::Studio(studio::Command::EventAdd));
    s.activate(Hit::Studio(studio::Command::Trigger));
    let l = s.layout(Rect::from_size(0.0, 0.0, 1200.0, 800.0));
    let (r, h) = l
        .controls
        .iter()
        .find(|(_, h)| *h == Hit::Studio(studio::Command::PickTrigger(Trigger::Interval)))
        .unwrap();
    let h = *h;
    let (x, y) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    assert_eq!(l.hit(x, y), Some(h));
    s.click(&l, x, y);
    assert_eq!(
        s.selected_page_ref().unwrap().studio.nodes[0].events[0].trigger,
        Trigger::Interval
    );
    assert!(s.studio_editor.picker.is_none());
}

#[test]
fn desktop_studio_preview_keeps_card_proportions_and_scales_text() {
    use mochi_core::desktop_cards::studio::{Kind, Node};
    let mut cfg = DesktopConfig::default();
    let mut card = Card::new("比例", Module::Custom);
    card.width = 480;
    card.height = 720;
    card.appearance.tabs_left = true;
    card.appearance.tabs_ratio = 30;
    let mut node = Node::new(Kind::Text);
    node.title = "比例预览".into();
    node.font_size = 48;
    node.width = 10000;
    card.pages[0].studio.nodes = vec![node];
    cfg.cards.push(card);
    let mut s = State::new(&cfg);
    s.editor_tab = 2;
    let (w, h) = crate::desktop_window::widgets::design_size(&cfg.cards[0], &cfg.cards[0].pages[0]);
    let mut sizes = vec![];
    for width in [1200.0, 760.0] {
        let l = s.layout(Rect::from_size(0.0, 0.0, width, 800.0));
        let r = studio::canvas(&s, l.right_body);
        assert!((r.width() / r.height() - w / h).abs() < 0.01);
        assert_eq!(r, l.studio_canvas);
        let mut list = crate::ui::draw::DrawList::new();
        studio::paint(
            &mut list,
            &mut s,
            l.right_body,
            &crate::ui::theme::configured_palette(false),
        );
        let scale = list
            .cmds()
            .iter()
            .find_map(|c| match c {
                crate::ui::draw::DrawCmd::ScaledText { text, scale, .. } if text == "比例预览" => {
                    Some(*scale)
                }
                _ => None,
            })
            .unwrap();
        sizes.push(scale);
    }
    assert!(sizes[1] < sizes[0]);
    assert_eq!(cfg.cards[0].pages[0].studio.nodes[0].font_size, 48);
}

#[test]
fn desktop_event_picker_captures_keyboard_without_deleting_component() {
    use mochi_core::desktop_cards::studio::{Kind, Trigger};
    let mut cfg = DesktopConfig::default();
    cfg.cards.push(Card::new("事件", Module::Custom));
    let mut s = State::new(&cfg);
    s.editor_tab = 2;
    s.activate(Hit::Studio(studio::Command::Add(Kind::Button)));
    s.activate(Hit::Studio(studio::Command::EventAdd));
    s.activate(Hit::Studio(studio::Command::Trigger));
    s.key(46, false, false);
    assert_eq!(s.selected_page_ref().unwrap().studio.nodes.len(), 1);
    s.key(40, false, false);
    s.key(13, false, false);
    assert_eq!(
        s.selected_page_ref().unwrap().studio.nodes[0].events[1].trigger,
        Trigger::DoubleClick
    );
    assert_eq!(
        s.selected_page_ref().unwrap().studio.nodes[0].events[0].trigger,
        Trigger::Click
    );
    assert!(s.studio_editor.picker.is_none());
}

#[test]
fn desktop_shortcut_editor_exposes_properties_in_pages_and_preserves_sources() {
    let mut config = DesktopConfig::default();
    let mut card = Card::new("图标", Module::Shortcuts);
    card.pages[0]
        .studio
        .add_shortcuts(&["C:/a.lnk".into(), "C:/b.lnk".into()]);
    config.cards.push(card);
    let mut state = State::new(&config);
    state.activate(Hit::ShortcutSelect(0));
    assert!(!studio::enabled(&state));
    assert_eq!(state.editor_tab, 1);
    state.activate(Hit::EditPreference(Field::ShortcutName(0)));
    state.preference_text.set_text("常用程序");
    state.commit_focused_field();
    assert_eq!(
        state.selected_page_ref().unwrap().studio.nodes[0].title,
        "常用程序"
    );
    state.activate(Hit::ShortcutMove(1));
    assert_eq!(
        state.selected_page_ref().unwrap().studio.nodes[1].target,
        "C:/a.lnk"
    );
    state.activate(Hit::ShortcutDelete);
    assert_eq!(state.selected_page_ref().unwrap().studio.nodes.len(), 1);
    assert_eq!(
        state.selected_page_ref().unwrap().studio.nodes[0].target,
        "C:/b.lnk"
    );
}

#[test]
fn folder_picker_targets_the_selected_draft_page() {
    let mut config = DesktopConfig::default();
    let mut card = Card::new("文件夹", Module::Folder);
    card.pages[0].folder.path = "C:/before".into();
    config.cards.push(card);
    let original = config.clone();
    let mut state = State::new(&config);
    assert!(state.selected_page_ref().unwrap().selected("files"));
    assert!(state.selected_page_ref().unwrap().selected("folders"));

    let action = state.activate(Hit::FolderChoose);
    let page_id = state.selected_page_ref().unwrap().id.clone();
    assert_eq!(
        action,
        Some(Action::ChooseFolder {
            card_id: state.selected_card_ref().unwrap().id.clone(),
            page_id: page_id.clone(),
        })
    );
    assert_eq!(state.selected_page_ref().unwrap().folder.path, "C:/before");

    let card_id = state.selected_card_ref().unwrap().id.clone();
    state.set_folder_source(&card_id, &page_id, "C:/after".into());
    assert_eq!(state.selected_page_ref().unwrap().folder.path, "C:/after");
    assert_eq!(config, original);
}

#[test]
fn folder_filter_commits_when_switching_pages_and_keyboard_focus_loads_value() {
    let mut config = DesktopConfig::default();
    let mut card = Card::new("文件夹", Module::Folder);
    let mut second = card.pages[0].clone();
    second.id = "folder-page-two".into();
    second.title = "第二页".into();
    second.folder.filter = "会议".into();
    card.pages.push(second);
    config.cards.push(card);
    let mut state = State::new(&config);

    state.activate(Hit::EditPreference(Field::FolderFilter));
    state.preference_text.set_text(".md");
    state.activate(Hit::Page(1));
    assert_eq!(state.config.cards[0].pages[0].folder.filter, ".md");
    assert_eq!(state.selected_page, Some(1));

    let viewport = Rect::from_size(0.0, 0.0, 900.0, 700.0);
    state.focused = Some(Hit::FolderChoose);
    state.key(0x09, false, false);
    assert_eq!(state.focus_field, Some(Field::FolderFilter));
    assert_eq!(state.preference_text.text(), "会议");
    state.key(0x09, false, false);
    assert_eq!(state.focus_field, None);
    assert_eq!(state.focused, Some(Hit::FolderSort));
    state.key(0x09, true, false);
    assert_eq!(state.focus_field, Some(Field::FolderFilter));
    assert_eq!(state.preference_text.text(), "会议");
    let layout = state.layout(viewport);
    assert!(layout
        .controls
        .iter()
        .any(|(_, hit)| *hit == Hit::EditPreference(Field::FolderFilter)));
}

#[test]
fn folder_editor_controls_fit_without_overlapping_at_compact_sizes() {
    let mut config = DesktopConfig::default();
    config.cards.push(Card::new("文件夹", Module::Folder));
    let state = State::new(&config);
    let preferences = super::preferences::settings(&state);
    assert!(preferences.contains(&super::preferences::Setting::Grid));
    assert!(preferences.contains(&super::preferences::Setting::Modified));
    for (width, height) in [(820.0, 640.0), (560.0, 520.0)] {
        let layout = state.layout(Rect::from_size(0.0, 0.0, width, height));
        let control = |hit| {
            layout
                .controls
                .iter()
                .find(|(_, candidate)| *candidate == hit)
                .map(|(rect, _)| *rect)
                .unwrap()
        };
        let separate = |a: Rect, b: Rect| {
            a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top
        };
        let path = control(Hit::EditPreference(Field::FolderPath));
        let choose = control(Hit::FolderChoose);
        let filter = control(Hit::EditPreference(Field::FolderFilter));
        let sort = control(Hit::FolderSort);
        let hidden = control(Hit::FolderToggleHidden);
        assert!(separate(path, choose));
        assert!(separate(filter, sort));
        assert!(separate(sort, hidden));
        for rect in [path, choose, filter, sort, hidden] {
            assert!(layout
                .options_view
                .contains(rect.left + 1.0, rect.top + 1.0));
            assert!(layout
                .options_view
                .contains(rect.right - 1.0, rect.bottom - 1.0));
        }
    }
}

#[test]
fn embedded_desktop_manager_uses_workspace_bounds_and_keeps_actions_reachable() {
    for width in [520.0, 1000.0, 1700.0] {
        let state = State::workspace(&config_with_cards(2));
        let area = Rect::from_size(200.0, 40.0, width, 650.0);
        let layout = state.layout(area);
        assert_eq!(layout.frame, area);
        assert!(!layout
            .controls
            .iter()
            .any(|(_, hit)| matches!(hit, Hit::Close | Hit::Save)));
        for target in [
            Hit::Import,
            Hit::ExportAll,
            Hit::NewCard,
            Hit::SaveKeepOpen,
            Hit::Cancel,
        ] {
            let rect = layout
                .controls
                .iter()
                .find(|(_, hit)| *hit == target)
                .unwrap()
                .0;
            assert!(rect.left >= area.left && rect.right <= area.right);
            assert_eq!(layout.hit(rect.left + 5.0, rect.top + 5.0), Some(target));
        }
    }
}
