//! 落盘时间使用 UTC、三位毫秒和 Z 后缀，与 Date.toISOString() 一致。

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

/// 当前时刻的 ISO 串，等价于 JS `new Date().toISOString()`。
pub fn now() -> String {
    from(Utc::now())
}

/// 任意时刻 → ISO 串（先归一到 UTC）。
pub fn from(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Unix 毫秒 → ISO 串。对应 JS `new Date(ms).toISOString()`。
pub fn from_millis(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms).single().map(from)
}

/// 当前 Unix 毫秒，等价于 JS `Date.now()`。
pub fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

/// 解析 ISO 串；失败返回 `None`（容忍旧数据脏值，不 panic）。
pub fn try_parse(iso: &str) -> Option<DateTime<Utc>> {
    if iso.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// 解析 ISO 串为 Unix 毫秒；失败返回 `None`。
pub fn try_parse_millis(iso: &str) -> Option<i64> {
    try_parse(iso).map(|t| t.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_matches_js_to_iso_string() {
        let s = now();
        // 2026-08-22T03:14:15.926Z —— 固定 24 字符
        assert_eq!(s.len(), 24, "长度不对: {s}");
        assert!(s.ends_with('Z'), "结尾不是 Z: {s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[19..20], ".", "缺少毫秒小数点: {s}");
    }

    #[test]
    fn millis_are_always_three_digits() {
        // 期望值取自 Node：new Date(ms).toISOString()
        // 整秒时刻仍须写出 .000，否则字节数与 JS 不一致
        assert_eq!(
            from_millis(1_755_830_055_000).unwrap(),
            "2025-08-22T02:34:15.000Z"
        );
        assert_eq!(
            from_millis(1_755_830_055_926).unwrap(),
            "2025-08-22T02:34:15.926Z"
        );
    }

    #[test]
    fn epoch_zero() {
        assert_eq!(from_millis(0).unwrap(), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn roundtrip_preserves_millis() {
        let ms = 1_755_830_055_926;
        let iso = from_millis(ms).unwrap();
        assert_eq!(try_parse_millis(&iso), Some(ms));
    }

    #[test]
    fn parse_tolerates_garbage() {
        assert!(try_parse("").is_none());
        assert!(try_parse("not a date").is_none());
        assert!(try_parse("2026-13-45T99:99:99Z").is_none());
    }

    /// 旧数据里可能存在带时区偏移的串，需归一到 UTC 而不是拒绝。
    #[test]
    fn parse_normalizes_offset_to_utc() {
        let t = try_parse("2026-08-22T11:14:15.926+08:00").unwrap();
        assert_eq!(from(t), "2026-08-22T03:14:15.926Z");
    }
}
