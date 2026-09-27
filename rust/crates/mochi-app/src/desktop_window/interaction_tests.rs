use super::*;
use mochi_core::desktop_cards::{ItemStyle, Module};
fn spec(id: &str) -> Spec {
    Spec {
        id: id.into(),
        title: id.into(),
        x: 30,
        y: 40,
        width: 400,
        height: 400,
        locked: false,
        dark: false,
        appearance: Default::default(),
    }
}
#[test]
fn desktop_all_tabs_remain_reachable_in_both_orientations() {
    let mut spec = spec("tabs");
    let mut view = View {
        page_id: "7".into(),
        tabs: (0..8)
            .map(|i| (i.to_string(), format!("分页 {i}")))
            .collect(),
        ..Default::default()
    };
    let area = Rect::from_size(0.0, 0.0, 280.0, 220.0);
    for left in [false, true] {
        spec.appearance.tabs_left = left;
        view.tab_scroll = None;
        assert!(navigation::controls(&spec, &view, area)
            .iter()
            .any(|(_, h)| *h == Hit::Page("7".into())));
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10 {
            for (r, h) in navigation::controls(&spec, &view, area) {
                assert!(r.left >= 0.0 && r.right <= area.right && r.bottom <= area.bottom);
                if let Hit::Page(id) = h {
                    seen.insert(id);
                }
            }
            navigation::scroll(&spec, &mut view, area, -1);
        }
        assert_eq!(seen.len(), 8);
    }
}

#[test]
fn horizontal_tab_overflow_and_folder_chrome_do_not_overlap_at_narrow_widths() {
    use crate::ui::layout::Rect;

    let mut spec = spec("chrome-overflow");
    let view = View {
        module: Some(Module::Folder),
        page_id: "0".into(),
        tabs: (0..14)
            .map(|i| (i.to_string(), format!("分页 {i}")))
            .collect(),
        ..Default::default()
    };
    spec.appearance.tabs_left = false;

    for width in (280..=720).step_by(11) {
        let area = Rect::from_size(0.0, 0.0, width as f32, 320.0);
        let chrome: Vec<_> = painting::controls(&spec, &view, area, 0)
            .into_iter()
            .filter(|(_, hit)| {
                matches!(
                    hit,
                    Hit::Pin
                        | Hit::Capsule
                        | Hit::Page(_)
                        | Hit::TabScroll(_)
                        | Hit::FolderCommand(_)
                )
            })
            .collect();

        assert!(chrome
            .iter()
            .any(|(_, hit)| matches!(hit, Hit::TabScroll(_))));
        for (index, (rect, hit)) in chrome.iter().enumerate() {
            assert!(
                rect.left >= area.left && rect.right <= area.right,
                "width={width}, hit={hit:?}, rect={rect:?}"
            );
            for (other_rect, other_hit) in chrome.iter().skip(index + 1) {
                assert!(
                    rect.intersect(other_rect).is_empty(),
                    "width={width}, overlapping {hit:?} and {other_hit:?}: {rect:?} / {other_rect:?}"
                );
            }
        }
    }
}

#[test]
fn desktop_fixed_launcher_drag_and_temporary_dock_survive_sync() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut manager = Manager::default();
    let mut spec = spec("organizer");
    spec.appearance.edge_dock = true;
    let mut view = View {
        module: Some(Module::Shortcuts),
        page_id: "icons".into(),
        ..Default::default()
    };
    view.studio
        .add_shortcuts(&["C:/a.lnk".into(), "C:/b.lnk".into(), "C:/c.lnk".into()]);
    view.presentation.columns = 2;
    view.presentation.grid_height = 80;
    let a = Rect::from_size(0.0, 0.0, 400.0, 400.0);
    let b = Rect::from_size(0.0, 0.0, 400.0, 800.0);
    assert_eq!(
        shortcuts::controls(&spec, &view, a, 0)[0].0.height(),
        shortcuts::controls(&spec, &view, b, 0)[0].0.height()
    );
    manager.sync(HWND::default(), vec![(spec.clone(), view.clone())]);
    let hwnd = manager.windows[&spec.id];
    dock::toggle_suspended(hwnd);
    manager.sync(HWND::default(), vec![(spec.clone(), view.clone())]);
    unsafe {
        let s = state(hwnd).unwrap();
        assert!(s.spec.appearance.edge_dock && s.dock.suspended);
        assert_eq!(s.dock.progress, 1.0);
    }
    let r = shortcuts::controls(&spec, &view, a, 0)[0].0;
    shortcuts::pointer_down(hwnd, r.left + 10.0, r.top + 10.0);
    unsafe {
        state(hwnd).unwrap().shortcut_drag.as_mut().unwrap().since -=
            std::time::Duration::from_millis(400);
    }
    assert!(shortcuts::pointer_move(hwnd, 380.0, 210.0));
    assert!(shortcuts::pointer_up(hwnd));
    assert!(manager
        .events()
        .iter()
        .any(|e| matches!(e.kind, EventKind::ShortcutMove(_, 3))));
    dock::toggle_suspended(hwnd);
    unsafe {
        assert!(!state(hwnd).unwrap().dock.suspended);
        assert!(state(hwnd).unwrap().spec.appearance.edge_dock);
    }
}
#[test]
fn desktop_tree_window_messages_deliver_children_and_cache_collapsed_directories() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let root = std::env::temp_dir().join(format!("mochi-desktop-tree-{}", std::process::id()));
    std::fs::create_dir_all(root.join("Library/folder/deep")).unwrap();
    std::fs::write(root.join("Library/folder/note.md"), "note").unwrap();
    let mut view = View {
        workspace: root.clone(),
        module: Some(Module::Knowledge),
        page_id: "tree".into(),
        rows: vec![Row {
            id: "library".into(),
            title: "Library".into(),
            meta: mochi_core::desktop_cards::RowMeta {
                path: "Library".into(),
                directory: true,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    view.presentation.expand_libraries = true;
    let spec = spec("tree");
    let mut manager = Manager::default();
    manager.sync(HWND::default(), vec![(spec.clone(), view)]);
    let hwnd = manager.windows[&spec.id];
    let library = root.join("Library").to_string_lossy().into_owned();
    unsafe {
        let s = state(hwnd).unwrap();
        assert_eq!(s.view.tree_rows.len(), 1);
        tree::toggle(s, &library);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
            if state(hwnd).unwrap().view.tree_rows.len() == 2 {
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    unsafe {
        let s = state(hwnd).unwrap();
        assert_eq!(s.view.tree_rows.len(), 2, "only one level is loaded");
        assert_eq!(s.view.tree_rows[1].depth, 1);
        tree::toggle(s, &library);
        tree::toggle(s, &library);
        assert_eq!(s.view.tree_rows.len(), 2);
        assert!(
            s.view.tree_loading.is_empty(),
            "reopening must use cached children"
        );
    }
    manager.sync(HWND::default(), vec![]);
    // 测试夹具的根目录仅供当前测试进程使用。
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn desktop_single_item_color_overrides_do_not_leak_to_other_rows() {
    let spec = spec("colors");
    let mut view = View::default();
    view.presentation.item_foreground = Some(0x123456);
    view.item_styles.insert(
        "task-a".into(),
        ItemStyle {
            foreground: Some(0xffffff),
            background: Some(0x234567),
        },
    );
    let p = painting::palette(&spec);
    let (a, bg) = painting::item_palette(&p, &view, "task-a");
    let (b, other) = painting::item_palette(&p, &view, "task-b");
    assert_eq!(a.foreground, 0xffffff);
    assert_eq!(bg, Some(0x234567));
    assert_eq!(b.foreground, 0x123456);
    assert_eq!(other, None);
}
