use super::*;
use crate::ui::{draw::DrawList, theme};

#[test]
fn legacy_document_node_can_toggle_unread_without_recreating_the_node() {
    let mut s = state();
    s.add_node("file_write");
    let index = s.selected_node.unwrap();
    s.draft.as_mut().unwrap().nodes[index]
        .config
        .as_object_mut()
        .unwrap()
        .remove("mark_unread");
    s.select_node(index);
    let before = s.draft.as_ref().unwrap().nodes[index].clone();
    s.edit_parameter("/config/mark_unread").unwrap();
    assert_eq!(s.field.text(), "false");
    assert_eq!(s.draft.as_ref().unwrap().nodes[index], before);
    s.field.set_text("true");
    s.apply_node().unwrap();
    assert_eq!(
        s.draft.as_ref().unwrap().nodes[index].config["mark_unread"],
        true
    );
    s.undo(false);
    assert_eq!(s.draft.as_ref().unwrap().nodes[index], before);
}

#[test]
fn variable_wires_bind_individual_inputs_and_undo_atomically() {
    let mut s = state();
    s.draft.as_mut().unwrap().nodes[1].inputs = serde_json::json!({"a": 0, "b": {}});
    let before = s.draft.clone().unwrap();
    s.connect_variable(0, 1, "a", "output").unwrap();
    s.connect_variable(0, 1, "b", "output").unwrap();
    let g = s.draft.as_ref().unwrap();
    assert_eq!(g.edges.len(), 1);
    assert_eq!(g.nodes[1].inputs["a"], "$nodes.start.output");
    assert_eq!(g.nodes[1].inputs["b"], "$nodes.start.output");
    assert_eq!(s.bindings().len(), 2);
    s.remove_binding(1, "a");
    assert_eq!(s.bindings().len(), 1);
    s.undo(false);
    assert_eq!(s.bindings().len(), 2);
    s.undo(false);
    s.undo(false);
    assert_eq!(s.draft.unwrap(), before);
}

#[test]
fn variable_wire_drag_uses_rendered_ports_and_rejects_cycles() {
    let mut s = state();
    s.canvas = Rect::from_size(0., 0., 1200., 800.);
    s.hits.push((
        Rect::from_size(200., 100., 24., 24.),
        Hit::InputVariable(1, "result".into()),
    ));
    s.begin(Some(&Hit::OutputVariable(0, 0)), 10., 10.);
    s.release(212., 112.);
    assert_eq!(
        s.draft.as_ref().unwrap().nodes[1].inputs["result"],
        "$nodes.start.output"
    );
    s.add_node("json");
    s.add_node("json");
    s.connect_variable(2, 3, "result", "output").unwrap();
    let before = s.draft.clone().unwrap();
    assert!(s
        .connect_variable(3, 2, "result", "output")
        .unwrap_err()
        .contains("循环"));
    assert_eq!(s.draft.unwrap(), before);
}

#[test]
fn variable_picker_supports_json_and_custom_values_with_real_labels() {
    let mut s = state();
    s.select_node(1);
    s.edit_parameter("/inputs/result").unwrap();
    s.field.set_text("{\"a\":true}");
    s.apply_node().unwrap();
    assert_eq!(
        s.draft.as_ref().unwrap().nodes[1].inputs["result"]["a"],
        true
    );
    s.field.set_text("$nodes.start.output");
    s.apply_node().unwrap();
    assert!(s
        .variable_options()
        .iter()
        .any(|(label, value)| label == "开始 — output" && value == "$nodes.start.output"));
    let mut list = DrawList::new();
    paint(
        &mut list,
        Rect::from_size(0., 0., 1440., 900.),
        &mut s,
        false,
        theme::tokens().palette(false),
    );
    assert!(s
        .hits
        .iter()
        .any(|(_, hit)| *hit == Hit::VariablePicker("/inputs/result".into())));
    assert!(s
        .hits
        .iter()
        .any(|(_, hit)| matches!(hit, Hit::InputVariable(1, _))));
    assert!(s
        .hits
        .iter()
        .any(|(_, hit)| matches!(hit, Hit::OutputVariable(0, _))));
}

#[test]
fn custom_variable_values_preserve_literal_strings_and_json_types() {
    let mut s = state();
    s.add_node("notify");
    let i = s.selected_node.unwrap();
    s.edit_parameter("/inputs/title").unwrap();
    s.field.set_text("2026");
    s.apply_node().unwrap();
    assert_eq!(s.draft.as_ref().unwrap().nodes[i].inputs["title"], "2026");
    s.add_node("chart");
    s.edit_parameter("/inputs/values").unwrap();
    s.field.set_text("invalid JSON");
    assert!(s.apply_node().is_err());
    s.field.set_text("$input.values");
    s.apply_node().unwrap();
    s.field.set_text("[3, 8]");
    s.apply_node().unwrap();
    let i = s.selected_node.unwrap();
    assert_eq!(
        s.draft.as_ref().unwrap().nodes[i].inputs["values"],
        serde_json::json!([3, 8])
    );
}
fn state() -> State {
    let mut s = State::default();
    let d = Workflow::blank();
    s.open(SavedWorkflow {
        definition: d,
        revision: 1,
        approved: false,
        enabled: false,
        updated_at: 0,
    });
    s
}

#[test]
fn branch_ports_use_scaled_icons_and_keep_distinct_connection_targets() {
    use crate::ui::{draw::DrawCmd, icons::Icon};
    for zoom in [0.28, 0.54, 1., 1.6] {
        let mut s = state();
        s.add_node("condition");
        s.editor = None;
        s.palette = false;
        s.selected_node = None;
        let graph = s.draft.as_mut().unwrap();
        graph.nodes[2].position = mochi_core::workflows::Position { x: 80., y: 60. };
        s.zoom = zoom;
        s.pan = (0., 0.);
        s.needs_fit = false;
        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::from_size(0., 0., 1600., 900.),
            &mut s,
            false,
            theme::tokens().palette(false),
        );
        let mut targets = Vec::new();
        for (branch, icon) in [(true, Icon::CHECK), (false, Icon::X)] {
            let rect = s
                .hits
                .iter()
                .find(|(_, h)| *h == Hit::Port(2, Some(branch)))
                .unwrap()
                .0;
            assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Icon { rect: r, icon: i, .. } if *i == icon && r.left >= rect.left && r.right <= rect.right && r.top >= rect.top && r.bottom <= rect.bottom)));
            targets.push(rect);
        }
        assert!(targets[0].bottom < targets[1].top);
        assert!(!list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { text, .. } | DrawCmd::ScaledText { text, .. } if text == "是" || text == "否")));
    }
}

#[test]
fn history_title_stays_above_list_and_rows_are_clipped_when_scrolled() {
    use crate::ui::draw::DrawCmd;
    let mut s = state();
    s.history_open = true;
    s.panel_scroll = 103.;
    s.scroll = 47.;
    s.history = (0..50)
        .map(|i| mochi_core::workflows::RunSummary {
            id: format!("run{i}"),
            workflow_id: "flow".into(),
            status: "succeeded".into(),
            started_at: 1789914268000 - i * 60000,
            finished_at: Some(1789914269000 - i * 60000),
            source: "manual".into(),
        })
        .collect();
    let mut list = DrawList::new();
    paint(
        &mut list,
        Rect::from_size(0., 0., 1440., 900.),
        &mut s,
        false,
        theme::tokens().palette(false),
    );
    let title = list
        .cmds()
        .iter()
        .find_map(|c| match c {
            DrawCmd::Text { rect, text, .. } if text == "运行历史" => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert!(title.height() < 32.);
    assert!(title.bottom < s.panel_body.top);
    assert_eq!(s.scroll, 47.);
    let rows: Vec<_> = s
        .hits
        .iter()
        .filter(|(_, h)| matches!(h, Hit::RunHistory(_)))
        .collect();
    assert!(rows.len() > 3 && rows.len() < 50);
    assert!(rows
        .iter()
        .all(|(r, _)| r.top >= s.panel_body.top && r.bottom <= s.panel_body.bottom));
    assert!(rows.windows(2).all(|w| w[0].0.bottom < w[1].0.top));
}

#[test]
fn generated_documents_use_successful_outputs_and_prefer_written_content() {
    use mochi_core::workflows::NodeRun;
    use serde_json::json;
    let mut definition = Workflow::blank();
    definition.nodes = vec![
        Node::new("format", "script", 0., 0.),
        Node::new("write", "file_write", 0., 0.),
        Node::new("failed", "file_write", 0., 0.),
    ];
    let mut run = Run {
        id: "r".into(),
        workflow_id: definition.id.clone(),
        source: "manual".into(),
        status: "succeeded".into(),
        started_at: 0,
        finished_at: Some(10),
        input: json!({}),
        output: json!({"report":"reports/today.md"}),
        error: None,
        definition,
        nodes: Default::default(),
    };
    run.nodes.insert(
        "format".into(),
        NodeRun {
            status: "succeeded".into(),
            output: json!({"data":{"path":"reports/today.md"}}),
            ..Default::default()
        },
    );
    run.nodes.insert(
        "write".into(),
        NodeRun {
            status: "succeeded".into(),
            input: json!({"content":"# 今日简报\n已生成报告"}),
            output: json!({"path":"C:/workspace/reports/today.md"}),
            ..Default::default()
        },
    );
    run.nodes.insert(
        "failed".into(),
        NodeRun {
            status: "failed".into(),
            input: json!({"path":"missing.md"}),
            output: json!({"path":"missing.md"}),
            ..Default::default()
        },
    );
    let files = super::results::artifacts(&run, None);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "C:/workspace/reports/today.md");
    assert!(files[0].preview.contains("已生成报告"));
    assert!(super::results::artifacts(&run, Some(2)).is_empty());
}
#[test]
fn painting_hit_targets_match_canvas_and_inspector() {
    let mut s = state();
    let mut list = DrawList::new();
    paint(
        &mut list,
        Rect::from_size(0., 0., 1200., 800.),
        &mut s,
        false,
        theme::tokens().palette(false),
    );
    let r = s.node_rect(&s.draft.as_ref().unwrap().nodes[0]);
    assert_eq!(s.hit(r.left + 30., r.top + 30.), Some(Hit::Node(0)));
    assert!(
        !s.canvas.intersect(&s.inspector).width().is_sign_positive()
            || s.canvas.intersect(&s.inspector).is_empty()
    );
    assert!(s.hits.iter().any(|(_, h)| *h == Hit::Run));
}
#[test]
fn dragging_node_updates_only_position_and_can_undo() {
    let mut s = state();
    s.canvas = Rect::from_size(0., 0., 900., 600.);
    let before = s.draft.clone().unwrap();
    s.begin(Some(&Hit::Node(0)), 70., 110.);
    s.motion(100., 160.);
    s.release(100., 160.);
    assert_ne!(
        s.draft.as_ref().unwrap().nodes[0].position,
        before.nodes[0].position
    );
    s.undo(false);
    assert_eq!(s.draft.as_ref().unwrap(), &before);
}
#[test]
fn connect_ports_and_delete_edge() {
    let mut s = state();
    s.draft.as_mut().unwrap().edges.clear();
    s.hits
        .push((Rect::from_size(200., 0., 30., 30.), Hit::Input(1)));
    s.begin(Some(&Hit::Port(0, None)), 0., 0.);
    s.release(205., 5.);
    assert_eq!(s.draft.as_ref().unwrap().edges.len(), 1);
    s.selected_edge = Some(0);
    s.remove_selected();
    assert!(s.draft.as_ref().unwrap().edges.is_empty());
}
#[test]
fn invalid_json_preserves_node_and_unicode_inputs_roundtrip() {
    let mut s = state();
    s.select_node(0);
    s.field.set_text("{");
    assert!(s.apply_node().is_err());
    assert_eq!(s.draft.as_ref().unwrap().nodes[0].label, "开始");
    s.field
        .set_text(r#"{"label":"每日整理","inputs":{"中文":"保留输入"}}"#);
    s.apply_node().unwrap();
    assert_eq!(s.draft.as_ref().unwrap().nodes[0].label, "每日整理");
}
#[test]
fn zoom_keeps_pointer_world_position() {
    let mut s = state();
    s.canvas = Rect::from_size(100., 100., 900., 600.);
    let before = (
        (330. - s.canvas.left - s.pan.0) / s.zoom,
        (220. - s.canvas.top - s.pan.1) / s.zoom,
    );
    s.zoom_at(330., 220., 1.2);
    assert!(((330. - s.canvas.left - s.pan.0) / s.zoom - before.0).abs() < 0.01);
    assert!(((220. - s.canvas.top - s.pan.1) / s.zoom - before.1).abs() < 0.01);
}

#[test]
fn selecting_a_node_does_not_dirty_the_graph() {
    let mut s = state();
    s.canvas = Rect::from_size(0., 0., 900., 600.);
    s.begin(Some(&Hit::Node(0)), 70., 110.);
    s.motion(70., 110.);
    s.release(70., 110.);
    assert!(!s.dirty);
}

#[test]
fn result_paging_preserves_complete_unicode_output() {
    let mut s = state();
    s.result_text = "中🙂文".repeat(6000);
    s.result_page = 1;
    s.show_result_page();
    assert_eq!(s.field.text().chars().count(), 8000);
    assert_eq!(s.result_text.chars().count(), 18000);
    s.result_page = 2;
    s.show_result_page();
    assert_eq!(s.field.text().chars().count(), 2000);
}

#[test]
fn narrow_toolbar_and_node_actions_do_not_overlap() {
    let mut s = state();
    s.select_node(0);
    paint(
        &mut DrawList::new(),
        Rect::from_size(0., 0., 500., 720.),
        &mut s,
        false,
        theme::tokens().palette(false),
    );
    let find = |h| s.hits.iter().find(|(_, hit)| *hit == h).unwrap().0;
    assert!(find(Hit::Apply)
        .intersect(&find(Hit::PickObject))
        .is_empty());
    assert!(find(Hit::PickObject)
        .intersect(&find(Hit::Delete))
        .is_empty());
    assert!(find(Hit::Run).bottom <= s.canvas.top);
    assert!(find(Hit::More).right <= s.area.right);
}

#[test]
fn edges_can_be_selected_along_horizontal_segments() {
    let mut s = state();
    s.needs_fit = false;
    paint(
        &mut DrawList::new(),
        Rect::from_size(0., 0., 1400., 800.),
        &mut s,
        false,
        theme::tokens().palette(false),
    );
    let a = s.node_rect(&s.draft.as_ref().unwrap().nodes[0]);
    assert_eq!(
        s.hit(a.right + 35., a.top + 32. * s.zoom),
        Some(Hit::Edge(0))
    );
}

#[test]
fn modern_layout_preserves_graph_and_undo_restores_positions() {
    let mut s = state();
    s.canvas = Rect::from_size(0., 0., 1200., 700.);
    let before = s.draft.clone().unwrap();
    s.auto_layout();
    let g = s.draft.as_ref().unwrap();
    assert_eq!(g.edges, before.edges);
    assert!(g.nodes[1].position.x > g.nodes[0].position.x + 292.);
    s.undo(false);
    assert_eq!(s.draft.unwrap(), before);
}

#[test]
fn typed_parameters_preserve_unicode_and_reject_wrong_types() {
    let mut s = state();
    s.add_node("ai");
    s.edit_parameter("/inputs/prompt").unwrap();
    s.field.set_text("整理中文资料\n{{ $input.content }}");
    s.apply_node().unwrap();
    let i = s.selected_node.unwrap();
    assert_eq!(
        s.draft.as_ref().unwrap().nodes[i].inputs["prompt"],
        "整理中文资料\n{{ $input.content }}"
    );
    s.edit_parameter("/retries").unwrap();
    s.field.set_text("true");
    assert!(s.apply_node().is_err());
    assert_eq!(s.draft.as_ref().unwrap().nodes[i].retries, 0);
    s.field.set_text("2");
    s.apply_node().unwrap();
    assert_eq!(s.draft.as_ref().unwrap().nodes[i].retries, 2);
}

#[test]
fn inserting_into_a_branch_keeps_branch_handle_and_is_one_undo() {
    let mut s = state();
    s.draft.as_mut().unwrap().edges[0].source_handle = Some("false".into());
    let before = s.draft.clone().unwrap();
    s.insert_edge = Some(0);
    s.add_node("script");
    let g = s.draft.as_ref().unwrap();
    assert_eq!(g.edges.len(), 2);
    assert_eq!(g.edges[0].source_handle.as_deref(), Some("false"));
    assert_eq!(g.edges[0].target, g.nodes[2].id);
    assert_eq!(g.edges[1].source, g.nodes[2].id);
    assert_eq!(g.edges[1].target, "end");
    s.undo(false);
    assert_eq!(s.draft.unwrap(), before);
}

#[test]
fn palette_click_and_drag_create_nodes_without_saving() {
    let mut s = state();
    s.canvas = Rect::from_size(0., 0., 1000., 700.);
    s.pan = (30., 40.);
    s.zoom = 0.5;
    let kind = mochi_core::workflows::catalog::KINDS
        .iter()
        .position(|k| *k == "script")
        .unwrap();
    s.begin(Some(&Hit::Kind(kind)), 20., 30.);
    s.motion(500., 300.);
    s.release(500., 300.);
    let n = s.draft.as_ref().unwrap().nodes.last().unwrap();
    assert_eq!(n.kind, "script");
    assert_eq!(n.position.x, 944.);
    assert_eq!(n.position.y, 528.);
    assert!(s.dirty);
    s.undo(false);
    assert_eq!(s.draft.unwrap().nodes.len(), 2);
}

#[test]
fn duplicate_connections_and_cycles_do_not_change_history() {
    let mut s = state();
    s.hits
        .push((Rect::from_size(200., 0., 30., 30.), Hit::Input(0)));
    let before = s.draft.clone().unwrap();
    s.begin(Some(&Hit::Port(1, None)), 0., 0.);
    s.release(205., 5.);
    assert_eq!(s.draft.unwrap(), before);
    assert!(!s.dirty);
    assert!(s.error.contains("循环"));
}

#[test]
fn legacy_spacing_migration_is_reversible_and_keeps_edges() {
    let mut s = state();
    let mut saved = s.selected.clone().unwrap();
    saved.definition.nodes[1].position.x = 280.;
    let before = saved.definition.clone();
    s.open(saved);
    assert!(!s.has_overlaps());
    assert!(s.dirty);
    assert_eq!(s.draft.as_ref().unwrap().edges, before.edges);
    s.undo(false);
    assert_eq!(s.draft.unwrap(), before);
}

#[test]
fn group_drag_delete_and_undo_preserve_graph() {
    let mut s = state();
    s.canvas = Rect::from_size(0., 0., 1200., 700.);
    s.zoom = 1.;
    s.pan = (0., 0.);
    s.shift = true;
    s.begin(Some(&Hit::Node(0)), 70., 110.);
    s.begin(Some(&Hit::Node(1)), 630., 110.);
    assert_eq!(s.group.len(), 2);
    s.shift = false;
    let before = s.draft.clone().unwrap();
    s.begin(Some(&Hit::Node(0)), 70., 110.);
    s.motion(150., 190.);
    s.release(150., 190.);
    let g = s.draft.as_ref().unwrap();
    assert_eq!(g.nodes[1].position.x - g.nodes[0].position.x, 560.);
    s.undo(false);
    assert_eq!(s.draft.as_ref().unwrap(), &before);
    s.group.extend([0, 1]);
    s.remove_selected();
    assert!(s.draft.as_ref().unwrap().nodes.is_empty());
    assert!(s.draft.as_ref().unwrap().edges.is_empty());
    s.undo(false);
    assert_eq!(s.draft.unwrap(), before);
}

#[test]
fn trigger_parameters_are_typed_and_low_zoom_keeps_card_details() {
    let mut s = state();
    s.select_node(0);
    s.edit_parameter("/workflow/trigger/type").unwrap();
    s.field.set_text("daily");
    s.apply_node().unwrap();
    let trigger = serde_json::to_value(&s.draft.as_ref().unwrap().trigger).unwrap();
    assert_eq!(trigger["type"], "daily");
    assert_eq!(trigger["time"], "09:00");
    s.edit_parameter("/workflow/trigger/time").unwrap();
    s.field.set_text("25:90");
    assert!(s.apply_node().is_err());
    s.field.set_text("08:30");
    s.apply_node().unwrap();
    s.editor = None;
    s.selected_node = None;
    s.needs_fit = false;
    s.zoom = 0.54;
    let mut list = DrawList::new();
    paint(
        &mut list,
        Rect::from_size(0., 0., 1440., 900.),
        &mut s,
        false,
        theme::tokens().palette(false),
    );
    assert!(list.cmds().iter().filter(|c|matches!(c,crate::ui::draw::DrawCmd::ScaledText{scale,..} if (*scale-0.54).abs()<0.001)).count()>=6);
}
