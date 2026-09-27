//! 实现工作流中的条件判断、数据转换和其他节点操作。
use super::{Node, Result};
use serde_json::{json, Value};

pub(super) fn pure(node: &Node, input: &Value) -> Option<Result<Value>> {
    Some(match node.kind.as_str() {
        "start" | "end" => Ok(input.clone()),
        "condition" => condition(&node.config, input),
        "json" => transform(&node.config, input),
        "chart" => chart(&node.config, input),
        _ => return None,
    })
}
fn condition(config: &Value, input: &Value) -> Result<Value> {
    let (a, b) = (&input["left"], &input["right"]);
    let branch = match config["operator"].as_str().unwrap_or("equals") {
        "equals" => a == b,
        "not_equals" => a != b,
        "contains" => {
            a.as_str()
                .zip(b.as_str())
                .is_some_and(|(a, b)| a.contains(b))
                || a.as_array().is_some_and(|a| a.contains(b))
        }
        "greater" => a.as_f64().zip(b.as_f64()).ok_or("greater 需要数字")?.0 > b.as_f64().unwrap(),
        "less" => a.as_f64().ok_or("less 需要数字")? < b.as_f64().ok_or("less 需要数字")?,
        "exists" => !a.is_null(),
        "truthy" => a == true,
        _ => return Err("不支持的条件运算符".into()),
    };
    Ok(json!({"branch":if branch{"true"}else{"false"},"value":branch}))
}
fn transform(config: &Value, input: &Value) -> Result<Value> {
    match config["operation"].as_str().unwrap_or("identity") {
        "identity" => Ok(input.clone()),
        "parse" => serde_json::from_str(input["text"].as_str().ok_or("缺少 text")?)
            .map_err(|e| e.to_string()),
        "count" => Ok(json!({"count":input["items"].as_array().ok_or("items 必须是数组")?.len()})),
        "sum" => Ok(
            json!({"sum":input["items"].as_array().ok_or("items 必须是数组")?.iter().map(|v|v.as_f64().ok_or("sum 需要数字数组")).collect::<std::result::Result<Vec<_>,_>>()?.iter().sum::<f64>()}),
        ),
        "join" => Ok(
            json!({"text":input["items"].as_array().ok_or("items 必须是数组")?.iter().map(|v|v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string())).collect::<Vec<_>>().join(config["separator"].as_str().unwrap_or("\n"))}),
        ),
        _ => Err("不支持的数据处理方式".into()),
    }
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn chart(config: &Value, input: &Value) -> Result<Value> {
    let labels = input["labels"].as_array().ok_or("labels 必须是数组")?;
    let values = input["values"].as_array().ok_or("values 必须是数组")?;
    if values.is_empty() || values.len() > 100 || values.len() != labels.len() {
        return Err("图表需要 1–100 个标签及对应数值".into());
    }
    let numbers = values
        .iter()
        .map(|v| {
            v.as_f64()
                .filter(|v| v.is_finite())
                .ok_or("图表数据必须为有限数字")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let min = numbers.iter().copied().fold(0., f64::min);
    let max = numbers.iter().copied().fold(0., f64::max);
    let span = (max - min).max(1.);
    let y = |v: f64| 320. - (v - min) / span * 240.;
    let step = 640. / numbers.len() as f64;
    let mut svg=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"760\" height=\"400\" viewBox=\"0 0 760 400\"><rect width=\"760\" height=\"400\" fill=\"#fff\"/><g font-family=\"sans-serif\" fill=\"#344054\"><text x=\"40\" y=\"36\" font-size=\"20\">{}</text><path d=\"M50 {}H710\" stroke=\"#d0d5dd\"/>",escape(input["title"].as_str().unwrap_or("统计")),y(0.));
    let line = config["type"].as_str() == Some("line");
    let mut points = Vec::new();
    for (i, v) in numbers.iter().enumerate() {
        let x = 50. + step * (i as f64 + 0.5);
        let top = y(*v);
        points.push(format!("{x},{top}"));
        if !line {
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"2\" fill=\"#4875b8\"/>",
                x - step * 0.3,
                top.min(y(0.)),
                step * 0.6,
                (top - y(0.)).abs().max(1.)
            ));
        }
        if numbers.len() <= 20 {
            svg.push_str(&format!("<text x=\"{x}\" y=\"352\" text-anchor=\"middle\" font-size=\"12\">{}</text><text x=\"{x}\" y=\"{}\" text-anchor=\"middle\" font-size=\"12\">{v}</text>",escape(labels[i].as_str().unwrap_or("")),top-8.));
        }
    }
    if line {
        svg.push_str(&format!(
            "<polyline points=\"{}\" fill=\"none\" stroke=\"#4875b8\" stroke-width=\"3\"/>",
            points.join(" ")
        ));
    }
    svg.push_str("</g></svg>");
    Ok(json!({"svg":svg,"mime":"image/svg+xml","count":numbers.len(),"min":min,"max":max}))
}
