use super::*;
#[test]
fn content_copy_hit_tracks_scroll_clip_and_rejects_stale_session() {
    use mochi_core::ai::session::AiStoredMessage;
    let mut state = State::default();
    state.active = Some(AiConversation {
        id: "s".into(),
        messages: vec![AiStoredMessage::new("user", "$x^2$")],
        ..Default::default()
    });
    let lay = layout(&state, Rect::from_size(80.0, 30.0, 420.0, 650.0));
    let (index, r, _) = lay.messages[0].body.copy_targets().next().unwrap();
    let origin = lay.body_origin(0, 12.0).unwrap();
    let x = origin.0 + (r.left + r.right) / 2.0;
    let y = origin.1 + (r.top + r.bottom) / 2.0;
    assert_eq!(lay.content_at(12.0, x, y, false), Some((0, index)));
    assert_eq!(lay.content_copy(&state, 0, index).unwrap().text, "x^2");
    assert!(lay.content_at(10000.0, x, y, false).is_none());
    assert!(lay
        .content_at(12.0, lay.messages_rect.right + 1.0, y, false)
        .is_none());
    let mut covered = lay.clone();
    covered.popover = Some(Rect::from_size(x - 1.0, y - 1.0, 3.0, 3.0));
    assert!(covered.content_at(12.0, x, y, false).is_none());
    state.active.as_mut().unwrap().id = "changed".into();
    assert!(lay.content_copy(&state, 0, index).is_none());
    state.active.as_mut().unwrap().id = "s".into();
    state.active.as_mut().unwrap().messages[0] = AiStoredMessage::new("user", "$other$");
    assert!(lay.content_copy(&state, 0, index).is_none());
}
#[test]
fn streaming_copy_rejects_changed_partial_reply() {
    let mut state = State::default();
    state.streaming = Some(Streaming {
        content: "$x$".into(),
        ..Default::default()
    });
    let lay = layout(&state, Rect::from_size(0.0, 0.0, 420.0, 650.0));
    let (i, _, _) = lay.messages[0].body.copy_targets().next().unwrap();
    assert_eq!(lay.content_copy(&state, 0, i).unwrap().text, "x");
    state.streaming.as_mut().unwrap().content.push_str(" tail");
    assert!(lay.content_copy(&state, 0, i).is_none());
}
#[test]
fn action_targets_are_frozen_and_clipped_to_visible_message_area() {
    let mut state = State::default();
    let mut message = mochi_core::ai::session::AiStoredMessage::new("assistant", "回复");
    message.set("id", serde_json::json!("answer"));
    state.active = Some(AiConversation {
        id: "session".into(),
        messages: vec![message],
        ..Default::default()
    });
    let area = Rect::from_size(0.0, 0.0, 420.0, 700.0);
    let lay = layout(&state, area);
    assert_eq!(
        lay.messages[0].action_target,
        Some(("session".into(), "answer".into()))
    );
    for (rect, action) in lay.message_buttons(0, 0.0) {
        assert_eq!(
            lay.message_hit(
                0.0,
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0
            ),
            Some(Hit::Message(0, action))
        );
    }
    assert!(lay.message_hit(1000.0, 20.0, 0.0).is_none());
    state.active.as_mut().unwrap().id = "another".into();
    assert_eq!(lay.messages[0].action_target.as_ref().unwrap().0, "session");
}
