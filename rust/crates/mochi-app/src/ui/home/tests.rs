use super::*;
use crate::ui::draw::DrawCmd;
use chrono::TimeZone;

const AREA: Rect = Rect {
    left: 480.0,
    top: 68.0,
    right: 1200.0,
    bottom: 776.0,
};

fn empty() -> HomeAnalytics {
    let now = chrono::Local
        .with_ymd_and_hms(2026, 8, 30, 12, 0, 0)
        .unwrap();
    mochi_core::analytics::empty_snapshot(365, now)
}

fn texts(list: &DrawList) -> Vec<String> {
    list.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn painted(a: &HomeAnalytics) -> DrawList {
    let page = layout(a, AREA);
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, AREA, &page, 0.0, &p);
    assert!(list.finish().is_ok(), "裁剪栈没配平");
    list
}

fn dashboard_with_entries() -> Dashboard {
    Dashboard {
        recent_documents: vec![DashboardDocument {
            path: PathBuf::from("C:/workspace/recent.md"),
            title: "最近的一篇笔记".into(),
            subtitle: "知识库".into(),
            mtime_ms: 1,
            favorite: false,
        }],
        favorite_documents: vec![DashboardDocument {
            path: PathBuf::from("C:/workspace/favorite.md"),
            title: "收藏的一篇笔记".into(),
            subtitle: "项目".into(),
            mtime_ms: 1,
            favorite: true,
        }],
        inbox_items: vec![DashboardInboxItem {
            id: "capture-1".into(),
            content: "记下一个待整理的想法".into(),
            created_at_ms: 1,
        }],
        libraries: vec![DashboardLibrary {
            id: "library-1".into(),
            name: "知识库".into(),
            type_name: "本地".into(),
        }],
        ai_sessions: vec![DashboardSession {
            id: "session-1".into(),
            title: "整理会议记录".into(),
            updated_at_ms: 1,
            message_count: 3,
            pinned: true,
        }],
        schedule_items: vec![DashboardScheduleItem {
            id: "task-1".into(),
            title: "完成首页验收".into(),
            time_label: "14:00".into(),
            status: "todo".into(),
            kind: "task".into(),
            priority: "high".into(),
        }],
        schedule_conflict_count: 1,
        schedule_risk: "medium".into(),
        journal: DashboardJournal {
            path: PathBuf::from("C:/workspace/日记/2026-08-30.md"),
            date: "2026-08-30".into(),
            exists: true,
            words: 42,
            excerpt: "今天完成了首页的第一轮整理。".into(),
        },
        graph: DashboardGraph {
            local_link_count: 2,
            nodes: vec![DashboardGraphNode {
                path: PathBuf::from("C:/workspace/graph.md"),
                title: "首页设计".into(),
                link_count: 2,
            }],
        },
    }
}

fn dashboard_painted(
    analytics: Option<&HomeAnalytics>,
    dashboard: &Dashboard,
    area: Rect,
) -> DrawList {
    let page = layout_with_dashboard(analytics, dashboard, area);
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, area, &page, 0.0, &p);
    assert!(list.finish().is_ok(), "首页裁剪栈没配平");
    list
}

#[test]
fn dashboard_keeps_all_destinations_and_preserves_hit_geometry() {
    let analytics = empty();
    let dashboard = dashboard_with_entries();
    let area = Rect::new(0.0, 0.0, 1200.0, 720.0);
    let page = layout_with_dashboard(Some(&analytics), &dashboard, area);
    let titles = section_titles(&page);
    let expected = [
        "收件箱",
        "最近笔记",
        "收藏文档",
        "知识库",
        "写作产出",
        "笔记增长",
        "活跃时段",
        "AI 对话",
        "AI 用量",
        "年度活跃图",
        "今日日程",
        "AI 助手",
        "个人画像",
        "今日日记",
        "知识图谱预览",
        "数据统计",
    ];
    for title in expected {
        assert!(titles.contains(&title.to_owned()), "首页少了「{title}」");
    }
    let right_titles: Vec<&Block> = page
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::SectionTitle { text, .. } if ["今日日程", "AI 助手", "个人画像", "今日日记", "知识图谱预览", "数据统计"].contains(&text.as_str())))
        .collect();
    assert!(!right_titles.is_empty());
    assert!(right_titles
        .iter()
        .all(|block| block_rect(block).left >= 860.0));

    let action_rect = page
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::ActionTile {
                action: Action::NewNote,
                rect,
                ..
            } => Some(*rect),
            _ => None,
        })
        .expect("快速开始没有新建笔记入口");
    assert_eq!(
        page.hit(
            (action_rect.left + action_rect.right) / 2.0,
            (action_rect.top + action_rect.bottom) / 2.0,
            0.0
        ),
        Some(Action::NewNote)
    );
    let recent_rect = page
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::DocumentRow {
                action: Action::OpenRecent(0),
                rect,
                ..
            } => Some(*rect),
            _ => None,
        })
        .expect("最近笔记没有入口");
    let scroll = 80.0;
    assert_eq!(
        page.hit(
            (recent_rect.left + recent_rect.right) / 2.0,
            (recent_rect.top + recent_rect.bottom) / 2.0 - scroll,
            scroll,
        ),
        Some(Action::OpenRecent(0))
    );
    assert!(page.height > area.height());
    let _ = dashboard_painted(Some(&analytics), &dashboard, area);
}

#[test]
fn primary_destinations_stay_on_first_screen_and_do_not_shift_after_loading() {
    let analytics = empty();
    let mut dashboard = dashboard_with_entries();
    dashboard.recent_documents = vec![dashboard.recent_documents[0].clone(); 5];
    for width in [940.0, 1200.0, 1800.0] {
        let area = Rect::from_size(12.0, 48.0, width, 740.0);
        let pending = layout_with_dashboard(None, &dashboard, area);
        let ready = layout_with_dashboard(Some(&analytics), &dashboard, area);
        for action in [
            Action::NewNote,
            Action::OpenRecent(0),
            Action::OpenRecent(4),
            Action::OpenSchedule,
        ] {
            let rect = |page: &Layout| {
                page.blocks
                    .iter()
                    .find(|b| block_action(b) == Some(action.clone()))
                    .map(block_rect)
                    .unwrap()
            };
            assert_eq!(rect(&pending), rect(&ready), "loading moved {action:?}");
            assert!(
                rect(&ready).bottom <= area.bottom,
                "{action:?} below first screen at {width}"
            );
        }
    }
}

#[test]
fn responsive_dashboard_has_nonoverlapping_targets_in_both_themes() {
    let analytics = empty();
    let mut dashboard = dashboard_with_entries();
    dashboard.recent_documents[0].title = "很长的中英文笔记标题 A long title 😀".repeat(12);
    dashboard.recent_documents[0].subtitle = "知识库/很长的目录".repeat(12);
    dashboard.schedule_items[0].title = "一个需要完整阅读的任务名称".repeat(8);
    dashboard.libraries[0].type_name = "知识库类型".repeat(8);
    for width in [320.0, 660.0, 939.0, 940.0, 1200.0, 1800.0] {
        let area = Rect::from_size(12.0, 48.0, width, 700.0);
        let page = layout_with_dashboard(Some(&analytics), &dashboard, area);
        for block in &page.blocks {
            let r = block_rect(block);
            assert!(
                r.left >= area.left
                    && r.right <= area.right
                    && r.width() >= 0.0
                    && r.height() > 0.0,
                "{width}: {r:?}"
            );
        }
        let targets: Vec<_> = page
            .blocks
            .iter()
            .filter(|b| block_action(b).is_some())
            .map(block_rect)
            .collect();
        for (i, a) in targets.iter().enumerate() {
            for b in &targets[i + 1..] {
                assert!(
                    a.right <= b.left
                        || b.right <= a.left
                        || a.bottom <= b.top
                        || b.bottom <= a.top,
                    "overlapping controls at {width}: {a:?}, {b:?}"
                );
            }
        }
        for dark in [false, true] {
            let mut list = DrawList::new();
            paint(
                &mut list,
                Rect::from_size(area.left, area.top, area.width(), page.height),
                &page,
                0.0,
                theme::tokens().palette(dark),
            );
            assert!(list.finish().is_ok());
            for cmd in list.cmds() {
                if let DrawCmd::Text {
                    rect,
                    text: value,
                    style,
                    ..
                } = cmd
                {
                    assert!(
                        rect.width() >= 0.0,
                        "negative text rect at {width}: {value} {rect:?}"
                    );
                    if value == &relative_time(dashboard.recent_documents[0].mtime_ms, now_ms()) {
                        assert!(
                            rect.width() >= text::measure(value, *style),
                            "document time has no space"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn hover_is_local_to_one_row_and_keyboard_order_is_visual() {
    let area = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
    let dashboard = dashboard_with_entries();
    let page = layout_with_dashboard(Some(&empty()), &dashboard, area);
    let mut index = None;
    for action in [
        Action::NewNote,
        Action::OpenAi,
        Action::NewJournal,
        Action::ImportFile,
    ] {
        index = page.next_action(index, false);
        assert_eq!(page.action(index.unwrap()), Some(action));
    }
    let focused = index.unwrap();
    let rect = page.action_rect(focused).unwrap();
    assert_eq!(
        page.hit_index(rect.left + 2.0, rect.top + 2.0, 0.0),
        Some(focused)
    );
    let p = theme::tokens().palette(false);
    let mut list = DrawList::new();
    paint_interactive(&mut list, area, &page, 0.0, p, Some(focused), Some(focused));
    assert!(list.finish().is_ok());
    assert_eq!(
        list.cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::RoundedBorder { color, .. } if *color == p.accent))
            .count(),
        1
    );
    assert_eq!(
        page.next_action(page.next_action(None, true), false),
        page.next_action(None, false)
    );
}

#[test]
fn dashboard_collapses_to_a_single_scrollable_column_without_overflow() {
    let analytics = empty();
    let dashboard = dashboard_with_entries();
    let area = Rect::new(12.0, 48.0, 672.0, 640.0);
    let page = layout_with_dashboard(Some(&analytics), &dashboard, area);
    assert!(page.height > area.height());
    assert!(page.blocks.iter().all(|block| {
        let rect = block_rect(block);
        rect.left >= area.left - 0.01 && rect.right <= area.right + 0.01
    }));
    let action_rect = page
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::ActionTile {
                action: Action::ImportFile,
                rect,
                ..
            } => Some(*rect),
            _ => None,
        })
        .expect("窄窗丢失导入入口");
    assert_eq!(
        page.hit(
            (action_rect.left + action_rect.right) / 2.0,
            (action_rect.top + action_rect.bottom) / 2.0,
            0.0
        ),
        Some(Action::ImportFile)
    );
    let _ = dashboard_painted(Some(&analytics), &dashboard, area);
}

#[test]
fn hourly_activity_and_real_today_narrative_keep_zero_data_honest() {
    let points = vec![
        HourlyActivityPoint {
            day: 1,
            hour: 9,
            count: 2,
            weight: 2,
        },
        HourlyActivityPoint {
            day: 1,
            hour: 10,
            count: 4,
            weight: 8,
        },
    ];
    let cells = hourly_cells(&points);
    assert_eq!(cells[0], (9, 1, 0.25));
    assert_eq!(cells[1], (10, 1, 1.0));
    let empty_day = mochi_core::analytics::DaySummary::empty("2026-08-30");
    assert_eq!(
        today_narrative(&empty_day, 0),
        "先写下一行，今天的节奏会从这里开始。"
    );
}

/// 排版结果里的所有板块标题。断言「有没有这一块」要问排版，不能问绘制——
/// 绘制只画视口内的那一段，滚在下面的板块本来就不该出现在指令里。
fn section_titles(page: &Layout) -> Vec<String> {
    page.blocks
        .iter()
        .filter_map(|b| match b {
            Block::SectionTitle { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_empty_workspace_still_lays_out_every_section() {
    // 空工作区不能画成一片白——用户会以为是坏了
    let page = layout(&empty(), AREA);
    let titles = section_titles(&page);
    for title in ["数据统计", "写作产出", "笔记增长", "年度活跃图", "个人画像"]
    {
        assert!(titles.contains(&title.to_owned()), "少了「{title}」这一块");
    }
    // 今日战报没有小标题，认那句大字
    assert!(page
        .blocks
        .iter()
        .any(|b| matches!(b, Block::Headline { text, .. } if text == "今天还没动笔")));
}

#[test]
fn the_first_screen_shows_the_top_sections() {
    let list = painted(&empty());
    let t = texts(&list);
    assert!(t.contains(&"今天还没动笔".to_owned()));
    assert!(t.contains(&"数据统计".to_owned()));
    // 靠下的板块滚出视口，不该在指令里
    assert!(!t.contains(&"个人画像".to_owned()));
}

#[test]
fn scrolling_down_brings_the_lower_sections_into_view() {
    let page = layout(&empty(), AREA);
    let p = *theme::tokens().palette(false);
    let mut list = DrawList::new();
    paint(&mut list, AREA, &page, max_scroll(&page, AREA), &p);
    let t = texts(&list);
    assert!(
        t.contains(&"个人画像".to_owned()),
        "滚到底应当看得见最后一块"
    );
}

#[test]
fn blocks_never_overlap_vertically() {
    // 面板高度是回填的，最容易错的就是某块忘了推进游标
    let page = layout(&empty(), AREA);
    let mut panels: Vec<Rect> = page
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Panel { rect, .. } => Some(*rect),
            _ => None,
        })
        .collect();
    assert!(panels.len() >= 5);
    panels.sort_by(|a, b| a.top.partial_cmp(&b.top).unwrap());
    for pair in panels.windows(2) {
        assert!(
            pair[1].top >= pair[0].bottom - 0.01,
            "面板重叠：{:?} 与 {:?}",
            pair[0],
            pair[1]
        );
        assert!(pair[0].height() > 0.0, "面板高度没回填：{:?}", pair[0]);
    }
}

#[test]
fn the_page_height_covers_the_last_block() {
    let page = layout(&empty(), AREA);
    let lowest = page
        .blocks
        .iter()
        .map(|b| block_rect(b).bottom)
        .fold(0.0f32, f32::max);
    assert!(page.height + AREA.top >= lowest, "页高没覆盖到最后一块");
}

#[test]
fn only_the_visible_slice_is_painted() {
    let page = layout(&empty(), AREA);
    let p = *theme::tokens().palette(false);

    let mut top = DrawList::new();
    paint(&mut top, AREA, &page, 0.0, &p);
    let mut far = DrawList::new();
    paint(&mut far, AREA, &page, 100_000.0, &p);

    assert!(!top.is_empty());
    // 滚到远处什么都看不见了
    assert!(far
        .cmds()
        .iter()
        .all(|c| matches!(c, DrawCmd::PushClip { .. } | DrawCmd::PopClip)));
}

#[test]
fn scrolling_is_bounded_by_the_page_height() {
    let page = layout(&empty(), AREA);
    assert!(max_scroll(&page, AREA) >= 0.0);
    // 视口比页面还高时不该能滚
    let tall = Rect::new(0.0, 0.0, 720.0, 100_000.0);
    assert_eq!(max_scroll(&page, tall), 0.0);
}

#[test]
fn compact_uses_wan_for_chinese_readers() {
    // 12.3w 比 123.0k 直观
    assert_eq!(compact(999), "999");
    assert_eq!(compact(1_500), "1.5k");
    assert_eq!(compact(123_000), "12.3w");
    assert_eq!(compact(-1_500), "-1.5k");
}

#[test]
fn bytes_are_formatted_with_a_sensible_unit() {
    assert_eq!(bytes(512), "512 B");
    assert_eq!(bytes(2048), "2.0 KB");
    assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
}

#[test]
fn duration_switches_to_hours_past_sixty_minutes() {
    assert_eq!(duration(45), "45 分钟");
    assert_eq!(duration(90), "1 小时 30 分");
}

#[test]
fn the_headline_reflects_what_actually_happened_today() {
    assert_eq!(headline(0, 0), "今天还没动笔");
    assert_eq!(headline(1200, 3), "今天写了 1.2k 字");
    assert_eq!(headline(0, 4), "今天整理了 4 篇笔记");
    // 净字数为负说明今天在删——别报成"写了 -300 字"
    assert_eq!(headline(-300, 0), "今天在做减法");
}

#[test]
fn weekday_matches_the_javascript_convention() {
    // 0 = 周日，与 JS 的 getDay() 一致。2026-08-30 是周日
    assert_eq!(weekday_of("2026-08-30"), Some(0));
    assert_eq!(weekday_of("2026-08-31"), Some(1));
    assert_eq!(weekday_of("2026-01-01"), Some(4)); // 周四
    assert_eq!(weekday_of("坏日期"), None);
}

#[test]
fn the_heatmap_is_a_week_by_weekday_grid_aligned_to_the_first_date() {
    // 首日是周日 → 第 0 列从第 0 行开始
    let points: Vec<DailyActivityPoint> = (0..14)
        .map(|i| DailyActivityPoint {
            date: format!("2026-08-{:02}", 30 + i)
                .replace("08-3", if i < 2 { "08-3" } else { "09-0" }),
            count: i as i64,
            weight: 0,
        })
        .collect();
    let cells = heat_cells(&points);
    assert_eq!(cells.len(), 14);
    assert_eq!(cells[0], (0, 0, 0.0));
    // 第 8 天进入第二列
    assert_eq!(cells[7].0, 1);
    assert_eq!(cells[7].1, 0);
}

#[test]
fn an_empty_heatmap_produces_no_cells() {
    assert!(heat_cells(&[]).is_empty());
}

#[test]
fn days_with_no_activity_still_get_a_cell() {
    // 全零的热力图不该是一片空白——「哪些天没写」同样是信息
    let points: Vec<DailyActivityPoint> = (1..=7)
        .map(|i| DailyActivityPoint {
            date: format!("2026-08-{i:02}"),
            count: 0,
            weight: 0,
        })
        .collect();
    let cells = heat_cells(&points);
    assert_eq!(cells.len(), 7);
    assert!(cells.iter().all(|(_, _, v)| *v == 0.0));

    let page = layout(&empty(), AREA);
    let _ = page;
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    let rect = Rect::new(0.0, 0.0, 400.0, 200.0);
    paint(
        &mut list,
        rect,
        &Layout {
            blocks: vec![Block::Heatmap {
                rect: Rect::new(0.0, 0.0, 400.0, 100.0),
                cells,
            }],
            height: 100.0,
        },
        0.0,
        &p,
    );
    // 七个零活跃格全都画了底
    assert_eq!(
        list.cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == p.surface_muted))
            .count(),
        7
    );
}

#[test]
fn a_tiny_but_nonzero_bar_is_still_visible() {
    // 比例极小的柱子四舍五入成 0 高就等于"那天没写"，是错的
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    let rect = Rect::new(0.0, 0.0, 400.0, 200.0);
    paint(
        &mut list,
        rect,
        &Layout {
            blocks: vec![Block::Bars {
                rect: Rect::new(0.0, 0.0, 300.0, 96.0),
                bars: vec![("a".into(), 0.0001, false), ("b".into(), 0.0, false)],
            }],
            height: 96.0,
        },
        0.0,
        &p,
    );
    let bars: Vec<&Rect> = list
        .cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Rect { rect, .. } => Some(rect),
            _ => None,
        })
        .collect();
    // 只有非零那根被画出来，且至少 2px 高
    assert_eq!(bars.len(), 1);
    assert!(bars[0].height() >= 2.0);
}

#[test]
fn a_zero_sized_area_is_skipped_entirely() {
    let page = layout(&empty(), AREA);
    let mut list = DrawList::new();
    let p = *theme::tokens().palette(false);
    paint(&mut list, Rect::new(0.0, 0.0, 0.0, 0.0), &page, 0.0, &p);
    assert!(list.is_empty());
    assert!(list.finish().is_ok());
}
