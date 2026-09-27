use super::*;

fn state() -> State {
    State::new(
        (0..60)
            .map(|i| {
                Folder::new(
                    PathBuf::from(format!("library-{i}")),
                    format!("知识库 {i}"),
                    0,
                )
            })
            .collect(),
        &[PathBuf::from("报告.pdf"), PathBuf::from("附件.png")],
        None,
    )
}

#[test]
fn import_picker_scroll_and_hit_test_use_the_same_visible_rows() {
    for viewport in [
        Rect::from_size(0.0, 0.0, 1200.0, 800.0),
        Rect::from_size(0.0, 0.0, 560.0, 460.0),
    ] {
        let mut picker = state();
        picker.select(59, viewport);
        let layout = picker.layout(viewport);
        assert!(picker.scroll > 0.0);
        assert!(layout.tree.bottom < layout.cancel.top);
        let row = picker.row_rect(&layout, 59);
        assert!(row.top >= layout.tree.top);
        assert!(row.bottom <= layout.tree.bottom + 0.01);
        assert_eq!(
            picker.hit(viewport, row.right - 40.0, row.top + 16.0),
            Hit::Select(59)
        );
        assert_eq!(
            picker.hit(
                viewport,
                layout.confirm.left + 10.0,
                layout.confirm.top + 10.0
            ),
            Hit::Confirm
        );
        // 树下方被裁切的行绝不能拦截对底部区域的点击。
        assert_eq!(
            picker.hit(viewport, layout.tree.left + 20.0, layout.tree.bottom + 12.0),
            Hit::Inside
        );
        let mut list = DrawList::new();
        picker.paint(
            &mut list,
            viewport,
            &crate::ui::theme::configured_palette(false),
        );
        assert!(list.finish().is_ok());
    }
}

#[test]
fn import_picker_keyboard_navigation_and_collapsing_keep_selection_visible() {
    let viewport = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
    let mut picker = state();
    picker.folders[0].expanded = true;
    picker.folders.insert(
        1,
        Folder::new(PathBuf::from("library-0/child"), "子目录".into(), 1),
    );
    picker.key(0x28, false, viewport);
    assert_eq!(picker.selected_index(), Some(1));
    picker.collapse(0, viewport);
    assert_eq!(picker.selected_index(), Some(0));
    assert_eq!(picker.folders.len(), 60);
    picker.key(0x23, false, viewport);
    assert_eq!(picker.selected_index(), Some(59));
    picker.key(0x09, false, viewport);
    assert_eq!(picker.key(0x0d, false, viewport), Some(Hit::Cancel));
    picker.key(0x09, false, viewport);
    assert_eq!(picker.key(0x20, false, viewport), Some(Hit::Confirm));
    assert_eq!(picker.key(0x1b, false, viewport), Some(Hit::Cancel));
}

#[test]
fn import_picker_empty_workspace_has_no_active_confirm_target() {
    let viewport = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
    let mut picker = State::new(Vec::new(), &[], None);
    let layout = picker.layout(viewport);
    assert_eq!(
        picker.hit(
            viewport,
            layout.confirm.left + 10.0,
            layout.confirm.top + 10.0
        ),
        Hit::Inside
    );
    assert_eq!(picker.key(0x0d, false, viewport), None);
}
