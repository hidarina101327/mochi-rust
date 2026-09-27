use super::*;

#[test]
fn large_tree_only_places_viewport_rows_and_keeps_scroll_and_hit_coordinates() {
    let rows = (0..10_000)
        .map(|i| row(&format!("article-{i}.md"), 0, false, false))
        .collect::<Vec<_>>();
    let search = TextField::new("");
    let model = model(&rows, &search, None);
    let step = row_height() + row_gap();
    let capacity = ((AREA.height() - HEADER_HEIGHT) / step).ceil() as usize;
    for first in [0, 100, 5_000, 9_980] {
        let lay = layout(&model, AREA, first as f32 * step);
        assert!(lay.rows.len() <= capacity + 1);
        assert!(lay.entries.len() <= capacity + 6);
        assert_eq!(lay.content_height, 10_000.0 * step);
        let placement = &lay.rows[0];
        assert_eq!(placement.index, first);
        assert_eq!(lay.row_path(first), Some(rows[first].path.as_path()));
        assert_eq!(
            lay.hit(placement.rect.left + 60.0, placement.rect.top + 5.0),
            Some(SidebarHit::Row(first))
        );
    }
}

#[test]
fn scrolled_ancestors_and_filtered_siblings_keep_correct_guides() {
    let rows = vec![
        row("parent", 0, true, true),
        row("nested", 1, true, true),
        row("match.md", 2, false, false),
        row("hidden.md", 2, false, false),
        row("next-parent", 0, true, false),
    ];
    let search = TextField::new("");
    let lay = layout(
        &model(&rows, &search, None),
        AREA,
        2.0 * (row_height() + row_gap()),
    );
    assert_eq!(lay.rows[0].index, 2);
    assert_eq!(lay.rows[0].has_next, vec![true, false, true]);
    let search = TextField::new("").with_text("match");
    let lay = layout(&model(&rows, &search, None), AREA, 0.0);
    assert_eq!(lay.rows[2].has_next, vec![false, false, false]);
}

#[test]
fn offscreen_inline_editor_reserves_space_without_a_hit_target() {
    let rows = (0..1_000)
        .map(|i| row(&format!("article-{i}.md"), 0, false, false))
        .collect::<Vec<_>>();
    let search = TextField::new("");
    let editing = Editing::new(
        EditKind::NewFile {
            parent: PathBuf::new(),
        },
        "",
    );
    let step = row_height() + row_gap();
    let lay = layout(&model(&rows, &search, Some(&editing)), AREA, step * 10.0);
    assert!(lay.rect_of(SidebarHit::Editor).is_none());
    assert!(lay.editor_rect.is_none());
    assert_eq!(lay.rows[0].index, 9);
    assert_eq!(lay.content_height, 1001.0 * step);
}

#[test]
#[ignore = "manual timing, no wall-clock assertion"]
fn large_tree_layout_benchmark() {
    let search = TextField::new("");
    for count in [100, 1_000, 10_000] {
        let rows = (0..count)
            .map(|i| row(&format!("article-{i}.md"), 0, false, false))
            .collect::<Vec<_>>();
        let model = model(&rows, &search, None);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(layout(&model, AREA, 0.0));
        }
        eprintln!(
            "{count} rows: {:?}/layout, {} viewport placements",
            start.elapsed() / 100,
            layout(&model, AREA, 0.0).rows.len()
        );
    }
}
