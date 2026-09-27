use super::*;

fn state(rows: usize, columns: usize) -> State {
    let mut document = create_base_document();
    let table = &mut document.tables[0];
    table.fields = (0..columns)
        .map(|n| create_base_field(FieldType::Text, &format!("列 {n}")))
        .collect();
    table.records = (0..rows)
        .map(|n| BaseRecord {
            id: format!("row_{n}"),
            ..Default::default()
        })
        .collect();
    let mut state = State::parse(serialize_base_document(&document).unwrap()).unwrap();
    state.editing = true;
    state
}

const AREA: Rect = Rect {
    left: 210.0,
    top: 100.0,
    right: 1110.0,
    bottom: 750.0,
};

struct ResetTableCustomization {
    preferences: crate::ui::editor_preferences::Preferences,
}

impl Drop for ResetTableCustomization {
    fn drop(&mut self) {
        crate::ui::editor_preferences::set(self.preferences.clone());
        crate::ui::settings_values::reset();
    }
}

fn load_table_customization() -> ResetTableCustomization {
    use mochi_core::{
        app_settings::{AppSettings, SettingValue},
        settings::SettingsService,
    };
    use std::sync::Arc;

    let previous = crate::ui::editor_preferences::current();
    crate::ui::settings_values::reset();
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let settings = AppSettings::new(Arc::new(SettingsService::new(Some(
        std::env::temp_dir().join(format!("mochi-table-ui-test-{suffix}.json")),
    ))));
    for (key, value) in [
        ("tables.fontSize", 24.0),
        ("tables.cellPadding", 20.0),
        ("tables.borderWidth", 3.0),
        ("tables.boardCardRadius", 16.0),
    ] {
        let descriptor = mochi_core::app_settings::descriptor(key)
            .unwrap_or_else(|| panic!("missing test setting {key}"));
        settings.write(descriptor, &SettingValue::Number(value));
    }
    crate::ui::settings_values::load(&settings);
    crate::ui::editor_preferences::set(crate::ui::editor_preferences::Preferences::read(&settings));
    ResetTableCustomization {
        preferences: previous,
    }
}

#[test]
fn frozen_cells_keep_their_coordinates_during_both_scroll_axes() {
    let mut s = state(100, 10);
    s.submit(Edit::FrozenColumns, "2").unwrap();
    s.submit(Edit::FrozenRows, "3").unwrap();
    let before = layout(&s, AREA);
    let first = before.rect_of(Hit::Cell(0, 0)).unwrap();
    let corner = before.rect_of(Hit::Cell(2, 1)).unwrap();
    s.scroll_x = 451.0;
    s.scroll_y = 329.0;
    let after = layout(&s, AREA);
    assert_eq!(after.rect_of(Hit::Cell(0, 0)), Some(first));
    assert_eq!(after.rect_of(Hit::Cell(2, 1)), Some(corner));
    let grid = after.grid.as_ref().unwrap();
    for row in grid.rows.iter().filter(|row| !row.frozen) {
        assert!(row.clip_start >= grid.frozen_y);
    }
    for column in grid.columns.iter().filter(|column| !column.frozen) {
        assert!(column.clip_start >= grid.frozen_x);
    }
    assert_eq!(
        after.hit(first.left + 12.0, first.top + 12.0),
        Some(Hit::Cell(0, 0))
    );
    assert!(after.rect_of(Hit::Cell(11, 4)).is_some());
}

#[test]
fn frozen_rows_use_sorted_and_filtered_view_order() {
    let mut s = state(4, 2);
    let field_id = s.table().fields[0].id.clone();
    for (i, record) in s.table_mut().records.iter_mut().enumerate() {
        record
            .values
            .insert(field_id.clone(), json!(format!("{i}")));
    }
    s.choose(Choice::Sort(0, SortDirection::Desc));
    s.submit(Edit::FrozenRows, "1").unwrap();
    s.scroll_y = 20.0;
    let layout = layout(&s, AREA);
    assert_eq!(layout.grid.as_ref().unwrap().rows[0].index, 3);
    s.query = "2".into();
    let layout = super::layout(&s, AREA);
    assert_eq!(layout.grid.as_ref().unwrap().rows[0].index, 2);
}

#[test]
fn large_frozen_counts_keep_scroll_space_and_restore_when_resized() {
    let mut s = state(100, 10);
    s.submit(Edit::FrozenRows, "100").unwrap();
    s.submit(Edit::FrozenColumns, "10").unwrap();
    let small = layout(&s, Rect::new(0.0, 0.0, 400.0, 300.0));
    let g = small.grid.as_ref().unwrap();
    assert!(g.frozen_x <= small.body.right - 100.0);
    assert!(g.frozen_y <= small.body.bottom - grid_row_height(&s));
    assert_eq!(s.view().frozen_columns, 10);
    let larger = layout(&s, AREA);
    assert!(
        larger
            .grid
            .as_ref()
            .unwrap()
            .columns
            .iter()
            .filter(|c| c.frozen)
            .count()
            > g.columns.iter().filter(|c| c.frozen).count()
    );
}

#[test]
fn row_geometry_remains_viewport_bounded() {
    let mut s = state(10_000, 3);
    s.view_mut().frozen_rows = 9_999;
    s.scroll_y = 300_000.0;
    let l = layout(&s, AREA);
    assert!(l.grid.as_ref().unwrap().rows.len() < 30);
    assert!(l.entries.len() < 150);
}

#[test]
fn table_customization_keeps_large_cell_text_inside_rows_and_shares_progress_hit_geometry() {
    let _reset = load_table_customization();
    let mut s = state(1, 1);
    let expected_minimum = TextStyle::Table.line_height() + 4.0;
    let row_height = grid_row_height(&s);
    assert!(row_height >= expected_minimum);
    assert_eq!(TextStyle::Table.font_size(), 24.0);
    assert_eq!(super::table_cell_padding(), 20.0);
    assert_eq!(super::table_border_width(), 3.0);
    assert_eq!(super::board_card_radius(), 16.0);

    let field = &mut s.table_mut().fields[0];
    field.field_type = FieldType::Progress;
    let field_id = field.id.clone();
    s.table_mut().records[0]
        .values
        .insert(field_id, json!(60.0));
    let laid_out = layout(&s, AREA);
    let cell = laid_out.rect_of(Hit::Cell(0, 0)).unwrap();
    let (_, input) = super::progress_regions(cell);
    assert_eq!(laid_out.rect_of(Hit::ProgressInput(0, 0)), Some(input));
}

#[test]
fn table_cell_paint_uses_the_configured_padding_and_table_font() {
    let _reset = load_table_customization();
    let s = state(1, 1);
    let field = &s.table().fields[0];
    let area = Rect::new(20.0, 30.0, 240.0, 68.0);
    let mut list = DrawList::new();
    cell(
        &mut list,
        area,
        field,
        &json!("cell text"),
        theme::tokens().palette(false),
    );
    let (text_rect, style) = list
        .cmds()
        .iter()
        .find_map(|command| match command {
            super::super::draw::DrawCmd::Text {
                rect, text, style, ..
            } if text == "cell text" => Some((*rect, *style)),
            _ => None,
        })
        .expect("table cell text command");
    assert_eq!(text_rect.left, area.left + 20.0);
    assert_eq!(text_rect.right, area.right - 20.0);
    assert_eq!(style, TextStyle::Table);
    assert!(text_rect.height() >= style.line_height());
}

#[test]
fn hidden_columns_preserve_data_and_are_saved_per_view() {
    let mut s = state(1, 3);
    s.submit(Edit::Cell(0, 1), "不可丢失").unwrap();
    let original = s.table().records.clone();
    s.activate(Hit::Columns);
    s.choose(Choice::ToggleColumn(1));
    assert_eq!(s.popup, Some(Popup::Columns));
    assert_eq!(s.visible_fields(), vec![0, 2]);
    assert_eq!(s.table().records, original);
    let l = layout(&s, AREA);
    assert!(l.rect_of(Hit::Cell(0, 1)).is_none());
    let reloaded = State::parse(serialize_base_document(&s.document).unwrap()).unwrap();
    assert_eq!(reloaded.visible_fields(), vec![0, 2]);
    s.submit(Edit::NewView(ViewType::Grid), "其他视图").unwrap();
    assert_eq!(s.visible_fields(), vec![0, 1, 2]);
    s.activate(Hit::View(0));
    s.choose(Choice::ShowAllColumns);
    assert_eq!(s.visible_fields(), vec![0, 1, 2]);
}

#[test]
fn visibility_checkboxes_cannot_hide_last_column() {
    let mut s = state(1, 2);
    s.editing = false;
    s.activate(Hit::Columns);
    assert_eq!(s.menu_items().len(), 3);
    s.choose(Choice::ToggleColumn(1));
    s.choose(Choice::ToggleColumn(0));
    assert_eq!(s.visible_fields(), vec![0]);
    assert!(s.error.contains("至少保留一列"));
    assert!(s.dirty);
}

#[test]
fn frozen_columns_count_visible_fields_not_hidden_ones() {
    let mut s = state(40, 10);
    s.choose(Choice::ToggleColumn(0));
    s.submit(Edit::FrozenColumns, "1").unwrap();
    s.scroll_x = 380.0;
    let l = layout(&s, AREA);
    let frozen = &l.grid.as_ref().unwrap().columns[0];
    assert_eq!(frozen.index, 1);
    assert!(frozen.frozen);
    assert_eq!(frozen.start, AREA.left + grid::GUTTER);
}

#[test]
fn board_cards_respect_hidden_fields_but_details_keep_them() {
    let mut s = state(2, 3);
    s.submit(Edit::NewField(FieldType::SingleSelect), "分组")
        .unwrap();
    s.submit(Edit::NewView(ViewType::Board), "看板").unwrap();
    s.choose(Choice::ToggleColumn(1));
    let l = layout(&s, AREA);
    assert!(l.rect_of(Hit::Cell(0, 1)).is_none());
    s.activate(Hit::Record(0));
    let l = layout(&s, AREA);
    assert!(l.rect_of(Hit::Cell(0, 1)).is_some());
}

#[test]
fn field_menu_anchors_to_its_header_and_scrolls_without_drifting() {
    let mut s = state(30, 8);
    s.scroll_x = 120.0;
    let l = layout(&s, AREA);
    let header = l.rect_of(Hit::Field(2)).unwrap();
    s.activate(Hit::Field(2));
    let opened = layout(&s, AREA);
    let menu = opened.menu.unwrap();
    assert_eq!(menu.top, header.bottom + 6.0);
    assert!((menu.left - header.left).abs() <= 1.0);
    assert!(opened.menu_max > 0.0);
    s.scroll(AREA, -2.0, false);
    assert_eq!(layout(&s, AREA).menu, Some(menu));
    assert_eq!(s.scroll_x, 120.0);
    assert_eq!(s.scroll_y, 0.0);
}

#[test]
fn nested_menus_retain_cell_anchor_instead_of_jumping_to_header() {
    let mut s = state(10, 3);
    s.submit(Edit::NewField(FieldType::SingleSelect), "选项")
        .unwrap();
    s.activate(Hit::Cell(3, 3));
    let anchor = s.popup_anchor;
    s.choose(Choice::Popup(Popup::Options(3)));
    assert_eq!(s.popup_anchor, anchor);
    assert_eq!(anchor, Some(Hit::Cell(3, 3)));
}

#[test]
fn popup_placement_stays_inside_offset_and_narrow_viewports() {
    for area in [AREA, Rect::new(740.0, 120.0, 910.0, 340.0)] {
        for anchor in [
            Rect::new(
                area.right - 30.0,
                area.bottom - 38.0,
                area.right,
                area.bottom - 8.0,
            ),
            Rect::new(
                area.left,
                area.top + 10.0,
                area.left + 20.0,
                area.top + 40.0,
            ),
        ] {
            let popup = popup_geometry::place(area, Some(anchor), 300.0, 520.0);
            assert!(popup.left >= area.left && popup.right <= area.right);
            assert!(popup.top >= area.top && popup.bottom <= area.bottom);
            assert!(popup.width() > 0.0 && popup.height() > 0.0);
        }
    }
}

#[test]
fn freeze_input_rejects_bad_counts_and_can_reset() {
    let mut s = state(20, 5);
    for value in ["-1", "2.5", "bad", "6"] {
        assert!(s.submit(Edit::FrozenColumns, value).is_err());
    }
    s.submit(Edit::FrozenColumns, "2").unwrap();
    s.submit(Edit::FrozenRows, "4").unwrap();
    s.choose(Choice::ClearFreeze);
    assert_eq!((s.view().frozen_rows, s.view().frozen_columns), (0, 0));
}

#[test]
fn export_is_available_from_view_menu_in_read_mode() {
    let mut s = state(0, 1);
    s.editing = false;
    s.activate(Hit::ViewsMenu);
    assert!(s
        .menu_items()
        .iter()
        .any(|item| matches!(item.choice, Choice::ExportXlsx)));
    s.choose(Choice::ExportXlsx);
    assert_eq!(s.action, Some(Action::ExportXlsx));
}

#[test]
fn search_reveals_hidden_columns_and_avoids_frozen_panes_without_changing_view() {
    let mut s = state(100, 10);
    s.choose(Choice::ToggleColumn(4));
    s.submit(Edit::FrozenColumns, "2").unwrap();
    s.submit(Edit::FrozenRows, "3").unwrap();
    s.choose(Choice::SetRowHeight(64.0));
    let view = s.view().clone();
    s.focus_cell(0, 60, 4, AREA);
    assert!(s.detail.is_none());
    let l = layout(&s, AREA);
    let target = l.rect_of(Hit::Cell(60, 4)).unwrap();
    let g = l.grid.as_ref().unwrap();
    assert!(target.left >= g.frozen_x);
    assert!(target.top >= g.frozen_y);
    assert_eq!(s.view(), &view);
    s.activate(Hit::ClearLocation);
    assert!(!s.visible_fields().contains(&4));
}
