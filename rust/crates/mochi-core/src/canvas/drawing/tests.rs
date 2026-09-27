use super::*;

fn drawing(value: Value) -> Drawing {
    serde_json::from_value(value).unwrap()
}

#[test]
fn canvas_drawing_cat_is_editable_and_roundtrips() {
    let before = CanvasDocument::empty();
    let cat: Drawing = serde_json::from_str(include_str!("cat.json")).unwrap();
    let (next, result) = apply(&before, &revision(&before), cat).unwrap();
    assert!(before.strokes.is_empty());
    assert_eq!(next.texts[0].text, "今天也要开心");
    assert!(next.strokes.len() > 10);
    assert!(next.strokes.iter().any(|s| s.points.len() > 20));
    assert_eq!(
        result["addedIds"].as_array().unwrap().len(),
        next.strokes.len() + 1
    );
    assert_eq!(
        crate::canvas::parse(&crate::canvas::serialize(&next).unwrap()).unwrap(),
        next
    );
}

#[test]
fn canvas_drawing_replaces_only_named_objects_and_detects_stale_retries() {
    let empty = CanvasDocument::empty();
    let input = json!({"ellipses":[{"cx":-40,"cy":30,"rx":10,"ry":20}],"texts":[{"text":"保留我","x":20,"y":30}]});
    let (mut first, _) = apply(&empty, &revision(&empty), drawing(input)).unwrap();
    let rev = revision(&first);
    first.viewport.x = 777.0;
    first.viewport.zoom = 0.25;
    assert_eq!(revision(&first), rev);
    let stroke = first.strokes[0].id.clone();
    let replacement = json!({"replaceIds":[stroke],"paths":[{"commands":[["M",0,0],["L",5,6]]}]});
    let (second, _) = apply(&first, &rev, drawing(replacement.clone())).unwrap();
    assert_eq!(second.texts, first.texts);
    assert_eq!(second.viewport, first.viewport);
    assert_eq!(second.strokes.len(), 1);
    assert_ne!(second.strokes[0].id, stroke);
    assert!(apply(&second, &rev, drawing(replacement))
        .unwrap_err()
        .to_string()
        .contains("变化"));
}

#[test]
fn canvas_drawing_rejects_invalid_batches_without_changing_input() {
    let empty = CanvasDocument::empty();
    for invalid in [
        json!({"paths":[{"commands":[["L",0,0]]}]}),
        json!({"paths":[{"commands":[["M",0,0],["Q",1,2]]}]}),
        json!({"paths":[{"commands":[["M",0,0],["L",10000001,0]]}]}),
        json!({"paths":[{"commands":[["M",0,0]],"color":"red"}]}),
        json!({"ellipses":[{"cx":0,"cy":0,"rx":-1,"ry":1}]}),
        json!({"texts":[{"text":"ok","x":0,"y":0},{"text":"bad","x":0,"y":0,"fontSize":0}]}),
        json!({"replaceIds":["missing"]}),
        json!({}),
    ] {
        assert!(
            apply(&empty, &revision(&empty), drawing(invalid.clone())).is_err(),
            "{invalid}"
        );
        assert_eq!(empty, CanvasDocument::empty());
    }
    let many = json!({"ellipses":(0..100).map(|_|json!({"cx":0,"cy":0,"rx":10000,"ry":10000})).collect::<Vec<_>>()});
    assert!(apply(&empty, &revision(&empty), drawing(many))
        .unwrap_err()
        .to_string()
        .contains("20000"));
}

#[test]
fn canvas_drawing_curves_close_exactly_and_ellipses_have_correct_bounds() {
    let before = CanvasDocument::empty();
    let (next,_)=apply(&before,&revision(&before),drawing(json!({"paths":[{"commands":[["M",0,0],["C",0,100,100,100,100,0],["Z"]]}],"ellipses":[{"cx":50,"cy":50,"rx":30,"ry":20,"width":2}]}))).unwrap();
    let curve = &next.strokes[0].points;
    assert_eq!(curve.first(), curve.last());
    assert!(curve.iter().any(|p| p.y >= 74.0));
    let ellipse = &next.strokes[1];
    assert_eq!(ellipse.points.first(), ellipse.points.last());
    let bounds = ellipse.bounds().unwrap();
    assert!((bounds.left - 19.0).abs() < 0.5 && (bounds.right - 81.0).abs() < 0.5);
    let mut next = next;
    next.texts.push(TextNote {
        id: "text-metrics".into(),
        x: 0.0,
        y: 0.0,
        width: 300.0,
        height: 101.23077392578125,
        text: "精确恢复排版高度".into(),
        color: 0x334155,
        font_size: 28.0,
    });
    let restored = crate::canvas::parse(&crate::canvas::serialize(&next).unwrap()).unwrap();
    assert_eq!(revision(&restored), revision(&next));
    assert_eq!(restored, next);
}
