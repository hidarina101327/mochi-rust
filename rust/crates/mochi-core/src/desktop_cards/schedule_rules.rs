//! 供桌面日程分组使用的精简比较语法，不会执行代码。
use anyhow::{bail, Result};
use serde_json::Value;

pub const HELP: &str = "元属性：日期、标题、优先级、状态、类型、projectId、categoryId、createdAt、updatedAt，以及对象中的字段路径（如 time.startAt）。常数：今日、明日、昨日。比较：= != < <= > >=；组合：&& ||。例：日期 < 今日 && 状态 != 已完成";

fn comparison(input: &str) -> Result<(&str, &str, &str)> {
    for op in ["<=", ">=", "!=", "==", "=", "<", ">"] {
        if let Some((left, right)) = input.split_once(op) {
            let left = left.trim();
            let right = right.trim().trim_matches(['\'', '"']);
            if left.is_empty() || right.is_empty() || left.contains(['(', ')']) {
                bail!("规则需要“属性 比较符 值”：{input}");
            }
            return Ok((left, op, right));
        }
    }
    bail!("规则缺少比较符：{input}")
}

pub fn validate(rule: &str) -> Result<()> {
    anyhow::ensure!(
        !rule.is_empty() && rule.len() <= 1024,
        "规则长度须为 1–1024 字节"
    );
    for clause in rule.split("||").flat_map(|s| s.split("&&")) {
        comparison(clause)?;
    }
    Ok(())
}

fn canonical(value: &str) -> &str {
    match value {
        "紧急" => "urgent",
        "高" => "high",
        "一般" | "普通" | "中" | "medium" => "normal",
        "低" => "low",
        "已完成" | "已执行" | "completed" | "complete" => "done",
        "未完成" | "待做" | "待办" | "draft" | "scheduled" => "todo",
        "进行中" => "doing",
        "计划中" => "planned",
        "任务" => "task",
        "日程" | "event" => "entry",
        other => other,
    }
}

pub fn matches(rule: &str, object: &Value, date: &str, today: chrono::NaiveDate) -> bool {
    if validate(rule).is_err() {
        return false;
    }
    rule.split("||").any(|group| {
        group.split("&&").all(|clause| {
            let Ok((key, op, expected)) = comparison(clause) else {
                return false;
            };
            let key = match key {
                "日期" => "date",
                "标题" => "title",
                "优先级" => "priority",
                "状态" => "status",
                "类型" => "kind",
                other => other,
            };
            let value = if key == "date" {
                Value::String(date.into())
            } else {
                key.split('.')
                    .try_fold(object, |current, part| current.get(part))
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            if value.is_null() || (key == "date" && date.is_empty()) {
                return false;
            }
            let actual = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            let expected = match expected {
                "今日" | "today" => today.to_string(),
                "明日" | "tomorrow" => (today + chrono::Duration::days(1)).to_string(),
                "昨日" | "yesterday" => (today - chrono::Duration::days(1)).to_string(),
                other => canonical(other).to_owned(),
            };
            let actual = canonical(&actual);
            let order = match (actual.parse::<f64>(), expected.parse::<f64>()) {
                (Ok(a), Ok(b)) => a.partial_cmp(&b),
                _ => Some(actual.cmp(&expected)),
            };
            match op {
                "=" | "==" => actual == expected,
                "!=" => actual != expected,
                "<" => order == Some(std::cmp::Ordering::Less),
                ">" => order == Some(std::cmp::Ordering::Greater),
                "<=" => order.is_some_and(|o| o != std::cmp::Ordering::Greater),
                ">=" => order.is_some_and(|o| o != std::cmp::Ordering::Less),
                _ => false,
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comparison_groups_use_dates_metadata_and_local_constants() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let object = serde_json::json!({"priority":"urgent", "status":"todo", "time":{"startAt":"2026-09-20"}});
        assert!(matches(
            "日期 < 今日 && 优先级 = 紧急",
            &object,
            "2026-09-20",
            today
        ));
        assert!(!matches("日期 > 今日", &object, "2026-09-20", today));
        assert!(matches("time.startAt = 昨日", &object, "", today));
        assert!(!matches("日期 < 今日", &object, "", today));
        assert!(validate("日期").is_err());
    }
}
