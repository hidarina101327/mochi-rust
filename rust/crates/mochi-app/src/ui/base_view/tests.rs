use super::*;
fn state() -> State {
    let mut state =
        State::parse(serialize_base_document(&create_base_document()).unwrap()).unwrap();
    state.editing = true;
    state
}
#[test]
fn default_mode_is_edit_and_view_mode_can_still_be_selected() {
    let mut state =
        State::parse(serialize_base_document(&create_base_document()).unwrap()).unwrap();
    assert!(state.editing);
    state.activate(Hit::ToggleEditing);
    assert!(!state.editing);
}
#[test]
fn right_click_insert_adds_rows_and_columns_next_to_the_cell() {
    let mut state = state();
    state.activate(Hit::NewRecord);
    state.activate(Hit::NewRecord);
    state.submit(Edit::Cell(1, 0), "原记录").unwrap();

    assert!(state.insert_next_to_cell(1, 0, InsertDirection::Above));
    assert_eq!(state.table().records.len(), 3);
    assert_eq!(state.selected, Some((1, 0)));
    assert!(state.table().records[1].values.is_empty());
    assert_eq!(
        format_cell_value(
            &state.table().fields[0],
            &state.table().records[2].values[&state.table().fields[0].id]
        ),
        "原记录"
    );

    assert!(state.insert_next_to_cell(1, 0, InsertDirection::Right));
    assert_eq!(state.table().fields.len(), 2);
    assert_eq!(state.table().fields[1].name, "新字段");
    assert_eq!(state.selected, Some((1, 1)));
    assert!(state.insert_next_to_cell(1, 1, InsertDirection::Left));
    assert_eq!(state.table().fields[1].name, "新字段 2");
    assert_eq!(state.table().fields[2].name, "新字段");

    state.editing = false;
    let before = state.document.clone();
    assert!(!state.insert_next_to_cell(1, 1, InsertDirection::Below));
    assert_eq!(state.document, before);
}
#[test]
fn cell_ai_context_is_a_labelled_record_snapshot() {
    let mut state = state();
    state.activate(Hit::NewRecord);
    state.submit(Edit::Cell(0, 0), "KMP 算法").unwrap();
    let context = state.cell_ai_context(0, 0).unwrap();
    assert!(context.title.contains("名称"));
    assert!(context.title.contains("KMP 算法"));
    assert!(context.content.contains("KMP 算法"));
    assert!(context.content.contains("该记录"));
}
#[test]
fn time_point_picker_defaults_to_now_and_confirms_the_selected_components() {
    let mut state = state();
    state
        .submit(Edit::NewField(FieldType::DateTime), "时间点")
        .unwrap();
    state.activate(Hit::NewRecord);
    state.activate(Hit::Cell(0, 1));
    let initial = state.date_time_picker.unwrap();
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let layout = layout(&state, area);
    assert!(layout
        .entries
        .iter()
        .any(|(_, hit)| *hit == Hit::DateTimeAdjust(DateTimePart::Year, -1)));
    assert!(layout
        .entries
        .iter()
        .any(|(_, hit)| *hit == Hit::DateTimeConfirm));
    state.activate(Hit::DateTimeConfirm);
    let saved = state.table().records[0].values[&state.table().fields[1].id]
        .as_str()
        .unwrap();
    assert!(saved.starts_with(&initial.display().replace(' ', "T")));
    assert!(state.date_time_picker.is_none());

    state.activate(Hit::Cell(0, 1));
    state.activate(Hit::DateTimeAdjust(DateTimePart::Year, 1));
    let selected = state.date_time_picker.unwrap();
    assert_eq!(selected.year, initial.year + 1);
    state.activate(Hit::DateTimeConfirm);
    let saved = state.table().records[0].values[&state.table().fields[1].id]
        .as_str()
        .unwrap();
    assert!(saved.starts_with(&selected.display().replace(' ', "T")));
}
#[test]
fn viewing_mode_keeps_cells_read_only() {
    let mut seed = state();
    seed.activate(Hit::NewRecord);
    seed.submit(Edit::Cell(0, 0), "学习主题").unwrap();
    seed.submit(Edit::NewField(FieldType::Checkbox), "完成")
        .unwrap();
    seed.submit(Edit::NewField(FieldType::Reference), "笔记")
        .unwrap();
    seed.submit(Edit::ReferenceLink(0, 2), "mochi://open?path=note.md")
        .unwrap();
    let mut state = State::parse(serialize_base_document(&seed.document).unwrap()).unwrap();
    state.editing = false;
    let before = state.document.clone();
    assert!(!state.editing);
    assert!(state.activate(Hit::Cell(0, 0)).is_none());
    assert!(state.activate(Hit::Cell(0, 1)).is_none());
    assert!(state.activate(Hit::NewRecord).is_none());
    assert_eq!(state.document, before);
    assert!(!state.dirty);
    state.activate(Hit::Record(0));
    assert_eq!(state.detail, Some(0));
    state.activate(Hit::OpenReference(0, 2, 0));
    assert_eq!(
        state.action.take(),
        Some(Action::Open("mochi://open?path=note.md".into()))
    );
    state.activate(Hit::ToggleEditing);
    state.activate(Hit::Cell(0, 1));
    assert_eq!(
        state.table().records[0].values[&state.table().fields[1].id],
        json!(true)
    );
}
#[test]
fn resize_drag_persists_width_and_scrollbar_moves_only_horizontal_offset() {
    let mut state = state();
    for index in 0..5 {
        state
            .submit(Edit::NewField(FieldType::Text), &format!("字段{index}"))
            .unwrap();
    }
    let area = Rect::new(0.0, 0.0, 650.0, 500.0);
    let l = layout(&state, area);
    let resize = l
        .entries
        .iter()
        .find(|(_, hit)| *hit == Hit::Resize(0))
        .unwrap()
        .0;
    assert!(state.pointer_down(area, resize.left + 4.0, resize.top + 10.0));
    state.drag_to(area, resize.left + 104.0);
    assert!(state.end_drag());
    assert_eq!(state.table().fields[0].width, Some(280.0));
    let restored = State::parse(serialize_base_document(&state.document).unwrap()).unwrap();
    assert_eq!(restored.table().fields[0].width, Some(280.0));
    let l = layout(&state, area);
    let thumb = l.scroll_thumb.unwrap();
    let track = l.scroll_track.unwrap();
    assert!(state.pointer_down(area, thumb.left + 4.0, thumb.top + 4.0));
    state.drag_to(area, track.right);
    assert!(state.scroll_x > 0.0);
    assert_eq!(state.scroll_y, 0.0);
    assert!(state.scroll_x <= layout(&state, area).max_x);
    state.end_drag();
}
#[test]
fn overlay_scrollbars_keep_cells_fixed_and_scroll_both_axes_without_dirtying_data() {
    let mut state = state();
    for index in 0..5 {
        state
            .submit(Edit::NewField(FieldType::Text), &format!("字段{index}"))
            .unwrap();
    }
    for _ in 0..80 {
        state.activate(Hit::NewRecord);
    }
    state.dirty = false;
    let saved = state.document.clone();
    let area = Rect::new(0.0, 0.0, 650.0, 500.0);
    let l = layout(&state, area);
    assert_eq!(l.scrollbars.len(), 2);
    for (axis, bar) in &l.scrollbars {
        assert!(area.contains(bar.thumb.left, bar.thumb.top));
        assert!(area.contains(bar.thumb.right, bar.thumb.bottom));
        state.pointer(area, bar.thumb.right - 1.0, bar.thumb.top + 2.0);
        let hovered = layout(&state, area);
        assert_eq!(hovered.body, l.body);
        assert_eq!(hovered.entries, l.entries);
        assert_eq!(state.scrollbars.hover, Some(*axis));
    }
    state.pointer(area, 100.0, 200.0);
    assert_eq!(state.scrollbars.hover, None);
    for axis in [Axis::Vertical, Axis::Horizontal] {
        let l = layout(&state, area);
        let b = l.scrollbars.iter().find(|(a, _)| *a == axis).unwrap().1;
        let before = (state.scroll_x, state.scroll_y);
        assert!(state.pointer_down(area, b.thumb.right - 1.0, b.thumb.top + 2.0));
        state.pointer(area, area.right + 100.0, area.bottom + 100.0);
        match axis {
            Axis::Vertical => {
                assert_eq!(state.scroll_y, l.max_y);
                assert_eq!(state.scroll_x, before.0);
            }
            Axis::Horizontal => {
                assert_eq!(state.scroll_x, l.max_x);
                assert_eq!(state.scroll_y, before.1);
            }
        }
        assert!(state.end_drag());
        state.pointer_leave();
        assert_eq!(state.scrollbars.hover, None);
    }
    assert_eq!(state.document, saved);
    assert!(!state.dirty);
    state.popup = Some(Popup::Table);
    assert!(layout(&state, area).scrollbars.is_empty());
    state.popup = None;
    state.detail = Some(0);
    assert!(layout(&state, area).scrollbars.is_empty());
}
#[test]
fn stable_record_reveal_bypasses_filters_without_mutating_saved_view() {
    let mut state = state();
    state.activate(Hit::NewRecord);
    state.activate(Hit::NewRecord);
    state.submit(Edit::Cell(1, 0), "定位记录").unwrap();
    state
        .submit(Edit::Filter(0, FilterOperator::Contains), "不匹配")
        .unwrap();
    assert!(state.visible_records().is_empty());
    let filter = state.view().filters.clone();
    let location = BaseLocation {
        table_id: state.table().id.clone(),
        record_id: Some(state.table().records[1].id.clone()),
        field_id: Some(state.table().fields[0].id.clone()),
    };
    state.editing = false;
    state.dirty = false;
    state.reveal(location).unwrap();
    assert_eq!(state.visible_records(), vec![0, 1]);
    assert_eq!(state.detail, Some(1));
    assert_eq!(state.selected, Some((1, 0)));
    assert_eq!(state.view().filters, filter);
    assert!(!state.dirty);
    state.activate(Hit::ClearLocation);
    assert!(state.visible_records().is_empty());
    assert!(state
        .reveal(BaseLocation {
            table_id: "missing".into(),
            ..Default::default()
        })
        .is_err());
}
#[test]
fn typed_edits_validate_before_changing_document() {
    let mut s = state();
    s.submit(Edit::NewField(FieldType::Progress), "百分比")
        .unwrap();
    s.activate(Hit::NewRecord);
    let f = s.table().fields.len() - 1;
    assert!(s.submit(Edit::Cell(0, f), "120").is_err());
    assert!(s.table().records[0].values.is_empty());
    s.submit(Edit::Cell(0, f), "42").unwrap();
    assert_eq!(
        s.table().records[0].values[&s.table().fields[f].id],
        json!(42.0)
    );
    for input in ["NaN", "inf", "1e999"] {
        assert!(s
            .submit(Edit::Filter(f, FilterOperator::Equals), input)
            .is_err());
    }
    assert!(s.view().filters.is_empty());
}
#[test]
fn removing_select_option_clears_records_and_round_trips() {
    let mut s = state();
    s.submit(Edit::NewField(FieldType::MultiSelect), "标签")
        .unwrap();
    let f = s.table().fields.len() - 1;
    s.submit(Edit::NewOption(f), "自定义标签").unwrap();
    s.activate(Hit::NewRecord);
    s.choose(Choice::ToggleOption(0, f, 0));
    assert!(!s.table().records[0].values.is_empty());
    let field = &s.table().fields[f];
    let field_id = field.id.clone();
    let option_id = field.options[0].id.clone();
    s.view_mut().filters.push(BaseFilter {
        field_id,
        operator: FilterOperator::Equals,
        value: Some(json!([option_id])),
        ..Default::default()
    });
    s.submit(Edit::DeleteOption(f, 0), "").unwrap();
    assert!(s.view().filters.is_empty());
    let raw = serialize_base_document(&s.document).unwrap();
    let restored = State::parse(raw).unwrap();
    assert_eq!(
        restored.table().records[0].values[&restored.table().fields[f].id],
        json!([])
    );
}
#[test]
fn view_filters_and_board_groups_use_option_ids() {
    let mut s = state();
    s.submit(Edit::NewField(FieldType::SingleSelect), "分类")
        .unwrap();
    let f = s.table().fields.len() - 1;
    s.submit(Edit::NewOption(f), "标签").unwrap();
    s.activate(Hit::NewRecord);
    s.activate(Hit::NewRecord);
    s.choose(Choice::ToggleOption(0, f, 0));
    s.submit(Edit::NewView(ViewType::Board), "看板").unwrap();
    assert_eq!(
        s.groups()
            .iter()
            .map(|(_, rows)| rows.len())
            .collect::<Vec<_>>(),
        vec![1, 1]
    );
    s.submit(Edit::Filter(f, FilterOperator::Equals), "标签")
        .unwrap();
    assert_eq!(s.visible_records(), vec![0]);
    s.submit(Edit::DeleteField(f), "").unwrap();
    assert!(s.view().filters.is_empty());
    assert!(s.view().group_by.is_none());
}
#[test]
fn existing_field_type_menu_converts_values_and_previews_destructive_changes() {
    let mut s = state();
    s.activate(Hit::NewRecord);
    s.submit(Edit::Cell(0, 0), "自定标签").unwrap();
    let id = s.table().fields[0].id.clone();
    s.activate(Hit::Field(0));
    let dropdown = s
        .menu_items()
        .iter()
        .position(|item| item.label == "字段类型 · 下拉菜单")
        .unwrap();
    assert!(s.activate(Hit::Menu(dropdown)).is_none());
    assert_eq!(s.table().fields[0].field_type, FieldType::SingleSelect);
    assert_eq!(
        format_cell_value(&s.table().fields[0], &s.table().records[0].values[&id]),
        "自定标签"
    );
    let before = s.document.clone();
    let dirty = s.dirty;
    let prompt = s.choose(Choice::SetType(0, FieldType::DateTime)).unwrap();
    assert!(prompt.description.contains("清空 1 个"));
    assert_eq!(s.document, before);
    assert_eq!(s.dirty, dirty);
    // 取消提示无需回滚：只有确认提交才改动数据。
    s.submit(prompt.edit, "").unwrap();
    assert_eq!(s.table().fields[0].field_type, FieldType::DateTime);
    assert!(s.table().records[0].values[&id].is_null());
    s.submit(Edit::Cell(0, 0), "2026-09-12T09:15").unwrap();
    assert!(s.choose(Choice::SetType(0, FieldType::DateRange)).is_none());
    assert_eq!(
        s.table().records[0].values[&id],
        json!(["2026-09-12T09:15", "2026-09-12T09:15"])
    );
}
#[test]
fn time_cell_dialogs_validate_and_roundtrip_canonical_values() {
    let mut s = state();
    s.submit(Edit::NewField(FieldType::DateTime), "时间点")
        .unwrap();
    s.submit(Edit::NewField(FieldType::DateRange), "时间段")
        .unwrap();
    s.activate(Hit::NewRecord);
    s.submit(Edit::Cell(0, 1), "2026-09-12 09:15").unwrap();
    s.submit(Edit::Cell(0, 2), "2026-09-12 09:15 至 2026-09-13 17:00")
        .unwrap();
    let before = s.document.clone();
    assert!(s
        .submit(Edit::Cell(0, 2), "2026-09-13 17:00 至 2026-09-12 09:15")
        .is_err());
    assert_eq!(s.document, before);
    assert!(s.submit(Edit::Cell(0, 1), "2026-09-12T25:00").is_err());
    assert_eq!(s.document, before);
    let prompt = s.activate(Hit::Cell(0, 2)).unwrap();
    assert_eq!(
        prompt.value.as_deref(),
        Some("2026-09-12 09:15 至 2026-09-13 17:00")
    );
    s.submit(
        Edit::Filter(2, FilterOperator::Equals),
        prompt.value.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(s.visible_records(), vec![0]);
    let raw = serialize_base_document(&s.document).unwrap();
    assert_eq!(State::parse(raw).unwrap().document, s.document);
}
#[test]
fn grid_and_board_draw_colored_pills_with_matching_edit_hits() {
    use super::super::draw::DrawCmd;
    let mut s = state();
    s.submit(Edit::NewField(FieldType::SingleSelect), "标签")
        .unwrap();
    let f = s.table().fields.len() - 1;
    s.submit(Edit::NewOption(f), "自定内容").unwrap();
    s.activate(Hit::NewRecord);
    s.choose(Choice::ToggleOption(0, f, 0));
    s.submit(Edit::NewView(ViewType::Board), "卡片").unwrap();
    let area = Rect::new(0.0, 0.0, 1000.0, 700.0);
    for dark in [false, true] {
        let palette = super::super::theme::tokens().palette(dark);
        for v in 0..2 {
            s.activate(Hit::View(v));
            let l = layout(&s, area);
            let mut list = DrawList::new();
            paint(&mut list, area, &s, &l, palette);
            assert!(list
                .cmds()
                .iter()
                .any(|cmd| matches!(cmd,DrawCmd::Text{text,..} if text=="自定内容")));
            let color = theme::mix(option_color(OptionColor::Blue), palette.background, 0.18);
            assert!(list.cmds().iter().any(|cmd|matches!(cmd,DrawCmd::RoundedRect{color:c,radius,..}if *c==color&&*radius==6.0)));
            let (r, _) = l
                .entries
                .iter()
                .find(|(_, hit)| *hit == Hit::Cell(0, f))
                .unwrap();
            assert_eq!(
                l.hit((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0),
                Some(Hit::Cell(0, f))
            );
            assert!(!list.cmds().iter().any(|cmd|matches!(cmd,DrawCmd::Text{text,..}if text.contains("mochi-base")||text.contains(&s.table().fields[f].id))));
            list.finish().unwrap();
        }
    }
}
#[test]
fn large_grid_only_draws_visible_records() {
    let mut s = state();
    for _ in 0..10000 {
        s.table_mut().records.push(BaseRecord {
            id: create_id("rec"),
            ..Default::default()
        });
    }
    let area = Rect::new(0.0, 0.0, 1000.0, 700.0);
    let l = layout(&s, area);
    let mut list = DrawList::new();
    paint(
        &mut list,
        area,
        &s,
        &l,
        super::super::theme::tokens().palette(false),
    );
    assert!(
        l.entries
            .iter()
            .filter(|(_, hit)| matches!(hit, Hit::Record(_)))
            .count()
            < 20
    );
    assert!(list.cmds().len() < 400);
    list.finish().unwrap();
}
#[test]
fn grid_row_height_is_per_view_persisted_and_drives_layout() {
    let mut s = state();
    for _ in 0..20 {
        s.table_mut().records.push(BaseRecord {
            id: create_id("rec"),
            ..Default::default()
        });
    }
    let area = Rect::new(0.0, 0.0, 900.0, 400.0);
    let default_max = layout(&s, area).max_y;
    s.choose(Choice::SetRowHeight(64.0));
    assert_eq!(s.view().row_height, Some(64.0));
    assert!(layout(&s, area).max_y > default_max);
    let raw = serialize_base_document(&s.document).unwrap();
    let restored = State::parse(raw).unwrap();
    assert_eq!(grid_row_height(&restored), 64.0);
}
#[test]
fn view_tools_sit_on_the_view_row_without_search_save_or_status_footer() {
    let mut state =
        State::parse(serialize_base_document(&create_base_document()).unwrap()).unwrap();
    state.editing = false;
    let area = Rect::new(0.0, 0.0, 900.0, 600.0);
    let l = layout(&state, area);
    assert!(l.entries.iter().all(|(_, hit)| !matches!(
        hit,
        Hit::Search | Hit::Save | Hit::NewRecord | Hit::NewField
    )));
    assert!((l.body.top - 85.0).abs() < 0.1);
    assert!((l.body.bottom - 600.0).abs() < 0.1);
    let filter = l.rect_of(Hit::Filter).unwrap();
    let sort = l.rect_of(Hit::Sort).unwrap();
    let edit = l.rect_of(Hit::ToggleEditing).unwrap();
    let automation = l.rect_of(Hit::Automations).unwrap();
    assert!((automation.top - 51.0).abs() < 0.1);
    assert!(automation.intersect(&edit).is_empty());
    assert!(automation.intersect(&sort).is_empty());
    assert!((filter.top - 51.0).abs() < 0.1);
    assert!(edit.right > sort.right);
    assert!(sort.right > filter.right);
    assert!((edit.right - (area.right - 10.0)).abs() < 0.1);
    let mut list = DrawList::new();
    paint(
        &mut list,
        area,
        &state,
        &l,
        super::super::theme::tokens().palette(false),
    );
    let texts: Vec<String> = list
        .cmds()
        .iter()
        .filter_map(|cmd| match cmd {
            super::super::draw::DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|text| text == "编辑"));
    assert!(texts.iter().any(|text| text == "筛选"));
    assert!(texts.iter().any(|text| text == "排序"));
    assert!(texts.iter().all(|text| {
        !text.contains("条记录") && !text.contains("已保存") && text != "搜索" && text != "编辑表格"
    }));
    state.activate(Hit::ToggleEditing);
    let editing = layout(&state, area);
    assert!((editing.body.top - 132.0).abs() < 0.1);
    assert!(editing
        .entries
        .iter()
        .any(|(_, hit)| *hit == Hit::NewRecord));
    state.activate(Hit::Filter);
    let open = layout(&state, area);
    let menu = open.menu.unwrap();
    let filter = open.rect_of(Hit::Filter).unwrap();
    assert!(menu.top >= filter.bottom);
    assert!((menu.right - filter.right).abs() < 1.0);
}
#[test]
fn search_cells_match_display_text_and_focus_without_opening_details() {
    let mut state = state();
    state.activate(Hit::NewRecord);
    state.submit(Edit::Cell(0, 0), "KMP 算法").unwrap();
    state.activate(Hit::NewRecord);
    state.submit(Edit::Cell(1, 0), "两数之和").unwrap();
    let hits = state.search_cells("kmp");
    assert_eq!(hits, vec![(0, 0, 0)]);
    state.focus_cell(0, 1, 0, Rect::new(0.0, 0.0, 900.0, 600.0));
    assert_eq!(state.selected, Some((1, 0)));
    assert!(state.detail.is_none());
    assert!(state.search_cells("不存在").is_empty());
}
