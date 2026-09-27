//! 重复规则：RRULE 子集（DAILY / WEEKLY / MONTHLY + INTERVAL、BYDAY、BYMONTHDAY、UNTIL）。

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};

use super::time::{add_days, date_key, parse_date, week_start};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: i64,
    /// 0 = 周日 … 6 = 周六
    pub by_day: Vec<u32>,
    pub by_month_day: Option<u32>,
    pub until: Option<NaiveDate>,
}

pub const PRESETS: &[(&str, &str)] = &[
    ("FREQ=DAILY", "每天"),
    ("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "工作日"),
    ("FREQ=WEEKLY;BYDAY=SA,SU", "周末"),
    ("FREQ=WEEKLY", "每周"),
    ("FREQ=WEEKLY;INTERVAL=2", "每两周"),
    ("FREQ=MONTHLY", "每月"),
];

const DAY_CODES: [&str; 7] = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
const DAY_LABELS: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

pub fn parse(rule: &str) -> Option<Rule> {
    let mut parts: HashMap<String, String> = HashMap::new();
    for segment in rule.trim().trim_start_matches("RRULE:").split(';') {
        if let Some((k, v)) = segment.split_once('=') {
            parts.insert(k.trim().to_uppercase(), v.trim().to_uppercase());
        }
    }
    let freq = match parts.get("FREQ")?.as_str() {
        "DAILY" => Freq::Daily,
        "WEEKLY" => Freq::Weekly,
        "MONTHLY" => Freq::Monthly,
        _ => return None,
    };
    let interval = parts
        .get("INTERVAL")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(1)
        .clamp(1, 365);
    let by_day = parts
        .get("BYDAY")
        .map(|raw| {
            let mut days: Vec<u32> = raw
                .split(',')
                .filter_map(|c| DAY_CODES.iter().position(|d| *d == c.trim()))
                .map(|d| d as u32)
                .collect();
            days.sort_unstable();
            days.dedup();
            days
        })
        .unwrap_or_default();
    let by_month_day = parts
        .get("BYMONTHDAY")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|d| (1..=31).contains(d));
    let until = parts.get("UNTIL").and_then(|raw| {
        let digits: String = raw.chars().filter(char::is_ascii_digit).take(8).collect();
        (digits.len() == 8)
            .then(|| format!("{}-{}-{}", &digits[..4], &digits[4..6], &digits[6..8]))
            .and_then(|v| parse_date(&v))
    });
    Some(Rule {
        freq,
        interval,
        by_day,
        by_month_day,
        until,
    })
}

impl Rule {
    pub fn to_wire(&self) -> String {
        let mut out = String::from(match self.freq {
            Freq::Daily => "FREQ=DAILY",
            Freq::Weekly => "FREQ=WEEKLY",
            Freq::Monthly => "FREQ=MONTHLY",
        });
        if self.interval > 1 {
            out.push_str(&format!(";INTERVAL={}", self.interval));
        }
        if !self.by_day.is_empty() && self.freq == Freq::Weekly {
            let codes: Vec<&str> = self.by_day.iter().map(|d| DAY_CODES[*d as usize]).collect();
            out.push_str(&format!(";BYDAY={}", codes.join(",")));
        }
        if let (Some(day), Freq::Monthly) = (self.by_month_day, self.freq) {
            out.push_str(&format!(";BYMONTHDAY={day}"));
        }
        if let Some(until) = self.until {
            out.push_str(&format!(";UNTIL={}", until.format("%Y%m%d")));
        }
        out
    }
}

/// 规范化外部给的规则串；无法识别返回 `None`。
pub fn normalize(rule: &str) -> Option<String> {
    parse(rule).map(|r| r.to_wire())
}

pub fn describe(rule: &str) -> String {
    let Some(parsed) = parse(rule) else {
        return "不重复".into();
    };
    for (value, label) in PRESETS {
        if parse(value).as_ref() == Some(&parsed) {
            return (*label).into();
        }
    }
    let every = if parsed.interval > 1 {
        format!("每{}", parsed.interval)
    } else {
        "每".into()
    };
    let mut text = match parsed.freq {
        Freq::Daily => format!("{every}天"),
        Freq::Weekly if parsed.by_day.is_empty() => format!("{every}周"),
        Freq::Weekly => {
            let names: Vec<&str> = parsed
                .by_day
                .iter()
                .map(|d| DAY_LABELS[*d as usize])
                .collect();
            format!("{every}周 {}", names.join("、"))
        }
        Freq::Monthly => match parsed.by_month_day {
            Some(d) => format!("{every}月 {d} 日"),
            None => format!("{every}月"),
        },
    };
    if let Some(until) = parsed.until {
        text.push_str(&format!("，至 {}", date_key(until)));
    }
    text
}

fn weekday0(d: NaiveDate) -> u32 {
    d.weekday().num_days_from_sunday()
}

/// 规则在 `day` 是否发生。`anchor` 是起始日期，早于它的日子一律不发生。
pub fn occurs_on(rule: &Rule, anchor: NaiveDate, day: NaiveDate) -> bool {
    let diff = (day - anchor).num_days();
    if diff < 0 || rule.until.is_some_and(|u| day > u) {
        return false;
    }
    match rule.freq {
        Freq::Daily => diff % rule.interval == 0,
        Freq::Weekly => {
            let weeks = (week_start(day) - week_start(anchor)).num_days() / 7;
            let days = if rule.by_day.is_empty() {
                vec![weekday0(anchor)]
            } else {
                rule.by_day.clone()
            };
            weeks % rule.interval == 0 && days.contains(&weekday0(day))
        }
        Freq::Monthly => {
            let months = (day.year() as i64 - anchor.year() as i64) * 12
                + (day.month() as i64 - anchor.month() as i64);
            let target = rule.by_month_day.unwrap_or_else(|| anchor.day());
            // 31 号的规则在短月落到月末，免得整月消失。
            let last = super::time::days_in_month(day);
            months % rule.interval == 0 && day.day() == target.min(last)
        }
    }
}

/// `[from, to]` 闭区间内的发生日期。
pub fn dates_between(
    rule: &Rule,
    anchor: NaiveDate,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut day = from.max(anchor);
    while day <= to {
        if occurs_on(rule, anchor, day) {
            out.push(day);
        }
        day = add_days(day, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        parse_date(s).unwrap()
    }

    #[test]
    fn weekly_by_day_and_interval() {
        let rule = parse("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE").unwrap();
        let dates = dates_between(&rule, d("2026-09-21"), d("2026-09-21"), d("2026-10-04"));
        let keys: Vec<String> = dates.into_iter().map(date_key).collect();
        assert_eq!(keys, ["2026-09-21", "2026-09-23"]);
        assert_eq!(rule.to_wire(), "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE");
    }

    #[test]
    fn monthly_31_falls_on_month_end() {
        let rule = parse("FREQ=MONTHLY;BYMONTHDAY=31").unwrap();
        assert!(occurs_on(&rule, d("2026-01-31"), d("2026-02-28")));
        assert!(!occurs_on(&rule, d("2026-01-31"), d("2026-02-27")));
    }

    #[test]
    fn describe_presets() {
        assert_eq!(describe("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"), "工作日");
        assert_eq!(describe("FREQ=DAILY;INTERVAL=3"), "每3天");
    }
}
