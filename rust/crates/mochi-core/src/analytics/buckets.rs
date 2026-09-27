//! 时间戳存 UTC，分桶前转成本地时间；不要换成 getUTC* 语义。

use chrono::{DateTime, Datelike, Local, TimeZone};

/// 一个时间桶。`start` / `end` 是闭区间（`end` 是该桶最后一毫秒）。
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    /// 分组键：日 `2026-08-29`、周取周一那天的日键、月 `2026-08`。
    pub key: String,
    /// 展示标签：日/周 `8/29`，月 `2026.8`。**都不补零**，与上游一致。
    pub label: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Day,
    Week,
    Month,
}

pub fn start_of_day(date: DateTime<Local>) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .earliest()
        // 极少数时区在午夜有 DST 跳变，那一天没有 00:00:00。退回原时刻好过 panic。
        .unwrap_or(date)
}

pub fn end_of_day(date: DateTime<Local>) -> DateTime<Local> {
    start_of_day(date) + chrono::Duration::days(1) - chrono::Duration::milliseconds(1)
}

pub fn date_key(date: DateTime<Local>) -> String {
    format!("{}-{:02}-{:02}", date.year(), date.month(), date.day())
}

pub fn month_key(date: DateTime<Local>) -> String {
    format!("{}-{:02}", date.year(), date.month())
}

/// 周一为一周之首。JS 的 `getDay()` 里 0 是周日，所以周日要往回退 6 天。
pub fn week_start(date: DateTime<Local>) -> DateTime<Local> {
    let start = start_of_day(date);
    // chrono 的 num_days_from_sunday() 与 JS 的 getDay() 同义
    let weekday = start.weekday().num_days_from_sunday() as i64;
    let diff = if weekday == 0 { -6 } else { 1 - weekday };
    start + chrono::Duration::days(diff)
}

/// `8/29`——**不补零**。
pub fn label_for_date(date: DateTime<Local>) -> String {
    format!("{}/{}", date.month(), date.day())
}

/// `2026.8`——**不补零**。
pub fn label_for_month(date: DateTime<Local>) -> String {
    format!("{}.{}", date.year(), date.month())
}

/// 天数范围钳位：7 ~ 730 天，缺省 365。
pub fn clamp_range_days(value: Option<f64>) -> i64 {
    let Some(value) = value.filter(|v| v.is_finite()) else {
        return 365;
    };
    // TS 是 `Math.round(value || 365)`：0 会落回 365
    let rounded = if value == 0.0 {
        365
    } else {
        super::events::js_round(value)
    };
    rounded.clamp(7, 730)
}

/// 最近 `range_days` 天，每天一个桶，升序。
pub fn day_buckets(range_days: i64, now: DateTime<Local>) -> Vec<Bucket> {
    let today = start_of_day(now);
    let start = today - chrono::Duration::days(range_days - 1);

    (0..range_days)
        .map(|index| {
            let day = start + chrono::Duration::days(index);
            Bucket {
                key: date_key(day),
                label: label_for_date(day),
                start: day,
                end: end_of_day(day),
            }
        })
        .collect()
}

/// 把日桶折叠成周桶。同一周只留一个，按开始时间升序。
pub fn week_buckets(days: &[Bucket]) -> Vec<Bucket> {
    fold(days, |day| {
        let start = week_start(day.start);
        Bucket {
            key: date_key(start),
            label: label_for_date(start),
            start,
            end: start + chrono::Duration::days(7) - chrono::Duration::milliseconds(1),
        }
    })
}

/// 把日桶折叠成月桶。
pub fn month_buckets(days: &[Bucket]) -> Vec<Bucket> {
    fold(days, |day| {
        let start = Local
            .with_ymd_and_hms(day.start.year(), day.start.month(), 1, 0, 0, 0)
            .earliest()
            .unwrap_or(day.start);
        let next = if start.month() == 12 {
            Local.with_ymd_and_hms(start.year() + 1, 1, 1, 0, 0, 0)
        } else {
            Local.with_ymd_and_hms(start.year(), start.month() + 1, 1, 0, 0, 0)
        }
        .earliest()
        .unwrap_or(start);

        Bucket {
            key: month_key(start),
            label: label_for_month(start),
            start,
            end: next - chrono::Duration::milliseconds(1),
        }
    })
}

fn fold(days: &[Bucket], to_bucket: impl Fn(&Bucket) -> Bucket) -> Vec<Bucket> {
    let mut seen: Vec<Bucket> = Vec::new();
    for day in days {
        let bucket = to_bucket(day);
        if !seen.iter().any(|b| b.key == bucket.key) {
            seen.push(bucket);
        }
    }
    seen.sort_by_key(|b| b.start);
    seen
}

/// 某个时刻属于哪个桶的键。
pub fn bucket_key(date: DateTime<Local>, granularity: Granularity) -> String {
    match granularity {
        Granularity::Day => date_key(date),
        Granularity::Week => date_key(week_start(date)),
        Granularity::Month => month_key(date),
    }
}

/// 把 UTC 毫秒转成本地时刻。
pub fn to_local(millis: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp_millis(millis).map(|utc| utc.with_timezone(&Local))
}

/// 日键（`2026-08-29`）转回当天零点。用于把摘要的日期字符串放回时间轴上。
pub fn parse_date_key(key: &str) -> Option<DateTime<Local>> {
    let mut parts = key.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    Local.with_ymd_and_hms(year, month, day, 0, 0, 0).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;
    // 只有测试要读时分，正文用不到
    use chrono::Timelike;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 30, 0).unwrap()
    }

    // ---------- 日期键与标签 ----------

    /// 键补零、标签不补零——这两个规则不一样，别顺手统一。
    #[test]
    fn keys_are_padded_but_labels_are_not() {
        let day = at(2026, 8, 5, 12);
        assert_eq!(date_key(day), "2026-08-05");
        assert_eq!(month_key(day), "2026-08");
        assert_eq!(label_for_date(day), "8/5");
        assert_eq!(label_for_month(day), "2026.8");
    }

    #[test]
    fn day_boundaries_span_the_full_local_day() {
        let day = at(2026, 8, 29, 15);
        assert_eq!(date_key(start_of_day(day)), "2026-08-29");
        assert_eq!(start_of_day(day).hour(), 0);
        assert_eq!(end_of_day(day).hour(), 23);
        assert_eq!(end_of_day(day).minute(), 59);
        assert_eq!(
            end_of_day(day).timestamp_millis() - start_of_day(day).timestamp_millis(),
            86_400_000 - 1
        );
    }

    /// 周一为首。周日要退 6 天，不是往前 1 天。
    #[test]
    fn weeks_start_on_monday() {
        // 2026-08-29 是周六，2026-08-30 是周日
        assert_eq!(date_key(week_start(at(2026, 8, 29, 12))), "2026-08-24");
        assert_eq!(
            date_key(week_start(at(2026, 8, 30, 12))),
            "2026-08-24",
            "周日属于上一周"
        );
        assert_eq!(
            date_key(week_start(at(2026, 8, 31, 12))),
            "2026-08-31",
            "周一是新一周"
        );
        assert_eq!(
            date_key(week_start(at(2026, 8, 24, 12))),
            "2026-08-24",
            "周一取自身"
        );
    }

    // ---------- 范围钳位 ----------

    #[test]
    fn range_days_are_clamped_with_a_365_default() {
        assert_eq!(clamp_range_days(None), 365);
        assert_eq!(clamp_range_days(Some(f64::NAN)), 365);
        assert_eq!(clamp_range_days(Some(0.0)), 365, "0 落回默认值");
        assert_eq!(clamp_range_days(Some(1.0)), 7, "下限 7");
        assert_eq!(clamp_range_days(Some(10_000.0)), 730, "上限 730");
        assert_eq!(clamp_range_days(Some(30.4)), 30, "先四舍五入");
        assert_eq!(clamp_range_days(Some(-5.0)), 7);
    }

    // ---------- 分桶 ----------

    #[test]
    fn day_buckets_end_on_today_and_are_ascending() {
        let now = at(2026, 8, 29, 15);
        let buckets = day_buckets(7, now);

        assert_eq!(buckets.len(), 7);
        assert_eq!(buckets[0].key, "2026-08-23");
        assert_eq!(buckets[6].key, "2026-08-29", "最后一个桶是今天");
        assert_eq!(buckets[6].label, "8/29");
        assert!(buckets.windows(2).all(|w| w[0].start < w[1].start));
        // 相邻桶首尾相接，中间不留缝
        assert_eq!(
            buckets[0].end.timestamp_millis() + 1,
            buckets[1].start.timestamp_millis()
        );
    }

    #[test]
    fn week_buckets_collapse_days_into_iso_weeks() {
        // 8/23（周日）~ 8/29（周六）跨两个「周一起始」的周
        let weeks = week_buckets(&day_buckets(7, at(2026, 8, 29, 15)));
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks[0].key, "2026-08-17", "8/23 是周日，属于 8/17 那一周");
        assert_eq!(weeks[1].key, "2026-08-24");
        assert_eq!(
            weeks[0].end.timestamp_millis() - weeks[0].start.timestamp_millis(),
            7 * 86_400_000 - 1,
            "周桶横跨整整 7 天"
        );
    }

    #[test]
    fn month_buckets_collapse_days_into_calendar_months() {
        let months = month_buckets(&day_buckets(40, at(2026, 8, 29, 15)));
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].key, "2026-07");
        assert_eq!(months[0].label, "2026.7");
        assert_eq!(months[1].key, "2026-08");
        assert_eq!(
            date_key(months[0].end),
            "2026-07-31",
            "月桶止于当月最后一天"
        );
    }

    /// 跨年时月桶要正确进位。
    #[test]
    fn month_buckets_roll_over_the_year() {
        let months = month_buckets(&day_buckets(45, at(2027, 1, 15, 12)));
        assert_eq!(
            months.iter().map(|b| b.key.as_str()).collect::<Vec<_>>(),
            ["2026-12", "2027-01"]
        );
        assert_eq!(date_key(months[0].end), "2026-12-31");
    }

    #[test]
    fn bucket_keys_agree_with_the_bucket_lists() {
        let day = at(2026, 8, 29, 15);
        assert_eq!(bucket_key(day, Granularity::Day), "2026-08-29");
        assert_eq!(bucket_key(day, Granularity::Week), "2026-08-24");
        assert_eq!(bucket_key(day, Granularity::Month), "2026-08");

        // 每个日桶的周/月键都必须能在对应的桶列表里找到，否则事件会被静默丢弃
        let days = day_buckets(60, day);
        let weeks = week_buckets(&days);
        let months = month_buckets(&days);
        for bucket in &days {
            let week = bucket_key(bucket.start, Granularity::Week);
            let month = bucket_key(bucket.start, Granularity::Month);
            assert!(weeks.iter().any(|b| b.key == week), "周桶漏了 {week}");
            assert!(months.iter().any(|b| b.key == month), "月桶漏了 {month}");
        }
    }

    #[test]
    fn date_keys_round_trip() {
        let day = start_of_day(at(2026, 8, 5, 15));
        assert_eq!(parse_date_key(&date_key(day)), Some(day));
        assert_eq!(parse_date_key("坏数据"), None);
        assert_eq!(parse_date_key("2026-13-99"), None);
    }

    #[test]
    fn local_conversion_round_trips_through_millis() {
        let day = at(2026, 8, 29, 15);
        assert_eq!(to_local(day.timestamp_millis()), Some(day));
    }
}
