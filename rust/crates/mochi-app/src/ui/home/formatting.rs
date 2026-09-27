//! 格式化首页展示的日期、统计值和活动热度数据。
use super::*;

/// 把日活跃点铺成「列 = 周、行 = 星期几」的网格，与 GitHub 贡献图同构。
pub(super) fn heat_cells(points: &[DailyActivityPoint]) -> Vec<(usize, usize, f32)> {
    if points.is_empty() {
        return Vec::new();
    }
    let peak = points.iter().map(|p| p.count).max().unwrap_or(0).max(1) as f32;
    // 首个点落在星期几，决定第一列从第几行开始
    let first_weekday = points
        .first()
        .and_then(|p| weekday_of(&p.date))
        .unwrap_or(0);
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let offset = i + first_weekday;
            (offset / 7, offset % 7, (p.count as f32 / peak).min(1.0))
        })
        .collect()
}

/// `YYYY-MM-DD` → 星期几（0 = 周日，与 JS 的 `getDay()` 同口径）。
///
/// 用蔡勒公式而不是拉 chrono 进来解析：这里只需要一个星期几，
/// 而 `DailyActivityPoint.date` 的格式是上游固定死的。
pub(super) fn weekday_of(date: &str) -> Option<usize> {
    let mut parts = date.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: i32 = parts.next()?.parse().ok()?;
    let d: i32 = parts.next()?.parse().ok()?;
    let (y, m) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
    let k = y % 100;
    let j = y / 100;
    let h = (d + 13 * (m + 1) / 5 + k + k / 4 + j / 4 + 5 * j) % 7;
    // 蔡勒的 0 是周六，换算成 0 = 周日
    Some(((h + 6) % 7) as usize)
}

pub(super) fn headline(net_words: i64, notes: usize) -> String {
    match (net_words, notes) {
        (0, 0) => "今天还没动笔".to_owned(),
        (w, _) if w > 0 => format!("今天写了 {} 字", compact(w)),
        (_, n) if n > 0 => format!("今天整理了 {n} 篇笔记"),
        _ => "今天在做减法".to_owned(),
    }
}

pub(super) fn signed(v: i64) -> String {
    if v > 0 {
        format!("+{v}")
    } else {
        v.to_string()
    }
}

/// 大数字压缩成 `1.2k` / `3.4w`。
///
/// 中文语境用「w」（万）而不是「M」——`12.3w` 比 `123.0k` 直观得多。
pub(super) fn compact(v: i64) -> String {
    let abs = v.abs();
    if abs >= 10_000 {
        format!("{:.1}w", v as f64 / 10_000.0)
    } else if abs >= 1_000 {
        format!("{:.1}k", v as f64 / 1_000.0)
    } else {
        v.to_string()
    }
}

pub(super) fn bytes(v: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = v as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{v} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub(super) fn duration(minutes: i64) -> String {
    if minutes < 60 {
        format!("{minutes} 分钟")
    } else {
        format!("{} 小时 {} 分", minutes / 60, minutes % 60)
    }
}

pub(super) fn now_ms() -> i64 {
    chrono::Local::now().timestamp_millis()
}

pub(super) fn relative_time(timestamp_ms: i64, now: i64) -> String {
    if timestamp_ms <= 0 {
        return "未知时间".to_owned();
    }
    let elapsed = now.saturating_sub(timestamp_ms);
    if elapsed < 60_000 {
        "刚刚".to_owned()
    } else if elapsed < 3_600_000 {
        format!("{} 分钟前", elapsed / 60_000)
    } else if elapsed < 86_400_000 {
        format!("{} 小时前", elapsed / 3_600_000)
    } else if elapsed < 7 * 86_400_000 {
        format!("{} 天前", elapsed / 86_400_000)
    } else {
        // 不依赖本地时区解析，供给侧也可以传入将来的时间戳；这种情况下直接显示未知。
        let days = elapsed / 86_400_000;
        if days > 0 {
            format!("{days} 天前")
        } else {
            "未知时间".to_owned()
        }
    }
}

pub(super) fn chronotype_label(c: &mochi_core::analytics::Chronotype) -> &'static str {
    use mochi_core::analytics::Chronotype::*;
    match c {
        EarlyBird => "晨型人",
        DayWorker => "日间型",
        NightOwl => "夜猫子",
        Irregular => "作息不规律",
    }
}

pub(super) fn weekday_label(d: usize) -> &'static str {
    ["日", "一", "二", "三", "四", "五", "六"][d.min(6)]
}

pub(super) fn greeting() -> String {
    use chrono::Timelike;
    let hour = chrono::Local::now().hour();
    let salutation = match hour {
        5..=11 => "早上好",
        12..=17 => "下午好",
        18..=22 => "晚上好",
        _ => "夜深了",
    };
    format!("{salutation}，今天想先从哪里开始？")
}

pub(super) fn today_narrative(d: &mochi_core::analytics::DaySummary, streak: i64) -> String {
    let mut parts = Vec::new();
    if d.words_added > 0 {
        parts.push(format!("写下 {} 字", compact(d.words_added)));
    }
    if d.notes_edited > 0 {
        parts.push(format!("整理 {} 篇笔记", d.notes_edited));
    }
    if d.ai_messages > 0 {
        parts.push(format!("和 AI 交流 {} 条消息", d.ai_messages));
    }
    if d.focus_minutes > 0 {
        parts.push(format!("专注 {}", duration(d.focus_minutes)));
    }
    let mut narrative = if parts.is_empty() {
        if d.event_count > 0 {
            "今天已经开始，继续推进一个小步骤。".to_owned()
        } else {
            "先写下一行，今天的节奏会从这里开始。".to_owned()
        }
    } else {
        format!("今天{}。", parts.join("、"))
    };
    if streak >= 3 {
        narrative.push_str(&format!(" 已连续记录 {streak} 天。"));
    }
    narrative
}

pub(super) fn schedule_risk_label(risk: &str) -> &'static str {
    match risk {
        "high" => "较高",
        "medium" => "中等",
        _ => "较低",
    }
}

pub(super) fn schedule_status_label(status: &str) -> &'static str {
    match status {
        "done" | "completed" => "已完成",
        "in_progress" => "进行中",
        "cancelled" | "canceled" => "已取消",
        "archived" => "已归档",
        _ => "待处理",
    }
}

pub(super) fn schedule_kind_label(kind: &str) -> String {
    match kind {
        "event" => "日程".to_owned(),
        "task" => "任务".to_owned(),
        _ => kind.to_owned(),
    }
}

pub(super) fn hourly_cells(points: &[HourlyActivityPoint]) -> Vec<(usize, usize, f32)> {
    let peak = points
        .iter()
        .map(|point| point.weight)
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    points
        .iter()
        .map(|point| {
            (
                point.hour.min(23) as usize,
                point.day.min(6) as usize,
                (point.weight.max(0) as f32 / peak).min(1.0),
            )
        })
        .collect()
}
