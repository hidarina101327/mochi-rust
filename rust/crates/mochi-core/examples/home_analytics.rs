//! 只读分析；不要调用 WorkspaceService::open，以免触发迁移。

use std::path::PathBuf;
use std::time::Instant;

use mochi_core::analytics;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next().map(PathBuf::from) else {
        eprintln!("用法: home_analytics <工作区路径> [天数]");
        std::process::exit(2);
    };
    if !root.is_dir() {
        eprintln!("工作区不存在: {}", root.display());
        std::process::exit(2);
    }
    let range_days = args.next().and_then(|v| v.parse::<f64>().ok());

    let started = Instant::now();
    let s = analytics::build(&root, range_days, chrono::Local::now());
    let elapsed = started.elapsed();

    // 设置 MOCHI_ANALYTICS_JSON=1 后只输出 JSON，方便与 TS 版本逐项比较。
    if std::env::var("MOCHI_ANALYTICS_JSON").is_ok() {
        println!("{}", serde_json::to_string_pretty(&s).unwrap());
        return;
    }

    println!("工作区: {}", root.display());
    println!("统计窗口: {} 天，耗时 {:?}\n", s.range_days, elapsed);

    println!("文件");
    println!(
        "  笔记 {} / 全部 {} 个，共 {:.1} MB，{} 字",
        s.files.total_notes,
        s.files.total_files,
        s.files.total_size as f64 / 1_048_576.0,
        s.files.total_words
    );
    print!("  类型:");
    for stat in s.files.file_types.iter().take(6) {
        print!(" {}×{}", stat.extension, stat.count);
    }
    println!("\n");

    println!("双链图");
    println!(
        "  本地链接 {} 条，孤立笔记 {} 篇，未打标签 {:.0}%",
        s.graph.local_link_count,
        s.graph.isolated_note_count,
        s.graph.untagged_note_ratio * 100.0
    );
    for node in s.graph.central_nodes.iter().take(3) {
        println!(
            "  枢纽: {} （{} 条链接，分数 {}）",
            node.title, node.link_count, node.score
        );
    }
    println!();

    println!("写作");
    println!("  窗口内新增 {} 字", s.writing.total_words_added);
    println!(
        "  今天 +{} / -{} 字，{} 次保存，{} 篇笔记，在线 {} 分钟",
        s.today.day.words_added,
        s.today.day.words_removed,
        s.today.day.save_count,
        s.today.day.notes_edited,
        s.today.day.active_minutes
    );
    println!(
        "  昨天 +{} 字；近 {} 天均值 +{} 字",
        s.today.previous.words_added, s.today.baseline.days, s.today.baseline.words_added
    );
    for note in s.today.top_notes.iter().take(3) {
        println!(
            "  今日重点: {} （净 {:+} 字，{} 次保存）",
            note.title, note.net_words, note.save_count
        );
    }
    println!();

    println!("连续");
    println!(
        "  当前 {} 天，最长 {} 天，窗口内活跃 {} 天",
        s.streak.current, s.streak.longest, s.streak.active_days_in_range
    );
    println!(
        "  本周 {} 字 / 上周 {} 字",
        s.streak.this_week_words, s.streak.last_week_words
    );
    println!();

    println!("AI");
    println!(
        "  会话 {} 个，消息 {} 条，token {}（其中估算 {}）",
        s.ai.total_sessions, s.ai.total_messages, s.ai.total_tokens, s.ai.estimated_tokens
    );
    println!();

    println!("画像");
    let hhmm = |h: Option<usize>| h.map_or("—".into(), |h| format!("{h}:00"));
    let weekday = |d: Option<usize>| {
        d.map_or("—".to_string(), |d| {
            ["日", "一", "二", "三", "四", "五", "六"][d].to_string()
        })
    };
    println!(
        "  作息 {:?}，峰值 {}（占 {:.0}%），最忙星期{}",
        s.persona.chronotype,
        hhmm(s.persona.peak_hour),
        s.persona.peak_hour_share * 100.0,
        weekday(s.persona.busiest_weekday)
    );
    let t = &s.persona.time_of_day_shares;
    println!(
        "  凌晨 {:.0}% / 上午 {:.0}% / 下午 {:.0}% / 夜间 {:.0}%",
        t.dawn * 100.0,
        t.morning * 100.0,
        t.afternoon * 100.0,
        t.night * 100.0
    );
    println!(
        "  活跃日均在线 {} 分钟、写 {} 字，AI 协作占比 {:.0}%",
        s.persona.average_active_minutes,
        s.persona.average_words_per_active_day,
        s.persona.ai_collaboration_ratio * 100.0
    );
    for area in s.persona.top_areas.iter().take(4) {
        println!(
            "  领域: {} （{} 次活动，净 {:+} 字，{} 篇）",
            area.name, area.event_count, area.net_words, area.note_count
        );
    }

    // 各段长度自检：图表按下标取值，长度对不上会画错格子
    println!("\n形状自检");
    println!(
        "  贡献格 {} = 窗口天数 {}",
        s.activity.contributions.len(),
        s.range_days
    );
    println!("  作息网格 {} = 7×24", s.activity.hourly.len());
    println!(
        "  笔记趋势 日{}/周{}/月{}",
        s.note_trends.day.len(),
        s.note_trends.week.len(),
        s.note_trends.month.len()
    );
    assert_eq!(s.activity.contributions.len() as i64, s.range_days);
    assert_eq!(s.activity.hourly.len(), 168);
    assert_eq!(s.note_trends.day.len(), s.writing.day.len());
    assert_eq!(s.ai.day.len(), s.ai.tokens.day.len());
    println!("  ✓ 全部对齐");
}
