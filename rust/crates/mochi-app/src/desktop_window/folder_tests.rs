use super::*;
use mochi_core::desktop_cards::{Module, RowMeta};

#[test]
fn folder_selection_supports_ranges_and_individual_toggles() {
    let (_, mut view, _) = fixture(480, false, true);
    folder::select(&mut view, None, "entry-2", false, false);
    folder::select(&mut view, Some("entry-2"), "entry-5", false, true);
    assert_eq!(view.folder_selected.len(), 4);
    folder::select(&mut view, None, "entry-3", true, false);
    assert_eq!(view.folder_selected.len(), 3);
    assert!(!view.folder_selected.contains("entry-3"));
    folder::select(&mut view, None, "entry-8", false, false);
    assert_eq!(
        view.folder_selected.iter().cloned().collect::<Vec<_>>(),
        vec!["entry-8"]
    );
}

#[test]
fn folder_stacks_collapse_without_losing_rows_and_capsules_hide_file_hits() {
    let (mut spec, mut view, area) = fixture(480, false, true);
    for row in &mut view.rows {
        row.meta.group = "文档".into();
    }
    assert!(painting::controls(&spec, &view, area, 0)
        .iter()
        .any(|(_, h)| matches!(h, Hit::FolderGroup(..))));
    view.collapsed.insert("folder-group:文档".into());
    assert_eq!(view.rows.len(), 18);
    assert!(!painting::controls(&spec, &view, area, 0)
        .iter()
        .any(|(_, h)| matches!(h, Hit::Row(..))));
    view.collapsed.clear();
    assert!(painting::controls(&spec, &view, area, 0)
        .iter()
        .any(|(_, h)| matches!(h, Hit::Row(..))));
    spec.appearance.capsule = true;
    let hits = painting::controls(&spec, &view, Rect::from_size(0.0, 0.0, 480.0, 44.0), 0);
    assert!(hits.iter().any(|(_, h)| *h == Hit::Capsule));
    assert!(!hits
        .iter()
        .any(|(_, h)| matches!(h, Hit::Row(..) | Hit::Page(..) | Hit::FolderCommand(..))));
}

fn fixture(width: u32, dark: bool, grid: bool) -> (Spec, View, Rect) {
    let spec = Spec {
        appearance: mochi_core::desktop_cards::Appearance {
            background_color: None,
            ..Default::default()
        },
        id: "folder-review".into(),
        title: "项目资料".into(),
        x: 0,
        y: 0,
        width,
        height: 480,
        locked: false,
        dark,
    };
    let mut view = View {
        module: Some(Module::Folder),
        page_id: "folder-page".into(),
        tabs: vec![("folder-page".into(), "文件夹映射".into())],
        subtitle: "已显示 18 项 · D:\\资料\\项目文档".into(),
        ..Default::default()
    };
    view.presentation.grid = grid;
    view.presentation.columns = 3;
    view.rows = (0..18)
        .map(|index| Row {
            id: format!("entry-{index}"),
            title: if index == 0 {
                "参考资料".into()
            } else {
                format!("项目记录 {index}.md")
            },
            detail: "修改于 2026-09-26".into(),
            meta: RowMeta {
                directory: index == 0,
                icon: if index == 0 {
                    "folder".into()
                } else {
                    "file".into()
                },
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    (spec, view, Rect::from_size(0.0, 0.0, width as f32, 480.0))
}

#[test]
fn folder_directory_still_opens_when_imported_presentation_expands_libraries() {
    let (spec, mut view, area) = fixture(480, false, false);
    view.presentation.expand_libraries = true;
    let controls = painting::controls(&spec, &view, area, 0);
    assert!(controls
        .iter()
        .any(|(_, hit)| *hit == Hit::Row("entry-0".into(), false)));
    assert!(!controls
        .iter()
        .any(|(_, hit)| matches!(hit, Hit::Folder(_))));
}

#[test]
fn folder_shift_selection_keeps_its_anchor_across_repeated_and_ctrl_ranges() {
    let (_, mut view, _) = fixture(480, false, true);
    folder::select(&mut view, None, "entry-2", false, false);
    folder::select(&mut view, Some("entry-2"), "entry-5", false, true);
    folder::select(&mut view, Some("entry-2"), "entry-8", false, true);
    assert_eq!(view.folder_selected.len(), 7);
    assert!(view.folder_selected.contains("entry-2"));
    assert!(view.folder_selected.contains("entry-8"));

    folder::select(&mut view, Some("entry-2"), "entry-10", true, true);
    assert_eq!(view.folder_selected.len(), 9);
    assert!(view.folder_selected.contains("entry-10"));
    folder::select(&mut view, None, "entry-5", true, false);
    assert!(!view.folder_selected.contains("entry-5"));
    assert!(view.folder_selected.contains("entry-4"));
}

#[test]
fn folder_group_heading_is_a_collapse_hit_separate_from_file_rows() {
    let (spec, mut view, area) = fixture(480, false, true);
    for row in &mut view.rows {
        row.meta.group = "文档".into();
    }
    let hits = painting::controls(&spec, &view, area, 0);
    let heading = hits
        .iter()
        .find(|(_, hit)| matches!(hit, Hit::FolderGroup(_)))
        .unwrap();
    assert_eq!(heading.1, Hit::FolderGroup("folder-group:文档".into()));
    assert!(hits
        .iter()
        .filter(|(_, hit)| matches!(hit, Hit::Row(..)))
        .all(|(rect, _)| rect.intersect(&heading.0).is_empty()));
}

#[test]
fn folder_status_header_does_not_activate_rows_and_scrolling_reaches_last_entry() {
    for grid in [true, false] {
        for width in [280, 480] {
            let (spec, view, area) = fixture(width, false, grid);
            let header = painting::content_rect(&spec, &view, area);
            let controls = painting::controls(&spec, &view, area, 0);
            for (rect, hit) in controls
                .iter()
                .filter(|(_, hit)| matches!(hit, Hit::Row(..)))
            {
                assert!(
                    rect.top >= header.top + 32.0,
                    "{hit:?} overlaps folder status"
                );
                assert!(rect.left >= header.left && rect.right <= header.right);
            }
            let max = painting::scroll_max(&spec, &view, area);
            assert!(max > 0);
            let controls = painting::controls(&spec, &view, area, max);
            assert!(controls
                .iter()
                .any(|(_, hit)| *hit == Hit::Row("entry-17".into(), false)));
        }
    }
}

#[test]
#[cfg(debug_assertions)]
#[ignore = "native folder card visual verification"]
fn render_folder_cards_review() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/desktop-verification/deskbox");
    std::fs::create_dir_all(&output).unwrap();
    for (name, width, dark, grid) in [
        ("folder-grid", 480, false, true),
        ("folder-narrow-dark", 280, true, true),
        ("folder-list", 480, false, false),
    ] {
        let (spec, view, area) = fixture(width, dark, grid);
        let mut list = DrawList::new();
        painting::paint(&mut list, &spec, &view, area, 0, None, None);
        list.finish().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let snapshot = renderer
            .prepare_snapshot(width * 3 / 2, 720, 144.0)
            .unwrap();
        renderer.present(HWND::default(), 0xffffff, &list).unwrap();
        renderer
            .save_snapshot(&snapshot, &output.join(format!("{name}.png")))
            .unwrap();
    }
    for scenario in [
        "folder",
        "folder-narrow",
        "folder-preferences-dark",
        "weather",
        "search-narrow",
        "music-dark",
    ] {
        crate::app::App::desktop_snapshot(
            scenario,
            &output.join(format!("manager-{scenario}.png")),
        )
        .unwrap();
    }
    for scenario in [
        "folder-stacks",
        "folder-capsule",
        "weather",
        "music",
        "search",
    ] {
        let (mut spec, mut view, mut area) = fixture(480, scenario == "music", true);
        if scenario == "folder-stacks" {
            for (index, row) in view.rows.iter_mut().enumerate() {
                row.meta.group = if index < 6 {
                    "进行中的项目"
                } else {
                    "归档资料"
                }
                .into();
            }
            view.collapsed.insert("folder-group:归档资料".into());
            view.folder_selected
                .extend(["entry-1".into(), "entry-2".into()]);
        } else if scenario == "folder-capsule" {
            spec.appearance.capsule = true;
            area = Rect::from_size(0.0, 0.0, 480.0, 44.0);
        } else {
            let (module, title, subtitle, entries) = match scenario {
                "weather" => (
                    Module::Weather,
                    "深圳天气",
                    "Open-Meteo · 更新于 10:30",
                    vec![
                        ("当前 · 多云", "28.4°C · 湿度 76% · 风速 8.2 km/h"),
                        ("11:00 · 多云", "29.1°C · 降水概率 20%"),
                        ("今天 · 阵雨", "25–30°C · 降水概率 60%"),
                        ("明天 · 晴", "24–31°C · 降水概率 10%"),
                    ],
                ),
                "music" => (
                    Module::Music,
                    "正在播放",
                    "系统播放器 · 64 / 245 秒",
                    vec![
                        ("海边散步", "墨池音乐 · 午后歌单"),
                        ("上一首", ""),
                        ("暂停", ""),
                        ("下一首", ""),
                        ("播放来源：跟随系统", ""),
                    ],
                ),
                _ => (
                    Module::Search,
                    "桌面搜索",
                    "3 项 · 项目",
                    vec![
                        ("项目计划.md", "墨池 / 工作 / 项目计划.md"),
                        ("项目资料", "D:\\Documents\\项目资料"),
                        ("项目说明.pdf", "D:\\Documents\\项目说明.pdf"),
                    ],
                ),
            };
            spec.title = title.into();
            view.module = Some(module);
            view.tabs = vec![(view.page_id.clone(), module.label().into())];
            view.subtitle = subtitle.into();
            view.presentation.grid = false;
            view.rows = entries
                .into_iter()
                .enumerate()
                .map(|(i, (title, detail))| Row {
                    id: format!("fixture-{i}"),
                    title: title.into(),
                    detail: detail.into(),
                    meta: RowMeta {
                        always_detail: true,
                        icon: "file".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .collect();
        }
        let mut list = DrawList::new();
        painting::paint(&mut list, &spec, &view, area, 0, None, None);
        list.finish().unwrap();
        let mut renderer = Renderer::new().unwrap();
        let snapshot = renderer
            .prepare_snapshot(720, (area.height() * 1.5) as u32, 144.0)
            .unwrap();
        renderer.present(HWND::default(), 0xffffff, &list).unwrap();
        renderer
            .save_snapshot(&snapshot, &output.join(format!("{scenario}.png")))
            .unwrap();
    }
}

#[test]
fn utility_subtitles_reserve_matching_header_and_row_hit_geometry() {
    for module in [Module::Weather, Module::Music, Module::Search] {
        let (spec, mut view, area) = fixture(360, false, false);
        view.module = Some(module);
        view.subtitle = "实时来源 · 更新状态 · 3 项".into();
        let header = painting::content_rect(&spec, &view, area);
        let body = painting::scroll_body(&spec, &view, area);
        assert_eq!(body.top, header.top + 32.0);
        let title_bar = Rect::from_size(header.left, header.top, header.width(), 28.0);

        let row_hits = painting::controls(&spec, &view, area, 0)
            .into_iter()
            .filter(|(_, hit)| matches!(hit, Hit::Row(..)))
            .collect::<Vec<_>>();
        assert!(!row_hits.is_empty());
        assert!(row_hits.iter().all(|(rect, _)| rect.top >= body.top));
        assert!(row_hits
            .iter()
            .all(|(rect, _)| rect.intersect(&title_bar).is_empty()));
    }
}
