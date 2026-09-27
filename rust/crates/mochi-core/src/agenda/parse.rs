//! 自然语言快速添加：「明天下午3点到5点 评审 !高 #工作」→ 结构化字段。
//!
//! Rust regex 不支持环视；日期和时间边界在匹配后校验。
//! 输入前缀可显式指定种类：`愿望：`、`目标：`、`任务：`、`日程：`、`重复：`。

use std::sync::LazyLock;

use chrono::{Datelike, Days, NaiveDate};
use regex::Regex;

use super::model::{Kind, Priority};
use super::recur;

#[derive(Debug, Clone, PartialEq)]
pub struct QuickParse {
    pub title: String,
    /// 解析出的日期（本地当天），`None` 表示未指定
    pub date: Option<NaiveDate>,
    pub time: Option<(u32, u32)>,
    pub duration_minutes: Option<i32>,
    /// `low` | `normal` | `high` | `urgent`
    pub priority: Option<String>,
    pub tags: Vec<String>,
    pub recurrence_rule: Option<String>,
    /// 用户用前缀明确写出的种类。
    pub explicit_kind: Option<Kind>,
}

impl QuickParse {
    /// 显式前缀优先；否则有重复 → 重复安排，有时刻 → 日程，其余 → 任务。
    pub fn kind(&self) -> Kind {
        if let Some(kind) = self.explicit_kind {
            return kind;
        }
        if self.recurrence_rule.is_some() {
            Kind::Routine
        } else if self.time.is_some() {
            Kind::Entry
        } else {
            Kind::Task
        }
    }

    pub fn priority(&self) -> Option<Priority> {
        self.priority.as_deref().and_then(Priority::from_wire)
    }
}

const KIND_PREFIXES: &[(&str, Kind)] = &[
    ("愿望", Kind::Wish),
    ("心愿", Kind::Wish),
    ("目标", Kind::Goal),
    ("任务", Kind::Task),
    ("待办", Kind::Task),
    ("日程", Kind::Entry),
    ("事件", Kind::Entry),
    ("重复", Kind::Routine),
    ("习惯", Kind::Routine),
];

pub fn parse_quick_add(input: &str, today: NaiveDate) -> QuickParse {
    let trimmed = input.trim();
    for (prefix, kind) in KIND_PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if let Some(rest) = rest
                .strip_prefix('：')
                .or_else(|| rest.strip_prefix(':'))
                .or_else(|| rest.strip_prefix(' '))
            {
                // 愿望和目标不解析时间词：「目标：明年考过六级」的「明年」属于标题。
                if matches!(kind, Kind::Wish | Kind::Goal) {
                    return QuickParse {
                        title: rest.trim().to_owned(),
                        date: None,
                        time: None,
                        duration_minutes: None,
                        priority: None,
                        tags: Vec::new(),
                        recurrence_rule: None,
                        explicit_kind: Some(*kind),
                    };
                }
                let mut parsed = parse_tokens(rest, today);
                parsed.explicit_kind = Some(*kind);
                return parsed;
            }
        }
    }
    parse_tokens(trimmed, today)
}

/// 预览文案，供快速添加栏实时展示。
pub fn summarize(parse: &QuickParse) -> String {
    let mut parts: Vec<String> = vec![parse.kind().label().into()];
    if let Some(d) = parse.date {
        parts.push(format!(
            "{}月{}日{}",
            d.month(),
            d.day(),
            super::time::weekday_label(d)
        ));
    }
    if let Some((h, m)) = parse.time {
        parts.push(format!("{h:02}:{m:02}"));
    }
    if let Some(d) = parse.duration_minutes {
        parts.push(super::time::duration_label(d as i64));
    }
    if let Some(rule) = &parse.recurrence_rule {
        parts.push(recur::describe(rule));
    }
    if let Some(p) = parse.priority() {
        parts.push(format!("{}优先级", p.label()));
    }
    if !parse.tags.is_empty() {
        let tags: Vec<String> = parse.tags.iter().map(|t| format!("#{t}")).collect();
        parts.push(tags.join(" "));
    }
    parts.join(" · ")
}

/// 顺序与 TS 的 `PRIORITY_TOKENS` 字面量一致（JS 对象迭代按插入序）。
const PRIORITY_TOKENS: &[(&str, &str)] = &[
    ("!低", "low"),
    ("!中", "normal"),
    ("!高", "high"),
    ("!紧急", "urgent"),
];

/// 重复规则按「更具体的写法优先」匹配：`每周一三` 要落到 BYDAY，
/// 不能先被 `每周` 吃掉只剩一个「一」留在标题里。
const RECURRENCE_TOKENS: &[(&str, &str)] = &[
    ("每(?:个)?工作日|工作日", "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
    ("每(?:个)?周末", "FREQ=WEEKLY;BYDAY=SA,SU"),
    ("每天|每日", "FREQ=DAILY"),
    ("每两周|隔周", "FREQ=WEEKLY;INTERVAL=2"),
    ("每周", "FREQ=WEEKLY"),
    ("每月", "FREQ=MONTHLY"),
];

static RECURRENCE_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    RECURRENCE_TOKENS
        .iter()
        .map(|(pattern, rule)| (Regex::new(pattern).expect("内置重复规则正则"), *rule))
        .collect()
});

fn weekday_char(c: char) -> Option<u32> {
    match c {
        '一' => Some(1),
        '二' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '日' | '天' => Some(0),
        _ => None,
    }
}

macro_rules! re {
    ($name:ident, $p:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).unwrap());
    };
}

re!(TAG, r"#([^\s#!@，。]+)");
re!(FULL_DATE, r"(\d{4})[-/](\d{1,2})[-/](\d{1,2})");
re!(MONTH_DAY, r"(\d{1,2})月(\d{1,2})[日号]");
re!(SHORT_DATE, r"(\d{1,2})[-/](\d{1,2})");
re!(WEEKDAY, r"(下{1,2})?(?:周|星期|礼拜)([一二三四五六日天])");
re!(HOUR_HALF, r"(\d+(?:\.\d+)?)\s*(?:个)?小时半");
re!(HOUR, r"(?i)(\d+(?:\.\d+)?)\s*(?:个)?(?:小时|h)");
re!(HALF_HOUR, r"半(?:个)?小时");
re!(MINUTES, r"(?i)(\d+)\s*(?:分钟|min|m)");
re!(
    COLON_TIME,
    r"(凌晨|早上|上午|中午|下午|傍晚|晚上)?\s*(\d{1,2})[:：](\d{2})"
);
re!(
    HALF_PAST,
    r"(凌晨|早上|上午|中午|下午|傍晚|晚上)?\s*(\d{1,2})\s*点\s*半"
);
re!(
    DOT_TIME,
    r"(凌晨|早上|上午|中午|下午|傍晚|晚上)?\s*(\d{1,2})\s*点\s*(\d{1,2})?\s*分?"
);
re!(
    WEEKLY_DAYS,
    r"每(?:个)?(?:周|星期|礼拜)((?:[一二三四五六日天][、,，和及]?)+)"
);
re!(MONTHLY_DAY, r"每(?:个)?月\s*(\d{1,2})\s*[号日]");
re!(
    TIME_RANGE_END,
    r"^\s*(?:到|至|~|～|-|—)\s*(凌晨|早上|上午|中午|下午|傍晚|晚上)?\s*(\d{1,2})(?:[:：](\d{2})|\s*点\s*(半|\d{1,2})?\s*分?)?"
);
re!(WHITESPACE_RUN, r"\s{2,}");
re!(MERIDIEM_PM, r"下午|晚上|傍晚|晚");
re!(MERIDIEM_AM, r"凌晨|早上|上午|早");

fn char_before(text: &str, byte_idx: usize) -> Option<char> {
    text[..byte_idx].chars().next_back()
}

fn char_after(text: &str, byte_idx: usize) -> Option<char> {
    text[byte_idx..].chars().next()
}

/// JS `String.replace(str, ' ')`：只替换**第一次出现**。
fn replace_first(text: &str, needle: &str) -> String {
    text.replacen(needle, " ", 1)
}

fn cleanup_title(text: &str) -> String {
    let collapsed = WHITESPACE_RUN.replace_all(text, " ");
    collapsed
        .trim_matches(|c: char| c.is_whitespace() || ",，。·-".contains(c))
        .to_owned()
}

fn add_days(date: NaiveDate, days: u64) -> NaiveDate {
    date.checked_add_days(Days::new(days)).unwrap_or(date)
}

fn apply_meridiem(hour: u32, meridiem: Option<&str>) -> u32 {
    let Some(m) = meridiem.filter(|s| !s.is_empty()) else {
        return hour;
    };
    if MERIDIEM_PM.is_match(m) && hour < 12 {
        return hour + 12;
    }
    if m.contains("中午") && hour <= 2 {
        return hour + 12;
    }
    if MERIDIEM_AM.is_match(m) && hour == 12 {
        return 0;
    }
    hour
}

fn parse_tokens(input: &str, now: NaiveDate) -> QuickParse {
    let mut text = format!(" {} ", input.trim());
    let today = now;

    // --- 优先级（多个标记时以最后出现的为准）---
    let mut priority: Option<String> = None;
    if let Some((token, value)) = PRIORITY_TOKENS
        .iter()
        .filter_map(|(token, value)| text.rfind(token).map(|at| (at, *token, *value)))
        .max_by_key(|(at, _, _)| *at)
        .map(|(_, token, value)| (token, value))
    {
        priority = Some(value.into());
        for (other, _) in PRIORITY_TOKENS {
            while text.contains(other) {
                text = replace_first(&text, other);
            }
        }
        let _ = token;
    }

    // --- 标签 #xxx ---
    let mut tags: Vec<String> = Vec::new();
    text = TAG
        .replace_all(&text, |c: &regex::Captures| {
            tags.push(c[1].to_owned());
            " ".to_string()
        })
        .into_owned();

    // --- 重复规则 ---
    let mut recurrence_rule: Option<String> = None;
    if let Some(c) = WEEKLY_DAYS.captures(&text) {
        let mut days: Vec<&str> = Vec::new();
        for ch in c[1].chars() {
            let code = match weekday_char(ch) {
                Some(0) => "SU",
                Some(1) => "MO",
                Some(2) => "TU",
                Some(3) => "WE",
                Some(4) => "TH",
                Some(5) => "FR",
                Some(6) => "SA",
                _ => continue,
            };
            if !days.contains(&code) {
                days.push(code);
            }
        }
        if !days.is_empty() {
            recurrence_rule = Some(format!("FREQ=WEEKLY;BYDAY={}", days.join(",")));
            let whole = c[0].to_owned();
            text = replace_first(&text, &whole);
        }
    }
    if recurrence_rule.is_none() {
        if let Some(c) = MONTHLY_DAY.captures(&text) {
            let day = num(&c[1]);
            if (1..=31).contains(&day) {
                recurrence_rule = Some(format!("FREQ=MONTHLY;BYMONTHDAY={day}"));
                let whole = c[0].to_owned();
                text = replace_first(&text, &whole);
            }
        }
    }
    if recurrence_rule.is_none() {
        for (re, rule) in RECURRENCE_PATTERNS.iter() {
            if let Some(m) = re.find(&text) {
                recurrence_rule = Some((*rule).into());
                let matched = m.as_str().to_owned();
                text = replace_first(&text, &matched);
                break;
            }
        }
    }

    // --- 日期 ---
    let mut date: Option<NaiveDate> = None;

    // yyyy-m-d
    if date.is_none() {
        if let Some(c) = FULL_DATE.captures(&text) {
            let (y, m, d) = (num(&c[1]), num(&c[2]), num(&c[3]));
            if let Some(v) = NaiveDate::from_ymd_opt(y as i32, m, d) {
                date = Some(v);
                let whole = c[0].to_owned();
                text = replace_first(&text, &whole);
            }
        }
    }

    // m月d日
    if date.is_none() {
        if let Some(c) = MONTH_DAY.captures(&text) {
            let (m, d) = (num(&c[1]), num(&c[2]));
            if let Some(v) = NaiveDate::from_ymd_opt(today.year(), m, d) {
                let v = if v < today {
                    NaiveDate::from_ymd_opt(today.year() + 1, m, d).unwrap_or(v)
                } else {
                    v
                };
                date = Some(v);
                let whole = c[0].to_owned();
                text = replace_first(&text, &whole);
            }
        }
    }

    // m-d / m/d，需人工做 TS 里的 (?<![\d:：]) 与 (?![\d:：日号月]) 断言
    if date.is_none() {
        if let Some(c) = SHORT_DATE.captures_iter(&text).find(|c| {
            let m = c.get(0).unwrap();
            let before_ok = !matches!(char_before(&text, m.start()), Some(ch) if ch.is_ascii_digit() || ch == ':' || ch == '：');
            let after_ok = !matches!(char_after(&text, m.end()), Some(ch) if ch.is_ascii_digit() || ":：日号月".contains(ch));
            before_ok && after_ok
        }) {
            let (m, d) = (num(&c[1]), num(&c[2]));
            if let Some(v) = NaiveDate::from_ymd_opt(today.year(), m, d) {
                let v = if v < today {
                    NaiveDate::from_ymd_opt(today.year() + 1, m, d).unwrap_or(v)
                } else {
                    v
                };
                date = Some(v);
                let whole = c[0].to_owned();
                text = replace_first(&text, &whole);
            }
        }
    }

    for (literal, offset) in [
        ("大后天", 3u64),
        ("后天", 2),
        ("明天", 1),
        ("明日", 1),
        ("今天", 0),
        ("今日", 0),
    ] {
        if date.is_some() {
            break;
        }
        if text.contains(literal) {
            date = Some(add_days(today, offset));
            text = replace_first(&text, literal);
        }
    }

    // 周几（如"周五""下周二"）
    if date.is_none() {
        if let Some(c) = WEEKDAY.captures(&text) {
            if let Some(target) = c[2].chars().next().and_then(weekday_char) {
                let current = today.weekday().num_days_from_sunday();
                let mut offset = (target + 7 - current) % 7;
                match c.get(1) {
                    Some(next) if !next.as_str().is_empty() => {
                        offset += 7 * next.as_str().chars().count() as u32
                    }
                    _ if offset == 0 => offset = 7,
                    _ => {}
                }
                date = Some(add_days(today, offset as u64));
                let whole = c[0].to_owned();
                text = replace_first(&text, &whole);
            }
        }
    }

    // --- 时长（先于时刻解析，避免"30分钟"被当作时刻）---
    let mut duration_minutes: Option<i32> = None;
    for (re, kind) in [
        (&*HOUR_HALF, DurationKind::HourPlusHalf),
        (&*HOUR, DurationKind::Hour),
        (&*HALF_HOUR, DurationKind::HalfOnly),
        (&*MINUTES, DurationKind::Minutes),
    ] {
        if duration_minutes.is_some() {
            break;
        }
        let Some(c) = re.captures(&text) else {
            continue;
        };
        let whole = c[0].to_owned();
        // TS 的 (?![a-z])：后面不能紧跟英文小写字母（挡住 "3 hours"→"h" 之类的误配）
        let m = c.get(0).unwrap();
        if matches!(kind, DurationKind::Hour | DurationKind::Minutes)
            && matches!(char_after(&text, m.end()), Some(ch) if ch.is_ascii_lowercase())
        {
            continue;
        }
        duration_minutes = Some(match kind {
            DurationKind::HourPlusHalf => (fnum(&c[1]) * 60.0 + 30.0).round() as i32,
            DurationKind::Hour => (fnum(&c[1]) * 60.0).round() as i32,
            DurationKind::HalfOnly => 30,
            DurationKind::Minutes => num(&c[1]) as i32,
        });
        text = replace_first(&text, &whole);
    }

    // --- 时刻 ---
    let mut time: Option<(u32, u32)> = None;
    for kind in [TimeKind::Colon, TimeKind::HalfPast, TimeKind::Dot] {
        if time.is_some() {
            break;
        }
        let re: &Regex = match kind {
            TimeKind::Colon => &COLON_TIME,
            TimeKind::HalfPast => &HALF_PAST,
            TimeKind::Dot => &DOT_TIME,
        };
        let Some(c) = re.captures(&text) else {
            continue;
        };
        let meridiem = c.get(1).map(|m| m.as_str());
        let hour = apply_meridiem(num(&c[2]), meridiem);
        let minute = match kind {
            TimeKind::Colon => num(&c[3]),
            TimeKind::HalfPast => 30,
            TimeKind::Dot => c.get(3).map(|m| num(m.as_str())).unwrap_or(0),
        };
        if hour < 24 && minute < 60 {
            time = Some((hour, minute));
            let whole_match = c.get(0).unwrap();
            let mut whole = whole_match.as_str().to_owned();
            // 「3点到5点」「14:00-15:30」：结束时刻换算为时长。
            let tail = &text[whole_match.end()..];
            if let Some(end) = TIME_RANGE_END.captures(tail) {
                let end_meridiem = end.get(1).map(|m| m.as_str()).or(meridiem);
                let mut end_hour = apply_meridiem(num(&end[2]), end_meridiem);
                if end_hour < hour && end_hour + 12 < 24 && end.get(1).is_none() {
                    end_hour += 12;
                }
                let end_minute = match (end.get(3), end.get(4)) {
                    (Some(m), _) => num(m.as_str()),
                    (None, Some(m)) if m.as_str() == "半" => 30,
                    (None, Some(m)) => num(m.as_str()),
                    _ => 0,
                };
                let span = (end_hour * 60 + end_minute) as i32 - (hour * 60 + minute) as i32;
                if end_hour < 24 && end_minute < 60 && span > 0 {
                    if duration_minutes.is_none() {
                        duration_minutes = Some(span);
                    }
                    whole.push_str(&end[0]);
                }
            }
            text = replace_first(&text, &whole);
        }
    }

    QuickParse {
        title: cleanup_title(&text),
        date,
        time,
        duration_minutes,
        priority,
        tags,
        recurrence_rule,
        explicit_kind: None,
    }
}

#[derive(Clone, Copy)]
enum DurationKind {
    HourPlusHalf,
    Hour,
    HalfOnly,
    Minutes,
}

#[derive(Clone, Copy)]
enum TimeKind {
    Colon,
    HalfPast,
    Dot,
}

fn num(s: &str) -> u32 {
    s.parse().unwrap_or(0)
}

fn fnum(s: &str) -> f64 {
    s.parse().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-28 是周五。
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()
    }

    fn p(input: &str) -> QuickParse {
        parse_quick_add(input, today())
    }

    #[test]
    fn weekday_and_month_day_recurrences_keep_the_title_clean() {
        let r = p("每周一三 8点 晨会");
        assert_eq!(
            r.recurrence_rule.as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,WE")
        );
        assert_eq!(r.title, "晨会");
        assert_eq!(r.kind(), Kind::Routine);
        let r = p("每月15号 交房租");
        assert_eq!(
            r.recurrence_rule.as_deref(),
            Some("FREQ=MONTHLY;BYMONTHDAY=15")
        );
        assert_eq!(r.title, "交房租");
    }

    #[test]
    fn time_ranges_become_durations_and_the_last_priority_wins() {
        let r = p("明天下午3点到5点 评审");
        assert_eq!(r.time, Some((15, 0)));
        assert_eq!(r.duration_minutes, Some(120));
        assert_eq!(r.title, "评审");
        let r = p("14:00-15:30 设计讨论 !低 !紧急");
        assert_eq!(r.duration_minutes, Some(90));
        assert_eq!(r.priority(), Some(Priority::Urgent));
        assert_eq!(r.title, "设计讨论");
    }

    #[test]
    fn relative_dates_and_weekdays() {
        assert_eq!(p("后天 写周报").date, Some(add_days(today(), 2)));
        assert_eq!(p("后天 写周报").title, "写周报");
        assert_eq!(p("周五 例会").date, Some(add_days(today(), 7)));
        assert_eq!(p("下周一 例会").date, Some(add_days(today(), 10)));
        assert_eq!(p("9/3 开学").date, NaiveDate::from_ymd_opt(2026, 9, 3));
        assert_eq!(p("15:00 开会").date, None);
    }

    #[test]
    fn durations_before_times() {
        let r = p("会议 30分钟");
        assert_eq!(r.duration_minutes, Some(30));
        assert_eq!(r.time, None);
        assert_eq!(p("会议 1小时半").duration_minutes, Some(90));
    }

    #[test]
    fn kinds_follow_prefix_then_content() {
        assert_eq!(p("15:00 开会").kind(), Kind::Entry);
        assert_eq!(p("写周报").kind(), Kind::Task);
        let wish = p("愿望：明年去一趟冰岛");
        assert_eq!(wish.kind(), Kind::Wish);
        assert_eq!(wish.title, "明年去一趟冰岛");
        let goal = p("目标: 六级 550 分");
        assert_eq!(goal.kind(), Kind::Goal);
        assert_eq!(goal.title, "六级 550 分");
        let task = p("任务：明天 数学作业 !高");
        assert_eq!(task.kind(), Kind::Task);
        assert_eq!(task.date, Some(add_days(today(), 1)));
        assert_eq!(task.title, "数学作业");
    }

    #[test]
    fn summary_reads_naturally() {
        let r = p("明天 15:00 产品评审 60分钟 !高 #工作");
        assert_eq!(
            summarize(&r),
            "日程 · 8月29日周六 · 15:00 · 1 小时 · 高优先级 · #工作"
        );
        assert_eq!(summarize(&p("每天 晨跑")), "重复 · 每天");
    }
}
