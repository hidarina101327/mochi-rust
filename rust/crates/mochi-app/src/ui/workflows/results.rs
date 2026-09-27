//! 整理工作流运行产物、文件路径和耗时信息。
use super::chrome::{icon_button, panel_header};
use super::painting::{button, status};
use super::*;
use crate::ui::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    text,
    theme::Palette,
};
use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub(super) struct Artifact {
    pub path: String,
    pub preview: String,
}

fn file_path(value: &str) -> bool {
    !value.contains(['\n', '\r'])
        && !value.contains("://")
        && value.len() < 4096
        && std::path::Path::new(value)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                [
                    "md", "mc", "txt", "pdf", "docx", "xlsx", "csv", "json", "svg", "png", "jpg",
                    "html", "mcb",
                ]
                .contains(&e.to_ascii_lowercase().as_str())
            })
}

/// 生成的内容来自各节点的实际输出，包括中途写入文档的节点。
/// 仅出现在输入中的路径绝不显示为生成的文档。
pub(super) fn artifacts(run: &Run, selected: Option<usize>) -> Vec<Artifact> {
    fn paths(value: &Value, found: &mut Vec<String>, depth: usize) {
        if depth > 8 {
            return;
        }
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if [
                        "path",
                        "file",
                        "filePath",
                        "file_path",
                        "report",
                        "chart",
                        "document",
                    ]
                    .contains(&key.as_str())
                    {
                        if let Some(path) = value.as_str().filter(|path| file_path(path)) {
                            found.push(path.into());
                        }
                    }
                    paths(value, found, depth + 1);
                }
            }
            Value::Array(values) => {
                for value in values.iter().take(100) {
                    paths(value, found, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut result: Vec<Artifact> = Vec::new();
    let nodes: Vec<_> = run
        .definition
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, _)| selected.is_none_or(|index| index == *i))
        .collect();
    for (_, node) in nodes {
        let Some(state) = run.nodes.get(&node.id) else {
            continue;
        };
        if state.status != "succeeded" {
            continue;
        }
        let mut found = Vec::new();
        // 脚本可能会返回文件名；结构化对象输出也会按统一方式处理。
        if matches!(
            node.kind.as_str(),
            "file_write" | "document_append" | "base_write" | "script" | "end" | "tool"
        ) {
            paths(&state.output, &mut found, 0);
        }
        for path in found {
            let norm = path.replace('\\', "/");
            let existing = result.iter_mut().find(|a| {
                let old = a.path.replace('\\', "/");
                old == norm
                    || old.ends_with(&format!("/{norm}"))
                    || norm.ends_with(&format!("/{old}"))
            });
            let preview: String = state
                .input
                .get("content")
                .and_then(Value::as_str)
                .or_else(|| state.output.pointer("/data/report").and_then(Value::as_str))
                .unwrap_or("")
                .chars()
                .take(600)
                .collect();
            if let Some(existing) = existing {
                if matches!(node.kind.as_str(), "file_write" | "document_append") {
                    existing.path = path;
                    existing.preview = preview;
                } else if existing.preview.is_empty() {
                    existing.preview = preview;
                }
                continue;
            }
            result.push(Artifact { path, preview });
        }
    }
    if selected.is_none() {
        let mut found = Vec::new();
        paths(&run.output, &mut found, 0);
        for path in found {
            let norm = path.replace('\\', "/");
            if !result
                .iter()
                .any(|a| a.path.replace('\\', "/").ends_with(&norm))
            {
                result.push(Artifact {
                    path,
                    preview: String::new(),
                });
            }
        }
    }
    result.truncate(32);
    result
}

pub(super) fn source_label(source: &str) -> &str {
    match source {
        "schedule" => "定时运行",
        "manual" => "手动运行",
        "agent" => "AI 发起",
        "cli" => "命令行运行",
        _ => "运行记录",
    }
}
fn elapsed(start: i64, end: Option<i64>) -> String {
    match end {
        Some(end) => {
            let ms = end.saturating_sub(start);
            if ms < 1000 {
                format!("{ms} ms")
            } else {
                format!("{:.1} 秒", ms as f64 / 1000.)
            }
        }
        None => "进行中".into(),
    }
}
fn time(stamp: i64) -> String {
    chrono::DateTime::from_timestamp_millis(stamp)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}
fn state_icon(value: &str) -> Icon {
    match value {
        "succeeded" => Icon::CHECK_CIRCLE2,
        "failed" | "partial" => Icon::ALERT_CIRCLE,
        "running" => Icon::LOADER2,
        "cancelled" | "interrupted" => Icon::SQUARE,
        "skipped" => Icon::MINUS,
        _ => Icon::CLOCK,
    }
}
fn state_color(value: &str, p: &Palette) -> u32 {
    match value {
        "failed" | "partial" => p.danger,
        "succeeded" => 0x169b78,
        "running" => 0x557cc0,
        _ => p.muted,
    }
}

impl State {
    pub fn show_run(&mut self, run: Run) {
        if self.run.is_none() {
            self.run_palette = self.palette;
        }
        self.palette = false;
        self.run = Some(run);
        self.result_hidden = false;
        self.result_tab = 0;
        self.panel_scroll = 0.;
        self.history_open = false;
        self.settings_open = false;
        self.selected_node = None;
        self.editor = None;
        self.needs_fit = true;
        self.refresh_result_text();
    }
    pub fn refresh_result_text(&mut self) {
        let Some(run) = &self.run else { return };
        let value = if let Some(node) = self.selected_node.and_then(|i| run.definition.nodes.get(i))
        {
            json!({"node":node.label, "id":node.id, "result":run.nodes.get(&node.id)})
        } else {
            json!({"status":run.status,"input":run.input,"output":run.output,"error":run.error,"nodes":run.nodes})
        };
        let text = serde_json::to_string_pretty(&value).unwrap_or_default();
        if self.result_text != text {
            self.result_text = text;
            self.result_page = self.result_page.min(
                self.result_text
                    .chars()
                    .count()
                    .div_ceil(8000)
                    .saturating_sub(1),
            );
            if self.result_tab == 2 {
                self.show_result_page();
            }
        }
    }
}

pub(super) fn history(list: &mut DrawList, s: &mut State, p: &Palette) {
    panel_header(list, s, Icon::HISTORY, "运行历史", p);
    let r = s.inspector;
    list.text(
        Rect::from_size(r.left + 18., r.top + 64., r.width() - 36., 22.),
        format!("最近 {} 次运行", s.history.len()),
        TextStyle::Caption,
        p.muted,
    );
    let body = Rect::new(r.left + 8., r.top + 96., r.right - 8., r.bottom - 8.);
    s.panel_body = body;
    s.panel_height = s.history.len() as f32 * 72.;
    s.panel_scroll = s
        .panel_scroll
        .clamp(0., (s.panel_height - body.height()).max(0.));
    list.push_clip(body);
    if s.history.is_empty() {
        list.icon_centered(
            Rect::from_size(body.left + 16., body.top + 20., 28., 28.),
            Icon::CLOCK,
            22.,
            p.muted,
        );
        list.text(
            Rect::from_size(body.left + 54., body.top + 18., body.width() - 66., 24.),
            "暂无运行记录",
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::from_size(body.left + 54., body.top + 46., body.width() - 66., 22.),
            "运行流程后会显示在这里",
            TextStyle::Caption,
            p.muted,
        );
    }
    for (i, run) in s.history.clone().iter().enumerate() {
        let row = Rect::from_size(
            body.left,
            body.top + i as f32 * 72. - s.panel_scroll,
            body.width(),
            66.,
        );
        if row.intersect(&body).is_empty() {
            continue;
        }
        if s.hover == Some(Hit::RunHistory(i)) {
            list.rounded_rect(row, 8., p.surface_muted);
        }
        let color = state_color(&run.status, p);
        list.icon_centered(
            Rect::from_size(row.left + 10., row.top + 18., 24., 24.),
            state_icon(&run.status),
            18.,
            color,
        );
        list.text(
            Rect::new(row.left + 46., row.top + 8., row.right - 64., row.top + 32.),
            time(run.started_at),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(row.right - 60., row.top + 8., row.right - 8., row.top + 32.),
            status(&run.status),
            TextStyle::Tiny,
            color,
        );
        list.text(
            Rect::new(
                row.left + 46.,
                row.top + 34.,
                row.right - 30.,
                row.top + 54.,
            ),
            format!(
                "{} · {}",
                source_label(&run.source),
                elapsed(run.started_at, run.finished_at)
            ),
            TextStyle::Tiny,
            p.muted,
        );
        list.icon_centered(
            Rect::from_size(row.right - 26., row.top + 36., 16., 16.),
            Icon::CHEVRON_RIGHT,
            13.,
            p.muted,
        );
        list.hline(row.left + 46., row.right - 10., row.bottom + 2., p.border);
        s.hits.push((row.intersect(&body), Hit::RunHistory(i)));
    }
    list.pop_clip();
    scrollbar(list, s, p);
}

fn wrapped(
    list: &mut DrawList,
    area: Rect,
    value: &str,
    style: TextStyle,
    color: u32,
    max_lines: usize,
) -> f32 {
    let mut y = area.top;
    let mut count = 0;
    // 在这里逐行换行，每行使用固定高度的矩形。DirectWrite
    // 绝不能将段落垂直居中到面板剩余的高度中。
    for paragraph in value.lines() {
        for line in text::wrap_source(paragraph, style, area.width()) {
            if count >= max_lines {
                return y - area.top;
            }
            let content: String = line.iter().map(|r| r.text.as_str()).collect();
            list.text(
                Rect::from_size(area.left, y, area.width(), 22.),
                content,
                style,
                color,
            );
            y += 22.;
            count += 1;
        }
        if paragraph.is_empty() {
            y += 10.;
        }
    }
    (y - area.top).max(22.)
}

fn value_card(
    list: &mut DrawList,
    left: f32,
    right: f32,
    y: f32,
    key: &str,
    value: &Value,
    p: &Palette,
) -> f32 {
    let content = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default());
    let content: String = content.chars().take(1800).collect();
    list.text(
        Rect::new(left, y, right, y + 24.),
        key,
        TextStyle::Caption,
        p.muted,
    );
    let h = content
        .lines()
        .map(|line| {
            text::wrap_source(line, TextStyle::Label, right - left - 24.)
                .len()
                .max(1)
        })
        .sum::<usize>()
        .clamp(1, 10) as f32
        * 22.;
    // 正文绘制在简洁的内嵌底色上，不显示原生编辑框的外观。
    let rect = Rect::new(left, y + 28., right, y + 46. + h);
    list.rounded_rect(rect, 8., p.surface_muted);
    wrapped(
        list,
        Rect::new(left + 12., y + 34., right - 12., y + 300.),
        &content,
        TextStyle::Label,
        p.foreground,
        10,
    );
    h + 60.
}

pub(super) fn paint(list: &mut DrawList, s: &mut State, p: &Palette) {
    let Some(run) = s.run.clone() else { return };
    let selected = s.selected_node.and_then(|i| run.definition.nodes.get(i));
    let node_run = selected.and_then(|n| run.nodes.get(&n.id));
    let r = s.inspector;
    let title = selected.map(|n| n.label.as_str()).unwrap_or("运行结果");
    panel_header(
        list,
        s,
        if selected.is_some() {
            Icon::BOXES
        } else {
            Icon::FILE_TEXT
        },
        title,
        p,
    );
    if selected.is_some() {
        icon_button(
            list,
            s,
            Rect::from_size(r.right - 80., r.top + 12., 32., 32.),
            Icon::ARROW_LEFT,
            "返回运行总览",
            Hit::ResultOverview,
            p,
        );
    }
    let state = node_run.map(|n| n.status.as_str()).unwrap_or(&run.status);
    let color = state_color(state, p);
    list.icon_centered(
        Rect::from_size(r.left + 18., r.top + 69., 22., 22.),
        state_icon(state),
        18.,
        color,
    );
    list.text(
        Rect::from_size(r.left + 48., r.top + 65., r.width() - 66., 26.),
        status(state),
        TextStyle::Label,
        color,
    );
    let duration = node_run
        .and_then(|n| n.started_at)
        .map(|start| elapsed(start, node_run.and_then(|n| n.finished_at)))
        .unwrap_or_else(|| elapsed(run.started_at, run.finished_at));
    list.text(
        Rect::from_size(r.left + 18., r.top + 99., r.width() - 36., 22.),
        format!("{} · {}", time(run.started_at), duration),
        TextStyle::Tiny,
        p.muted,
    );
    let tab_y = r.top + 134.;
    let tab_width = (r.width() - 24.) / 3.;
    for (i, title) in ["输出结果", "输入参数", "原始 JSON"].iter().enumerate() {
        let tab = Rect::from_size(r.left + 12. + i as f32 * tab_width, tab_y, tab_width, 34.);
        if s.result_tab == i as u8 {
            list.rounded_rect(tab, 7., p.surface_muted);
        }
        button(list, s, tab, title, Hit::ResultTab(i as u8), p, false);
    }
    let body = Rect::new(r.left + 18., tab_y + 48., r.right - 18., r.bottom - 54.);
    s.panel_body = body;
    let footer = Rect::new(r.left, r.bottom - 46., r.right, r.bottom);
    list.hline(r.left, r.right, footer.top, p.border);
    icon_button(
        list,
        s,
        Rect::from_size(r.left + 12., footer.top + 6., 32., 32.),
        Icon::COPY,
        "复制完整结果",
        Hit::CopyResult,
        p,
    );
    list.text(
        Rect::from_size(r.left + 50., footer.top + 6., 110., 32.),
        "复制完整结果",
        TextStyle::Tiny,
        p.muted,
    );
    s.hits.push((
        Rect::from_size(r.left + 12., footer.top + 6., 148., 32.),
        Hit::CopyResult,
    ));
    if s.result_tab == 2 {
        s.field_rect = body;
        s.field.paint_multiline(list, body, false, p);
        let count = s.result_text.chars().count().div_ceil(8000).max(1);
        if count > 1 {
            button(
                list,
                s,
                Rect::from_size(r.right - 124., footer.top + 6., 28., 32.),
                "‹",
                Hit::ResultPage(false),
                p,
                false,
            );
            list.text(
                Rect::from_size(r.right - 90., footer.top + 6., 50., 32.),
                format!("{}/{}", s.result_page + 1, count),
                TextStyle::Tiny,
                p.muted,
            );
            button(
                list,
                s,
                Rect::from_size(r.right - 40., footer.top + 6., 28., 32.),
                "›",
                Hit::ResultPage(true),
                p,
                false,
            );
        }
        return;
    }
    list.push_clip(body);
    let mut y = body.top - s.panel_scroll;
    if let Some(error) = node_run
        .and_then(|n| n.error.as_deref())
        .or(run.error.as_deref())
    {
        y += wrapped(
            list,
            Rect::new(body.left, y, body.right, y + 200.),
            error,
            TextStyle::Caption,
            p.danger,
            8,
        ) + 16.;
    }
    if s.result_tab == 0 {
        let files = artifacts(&run, s.selected_node);
        if !files.is_empty() {
            list.text(
                Rect::from_size(body.left, y, body.width(), 24.),
                "生成的文件",
                TextStyle::Label,
                p.foreground,
            );
            y += 34.;
            for file in &files {
                let card = Rect::from_size(body.left, y, body.width(), 70.);
                list.rounded_rect(
                    card,
                    9.,
                    if s.hover == Some(Hit::OpenArtifact(file.path.clone())) {
                        p.surface_muted
                    } else {
                        p.surface
                    },
                );
                list.rounded_border(card, 9., p.border);
                list.icon_centered(
                    Rect::from_size(card.left + 12., card.top + 18., 30., 30.),
                    Icon::FILE_TEXT,
                    23.,
                    p.muted,
                );
                let normalized = file.path.replace('\\', "/");
                let name = normalized.rsplit('/').next().unwrap_or(&normalized);
                list.text(
                    Rect::new(card.left + 52., y + 10., card.right - 32., y + 34.),
                    text::ellipsize(name, TextStyle::Label, card.width() - 88.),
                    TextStyle::Label,
                    p.foreground,
                );
                list.text(
                    Rect::new(card.left + 52., y + 37., card.right - 32., y + 57.),
                    "点击打开文档",
                    TextStyle::Tiny,
                    p.muted,
                );
                list.icon_centered(
                    Rect::from_size(card.right - 28., y + 26., 18., 18.),
                    Icon::ARROW_UP,
                    14.,
                    p.muted,
                );
                s.hits
                    .push((card.intersect(&body), Hit::OpenArtifact(file.path.clone())));
                y += 80.;
                if !file.preview.is_empty() {
                    let preview = file
                        .preview
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .map(|line| {
                            let trimmed = line.trim_start();
                            let heading = trimmed.trim_start_matches('#');
                            if heading.len() != trimmed.len() && heading.starts_with(' ') {
                                heading.trim_start()
                            } else {
                                trimmed
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    y += wrapped(
                        list,
                        Rect::new(body.left + 4., y, body.right - 4., y + 140.),
                        &preview,
                        TextStyle::Caption,
                        p.muted,
                        4,
                    ) + 18.;
                }
            }
        }
    }
    let value = if s.result_tab == 1 {
        node_run.map(|n| &n.input).unwrap_or(&run.input)
    } else {
        node_run.map(|n| &n.output).unwrap_or(&run.output)
    };
    if let Some(object) = value.as_object().filter(|o| !o.is_empty()) {
        for (key, value) in object.iter().take(30) {
            if s.result_tab == 0
                && ["report", "path", "file", "filePath"].contains(&key.as_str())
                && value.as_str().is_some_and(file_path)
            {
                continue;
            }
            y += value_card(list, body.left, body.right, y, key, value, p);
        }
    } else if !value.is_null() {
        y += value_card(
            list,
            body.left,
            body.right,
            y,
            if s.result_tab == 0 {
                "输出"
            } else {
                "输入"
            },
            value,
            p,
        );
    } else {
        let message = if matches!(state, "queued" | "pending" | "running") {
            "结果会在节点完成后自动显示"
        } else if state == "skipped" {
            "此分支未执行，没有输出"
        } else {
            "本次运行没有返回输出值"
        };
        y += wrapped(
            list,
            Rect::new(body.left, y, body.right, y + 60.),
            message,
            TextStyle::Caption,
            p.muted,
            3,
        ) + 16.;
    }
    if selected.is_none() && s.result_tab == 0 {
        list.text(
            Rect::from_size(body.left, y + 4., body.width(), 28.),
            "节点执行",
            TextStyle::Label,
            p.foreground,
        );
        y += 42.;
        for (i, node) in run.definition.nodes.iter().enumerate() {
            let Some(nr) = run.nodes.get(&node.id) else {
                continue;
            };
            let row = Rect::from_size(body.left, y, body.width(), 48.);
            if s.hover == Some(Hit::Node(i)) {
                list.rounded_rect(row, 7., p.surface_muted);
            }
            list.icon_centered(
                Rect::from_size(row.left + 4., y + 12., 22., 22.),
                state_icon(&nr.status),
                15.,
                state_color(&nr.status, p),
            );
            list.text(
                Rect::new(row.left + 34., y + 2., row.right - 60., y + 28.),
                text::ellipsize(&node.label, TextStyle::Caption, row.width() - 98.),
                TextStyle::Caption,
                p.foreground,
            );
            list.text(
                Rect::new(row.left + 34., y + 26., row.right - 60., y + 44.),
                nr.started_at
                    .map(|start| elapsed(start, nr.finished_at))
                    .unwrap_or_else(|| status(&nr.status).into()),
                TextStyle::Tiny,
                p.muted,
            );
            list.text(
                Rect::from_size(row.right - 56., y + 9., 56., 26.),
                status(&nr.status),
                TextStyle::Tiny,
                state_color(&nr.status, p),
            );
            s.hits.push((row.intersect(&body), Hit::Node(i)));
            y += 54.;
        }
    }
    s.panel_height = y + s.panel_scroll - body.top + 12.;
    s.panel_scroll = s
        .panel_scroll
        .clamp(0., (s.panel_height - body.height()).max(0.));
    list.pop_clip();
    scrollbar(list, s, p);
}

fn scrollbar(list: &mut DrawList, s: &State, p: &Palette) {
    let body = s.panel_body;
    if s.panel_height <= body.height() {
        return;
    }
    let height = (body.height() * body.height() / s.panel_height).max(30.);
    let y = body.top + (body.height() - height) * s.panel_scroll / (s.panel_height - body.height());
    list.rounded_rect(
        Rect::from_size(s.inspector.right - 6., y, 3., height),
        1.5,
        p.border,
    );
}
