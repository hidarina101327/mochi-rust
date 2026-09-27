//! 按日期、时段和活动类型整理工作区趋势数据。
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::buckets::{self, Bucket, Granularity};
use super::events::{ActivityEvent, ActivityEventType, TokenUsage};
use super::graph::ScannedFile;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    pub key: String,
    pub label: String,
    pub created_count: usize,
    pub updated_count: usize,
    pub total_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivityPoint {
    pub date: String,
    pub count: i64,
    pub weight: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourlyActivityPoint {
    /// 0 = 周日，与 JS 的 `getDay()` 同义。
    pub day: u32,
    pub hour: u32,
    pub count: i64,
    pub weight: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTrendPoint {
    pub key: String,
    pub label: String,
    pub conversation_count: i64,
    pub user_message_count: i64,
    pub assistant_message_count: i64,
    pub active_session_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTrendPoint {
    pub key: String,
    pub label: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub estimated_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WritingTrendPoint {
    pub key: String,
    pub label: String,
    pub words_added: i64,
    pub words_removed: i64,
    pub net_words: i64,
    pub save_count: i64,
    pub note_count: usize,
}

/// 24×7 的空热力图，按 day 外层、hour 内层排列。顺序即前端网格的读取顺序。
pub fn empty_hourly_points() -> Vec<HourlyActivityPoint> {
    (0..7)
        .flat_map(|day| {
            (0..24).map(move |hour| HourlyActivityPoint {
                day,
                hour,
                count: 0,
                weight: 0,
            })
        })
        .collect()
}

/// 笔记数量趋势。
///
/// `total_count` 是**截至该桶结束时的累计存量**（不是桶内新增），
/// 所以它随时间单调不减——图上是一条增长曲线，不是柱状。
pub fn note_trend(files: &[ScannedFile], buckets: &[Bucket]) -> Vec<TrendPoint> {
    let notes: Vec<&ScannedFile> = files.iter().filter(|f| f.is_note()).collect();

    buckets
        .iter()
        .map(|bucket| {
            let (start, end) = (
                bucket.start.timestamp_millis(),
                bucket.end.timestamp_millis(),
            );
            TrendPoint {
                key: bucket.key.clone(),
                label: bucket.label.clone(),
                created_count: notes
                    .iter()
                    .filter(|f| f.created_at >= start && f.created_at <= end)
                    .count(),
                updated_count: notes
                    .iter()
                    .filter(|f| f.updated_at >= start && f.updated_at <= end)
                    .count(),
                total_count: notes.iter().filter(|f| f.created_at <= end).count(),
            }
        })
        .collect()
}

pub struct ActivitySummary {
    pub contributions: Vec<DailyActivityPoint>,
    pub hourly: Vec<HourlyActivityPoint>,
}

/// 贡献热力（按日）与作息热力（按星期×小时）。
///
/// 只统计**落在日桶范围内**的事件；范围外的被静默忽略，
/// 因为 `contributions` 的长度必须等于日桶数，前端按下标画格子。
pub fn activity_summary(events: &[ActivityEvent], day_buckets: &[Bucket]) -> ActivitySummary {
    let mut contributions: Vec<DailyActivityPoint> = day_buckets
        .iter()
        .map(|b| DailyActivityPoint {
            date: b.key.clone(),
            count: 0,
            weight: 0,
        })
        .collect();
    let index_by_date: BTreeMap<String, usize> = day_buckets
        .iter()
        .enumerate()
        .map(|(index, b)| (b.key.clone(), index))
        .collect();

    let mut hourly = empty_hourly_points();

    for event in events {
        let Some(millis) = event.timestamp_millis() else {
            continue;
        };
        let Some(local) = buckets::to_local(millis) else {
            continue;
        };
        let weight = event.kind.weight();

        if let Some(index) = index_by_date.get(&buckets::date_key(local)) {
            contributions[*index].count += 1;
            contributions[*index].weight += weight;
        }

        use chrono::{Datelike, Timelike};
        let slot = local.weekday().num_days_from_sunday() as usize * 24 + local.hour() as usize;
        if let Some(point) = hourly.get_mut(slot) {
            point.count += 1;
            point.weight += weight;
        }
    }

    ActivitySummary {
        contributions,
        hourly,
    }
}

pub struct AiTrend {
    pub trend: Vec<AiTrendPoint>,
    pub tokens: Vec<TokenTrendPoint>,
}

/// AI 对话与模型用量趋势。
///
/// `conversation_count` 跟着**用户消息**递增而不是助手消息——一轮对话由用户发起，
/// 用助手消息计数会把一次追问算成两轮。
pub fn ai_trend(
    events: &[ActivityEvent],
    bucket_list: &[Bucket],
    granularity: Granularity,
) -> AiTrend {
    let mut trend: Vec<AiTrendPoint> = bucket_list
        .iter()
        .map(|b| AiTrendPoint {
            key: b.key.clone(),
            label: b.label.clone(),
            conversation_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            active_session_count: 0,
        })
        .collect();
    let mut tokens: Vec<TokenTrendPoint> = bucket_list
        .iter()
        .map(|b| TokenTrendPoint {
            key: b.key.clone(),
            label: b.label.clone(),
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            estimated_tokens: 0,
        })
        .collect();

    let index_by_key: BTreeMap<String, usize> = bucket_list
        .iter()
        .enumerate()
        .map(|(index, b)| (b.key.clone(), index))
        .collect();
    let mut sessions_by_bucket: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();

    for event in events {
        if !matches!(
            event.kind,
            ActivityEventType::AiMessage | ActivityEventType::AiSessionOpen
        ) {
            continue;
        }
        let Some(millis) = event.timestamp_millis() else {
            continue;
        };
        let Some(local) = buckets::to_local(millis) else {
            continue;
        };
        let Some(&index) = index_by_key.get(&buckets::bucket_key(local, granularity)) else {
            continue;
        };

        if let Some(session) = event.session_id.as_deref().filter(|s| !s.is_empty()) {
            sessions_by_bucket
                .entry(index)
                .or_default()
                .insert(session.to_owned());
        }

        if event.kind != ActivityEventType::AiMessage {
            continue;
        }
        match event.role.as_deref() {
            Some("user") => {
                trend[index].user_message_count += 1;
                trend[index].conversation_count += 1;
            }
            Some("assistant") => trend[index].assistant_message_count += 1,
            _ => {}
        }

        if let Some(usage) = event.usage.as_ref().and_then(TokenUsage::normalize) {
            tokens[index].prompt_tokens += usage.prompt_tokens.unwrap_or(0);
            tokens[index].completion_tokens += usage.completion_tokens.unwrap_or(0);
            tokens[index].total_tokens += usage.total();
            // 只有明确标记为 `estimated` 的才计入“估算”；服务方返回的是真实用量。
            if event.usage_source.as_deref() == Some("estimated") {
                tokens[index].estimated_tokens += usage.total();
            }
        }
    }

    for (index, sessions) in sessions_by_bucket {
        trend[index].active_session_count = sessions.len();
    }

    AiTrend { trend, tokens }
}

/// 写作产出趋势。只看 `file_save`——只有保存才代表落到磁盘的产出。
pub fn writing_trend(
    events: &[ActivityEvent],
    bucket_list: &[Bucket],
    granularity: Granularity,
) -> Vec<WritingTrendPoint> {
    let mut points: Vec<WritingTrendPoint> = bucket_list
        .iter()
        .map(|b| WritingTrendPoint {
            key: b.key.clone(),
            label: b.label.clone(),
            words_added: 0,
            words_removed: 0,
            net_words: 0,
            save_count: 0,
            note_count: 0,
        })
        .collect();
    let index_by_key: BTreeMap<String, usize> = bucket_list
        .iter()
        .enumerate()
        .map(|(index, b)| (b.key.clone(), index))
        .collect();
    let mut notes_by_bucket: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();

    for event in events {
        if event.kind != ActivityEventType::FileSave {
            continue;
        }
        let Some(millis) = event.timestamp_millis() else {
            continue;
        };
        let Some(local) = buckets::to_local(millis) else {
            continue;
        };
        let Some(&index) = index_by_key.get(&buckets::bucket_key(local, granularity)) else {
            continue;
        };

        points[index].save_count += 1;
        let delta = event.words_delta_or_zero();
        if delta > 0 {
            points[index].words_added += delta;
        } else if delta < 0 {
            points[index].words_removed += -delta;
        }

        if let Some(path) = event.relative_path.as_deref() {
            notes_by_bucket
                .entry(index)
                .or_default()
                .insert(path.to_owned());
        }
    }

    for (index, point) in points.iter_mut().enumerate() {
        point.net_words = point.words_added - point.words_removed;
        point.note_count = notes_by_bucket.get(&index).map_or(0, BTreeSet::len);
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::buckets::day_buckets;
    use crate::analytics::events::ActivityInput;
    use chrono::{DateTime, Local, TimeZone};

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
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

    fn save(when: DateTime<Local>, path: &str, delta: i64) -> ActivityEvent {
        ActivityEvent {
            relative_path: Some(path.into()),
            words_delta: Some(delta),
            ..event(ActivityEventType::FileSave, when)
        }
    }

    fn ai(
        when: DateTime<Local>,
        session: &str,
        role: &str,
        usage: Option<(i64, i64)>,
        source: &str,
    ) -> ActivityEvent {
        ActivityEvent {
            session_id: Some(session.into()),
            role: Some(role.into()),
            usage: usage.map(|(p, c)| TokenUsage {
                prompt_tokens: Some(p),
                completion_tokens: Some(c),
                total_tokens: Some(p + c),
            }),
            usage_source: usage.map(|_| source.to_owned()),
            ..event(ActivityEventType::AiMessage, when)
        }
    }

    fn note(relative: &str, created: DateTime<Local>, updated: DateTime<Local>) -> ScannedFile {
        ScannedFile {
            absolute_path: format!("D:/ws/{relative}"),
            relative_path: relative.into(),
            extension: format!(".{}", relative.rsplit('.').next().unwrap_or("mc")),
            size: 100,
            created_at: created.timestamp_millis(),
            updated_at: updated.timestamp_millis(),
        }
    }

    /// 活动埋点的字段不该被测试绕过——这里确认 ActivityInput 也能造出等价事件。
    #[test]
    fn the_test_helper_agrees_with_real_normalization() {
        let dir = std::env::temp_dir();
        let real = crate::analytics::events::normalize_event(
            &dir,
            &ActivityInput {
                kind: Some(ActivityEventType::FileSave),
                timestamp: Some(at(2026, 8, 29, 10).timestamp_millis()),
                ..Default::default()
            },
            0,
            "x",
        )
        .unwrap();
        let helper = event(ActivityEventType::FileSave, at(2026, 8, 29, 10));
        assert_eq!(real.timestamp, helper.timestamp);
        assert_eq!(real.kind, helper.kind);
    }

    // ---------- 笔记趋势 ----------

    /// totalCount 是累计存量，不是桶内新增——图上应单调不减。
    #[test]
    fn note_totals_accumulate_while_created_counts_do_not() {
        let now = at(2026, 8, 29, 12);
        let files = vec![
            note("a.mc", at(2026, 8, 27, 9), at(2026, 8, 27, 9)),
            note("b.mc", at(2026, 8, 28, 9), at(2026, 8, 29, 9)),
        ];
        let trend = note_trend(&files, &day_buckets(3, now));

        assert_eq!(
            trend.iter().map(|p| p.created_count).collect::<Vec<_>>(),
            [1, 1, 0]
        );
        assert_eq!(
            trend.iter().map(|p| p.updated_count).collect::<Vec<_>>(),
            [1, 0, 1]
        );
        assert_eq!(
            trend.iter().map(|p| p.total_count).collect::<Vec<_>>(),
            [1, 2, 2],
            "存量累计"
        );
    }

    #[test]
    fn only_note_extensions_count_towards_the_trend() {
        let now = at(2026, 8, 29, 12);
        let files = vec![
            note("a.mc", at(2026, 8, 29, 9), at(2026, 8, 29, 9)),
            note("b.md", at(2026, 8, 29, 9), at(2026, 8, 29, 9)),
            note("c.png", at(2026, 8, 29, 9), at(2026, 8, 29, 9)),
            note("d.json", at(2026, 8, 29, 9), at(2026, 8, 29, 9)),
        ];
        let trend = note_trend(&files, &day_buckets(1, now));
        assert_eq!(trend[0].created_count, 2, "只有 .mc/.md 算笔记");
    }

    // ---------- 活动热力 ----------

    #[test]
    fn contributions_line_up_with_the_day_buckets() {
        let now = at(2026, 8, 29, 12);
        let events = vec![
            event(ActivityEventType::FileSave, at(2026, 8, 29, 10)), // 权重 3
            event(ActivityEventType::HomeView, at(2026, 8, 29, 11)), // 权重 1
            event(ActivityEventType::FileCreate, at(2026, 8, 28, 9)), // 权重 4
        ];
        let summary = activity_summary(&events, &day_buckets(3, now));

        assert_eq!(
            summary.contributions.len(),
            3,
            "长度必须等于日桶数，前端按下标画格子"
        );
        assert_eq!(summary.contributions[2].date, "2026-08-29");
        assert_eq!(summary.contributions[2].count, 2);
        assert_eq!(summary.contributions[2].weight, 4, "3 + 1");
        assert_eq!(summary.contributions[1].weight, 4);
        assert_eq!(summary.contributions[0].count, 0);
    }

    /// 范围外的事件被忽略，而不是挤进最近的桶里。
    #[test]
    fn events_outside_the_range_do_not_leak_into_the_nearest_bucket() {
        let now = at(2026, 8, 29, 12);
        let events = vec![event(ActivityEventType::FileSave, at(2020, 1, 1, 10))];
        let summary = activity_summary(&events, &day_buckets(3, now));
        assert!(summary.contributions.iter().all(|p| p.count == 0));
        // 但作息热力没有范围概念，仍会记上
        assert_eq!(summary.hourly.iter().map(|p| p.count).sum::<i64>(), 1);
    }

    #[test]
    fn hourly_points_are_a_full_seven_by_twenty_four_grid() {
        let points = empty_hourly_points();
        assert_eq!(points.len(), 168);
        assert_eq!((points[0].day, points[0].hour), (0, 0));
        assert_eq!((points[23].day, points[23].hour), (0, 23));
        assert_eq!((points[24].day, points[24].hour), (1, 0));
        assert_eq!((points[167].day, points[167].hour), (6, 23));
    }

    #[test]
    fn hourly_slots_use_local_weekday_and_hour() {
        // 2026-08-29 是周六 → day 6
        let summary = activity_summary(
            &[event(ActivityEventType::FileSave, at(2026, 8, 29, 14))],
            &day_buckets(1, at(2026, 8, 29, 23)),
        );
        let hit: Vec<_> = summary.hourly.iter().filter(|p| p.count > 0).collect();
        assert_eq!(hit.len(), 1);
        assert_eq!((hit[0].day, hit[0].hour), (6, 14));
        assert_eq!(hit[0].weight, 3);
    }

    // ---------- AI 趋势 ----------

    /// 对话轮数跟用户消息走，不跟助手消息——否则一次追问会被算成两轮。
    #[test]
    fn conversation_count_follows_user_messages_only() {
        let now = at(2026, 8, 29, 12);
        let events = vec![
            ai(at(2026, 8, 29, 10), "s1", "user", None, ""),
            ai(at(2026, 8, 29, 10), "s1", "assistant", None, ""),
            ai(at(2026, 8, 29, 11), "s1", "user", None, ""),
            ai(at(2026, 8, 29, 11), "s1", "assistant", None, ""),
        ];
        let result = ai_trend(&events, &day_buckets(1, now), Granularity::Day);

        assert_eq!(result.trend[0].user_message_count, 2);
        assert_eq!(result.trend[0].assistant_message_count, 2);
        assert_eq!(result.trend[0].conversation_count, 2, "两轮，不是四轮");
    }

    #[test]
    fn active_sessions_are_deduplicated_per_bucket() {
        let now = at(2026, 8, 29, 12);
        let events = vec![
            ai(at(2026, 8, 29, 9), "s1", "user", None, ""),
            ai(at(2026, 8, 29, 10), "s1", "user", None, ""),
            ai(at(2026, 8, 29, 11), "s2", "user", None, ""),
        ];
        let result = ai_trend(&events, &day_buckets(1, now), Granularity::Day);
        assert_eq!(result.trend[0].active_session_count, 2, "同一会话只算一次");
    }

    /// 只有标记为 `estimated` 的才计入估算列；服务方返回的是真实用量。
    #[test]
    fn estimated_tokens_are_tracked_separately_from_real_ones() {
        let now = at(2026, 8, 29, 12);
        let events = vec![
            ai(at(2026, 8, 29, 9), "s1", "user", Some((100, 0)), "provider"),
            ai(
                at(2026, 8, 29, 10),
                "s1",
                "assistant",
                Some((0, 50)),
                "estimated",
            ),
        ];
        let result = ai_trend(&events, &day_buckets(1, now), Granularity::Day);

        assert_eq!(result.tokens[0].prompt_tokens, 100);
        assert_eq!(result.tokens[0].completion_tokens, 50);
        assert_eq!(result.tokens[0].total_tokens, 150);
        assert_eq!(
            result.tokens[0].estimated_tokens, 50,
            "只有 estimated 那条计入"
        );
    }

    #[test]
    fn non_ai_events_are_ignored_by_the_ai_trend() {
        let now = at(2026, 8, 29, 12);
        let events = vec![save(at(2026, 8, 29, 10), "a.mc", 50)];
        let result = ai_trend(&events, &day_buckets(1, now), Granularity::Day);
        assert_eq!(result.trend[0].conversation_count, 0);
        assert_eq!(result.tokens[0].total_tokens, 0);
    }

    /// session_open 只贡献活跃会话数，不贡献消息数。
    #[test]
    fn opening_a_session_counts_as_activity_but_not_as_a_message() {
        let now = at(2026, 8, 29, 12);
        let events = vec![ActivityEvent {
            session_id: Some("s9".into()),
            ..event(ActivityEventType::AiSessionOpen, at(2026, 8, 29, 10))
        }];
        let result = ai_trend(&events, &day_buckets(1, now), Granularity::Day);
        assert_eq!(result.trend[0].active_session_count, 1);
        assert_eq!(result.trend[0].user_message_count, 0);
        assert_eq!(result.trend[0].conversation_count, 0);
    }

    #[test]
    fn ai_events_roll_up_into_week_and_month_buckets() {
        let now = at(2026, 8, 29, 12);
        let days = day_buckets(30, now);
        let events = vec![
            ai(at(2026, 8, 10, 9), "s1", "user", None, ""),
            ai(at(2026, 8, 29, 9), "s2", "user", None, ""),
        ];

        let weekly = ai_trend(&events, &buckets::week_buckets(&days), Granularity::Week);
        assert_eq!(
            weekly
                .trend
                .iter()
                .map(|p| p.conversation_count)
                .sum::<i64>(),
            2
        );
        assert!(weekly.trend.len() > 1, "30 天该横跨多周");

        let monthly = ai_trend(&events, &buckets::month_buckets(&days), Granularity::Month);
        let august = monthly.trend.iter().find(|p| p.key == "2026-08").unwrap();
        assert_eq!(august.conversation_count, 2, "同月的两条应合并");
    }

    // ---------- 写作趋势 ----------

    #[test]
    fn writing_splits_additions_from_deletions() {
        let now = at(2026, 8, 29, 12);
        let events = vec![
            save(at(2026, 8, 29, 9), "a.mc", 120),
            save(at(2026, 8, 29, 10), "a.mc", -30),
            save(at(2026, 8, 29, 11), "b.mc", 40),
        ];
        let trend = writing_trend(&events, &day_buckets(1, now), Granularity::Day);

        assert_eq!(trend[0].words_added, 160);
        assert_eq!(trend[0].words_removed, 30, "删除记正数");
        assert_eq!(trend[0].net_words, 130);
        assert_eq!(trend[0].save_count, 3);
        assert_eq!(trend[0].note_count, 2, "同一篇多次保存只算一篇");
    }

    #[test]
    fn only_saves_count_towards_writing() {
        let now = at(2026, 8, 29, 12);
        let events = vec![
            event(ActivityEventType::FileOpen, at(2026, 8, 29, 9)),
            event(ActivityEventType::FileCreate, at(2026, 8, 29, 10)),
        ];
        let trend = writing_trend(&events, &day_buckets(1, now), Granularity::Day);
        assert_eq!(trend[0].save_count, 0, "打开和新建都不算产出，只有保存算");
    }

    #[test]
    fn every_bucket_appears_even_with_no_activity() {
        let now = at(2026, 8, 29, 12);
        let trend = writing_trend(&[], &day_buckets(5, now), Granularity::Day);
        assert_eq!(trend.len(), 5, "空桶也要在，否则图表会缺格");
        assert!(trend.iter().all(|p| p.save_count == 0 && p.note_count == 0));
    }
}
