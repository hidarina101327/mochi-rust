//! 校验工作流 ID、节点关系、参数和输入数据。
use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

pub fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
pub fn fingerprint(flow: &Workflow) -> String {
    let mut executable = flow.clone();
    for n in &mut executable.nodes {
        n.position = Position::default();
        n.label.clear();
    }
    executable.name.clear();
    executable.description.clear();
    Sha256::digest(serde_json::to_vec(&executable).unwrap_or_default())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn validate(flow: &Workflow) -> Result<Vec<String>> {
    if flow.format != "mochi.workflow" || flow.schema_version != 1 {
        return Err("仅支持 mochi.workflow v1 JSON".into());
    }
    if !safe_id(&flow.id) || flow.name.trim().is_empty() || flow.name.len() > 300 {
        return Err("工作流 id 或名称无效".into());
    }
    if flow.nodes.len() < 2
        || flow.nodes.len() > 100
        || flow.edges.len() > 300
        || serde_json::to_vec(flow).map_err(|e| e.to_string())?.len() > 524288
    {
        return Err("工作流需有 2–100 个节点，最多 300 条连线、512 KiB".into());
    }
    super::scheduler::validate_trigger(&flow.trigger)?;
    if !flow.defaults.is_object() || !flow.input_schema.is_object() {
        return Err("默认输入和输入 schema 必须是对象".into());
    }
    let nodes: BTreeMap<_, _> = flow.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    if nodes.len() != flow.nodes.len() {
        return Err("节点 id 重复".into());
    }
    if flow.nodes.iter().filter(|n| n.kind == "start").count() != 1
        || !flow.nodes.iter().any(|n| n.kind == "end")
    {
        return Err("需要且仅需要一个开始节点，以及至少一个结束节点".into());
    }
    for n in &flow.nodes {
        if !safe_id(&n.id) || !catalog::KINDS.contains(&n.kind.as_str()) {
            return Err(format!("节点类型或 id 无效：{}", n.id));
        }
        if n.label.len() > 240
            || n.position.x.abs() > 100000.
            || n.position.y.abs() > 100000.
            || !n.position.x.is_finite()
            || !n.position.y.is_finite()
            || !n.inputs.is_object()
            || !n.config.is_object()
        {
            return Err(format!("节点参数无效：{}", n.id));
        }
        if !["stop", "continue"].contains(&n.on_error.as_str())
            || n.retries > 3
            || !(1000..=300_000).contains(&n.timeout_ms)
        {
            return Err(format!("节点错误策略或超时无效：{}", n.id));
        }
        if n.mutates() && n.retries > 0 {
            return Err(format!(
                "{} 有副作用，不允许自动重试；避免重复写入",
                n.label
            ));
        }
        if ["start", "end", "condition"].contains(&n.kind.as_str()) && n.for_each.is_some() {
            return Err("开始、结束和条件节点不支持 for_each".into());
        }
        if ["start", "end"].contains(&n.kind.as_str()) && !n.enabled {
            return Err("开始和结束节点不能禁用".into());
        }
        if n.kind == "http"
            && n.config["method"].as_str().is_some_and(|m| m != "GET")
            && n.retries > 0
        {
            return Err("非 GET 请求不允许自动重试".into());
        }
        if n.kind == "script"
            && n.config["code"].as_str().is_some_and(|code| {
                code.contains("{{ $") || code.starts_with("$nodes.") || code.starts_with("$input.")
            })
        {
            return Err("脚本配置不能引用动态变量；使用节点 input 传递数据".into());
        }
    }
    let mut ids = HashSet::new();
    for e in &flow.edges {
        if !safe_id(&e.id)
            || !ids.insert(&e.id)
            || !nodes.contains_key(e.source.as_str())
            || !nodes.contains_key(e.target.as_str())
            || e.source == e.target
        {
            return Err(format!("连线无效：{}", e.id));
        }
        if nodes[e.target.as_str()].kind == "start" || nodes[e.source.as_str()].kind == "end" {
            return Err("开始节点不能有输入连线；结束节点不能有输出连线".into());
        }
        if nodes[e.source.as_str()].kind == "condition" {
            if !matches!(e.source_handle.as_deref(), Some("true" | "false")) {
                return Err("条件节点连线须指定 true / false 分支".into());
            }
        } else if e.source_handle.is_some() {
            return Err("只有条件节点支持分支连线".into());
        }
    }
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    while order.len() < nodes.len() {
        let next = flow.nodes.iter().find(|n| {
            !seen.contains(&n.id)
                && flow
                    .edges
                    .iter()
                    .filter(|e| e.target == n.id)
                    .all(|e| seen.contains(&e.source))
        });
        let n = next.ok_or("工作流存在循环；请使用 for_each 处理数组")?;
        if n.kind != "start" && !flow.edges.iter().any(|e| e.target == n.id) {
            return Err(format!("节点 {} 没有连接到开始节点", n.label));
        }
        let mut ancestors = HashSet::new();
        fn collect(id: &str, edges: &[Edge], out: &mut HashSet<String>) {
            for e in edges.iter().filter(|e| e.target == id) {
                if out.insert(e.source.clone()) {
                    collect(&e.source, edges, out);
                }
            }
        }
        collect(&n.id, &flow.edges, &mut ancestors);
        let mut refs = references::expressions(&n.inputs);
        if let Some(expr) = &n.for_each {
            refs.push(expr.clone());
        }
        for expr in refs {
            let p: Vec<_> = expr.split('.').collect();
            match p[0] {
                "$input" | "$run" => {}
                "$item" if n.for_each.is_some() => {}
                "$nodes" if p.len() >= 3 && ancestors.contains(p[1]) && p[2] == "output" => {}
                _ => return Err(format!("{} 引用了无效变量或非上游节点：{}", n.label, expr)),
            }
        }
        seen.insert(n.id.clone());
        order.push(n.id.clone());
    }
    Ok(order)
}

pub fn validate_input(value: &Value, schema: &Value) -> Result<()> {
    if let Some(kind) = schema["type"].as_str() {
        let ok = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "null" => value.is_null(),
            _ => false,
        };
        if !ok {
            return Err(format!("输入类型应为 {kind}"));
        }
    }
    if let Some(required) = schema["required"].as_array() {
        for key in required {
            if value
                .get(key.as_str().ok_or("required 必须是字符串数组")?)
                .is_none()
            {
                return Err(format!("缺少必填输入 {key}"));
            }
        }
    }
    if let Some(props) = schema["properties"].as_object() {
        for (k, s) in props {
            if let Some(v) = value.get(k) {
                validate_input(v, s).map_err(|e| format!("{k}: {e}"))?;
            }
        }
    }
    if let Some(items) = value.as_array() {
        if schema.get("items").is_some() {
            for v in items {
                validate_input(v, &schema["items"])?;
            }
        }
    }
    Ok(())
}
