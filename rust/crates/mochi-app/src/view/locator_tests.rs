use super::*;
#[test]
fn closing_a_document_releases_layout_without_discarding_an_open_tabs_cache() {
    let path = Path::new("notes/closed.md");
    let other = Path::new("notes/open.md");
    let source = "# 标题\n\n保留 **中文🙂** 以及链接 [Mochi](https://example.com)。\n";
    let mut pane = DocPane::default();
    pane.set_code_document(path);
    pane.set_base_dir(path.parent());
    pane.ensure(
        Rect::from_size(0.0, 0.0, 700.0, 500.0),
        Some(0),
        Some(source),
        None,
    );
    pane.table_offsets
        .entry(path.to_path_buf())
        .or_default()
        .insert(0, 12.0);
    pane.table_widths
        .entry(path.to_path_buf())
        .or_default()
        .insert(0, vec![180.0, 220.0]);
    pane.table_offsets
        .entry(other.to_path_buf())
        .or_default()
        .insert(0, 24.0);
    let key = pane.key;
    let lines = pane.live.layout.lines.as_ptr();
    pane.retain_open_documents(&[path, other]);
    assert_eq!(pane.key, key);
    assert_eq!(pane.live.layout.lines.as_ptr(), lines);
    assert!(!pane.live.parsed.blocks.is_empty());
    pane.retain_open_documents(&[other]);
    assert!(pane.live.parsed.blocks.is_empty());
    assert!(pane.live.layout.lines.is_empty());
    assert!(pane.key.is_none());
    assert!(pane.code_document.as_os_str().is_empty());
    assert!(!pane.table_offsets.contains_key(path));
    assert!(!pane.table_widths.contains_key(path));
    assert_eq!(pane.table_offsets[other][&0], 24.0);
}

#[test]
fn markdown_table_column_drag_geometry_survives_relayout_and_updates_cells() {
    let source = "| A | B |\n| --- | --- |\n| one | two |\n";
    let path = Path::new("notes/table.md");
    let area = Rect::from_size(20.0, 30.0, 640.0, 500.0);
    let mut pane = DocPane::default();
    pane.set_code_document(path);
    pane.hide_title(true);
    pane.set_table_widths(0, vec![210.0, 330.0]);
    pane.ensure(area, Some(0), Some(source), None);
    let handles = pane.table_column_handles(area, 0.0);
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].1, 0);
    let cells = pane.table_cells(area, source, 0.0);
    assert_eq!(cells.len(), 4);
    assert!((cells[0].rect.width() - 210.0).abs() < 0.1);
    assert!((cells[1].rect.width() - 330.0).abs() < 0.1);
    pane.invalidate();
    pane.ensure(area, Some(0), Some(source), None);
    assert!((pane.table_cells(area, source, 0.0)[0].rect.width() - 210.0).abs() < 0.1);
}

#[test]
fn card_hit_testing_respects_bounds_scroll_and_source_editing() {
    let url = "mochi://ai-locate?session=s&message=m&title=%E6%A0%87%E9%A2%98&snippet=hello";
    let source = format!("# 前文\n\n{url}\n");
    let area = Rect::from_size(10.0, 20.0, 500.0, 600.0);
    let mut pane = DocPane::default();
    pane.hide_title(true);
    pane.ensure(area, Some(0), Some(&source), None);
    let line = pane
        .live
        .layout
        .lines
        .iter()
        .find(|l| matches!(l.decoration, document::Decoration::AiLocator))
        .unwrap();
    let x = area.left + crate::ui::editor_preferences::current().padding_left + 8.0;
    let y = area.top + line.y + 15.0;
    assert_eq!(pane.link_at(area, &source, 0.0, x, y).as_deref(), Some(url));
    assert!(pane
        .link_at(area, &source, 0.0, x, area.top + line.y - 1.0)
        .is_none());
    assert!(pane.link_at(area, &source, 1000.0, x, y).is_none());
    pane.invalidate();
    pane.ensure(
        area,
        Some(0),
        Some(&source),
        Some(source.find(url).unwrap()),
    );
    assert_eq!(pane.link_at(area, &source, 0.0, x, y).as_deref(), Some(url));
}
