//! 解析工作流表达式，并从输入数据中查找引用值。
use super::Result;
use serde_json::Value;

pub fn expressions(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match value {
        Value::String(s) => {
            if s.starts_with('$') && !s.contains("{{") {
                out.push(s.clone());
            }
            let mut rest = s.as_str();
            while let Some((_, tail)) = rest.split_once("{{") {
                if let Some((expr, next)) = tail.split_once("}}") {
                    out.push(expr.trim().into());
                    rest = next;
                } else {
                    out.push("INVALID_TEMPLATE".into());
                    break;
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|v| out.extend(expressions(v))),
        Value::Object(o) => o.values().for_each(|v| out.extend(expressions(v))),
        _ => {}
    }
    out
}

fn lookup<'a>(expr: &str, context: &'a Value) -> Result<&'a Value> {
    let path = expr
        .strip_prefix('$')
        .ok_or_else(|| format!("无效引用：{expr}"))?;
    let mut value = context;
    for key in path.split('.') {
        value = if let Value::Array(a) = value {
            a.get(
                key.parse::<usize>()
                    .map_err(|_| format!("数组下标无效：{expr}"))?,
            )
        } else {
            value.get(key)
        }
        .ok_or_else(|| format!("引用不存在：{expr}"))?;
    }
    Ok(value)
}

pub fn resolve(value: &Value, context: &Value) -> Result<Value> {
    let result = match value {
        Value::String(s) if s.starts_with('$') && !s.contains("{{") => lookup(s, context)?.clone(),
        Value::String(s) => {
            let mut out = String::new();
            let mut rest = s.as_str();
            while let Some((before, tail)) = rest.split_once("{{") {
                out.push_str(before);
                let (expr, next) = tail.split_once("}}").ok_or("模板缺少 }}")?;
                let v = lookup(expr.trim(), context)?;
                out.push_str(
                    v.as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string())
                        .as_str(),
                );
                if out.len() > 1_048_576 {
                    return Err("模板输出超过 1 MiB".into());
                }
                rest = next;
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| resolve(v, context))
                .collect::<Result<_>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| Ok((k.clone(), resolve(v, context)?)))
                .collect::<Result<_>>()?,
        ),
        v => v.clone(),
    };
    if result.to_string().len() > 1_048_576 {
        return Err("节点数据超过 1 MiB".into());
    }
    Ok(result)
}
