//! 可滚动的原生规则编辑器；输入文本时复用应用中支持 IME 的对话框。
use super::*;
use crate::ui::{draw::TextStyle, workspace_ui};
use engine::{Action, Input, Rule, Trigger};
use model::{BaseFilter, FilterOperator};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) enum TextEdit {
    Name,
    Minutes,
    StartAt,
    Condition(usize),
    Value(usize, String),
    Delete,
    Discard(bool),
}
#[derive(Debug, Clone, PartialEq)]
enum Change {
    Trigger(Trigger),
    Watch(String),
    ConditionField(usize, String),
    Operator(usize, FilterOperator),
    ConditionValue(usize, serde_json::Value),
    ActionType(usize, Option<String>),
    AddValue(usize, String),
    Input(usize, String, Input),
    Record(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Close,
    Back,
    New,
    Rule(usize),
    Name,
    Trigger,
    Watch,
    Minutes,
    StartAt,
    AddCondition,
    ConditionField(usize),
    Operator(usize),
    ConditionValue(usize),
    RemoveCondition(usize),
    AddAction,
    ActionType(usize),
    AddValue(usize),
    Value(usize, usize),
    RemoveValue(usize, usize),
    RemoveAction(usize),
    PickRecord,
    Test,
    Run,
    Save,
    Enable,
    Pause,
    Delete,
    Logs,
    Choice(usize),
    CancelChoice,
    Guide,
}
pub(in crate::app) struct Panel {
    pub path: PathBuf,
    original: String,
    document: model::BaseDocument,
    table_id: String,
    view_id: String,
    draft: Option<Rule>,
    initial_draft: Option<Rule>,
    choices: Vec<(String, Change)>,
    record: Option<String>,
    preview: Option<(model::BaseDocument, engine::Run)>,
    message: Vec<String>,
    logs: bool,
    scroll: f32,
    focused: Option<usize>,
    hover: Option<Hit>,
    granted: BTreeSet<String>,
}
#[derive(Clone)]
struct Row {
    label: String,
    hit: Option<Hit>,
    primary: bool,
}
fn row(label: impl Into<String>, hit: Hit) -> Row {
    Row {
        label: label.into(),
        hit: Some(hit),
        primary: false,
    }
}
fn note(label: impl Into<String>) -> Row {
    Row {
        label: label.into(),
        hit: None,
        primary: false,
    }
}
struct Layout {
    area: Rect,
    body: Rect,
    close: Rect,
    rows: Vec<(Rect, Row)>,
    max_scroll: f32,
}

impl Panel {
    fn table(&self) -> &model::BaseTable {
        engine::scope(&self.document, &self.table_id, &self.view_id)
            .unwrap()
            .0
    }
    fn view(&self) -> &model::BaseView {
        engine::scope(&self.document, &self.table_id, &self.view_id)
            .unwrap()
            .1
    }
    fn target(&self, index: usize) -> &model::BaseTable {
        match &self.draft.as_ref().unwrap().actions[index] {
            Action::CreateRecord { table_id, .. } => self
                .document
                .tables
                .iter()
                .find(|t| &t.id == table_id)
                .unwrap_or(self.table()),
            _ => self.table(),
        }
    }
    fn changed(&self) -> bool {
        self.draft != self.initial_draft
    }
    fn edit_changed(&mut self) {
        self.preview = None;
        self.message.clear();
        self.logs = false;
    }
    fn rows(&self) -> Vec<Row> {
        if !self.choices.is_empty() {
            let mut rows = vec![row("‹ 返回配置", Hit::CancelChoice)];
            rows.extend(
                self.choices
                    .iter()
                    .enumerate()
                    .map(|(i, (label, _))| row(label.clone(), Hit::Choice(i))),
            );
            return rows;
        }
        let mut rows = vec![];
        if let Some(rule) = &self.draft {
            rows.push(row("‹ 所有自动化", Hit::Back));
            rows.push(row(format!("名称    {}", rule.name), Hit::Name));
            rows.push(row(
                format!("01  触发    {}", trigger_label(&rule.trigger)),
                Hit::Trigger,
            ));
            if let Trigger::RecordUpdated { field_ids } = &rule.trigger {
                let labels = field_ids
                    .iter()
                    .map(|id| {
                        self.table()
                            .fields
                            .iter()
                            .find(|f| &f.id == id)
                            .map(|f| f.name.as_str())
                            .unwrap_or(id)
                    })
                    .collect::<Vec<_>>()
                    .join("、");
                rows.push(row(format!("监控字段    {labels}"), Hit::Watch));
            }
            if let Trigger::Interval { minutes, start_at } = &rule.trigger {
                rows.push(row(format!("每 {minutes} 分钟执行一次"), Hit::Minutes));
                rows.push(row(
                    format!("开始时间    {}", time_label(*start_at)),
                    Hit::StartAt,
                ));
            }
            rows.push(note("02  条件    与当前视图筛选共同生效（全部满足）"));
            for (i, condition) in rule.conditions.iter().enumerate() {
                let field = self
                    .table()
                    .fields
                    .iter()
                    .find(|f| f.id == condition.field_id);
                rows.push(row(
                    format!(
                        "条件 {} · {}",
                        i + 1,
                        field.map(|f| f.name.as_str()).unwrap_or("字段已删除")
                    ),
                    Hit::ConditionField(i),
                ));
                rows.push(row(
                    format!("判断    {}", operator_label(condition.operator)),
                    Hit::Operator(i),
                ));
                if !matches!(
                    condition.operator,
                    FilterOperator::IsEmpty | FilterOperator::IsNotEmpty
                ) {
                    let value = condition.value.as_ref().unwrap_or(&serde_json::Value::Null);
                    rows.push(row(
                        format!(
                            "比较值    {}",
                            field
                                .map(|f| model::format_cell_value(f, value))
                                .unwrap_or_default()
                        ),
                        Hit::ConditionValue(i),
                    ));
                }
                rows.push(row("移除此条件", Hit::RemoveCondition(i)));
            }
            rows.push(row("＋ 添加条件", Hit::AddCondition));
            rows.push(note("03  动作    按顺序执行，字段引用始终读取触发前的记录"));
            for (i, action) in rule.actions.iter().enumerate() {
                let (label, values) = match action {
                    Action::UpdateRecord { values } => ("修改触发记录".into(), values),
                    Action::CreateRecord { values, .. } => {
                        (format!("在「{}」创建记录", self.target(i).name), values)
                    }
                };
                rows.push(row(format!("动作 {} · {label}", i + 1), Hit::ActionType(i)));
                for (j, (id, input)) in values.iter().enumerate() {
                    let field = self.target(i).fields.iter().find(|f| &f.id == id);
                    let label = match input {
                        Input::Literal { value } => field
                            .map(|f| model::format_cell_value(f, value))
                            .unwrap_or_else(|| value.to_string()),
                        Input::Field { field_id } => format!(
                            "引用 · {}",
                            self.table()
                                .fields
                                .iter()
                                .find(|f| &f.id == field_id)
                                .map(|f| f.name.as_str())
                                .unwrap_or(field_id)
                        ),
                        Input::Now => "执行时的当前时间".into(),
                        Input::RecordId => "触发记录 ID".into(),
                    };
                    rows.push(row(
                        format!(
                            "{}  ←  {}",
                            field.map(|f| f.name.as_str()).unwrap_or(id),
                            if label.is_empty() {
                                "（空值）"
                            } else {
                                &label
                            }
                        ),
                        Hit::Value(i, j),
                    ));
                    rows.push(row("移除此字段赋值", Hit::RemoveValue(i, j)));
                }
                rows.push(row("＋ 设置写入字段", Hit::AddValue(i)));
                rows.push(row("移除此动作", Hit::RemoveAction(i)));
            }
            rows.push(row("＋ 添加动作", Hit::AddAction));
            rows.push(note("04  验证    先选择一条符合条件的记录；试跑不会写入"));
            let selected = self
                .record
                .as_ref()
                .and_then(|id| self.table().records.iter().find(|r| &r.id == id));
            let label = selected
                .map(|r| record_label(self.table(), r))
                .unwrap_or_else(|| "选择测试记录…".into());
            rows.push(row(label, Hit::PickRecord));
            rows.push(row("试跑 · 预览修改", Hit::Test));
            if self.preview.is_some() {
                rows.push(row("确认执行上述预览 · 仅一次", Hit::Run));
            }
            rows.extend(self.message.iter().cloned().map(note));
            rows.push(row("保存草稿（不自动执行）", Hit::Save));
            let mut enable = row("保存并启用此自动化", Hit::Enable);
            enable.primary = true;
            rows.push(enable);
            rows.push(note(
                "启用即允许以上动作持续写入；仅应用运行且工作区打开时执行。",
            ));
            if rule.enabled {
                rows.push(row("暂停自动化", Hit::Pause));
            }
            if engine::rules(self.view())
                .unwrap_or_default()
                .iter()
                .any(|r| r.id == rule.id)
            {
                rows.push(row("删除自动化…", Hit::Delete));
            }
        } else {
            rows.push(row("＋ 新建自动化", Hit::New));
            match engine::rules(self.view()) {
                Ok(rules) => {
                    if rules.is_empty() {
                        rows.push(note("还没有自动化。让重复的整理工作自动完成。"));
                    }
                    for (i, rule) in rules.iter().enumerate() {
                        let status = if !rule.enabled {
                            "草稿 / 已暂停"
                        } else if self.granted.contains(&engine::signature(
                            &self.document,
                            self.table(),
                            self.view(),
                            rule,
                        )) {
                            "已启用"
                        } else {
                            "待确认启用"
                        };
                        rows.push(row(
                            format!(
                                "{}    ·    {status}    ·    {}",
                                rule.name,
                                trigger_label(&rule.trigger)
                            ),
                            Hit::Rule(i),
                        ));
                    }
                }
                Err(e) => rows.push(note(format!("配置错误：{e}"))),
            }
            rows.extend(self.message.iter().cloned().map(note));
        }
        rows.push(row(
            if self.logs {
                "收起运行记录"
            } else {
                "查看最近运行记录"
            },
            Hit::Logs,
        ));
        if self.logs {
            let state = engine::runtime(&self.document);
            let runs: Vec<_> = state
                .runs
                .iter()
                .rev()
                .filter(|r| {
                    r.table_id == self.table_id
                        && r.view_id == self.view_id
                        && self.draft.as_ref().is_none_or(|d| d.id == r.rule_id)
                })
                .take(30)
                .collect();
            if runs.is_empty() {
                rows.push(note("暂无执行记录。试跑不会计入正式运行日志。"));
            }
            for run in runs {
                rows.push(note(format!(
                    "{} · {} · 更新 {} / 新建 {}",
                    time_label(run.at),
                    if run.error.is_some() {
                        "失败 / 已暂停"
                    } else {
                        "成功"
                    },
                    run.updates,
                    run.creates
                )));
                if let Some(error) = &run.error {
                    rows.push(note(error.clone()));
                }
            }
        }
        rows.push(row("Agent 编写指南", Hit::Guide));
        rows
    }
    fn layout(&self, viewport: Rect) -> Layout {
        let width = (viewport.width() - 24.0).clamp(0.0, 780.0);
        let height = (viewport.height() - 24.0).clamp(0.0, 840.0);
        let area = Rect::from_size(
            viewport.left + (viewport.width() - width) / 2.0,
            viewport.top + (viewport.height() - height) / 2.0,
            width,
            height,
        );
        let body = Rect::new(
            area.left + 18.0,
            area.top + 94.0,
            area.right - 18.0,
            area.bottom - 18.0,
        );
        let rows = self.rows();
        let max_scroll = (rows.len() as f32 * 42.0 - body.height()).max(0.0);
        let start = body.top - self.scroll.clamp(0.0, max_scroll);
        Layout {
            area,
            body,
            close: Rect::from_size(area.right - 50.0, area.top + 16.0, 32.0, 32.0),
            rows: rows
                .into_iter()
                .enumerate()
                .map(|(i, row)| {
                    (
                        Rect::from_size(body.left, start + i as f32 * 42.0, body.width(), 36.0),
                        row,
                    )
                })
                .collect(),
            max_scroll,
        }
    }
    fn hit(&self, viewport: Rect, x: f32, y: f32) -> Option<Hit> {
        let layout = self.layout(viewport);
        if layout.close.contains(x, y) {
            return Some(Hit::Close);
        }
        if !layout.body.contains(x, y) {
            return None;
        }
        layout
            .rows
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .and_then(|(_, r)| r.hit)
    }
}

fn trigger_label(trigger: &Trigger) -> &'static str {
    match trigger {
        Trigger::Manual => "手动运行",
        Trigger::RecordCreated => "新增记录",
        Trigger::RecordUpdated { .. } => "指定字段修改",
        Trigger::EnterView => "记录进入视图 / 首次满足条件",
        Trigger::Interval { .. } => "定时间隔",
    }
}
fn operator_label(operator: FilterOperator) -> &'static str {
    match operator {
        FilterOperator::Contains => "包含",
        FilterOperator::Equals => "等于",
        FilterOperator::NotEquals => "不等于",
        FilterOperator::IsEmpty => "为空",
        FilterOperator::IsNotEmpty => "不为空",
    }
}
fn time_label(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .unwrap_or_else(|| ms.to_string())
}
fn record_label(table: &model::BaseTable, r: &model::BaseRecord) -> String {
    let title = table
        .fields
        .first()
        .map(|f| {
            model::format_cell_value(f, r.values.get(&f.id).unwrap_or(&serde_json::Value::Null))
        })
        .unwrap_or_default();
    format!(
        "{} · {}",
        if title.is_empty() {
            "无标题"
        } else {
            &title
        },
        r.id
    )
}
fn values_mut(rule: &mut Rule, index: usize) -> &mut BTreeMap<String, Input> {
    match &mut rule.actions[index] {
        Action::UpdateRecord { values } | Action::CreateRecord { values, .. } => values,
    }
}

impl App {
    pub(in crate::app) fn open_automations(&mut self) {
        self.save_active();
        let Some((path, viewer::Content::Base(state))) = self.viewer_tab() else {
            return;
        };
        if state.dirty {
            self.show_global_notice("请先保存表格，再配置自动化");
            return;
        }
        let path = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => return,
        };
        let table_id = state.table().id.clone();
        let view_id = state.view().id.clone();
        let record = state
            .selected
            .and_then(|(r, _)| state.table().records.get(r))
            .map(|r| r.id.clone());
        let original = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                self.show_global_notice(&e.to_string());
                return;
            }
        };
        let document = match model::parse_base_document(&original) {
            Ok(doc) => doc,
            Err(e) => {
                self.show_global_notice(&e.to_string());
                return;
            }
        };
        if original.len() as u64 > engine::MAX_FILE_BYTES {
            self.show_global_notice("自动化当前支持最大 10 MiB 的表格文件");
            return;
        }
        if engine::scope(&document, &table_id, &view_id).is_err() {
            self.show_global_notice("视图已被外部修改，请刷新表格后重试");
            return;
        }
        self.automation.panel = Some(Panel {
            granted: authorized(&grants(&self.settings), &path),
            path,
            original,
            document,
            table_id,
            view_id,
            draft: None,
            initial_draft: None,
            choices: vec![],
            record,
            preview: None,
            message: vec![],
            logs: false,
            scroll: 0.0,
            focused: None,
            hover: None,
        });
        self.menu = None;
        self.focus = Focus::Main;
        self.invalidate_main();
    }
    pub(in crate::app) fn paint_automations(&mut self, p: &theme::Palette) {
        let Some(panel) = &self.automation.panel else {
            return;
        };
        let viewport = self.renderer.viewport();
        let layout = panel.layout(viewport);
        self.list.rect_alpha(viewport, 0x000000, 0.28);
        self.list.rounded_rect(layout.area, 10.0, p.surface);
        self.list.rounded_border(layout.area, 10.0, p.border);
        self.list.text(
            Rect::new(
                layout.area.left + 22.0,
                layout.area.top + 16.0,
                layout.close.left - 12.0,
                layout.area.top + 44.0,
            ),
            "视图自动化",
            TextStyle::Large,
            p.foreground,
        );
        self.list.text(
            Rect::new(
                layout.area.left + 22.0,
                layout.area.top + 52.0,
                layout.area.right - 20.0,
                layout.area.top + 78.0,
            ),
            crate::ui::text::ellipsize(
                &format!(
                    "{} / {} · 本地执行 · 不回填历史记录",
                    panel.table().name,
                    panel.view().name
                ),
                TextStyle::Caption,
                layout.area.width() - 44.0,
            ),
            TextStyle::Caption,
            p.muted,
        );
        self.list
            .icon_centered(layout.close, Icon::X, 18.0, p.muted);
        self.list.hline(
            layout.area.left,
            layout.area.right,
            layout.body.top - 8.0,
            p.border,
        );
        self.list.push_clip(layout.body);
        for (i, (r, row)) in layout.rows.iter().enumerate() {
            if r.intersect(&layout.body).is_empty() {
                continue;
            }
            let label = crate::ui::text::ellipsize(
                &row.label,
                TextStyle::Label,
                (r.width() - 24.0).max(0.0),
            );
            if let Some(hit) = row.hit {
                workspace_ui::button(
                    &mut self.list,
                    *r,
                    &label,
                    None,
                    row.primary,
                    panel.hover == Some(hit),
                    p,
                );
                if panel.focused == Some(i) {
                    self.list.rounded_border(*r, 7.0, p.accent);
                }
            } else {
                self.list.text(*r, label, TextStyle::Caption, p.muted);
            }
        }
        self.list.pop_clip();
        if layout.max_scroll > 0.0 {
            let h = (layout.body.height() * layout.body.height()
                / (layout.body.height() + layout.max_scroll))
                .max(20.0);
            let y = layout.body.top
                + (layout.body.height() - h) * panel.scroll.clamp(0.0, layout.max_scroll)
                    / layout.max_scroll;
            self.list.rounded_rect(
                Rect::from_size(layout.area.right - 9.0, y, 3.0, h),
                1.5,
                p.border,
            );
        }
    }
    pub(in crate::app) fn automation_click(&mut self, x: f32, y: f32) {
        let hit = self
            .automation
            .panel
            .as_ref()
            .and_then(|p| p.hit(self.renderer.viewport(), x, y));
        if let Some(hit) = hit {
            self.automation_activate(hit);
        }
    }
    pub(in crate::app) fn automation_hover(&mut self, x: f32, y: f32) -> bool {
        let viewport = self.renderer.viewport();
        let Some(panel) = self.automation.panel.as_mut() else {
            return false;
        };
        let hit = panel.hit(viewport, x, y);
        let changed = panel.hover != hit;
        panel.hover = hit;
        changed
    }
    pub(in crate::app) fn automation_wheel(&mut self, delta: i16) {
        let viewport = self.renderer.viewport();
        if let Some(panel) = &mut self.automation.panel {
            let max = panel.layout(viewport).max_scroll;
            panel.scroll = (panel.scroll - f32::from(delta) / 120.0 * 126.0).clamp(0.0, max);
        }
    }
    pub(in crate::app) fn automation_key(&mut self, key: u16, shift: bool) -> bool {
        if self.automation.panel.is_none() {
            return false;
        }
        if key == 0x1b {
            self.automation_activate(
                if self
                    .automation
                    .panel
                    .as_ref()
                    .is_some_and(|p| !p.choices.is_empty())
                {
                    Hit::CancelChoice
                } else {
                    Hit::Close
                },
            );
            return true;
        }
        if key == 0x09 || key == 0x26 || key == 0x28 {
            let viewport = self.renderer.viewport();
            let p = self.automation.panel.as_mut().unwrap();
            let lay = p.layout(viewport);
            let indices: Vec<_> = lay
                .rows
                .iter()
                .enumerate()
                .filter(|(_, (_, r))| r.hit.is_some())
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                let reverse = key == 0x26 || (key == 0x09 && shift);
                let index = p.focused.and_then(|i| indices.iter().position(|x| *x == i));
                let next = match index {
                    Some(i) if reverse => (i + indices.len() - 1) % indices.len(),
                    Some(i) => (i + 1) % indices.len(),
                    None => 0,
                };
                let selected = indices[next];
                p.focused = Some(selected);
                let r = lay.rows[selected].0;
                if r.top < lay.body.top {
                    p.scroll = (p.scroll - (lay.body.top - r.top)).max(0.0);
                } else if r.bottom > lay.body.bottom {
                    p.scroll = (p.scroll + r.bottom - lay.body.bottom).min(lay.max_scroll);
                }
            }
        } else if key == 0x0d || key == 0x20 {
            let hit = self
                .automation
                .panel
                .as_ref()
                .and_then(|p| p.focused.and_then(|i| p.rows().get(i).and_then(|r| r.hit)));
            if let Some(hit) = hit {
                self.automation_activate(hit);
            }
        }
        true
    }
    fn automation_text(
        &mut self,
        edit: TextEdit,
        title: &str,
        value: Option<String>,
        description: &str,
    ) {
        self.dialog = Some(Dialog {
            title: title.into(),
            description: description.into(),
            field: value.map(|s| TextField::new("").with_text(&s)),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "确定".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::AutomationText(edit),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }
    fn automation_activate(&mut self, hit: Hit) {
        let Some(mut panel) = self.automation.panel.take() else {
            return;
        };
        panel.hover = None;
        let mut text: Option<(TextEdit, String, Option<String>, String)> = None;
        let mut save = None;
        let mut run = false;
        match hit {
            Hit::Close | Hit::Back if panel.changed() => {
                text = Some((
                    TextEdit::Discard(hit == Hit::Close),
                    "放弃未保存的配置？".into(),
                    None,
                    "仅放弃本次规则编辑，不修改表格记录。".into(),
                ));
            }
            Hit::Close => {
                self.focus = Focus::Main;
                return;
            }
            Hit::Back => {
                panel.draft = None;
                panel.initial_draft = None;
                panel.preview = None;
                panel.scroll = 0.0;
                panel.message.clear();
            }
            Hit::New => {
                panel.draft = Some(Rule {
                    id: model::create_id("automation"),
                    name: "新的自动化".into(),
                    enabled: false,
                    trigger: Trigger::RecordCreated,
                    conditions: vec![],
                    actions: vec![Action::UpdateRecord {
                        values: BTreeMap::new(),
                    }],
                });
                panel.initial_draft = panel.draft.clone();
                panel.scroll = 0.0;
                panel.message.clear();
            }
            Hit::Rule(i) => {
                panel.draft = engine::rules(panel.view())
                    .ok()
                    .and_then(|r| r.get(i).cloned());
                panel.initial_draft = panel.draft.clone();
                panel.scroll = 0.0;
                panel.preview = None;
                panel.message.clear();
            }
            Hit::Name => {
                text = Some((
                    TextEdit::Name,
                    "自动化名称".into(),
                    panel.draft.as_ref().map(|r| r.name.clone()),
                    "用名称说明它在什么情况下做什么。".into(),
                ));
            }
            Hit::Trigger => {
                let fields = panel
                    .table()
                    .fields
                    .first()
                    .map(|f| vec![f.id.clone()])
                    .unwrap_or_default();
                panel.choices = vec![
                    Trigger::Manual,
                    Trigger::RecordCreated,
                    Trigger::RecordUpdated { field_ids: fields },
                    Trigger::EnterView,
                    Trigger::Interval {
                        minutes: 60,
                        start_at: engine::now_ms() + 60_000,
                    },
                ]
                .into_iter()
                .map(|t| (trigger_label(&t).into(), Change::Trigger(t)))
                .collect();
                panel.scroll = 0.0;
            }
            Hit::Watch => {
                let watched = match &panel.draft.as_ref().unwrap().trigger {
                    Trigger::RecordUpdated { field_ids } => field_ids.clone(),
                    _ => vec![],
                };
                panel.choices = panel
                    .table()
                    .fields
                    .iter()
                    .map(|f| {
                        (
                            format!(
                                "{} {}",
                                if watched.contains(&f.id) {
                                    "✓"
                                } else {
                                    "＋"
                                },
                                f.name
                            ),
                            Change::Watch(f.id.clone()),
                        )
                    })
                    .collect();
                panel.scroll = 0.0;
            }
            Hit::Minutes => {
                if let Trigger::Interval { minutes, .. } = panel.draft.as_ref().unwrap().trigger {
                    text = Some((
                        TextEdit::Minutes,
                        "执行间隔".into(),
                        Some(minutes.to_string()),
                        "分钟数，1–525600。休眠恢复只补一次，不补跑所有错过的周期。".into(),
                    ));
                }
            }
            Hit::StartAt => {
                if let Trigger::Interval { start_at, .. } = panel.draft.as_ref().unwrap().trigger {
                    text = Some((
                        TextEdit::StartAt,
                        "首次执行时间".into(),
                        Some(
                            chrono::DateTime::from_timestamp_millis(start_at as i64)
                                .map(|time| time.to_rfc3339())
                                .unwrap_or_else(|| start_at.to_string()),
                        ),
                        "ISO 8601 时间，包含时区，例如 2026-09-20T09:00:00+08:00。".into(),
                    ));
                }
            }
            Hit::AddCondition => {
                let field = panel.table().fields.first().map(|f| f.id.clone());
                if let Some(id) = field {
                    let r = panel.draft.as_mut().unwrap();
                    if r.conditions.len() < 20 {
                        r.conditions.push(BaseFilter {
                            field_id: id,
                            operator: FilterOperator::IsNotEmpty,
                            ..Default::default()
                        });
                        panel.edit_changed();
                    }
                }
            }
            Hit::ConditionField(i) => {
                panel.choices = panel
                    .table()
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), Change::ConditionField(i, f.id.clone())))
                    .collect();
                panel.scroll = 0.0;
            }
            Hit::Operator(i) => {
                panel.choices = [
                    FilterOperator::Contains,
                    FilterOperator::Equals,
                    FilterOperator::NotEquals,
                    FilterOperator::IsEmpty,
                    FilterOperator::IsNotEmpty,
                ]
                .into_iter()
                .map(|o| (operator_label(o).into(), Change::Operator(i, o)))
                .collect();
                panel.scroll = 0.0;
            }
            Hit::ConditionValue(i) => {
                let c = &panel.draft.as_ref().unwrap().conditions[i];
                let field = panel.table().fields.iter().find(|f| f.id == c.field_id);
                if field.is_some_and(|f| !f.options.is_empty()) {
                    panel.choices = field
                        .unwrap()
                        .options
                        .iter()
                        .map(|o| {
                            (
                                o.label.clone(),
                                Change::ConditionValue(i, serde_json::json!(o.id)),
                            )
                        })
                        .collect();
                    panel.scroll = 0.0;
                } else {
                    text = Some((
                        TextEdit::Condition(i),
                        "条件比较值".into(),
                        Some(display_value(
                            c.value.as_ref().unwrap_or(&serde_json::Value::Null),
                        )),
                        "文本直接输入；数字、布尔、数组使用 JSON。".into(),
                    ));
                }
            }
            Hit::RemoveCondition(i) => {
                panel.draft.as_mut().unwrap().conditions.remove(i);
                panel.edit_changed();
            }
            Hit::AddAction => {
                let r = panel.draft.as_mut().unwrap();
                if r.actions.len() < 16 {
                    r.actions.push(Action::UpdateRecord {
                        values: BTreeMap::new(),
                    });
                    panel.edit_changed();
                }
            }
            Hit::RemoveAction(i) => {
                panel.draft.as_mut().unwrap().actions.remove(i);
                panel.edit_changed();
            }
            Hit::ActionType(i) => {
                panel.choices = vec![(
                    "修改触发记录（切换将清空此动作的赋值）".into(),
                    Change::ActionType(i, None),
                )];
                panel.choices.extend(panel.document.tables.iter().map(|t| {
                    (
                        format!("在「{}」创建记录（切换将清空赋值）", t.name),
                        Change::ActionType(i, Some(t.id.clone())),
                    )
                }));
                panel.scroll = 0.0;
            }
            Hit::AddValue(i) => {
                panel.choices = panel
                    .target(i)
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), Change::AddValue(i, f.id.clone())))
                    .collect();
                panel.scroll = 0.0;
            }
            Hit::Value(i, j) => {
                let id = values_mut(panel.draft.as_mut().unwrap(), i)
                    .keys()
                    .nth(j)
                    .cloned()
                    .unwrap();
                let input = values_mut(panel.draft.as_mut().unwrap(), i)[&id].clone();
                let value = match input {
                    Input::Literal { value } => value,
                    _ => serde_json::Value::Null,
                };
                panel.choices = vec![
                    (
                        "输入固定值…".into(),
                        Change::Input(i, id.clone(), Input::Literal { value }),
                    ),
                    (
                        "清空此字段".into(),
                        Change::Input(
                            i,
                            id.clone(),
                            Input::Literal {
                                value: serde_json::Value::Null,
                            },
                        ),
                    ),
                    (
                        "执行时的当前时间".into(),
                        Change::Input(i, id.clone(), Input::Now),
                    ),
                    (
                        "触发记录 ID".into(),
                        Change::Input(i, id.clone(), Input::RecordId),
                    ),
                ];
                if let Some(field) = panel.target(i).fields.iter().find(|f| f.id == id).cloned() {
                    for option in field.options {
                        let value = if field.field_type == model::FieldType::MultiSelect {
                            serde_json::json!([option.id])
                        } else {
                            serde_json::json!(option.id)
                        };
                        panel.choices.push((
                            format!("选项 · {}", option.label),
                            Change::Input(i, id.clone(), Input::Literal { value }),
                        ));
                    }
                }
                let fields: Vec<_> = panel
                    .table()
                    .fields
                    .iter()
                    .map(|f| {
                        (
                            format!("引用字段 · {}", f.name),
                            Change::Input(
                                i,
                                id.clone(),
                                Input::Field {
                                    field_id: f.id.clone(),
                                },
                            ),
                        )
                    })
                    .collect();
                panel.choices.extend(fields);
                panel.scroll = 0.0;
            }
            Hit::RemoveValue(i, j) => {
                let values = values_mut(panel.draft.as_mut().unwrap(), i);
                if let Some(id) = values.keys().nth(j).cloned() {
                    values.remove(&id);
                }
                panel.edit_changed();
            }
            Hit::PickRecord => {
                let r = panel.draft.as_ref().unwrap();
                let mut view = panel.view().clone();
                view.filters.extend(r.conditions.clone());
                panel.choices = model::get_view_records(panel.table(), &view, "")
                    .into_iter()
                    .take(200)
                    .map(|r| (record_label(panel.table(), r), Change::Record(r.id.clone())))
                    .collect();
                panel.scroll = 0.0;
                if panel.choices.is_empty() {
                    panel.message =
                        vec!["当前没有满足视图及规则条件的记录。请先在表格中准备测试记录。".into()];
                }
            }
            Hit::Choice(i) => {
                let change = panel.choices[i].1.clone();
                let edit_literal =
                    i == 0 && matches!(&change, Change::Input(_, _, Input::Literal { .. }));
                panel.choices.clear();
                panel.scroll = 0.0;
                panel.edit_changed();
                match change {
                    Change::Trigger(t) => panel.draft.as_mut().unwrap().trigger = t,
                    Change::Watch(id) => {
                        if let Trigger::RecordUpdated { field_ids } =
                            &mut panel.draft.as_mut().unwrap().trigger
                        {
                            if field_ids.contains(&id) {
                                field_ids.retain(|x| x != &id);
                            } else {
                                field_ids.push(id);
                            }
                        }
                    }
                    Change::ConditionField(i, id) => {
                        panel.draft.as_mut().unwrap().conditions[i].field_id = id
                    }
                    Change::ConditionValue(i, value) => {
                        panel.draft.as_mut().unwrap().conditions[i].value = Some(value)
                    }
                    Change::Operator(i, o) => {
                        let c = &mut panel.draft.as_mut().unwrap().conditions[i];
                        c.operator = o;
                        if c.value.is_none() {
                            c.value = Some(serde_json::Value::Null);
                        }
                    }
                    Change::ActionType(i, target) => {
                        panel.draft.as_mut().unwrap().actions[i] = match target {
                            Some(table_id) => Action::CreateRecord {
                                table_id,
                                values: BTreeMap::new(),
                            },
                            None => Action::UpdateRecord {
                                values: BTreeMap::new(),
                            },
                        };
                    }
                    Change::AddValue(i, id) => {
                        values_mut(panel.draft.as_mut().unwrap(), i)
                            .entry(id)
                            .or_insert(Input::Literal {
                                value: serde_json::Value::Null,
                            });
                    }
                    Change::Input(i, id, input) => {
                        if edit_literal {
                            let value = match input {
                                Input::Literal { value } => value,
                                _ => unreachable!(),
                            };
                            text = Some((
                                TextEdit::Value(i, id),
                                "字段值".into(),
                                Some(display_value(&value)),
                                "文本直接输入；数字/布尔/数组填写 JSON。下拉可返回选择已有选项。"
                                    .into(),
                            ));
                        } else {
                            values_mut(panel.draft.as_mut().unwrap(), i).insert(id, input);
                        }
                    }
                    Change::Record(id) => panel.record = Some(id),
                }
            }
            Hit::CancelChoice => {
                panel.choices.clear();
                panel.scroll = 0.0;
            }
            Hit::Test => {
                let result = panel
                    .record
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("请先选择测试记录"))
                    .and_then(|id| {
                        engine::preview(
                            &panel.document,
                            &panel.table_id,
                            &panel.view_id,
                            panel.draft.as_ref().unwrap(),
                            &[id.clone()],
                            engine::now_ms(),
                        )
                    });
                match result {
                    Ok((doc, report)) => {
                        panel.message = vec![format!(
                            "试跑通过 · 预计更新 {}，新建 {}。尚未写入。",
                            report.updates, report.creates
                        )];
                        for table in &doc.tables {
                            let old = panel
                                .document
                                .tables
                                .iter()
                                .find(|t| t.id == table.id)
                                .unwrap();
                            for r in &table.records {
                                let before = old.records.iter().find(|x| x.id == r.id);
                                if before.is_none_or(|x| x.values != r.values) {
                                    for field in &table.fields {
                                        let after = r
                                            .values
                                            .get(&field.id)
                                            .unwrap_or(&serde_json::Value::Null);
                                        if before.and_then(|r| r.values.get(&field.id))
                                            != Some(after)
                                        {
                                            panel.message.push(format!(
                                                "{} / {} → {}",
                                                table.name,
                                                field.name,
                                                model::format_cell_value(field, after)
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        panel.preview = Some((doc, report));
                    }
                    Err(e) => {
                        panel.preview = None;
                        panel.message = vec![format!("试跑失败：{e}")];
                    }
                }
            }
            Hit::Run => run = true,
            Hit::Save | Hit::Pause => save = Some(false),
            Hit::Enable => save = Some(true),
            Hit::Delete => {
                text = Some((
                    TextEdit::Delete,
                    "删除自动化？".into(),
                    None,
                    "删除此规则，不删除任何表格记录；历史运行日志保留。".into(),
                ))
            }
            Hit::Logs => panel.logs = !panel.logs,
            Hit::Guide => {
                panel.message=vec!["向 Agent 说明：在当前视图，当什么发生且满足什么条件，对哪些字段做什么。".into(),"例：当任务进入待归档视图，在归档表创建记录，并复制标题与完成时间。".into(),"Agent 流程：base_get_schema → base_automation_list → base_automation_test → base_automation_put。".into(),"生成的是草稿；在此检查、试跑并确认启用。单次执行使用 base_automation_run。".into()];
            }
        }
        self.automation.panel = Some(panel);
        if let Some((edit, title, value, desc)) = text {
            self.automation_text(edit, &title, value, &desc);
        }
        if let Some(enabled) = save {
            self.save_automation_rule(enabled);
        }
        if run {
            self.run_automation_preview();
        }
        self.invalidate_main();
    }

    pub(in crate::app) fn submit_automation_text(&mut self, edit: TextEdit) {
        let value = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().to_owned())
            .unwrap_or_default();
        let result = (|| -> anyhow::Result<()> {
            let panel = self.automation.panel.as_mut().context("自动化面板已关闭")?;
            match edit {
                TextEdit::Name => {
                    anyhow::ensure!(
                        !value.trim().is_empty() && value.chars().count() <= 100,
                        "名称须为 1–100 字"
                    );
                    panel.draft.as_mut().unwrap().name = value;
                }
                TextEdit::Minutes => {
                    let n: u32 = value.parse().context("请输入整数分钟")?;
                    anyhow::ensure!((1..=525600).contains(&n), "范围为 1–525600 分钟");
                    if let Trigger::Interval { minutes, .. } =
                        &mut panel.draft.as_mut().unwrap().trigger
                    {
                        *minutes = n;
                    }
                }
                TextEdit::StartAt => {
                    let n = chrono::DateTime::parse_from_rfc3339(&value)
                        .context("请填写包含时区的 ISO 8601 时间")?
                        .timestamp_millis();
                    anyhow::ensure!(n >= 0, "时间不能早于 1970 年");
                    if let Trigger::Interval { start_at, .. } =
                        &mut panel.draft.as_mut().unwrap().trigger
                    {
                        *start_at = n as u64;
                    }
                }
                TextEdit::Condition(i) => {
                    let field = &panel.draft.as_ref().unwrap().conditions[i].field_id;
                    let kind = panel
                        .table()
                        .fields
                        .iter()
                        .find(|f| &f.id == field)
                        .map(|f| f.field_type);
                    let parsed = parse_value(&value, kind)?;
                    panel.draft.as_mut().unwrap().conditions[i].value = Some(parsed);
                }
                TextEdit::Value(i, id) => {
                    let kind = panel
                        .target(i)
                        .fields
                        .iter()
                        .find(|f| f.id == id)
                        .map(|f| f.field_type);
                    let parsed = parse_value(&value, kind)?;
                    values_mut(panel.draft.as_mut().unwrap(), i)
                        .insert(id, Input::Literal { value: parsed });
                }
                TextEdit::Delete => {
                    let id = panel.draft.as_ref().unwrap().id.clone();
                    let mut doc = panel.document.clone();
                    engine::remove(&mut doc, &panel.table_id, &panel.view_id, &id)?;
                    let path = panel.path.clone();
                    let original = panel.original.clone();
                    let content = model::serialize_base_document(&doc)?;
                    self.persist_automation(&path, &original, &content, &doc)?;
                    let panel = self.automation.panel.as_mut().unwrap();
                    panel.document = doc;
                    panel.original = content;
                    panel.draft = None;
                    panel.initial_draft = None;
                    panel.scroll = 0.0;
                    self.automation.cache.remove(&path);
                }
                TextEdit::Discard(close) => {
                    if close {
                        self.automation.panel = None;
                    } else {
                        panel.draft = None;
                        panel.initial_draft = None;
                        panel.scroll = 0.0;
                    }
                }
            }
            if let Some(panel) = self.automation.panel.as_mut() {
                panel.edit_changed();
            }
            Ok(())
        })();
        match result {
            Ok(()) => self.close_dialog(),
            Err(e) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = e.to_string();
                }
            }
        }
        self.invalidate_main();
    }
    fn save_automation_rule(&mut self, enabled: bool) {
        let result = (|| -> anyhow::Result<()> {
            let p = self.automation.panel.as_ref().context("面板已关闭")?;
            let path = p.path.clone();
            let table = p.table_id.clone();
            let view = p.view_id.clone();
            let original = p.original.clone();
            let mut rule = p.draft.clone().context("未选择自动化")?;
            rule.enabled = enabled;
            let mut doc = p.document.clone();
            engine::put(&mut doc, &table, &view, rule.clone())?;
            let (t, v) = engine::scope(&doc, &table, &view)?;
            let sig = engine::signature(&doc, t, v, &rule);
            let content = model::serialize_base_document(&doc)?;
            // 先保存授权信息。如果文档保存失败，旧签名就不会匹配。
            let mut all = grants(&self.settings);
            let key = grant_key(&path, &table, &view, &rule.id);
            if enabled {
                all.insert(
                    key,
                    Grant {
                        path: path.clone(),
                        table_id: table.clone(),
                        view_id: view.clone(),
                        rule_id: rule.id.clone(),
                        signature: sig,
                    },
                );
            } else {
                all.remove(&key);
            }
            self.settings.set(GRANTS_KEY, &serde_json::to_string(&all)?);
            self.settings.flush()?;
            self.persist_automation(&path, &original, &content, &doc)?;
            let baseline = engine::snapshot(&doc, &authorized(&all, &path));
            self.automation.cache.insert(
                path.clone(),
                Cache {
                    stamp: None,
                    next_due: 0,
                    snapshot: baseline,
                    consent: authorized(&all, &path),
                },
            );
            let p = self.automation.panel.as_mut().unwrap();
            p.document = doc;
            p.original = content;
            p.granted = authorized(&all, &path);
            p.draft = Some(rule);
            p.initial_draft = p.draft.clone();
            p.preview = None;
            p.message = vec![if enabled {
                "已启用。应用和工作区打开时自动执行，关闭此表格页面不影响运行。".into()
            } else {
                "已保存并暂停，不会自动执行。".into()
            }];
            Ok(())
        })();
        if let Err(e) = result {
            if let Some(p) = self.automation.panel.as_mut() {
                p.message = vec![format!("未保存：{e}")];
            }
        }
    }
    fn run_automation_preview(&mut self) {
        let result = (|| -> anyhow::Result<()> {
            let p = self.automation.panel.as_ref().context("面板已关闭")?;
            anyhow::ensure!(p.preview.is_some(), "请先试跑");
            let path = p.path.clone();
            let original = p.original.clone();
            let (planned, report) = p.preview.clone().context("请先试跑")?;
            let doc = engine::finish_preview(planned, &report)?;
            let content = model::serialize_base_document(&doc)?;
            self.persist_automation(&path, &original, &content, &doc)?;
            let p = self.automation.panel.as_mut().unwrap();
            p.document = doc;
            p.original = content;
            p.preview = None;
            p.message = vec![format!(
                "已执行一次：更新 {}，新建 {}。",
                report.updates, report.creates
            )];
            self.automation.cache.remove(&path);
            Ok(())
        })();
        if let Err(e) = result {
            if let Some(p) = self.automation.panel.as_mut() {
                p.preview = None;
                p.message = vec![format!("未执行：{e}")];
            }
        }
    }
}

fn display_value(v: &serde_json::Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
fn parse_value(value: &str, kind: Option<model::FieldType>) -> anyhow::Result<serde_json::Value> {
    if matches!(
        kind,
        Some(
            model::FieldType::Text
                | model::FieldType::Url
                | model::FieldType::Date
                | model::FieldType::DateTime
                | model::FieldType::SingleSelect
        )
    ) {
        Ok(serde_json::Value::String(value.into()))
    } else {
        serde_json::from_str(value)
            .context("此字段需要合法的 JSON 值，例如 42、true 或 [\"选项ID\"]")
    }
}
