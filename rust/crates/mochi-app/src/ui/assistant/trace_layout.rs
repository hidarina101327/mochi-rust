//! 整理 AI 执行记录和工具调用信息，生成展示所需的步骤内容。
use super::*;

/// 一个由 UI 侧持有的 agent 轨迹条目视图。核心 runner 刻意只暴露可序列化的
/// 轨迹而不是渲染类型；把转换放在这里，旧 Electron 会话（只含 JSON）和
/// 实时强类型 Rust 轨迹就能共用完全相同的外观。
fn trace_value(step: &AgentTraceStep) -> Option<serde_json::Value> {
    serde_json::to_value(step).ok()
}

fn trace_values(steps: &[AgentTraceStep]) -> Vec<serde_json::Value> {
    steps.iter().filter_map(trace_value).collect()
}

pub(super) fn persisted_trace(
    message: &mochi_core::ai::session::AiStoredMessage,
) -> Vec<serde_json::Value> {
    message
        .get("trace")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn legacy_streaming_trace(streaming: &Streaming) -> Vec<serde_json::Value> {
    if !streaming.trace.is_empty() {
        return trace_values(&streaming.trace);
    }

    let mut values = Vec::new();
    if !streaming.plan.is_empty() {
        let steps = streaming
            .plan
            .iter()
            .map(|line| {
                let (status, title) = match line.chars().next() {
                    Some('✓') => ("done", line.trim_start_matches('✓').trim()),
                    Some('→') => ("in_progress", line.trim_start_matches('→').trim()),
                    Some('−') => ("skipped", line.trim_start_matches('−').trim()),
                    _ => ("pending", line.trim_start_matches('□').trim()),
                };
                serde_json::json!({"title": title, "status": status})
            })
            .collect::<Vec<_>>();
        values.push(serde_json::json!({"kind":"plan", "steps":steps}));
    }
    // `reasoning` 已经在下方作为实时尾部渲染。不要再把它复制成一条
    // 合成的轨迹行，否则旧版流式会话会把模型给出的 reasoning 显示两遍。
    // 运行时提供强类型 Trace::Thinking 时，它仍是权威数据源。
    for (name, ok, summary) in &streaming.tools {
        // 进行中的工具由 `active_tool` 渲染；兜底时间线只放已完成的结果。
        let Some(ok) = ok else { continue };
        values.push(serde_json::json!({
            "kind":"tool",
            "name":name,
            "ok":ok,
            "durationMs":0,
            "summary":summary,
        }));
    }
    values
}

pub(super) fn trace_kind(value: &serde_json::Value) -> Option<&str> {
    value.get("kind").and_then(serde_json::Value::as_str)
}

pub(super) fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn json_u64(value: &serde_json::Value, camel: &str, snake: &str) -> Option<u64> {
    value
        .get(camel)
        .or_else(|| value.get(snake))
        .and_then(serde_json::Value::as_u64)
}

pub(super) fn trace_duration(value: &serde_json::Value) -> Option<u64> {
    json_u64(value, "durationMs", "duration_ms")
}

pub(super) fn format_duration(duration_ms: Option<u64>) -> Option<String> {
    let ms = duration_ms?;
    if ms < 1_000 {
        Some(format!("{ms}ms"))
    } else if ms < 10_000 {
        Some(format!("{:.1}s", ms as f32 / 1_000.0))
    } else {
        Some(format!("{}s", ms / 1_000))
    }
}

fn run_text(runs: &[crate::ui::text::Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

/// 按宽度换行前先按显式段落换行拆分。即使关闭自动换行，DirectWrite 也会
/// 尊重内嵌换行符，因此把带换行的文本塞进单个 `DrawCmd::Text` 会在一个
/// 只按一行计高的矩形里画出多行。让测量和绘制共用这个辅助函数，
/// 轨迹行预留的高度才与实际绘制一致。
pub(super) fn wrapped_plain_lines(source: &str, width: f32, style: TextStyle) -> Vec<String> {
    source
        .split('\n')
        .flat_map(|hard_line| {
            let hard_line = hard_line.strip_suffix('\r').unwrap_or(hard_line);
            text::wrap_source(hard_line, style, width.max(1.0))
                .into_iter()
                .map(|runs| run_text(&runs))
        })
        .collect()
}

pub(super) fn wrapped_height(
    source: &str,
    width: f32,
    style: TextStyle,
    max_lines: Option<usize>,
) -> f32 {
    let line_count = wrapped_plain_lines(source, width, style).len().max(1);
    let line_count = max_lines.map_or(line_count, |limit| line_count.min(limit.max(1)));
    line_count as f32 * style.line_height()
}

pub(super) fn plan_steps(value: &serde_json::Value) -> Vec<serde_json::Value> {
    value
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn plan_height(value: &serde_json::Value, width: f32) -> f32 {
    let steps = plan_steps(value);
    let rows = steps
        .iter()
        .map(|step| {
            let title = step
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            wrapped_height(title, (width - 34.0).max(32.0), TextStyle::Small, Some(2)).max(18.0)
        })
        .sum::<f32>();
    let note_h = value
        .get("note")
        .and_then(serde_json::Value::as_str)
        .filter(|note| !note.is_empty())
        .map(|note| 8.0 + wrapped_height(note, (width - 24.0).max(32.0), TextStyle::Tiny, Some(2)))
        .unwrap_or(0.0);
    20.0 + 8.0 + rows + (steps.len().saturating_sub(1) as f32 * 5.0) + note_h + 20.0
}

pub(super) fn thinking_height(value: &serde_json::Value, width: f32, expanded: bool) -> f32 {
    let text_value = json_string(value, "text").unwrap_or_default();
    let text_value = if expanded || text_value.chars().count() <= TRACE_MAX_COLLAPSED_CHARS {
        text_value
    } else {
        let tail = text_value
            .chars()
            .take(TRACE_MAX_COLLAPSED_CHARS)
            .collect::<String>();
        format!("{tail}…")
    };
    22.0 + wrapped_height(
        &text_value,
        (width - 34.0).max(32.0),
        TextStyle::Small,
        None,
    )
}

pub(super) fn tool_height(value: &serde_json::Value, width: f32, expanded: bool) -> f32 {
    let preview =
        json_string(value, "resultPreview").or_else(|| json_string(value, "result_preview"));
    let detail = if expanded {
        preview
            .as_deref()
            .map(|text| {
                4.0 + wrapped_height(text, (width - 34.0).max(32.0), TextStyle::Tiny, Some(4))
            })
            .unwrap_or(0.0)
    } else {
        0.0
    };
    // 工具行在第二行（标题下方）画摘要，所以基础高度必须包含两行，
    // 即使结果预览处于折叠状态。否则后面的实时工具行会盖住摘要。
    42.0 + detail
}

pub(super) fn trace_height(
    trace: &[serde_json::Value],
    width: f32,
    expanded: bool,
    expanded_steps: &HashSet<(usize, usize)>,
    message_index: usize,
) -> f32 {
    if trace.is_empty() {
        return 0.0;
    }
    if !expanded {
        return TRACE_HEADER_H;
    }
    // 轨迹卡把每行都画在自己的水平内边距内。测量也用同样的内部宽度，
    // 免得长的 plan/thinking 行在绘制后变长，盖住下方的回答。
    let row_width = (width - TRACE_PAD_X * 2.0).max(1.0);
    let rows = trace
        .iter()
        .enumerate()
        .map(|(index, value)| match trace_kind(value) {
            Some("plan") => plan_height(value, row_width),
            Some("thinking") => thinking_height(
                value,
                row_width,
                expanded_steps.contains(&(message_index, index)),
            ),
            Some("tool") => tool_height(
                value,
                row_width,
                expanded_steps.contains(&(message_index, index)),
            ),
            _ => 0.0,
        })
        .filter(|height| *height > 0.0)
        .collect::<Vec<_>>();
    TRACE_HEADER_H
        + TRACE_PAD_Y
        + rows.iter().sum::<f32>()
        + rows.len().saturating_sub(1) as f32 * TRACE_ROW_GAP
        + TRACE_PAD_Y
}

pub(super) fn streaming_reasoning_text(reasoning: &str, expanded: bool) -> String {
    if expanded || reasoning.chars().count() <= STREAMING_REASONING_TAIL_CHARS {
        reasoning.to_owned()
    } else {
        let tail = reasoning
            .chars()
            .rev()
            .take(STREAMING_REASONING_TAIL_CHARS)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("…{tail}")
    }
}

pub(super) fn activity_height(
    trace: &[serde_json::Value],
    reasoning: Option<&str>,
    active_tool: Option<&(String, Option<String>)>,
    plan: Option<&serde_json::Value>,
    width: f32,
    reasoning_expanded: bool,
    status: Option<&str>,
) -> f32 {
    let row_width = (width - TRACE_PAD_X * 2.0).max(1.0);
    let timeline_rows = trace
        .iter()
        .filter(|value| trace_kind(value) != Some("plan"))
        .filter_map(|value| match trace_kind(value) {
            Some("thinking") => Some(thinking_height(value, row_width, reasoning_expanded)),
            Some("tool") => Some(tool_height(value, row_width, true)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let reasoning = reasoning.filter(|text| !text.is_empty());
    let has_rows = !timeline_rows.is_empty() || reasoning.is_some() || active_tool.is_some();
    let mut h = plan.map_or(0.0, |value| plan_height(value, width));
    if has_rows {
        // paint_activity 在实时 plan 后加 8px 间隙，再给时间线左右各 8px
        // 内边距。最后一行之后也留行间隙，因为下内边距从那个光标之后开始。
        if plan.is_some() {
            h += 8.0;
        }
        let mut rows_h = timeline_rows.iter().sum::<f32>();
        let mut rows = timeline_rows.len();
        if let Some(reasoning) = reasoning {
            let shown = streaming_reasoning_text(reasoning, reasoning_expanded);
            rows_h += 22.0
                + wrapped_height(
                    &shown,
                    (row_width - 28.0).max(32.0),
                    TextStyle::Small,
                    (!reasoning_expanded).then_some(18),
                );
            rows += 1;
        }
        if active_tool.is_some() {
            rows_h += 24.0;
            rows += 1;
        }
        h += 16.0 + rows_h + rows as f32 * TRACE_ROW_GAP + 16.0;
    } else if status.is_some_and(|text| !text.is_empty()) {
        // 没有轨迹/reasoning/工具行时，状态文本占用与 paint_activity
        // 画出的同一个带内边距的时间线卡片。
        h += if plan.is_some() { 8.0 } else { 0.0 } + 16.0 + 20.0 + 16.0;
    }
    // 活动块留在普通消息滚动区域里。轨迹是用户可见的历史，
    // 若裁剪到固定卡片高度，长的 reasoning/工具输出就看不到了，
    // 还可能与下方的回答重叠。
    h
}

/// 把运行轨迹变成会话里可回看的一条隐藏消息（TSX 存 `trace` 字段；这里先落成文字）。
pub fn trace_summary(trace: &[AgentTraceStep]) -> String {
    trace
        .iter()
        .filter_map(|s| match s {
            AgentTraceStep::Tool {
                name, ok, summary, ..
            } => Some(format!(
                "{} {name}{}",
                if *ok { "✓" } else { "✗" },
                summary
                    .as_ref()
                    .map(|x| format!(" · {x}"))
                    .unwrap_or_default()
            )),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn plan_lines(steps: &[serde_json::Value]) -> Vec<String> {
    steps
        .iter()
        .map(|s| {
            format!(
                "{} {}",
                match s["status"].as_str() {
                    Some("done") => "✓",
                    Some("in_progress") => "→",
                    Some("skipped") => "−",
                    _ => "□",
                },
                s["title"].as_str().unwrap_or("")
            )
        })
        .collect()
}
