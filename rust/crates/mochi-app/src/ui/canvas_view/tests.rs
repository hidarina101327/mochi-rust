use super::*;

const BODY: Rect = Rect {
    left: 100.0,
    top: 80.0,
    right: 900.0,
    bottom: 680.0,
};
fn state() -> State {
    State::parse(r#"{"version":1,"cards":[]}"#).unwrap()
}
fn draw(state: &mut State) {
    state.set_tool(Tool::Pen);
    state.pointer_down(BODY, 180.0, 180.0, false);
    state.pointer_move(BODY, 240.0, 220.0);
    state.release();
}

#[test]
fn canvas_ai_edit_preserves_user_work_and_rejects_active_gestures() {
    let mut s = state();
    draw(&mut s);
    let before = s.document.clone();
    let revision = mochi_core::canvas::drawing::revision(&before);
    let args = serde_json::json!({"expectedRevision":revision,"drawing":{"texts":[{"text":"第一行\n第二行\n第三行","x":200,"y":200,"width":200,"fontSize":24}]}});
    s.pointer_down(BODY, 400.0, 200.0, false);
    assert!(s.apply_ai_edit("canvas_draw", &args).is_err());
    s.release();
    assert!(s.apply_ai_edit("canvas_draw", &args).is_err()); // 手动绘制后，旧请求已过期
    assert!(s.undo_redo(false));
    s.apply_ai_edit("canvas_draw", &args).unwrap();
    assert_eq!(s.document.strokes, before.strokes);
    assert!(s.document.texts[0].height > 90.0);
    assert_eq!(s.undo.len(), 2); // 原有的手动绘制 + 一批 AI 操作
    assert!(s.undo_redo(false));
    assert_eq!(s.document, before);
}

#[test]
fn zoom_keeps_the_pointer_world_position_stable() {
    let mut s = state();
    s.document.viewport.x = -500.0;
    s.document.viewport.y = 200.0;
    let before = s.world(BODY, 420.0, 300.0);
    assert!(s.zoom_at(BODY, 420.0, 300.0, 1.2));
    let after = s.world(BODY, 420.0, 300.0);
    assert!((before.x - after.x).abs() < 0.001 && (before.y - after.y).abs() < 0.001);
    assert!(!s.zoom_at(BODY, 420.0, 300.0, f64::NAN));
}

#[test]
fn ink_uses_world_coordinates_and_survives_save_reload() {
    let mut s = state();
    s.document.viewport.x = -500.0;
    s.document.viewport.zoom = 2.0;
    draw(&mut s);
    assert_eq!(
        s.document.strokes[0].points[0],
        Point { x: -460.0, y: 50.0 }
    );
    assert_eq!(
        State::parse(&s.serialized().unwrap()).unwrap().document,
        s.document
    );
}

#[test]
fn tap_is_a_visible_stroke_and_cancel_does_not_save_draft() {
    let mut s = state();
    s.set_tool(Tool::Pen);
    s.pointer_down(BODY, 180.0, 180.0, false);
    s.release();
    assert_eq!(s.document.strokes[0].points.len(), 1);
    s.pointer_down(BODY, 220.0, 220.0, false);
    s.cancel_gesture();
    assert_eq!(s.document.strokes.len(), 1);
}

#[test]
fn eraser_is_one_undoable_gesture_and_does_not_erase_text() {
    let mut s = state();
    draw(&mut s);
    s.create_text(Point { x: 80.0, y: 100.0 });
    s.editor.as_mut().unwrap().field.buffer.insert("保留文字");
    s.finish_editing();
    s.set_tool(Tool::Eraser);
    s.pointer_down(BODY, 160.0, 180.0, false);
    s.pointer_move(BODY, 260.0, 220.0);
    s.release();
    assert!(s.document.strokes.is_empty());
    assert_eq!(s.document.texts.len(), 1);
    assert!(s.undo_redo(false));
    assert_eq!(s.document.strokes.len(), 1);
    assert!(s.undo_redo(true));
    assert!(s.document.strokes.is_empty());
}

#[test]
fn text_supports_chinese_ime_multiline_and_whole_note_undo() {
    let mut s = state();
    s.create_text(Point {
        x: -3000.0,
        y: 2000.0,
    });
    s.editor
        .as_mut()
        .unwrap()
        .field
        .buffer
        .set_composition("中文", 2);
    s.sync_text();
    assert_eq!(s.document.texts[0].text, "");
    s.editor
        .as_mut()
        .unwrap()
        .field
        .buffer
        .commit_composition("中文😀");
    s.sync_text();
    s.text_key(13, false, false);
    s.editor.as_mut().unwrap().field.buffer.insert("第二行");
    s.finish_editing();
    assert_eq!(s.document.texts[0].text, "中文😀\n第二行");
    assert!(s.undo_redo(false));
    assert!(s.document.texts.is_empty());
    assert!(s.undo_redo(true));
    assert_eq!(s.document.texts.len(), 1);
}

#[test]
fn empty_text_is_discarded_without_an_undo_entry() {
    let mut s = state();
    s.create_text(Point { x: 1.0, y: 2.0 });
    s.finish_editing();
    assert!(s.document.texts.is_empty());
    assert!(!s.undo_redo(false));
}

#[test]
fn moving_a_stroke_can_be_undone_without_resetting_viewport() {
    let mut s = state();
    draw(&mut s);
    let original = s.document.strokes[0].clone();
    s.set_tool(Tool::Select);
    s.pointer_down(BODY, 180.0, 180.0, false);
    assert_eq!(s.selected, Some(Element::Stroke(0)));
    s.pointer_move(BODY, 200.0, 210.0);
    s.release();
    assert_ne!(s.document.strokes[0], original);
    s.document.viewport.x = 999.0;
    s.undo_redo(false);
    assert_eq!(s.document.strokes[0], original);
    assert_eq!(s.document.viewport.x, 999.0);
}

#[test]
fn fit_frames_text_strokes_and_references_including_negative_coordinates() {
    let mut s = state();
    draw(&mut s);
    s.add_references(["mochi://open?path=notes%2Fa.md&kind=file&label=A".into()])
        .unwrap();
    s.create_text(Point {
        x: -3200.0,
        y: 1700.0,
    });
    s.editor.as_mut().unwrap().field.buffer.insert("远处的笔记");
    s.finish_editing();
    s.fit_to_content(BODY);
    let rect = s.screen_rect(BODY, s.document.bounds().unwrap());
    assert!(
        rect.left >= BODY.left
            && rect.top >= BODY.top
            && rect.right <= BODY.right
            && rect.bottom <= BODY.bottom
    );
}

#[test]
fn toolbar_hit_regions_do_not_overlap_surface_on_narrow_windows() {
    for width in [260.0, 420.0, 900.0] {
        let mut s = state();
        s.create_text(Point {
            x: -300.0,
            y: -500.0,
        });
        let l = layout(&s, Rect::from_size(0.0, 0.0, width, 600.0));
        for (rect, hit) in &l.entries {
            if *hit != Hit::Surface && *hit != Hit::Resize {
                assert!(rect.intersect(&l.body).is_empty(), "{hit:?}");
            }
        }
    }
}

#[test]
fn invalid_reference_batch_is_atomic() {
    let mut s = state();
    assert!(s
        .add_references(["mochi://open?path=a.md&kind=file".into(), "invalid".into()])
        .is_err());
    assert!(s.document.cards.is_empty());
}

#[test]
fn styling_text_keeps_editing_and_resizing_reflows_without_losing_content() {
    let mut s = state();
    s.create_text(Point { x: 100.0, y: 100.0 });
    s.editor
        .as_mut()
        .unwrap()
        .field
        .buffer
        .insert("这是一段需要自动换行的中文自由笔记。".repeat(5).as_str());
    s.sync_text();
    s.apply_style(Hit::Color(2));
    s.apply_style(Hit::FontSize(2));
    assert!(s.editor.is_some());
    assert_eq!(s.document.texts[0].color, COLORS[2]);
    assert_eq!(s.document.texts[0].font_size, FONT_SIZES[2]);
    let text = s.document.texts[0].text.clone();
    s.finish_editing();
    let height = s.document.texts[0].height;
    s.begin_resize();
    s.pointer_move(BODY, 350.0, 400.0);
    s.release();
    assert!(s.document.texts[0].height > height);
    assert_eq!(s.document.texts[0].text, text);
    s.undo_redo(false);
    assert_eq!(s.document.texts[0].width, 320.0);
}
