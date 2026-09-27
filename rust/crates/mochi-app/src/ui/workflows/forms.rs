//! 整理工作流参数表单的字段、选项和值。
use super::painting::button;
use super::*;
use crate::ui::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    text,
    theme::Palette,
};
use serde_json::Value;

pub fn parameter_choices(path: &str) -> Vec<(&'static str, &'static str)> {
    match path {
        "/workflow/trigger/type" => vec![
            ("manual", "手动触发"),
            ("daily", "每天"),
            ("weekly", "每周"),
            ("interval", "固定间隔"),
        ],
        "/on_error" => vec![("stop", "停止工作流"), ("continue", "继续后续节点")],
        "/config/language" => vec![
            ("python", "Python"),
            ("powershell", "PowerShell"),
            ("cmd", "CMD"),
        ],
        "/config/operator" => vec![
            ("equals", "等于"),
            ("not_equals", "不等于"),
            ("greater", "大于"),
            ("less", "小于"),
            ("contains", "包含"),
            ("exists", "存在"),
            ("truthy", "为真"),
        ],
        "/config/method" => vec![("GET", "GET"), ("POST", "POST")],
        "/config/response_format" => vec![("text", "文本"), ("json", "JSON")],
        "/config/extract" => vec![("text", "正文文本"), ("raw", "原始响应"), ("json", "JSON")],
        _ => vec![],
    }
}
fn section(path: &str) -> &'static str {
    if path == "/label" {
        "基本设置"
    } else if path.starts_with("/config/") || path.starts_with("/workflow/") {
        "节点参数"
    } else if path.starts_with("/inputs/") {
        "输入变量"
    } else {
        "运行设置"
    }
}

fn caption(key: &str) -> &str {
    match key {
        "label" => "节点名称",
        "provider_id" => "模型提供商",
        "system" => "System Prompt",
        "prompt" => "Prompt / 输入内容",
        "language" => "脚本语言",
        "code" => "代码",
        "summary" => "脚本说明",
        "url" => "请求地址",
        "method" => "请求方法",
        "extract" => "提取方式",
        "path" => "文件路径",
        "content" => "内容",
        "left" => "左侧变量",
        "right" => "比较值",
        "operator" => "条件运算符",
        "result" => "输出结果",
        "enabled" => "启用节点",
        "retries" => "失败重试次数",
        "timeout_ms" => "超时（毫秒）",
        "on_error" => "失败策略",
        "overwrite" => "覆盖已有文件",
        "mark_unread" => "产出文档标为未读",
        "response_format" => "输出格式",
        "object" => "墨池对象链接",
        "title" => "标题",
        "message" => "通知内容",
        _ => key,
    }
}
pub fn fields(n: &Node) -> Vec<(String, String, Value)> {
    let mut rows = vec![(
        "/label".into(),
        "基本设置 · 节点名称".into(),
        Value::String(n.label.clone()),
    )];
    for (section, object, label) in [
        ("config", &n.config, "参数"),
        ("inputs", &n.inputs, "输入变量"),
    ] {
        if let Some(object) = object.as_object() {
            for (key, value) in object {
                rows.push((
                    format!("/{section}/{}", key.replace('~', "~0").replace('/', "~1")),
                    format!("{label} · {}", caption(key)),
                    value.clone(),
                ));
            }
        }
    }
    if matches!(n.kind.as_str(), "file_write" | "document_append" | "script")
        && n.config.get("mark_unread").is_none()
    {
        rows.push((
            "/config/mark_unread".into(),
            "参数 · 产出文档标为未读".into(),
            Value::Bool(false),
        ));
    }
    rows.extend([
        ("/enabled".into(), "启用节点".into(), Value::Bool(n.enabled)),
        (
            "/retries".into(),
            "失败重试次数".into(),
            Value::from(n.retries),
        ),
        (
            "/timeout_ms".into(),
            "超时（毫秒）".into(),
            Value::from(n.timeout_ms),
        ),
        (
            "/on_error".into(),
            caption("on_error").into(),
            Value::String(n.on_error.clone()),
        ),
    ]);
    rows
}
fn form_fields(s: &State, n: &Node) -> Vec<(String, String, Value)> {
    let mut rows = fields(n);
    if n.kind == "start" {
        if let Some(g) = &s.draft {
            let trigger = serde_json::to_value(&g.trigger).unwrap_or_default();
            let mut extra = vec![(
                "/workflow/trigger/type".into(),
                "触发方式".into(),
                trigger["type"].clone(),
            )];
            for (key, label) in [
                ("time", "执行时间（HH:MM）"),
                ("minutes", "间隔（分钟）"),
                ("weekdays", "星期（1–7，JSON 数组）"),
                ("utc_offset_minutes", "时区偏移（分钟，北京为 480）"),
            ] {
                if let Some(value) = trigger.get(key) {
                    extra.push((
                        format!("/workflow/trigger/{key}"),
                        label.into(),
                        value.clone(),
                    ));
                }
            }
            rows.splice(1..1, extra);
        }
    }
    rows
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default())
}
fn field_height(path: &str, value: &Value) -> f32 {
    if value_text(value).contains('\n')
        || ["/prompt", "/code", "/system"]
            .iter()
            .any(|key| path.ends_with(key))
        || value.is_object()
        || value.is_array()
    {
        116.
    } else {
        40.
    }
}
impl State {
    pub fn edit_parameter(&mut self, path: &str) -> Result<(), String> {
        self.apply_node()?;
        let n = self
            .selected_node
            .and_then(|i| self.draft.as_ref()?.nodes.get(i))
            .ok_or("请先选择节点")?;
        let mut raw = if path.starts_with("/workflow/") {
            serde_json::to_value(self.draft.as_ref().unwrap())
        } else {
            serde_json::to_value(n)
        }
        .map_err(|e| e.to_string())?;
        if path == "/config/mark_unread"
            && matches!(n.kind.as_str(), "script" | "file_write" | "document_append")
            && raw.pointer(path).is_none()
        {
            raw["config"]["mark_unread"] = Value::Bool(false);
        }
        let pointer = path.strip_prefix("/workflow").unwrap_or(path);
        let value = raw.pointer(pointer).ok_or("参数不存在")?;
        let text = value_text(value);
        let rows = form_fields(self, n);
        let visible = self.inspector.height() - 202.;
        self.folded.remove(section(path));
        if visible > 120. {
            let mut top = 8.;
            let mut previous = "";
            for (key, _, value) in rows {
                let group = section(&key);
                if group != previous {
                    top += 40.;
                    previous = group;
                }
                if self.folded.contains(group) {
                    continue;
                }
                let height = field_height(&key, &value);
                if key == path {
                    if top < self.form_scroll {
                        self.form_scroll = top;
                    } else if top + 22. + height > self.form_scroll + visible {
                        self.form_scroll = top + 22. + height - visible;
                    }
                    break;
                }
                top += height + 38.;
            }
        }
        self.parameter = path.into();
        self.set_editor(Editor::Parameter, text);
        Ok(())
    }
    pub fn apply_parameter(&mut self) -> Result<(), String> {
        if self.field.text() == self.field_before {
            return Ok(());
        }
        if self.parameter.starts_with("/workflow/trigger/") {
            let g = self.draft.as_ref().ok_or("工作流不存在")?;
            let mut raw = serde_json::to_value(&g.trigger).map_err(|e| e.to_string())?;
            let key = self.parameter.trim_start_matches("/workflow/trigger/");
            if key == "type" {
                let offset = raw["utc_offset_minutes"].as_i64().unwrap_or(480);
                raw = match self.field.text() {
                    "manual" => serde_json::json!({"type":"manual"}),
                    "daily" => {
                        serde_json::json!({"type":"daily","time":"09:00","utc_offset_minutes":offset})
                    }
                    "weekly" => {
                        serde_json::json!({"type":"weekly","time":"09:00","weekdays":[1],"utc_offset_minutes":offset})
                    }
                    "interval" => serde_json::json!({"type":"interval","minutes":60}),
                    _ => return Err("不支持的触发方式".into()),
                };
            } else {
                raw[key] = if raw[key].is_string() {
                    Value::String(self.field.text().into())
                } else {
                    serde_json::from_str(self.field.text()).map_err(|e| e.to_string())?
                };
            }
            let trigger = serde_json::from_value(raw).map_err(|e| e.to_string())?;
            let mut probe = Workflow::blank();
            probe.trigger = trigger;
            mochi_core::workflows::validate(&probe)?;
            self.checkpoint();
            self.draft.as_mut().unwrap().trigger = probe.trigger;
            self.field_before = self.field.text().into();
            return Ok(());
        }
        let i = self.selected_node.ok_or("请先选择节点")?;
        let n = self
            .draft
            .as_ref()
            .and_then(|g| g.nodes.get(i))
            .ok_or("节点不存在")?;
        let mut raw = serde_json::to_value(n).map_err(|e| e.to_string())?;
        if self.parameter == "/config/mark_unread"
            && matches!(n.kind.as_str(), "script" | "file_write" | "document_append")
            && raw.pointer(&self.parameter).is_none()
        {
            raw["config"]["mark_unread"] = Value::Bool(false);
        }
        let original = raw.pointer_mut(&self.parameter).ok_or("参数不存在")?;
        let reference = |text: &str| {
            ["$input", "$nodes.", "$run.", "$item"]
                .iter()
                .any(|prefix| text.starts_with(prefix))
        };
        let value = if self.parameter.starts_with("/inputs/") && reference(self.field.text()) {
            Value::String(self.field.text().into())
        } else if self.parameter.starts_with("/inputs/") && original.as_str().is_some_and(reference)
        {
            serde_json::from_str(self.field.text())
                .unwrap_or_else(|_| Value::String(self.field.text().into()))
        } else if original.is_string() {
            Value::String(self.field.text().into())
        } else {
            let parsed: Value = serde_json::from_str(self.field.text())
                .map_err(|_| "请输入有效的数字、布尔值或 JSON".to_string())?;
            if (original.is_boolean() && !parsed.is_boolean())
                || (original.is_number() && !parsed.is_number())
                || (original.is_object() && !parsed.is_object())
                || (original.is_array() && !parsed.is_array())
            {
                return Err("参数类型与原字段不匹配".into());
            }
            parsed
        };
        *original = value;
        let updated: Node = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        if updated.label.trim().is_empty() {
            return Err("节点名称不能为空".into());
        }
        self.checkpoint();
        self.draft.as_mut().unwrap().nodes[i] = updated;
        self.field_before = self.field.text().into();
        Ok(())
    }
}
pub fn inspector(list: &mut DrawList, s: &mut State, focused: bool, p: &Palette) {
    let Some(n) = s
        .selected_node
        .and_then(|i| s.graph()?.nodes.get(i))
        .cloned()
    else {
        return;
    };
    let r = s.inspector;
    let (_, color, icon) = super::modern::category(&n.kind);
    let badge = Rect::from_size(r.left + 16., r.top + 16., 30., 30.);
    list.rounded_rect(badge, 7., color);
    list.icon_centered(badge, icon, 18., 0xffffff);
    list.text(
        Rect::new(r.left + 56., r.top + 12., r.right - 45., r.top + 37.),
        text::ellipsize(&n.label, TextStyle::Label, r.width() - 108.),
        TextStyle::Label,
        p.foreground,
    );
    super::chrome::icon_button(
        list,
        s,
        Rect::from_size(r.right - 42., r.top + 9., 32., 32.),
        Icon::X,
        "关闭节点配置",
        Hit::CloseEditor,
        p,
    );
    list.text(
        Rect::new(r.left + 56., r.top + 35., r.right - 45., r.top + 55.),
        mochi_core::workflows::catalog::label(&n.kind),
        TextStyle::Tiny,
        p.muted,
    );
    for (i, (label, history)) in [("配置", false), ("运行记录", true)]
        .into_iter()
        .enumerate()
    {
        let tab = Rect::from_size(r.left + 16. + i as f32 * 100., r.top + 64., 88., 32.);
        button(list, s, tab, label, Hit::ConfigTab(history), p, false);
        if s.node_history == history {
            list.rect(
                Rect::from_size(tab.left, tab.bottom - 2., tab.width(), 2.),
                p.accent,
            );
        }
    }
    list.hline(r.left, r.right, r.top + 98., p.border);
    let body = Rect::new(r.left, r.top + 108., r.right, r.bottom - 94.);
    if s.node_history {
        s.form_height = 24. + s.history.len() as f32 * 58.;
        list.push_clip(body);
        if s.history.is_empty() {
            list.text(
                Rect::new(
                    body.left + 20.,
                    body.top + 50.,
                    body.right - 20.,
                    body.top + 90.,
                ),
                "暂无运行记录",
                TextStyle::Label,
                p.muted,
            );
        }
        for (i, run) in s.history.clone().iter().enumerate() {
            let row = Rect::from_size(
                body.left + 16.,
                body.top + 12. + i as f32 * 58. - s.form_scroll,
                body.width() - 32.,
                48.,
            );
            if row.top >= body.top && row.bottom <= body.bottom {
                button(
                    list,
                    s,
                    row,
                    &format!(
                        "{} · {}",
                        chrono::DateTime::from_timestamp_millis(run.started_at)
                            .map(|d| d
                                .with_timezone(&chrono::Local)
                                .format("%m-%d %H:%M")
                                .to_string())
                            .unwrap_or_default(),
                        super::painting::status(&run.status)
                    ),
                    Hit::RunHistory(i),
                    p,
                    false,
                );
            }
        }
        list.pop_clip();
        return;
    }
    list.push_clip(body);
    let mut y = body.top + 8. - s.form_scroll;
    let mut previous = "";
    for (path, label, value) in form_fields(s, &n) {
        let group = section(&path);
        if group != previous {
            let header = Rect::from_size(r.left + 16., y, r.width() - 32., 30.);
            if header.top >= body.top && header.bottom <= body.bottom {
                let label = format!(
                    "{}  {}",
                    if s.folded.contains(group) {
                        "▸"
                    } else {
                        "▾"
                    },
                    group
                );
                list.text(header, &label, TextStyle::Label, p.foreground);
                s.hits.push((header, Hit::Section(group.into())));
            }
            y += 40.;
            previous = group;
        }
        if s.folded.contains(group) {
            continue;
        }
        let label = label.split(" · ").last().unwrap_or(&label).to_owned();
        let mut content = if s.editor == Some(Editor::Parameter) && s.parameter == path {
            s.field.text().to_owned()
        } else {
            value_text(&value)
        };
        if path == "/config/provider_id"
            && !(s.editor == Some(Editor::Parameter) && s.parameter == path)
        {
            content = if content.is_empty() {
                "当前默认提供商  ▾".into()
            } else {
                s.provider_labels
                    .iter()
                    .find(|(id, _)| id == &content)
                    .map(|(_, label)| format!("{label}  ▾"))
                    .unwrap_or(content)
            };
        }
        let height = field_height(&path, &value);
        let multiline = height > 40.;
        let variable = path.starts_with("/inputs/");
        let editing = s.editor == Some(Editor::Parameter) && s.parameter == path && focused;
        let field = Rect::from_size(r.left + 16., y + 22., r.width() - 32., height);
        let reference_label = s.reference_label(&content);
        if field.bottom >= body.top && y < body.bottom {
            list.text(
                Rect::new(field.left, y, field.right, y + 22.),
                label,
                TextStyle::Caption,
                p.muted,
            );
            let choices = parameter_choices(&path);
            if variable && !editing {
                list.rounded_rect(field, 8., p.background);
                list.rounded_border(field, 8., p.border);
                let display = if content.is_empty() {
                    "选择变量或填写自定义值"
                } else {
                    &reference_label
                };
                list.text(
                    Rect::new(
                        field.left + 12.,
                        field.top + 8.,
                        field.right - 30.,
                        field.bottom - 6.,
                    ),
                    text::ellipsize(
                        &display.replace('\n', " "),
                        TextStyle::Caption,
                        field.width() - 44.,
                    ),
                    TextStyle::Caption,
                    p.foreground,
                );
                list.text(
                    Rect::from_size(field.right - 26., field.top, 20., 40.),
                    "⌄",
                    TextStyle::Label,
                    p.muted,
                );
                s.hits
                    .push((field.intersect(&body), Hit::VariablePicker(path.clone())));
            } else if value.is_boolean() && !variable {
                let on = content == "true";
                let track = Rect::from_size(field.right - 42., field.top + 8., 36., 20.);
                list.rounded_rect(track, 10., if on { p.accent } else { p.border });
                list.rounded_rect(
                    Rect::from_size(
                        track.left + if on { 18. } else { 2. },
                        track.top + 2.,
                        16.,
                        16.,
                    ),
                    8.,
                    if on {
                        p.accent_foreground
                    } else {
                        p.surface_elevated
                    },
                );
                list.text(
                    Rect::new(field.left, field.top, track.left - 8., field.bottom),
                    if on { "已启用" } else { "已关闭" },
                    TextStyle::Caption,
                    p.muted,
                );
                s.hits
                    .push((field.intersect(&body), Hit::Parameter(path.clone())));
            } else if !choices.is_empty() {
                let label = choices
                    .iter()
                    .find(|(key, _)| *key == content)
                    .map(|(_, label)| *label)
                    .unwrap_or(&content);
                list.rounded_rect(field, 6., p.background);
                list.rounded_border(field, 6., p.border);
                list.text(
                    Rect::new(field.left + 10., field.top, field.right - 30., field.bottom),
                    label,
                    TextStyle::Label,
                    p.foreground,
                );
                list.text(
                    Rect::from_size(field.right - 24., field.top, 20., field.height()),
                    "⌄",
                    TextStyle::Label,
                    p.muted,
                );
                s.hits
                    .push((field.intersect(&body), Hit::Parameter(path.clone())));
            } else if s.editor == Some(Editor::Parameter)
                && s.parameter == path
                && field.top >= body.top
                && field.bottom <= body.bottom
            {
                let edit = if variable {
                    Rect::new(field.left, field.top, field.right - 34., field.bottom)
                } else {
                    field
                };
                s.field_rect = edit;
                s.field.paint_multiline(list, edit, focused, p);
                if variable {
                    button(
                        list,
                        s,
                        Rect::from_size(field.right - 32., field.top, 32., 40.),
                        "⌄",
                        Hit::VariablePicker(path.clone()),
                        p,
                        false,
                    );
                }
            } else {
                list.rounded_rect(field, 6., p.background);
                list.rounded_border(field, 6., p.border);
                let value = if content.is_empty() {
                    "点击填写"
                } else {
                    &content
                };
                if multiline {
                    let mut preview = TextField::new("点击填写");
                    preview.set_text(&value.chars().take(2000).collect::<String>());
                    preview.paint_multiline(list, field, false, p);
                } else {
                    list.text(
                        Rect::new(
                            field.left + 10.,
                            field.top + 8.,
                            field.right - 10.,
                            field.bottom - 6.,
                        ),
                        text::ellipsize(
                            &value.replace('\n', " "),
                            TextStyle::Label,
                            field.width() - 20.,
                        ),
                        TextStyle::Label,
                        p.foreground,
                    );
                }
                let hit = field.intersect(&body);
                if !hit.is_empty() {
                    s.hits.push((hit, Hit::Parameter(path.clone())));
                }
            }
        }
        y += height + 38.;
    }
    s.form_height = (y + s.form_scroll - body.top).max(0.);
    list.pop_clip();
    if s.form_height > body.height() && body.height() > 0. {
        let height = (body.height() * body.height() / s.form_height).max(24.);
        let top =
            body.top + (body.height() - height) * s.form_scroll / (s.form_height - body.height());
        list.rounded_rect(
            Rect::from_size(r.right - 6., top, 3., height),
            1.5,
            p.border,
        );
    }
    list.hline(r.left, r.right, r.bottom - 86., p.border);
    let w = (r.width() - 40.) / 3.;
    for (i, (label, hit)) in [
        ("应用", Hit::Apply),
        ("引用对象", Hit::PickObject),
        ("删除", Hit::Delete),
    ]
    .into_iter()
    .enumerate()
    {
        button(
            list,
            s,
            Rect::from_size(r.left + 12. + i as f32 * (w + 8.), r.bottom - 78., w, 32.),
            label,
            hit,
            p,
            i == 0,
        );
    }
    for (i, (label, hit)) in [
        ("复制节点", Hit::Duplicate),
        ("插入变量", Hit::Variables),
        ("高级 JSON", Hit::Advanced),
    ]
    .into_iter()
    .enumerate()
    {
        button(
            list,
            s,
            Rect::from_size(r.left + 12. + i as f32 * (w + 8.), r.bottom - 40., w, 30.),
            label,
            hit,
            p,
            false,
        );
    }
}

pub fn palette(list: &mut DrawList, s: &mut State, p: &Palette) {
    let r = if !s.palette_rect.is_empty() && s.palette_rect.left < s.canvas.left {
        s.palette_rect
    } else {
        Rect::from_size(
            s.canvas.left + 12.,
            s.canvas.top + 10.,
            (s.canvas.width() - 24.).min(280.).max(100.),
            (s.canvas.height() - 20.).max(60.),
        )
    };
    s.palette_rect = r;
    list.rounded_rect(r, 10., p.surface_elevated);
    list.rounded_border(r, 10., p.border);
    s.hits.push((r, Hit::Parameters));
    list.text(
        Rect::new(r.left + 16., r.top + 12., r.right - 16., r.top + 36.),
        "节点库",
        TextStyle::Label,
        p.foreground,
    );
    super::chrome::icon_button(
        list,
        s,
        Rect::from_size(r.right - 44., r.top + 8., 32., 30.),
        crate::ui::icons::Icon::PANEL_LEFT_CLOSE,
        "收起节点库",
        Hit::Add,
        p,
    );
    s.search_rect = Rect::new(r.left + 10., r.top + 44., r.right - 10., r.top + 80.);
    s.search.paint(
        list,
        s.search_rect,
        s.searching,
        p,
        crate::ui::widgets::FieldLook::dialog(p),
    );
    s.hits.push((s.search_rect, Hit::PaletteSearch));
    let body = Rect::new(r.left + 8., r.top + 92., r.right - 8., r.bottom - 42.);
    list.text(
        Rect::new(r.left + 14., r.bottom - 36., r.right - 14., r.bottom - 8.),
        "点击添加，或拖入画布",
        TextStyle::Tiny,
        p.muted,
    );
    list.push_clip(body);
    let query = s.search.text().to_lowercase();
    let mut y = body.top - s.scroll;
    for category in ["触发器", "AI", "逻辑", "脚本", "数据", "输出"] {
        let items: Vec<_> = mochi_core::workflows::catalog::KINDS
            .iter()
            .enumerate()
            .filter(|(_, kind)| {
                super::modern::category(kind).0 == category
                    && (mochi_core::workflows::catalog::label(kind)
                        .to_lowercase()
                        .contains(&query)
                        || kind.contains(&query)
                        || category.to_lowercase().contains(&query))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        list.text(
            Rect::from_size(body.left + 8., y, body.width() - 16., 26.),
            category,
            TextStyle::Tiny,
            p.muted,
        );
        y += 28.;
        for (i, kind) in items {
            let row = Rect::from_size(body.left, y, body.width(), 54.);
            if row.top >= body.top && row.bottom <= body.bottom {
                if s.hover == Some(Hit::Kind(i)) {
                    list.rounded_rect(row, 7., p.surface_muted);
                }
                list.text(
                    Rect::new(row.left + 38., row.top + 4., row.right - 4., row.top + 26.),
                    mochi_core::workflows::catalog::label(kind),
                    TextStyle::Caption,
                    p.foreground,
                );
                let n = Node::new("preview", kind, 0., 0.);
                let description = super::modern::preview(&n).0;
                list.text(
                    Rect::new(
                        row.left + 38.,
                        row.top + 27.,
                        row.right - 4.,
                        row.bottom - 4.,
                    ),
                    text::ellipsize(&description, TextStyle::Tiny, row.width() - 42.),
                    TextStyle::Tiny,
                    p.muted,
                );
                s.hits.push((row, Hit::Kind(i)));
                let (_, color, icon) = super::modern::category(kind);
                list.icon_centered(
                    Rect::from_size(row.left + 6., row.top + 10., 20., 20.),
                    icon,
                    16.,
                    color,
                );
            }
            y += 58.;
        }
    }
    list.pop_clip();
}
