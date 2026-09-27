//! 绘制 AI 执行计划、步骤摘要和工具调用记录。
use super::*;

pub(super) fn paint_wrapped_text(
    list: &mut DrawList,
    source: &str,
    rect: Rect,
    style: TextStyle,
    color: u32,
    max_lines: Option<usize>,
) {
    let lines = wrapped_plain_lines(source, rect.width(), style);
    let line_height = style.line_height();
    for (index, line) in lines.into_iter().enumerate() {
        if max_lines.is_some_and(|limit| index >= limit) {
            break;
        }
        let y = rect.top + index as f32 * line_height;
        if y >= rect.bottom {
            break;
        }
        list.text(
            Rect::new(rect.left, y, rect.right, y + line_height),
            line,
            style,
            color,
        );
    }
}

fn tool_label(name: &str) -> String {
    match name {
        "canvas_get" => return "查看画布".into(),
        "canvas_draw" => return "画布绘画与写字".into(),
        "canvas_add_cards" => return "添加画布引用".into(),
        "canvas_move_card" => return "移动画布卡片".into(),
        _ => {}
    }
    name.strip_prefix("mcp__")
        .map(|value| format!("MCP: {}", value.replace("__", " / ")))
        .unwrap_or_else(|| name.to_owned())
}

fn trace_summary_parts(trace: &[serde_json::Value]) -> (usize, usize, usize, Option<u64>) {
    let thinking = trace
        .iter()
        .filter(|value| trace_kind(value) == Some("thinking"))
        .count();
    let tools = trace
        .iter()
        .filter(|value| trace_kind(value) == Some("tool"))
        .count();
    let failed = trace
        .iter()
        .filter(|value| {
            trace_kind(value) == Some("tool")
                && value.get("ok").and_then(|v| v.as_bool()) == Some(false)
        })
        .count();
    let duration = trace
        .iter()
        .filter(|value| trace_kind(value) != Some("plan"))
        .filter_map(trace_duration)
        .sum::<u64>();
    (thinking, tools, failed, (duration > 0).then_some(duration))
}

fn trace_summary_text(trace: &[serde_json::Value]) -> String {
    let (thinking, tools, failed, duration) = trace_summary_parts(trace);
    let mut parts = Vec::new();
    if thinking > 0 {
        parts.push(format!("思考 {thinking} 段"));
    }
    if tools > 0 {
        parts.push(format!(
            "工具 {tools} 次{}",
            if failed > 0 {
                format!("（{failed} 失败）")
            } else {
                String::new()
            }
        ));
    }
    if let Some(duration) = format_duration(duration) {
        parts.push(duration);
    }
    parts.join(" · ")
}

fn paint_plan(list: &mut DrawList, rect: Rect, value: &serde_json::Value, p: &Palette) {
    let radius = super::card_radius();
    list.rounded_rect(rect, radius, theme::mix(p.accent, p.surface, 0.04));
    list.rounded_border(rect, radius, p.border);
    list.icon_centered(
        Rect::new(
            rect.left + 10.0,
            rect.top + 8.0,
            rect.left + 26.0,
            rect.top + 24.0,
        ),
        Icon("ListChecks"),
        13.0,
        p.accent,
    );
    list.text(
        Rect::new(
            rect.left + 31.0,
            rect.top + 6.0,
            rect.right - 45.0,
            rect.top + 26.0,
        ),
        "执行计划",
        TextStyle::Small,
        p.accent,
    );
    let steps = plan_steps(value);
    let done = steps
        .iter()
        .filter(|step| step.get("status").and_then(|v| v.as_str()) == Some("done"))
        .count();
    list.text_aligned(
        Rect::new(
            rect.right - 44.0,
            rect.top + 6.0,
            rect.right - 10.0,
            rect.top + 26.0,
        ),
        format!("{done}/{}", steps.len()),
        TextStyle::Tiny,
        p.muted,
        Align::Trailing,
    );
    let mut y = rect.top + 31.0;
    for step in steps {
        let status = step
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");
        let icon = match status {
            "done" => Icon::CHECK,
            "in_progress" => Icon("Loader2"),
            "skipped" => Icon::MINUS,
            _ => Icon("CircleDashed"),
        };
        let color = match status {
            "done" => theme::mix(0x2E8B57, p.muted, 0.92),
            "in_progress" => p.accent,
            _ => p.muted,
        };
        list.icon_centered(
            Rect::new(rect.left + 11.0, y, rect.left + 25.0, y + 18.0),
            icon,
            12.0,
            color,
        );
        let title = step.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let title_color = if status == "done" {
            p.muted
        } else {
            p.foreground
        };
        list.text(
            Rect::new(rect.left + 31.0, y, rect.right - 10.0, y + 18.0),
            context::single_line_preview(title, TextStyle::Small, (rect.width() - 43.0).max(1.0)),
            TextStyle::Small,
            title_color,
        );
        y += 18.0 + 5.0;
    }
    if let Some(note) = value
        .get("note")
        .and_then(|v| v.as_str())
        .filter(|note| !note.is_empty())
    {
        paint_wrapped_text(
            list,
            note,
            Rect::new(rect.left + 12.0, y, rect.right - 12.0, rect.bottom - 6.0),
            TextStyle::Tiny,
            p.muted,
            Some(2),
        );
    }
}

fn paint_trace_step(
    list: &mut DrawList,
    rect: Rect,
    value: &serde_json::Value,
    expanded: bool,
    p: &Palette,
) {
    match trace_kind(value) {
        Some("thinking") => {
            list.icon_centered(
                Rect::new(rect.left, rect.top + 1.0, rect.left + 20.0, rect.top + 21.0),
                Icon("Brain"),
                13.0,
                p.muted,
            );
            let title = format_duration(trace_duration(value)).map_or_else(
                || "思考".to_owned(),
                |duration| format!("思考  ·  {duration}"),
            );
            list.text(
                Rect::new(rect.left + 28.0, rect.top, rect.right, rect.top + 20.0),
                title,
                TextStyle::Small,
                p.foreground,
            );
            let source = json_string(value, "text").unwrap_or_default();
            let shown = if expanded || source.chars().count() <= TRACE_MAX_COLLAPSED_CHARS {
                source
            } else {
                format!(
                    "{}…",
                    source
                        .chars()
                        .take(TRACE_MAX_COLLAPSED_CHARS)
                        .collect::<String>()
                )
            };
            paint_wrapped_text(
                list,
                &shown,
                Rect::new(rect.left + 28.0, rect.top + 21.0, rect.right, rect.bottom),
                TextStyle::Small,
                p.muted,
                None,
            );
        }
        Some("tool") => {
            let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            list.icon_centered(
                Rect::new(rect.left, rect.top + 1.0, rect.left + 20.0, rect.top + 21.0),
                if ok { Icon::CHECK } else { Icon::X },
                13.0,
                if ok { p.accent } else { p.danger },
            );
            list.icon_centered(
                Rect::new(
                    rect.left + 24.0,
                    rect.top + 3.0,
                    rect.left + 38.0,
                    rect.top + 19.0,
                ),
                Icon("Wrench"),
                11.0,
                p.muted,
            );
            let name = tool_label(json_string(value, "name").as_deref().unwrap_or("工具"));
            let duration = format_duration(trace_duration(value));
            let suffix = duration.map_or_else(String::new, |duration| format!(" · {duration}"));
            let title = format!("{name}{suffix}");
            list.text(
                Rect::new(
                    rect.left + 44.0,
                    rect.top,
                    rect.right - 10.0,
                    rect.top + 20.0,
                ),
                context::single_line_preview(
                    &title,
                    TextStyle::Small,
                    (rect.width() - 54.0).max(1.0),
                ),
                TextStyle::Small,
                p.foreground,
            );
            if let Some(summary) = json_string(value, "summary") {
                list.text(
                    Rect::new(
                        rect.left + 44.0,
                        rect.top + 20.0,
                        rect.right - 10.0,
                        rect.top + 39.0,
                    ),
                    context::single_line_preview(
                        &summary,
                        TextStyle::Tiny,
                        (rect.width() - 54.0).max(1.0),
                    ),
                    TextStyle::Tiny,
                    p.muted,
                );
            }
            if expanded {
                if let Some(preview) = json_string(value, "resultPreview")
                    .or_else(|| json_string(value, "result_preview"))
                {
                    paint_wrapped_text(
                        list,
                        &preview,
                        Rect::new(
                            rect.left + 28.0,
                            rect.top + 42.0,
                            rect.right - 8.0,
                            rect.bottom,
                        ),
                        TextStyle::Tiny,
                        if ok { p.muted } else { p.danger },
                        Some(4),
                    );
                }
            }
        }
        _ => {}
    }
}

pub(super) fn paint_trace_card(
    list: &mut DrawList,
    message: &LaidMessage,
    rect: Rect,
    s: &State,
    p: &Palette,
    index: usize,
) {
    let radius = super::card_radius();
    list.rounded_rect(rect, radius, theme::mix(p.accent, p.surface, 0.03));
    list.rounded_border(rect, radius, p.border);
    let toggle = Rect::new(rect.left, rect.top, rect.right, rect.top + TRACE_HEADER_H);
    list.icon_centered(
        Rect::new(
            toggle.left + 9.0,
            toggle.top + 7.0,
            toggle.left + 22.0,
            toggle.top + 22.0,
        ),
        if message.trace_expanded {
            Icon::CHEVRON_DOWN
        } else {
            Icon::CHEVRON_RIGHT
        },
        12.0,
        p.muted,
    );
    list.icon_centered(
        Rect::new(
            toggle.left + 27.0,
            toggle.top + 7.0,
            toggle.left + 42.0,
            toggle.top + 22.0,
        ),
        Icon("Brain"),
        13.0,
        p.muted,
    );
    list.text(
        Rect::new(
            toggle.left + 48.0,
            toggle.top + 4.0,
            toggle.right - 130.0,
            toggle.bottom - 4.0,
        ),
        "思考过程",
        TextStyle::Small,
        p.muted,
    );
    list.text_aligned(
        Rect::new(
            toggle.right - 120.0,
            toggle.top + 4.0,
            toggle.right - 10.0,
            toggle.bottom - 4.0,
        ),
        trace_summary_text(&message.trace),
        TextStyle::Tiny,
        p.muted,
        Align::Trailing,
    );
    if !message.trace_expanded {
        return;
    }
    list.hline(rect.left, rect.right, toggle.bottom, p.border);
    let mut y = toggle.bottom + TRACE_PAD_Y;
    for (step_index, value) in message.trace.iter().enumerate() {
        let h = match trace_kind(value) {
            Some("plan") => plan_height(value, rect.width() - TRACE_PAD_X * 2.0),
            Some("thinking") => thinking_height(
                value,
                rect.width() - TRACE_PAD_X * 2.0,
                s.expanded_trace_steps.contains(&(index, step_index)),
            ),
            Some("tool") => tool_height(
                value,
                rect.width() - TRACE_PAD_X * 2.0,
                s.expanded_trace_steps.contains(&(index, step_index)),
            ),
            _ => 0.0,
        };
        if h <= 0.0 {
            continue;
        }
        let row = Rect::new(rect.left + TRACE_PAD_X, y, rect.right - TRACE_PAD_X, y + h);
        match trace_kind(value) {
            Some("plan") => paint_plan(list, row, value, p),
            Some("thinking") | Some("tool") => paint_trace_step(
                list,
                row,
                value,
                s.expanded_trace_steps.contains(&(index, step_index)),
                p,
            ),
            _ => {}
        }
        y += h + TRACE_ROW_GAP;
    }
}

pub(super) fn paint_activity(
    list: &mut DrawList,
    message: &LaidMessage,
    rect: Rect,
    s: &State,
    p: &Palette,
) {
    let mut y = rect.top;
    if let Some(plan) = &message.live_plan {
        let h = plan_height(plan, rect.width());
        paint_plan(list, Rect::new(rect.left, y, rect.right, y + h), plan, p);
        y += h + 8.0;
    }
    let timeline = Rect::new(rect.left, y, rect.right, rect.bottom);
    let has_timeline = message
        .trace
        .iter()
        .any(|value| matches!(trace_kind(value), Some("thinking" | "tool")))
        || message.streaming_reasoning.is_some()
        || message.active_tool.is_some()
        || message.streaming_status.is_some();
    if !has_timeline {
        return;
    }
    let radius = super::card_radius();
    list.rounded_rect(timeline, radius, theme::mix(p.accent, p.surface, 0.04));
    list.rounded_border(timeline, radius, p.border);
    let inner = Rect::new(
        timeline.left + TRACE_PAD_X,
        timeline.top + TRACE_PAD_Y,
        timeline.right - TRACE_PAD_X,
        timeline.bottom - TRACE_PAD_Y,
    );
    let mut row_y = inner.top;
    for (step_index, value) in message.trace.iter().enumerate() {
        if trace_kind(value) == Some("plan") {
            continue;
        }
        // 实时 Thinking 行沿用与 reasoning 尾部相同的折叠/展开偏好。
        // 轨迹生成期间，工具结果保持足以显示摘要的展开状态。
        let expanded = trace_kind(value) != Some("thinking") || s.streaming_reasoning_expanded;
        let h = match trace_kind(value) {
            Some("thinking") => thinking_height(value, inner.width(), expanded),
            Some("tool") => tool_height(value, inner.width(), expanded),
            _ => 0.0,
        };
        if h <= 0.0 {
            continue;
        }
        let row = Rect::new(inner.left, row_y, inner.right, row_y + h);
        paint_trace_step(list, row, value, expanded, p);
        row_y += h + TRACE_ROW_GAP;
        let _ = step_index;
    }
    if let Some(reasoning) = message
        .streaming_reasoning
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        let shown = streaming_reasoning_text(reasoning, s.streaming_reasoning_expanded);
        let h = 22.0
            + wrapped_height(
                &shown,
                (inner.width() - 28.0).max(32.0),
                TextStyle::Small,
                (!s.streaming_reasoning_expanded).then_some(18),
            );
        let row = Rect::new(inner.left, row_y, inner.right, row_y + h);
        list.icon_centered(
            Rect::new(row.left, row.top + 1.0, row.left + 20.0, row.top + 21.0),
            Icon("Brain"),
            13.0,
            p.accent,
        );
        list.text(
            Rect::new(row.left + 28.0, row.top, row.right, row.top + 20.0),
            if s.streaming_reasoning_expanded {
                "思考过程"
            } else {
                "正在思考"
            },
            TextStyle::Small,
            p.accent,
        );
        paint_wrapped_text(
            list,
            &shown,
            Rect::new(row.left + 28.0, row.top + 21.0, row.right, row.bottom),
            TextStyle::Small,
            p.muted,
            (!s.streaming_reasoning_expanded).then_some(18),
        );
        row_y += h + TRACE_ROW_GAP;
    }
    if let Some((name, summary)) = &message.active_tool {
        let row = Rect::new(inner.left, row_y, inner.right, row_y + 24.0);
        let spinner = Rect::new(row.left, row.top + 1.0, row.left + 20.0, row.top + 21.0);
        list.spinner(
            spinner,
            s.streaming
                .as_ref()
                .map_or(0, |streaming| streaming.animation_phase),
            p.accent,
        );
        list.icon_centered(
            Rect::new(
                row.left + 24.0,
                row.top + 3.0,
                row.left + 38.0,
                row.top + 19.0,
            ),
            Icon("Wrench"),
            11.0,
            p.muted,
        );
        list.text(
            Rect::new(row.left + 44.0, row.top, row.right - 10.0, row.bottom),
            context::single_line_preview(
                &format!(
                    "{}{}",
                    tool_label(name),
                    summary
                        .as_ref()
                        .map_or(String::new(), |summary| format!(" · {summary}"))
                ),
                TextStyle::Small,
                (row.width() - 54.0).max(1.0),
            ),
            TextStyle::Small,
            p.foreground,
        );
        row_y += 24.0 + TRACE_ROW_GAP;
    }
    if row_y <= inner.top + 1.0 {
        if let Some(status) = message
            .streaming_status
            .as_deref()
            .filter(|status| !status.is_empty())
        {
            list.spinner(
                Rect::new(inner.left, row_y, inner.left + 20.0, row_y + 20.0),
                s.streaming
                    .as_ref()
                    .map_or(0, |streaming| streaming.animation_phase),
                p.accent,
            );
            list.text(
                Rect::new(inner.left + 28.0, row_y, inner.right, row_y + 20.0),
                status,
                TextStyle::Small,
                p.muted,
            );
        }
    }
}
