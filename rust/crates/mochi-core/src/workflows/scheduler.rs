//! 校验工作流触发条件，并确定到期的调度时间。
use super::{Result, Trigger};
use chrono::{Datelike, FixedOffset, NaiveTime, TimeZone, Timelike, Utc};

pub(super) fn validate_trigger(trigger: &Trigger) -> Result<()> {
    match trigger {
        Trigger::Manual => Ok(()),
        Trigger::Interval { minutes } if (1..=525600).contains(minutes) => Ok(()),
        Trigger::Daily {
            time,
            utc_offset_minutes,
        }
        | Trigger::Weekly {
            time,
            utc_offset_minutes,
            ..
        } => {
            NaiveTime::parse_from_str(time, "%H:%M").map_err(|_| "时间格式应为 HH:MM")?;
            if !(-720..=840).contains(utc_offset_minutes) {
                return Err("时区偏移无效".into());
            }
            if let Trigger::Weekly { weekdays, .. } = trigger {
                if weekdays.is_empty() || weekdays.iter().any(|d| !(1..=7).contains(d)) {
                    return Err("星期取值为 1–7".into());
                }
            }
            Ok(())
        }
        _ => Err("定时触发配置无效".into()),
    }
}

pub fn due_slot(trigger: &Trigger, millis: i64) -> Option<String> {
    validate_trigger(trigger).ok()?;
    match trigger {
        Trigger::Manual => None,
        Trigger::Interval { minutes } => Some(format!(
            "interval:{}",
            millis.div_euclid(*minutes as i64 * 60000)
        )),
        Trigger::Daily {
            time,
            utc_offset_minutes,
        }
        | Trigger::Weekly {
            time,
            utc_offset_minutes,
            ..
        } => {
            let offset = FixedOffset::east_opt(*utc_offset_minutes * 60)?;
            let at = Utc
                .timestamp_millis_opt(millis)
                .single()?
                .with_timezone(&offset);
            if let Trigger::Weekly { weekdays, .. } = trigger {
                if !weekdays.contains(&at.weekday().number_from_monday()) {
                    return None;
                }
            }
            let time = NaiveTime::parse_from_str(time, "%H:%M").ok()?;
            (at.hour() * 60 + at.minute() >= time.hour() * 60 + time.minute())
                .then(|| format!("day:{}", at.date_naive()))
        }
    }
}
