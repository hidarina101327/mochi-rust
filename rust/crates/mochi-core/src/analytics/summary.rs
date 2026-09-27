//! 汇总工作区的每日活动、编辑记录和工作时段。
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Local, Timelike};
use serde::Serialize;

use super::buckets::{self, Bucket};
use super::events::{js_round, ActivityEvent, ActivityEventType};
use super::graph::stem;
use super::trends::{DailyActivityPoint, HourlyActivityPoint};

/// 相邻事件间隔超过这个阈值就切成两段专注，
/// 避免把「早上写完、晚上再开」算成一整天都在线。
const ACTIVE_SESSION_GAP_MS: i64 = 10 * 60 * 1000;
/// 单段专注至少记 1 分钟——只发生一次保存也是「来过」。
const MIN_SESSION_MINUTES: f64 = 1.0;
/// 「今天算不算高产」的对照窗口：不含今天的前 7 天。
const BASELINE_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkAreaStat {
    pub name: String,
    pub event_count: i64,
    pub net_words: i64,
    pub note_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchedNoteStat {
    pub path: String,
    pub title: String,
    pub save_count: i64,
    pub net_words: i64,
    pub last_touched_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummary {
    pub date: String,
    pub words_added: i64,
    pub words_removed: i64,
    pub net_words: i64,
    pub save_count: i64,
    pub notes_edited: usize,
    pub notes_created: usize,
    pub notes_opened: usize,
    pub ai_messages: i64,
    pub ai_tokens: i64,
    pub focus_sessions: i64,
    pub focus_minutes: i64,
    pub tasks_completed: i64,
    pub searches: i64,
    pub event_count: i64,
    pub active_minutes: i64,
    pub first_active_at: Option<String>,
    pub last_active_at: Option<String>,
    /// 24 个小时槽的**权重**（不是次数）。
    pub hourly: Vec<i64>,
}

impl DaySummary {
    pub fn empty(date: &str) -> Self {
        Self {
            date: date.to_owned(),
            words_added: 0,
            words_removed: 0,
            net_words: 0,
            save_count: 0,
            notes_edited: 0,
            notes_created: 0,
            notes_opened: 0,
            ai_messages: 0,
            ai_tokens: 0,
            focus_sessions: 0,
            focus_minutes: 0,
            tasks_completed: 0,
            searches: 0,
            event_count: 0,
            active_minutes: 0,
            first_active_at: None,
            last_active_at: None,
            hourly: vec![0; 24],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub days: i64,
    pub words_added: i64,
    pub notes_edited: i64,
    pub ai_messages: i64,
    pub focus_minutes: i64,
    pub active_minutes: i64,
}

/// 今日切片。`flatten` 让它序列化成 `DaySummary` 的字段 + 这几项，
/// 与 TS 的 `interface TodaySummary extends DaySummary` 同形。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodaySummary {
    #[serde(flatten)]
    pub day: DaySummary,
    pub top_notes: Vec<TouchedNoteStat>,
    pub areas: Vec<WorkAreaStat>,
    pub previous: DaySummary,
    pub baseline: Baseline,
}

impl TodaySummary {
    pub fn empty(date: &str) -> Self {
        let previous_key = buckets::parse_date_key(date)
            .map(|d| buckets::date_key(d - chrono::Duration::days(1)))
            .unwrap_or_default();
        Self {
            day: DaySummary::empty(date),
            top_notes: Vec::new(),
            areas: Vec::new(),
            previous: DaySummary::empty(&previous_key),
            baseline: Baseline {
                days: 0,
                words_added: 0,
                notes_edited: 0,
                ai_messages: 0,
                focus_minutes: 0,
                active_minutes: 0,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreakSummary {
    pub current: i64,
    pub longest: i64,
    pub active_days_this_week: i64,
    pub active_days_in_range: usize,
    pub this_week_words: i64,
    pub last_week_words: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Chronotype {
    EarlyBird,
    DayWorker,
    NightOwl,
    Irregular,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeOfDayShares {
    pub dawn: f64,
    pub morning: f64,
    pub afternoon: f64,
    pub night: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaSummary {
    pub chronotype: Option<Chronotype>,
    pub peak_hour: Option<usize>,
    pub peak_hour_share: f64,
    pub busiest_weekday: Option<usize>,
    pub time_of_day_shares: TimeOfDayShares,
    pub average_active_minutes: i64,
    pub average_words_per_active_day: i64,
    pub ai_collaboration_ratio: f64,
    pub focus_sessions: i64,
    pub focus_minutes: i64,
    pub top_areas: Vec<WorkAreaStat>,
}

impl Default for PersonaSummary {
    fn default() -> Self {
        Self {
            chronotype: None,
            peak_hour: None,
            peak_hour_share: 0.0,
            busiest_weekday: None,
            time_of_day_shares: TimeOfDayShares::default(),
            average_active_minutes: 0,
            average_words_per_active_day: 0,
            ai_collaboration_ratio: 0.0,
            focus_sessions: 0,
            focus_minutes: 0,
            top_areas: Vec::new(),
        }
    }
}

/// 在线时长：把事件序列切成若干段，段间空隙超过 10 分钟就断开。
///
/// 直接拿「最后一个事件减第一个」会把中间去吃饭的三小时也算成在写作；
/// 而按事件计数又完全反映不出时长。切段是这两者之间唯一说得通的口径。
pub fn active_minutes(timestamps: &[i64]) -> i64 {
    if timestamps.is_empty() {
        return 0;
    }
    let mut sorted = timestamps.to_vec();
    sorted.sort_unstable();

    let mut total = 0.0f64;
    let mut session_start = sorted[0];
    let mut previous = sorted[0];

    for &current in &sorted[1..] {
        if current - previous > ACTIVE_SESSION_GAP_MS {
            total += segment_minutes(session_start, previous);
            session_start = current;
        }
        previous = current;
    }
    total += segment_minutes(session_start, previous);
    js_round(total)
}

fn segment_minutes(start: i64, end: i64) -> f64 {
    ((end - start) as f64 / 60_000.0).max(MIN_SESSION_MINUTES)
}

/// 按本地日期把事件分组。返回的顺序即首次出现顺序（对齐 JS `Map` 的插入序）。
pub fn group_by_date(events: &[ActivityEvent]) -> Vec<(String, Vec<ActivityEvent>)> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: BTreeMap<String, Vec<ActivityEvent>> = BTreeMap::new();

    for event in events {
        let Some(local) = event.timestamp_millis().and_then(buckets::to_local) else {
            continue;
        };
        let key = buckets::date_key(local);
        if !grouped.contains_key(&key) {
            order.push(key.clone());
        }
        grouped.entry(key).or_default().push(event.clone());
    }

    order
        .into_iter()
        .filter_map(|key| grouped.remove(&key).map(|events| (key, events)))
        .collect()
}

/// 单日切片。
pub fn day_summary(events: &[ActivityEvent], date: &str) -> DaySummary {
    let mut summary = DaySummary::empty(date);
    if events.is_empty() {
        return summary;
    }

    let mut edited = BTreeSet::new();
    let mut created = BTreeSet::new();
    let mut opened = BTreeSet::new();
    let mut timestamps: Vec<i64> = Vec::new();
    let mut focus_minutes = 0.0f64;

    for event in events {
        let Some(millis) = event.timestamp_millis() else {
            continue;
        };
        timestamps.push(millis);
        summary.event_count += 1;
        if let Some(local) = buckets::to_local(millis) {
            summary.hourly[local.hour() as usize] += event.kind.weight();
        }

        match event.kind {
            ActivityEventType::FileSave => {
                summary.save_count += 1;
                if let Some(path) = event.relative_path.as_deref() {
                    edited.insert(path.to_owned());
                }
                let delta = event.words_delta_or_zero();
                if delta > 0 {
                    summary.words_added += delta;
                } else if delta < 0 {
                    summary.words_removed += -delta;
                }
            }
            ActivityEventType::FileCreate => {
                if let Some(path) = event.relative_path.as_deref() {
                    created.insert(path.to_owned());
                }
            }
            ActivityEventType::FileOpen => {
                if let Some(path) = event.relative_path.as_deref() {
                    opened.insert(path.to_owned());
                }
            }
            ActivityEventType::AiMessage => {
                // 只有用户消息算「我发起了几次对话」，助手回复不算
                if event.role.as_deref() == Some("user") {
                    summary.ai_messages += 1;
                }
                summary.ai_tokens += event.usage.as_ref().map_or(0, |u| u.total());
            }
            ActivityEventType::FocusSession => {
                summary.focus_sessions += 1;
                focus_minutes += event.duration_minutes.unwrap_or(0.0);
            }
            ActivityEventType::TaskComplete => {
                summary.tasks_completed += event.count.unwrap_or(1);
            }
            ActivityEventType::Search => summary.searches += 1,
            _ => {}
        }
    }

    summary.net_words = summary.words_added - summary.words_removed;
    summary.notes_edited = edited.len();
    summary.notes_created = created.len();
    summary.notes_opened = opened.len();
    summary.focus_minutes = js_round(focus_minutes);
    summary.active_minutes = active_minutes(&timestamps);
    summary.first_active_at = timestamps
        .iter()
        .min()
        .copied()
        .and_then(crate::jstime::from_millis);
    summary.last_active_at = timestamps
        .iter()
        .max()
        .copied()
        .and_then(crate::jstime::from_millis);
    summary
}

/// 工作区形如 `<库类型>/<库实例>/…`，取库实例作为「领域」最贴近用户心智。
pub fn area_name(relative_path: &str) -> String {
    let segments: Vec<&str> = relative_path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.len() {
        0 | 1 => "工作区根目录".to_owned(),
        // 有三段以上说明是 <库类型>/<库实例>/… ，取库实例
        n if n >= 3 => segments[1].to_owned(),
        _ => segments[0].to_owned(),
    }
}

/// 按「领域」聚合。事件数降序，相同则净字数降序。
pub fn area_stats(events: &[ActivityEvent], limit: usize) -> Vec<WorkAreaStat> {
    let mut order: Vec<String> = Vec::new();
    let mut stats: BTreeMap<String, (WorkAreaStat, BTreeSet<String>)> = BTreeMap::new();

    for event in events {
        let Some(path) = event.relative_path.as_deref() else {
            continue;
        };
        let name = area_name(path);
        if !stats.contains_key(&name) {
            order.push(name.clone());
        }
        let entry = stats.entry(name.clone()).or_insert_with(|| {
            (
                WorkAreaStat {
                    name,
                    event_count: 0,
                    net_words: 0,
                    note_count: 0,
                },
                BTreeSet::new(),
            )
        });
        entry.0.event_count += 1;
        entry.0.net_words += event.words_delta_or_zero();
        entry.1.insert(path.to_owned());
    }

    let mut out: Vec<WorkAreaStat> = order
        .into_iter()
        .filter_map(|name| stats.remove(&name))
        .map(|(mut stat, notes)| {
            stat.note_count = notes.len();
            stat
        })
        .collect();
    out.sort_by(|a, b| {
        b.event_count
            .cmp(&a.event_count)
            .then(b.net_words.cmp(&a.net_words))
    });
    out.truncate(limit);
    out
}

/// 碰过的笔记。净字数降序 → 保存次数降序 → 最近触碰时间降序。
pub fn touched_notes(events: &[ActivityEvent], limit: usize) -> Vec<TouchedNoteStat> {
    let mut order: Vec<String> = Vec::new();
    let mut notes: BTreeMap<String, TouchedNoteStat> = BTreeMap::new();

    for event in events {
        if !matches!(
            event.kind,
            ActivityEventType::FileSave
                | ActivityEventType::FileCreate
                | ActivityEventType::FileOpen
        ) {
            continue;
        }
        let Some(path) = event.relative_path.as_deref() else {
            continue;
        };

        if !notes.contains_key(path) {
            order.push(path.to_owned());
        }
        let entry = notes
            .entry(path.to_owned())
            .or_insert_with(|| TouchedNoteStat {
                path: path.to_owned(),
                title: stem(path),
                save_count: 0,
                net_words: 0,
                last_touched_at: event.timestamp.clone(),
            });

        if event.kind == ActivityEventType::FileSave {
            entry.save_count += 1;
            entry.net_words += event.words_delta_or_zero();
        }
        // ISO8601 同格式同时区，字典序即时间序
        if event.timestamp > entry.last_touched_at {
            entry.last_touched_at = event.timestamp.clone();
        }
    }

    let mut out: Vec<TouchedNoteStat> =
        order.into_iter().filter_map(|p| notes.remove(&p)).collect();
    out.sort_by(|a, b| {
        b.net_words
            .cmp(&a.net_words)
            .then(b.save_count.cmp(&a.save_count))
            .then(b.last_touched_at.cmp(&a.last_touched_at))
    });
    out.truncate(limit);
    out
}

/// 今日切片 + 昨日对照 + 近 7 天基线。
pub fn today_summary(
    summaries: &BTreeMap<String, DaySummary>,
    events_by_date: &BTreeMap<String, Vec<ActivityEvent>>,
    now: DateTime<Local>,
) -> TodaySummary {
    let today_key = buckets::date_key(now);
    let yesterday_key = buckets::date_key(buckets::start_of_day(now) - chrono::Duration::days(1));

    let day = summaries
        .get(&today_key)
        .cloned()
        .unwrap_or_else(|| DaySummary::empty(&today_key));
    let previous = summaries
        .get(&yesterday_key)
        .cloned()
        .unwrap_or_else(|| DaySummary::empty(&yesterday_key));
    let today_events = events_by_date.get(&today_key).cloned().unwrap_or_default();

    // 基线**不含今天**：拿今天跟包含今天的均值比是没有意义的
    let mut days = 0i64;
    let (mut words, mut notes, mut ai, mut focus, mut active) = (0i64, 0i64, 0i64, 0i64, 0i64);
    let mut cursor = buckets::start_of_day(now);
    for _ in 0..BASELINE_DAYS {
        cursor -= chrono::Duration::days(1);
        let Some(summary) = summaries.get(&buckets::date_key(cursor)) else {
            continue;
        };
        // 完全没活动的日子不拉低均值——那是休息日，不是低产日
        if summary.event_count == 0 {
            continue;
        }
        days += 1;
        words += summary.words_added;
        notes += summary.notes_edited as i64;
        ai += summary.ai_messages;
        focus += summary.focus_minutes;
        active += summary.active_minutes;
    }
    let average = |total: i64| {
        if days > 0 {
            js_round(total as f64 / days as f64)
        } else {
            0
        }
    };

    TodaySummary {
        day,
        top_notes: touched_notes(&today_events, 4),
        areas: area_stats(&today_events, 4),
        previous,
        baseline: Baseline {
            days,
            words_added: average(words),
            notes_edited: average(notes),
            ai_messages: average(ai),
            focus_minutes: average(focus),
            active_minutes: average(active),
        },
    }
}

/// 连续活跃天数与本周/上周对照。
pub fn streak_summary(
    contributions: &[DailyActivityPoint],
    summaries: &BTreeMap<String, DaySummary>,
    now: DateTime<Local>,
) -> StreakSummary {
    let active_dates: BTreeSet<&str> = contributions
        .iter()
        .filter(|p| p.count > 0)
        .map(|p| p.date.as_str())
        .collect();

    let mut longest = 0i64;
    let mut running = 0i64;
    for point in contributions {
        if point.count > 0 {
            running += 1;
            longest = longest.max(running);
        } else {
            running = 0;
        }
    }

    // 今天还没开工时不该把连续记录清零，从昨天继续往回数
    let mut cursor = buckets::start_of_day(now);
    if !active_dates.contains(buckets::date_key(cursor).as_str()) {
        cursor -= chrono::Duration::days(1);
    }
    let mut current = 0i64;
    while active_dates.contains(buckets::date_key(cursor).as_str()) {
        current += 1;
        cursor -= chrono::Duration::days(1);
    }

    let this_week_start = buckets::week_start(now);
    let last_week_start = this_week_start - chrono::Duration::days(7);

    let mut active_days_this_week = 0i64;
    let mut this_week_words = 0i64;
    let mut last_week_words = 0i64;

    for point in contributions {
        let Some(date) = buckets::parse_date_key(&point.date) else {
            continue;
        };
        let words = summaries.get(&point.date).map_or(0, |s| s.words_added);

        if date >= this_week_start {
            if point.count > 0 {
                active_days_this_week += 1;
            }
            this_week_words += words;
        } else if date >= last_week_start {
            last_week_words += words;
        }
    }

    StreakSummary {
        current,
        longest,
        active_days_this_week,
        active_days_in_range: active_dates.len(),
        this_week_words,
        last_week_words,
    }
}

/// 作息类型。占比不到 40% 就是 `irregular`——分散的作息不该被硬贴一个标签。
pub fn classify_chronotype(shares: &TimeOfDayShares) -> Option<Chronotype> {
    // 顺序即并列时的优先级（JS 的 sort 稳定），别重排
    let candidates = [
        (Chronotype::NightOwl, shares.dawn + shares.night),
        (Chronotype::EarlyBird, shares.morning),
        (Chronotype::DayWorker, shares.afternoon),
    ];
    let (kind, share) =
        candidates
            .iter()
            .copied()
            .reduce(|best, next| if next.1 > best.1 { next } else { best })?;

    if share <= 0.0 {
        return None;
    }
    Some(if share >= 0.4 {
        kind
    } else {
        Chronotype::Irregular
    })
}

/// 人格画像：作息、峰值时段、平均产出、AI 协作比例。
pub fn persona(
    events: &[ActivityEvent],
    summaries: &BTreeMap<String, DaySummary>,
    hourly: &[HourlyActivityPoint],
) -> PersonaSummary {
    let mut hour_weights = [0i64; 24];
    let mut weekday_weights = [0i64; 7];
    let mut total_weight = 0i64;

    for point in hourly {
        hour_weights[point.hour as usize] += point.weight;
        weekday_weights[point.day as usize] += point.weight;
        total_weight += point.weight;
    }

    let peak_weight = hour_weights.iter().copied().max().unwrap_or(0);
    let peak_hour = (peak_weight > 0)
        .then(|| hour_weights.iter().position(|w| *w == peak_weight))
        .flatten();
    let busiest_weight = weekday_weights.iter().copied().max().unwrap_or(0);
    let busiest_weekday = (busiest_weight > 0)
        .then(|| weekday_weights.iter().position(|w| *w == busiest_weight))
        .flatten();

    let share = |value: i64| {
        if total_weight > 0 {
            value as f64 / total_weight as f64
        } else {
            0.0
        }
    };
    let sum_hours = |from: usize, to: usize| hour_weights[from..to].iter().sum::<i64>();
    let time_of_day_shares = TimeOfDayShares {
        dawn: share(sum_hours(0, 6)),
        morning: share(sum_hours(6, 12)),
        afternoon: share(sum_hours(12, 18)),
        night: share(sum_hours(18, 24)),
    };

    let active_days: Vec<&DaySummary> = summaries.values().filter(|s| s.event_count > 0).collect();
    let count = active_days.len() as i64;
    let sum = |f: fn(&DaySummary) -> i64| active_days.iter().map(|s| f(s)).sum::<i64>();

    let total_saves = sum(|s| s.save_count);
    let total_ai = sum(|s| s.ai_messages);
    let collaboration_base = total_saves + total_ai;

    PersonaSummary {
        chronotype: classify_chronotype(&time_of_day_shares),
        peak_hour,
        peak_hour_share: share(peak_weight),
        busiest_weekday,
        time_of_day_shares,
        average_active_minutes: if count > 0 {
            js_round(sum(|s| s.active_minutes) as f64 / count as f64)
        } else {
            0
        },
        average_words_per_active_day: if count > 0 {
            js_round(sum(|s| s.words_added) as f64 / count as f64)
        } else {
            0
        },
        ai_collaboration_ratio: if collaboration_base > 0 {
            total_ai as f64 / collaboration_base as f64
        } else {
            0.0
        },
        focus_sessions: sum(|s| s.focus_sessions),
        focus_minutes: sum(|s| s.focus_minutes),
        top_areas: area_stats(events, 5),
    }
}

/// 把日桶的贡献点与日摘要对齐，供 `streak_summary` 使用。
pub fn summaries_by_date(groups: &[(String, Vec<ActivityEvent>)]) -> BTreeMap<String, DaySummary> {
    groups
        .iter()
        .map(|(date, events)| (date.clone(), day_summary(events, date)))
        .collect()
}

/// 日桶列表转成「日期 → 事件」映射，供 `today_summary` 使用。
pub fn events_by_date(
    groups: &[(String, Vec<ActivityEvent>)],
) -> BTreeMap<String, Vec<ActivityEvent>> {
    groups.iter().cloned().collect()
}

/// 空贡献序列（全部为 0），用于工作区不可读时的兜底快照。
pub fn empty_contributions(day_buckets: &[Bucket]) -> Vec<DailyActivityPoint> {
    day_buckets
        .iter()
        .map(|b| DailyActivityPoint {
            date: b.key.clone(),
            count: 0,
            weight: 0,
        })
        .collect()
}

/// 今天是星期几（0 = 周日），供上层做展示时对齐 `busiest_weekday` 的口径。
pub fn weekday_index(date: DateTime<Local>) -> usize {
    date.weekday().num_days_from_sunday() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::events::TokenUsage;
    use crate::analytics::trends::activity_summary;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn event(kind: ActivityEventType, when: DateTime<Local>) -> ActivityEvent {
        ActivityEvent {
            version: 1,
            id: format!("{}", when.timestamp_millis()),
            kind,
            timestamp: crate::jstime::from_millis(when.timestamp_millis()).unwrap(),
            relative_path: None,
            extension: None,
            session_id: None,
            agent_id: None,
            role: None,
            model: None,
            provider_id: None,
            usage: None,
            usage_source: None,
            words: None,
            words_delta: None,
            duration_minutes: None,
            count: None,
        }
    }

    fn at_path(kind: ActivityEventType, when: DateTime<Local>, path: &str) -> ActivityEvent {
        ActivityEvent {
            relative_path: Some(path.into()),
            ..event(kind, when)
        }
    }

    fn save(when: DateTime<Local>, path: &str, delta: i64) -> ActivityEvent {
        ActivityEvent {
            words_delta: Some(delta),
            ..at_path(ActivityEventType::FileSave, when, path)
        }
    }

    // ---------- 在线时长 ----------

    /// 中间去吃饭的三小时不该算成在写作。
    #[test]
    fn active_minutes_split_on_long_gaps() {
        let base = at(2026, 8, 29, 9, 0).timestamp_millis();
        let minute = 60_000;
        let stamps =
            |offsets: &[i64]| -> Vec<i64> { offsets.iter().map(|m| base + m * minute).collect() };

        // 09:00–09:30 每 5 分钟一次（都在 10 分钟阈值内，算一整段），
        // 空 3 小时，再 12:30–12:50 每 5 分钟一次
        let all = stamps(&[0, 5, 10, 15, 20, 25, 30, 210, 215, 220, 225, 230]);
        assert_eq!(active_minutes(&all), 50, "30 + 20，中间三小时不计");
    }

    /// 阈值是 10 分钟：正好 10 分钟仍算连续，超过就断开。
    /// 这个边界值得钉住——写这条测试时我一开始用了 15 分钟的间隔，
    /// 结果每个事件都自成一段，断言写的是「连续 30 分钟」，实际验的是「断成 5 段」。
    #[test]
    fn the_split_threshold_is_exactly_ten_minutes() {
        let base = at(2026, 8, 29, 9, 0).timestamp_millis();
        let minute = 60_000;

        // 间隔恰好 10 分钟：不断开，两段合成 20 分钟
        assert_eq!(
            active_minutes(&[base, base + 10 * minute, base + 20 * minute]),
            20
        );
        // 间隔 10 分钟零 1 毫秒：断开，两段各记最低 1 分钟
        assert_eq!(active_minutes(&[base, base + 10 * minute + 1]), 2);
    }

    #[test]
    fn a_single_event_still_counts_as_one_minute() {
        let base = at(2026, 8, 29, 9, 0).timestamp_millis();
        assert_eq!(active_minutes(&[base]), 1, "来过就算一分钟");
        assert_eq!(active_minutes(&[]), 0);
        // 两个挨得极近的事件也只算一分钟
        assert_eq!(active_minutes(&[base, base + 1000]), 1);
    }

    #[test]
    fn active_minutes_do_not_depend_on_input_order() {
        let base = at(2026, 8, 29, 9, 0).timestamp_millis();
        let ordered = vec![base, base + 600_000, base + 1_200_000];
        let mut shuffled = ordered.clone();
        shuffled.reverse();
        assert_eq!(active_minutes(&ordered), active_minutes(&shuffled));
    }

    // ---------- 单日切片 ----------

    #[test]
    fn a_day_summary_aggregates_every_event_kind() {
        let day = at(2026, 8, 29, 9, 0);
        let events = vec![
            save(day, "知识库/数学/a.mc", 120),
            save(at(2026, 8, 29, 9, 5), "知识库/数学/a.mc", -20),
            save(at(2026, 8, 29, 9, 10), "知识库/数学/b.mc", 30),
            at_path(
                ActivityEventType::FileCreate,
                at(2026, 8, 29, 9, 1),
                "知识库/数学/b.mc",
            ),
            at_path(
                ActivityEventType::FileOpen,
                at(2026, 8, 29, 9, 2),
                "知识库/数学/c.mc",
            ),
            ActivityEvent {
                role: Some("user".into()),
                usage: Some(TokenUsage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(5),
                    total_tokens: Some(15),
                }),
                ..event(ActivityEventType::AiMessage, at(2026, 8, 29, 9, 3))
            },
            ActivityEvent {
                role: Some("assistant".into()),
                usage: Some(TokenUsage {
                    prompt_tokens: Some(0),
                    completion_tokens: Some(25),
                    total_tokens: Some(25),
                }),
                ..event(ActivityEventType::AiMessage, at(2026, 8, 29, 9, 4))
            },
            ActivityEvent {
                duration_minutes: Some(25.0),
                ..event(ActivityEventType::FocusSession, at(2026, 8, 29, 9, 6))
            },
            ActivityEvent {
                count: Some(3),
                ..event(ActivityEventType::TaskComplete, at(2026, 8, 29, 9, 7))
            },
            event(ActivityEventType::Search, at(2026, 8, 29, 9, 8)),
        ];

        let summary = day_summary(&events, "2026-08-29");
        assert_eq!(summary.words_added, 150);
        assert_eq!(summary.words_removed, 20);
        assert_eq!(summary.net_words, 130);
        assert_eq!(summary.save_count, 3);
        assert_eq!(summary.notes_edited, 2, "两篇被保存过");
        assert_eq!(summary.notes_created, 1);
        assert_eq!(summary.notes_opened, 1);
        assert_eq!(summary.ai_messages, 1, "只数用户消息");
        assert_eq!(summary.ai_tokens, 40, "但 token 两边都算");
        assert_eq!(summary.focus_sessions, 1);
        assert_eq!(summary.focus_minutes, 25);
        assert_eq!(summary.tasks_completed, 3, "一次勾掉三个待办");
        assert_eq!(summary.searches, 1);
        assert_eq!(summary.event_count, 10);
        assert!(summary.first_active_at.is_some() && summary.last_active_at.is_some());
    }

    #[test]
    fn hourly_slots_hold_weights_not_counts() {
        let summary = day_summary(
            &[
                event(ActivityEventType::FileSave, at(2026, 8, 29, 14, 0)), // 3
                event(ActivityEventType::FileCreate, at(2026, 8, 29, 14, 30)), // 4
                event(ActivityEventType::HomeView, at(2026, 8, 29, 20, 0)), // 1
            ],
            "2026-08-29",
        );
        assert_eq!(summary.hourly.len(), 24);
        assert_eq!(summary.hourly[14], 7, "权重相加，不是次数");
        assert_eq!(summary.hourly[20], 1);
        assert_eq!(summary.hourly[0], 0);
    }

    #[test]
    fn an_empty_day_is_all_zeroes_with_no_timestamps() {
        let summary = day_summary(&[], "2026-08-29");
        assert_eq!(summary, DaySummary::empty("2026-08-29"));
        assert!(summary.first_active_at.is_none());
    }

    // ---------- 领域 ----------

    /// `<库类型>/<库实例>/…` 取库实例；层级不够时退到第一段。
    #[test]
    fn area_names_pick_the_library_instance() {
        assert_eq!(area_name("知识库/数学/概率.mc"), "数学");
        assert_eq!(area_name("知识库/数学/子目录/概率.mc"), "数学");
        assert_eq!(area_name("知识库/概率.mc"), "知识库", "只有两段时取第一段");
        assert_eq!(area_name("概率.mc"), "工作区根目录");
        assert_eq!(area_name(""), "工作区根目录");
    }

    #[test]
    fn areas_rank_by_event_count_then_net_words() {
        let day = at(2026, 8, 29, 9, 0);
        let events = vec![
            save(day, "知识库/数学/a.mc", 10),
            save(day, "知识库/数学/b.mc", 10),
            save(day, "知识库/物理/a.mc", 500),
        ];
        let areas = area_stats(&events, 5);
        assert_eq!(areas[0].name, "数学", "事件数优先于字数");
        assert_eq!(areas[0].event_count, 2);
        assert_eq!(areas[0].note_count, 2);
        assert_eq!(areas[1].name, "物理");
        assert_eq!(areas[1].net_words, 500);
    }

    #[test]
    fn areas_respect_the_limit_and_skip_pathless_events() {
        let day = at(2026, 8, 29, 9, 0);
        let mut events = vec![event(ActivityEventType::Search, day)];
        for i in 0..8 {
            events.push(save(day, &format!("知识库/领域{i}/a.mc"), 1));
        }
        assert_eq!(area_stats(&events, 3).len(), 3);
        assert!(area_stats(&events, 99)
            .iter()
            .all(|a| a.name != "工作区根目录"));
    }

    // ---------- 碰过的笔记 ----------

    #[test]
    fn touched_notes_rank_by_net_words_then_saves() {
        let day = at(2026, 8, 29, 9, 0);
        let events = vec![
            save(day, "a.mc", 10),
            save(at(2026, 8, 29, 9, 1), "a.mc", 10),
            save(day, "b.mc", 500),
            at_path(ActivityEventType::FileOpen, day, "c.mc"),
        ];
        let notes = touched_notes(&events, 5);

        assert_eq!(notes[0].path, "b.mc");
        assert_eq!(notes[0].net_words, 500);
        assert_eq!(notes[1].path, "a.mc");
        assert_eq!(notes[1].save_count, 2);
        assert_eq!(notes[2].path, "c.mc", "只打开过的排最后");
        assert_eq!(notes[2].save_count, 0);
        assert_eq!(notes[0].title, "b", "标题去掉扩展名");
    }

    #[test]
    fn last_touched_tracks_the_newest_event() {
        let events = vec![
            at_path(ActivityEventType::FileOpen, at(2026, 8, 29, 9, 0), "a.mc"),
            at_path(ActivityEventType::FileSave, at(2026, 8, 29, 18, 0), "a.mc"),
            at_path(ActivityEventType::FileOpen, at(2026, 8, 29, 12, 0), "a.mc"),
        ];
        let notes = touched_notes(&events, 5);
        assert_eq!(
            notes[0].last_touched_at,
            crate::jstime::from_millis(at(2026, 8, 29, 18, 0).timestamp_millis()).unwrap()
        );
    }

    #[test]
    fn only_file_events_can_be_touched_notes() {
        let day = at(2026, 8, 29, 9, 0);
        let events = vec![
            at_path(ActivityEventType::Search, day, "a.mc"),
            at_path(ActivityEventType::HomeView, day, "b.mc"),
        ];
        assert!(touched_notes(&events, 5).is_empty());
    }

    // ---------- 今日 ----------

    /// 基线是**不含今天**的前 7 天，而且跳过完全没活动的日子。
    #[test]
    fn the_baseline_excludes_today_and_idle_days() {
        let now = at(2026, 8, 29, 15, 0);
        let mut summaries = BTreeMap::new();
        // 今天产出很高，不该被算进自己的基线
        summaries.insert(
            "2026-08-29".to_string(),
            DaySummary {
                words_added: 9999,
                event_count: 5,
                ..DaySummary::empty("2026-08-29")
            },
        );
        // 前两天各写 100，再前一天完全没活动
        for date in ["2026-08-28", "2026-08-27"] {
            summaries.insert(
                date.to_string(),
                DaySummary {
                    words_added: 100,
                    event_count: 3,
                    ..DaySummary::empty(date)
                },
            );
        }
        summaries.insert("2026-08-26".to_string(), DaySummary::empty("2026-08-26"));

        let today = today_summary(&summaries, &BTreeMap::new(), now);
        assert_eq!(today.baseline.days, 2, "只有两天有活动");
        assert_eq!(today.baseline.words_added, 100, "9999 不该混进基线");
        assert_eq!(today.day.words_added, 9999);
        assert_eq!(today.previous.date, "2026-08-28");
    }

    #[test]
    fn a_day_with_no_data_still_produces_a_shaped_summary() {
        let now = at(2026, 8, 29, 15, 0);
        let today = today_summary(&BTreeMap::new(), &BTreeMap::new(), now);
        assert_eq!(today.day.date, "2026-08-29");
        assert_eq!(today.previous.date, "2026-08-28");
        assert_eq!(today.baseline.days, 0);
        assert!(today.top_notes.is_empty() && today.areas.is_empty());
    }

    #[test]
    fn the_empty_today_summary_derives_yesterday_correctly() {
        assert_eq!(
            TodaySummary::empty("2026-09-01").previous.date,
            "2026-08-31",
            "跨月要退到上月末"
        );
        assert_eq!(
            TodaySummary::empty("2027-01-01").previous.date,
            "2026-12-31",
            "跨年也要对"
        );
    }

    /// 今日切片序列化成扁平结构，与 TS 的 `extends DaySummary` 同形。
    #[test]
    fn today_serializes_flat_like_the_ts_interface() {
        let json = serde_json::to_value(TodaySummary::empty("2026-08-29")).unwrap();
        assert_eq!(json["date"], "2026-08-29", "DaySummary 的字段要在顶层");
        assert!(json["hourly"].is_array());
        assert!(json["baseline"]["days"].is_number());
        assert!(json["day"].is_null(), "不该出现嵌套的 day 键");
    }

    // ---------- 连续天数 ----------

    #[test]
    fn streaks_count_back_from_today() {
        let now = at(2026, 8, 29, 15, 0);
        let contributions: Vec<DailyActivityPoint> =
            ["2026-08-26", "2026-08-27", "2026-08-28", "2026-08-29"]
                .iter()
                .map(|d| DailyActivityPoint {
                    date: (*d).to_string(),
                    count: 1,
                    weight: 3,
                })
                .collect();

        let streak = streak_summary(&contributions, &BTreeMap::new(), now);
        assert_eq!(streak.current, 4);
        assert_eq!(streak.longest, 4);
        assert_eq!(streak.active_days_in_range, 4);
    }

    /// 今天还没开工不该把连续记录清零。
    #[test]
    fn an_idle_today_does_not_break_the_streak() {
        let now = at(2026, 8, 29, 9, 0);
        let mut contributions: Vec<DailyActivityPoint> = ["2026-08-27", "2026-08-28"]
            .iter()
            .map(|d| DailyActivityPoint {
                date: (*d).to_string(),
                count: 1,
                weight: 3,
            })
            .collect();
        contributions.push(DailyActivityPoint {
            date: "2026-08-29".into(),
            count: 0,
            weight: 0,
        });

        let streak = streak_summary(&contributions, &BTreeMap::new(), now);
        assert_eq!(streak.current, 2, "从昨天继续往回数");
    }

    #[test]
    fn the_longest_streak_survives_a_later_gap() {
        let now = at(2026, 8, 29, 15, 0);
        let contributions: Vec<DailyActivityPoint> = [
            ("2026-08-20", 1),
            ("2026-08-21", 1),
            ("2026-08-22", 1),
            ("2026-08-23", 1),
            ("2026-08-24", 0),
            ("2026-08-25", 0),
            ("2026-08-28", 1),
            ("2026-08-29", 1),
        ]
        .iter()
        .map(|(d, c)| DailyActivityPoint {
            date: (*d).to_string(),
            count: *c,
            weight: *c * 3,
        })
        .collect();

        let streak = streak_summary(&contributions, &BTreeMap::new(), now);
        assert_eq!(streak.longest, 4, "早先那段更长");
        assert_eq!(streak.current, 2);
    }

    #[test]
    fn weekly_word_counts_split_at_the_week_boundary() {
        // 2026-08-29 周六，本周从 8/24（周一）起
        let now = at(2026, 8, 29, 15, 0);
        let dates = ["2026-08-20", "2026-08-25", "2026-08-29"];
        let contributions: Vec<DailyActivityPoint> = dates
            .iter()
            .map(|d| DailyActivityPoint {
                date: (*d).to_string(),
                count: 1,
                weight: 3,
            })
            .collect();
        let summaries: BTreeMap<String, DaySummary> = dates
            .iter()
            .map(|d| {
                (
                    d.to_string(),
                    DaySummary {
                        words_added: 100,
                        event_count: 1,
                        ..DaySummary::empty(d)
                    },
                )
            })
            .collect();

        let streak = streak_summary(&contributions, &summaries, now);
        assert_eq!(streak.this_week_words, 200, "8/25 与 8/29 属于本周");
        assert_eq!(streak.last_week_words, 100, "8/20 属于上周");
        assert_eq!(streak.active_days_this_week, 2);
    }

    // ---------- 人格 ----------

    #[test]
    fn chronotypes_need_a_forty_percent_share() {
        assert_eq!(
            classify_chronotype(&TimeOfDayShares {
                dawn: 0.1,
                morning: 0.1,
                afternoon: 0.1,
                night: 0.7
            }),
            Some(Chronotype::NightOwl),
            "凌晨 + 夜间合并算夜猫子"
        );
        assert_eq!(
            classify_chronotype(&TimeOfDayShares {
                dawn: 0.0,
                morning: 0.5,
                afternoon: 0.3,
                night: 0.2
            }),
            Some(Chronotype::EarlyBird)
        );
        assert_eq!(
            classify_chronotype(&TimeOfDayShares {
                dawn: 0.0,
                morning: 0.2,
                afternoon: 0.6,
                night: 0.2
            }),
            Some(Chronotype::DayWorker)
        );
        assert_eq!(
            classify_chronotype(&TimeOfDayShares {
                dawn: 0.1,
                morning: 0.35,
                afternoon: 0.35,
                night: 0.2
            }),
            Some(Chronotype::Irregular),
            "都不到四成就是作息不规律，不该硬贴标签"
        );
        assert_eq!(
            classify_chronotype(&TimeOfDayShares::default()),
            None,
            "没数据就不下判断"
        );
    }

    #[test]
    fn chronotype_serializes_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&Chronotype::NightOwl).unwrap(),
            "\"night-owl\""
        );
        assert_eq!(
            serde_json::to_string(&Chronotype::EarlyBird).unwrap(),
            "\"early-bird\""
        );
        assert_eq!(
            serde_json::to_string(&Chronotype::DayWorker).unwrap(),
            "\"day-worker\""
        );
        assert_eq!(
            serde_json::to_string(&Chronotype::Irregular).unwrap(),
            "\"irregular\""
        );
    }

    #[test]
    fn persona_reads_peaks_off_the_hourly_grid() {
        let now = at(2026, 8, 29, 23, 0); // 周六
        let events: Vec<ActivityEvent> = (0..5)
            .map(|i| save(at(2026, 8, 29, 22, i), "知识库/数学/a.mc", 10))
            .collect();
        let summary = activity_summary(&events, &buckets::day_buckets(1, now));
        let groups = group_by_date(&events);
        let summaries = summaries_by_date(&groups);

        let persona = persona(&events, &summaries, &summary.hourly);
        assert_eq!(persona.peak_hour, Some(22));
        assert_eq!(persona.busiest_weekday, Some(6), "周六");
        assert!((persona.peak_hour_share - 1.0).abs() < 1e-9);
        assert!(persona.time_of_day_shares.night > 0.99, "22 点属于 night");
        assert_eq!(persona.chronotype, Some(Chronotype::NightOwl));
        assert_eq!(persona.top_areas[0].name, "数学");
    }

    #[test]
    fn the_collaboration_ratio_compares_ai_messages_to_saves() {
        let mut summaries = BTreeMap::new();
        summaries.insert(
            "2026-08-29".to_string(),
            DaySummary {
                save_count: 3,
                ai_messages: 1,
                words_added: 200,
                active_minutes: 60,
                event_count: 4,
                ..DaySummary::empty("2026-08-29")
            },
        );
        let persona = persona(&[], &summaries, &[]);
        assert!(
            (persona.ai_collaboration_ratio - 0.25).abs() < 1e-9,
            "1 / (3+1)"
        );
        assert_eq!(persona.average_words_per_active_day, 200);
        assert_eq!(persona.average_active_minutes, 60);
    }

    #[test]
    fn an_empty_persona_makes_no_claims() {
        let persona = persona(&[], &BTreeMap::new(), &[]);
        assert_eq!(persona, PersonaSummary::default());
        assert!(persona.chronotype.is_none() && persona.peak_hour.is_none());
    }

    // ---------- 分组 ----------

    #[test]
    fn events_group_by_local_date_in_first_seen_order() {
        let events = vec![
            event(ActivityEventType::FileSave, at(2026, 8, 28, 10, 0)),
            event(ActivityEventType::FileSave, at(2026, 8, 29, 10, 0)),
            event(ActivityEventType::FileSave, at(2026, 8, 28, 11, 0)),
        ];
        let groups = group_by_date(&events);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "2026-08-28");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, "2026-08-29");
    }
}
