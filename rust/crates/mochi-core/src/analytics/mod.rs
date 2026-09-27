//! 只有 events 是落盘数据；趋势和摘要按需计算，不持久化。

pub mod buckets;
pub mod events;
pub mod graph;
pub mod summary;
pub mod trends;

use std::path::Path;

use serde::Serialize;

pub use events::{
    read_events, record, record_input, ActivityEvent, ActivityEventType, ActivityInput, TokenUsage,
};
pub use graph::{FileTypeStat, GraphHealth, GraphNodeStat, ScannedFile};
pub use summary::{
    Chronotype, DaySummary, PersonaSummary, StreakSummary, TodaySummary, TouchedNoteStat,
    WorkAreaStat,
};
pub use trends::{
    AiTrendPoint, DailyActivityPoint, HourlyActivityPoint, TokenTrendPoint, TrendPoint,
    WritingTrendPoint,
};

use buckets::Granularity;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteTrends {
    pub day: Vec<TrendPoint>,
    pub week: Vec<TrendPoint>,
    pub month: Vec<TrendPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySection {
    pub contributions: Vec<DailyActivityPoint>,
    pub hourly: Vec<HourlyActivityPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTrends {
    pub day: Vec<TokenTrendPoint>,
    pub week: Vec<TokenTrendPoint>,
    pub month: Vec<TokenTrendPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSection {
    pub day: Vec<AiTrendPoint>,
    pub week: Vec<AiTrendPoint>,
    pub month: Vec<AiTrendPoint>,
    pub tokens: TokenTrends,
    pub total_sessions: usize,
    pub total_messages: i64,
    pub total_tokens: i64,
    pub estimated_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesSection {
    pub total_notes: usize,
    pub total_files: usize,
    pub total_size: u64,
    pub total_words: usize,
    pub file_types: Vec<FileTypeStat>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WritingSection {
    pub day: Vec<WritingTrendPoint>,
    pub week: Vec<WritingTrendPoint>,
    pub month: Vec<WritingTrendPoint>,
    pub total_words_added: i64,
}

/// 首页要用的全部指标。**字段顺序对齐 TS 的 `HomeAnalyticsSnapshot`。**
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeAnalytics {
    pub generated_at: String,
    pub range_days: i64,
    pub note_trends: NoteTrends,
    pub activity: ActivitySection,
    pub ai: AiSection,
    pub files: FilesSection,
    pub graph: GraphHealth,
    pub writing: WritingSection,
    pub today: TodaySummary,
    pub streak: StreakSummary,
    pub persona: PersonaSummary,
}

/// 空快照。工作区不可读时返回它，而不是报错——首页宁可显示一片空白，
/// 也不该因为统计失败而打不开。
pub fn empty_snapshot(range_days: i64, now: chrono::DateTime<chrono::Local>) -> HomeAnalytics {
    let today_key = buckets::date_key(now);
    HomeAnalytics {
        generated_at: crate::jstime::from_millis(now.timestamp_millis())
            .unwrap_or_else(crate::jstime::now),
        range_days,
        note_trends: NoteTrends {
            day: Vec::new(),
            week: Vec::new(),
            month: Vec::new(),
        },
        activity: ActivitySection {
            contributions: Vec::new(),
            hourly: trends::empty_hourly_points(),
        },
        ai: AiSection {
            day: Vec::new(),
            week: Vec::new(),
            month: Vec::new(),
            tokens: TokenTrends {
                day: Vec::new(),
                week: Vec::new(),
                month: Vec::new(),
            },
            total_sessions: 0,
            total_messages: 0,
            total_tokens: 0,
            estimated_tokens: 0,
        },
        files: FilesSection {
            total_notes: 0,
            total_files: 0,
            total_size: 0,
            total_words: 0,
            file_types: Vec::new(),
        },
        graph: GraphHealth::default(),
        writing: WritingSection {
            day: Vec::new(),
            week: Vec::new(),
            month: Vec::new(),
            total_words_added: 0,
        },
        today: TodaySummary::empty(&today_key),
        streak: StreakSummary::default(),
        persona: PersonaSummary::default(),
    }
}

/// 从已存的 AI 会话文件里**补造** `ai_message` 事件。
///
/// 只在活动日志里一条 `ai_message` 都没有时才走这条路：埋点是后加的，
/// 老用户的历史对话全在会话文件里而不在日志里，不补造的话 AI 面板会显示"从没用过"。
/// 一旦有了真实埋点就完全不用它——两者混用会重复计数。
fn backfill_ai_events(
    workspace: &Path,
    start: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> Vec<ActivityEvent> {
    let service = crate::ai::AiSessionService::new(workspace);
    let index = service.load_index();
    let (start_ms, now_ms) = (start.timestamp_millis(), now.timestamp_millis());
    let mut out = Vec::new();

    for meta in &index.sessions {
        let Some(conversation) = service.load_session(&meta.id) else {
            continue;
        };

        for message in &conversation.messages {
            let role = message.role();
            if role != "user" && role != "assistant" {
                continue;
            }
            let millis = message
                .timestamp()
                .or(Some(meta.updated_at))
                .filter(|ms| *ms > 0)
                .unwrap_or(meta.created_at);
            if millis < start_ms || millis > now_ms {
                continue;
            }

            // 消息自带用量就用它；没有就按内容估算，并如实标成 estimated
            let stored_usage = message
                .get("usage")
                .and_then(|v| serde_json::from_value::<TokenUsage>(v.clone()).ok());
            let estimated = crate::ai::estimate_tokens(message.content()) as i64;
            let usage = stored_usage.unwrap_or(if role == "user" {
                TokenUsage {
                    prompt_tokens: Some(estimated),
                    completion_tokens: None,
                    total_tokens: Some(estimated),
                }
            } else {
                TokenUsage {
                    prompt_tokens: None,
                    completion_tokens: Some(estimated),
                    total_tokens: Some(estimated),
                }
            });
            let usage_source = if message.get("usage").is_some() {
                message
                    .get("usageSource")
                    .and_then(|v| v.as_str())
                    .unwrap_or("provider")
                    .to_owned()
            } else {
                "estimated".to_owned()
            };

            let usage = TokenUsage::normalize(&usage);
            out.push(ActivityEvent {
                version: 1,
                id: format!("backfill-{}-{millis}-{}", meta.id, out.len()),
                kind: ActivityEventType::AiMessage,
                timestamp: crate::jstime::from_millis(millis).unwrap_or_else(crate::jstime::now),
                relative_path: None,
                extension: None,
                session_id: Some(meta.id.clone()),
                agent_id: None,
                role: Some(role.to_owned()),
                model: None,
                provider_id: None,
                usage_source: usage.as_ref().map(|_| usage_source),
                usage,
                words: None,
                words_delta: None,
                duration_minutes: None,
                count: None,
            });
        }
    }
    out
}

/// 生成首页快照。
///
/// `now` 为分桶计算的基准时间；显式传入可使统计结果可复现。
pub fn build(
    workspace: &Path,
    range_days: Option<f64>,
    now: chrono::DateTime<chrono::Local>,
) -> HomeAnalytics {
    let range_days = buckets::clamp_range_days(range_days);
    if !workspace.is_dir() {
        return empty_snapshot(range_days, now);
    }

    let day_buckets = buckets::day_buckets(range_days, now);
    let week_buckets = buckets::week_buckets(&day_buckets);
    let month_buckets = buckets::month_buckets(&day_buckets);
    let start = day_buckets.first().map_or(now, |b| b.start);

    let files = graph::scan_workspace(workspace);
    let events = read_events(workspace, start, now);

    // 有真实埋点就只用埋点，一条都没有才回退到补造，两者绝不混用
    let ai_events: Vec<ActivityEvent> = if events
        .iter()
        .any(|e| e.kind == ActivityEventType::AiMessage)
    {
        events
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    ActivityEventType::AiMessage | ActivityEventType::AiSessionOpen
                )
            })
            .cloned()
            .collect()
    } else {
        backfill_ai_events(workspace, start, now)
    };

    let day_ai = trends::ai_trend(&ai_events, &day_buckets, Granularity::Day);
    let week_ai = trends::ai_trend(&ai_events, &week_buckets, Granularity::Week);
    let month_ai = trends::ai_trend(&ai_events, &month_buckets, Granularity::Month);

    let session_index = crate::ai::AiSessionService::new(workspace).load_index();
    let content = graph::build_graph_health(workspace, &files);
    let activity = trends::activity_summary(&events, &day_buckets);

    let groups = summary::group_by_date(&events);
    let summaries = summary::summaries_by_date(&groups);
    let events_by_date = summary::events_by_date(&groups);

    let writing_day = trends::writing_trend(&events, &day_buckets, Granularity::Day);
    let total_words_added: i64 = writing_day.iter().map(|p| p.words_added).sum();

    HomeAnalytics {
        generated_at: crate::jstime::from_millis(now.timestamp_millis())
            .unwrap_or_else(crate::jstime::now),
        range_days,
        note_trends: NoteTrends {
            day: trends::note_trend(&files, &day_buckets),
            week: trends::note_trend(&files, &week_buckets),
            month: trends::note_trend(&files, &month_buckets),
        },
        ai: AiSection {
            day: day_ai.trend,
            week: week_ai.trend,
            month: month_ai.trend,
            // 总计只从**日桶**汇总。周/月桶覆盖的时间范围更宽（一周可能跨出 range），
            // 拿它们求和会把范围外的量算进来。
            total_tokens: day_ai.tokens.iter().map(|p| p.total_tokens).sum(),
            estimated_tokens: day_ai.tokens.iter().map(|p| p.estimated_tokens).sum(),
            tokens: TokenTrends {
                day: day_ai.tokens,
                week: week_ai.tokens,
                month: month_ai.tokens,
            },
            total_sessions: session_index.sessions.len(),
            total_messages: session_index
                .sessions
                .iter()
                .map(|s| s.message_count as i64)
                .sum(),
        },
        files: FilesSection {
            total_notes: files.iter().filter(|f| f.is_note()).count(),
            total_files: files.len(),
            total_size: files.iter().map(|f| f.size).sum(),
            total_words: content.total_words,
            file_types: graph::file_type_stats(&files),
        },
        graph: content.graph,
        writing: WritingSection {
            day: writing_day,
            week: trends::writing_trend(&events, &week_buckets, Granularity::Week),
            month: trends::writing_trend(&events, &month_buckets, Granularity::Month),
            total_words_added,
        },
        today: summary::today_summary(&summaries, &events_by_date, now),
        streak: summary::streak_summary(&activity.contributions, &summaries, now),
        persona: summary::persona(&events, &summaries, &activity.hourly),
        activity: ActivitySection {
            contributions: activity.contributions,
            hourly: activity.hourly,
        },
    }
}

/// 快照里各段的键，供测试与调用方核对形状。
pub fn snapshot_sections() -> &'static [&'static str] {
    &[
        "generatedAt",
        "rangeDays",
        "noteTrends",
        "activity",
        "ai",
        "files",
        "graph",
        "writing",
        "today",
        "streak",
        "persona",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Ws(PathBuf);
    impl Drop for Ws {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Ws {
        fn write(&self, relative: &str, body: &str) -> &Self {
            let target = self.0.join(relative);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, body).unwrap();
            self
        }
        fn log(&self, event: &ActivityEvent) -> &Self {
            assert!(record(&self.0, event), "写入活动日志失败");
            self
        }
    }

    fn ws(tag: &str) -> Ws {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-home-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Ws(root)
    }

    fn at(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn event(kind: ActivityEventType, when: chrono::DateTime<chrono::Local>) -> ActivityEvent {
        ActivityEvent {
            version: 1,
            id: format!("{}-{:?}", when.timestamp_millis(), kind),
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

    fn save(when: chrono::DateTime<chrono::Local>, path: &str, delta: i64) -> ActivityEvent {
        ActivityEvent {
            relative_path: Some(path.into()),
            extension: Some(".mc".into()),
            words_delta: Some(delta),
            ..event(ActivityEventType::FileSave, when)
        }
    }

    // ---------- 形状 ----------

    #[test]
    fn the_snapshot_has_every_section_the_ts_interface_declares() {
        let now = at(2026, 8, 29, 15);
        let json = serde_json::to_value(empty_snapshot(30, now)).unwrap();
        let object = json.as_object().unwrap();

        for key in snapshot_sections() {
            assert!(object.contains_key(*key), "快照缺少 {key} 段");
        }
        assert_eq!(
            object.len(),
            snapshot_sections().len(),
            "多了没声明的段: {:?}",
            object.keys()
        );
    }

    #[test]
    fn an_unreadable_workspace_yields_an_empty_snapshot_not_an_error() {
        let missing = std::env::temp_dir().join("mochi-home-绝不存在");
        let _ = std::fs::remove_dir_all(&missing);
        let now = at(2026, 8, 29, 15);

        let snapshot = build(&missing, Some(30.0), now);
        assert_eq!(snapshot, empty_snapshot(30, now));
        assert_eq!(snapshot.range_days, 30);
        assert_eq!(
            snapshot.activity.hourly.len(),
            168,
            "空快照也要给全 7×24 网格"
        );
    }

    #[test]
    fn the_range_is_clamped_before_anything_else_runs() {
        let w = ws("range");
        let now = at(2026, 8, 29, 15);
        assert_eq!(
            build(&w.0, Some(1.0), now).note_trends.day.len(),
            7,
            "下限 7"
        );
        assert_eq!(build(&w.0, Some(9999.0), now).range_days, 730);
        assert_eq!(build(&w.0, None, now).range_days, 365);
    }

    // ---------- 端到端 ----------

    #[test]
    fn a_real_workspace_produces_a_coherent_snapshot() {
        let w = ws("e2e");
        let now = at(2026, 8, 29, 20);
        w.write(
            "知识库/数学/概率.mc",
            "---\ntags: [数学]\n---\n概率论笔记，见 [[统计]]",
        )
        .write("知识库/数学/统计.mc", "统计学笔记 hello world")
        .write("知识库/物理/力学.mc", "力学笔记");

        w.log(&save(at(2026, 8, 29, 9), "知识库/数学/概率.mc", 120))
            .log(&save(at(2026, 8, 29, 10), "知识库/数学/统计.mc", 80))
            .log(&save(at(2026, 8, 28, 9), "知识库/物理/力学.mc", 50))
            .log(&event(ActivityEventType::Search, at(2026, 8, 29, 11)));

        let s = build(&w.0, Some(30.0), now);

        assert_eq!(s.files.total_notes, 3);
        assert_eq!(s.files.total_files, 3);
        assert!(s.files.total_words > 0);
        assert_eq!(s.graph.note_count, 3);
        assert_eq!(s.graph.local_link_count, 1, "概率 → 统计");
        assert_eq!(s.graph.isolated_note_count, 1, "力学没有链接");

        assert_eq!(s.writing.total_words_added, 250);
        assert_eq!(s.today.day.words_added, 200, "今天写了 120 + 80");
        assert_eq!(s.today.day.save_count, 2);
        assert_eq!(s.today.day.searches, 1);
        assert_eq!(s.today.previous.words_added, 50, "昨天写了 50");
        assert_eq!(s.today.areas[0].name, "数学");
        assert_eq!(s.today.top_notes[0].path, "知识库/数学/概率.mc");

        assert_eq!(s.streak.current, 2, "昨天和今天都活跃");
        assert_eq!(s.streak.this_week_words, 250, "8/28 和 8/29 同属本周");

        assert_eq!(s.activity.contributions.len(), 30, "长度等于 rangeDays");
        assert_eq!(s.activity.contributions.last().unwrap().date, "2026-08-29");
        assert_eq!(s.note_trends.day.len(), 30);
        assert!(!s.note_trends.week.is_empty() && !s.note_trends.month.is_empty());
    }

    /// 总量只从日桶汇总——周/月桶会覆盖到 range 之外，拿它们求和会多算。
    #[test]
    fn totals_come_from_day_buckets_only() {
        let w = ws("totals");
        let now = at(2026, 8, 29, 20);
        let ai_event = |when, session: &str, tokens: i64| ActivityEvent {
            session_id: Some(session.into()),
            role: Some("user".into()),
            usage: Some(TokenUsage {
                prompt_tokens: Some(tokens),
                completion_tokens: Some(0),
                total_tokens: Some(tokens),
            }),
            usage_source: Some("provider".into()),
            ..event(ActivityEventType::AiMessage, when)
        };
        w.log(&ai_event(at(2026, 8, 29, 9), "s1", 100))
            .log(&ai_event(at(2026, 8, 28, 9), "s1", 50));

        let s = build(&w.0, Some(7.0), now);
        assert_eq!(s.ai.total_tokens, 150);
        assert_eq!(
            s.ai.day.iter().map(|p| p.conversation_count).sum::<i64>(),
            2
        );
        // 周桶的总量可能更大（周一到周日超出 7 天窗口），所以不能用它当总计
        let week_total: i64 = s.ai.tokens.week.iter().map(|p| p.total_tokens).sum();
        assert!(week_total >= s.ai.total_tokens);
    }

    // ---------- AI 补造 ----------

    /// 埋点是后加的，老用户的历史对话只在会话文件里。不补造的话 AI 面板会显示"从没用过"。
    #[test]
    fn ai_history_is_backfilled_when_the_activity_log_has_none() {
        let w = ws("backfill");
        let now = at(2026, 8, 29, 20);
        let when = at(2026, 8, 28, 14).timestamp_millis();

        let service = crate::ai::AiSessionService::new(&w.0);
        service.initialize().unwrap();
        let mut index = service.load_index();
        index.sessions.push(crate::ai::AiSessionMeta {
            id: "conv-1".into(),
            title: "旧对话".into(),
            created_at: when,
            updated_at: when,
            message_count: 2,
            ..Default::default()
        });
        service.save_index(&index).unwrap();

        let mut conversation = crate::ai::AiConversation {
            id: "conv-1".into(),
            ..Default::default()
        };
        for (role, text) in [
            ("user", "你好，帮我看看这道题"),
            ("assistant", "好的，这道题的思路是"),
        ] {
            let mut message = crate::ai::AiStoredMessage::new(role, text);
            message.set("timestamp", serde_json::json!(when));
            conversation.messages.push(message);
        }
        service.save_session(&conversation).unwrap();

        let s = build(&w.0, Some(30.0), now);
        assert_eq!(
            s.ai.day.iter().map(|p| p.user_message_count).sum::<i64>(),
            1
        );
        assert_eq!(
            s.ai.day
                .iter()
                .map(|p| p.assistant_message_count)
                .sum::<i64>(),
            1
        );
        assert!(s.ai.total_tokens > 0, "补造的消息要按内容估算 token");
        assert_eq!(
            s.ai.total_tokens, s.ai.estimated_tokens,
            "补造的一律标成 estimated"
        );
        assert_eq!(s.ai.total_sessions, 1);
        assert_eq!(s.ai.total_messages, 2, "总消息数取自会话索引");
    }

    /// 一旦有真实埋点就不再补造——两者混用会重复计数。
    #[test]
    fn real_telemetry_suppresses_the_backfill() {
        let w = ws("no-backfill");
        let now = at(2026, 8, 29, 20);
        let when = at(2026, 8, 28, 14).timestamp_millis();

        let service = crate::ai::AiSessionService::new(&w.0);
        service.initialize().unwrap();
        let mut index = service.load_index();
        index.sessions.push(crate::ai::AiSessionMeta {
            id: "conv-1".into(),
            created_at: when,
            updated_at: when,
            message_count: 2,
            ..Default::default()
        });
        service.save_index(&index).unwrap();
        let mut conversation = crate::ai::AiConversation {
            id: "conv-1".into(),
            ..Default::default()
        };
        for role in ["user", "assistant"] {
            let mut message = crate::ai::AiStoredMessage::new(role, "内容");
            message.set("timestamp", serde_json::json!(when));
            conversation.messages.push(message);
        }
        service.save_session(&conversation).unwrap();

        // 只要日志里有一条 ai_message，就完全走日志
        w.log(&ActivityEvent {
            session_id: Some("conv-1".into()),
            role: Some("user".into()),
            ..event(ActivityEventType::AiMessage, at(2026, 8, 29, 9))
        });

        let s = build(&w.0, Some(30.0), now);
        assert_eq!(
            s.ai.day.iter().map(|p| p.user_message_count).sum::<i64>(),
            1,
            "只有埋点那一条，补造的不该叠加进来"
        );
        assert_eq!(
            s.ai.day
                .iter()
                .map(|p| p.assistant_message_count)
                .sum::<i64>(),
            0
        );
    }

    // ---------- 稳定性 ----------

    #[test]
    fn building_twice_gives_the_same_answer() {
        let w = ws("stable");
        let now = at(2026, 8, 29, 20);
        w.write("知识库/a.mc", "内容 [[b]]")
            .write("知识库/b.mc", "内容");
        w.log(&save(at(2026, 8, 29, 9), "知识库/a.mc", 42));

        assert_eq!(build(&w.0, Some(30.0), now), build(&w.0, Some(30.0), now));
    }

    /// 活动日志损坏不该让整张首页失败。
    #[test]
    fn a_corrupt_activity_log_degrades_to_zeroes_not_a_failure() {
        let w = ws("corrupt");
        let now = at(2026, 8, 29, 20);
        w.write("知识库/a.mc", "内容");
        w.write(".mochi/activity/2026-08.jsonl", "{全是坏数据\n还是坏数据\n");

        let s = build(&w.0, Some(30.0), now);
        assert_eq!(s.today.day.event_count, 0);
        assert_eq!(s.files.total_notes, 1, "文件统计不受日志影响");
    }

    #[test]
    fn the_snapshot_serializes_with_camel_case_keys() {
        let w = ws("json");
        let now = at(2026, 8, 29, 20);
        w.log(&save(at(2026, 8, 29, 9), "知识库/a.mc", 10));

        let json = serde_json::to_value(build(&w.0, Some(7.0), now)).unwrap();
        assert!(json["noteTrends"]["day"].is_array());
        assert!(json["activity"]["contributions"].is_array());
        assert!(json["ai"]["totalTokens"].is_number());
        assert!(json["files"]["totalNotes"].is_number());
        assert!(json["graph"]["untaggedNoteRatio"].is_number());
        assert!(json["writing"]["totalWordsAdded"].is_number());
        assert!(json["today"]["wordsAdded"].is_number(), "今日切片要扁平化");
        assert!(json["streak"]["activeDaysInRange"].is_number());
        assert!(json["persona"]["timeOfDayShares"]["morning"].is_number());
    }
}
