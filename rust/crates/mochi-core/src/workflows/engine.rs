//! 按依赖关系执行工作流节点，并收集运行结果。
use super::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

pub trait NodeHost: Send + Sync {
    fn call(
        &self,
        node: &Node,
        input: &Value,
        cancel: Arc<AtomicBool>,
        run_id: &str,
    ) -> Result<Value>;
}

pub fn execute(
    store: &Store,
    mut run: Run,
    host: &dyn NodeHost,
    cancel: Arc<AtomicBool>,
) -> Result<Run> {
    let result = execute_graph(store, &mut run, host, cancel.clone());
    if let Err(error) = result {
        run.status = if cancel.load(Ordering::Relaxed) || store.cancelled(&run.id) {
            "cancelled"
        } else {
            "failed"
        }
        .into();
        run.error = Some(error);
    } else if run.nodes.values().any(|n| n.status == "failed") {
        run.status = "partial".into();
    } else {
        run.status = "succeeded".into();
    }
    run.finished_at = Some(now());
    for n in run
        .nodes
        .values_mut()
        .filter(|n| n.status == "pending" || n.status == "running")
    {
        n.status = if run.status == "cancelled" {
            "cancelled"
        } else {
            "skipped"
        }
        .into();
        n.finished_at = run.finished_at;
    }
    store.save_run(&run)?;
    store.notify(
        &run.id,
        &format!("自动化 · {}", run.definition.name),
        &format!(
            "{}{}",
            run.status,
            run.error
                .as_ref()
                .map(|s| format!("：{s}"))
                .unwrap_or_default()
        ),
    )?;
    Ok(run)
}
fn execute_graph(
    store: &Store,
    run: &mut Run,
    host: &dyn NodeHost,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let current = store.get(&run.workflow_id)?;
    if run.source == "schedule" && !current.enabled {
        return Err("定时运行已暂停，本次排队任务不再执行".into());
    }
    let order = validate(&run.definition)?;
    let timestamp = chrono::DateTime::from_timestamp_millis(run.started_at).ok_or("时间无效")?;
    let offset = match &run.definition.trigger {
        Trigger::Daily {
            utc_offset_minutes, ..
        }
        | Trigger::Weekly {
            utc_offset_minutes, ..
        } => chrono::FixedOffset::east_opt(utc_offset_minutes * 60).ok_or("时区无效")?,
        _ => *timestamp.with_timezone(&chrono::Local).offset(),
    };
    let local = timestamp.with_timezone(&offset);
    let mut context = json!({"input":run.input,"nodes":{},"run":{"id":run.id,"date":local.format("%Y-%m-%d").to_string(),"datetime":local.to_rfc3339(),"week":local.format("%G-W%V").to_string()}});
    for id in order {
        if cancel.load(Ordering::Relaxed) || store.cancelled(&run.id) {
            return Err("运行已取消".into());
        }
        let node = run
            .definition
            .nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap()
            .clone();
        let incoming: Vec<_> = run
            .definition
            .edges
            .iter()
            .filter(|e| e.target == id)
            .collect();
        let active = node.kind == "start"
            || incoming.iter().any(|e| {
                let previous = &run.nodes[&e.source];
                previous.status == "succeeded"
                    && e.source_handle
                        .as_ref()
                        .is_none_or(|branch| previous.output["branch"].as_str() == Some(branch))
                    || previous.status == "failed"
                        && run
                            .definition
                            .nodes
                            .iter()
                            .any(|n| n.id == e.source && n.on_error == "continue")
            });
        if !active || !node.enabled {
            run.nodes.get_mut(&id).unwrap().status = "skipped".into();
            store.save_run(run)?;
            continue;
        }
        {
            let record = run.nodes.get_mut(&id).unwrap();
            record.status = "running".into();
            record.started_at = Some(now());
        }
        store.save_run(run)?;
        let start = Instant::now();
        let result: Result<Value> = (|| {
            let items = match &node.for_each {
                Some(expr) => resolve(&json!(expr), &context)?
                    .as_array()
                    .cloned()
                    .ok_or("for_each 必须引用数组")?,
                None => vec![Value::Null],
            };
            if items.len() > 100 {
                return Err("单节点 for_each 最多 100 项".into());
            }
            let mut outputs = Vec::new();
            for item in items {
                if cancel.load(Ordering::Relaxed) {
                    return Err("运行已取消".into());
                }
                context["item"] = item;
                let mut input = resolve(&node.inputs, &context)?;
                if node.kind == "start" {
                    let mut merged = run.input.clone();
                    for (key, value) in input.as_object().into_iter().flatten() {
                        merged[key] = value.clone();
                    }
                    input = merged;
                }
                run.nodes.get_mut(&id).unwrap().input = input.clone();
                if serde_json::to_vec(run).map_err(|e| e.to_string())?.len() > 6 * 1024 * 1024 {
                    run.nodes.get_mut(&id).unwrap().input = Value::Null;
                    return Err("运行详情超过 6 MiB，请分批处理或传递文件引用".into());
                }
                store.save_run(run)?;
                let mut result = Err("节点未执行".into());
                for attempt in 0..=node.retries {
                    if start.elapsed().as_millis() > node.timeout_ms as u128 {
                        return Err("节点已超时".into());
                    }
                    run.nodes.get_mut(&id).unwrap().attempts += 1;
                    let mut attempt_node = node.clone();
                    attempt_node.timeout_ms = node
                        .timeout_ms
                        .saturating_sub(start.elapsed().as_millis() as u64)
                        .max(1000);
                    result = super::nodes::pure(&node, &input).unwrap_or_else(|| {
                        host.call(
                            &attempt_node,
                            &input,
                            cancel.clone(),
                            &format!("{}:{}:{}", run.id, id, outputs.len()),
                        )
                    });
                    if result.is_ok() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if attempt < node.retries {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                let output = result.map_err(|e| {
                    if node.for_each.is_some() {
                        format!("第 {} 项：{e}", outputs.len() + 1)
                    } else {
                        e
                    }
                })?;
                if output.to_string().len() > 262144 {
                    return Err("节点输出超过 256 KiB，请分页或写入文件后传递对象引用".into());
                }
                outputs.push(output);
                if serde_json::to_vec(&outputs)
                    .map_err(|e| e.to_string())?
                    .len()
                    > 262144
                {
                    return Err("循环输出超过 256 KiB，请减少批次数或返回文件路径".into());
                }
                if node.for_each.is_some() {
                    run.nodes.get_mut(&id).unwrap().output =
                        json!({"items":outputs,"completed":outputs.len()});
                    store.save_run(run)?;
                }
            }
            if node.for_each.is_some() {
                Ok(json!({"items":outputs}))
            } else {
                Ok(outputs.pop().unwrap_or(Value::Null))
            }
        })();
        let record = run.nodes.get_mut(&id).unwrap();
        record.finished_at = Some(now());
        match result {
            Ok(output) => {
                record.status = "succeeded".into();
                record.output = output.clone();
                context["nodes"][&id] = json!({"output":output});
                if node.kind == "end" {
                    run.output = record.output.clone();
                }
            }
            Err(error) => {
                record.status = "failed".into();
                record.error = Some(error.clone());
                context["nodes"][&id] = json!({"output":{"error":error}});
                store.save_run(run)?;
                if node.on_error != "continue" {
                    return Err(format!("{}：{}", node.label, error));
                }
            }
        }
        store.save_run(run)?;
    }
    Ok(())
}
