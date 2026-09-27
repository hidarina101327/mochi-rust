use super::*;

#[test]
fn native_desktop_window_can_create_dispatch_and_release_without_an_owner() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut manager = Manager::default();
    let spec = Spec {
        // 纯色背景会按窗口透明度淡化整个分层窗口。
        appearance: mochi_core::desktop_cards::Appearance {
            background_color: None,
            ..Default::default()
        },
        id: "native-lifecycle".into(),
        title: "生命周期验证".into(),
        x: 40,
        y: 40,
        width: 280,
        height: 220,
        locked: false,
        dark: false,
    };
    let view = View {
        page_id: "a".into(),
        tabs: vec![("a".into(), "日程".into())],
        ..Default::default()
    };
    assert!(manager
        .sync(HWND::default(), vec![(spec.clone(), view.clone())])
        .is_empty());
    let hwnd = manager.windows[&spec.id];
    unsafe {
        assert!(IsWindow(Some(hwnd)).as_bool());
        let mut alpha = 0u8;
        GetLayeredWindowAttributes(hwnd, None, Some(&mut alpha), None).unwrap();
        assert_eq!(alpha, 216);
        assert!(!IsWindowVisible(hwnd).as_bool());
        assert!(GetWindow(hwnd, GW_OWNER).unwrap_or_default().is_invalid());
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0,
            0
        );
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0,
            0
        );
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0,
            0
        );
        assert_eq!(
            SendMessageW(hwnd, WM_MOUSEACTIVATE, Some(WPARAM(0)), Some(LPARAM(0))).0,
            MA_NOACTIVATE as isize
        );
        let point = LPARAM((150 << 16) | 100);
        let _ = SendMessageW(hwnd, WM_LBUTTONDOWN, Some(WPARAM(0)), Some(point));
        let _ = SendMessageW(hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(point));
        assert!(!manager.events().iter().any(|e| e.kind == EventKind::Hide));
        let _ = SendMessageW(hwnd, WM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
    }
    assert!(manager
        .events()
        .iter()
        .any(|e| e.card == spec.id && e.kind == EventKind::Hide));
    assert!(manager.sync(HWND::default(), vec![]).is_empty());
    assert!(manager.is_empty());
    unsafe {
        assert!(!IsWindow(Some(hwnd)).as_bool());
    }
}

#[test]
fn glass_cards_blur_behind_instead_of_fading_and_switch_back_to_layered() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut manager = Manager::default();
    let mut spec = Spec {
        appearance: Default::default(),
        id: "glass-lifecycle".into(),
        title: "毛玻璃验证".into(),
        x: 40,
        y: 40,
        width: 280,
        height: 220,
        locked: false,
        dark: false,
    };
    let view = View {
        page_id: "a".into(),
        tabs: vec![("a".into(), "日程".into())],
        ..Default::default()
    };
    assert!(painting::is_glass(&spec));
    assert!(manager
        .sync(HWND::default(), vec![(spec.clone(), view.clone())])
        .is_empty());
    let hwnd = manager.windows[&spec.id];
    let layered = || unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LAYERED.0 != 0 };
    let glass = unsafe { state(hwnd) }.is_some_and(|s| s.glass);
    // 系统不支持强调色 API 时，使用淡化后的分层窗口作为备用方案。
    assert_eq!(layered(), !glass);
    spec.appearance.background_color = Some(0xf5f5f2);
    assert!(manager
        .sync(HWND::default(), vec![(spec.clone(), view.clone())])
        .is_empty());
    assert!(layered());
    assert!(!unsafe { state(hwnd) }.is_some_and(|s| s.glass));
    assert!(manager.sync(HWND::default(), vec![]).is_empty());
}

#[test]
fn context_menu_defers_layer_refresh_and_drawer_motion_until_closed() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut manager = Manager::default();
    let spec = Spec {
        id: "context-lifecycle".into(),
        title: String::new(),
        x: 80,
        y: 120,
        width: 400,
        height: 400,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    assert!(manager
        .sync(HWND::default(), vec![(spec.clone(), View::default())])
        .is_empty());
    let hwnd = manager.windows[&spec.id];
    unsafe {
        // 使用独立的 Z 层级，即使测试窗口不可见，也能观察到任何意外的位置变化，
        // 而且不会在用户桌面上打开菜单。
        state(hwnd).unwrap().spec.appearance.pinned = true;
        dock::configure(hwnd);
        state(hwnd).unwrap().spec.appearance.pinned = false;
        state(hwnd).unwrap().context_active = true;
        let mut changed = spec.clone();
        changed.locked = true;
        manager.sync(HWND::default(), vec![(changed, View::default())]);
        manager.arrange(HWND::default());
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
            0
        );
        state(hwnd).unwrap().spec.appearance.edge_dock = true;
        state(hwnd).unwrap().spec.appearance.pinned = true;
        dock::tick(hwnd);
        assert_eq!(state(hwnd).unwrap().dock.progress, 0.0);
        state(hwnd).unwrap().context_active = false;
        dock::tick(hwnd);
        assert!(state(hwnd).unwrap().dock.progress > 0.0);

        state(hwnd).unwrap().spec.appearance.edge_dock = false;
        state(hwnd).unwrap().spec.appearance.pinned = false;
        dock::configure(hwnd);
        manager.arrange(HWND::default());
        assert_eq!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
            0
        );
    }
}

#[test]
fn desktop_drawer_and_pin_use_global_topmost_without_taking_focus() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut manager = Manager::default();
    let mut spec = Spec {
        id: "dock-lifecycle".into(),
        title: String::new(),
        x: 80,
        y: 120,
        width: 400,
        height: 400,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    spec.appearance.edge_dock = true;
    let view = View::default();
    assert!(manager
        .sync(HWND::default(), vec![(spec.clone(), view.clone())])
        .is_empty());
    let hwnd = manager.windows[&spec.id];
    unsafe {
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
            0
        );
        let foreground = GetForegroundWindow();
        state(hwnd).unwrap().spec.appearance.pinned = true;
        dock::tick(hwnd);
        assert_eq!(GetForegroundWindow(), foreground);
    }
    spec.appearance.edge_dock = false;
    spec.appearance.pinned = true;
    manager.sync(HWND::default(), vec![(spec.clone(), view.clone())]);
    unsafe {
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
            0
        );
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).unwrap();
        assert_eq!((r.left, r.top), (80, 120));
    }
    spec.appearance.pinned = false;
    manager.sync(HWND::default(), vec![(spec.clone(), view.clone())]);
    unsafe {
        assert_eq!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
            0
        );
    }
    spec.appearance.edge_dock = true;
    manager.sync(HWND::default(), vec![(spec, view)]);
    unsafe {
        assert_ne!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0,
            0
        );
    }
}

#[test]
fn positions_survive_negative_and_disconnected_displays() {
    let work = RECT {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1040,
    };
    let r = clamp_geometry(-2500, 1100, 360, 440, work);
    assert_eq!(
        (r.left, r.top, r.right, r.bottom),
        (-1920, 600, -1560, 1040)
    );
    let r = clamp_geometry(9000, -5000, 4000, 3000, work);
    assert_eq!((r.left, r.top, r.right, r.bottom), (-1920, 0, 0, 1040));
}

#[test]
fn compact_card_controls_fit_and_checkbox_does_not_overlap_open_action() {
    let spec = Spec {
        appearance: Default::default(),
        id: "c".into(),
        title: "待办".into(),
        x: 0,
        y: 0,
        width: 280,
        height: 220,
        locked: false,
        dark: false,
    };
    let view = View {
        page_id: "p".into(),
        tabs: (0..8).map(|i| (format!("p{i}"), "页面".into())).collect(),
        rows: (0..12)
            .map(|i| Row {
                id: i.to_string(),
                title: "条目".into(),
                detail: "详情".into(),
                checked: Some(false),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let a = Rect::new(0.0, 0.0, 280.0, 220.0);
    let hits = painting::controls(&spec, &view, a, 0);
    assert!(painting::scroll_max(&spec, &view, a) > 0);
    for (r, _) in &hits {
        assert!(r.left >= 0.0 && r.top >= 0.0 && r.right <= 280.0 && r.bottom <= 220.0);
    }
    assert_eq!(
        hits.iter()
            .filter(|(_, h)| matches!(h, Hit::Row(_, true)))
            .count(),
        5
    );
    for (r, hit) in &hits {
        if let Hit::Row(id, true) = hit {
            let text = hits
                .iter()
                .find(|(_, h)| *h == Hit::Row(id.clone(), false))
                .unwrap()
                .0;
            assert!(text.right <= r.left);
        }
    }
}

#[test]
fn desktop_card_has_content_only_and_clicks_do_not_hide() {
    let spec = Spec {
        id: "minimal".into(),
        title: "不要展示的卡片标题".into(),
        x: 0,
        y: 0,
        width: 480,
        height: 480,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    let view = View {
        page_id: "p".into(),
        tabs: vec![("p".into(), "日程".into())],
        subtitle: "不要展示的描述".into(),
        ..Default::default()
    };
    let area = Rect::from_size(0.0, 0.0, 480.0, 480.0);
    let hits = painting::controls(&spec, &view, area, 0);
    assert!(!hits
        .iter()
        .any(|(_, h)| matches!(h, Hit::Lock | Hit::Manage | Hit::Hide | Hit::Open)));
    let mut list = DrawList::new();
    painting::paint(&mut list, &spec, &view, area, 0, None, None);
    assert!(list.finish().is_ok());
}

#[test]
fn customized_rows_keep_paint_and_hit_capacity_in_sync() {
    let mut spec = Spec {
        appearance: Default::default(),
        id: "c".into(),
        title: "test".into(),
        x: 0,
        y: 0,
        width: 280,
        height: 220,
        locked: false,
        dark: false,
    };
    let view = View {
        rows: (0..12)
            .map(|i| Row {
                id: i.to_string(),
                checked: Some(false),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let area = Rect::from_size(0.0, 0.0, 280.0, 220.0);
    for spacing in 0..=2 {
        for details in [false, true] {
            for size in [12, 18, 24] {
                spec.appearance.spacing = spacing;
                spec.appearance.show_details = details;
                spec.appearance.font_size = size;
                let hits = painting::controls(&spec, &view, area, 0);
                let first = hits
                    .iter()
                    .find(|(_, h)| *h == Hit::Row("0".into(), false))
                    .unwrap()
                    .0;
                assert!(first.height() >= size as f32 + 8.0);
                let end = painting::scroll_max(&spec, &view, area);
                assert!(painting::controls(&spec, &view, area, end)
                    .iter()
                    .any(|(_, h)| *h == Hit::Row("11".into(), false)));
                for (rect, _) in hits {
                    assert!(rect.bottom <= area.bottom);
                }
            }
        }
    }
}

#[test]
#[ignore = "offscreen appearance review"]
fn render_appearance_review() {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
    }
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/desktop-verification/ui-refactor");
    std::fs::create_dir_all(&output).unwrap();
    for (name, background, font, details) in [
        ("desktop-paper", 0xf5f5f2, 18, false),
        ("desktop-blue", 0xd6e5ee, 20, false),
        ("desktop-dark", 0x152c42, 24, true),
    ] {
        let mut spec = Spec {
            appearance: Default::default(),
            id: "preview".into(),
            title: "收集箱".into(),
            x: 0,
            y: 0,
            width: 480,
            height: 340,
            locked: false,
            dark: false,
        };
        spec.appearance.background_color = Some(background);
        spec.appearance.font_size = font;
        spec.appearance.show_details = details;
        let view = View {
            page_id: "p".into(),
            tabs: vec![("p".into(), "收集箱".into())],
            rows: ["明年四月的安排", "完成 writing-agent", "给简历做一次整理"]
                .into_iter()
                .enumerate()
                .map(|(i, title)| Row {
                    id: i.to_string(),
                    title: title.into(),
                    detail: "今天 · 待处理".into(),
                    checked: Some(false),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut list = DrawList::new();
        painting::paint(
            &mut list,
            &spec,
            &view,
            Rect::from_size(0.0, 0.0, 480.0, 340.0),
            0,
            None,
            None,
        );
        list.finish().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let snapshot = renderer.prepare_snapshot(720, 510, 144.0).unwrap();
        renderer
            .present(HWND::default(), background, &list)
            .unwrap();
        renderer
            .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
            .unwrap();
    }
    unsafe {
        CoUninitialize();
    }
}

#[test]
#[ignore = "native rendering of the new desktop presentations"]
fn render_desktop_presentations_review() {
    use chrono::Datelike;
    use mochi_core::desktop_cards::{Module, RowMeta};
    unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
        .ok()
        .unwrap();
    }
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/desktop-verification/ui-refactor");
    std::fs::create_dir_all(&output).unwrap();
    for name in [
        "workbench-grid",
        "schedule-list",
        "calendar-expanded",
        "calendar-collapsed",
        "knowledge-tree",
        "compact-list",
    ] {
        let mut spec = Spec {
            id: "preview".into(),
            title: String::new(),
            x: 0,
            y: 0,
            width: 560,
            height: 600,
            locked: false,
            dark: false,
            appearance: Default::default(),
        };
        spec.appearance.background_color = Some(0xf7f9fc);
        let mut view = View {
            page_id: "p".into(),
            tabs: vec![("p".into(), "工作台".into())],
            ..Default::default()
        };
        if name == "workbench-grid" {
            view.module = Some(Module::Home);
            view.presentation.grid = true;
            view.presentation.columns = 3;
            view.presentation.rows = 5;
            view.rows = Module::Home
                .options()
                .into_iter()
                .map(|o| Row {
                    id: o.key.clone(),
                    title: o.label,
                    meta: RowMeta {
                        icon: o.key,
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect();
        } else if name.starts_with("calendar") || name == "schedule-list" {
            view.module = Some(Module::Schedule);
            view.tabs[0].1 = "日程".into();
            view.presentation.schedule_view = if name.starts_with("calendar") { 1 } else { 0 };
            view.presentation.calendar_expanded = name != "calendar-collapsed";
            spec.appearance.show_details = name == "schedule-list";
            let today = chrono::Local::now().date_naive();
            let first = today.with_day(1).unwrap();
            for i in 0..20 {
                let date = first + chrono::Duration::days(if i < 6 { 2 } else { (i - 3) as i64 });
                view.rows.push(Row {
                    id: i.to_string(),
                    title: ["项目评审", "整理本周计划", "阅读与记录", "团队讨论"][i % 4].into(),
                    detail: "紧急 · 准备讨论材料".into(),
                    checked: Some(i == 8),
                    meta: RowMeta {
                        date: date.to_string(),
                        group: date.to_string(),
                        ..Default::default()
                    },
                });
            }
            if name.starts_with("calendar") {
                spec.appearance.font_size = 14;
            }
        } else {
            view.module = Some(Module::Knowledge);
            view.tabs[0].1 = "知识库".into();
            view.presentation.expand_libraries = true;
            view.rows = [
                ("个人知识库", true, 0, "BookOpen"),
                ("项目资料", true, 1, "folder"),
                ("产品设计", false, 2, "file"),
                ("读书记录", false, 1, "file"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (title, dir, depth, icon))| Row {
                id: i.to_string(),
                title: title.into(),
                meta: RowMeta {
                    path: format!("root/{i}"),
                    directory: dir,
                    depth,
                    icon: icon.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
            view.tree_rows = view
                .rows
                .iter()
                .map(|r| crate::shell::Row {
                    name: r.title.clone(),
                    path: std::path::PathBuf::from(&r.meta.path),
                    depth: r.meta.depth,
                    is_dir: r.meta.directory,
                    is_mapped_folder: false,
                    expanded: r.meta.directory,
                    has_children: r.meta.directory,
                })
                .collect();
            if name == "compact-list" {
                spec.appearance.row_padding = 0;
                spec.appearance.show_single_page_name = false;
                view.presentation.show_numbers = true;
                view.presentation.show_icons = false;
            }
        }
        let mut list = DrawList::new();
        painting::paint(
            &mut list,
            &spec,
            &view,
            Rect::from_size(0.0, 0.0, 560.0, 600.0),
            0,
            None,
            None,
        );
        list.finish().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let snapshot = renderer.prepare_snapshot(840, 900, 144.0).unwrap();
        renderer.present(HWND::default(), 0xf7f9fc, &list).unwrap();
        renderer
            .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
            .unwrap();
    }
    unsafe {
        windows::Win32::System::Com::CoUninitialize();
    }
}

#[test]
fn desktop_ai_composer_accepts_chinese_and_sends_without_opening_main() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut m = Manager::default();
    let spec = Spec {
        id: "composer-test".into(),
        title: "AI".into(),
        x: 40,
        y: 40,
        width: 480,
        height: 480,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    let v = View {
        module: Some(mochi_core::desktop_cards::Module::Ai),
        ..Default::default()
    };
    assert!(m.sync(HWND::default(), vec![(spec.clone(), v)]).is_empty());
    let hwnd = m.windows[&spec.id];
    unsafe {
        let s = state(hwnd).unwrap();
        let edit = s.composer.expect("IME text input");
        let _ = SetWindowTextW(edit, w!("测试中文输入"));
        assert_eq!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0,
            0
        );
        widgets::send(hwnd);
    }
    assert!(m
        .events()
        .iter()
        .any(|e| e.kind == EventKind::AiSend("测试中文输入".into())));
    m.clear_composer(&spec.id);
    unsafe {
        assert_eq!(
            GetWindowTextLengthW(state(hwnd).unwrap().composer.unwrap()),
            0
        );
    }
    m.sync(HWND::default(), vec![]);
}
#[test]
fn desktop_grid_height_and_custom_focus_palette_are_stable() {
    let mut spec = Spec {
        id: "grid".into(),
        title: String::new(),
        x: 0,
        y: 0,
        width: 480,
        height: 480,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    spec.appearance.background_color = Some(0x152c42);
    spec.appearance.font_color = Some(0xffffff);
    let p = painting::palette(&spec);
    assert_eq!(p.surface, 0x152c42);
    assert_eq!(p.foreground, 0xffffff);
    assert_ne!(p.surface_muted, 0xffffff);
    let mut v = View {
        module: Some(mochi_core::desktop_cards::Module::Automations),
        rows: (0..12)
            .map(|i| Row {
                id: i.to_string(),
                title: format!("流程{i}"),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    v.presentation.grid = true;
    v.presentation.columns = 3;
    v.presentation.grid_height = 120;
    v.presentation.grid_lines = true;
    let a = Rect::from_size(0.0, 0.0, 480.0, 480.0);
    let controls = painting::controls(&spec, &v, a, 0);
    let rows: Vec<_> = controls
        .iter()
        .filter(|(_, h)| matches!(h, Hit::Row(..)))
        .collect();
    assert_eq!(rows[0].0.height(), 120.0);
    assert!(painting::scroll_max(&spec, &v, a) > 0);
}
#[test]
#[ignore = "native widget visual verification"]
fn render_desktop_widgets_review() {
    use mochi_core::desktop_cards::{
        studio::{Kind, Node},
        Module,
    };
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/desktop-verification/ui-refactor");
    std::fs::create_dir_all(&output).unwrap();
    for (module, name) in [
        (Module::Pomodoro, "widget-pomodoro"),
        (Module::Ai, "widget-ai"),
        (Module::Clock, "widget-clock"),
        (Module::Clock, "widget-analog"),
        (Module::Ai, "widget-ai-rich"),
        (Module::Ai, "widget-ai-loading"),
        (Module::Shortcuts, "widget-shortcuts"),
    ] {
        let spec = Spec {
            id: name.into(),
            title: name.into(),
            x: 0,
            y: 0,
            width: 480,
            height: 480,
            locked: false,
            dark: false,
            appearance: mochi_core::desktop_cards::Appearance {
                background_color: Some(0xe4ecf1),
                ..Default::default()
            },
        };
        let mut v=View{module:Some(module),page_id:"page".into(),tabs:vec![("page".into(),module.label().into())],live:widgets::Live{timer:"24:38".into(),running:true,title:"今天的工作计划".into(),messages:vec![("user".into(),"帮我整理今天的三件事。".into()),("assistant".into(),"可以先梳理最重要的任务，然后留出一段专注时间，最后给整理和回顾留一点空间。".into())],..Default::default()},..Default::default()};
        if module == Module::Clock {
            v.studio.nodes.push(Node::new(Kind::Clock));
            let mut n = Node::new(Kind::Date);
            n.y = 3500;
            n.width = 9000;
            v.studio.nodes.push(n);
        }
        if name == "widget-analog" {
            v.studio.nodes.clear();
            let mut n = Node::new(Kind::AnalogClock);
            n.x = 500;
            n.y = 500;
            n.width = 9000;
            n.height = 9000;
            v.studio.nodes.push(n);
        }
        if name == "widget-ai-rich" {
            v.live.messages=vec![("assistant".into(),"## 今天的安排\n\n**先做重要的事**，留一点时间整理。\n\n| 时间 | 日程 |\n|---|---|\n| 09:00 | 项目评审 |\n| 14:00 | 编写文档 |\n\n- [x] 整理收件箱\n- [ ] 完成设计".into())];
        }
        if name == "widget-ai-loading" {
            v.live.messages = vec![("user".into(), "帮我整理今天的日程".into())];
            v.live.streaming = true;
        }
        if module == Module::Shortcuts {
            v.studio.add_shortcuts(&[
                "C:/Windows/notepad.exe".into(),
                "C:/Windows/explorer.exe".into(),
                "C:/Windows/System32/calc.exe".into(),
            ]);
            v.studio.arrange_shortcuts();
        }
        let mut list = DrawList::new();
        painting::paint(
            &mut list,
            &spec,
            &v,
            Rect::from_size(0.0, 0.0, 480.0, 480.0),
            0,
            None,
            None,
        );
        list.finish().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let snapshot = renderer.prepare_snapshot(720, 720, 144.0).unwrap();
        renderer.present(HWND::default(), 0xffffff, &list).unwrap();
        renderer
            .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn desktop_live_clock_repaints_on_timer_and_analog_hands_advance() {
    use mochi_core::desktop_cards::{
        studio::{Kind, Node},
        Module,
    };
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut manager = Manager::default();
    let spec = Spec {
        id: "live-clock".into(),
        title: String::new(),
        x: 30,
        y: 40,
        width: 480,
        height: 480,
        locked: false,
        dark: false,
        // 不可见的非分层（玻璃）窗口没有可用于断言的更新区域。
        appearance: mochi_core::desktop_cards::Appearance {
            background_color: None,
            ..Default::default()
        },
    };
    let mut view = View {
        module: Some(Module::Clock),
        ..Default::default()
    };
    view.studio.nodes.push(Node::new(Kind::AnalogClock));
    manager.sync(HWND::default(), vec![(spec.clone(), view)]);
    let hwnd = manager.windows[&spec.id];
    unsafe {
        assert_eq!(state(hwnd).unwrap().live_timer_period, 1000);
        SendMessageW(hwnd, WM_TIMER, Some(WPARAM(LIVE_TIMER)), Some(LPARAM(0)));
        assert!(state(hwnd).unwrap().view.live.now > 0);
        assert!(GetUpdateRect(hwnd, None, false).as_bool());
        assert!(!IsWindowVisible(hwnd).as_bool());
    }
    let n = Node::new(Kind::AnalogClock);
    let r = Rect::from_size(0.0, 0.0, 240.0, 240.0);
    let mut first = DrawList::new();
    let mut next = DrawList::new();
    widgets::paint_node(
        &mut first,
        &n,
        r,
        0x202020,
        0xdddddd,
        &widgets::Live {
            now: 1789977600,
            ..Default::default()
        },
        std::path::Path::new(""),
        false,
    );
    widgets::paint_node(
        &mut next,
        &n,
        r,
        0x202020,
        0xdddddd,
        &widgets::Live {
            now: 1789977615,
            ..Default::default()
        },
        std::path::Path::new(""),
        false,
    );
    assert_ne!(first.cmds(), next.cmds());
}
#[test]
fn desktop_ai_markdown_loading_and_native_ime_enter_are_integrated() {
    use mochi_core::desktop_cards::Module;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let spec = Spec {
        id: "rich-ai".into(),
        title: String::new(),
        x: 30,
        y: 40,
        width: 480,
        height: 480,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    let mut v = View {
        module: Some(Module::Ai),
        live: widgets::Live {
            streaming: true,
            messages: vec![(
                "assistant".into(),
                "## 安排\n\n**今天**先处理任务。\n\n| 项目 | 状态 |\n|---|---|\n| 测试 | 完成 |"
                    .into(),
            )],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut list = DrawList::new();
    painting::paint(
        &mut list,
        &spec,
        &v,
        Rect::from_size(0.0, 0.0, 480.0, 480.0),
        0,
        None,
        None,
    );
    list.finish().unwrap();
    let texts: Vec<_> = list
        .cmds()
        .iter()
        .filter_map(|c| match c {
            crate::ui::draw::DrawCmd::Text { text, .. }
            | crate::ui::draw::DrawCmd::ScaledText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts.iter().any(|t| t.contains("正在思考")));
    assert!(texts.iter().any(|t| t.contains("安排")));
    assert!(!texts
        .iter()
        .any(|t| t.contains("##") || t.contains("**") || t.contains("|---")));
    v.live.streaming = false;
    let mut m = Manager::default();
    m.sync(HWND::default(), vec![(spec.clone(), v)]);
    let hwnd = m.windows[&spec.id];
    let edit = unsafe { state(hwnd).unwrap().composer.unwrap() };
    unsafe {
        assert_eq!(GetWindowLongPtrW(edit, GWL_STYLE) as u32 & WS_VSCROLL.0, 0);
        SetWindowTextW(edit, w!("中文输入测试")).unwrap();
        SendMessageW(
            edit,
            WM_IME_STARTCOMPOSITION,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        SendMessageW(edit, WM_KEYDOWN, Some(WPARAM(13)), Some(LPARAM(0)));
    }
    assert!(!m
        .events()
        .iter()
        .any(|e| matches!(e.kind, EventKind::AiSend(_))));
    unsafe {
        SendMessageW(
            edit,
            WM_IME_ENDCOMPOSITION,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
        SendMessageW(edit, WM_KEYDOWN, Some(WPARAM(13)), Some(LPARAM(0)));
    }
    assert!(m
        .events()
        .iter()
        .any(|e| matches!(&e.kind,EventKind::AiSend(v) if v.contains("中文输入测试"))));
}
#[test]
fn desktop_page_split_and_schedule_checks_share_visible_hit_geometry() {
    use mochi_core::desktop_cards::Module;
    let mut spec = Spec {
        id: "split".into(),
        title: String::new(),
        x: 0,
        y: 0,
        width: 600,
        height: 480,
        locked: false,
        dark: false,
        appearance: Default::default(),
    };
    spec.appearance.tabs_left = true;
    spec.appearance.tabs_ratio = 30;
    spec.appearance.tabs_divider = true;
    let mut v = View {
        module: Some(Module::Schedule),
        tabs: vec![
            ("one".into(), "日程".into()),
            ("two".into(), "工作台".into()),
        ],
        page_id: "one".into(),
        rows: vec![Row {
            id: "task".into(),
            title: "任务".into(),
            checked: Some(false),
            ..Default::default()
        }],
        ..Default::default()
    };
    let r = Rect::from_size(0.0, 0.0, 600.0, 480.0);
    assert_eq!(painting::content_rect(&spec, &v, r).left, 188.0);
    assert!(painting::controls(&spec, &v, r, 0)
        .iter()
        .any(|(_, h)| matches!(h, Hit::Row(_, true))));
    v.presentation.show_checks = false;
    assert!(!painting::controls(&spec, &v, r, 0)
        .iter()
        .any(|(_, h)| matches!(h, Hit::Row(_, true))));
    spec.appearance.tabs_left = false;
    assert_eq!(painting::content_rect(&spec, &v, r).top, 152.0);
}
