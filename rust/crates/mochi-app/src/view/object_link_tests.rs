use super::*;
#[test]
fn object_cards_and_text_blocks_open_only_the_visible_reference() {
    for (url, text_style) in [
        ("mochi://open?path=notes%2Fa.mc&label=参考", false),
        (
            "mochi://open?path=schedule&kind=task&item=t&label=复习&view=text",
            true,
        ),
        ("mochi://ai-locate?session=s&message=m&view=text", true),
    ] {
        let source = format!("# 前文\n\n{url}\n");
        let area = Rect::from_size(20.0, 30.0, 640.0, 600.0);
        let mut pane = DocPane::default();
        pane.hide_title(true);
        pane.ensure(area, Some(0), Some(&source), None);
        let line = pane
            .live
            .layout
            .lines
            .iter()
            .find(|l| matches!(l.decoration, document::Decoration::ObjectReference { .. }))
            .unwrap();
        let x = area.left + crate::ui::editor_preferences::current().padding_left + 12.0;
        let y = area.top + line.y + 12.0;
        assert_eq!(pane.link_at(area, &source, 0.0, x, y).as_deref(), Some(url));
        assert!(pane.link_at(area, &source, 1000.0, x, y).is_none());
        if text_style {
            assert!(pane
                .link_at(area, &source, 0.0, area.right - 30.0, y)
                .is_none());
        }
        let mut list = crate::ui::draw::DrawList::new();
        document::paint(
            &mut list,
            area,
            &pane.live.layout,
            0.0,
            crate::ui::theme::tokens().palette(false),
        );
        assert!(list.finish().is_ok());
    }
}
