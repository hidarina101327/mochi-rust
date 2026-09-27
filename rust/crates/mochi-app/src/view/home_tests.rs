use super::*;

#[test]
fn home_keyboard_navigation_reveals_every_destination_and_hover_clears_on_scroll() {
    let area = Rect::from_size(0.0, 0.0, 620.0, 220.0);
    let mut pane = HomePane::default();
    pane.set_dashboard(home::Dashboard::default());
    pane.set_analytics(mochi_core::analytics::empty_snapshot(
        365,
        chrono::Local::now(),
    ));
    assert!(pane.navigate(area, false));
    assert_eq!(pane.focused_action(), Some(home::Action::NewNote));
    let first = pane.keyboard_focus.unwrap();
    for _ in 0..100 {
        let page = pane.page.as_ref().unwrap();
        let rect = page.action_rect(pane.keyboard_focus.unwrap()).unwrap();
        assert!(rect.top - pane.scroll >= area.top);
        assert!(rect.bottom - pane.scroll <= area.bottom);
        pane.set_hover(area, rect.left + 2.0, rect.top - pane.scroll + 2.0);
        assert!(pane.hover.is_some());
        pane.scroll_by(area, -16.0);
        assert!(pane.hover.is_none());
        assert!(pane.navigate(area, false));
        if pane.keyboard_focus == Some(first) {
            break;
        }
    }
    assert_eq!(pane.keyboard_focus, Some(first));
    assert!(pane.clear_keyboard_focus());
    assert!(pane.focused_action().is_none());
    assert!(pane.navigate(area, true));
    pane.reset();
    assert!(pane.focused_action().is_none());
    assert!(pane.hover.is_none());
}
