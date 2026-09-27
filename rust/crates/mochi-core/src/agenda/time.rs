//! 本地墙钟时间的读写。日程待办的安排类时间全部是不带时区的本地时间字符串，
//! 这样「明早 9 点」换时区或夏令时后仍然是 9 点。

use chrono::{Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

pub const DATE_FMT: &str = "%Y-%m-%d";
pub const STAMP_FMT: &str = "%Y-%m-%dT%H:%M";

pub fn now() -> NaiveDateTime {
    let now = Local::now().naive_local();
    now.with_second(0)
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now)
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

pub fn date_key(date: NaiveDate) -> String {
    date.format(DATE_FMT).to_string()
}

pub fn stamp(at: NaiveDateTime) -> String {
    at.format(STAMP_FMT).to_string()
}

pub fn hm(time: NaiveTime) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

pub fn parse_date(value: &str) -> Option<NaiveDate> {
    let value = value.trim();
    let head = value.get(..10).unwrap_or(value);
    NaiveDate::parse_from_str(head, DATE_FMT).ok()
}

/// 接受 `YYYY-MM-DDTHH:MM[:SS]`、空格分隔形式，也接受带 `Z`/偏移的 ISO（转为本地）。
pub fn parse_stamp(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.with_timezone(&Local).naive_local());
    }
    for fmt in [
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(v) = NaiveDateTime::parse_from_str(value, fmt) {
            return Some(v);
        }
    }
    None
}

pub fn parse_hm(value: &str) -> Option<NaiveTime> {
    let value = value.trim().replace('：', ":");
    NaiveTime::parse_from_str(&value, "%H:%M")
        .ok()
        .or_else(|| NaiveTime::parse_from_str(&value, "%H:%M:%S").ok())
}

/// 截止值可以只有日期；只有日期时视为当天结束才算过期。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Date(NaiveDate),
    At(NaiveDateTime),
}

impl Due {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.len() <= 10 {
            parse_date(value).map(Self::Date)
        } else {
            parse_stamp(value).map(Self::At)
        }
    }

    pub fn date(self) -> NaiveDate {
        match self {
            Self::Date(d) => d,
            Self::At(t) => t.date(),
        }
    }

    /// 截止瞬间（仅日期时取次日零点）。
    pub fn deadline(self) -> NaiveDateTime {
        match self {
            Self::Date(d) => add_days(d, 1).and_time(NaiveTime::MIN),
            Self::At(t) => t,
        }
    }

    pub fn wire(self) -> String {
        match self {
            Self::Date(d) => date_key(d),
            Self::At(t) => stamp(t),
        }
    }
}

/// 把用户或 AI 给的截止值规范化为 `YYYY-MM-DD` 或 `YYYY-MM-DDTHH:MM`。
pub fn normalize_due(value: &str) -> Option<String> {
    Due::parse(value).map(Due::wire)
}

pub fn normalize_stamp(value: &str) -> Option<String> {
    parse_stamp(value).map(stamp)
}

pub fn normalize_date(value: &str) -> Option<String> {
    parse_date(value).map(date_key)
}

pub fn add_days(date: NaiveDate, days: i64) -> NaiveDate {
    if days >= 0 {
        date.checked_add_days(Days::new(days as u64))
            .unwrap_or(date)
    } else {
        date.checked_sub_days(Days::new(days.unsigned_abs()))
            .unwrap_or(date)
    }
}

pub fn week_start(date: NaiveDate) -> NaiveDate {
    add_days(date, -(date.weekday().num_days_from_monday() as i64))
}

pub fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

pub fn add_months(date: NaiveDate, months: i32) -> NaiveDate {
    let total = date.year() * 12 + date.month0() as i32 + months;
    let (year, month0) = (total.div_euclid(12), total.rem_euclid(12) as u32);
    let mut day = date.day();
    loop {
        if let Some(d) = NaiveDate::from_ymd_opt(year, month0 + 1, day) {
            return d;
        }
        if day <= 28 {
            return date;
        }
        day -= 1;
    }
}

pub fn days_in_month(date: NaiveDate) -> u32 {
    let first = month_start(date);
    (add_months(first, 1) - first).num_days() as u32
}

pub fn minutes_between(start: NaiveDateTime, end: NaiveDateTime) -> i64 {
    (end - start).num_minutes()
}

pub fn minute_of_day(at: NaiveDateTime) -> u32 {
    at.hour() * 60 + at.minute()
}

pub fn at_minute(date: NaiveDate, minute: i64) -> NaiveDateTime {
    date.and_time(NaiveTime::MIN) + chrono::Duration::minutes(minute)
}

pub fn weekday_label(date: NaiveDate) -> &'static str {
    ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
        [date.weekday().num_days_from_monday() as usize]
}

/// `9月25日 周四`
pub fn date_label(date: NaiveDate) -> String {
    format!("{}月{}日 {}", date.month(), date.day(), weekday_label(date))
}

/// 相对今天的叫法：今天 / 明天 / 昨天 / 周四 / 9月25日。
pub fn relative_date_label(date: NaiveDate, today: NaiveDate) -> String {
    let diff = (date - today).num_days();
    match diff {
        0 => "今天".into(),
        1 => "明天".into(),
        2 => "后天".into(),
        -1 => "昨天".into(),
        -2 => "前天".into(),
        2..=6 if week_start(date) == week_start(today) => weekday_label(date).into(),
        _ if date.year() == today.year() => format!("{}月{}日", date.month(), date.day()),
        _ => format!("{}年{}月{}日", date.year(), date.month(), date.day()),
    }
}

/// `45 分钟` / `1 小时 30 分` / `2 小时`
pub fn duration_label(minutes: i64) -> String {
    let minutes = minutes.max(0);
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} 分钟"),
        (h, 0) => format!("{h} 小时"),
        (h, m) => format!("{h} 小时 {m} 分"),
    }
}

/// 紧凑版：`45m` / `1.5h` / `2h`
pub fn duration_short(minutes: i64) -> String {
    let minutes = minutes.max(0);
    if minutes < 60 {
        format!("{minutes}m")
    } else if minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{:.1}h", minutes as f64 / 60.0)
    }
}

pub fn due_label(due: &str, today: NaiveDate) -> String {
    match Due::parse(due) {
        Some(Due::Date(d)) => relative_date_label(d, today),
        Some(Due::At(t)) => format!("{} {}", relative_date_label(t.date(), today), hm(t.time())),
        None => due.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_date_only_expires_at_midnight() {
        let due = Due::parse("2026-09-25").unwrap();
        assert_eq!(stamp(due.deadline()), "2026-09-26T00:00");
        assert_eq!(
            normalize_due("2026-09-25 14:30").unwrap(),
            "2026-09-25T14:30"
        );
    }

    #[test]
    fn add_months_clamps_day() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert_eq!(date_key(add_months(d, 1)), "2026-02-28");
        assert_eq!(date_key(add_months(d, -2)), "2025-11-30");
    }
}
