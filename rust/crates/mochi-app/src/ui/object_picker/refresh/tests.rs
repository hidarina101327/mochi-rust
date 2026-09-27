use super::*;
use crate::ui::object_picker::Hit;
use mochi_core::object_reference::{build_ai_message_reference, ObjectKind, ObjectScope};

fn candidate(name: &str) -> ObjectCandidate {
    ObjectCandidate::new(
        format!("mochi://open?path={name}.md"),
        name,
        "",
        ObjectKind::Document,
    )
}

#[test]
fn refresh_appends_new_rows_without_resetting_search_selection_or_scroll() {
    let initial = ["b", "c", "d"].map(candidate).to_vec();
    let mut state = State::for_ai_mount(initial.clone(), [initial[1].url.clone()]);
    state.query.set_text(".md");
    state.selected_row = 1;
    state.scroll = ROW_HEIGHT + 7.0;
    state.update_candidates(["a", "b", "c", "d"].map(candidate).to_vec());
    assert_eq!(state.candidates, ["b", "c", "d", "a"].map(candidate));
    assert_eq!(state.query.text(), ".md");
    assert_eq!(state.scope, ObjectScope::Files);
    assert_eq!(state.selected, vec![initial[1].url.clone()]);
    assert_eq!(state.selected_row, 1);
    assert_eq!(state.scroll, ROW_HEIGHT + 7.0);
}

#[test]
fn deletion_keeps_keyboard_focus_and_viewport_on_the_same_surviving_objects() {
    let mut state = State::for_references(["a", "b", "c", "d"].map(candidate).to_vec(), []);
    state.selected_row = 2;
    state.scroll = ROW_HEIGHT + 7.0;
    state.update_candidates(["b", "c", "d"].map(candidate).to_vec());
    assert_eq!(state.selected_row, 1);
    assert_eq!(state.scroll, 7.0);
    state.update_candidates(["c", "d"].map(candidate).to_vec());
    assert_eq!(state.selected_row, 0);
    assert_eq!(state.scroll, 7.0);
    state.update_candidates(vec![]);
    assert!(state.candidates.is_empty());
    assert_eq!(state.scroll, 0.0);
    assert_eq!(state.selected_row, 0);
}

#[test]
fn metadata_edits_update_rows_and_selected_urls_without_duplicates() {
    let message = |title, snippet| {
        ObjectCandidate::new(
            build_ai_message_reference("session-1", "message-1", Some(title), Some(snippet))
                .unwrap(),
            title,
            snippet,
            ObjectKind::AiMessage,
        )
    };
    let old = message("旧标题", "旧内容");
    let updated = message("新标题", "新内容");
    let mut state = State::for_references(vec![old.clone(), candidate("b")], [old.url]);
    state.update_candidates(vec![
        candidate("a"),
        updated.clone(),
        candidate("b"),
        updated.clone(),
    ]);
    assert_eq!(
        state.candidates,
        vec![updated.clone(), candidate("b"), candidate("a")]
    );
    assert_eq!(state.selected, vec![updated.url]);
}

#[test]
fn unchanged_refresh_preserves_hover_and_document_filter_stays_local() {
    let mut state = State::for_ai_mount(vec![candidate("b")], []);
    state.hover = Some(Hit::Candidate(0));
    state.update_candidates(vec![candidate("b")]);
    assert_eq!(state.hover, Some(Hit::Candidate(0)));
    state.update_candidates(vec![
        candidate("b"),
        ObjectCandidate::new(
            "mochi://ai-session?session=session-1",
            "AI",
            "",
            ObjectKind::AiSession,
        ),
    ]);
    assert_eq!(state.visible_count(), 1);
    assert_eq!(state.candidates.len(), 2);
}
