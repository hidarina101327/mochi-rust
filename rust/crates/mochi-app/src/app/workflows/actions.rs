//! 处理工作流保存和编辑器操作。
use super::*;
use serde_json::{json, Value};
use std::result::Result;

impl App {
    fn workflows_save(&mut self) -> Result<(), String> {
        if matches!(
            self.workflows.view.editor,
            Some(
                ui::Editor::Definition
                    | ui::Editor::Trigger
                    | ui::Editor::Rename
                    | ui::Editor::Import
            )
        ) && self.workflows.view.field.text() != self.workflows.view.field_before
        {
            self.workflow_apply()?;
        }
        self.workflows.view.apply_node()?;
        let store = self.workflows.store.as_ref().ok_or("请先打开工作区")?;
        let draft = self.workflows.view.draft.as_ref().ok_or("请先打开工作流")?;
        let saved = store.save(
            draft,
            self.workflows.view.selected.as_ref().map(|s| s.revision),
        )?;
        self.workflows.view.selected = Some(saved);
        self.workflows.view.dirty = false;
        self.workflows.view.status = "已保存".into();
        self.workflows.view.error.clear();
        Ok(())
    }
    pub(in crate::app) fn workflows_action(&mut self, hit: ui::Hit) -> Result<(), String> {
        use ui::{Editor, Hit};
        if self.workflows.view.run.is_some()
            && !matches!(
                hit,
                Hit::Node(_)
                    | Hit::Edge(_)
                    | Hit::Port(..)
                    | Hit::Input(_)
                    | Hit::CloseRun
                    | Hit::CancelRun
                    | Hit::CloseEditor
                    | Hit::CopyResult
                    | Hit::ResultPage(_)
                    | Hit::Fit
                    | Hit::ZoomIn
                    | Hit::ZoomOut
                    | Hit::ResetZoom
                    | Hit::Minimap
                    | Hit::Map
                    | Hit::Resize
                    | Hit::History
                    | Hit::RunHistory(_)
                    | Hit::ResultOverview
                    | Hit::ResultTab(_)
                    | Hit::OpenArtifact(_)
                    | Hit::ClosePanel
                    | Hit::Add
            )
        {
            return Err("执行快照只读，请先返回画布编辑".into());
        }
        if self.workflows_library_action(&hit)? {
            return Ok(());
        }
        if !matches!(hit, Hit::Back | Hit::CloseEditor) {
            self.workflows.view.apply_node()?;
        }
        if !matches!(
            hit,
            Hit::Node(_)
                | Hit::Port(..)
                | Hit::Input(_)
                | Hit::Edge(_)
                | Hit::OutputVariable(..)
                | Hit::InputVariable(..)
                | Hit::Binding(..)
                | Hit::Kind(_)
                | Hit::Resize
        ) {
            self.workflows.view.drag = None;
        }
        match hit {
            Hit::Validate => {
                core::validate(self.workflows.view.draft.as_ref().ok_or("请先打开工作流")?)?;
                self.workflows.view.status = "✓ 配置检查通过；尚未执行工作流".into();
            }
            Hit::Section(section) => {
                let v = &mut self.workflows.view;
                if !v.folded.remove(&section) {
                    v.folded.insert(section);
                }
                v.form_scroll = 0.;
            }
            Hit::ConfigTab(history) => {
                self.workflows.view.node_history = history;
                self.workflows.view.form_scroll = 0.;
                if history {
                    let id = self
                        .workflows
                        .view
                        .draft
                        .as_ref()
                        .ok_or("请选择工作流")?
                        .id
                        .clone();
                    self.workflows.view.history = self
                        .workflows
                        .store
                        .as_ref()
                        .ok_or("工作区不可用")?
                        .history(&id)?;
                    self.workflows.view.history_open = true;
                    self.workflows.view.panel_scroll = 0.;
                    self.workflows.view.editor = None;
                }
            }
            Hit::More => {
                let items = [
                    ("工作流设置", Hit::Settings),
                    ("启用定时运行", Hit::Authorize),
                    ("设置本次运行输入", Hit::RunOptions),
                    ("触发规则", Hit::Trigger),
                    ("运行历史", Hit::History),
                    ("查看 / 导入 JSON", Hit::Definition),
                    ("导出 ZIP", Hit::Export),
                    ("返回全部工作流", Hit::Back),
                ]
                .into_iter()
                .map(|(label, hit)| MenuItem::new(label, MenuAction::WorkflowAction(hit)))
                .collect();
                let anchor = self
                    .workflows
                    .view
                    .hits
                    .iter()
                    .find(|(_, hit)| *hit == Hit::More)
                    .map(|(r, _)| *r)
                    .unwrap_or(self.workflows.view.area);
                self.menu = Some(Menu::open_anchored(items, anchor, self.renderer.viewport()));
            }
            Hit::NodeMenu(i) => {
                self.workflows.view.select_node(i);
                let r = self
                    .workflows
                    .view
                    .node_rect(&self.workflows.view.graph().unwrap().nodes[i]);
                self.menu = Some(Menu::open_anchored(
                    vec![
                        MenuItem::new("复制节点", MenuAction::WorkflowAction(Hit::Duplicate)),
                        MenuItem::new("删除节点", MenuAction::WorkflowAction(Hit::Delete)),
                    ],
                    r,
                    self.renderer.viewport(),
                ));
            }
            Hit::Choice(path, value) => {
                self.workflows.view.edit_parameter(&path)?;
                self.workflows.view.field.set_text(&value);
                self.workflows.view.apply_node()?;
                self.focus = Focus::Main;
            }
            Hit::RemoveBinding(target, key) => self.workflows.view.remove_binding(target, &key),
            Hit::SetVariable(node_id, path, reference) => {
                if !self
                    .workflows
                    .view
                    .selected_node
                    .and_then(|i| self.workflows.view.draft.as_ref()?.nodes.get(i))
                    .is_some_and(|n| n.id == node_id)
                {
                    return Err("节点已切换，请重新选择变量".into());
                }
                self.workflows.view.edit_parameter(&path)?;
                self.workflows.view.field.set_text(&reference);
                self.workflows.view.apply_node()?;
                self.focus = Focus::Main;
            }
            Hit::Variables => {
                let path = self.workflows.view.parameter.clone();
                if path.is_empty() {
                    return Err("请先选择一个输入变量".into());
                }
                return self.workflows_action(Hit::VariablePicker(path));
            }
            Hit::VariablePicker(path) => {
                self.workflows.view.edit_parameter(&path)?;
                self.focus = Focus::Main;
                let v = &self.workflows.view;
                let node_id = v.draft.as_ref().unwrap().nodes[v.selected_node.unwrap()]
                    .id
                    .clone();
                let mut items = vec![MenuItem::new(
                    "填写自定义值…",
                    MenuAction::WorkflowAction(Hit::Parameter(path.clone())),
                )];
                items.extend(v.variable_options().into_iter().map(|(label, value)| {
                    MenuItem::new(
                        label,
                        MenuAction::WorkflowAction(Hit::SetVariable(
                            node_id.clone(),
                            path.clone(),
                            value,
                        )),
                    )
                }));
                let r = v
                    .hits
                    .iter()
                    .find(|(_, h)| *h == Hit::VariablePicker(path.clone()))
                    .map(|(r, _)| *r)
                    .unwrap_or(v.inspector);
                self.menu = Some(Menu::open_searchable_anchored(
                    items,
                    r,
                    self.renderer.viewport(),
                    "搜索节点或变量",
                ));
            }
            Hit::Parameter(path) => {
                self.workflows.view.edit_parameter(&path)?;
                self.focus = Focus::Workflow;
                let boolean = self
                    .workflows
                    .view
                    .selected_node
                    .and_then(|i| self.workflows.view.draft.as_ref()?.nodes.get(i))
                    .and_then(|n| serde_json::to_value(n).ok())
                    .is_some_and(|n| n.pointer(&path).is_some_and(Value::is_boolean));
                if boolean && !path.starts_with("/inputs/") {
                    let value = if self.workflows.view.field.text() == "true" {
                        "false"
                    } else {
                        "true"
                    };
                    self.workflows.view.field.set_text(value);
                    self.workflows.view.apply_node()?;
                    self.focus = Focus::Main;
                    return Ok(());
                }
                let choices = ui::parameter_choices(&path);
                if !choices.is_empty() {
                    let items = choices
                        .into_iter()
                        .map(|(value, label)| {
                            MenuItem::new(
                                label,
                                MenuAction::WorkflowAction(Hit::Choice(path.clone(), value.into())),
                            )
                        })
                        .collect();
                    let r = self
                        .workflows
                        .view
                        .hits
                        .iter()
                        .find(|(_, h)| *h == Hit::Parameter(path.clone()))
                        .map(|(r, _)| *r)
                        .unwrap_or(self.workflows.view.inspector);
                    self.menu = Some(Menu::open_anchored(items, r, self.renderer.viewport()));
                    return Ok(());
                }
                if path == "/config/provider_id" {
                    let v = &self.workflows.view;
                    let id = v.draft.as_ref().unwrap().nodes[v.selected_node.unwrap()]
                        .id
                        .clone();
                    let mut items = vec![
                        MenuItem::new(
                            "当前默认提供商",
                            MenuAction::WorkflowProvider(id.clone(), Some(String::new())),
                        ),
                        MenuItem::new(
                            "手动填写 ID",
                            MenuAction::WorkflowProvider(id.clone(), None),
                        ),
                    ];
                    for provider in mochi_core::ai::providers::list(&self.settings) {
                        items.push(MenuItem::new(
                            format!("{} · {}", provider.name, provider.model),
                            MenuAction::WorkflowProvider(id.clone(), Some(provider.id)),
                        ));
                    }
                    let r = v
                        .hits
                        .iter()
                        .find(|(_, h)| *h == Hit::Parameter(path.clone()))
                        .map(|(r, _)| *r)
                        .unwrap_or(v.inspector);
                    self.menu = Some(Menu::open_searchable_anchored(
                        items,
                        r,
                        self.renderer.viewport(),
                        "搜索已配置的模型",
                    ));
                }
            }
            Hit::Advanced => {
                if let Some(i) = self.workflows.view.selected_node {
                    self.workflows.view.select_node(i);
                    self.workflows.view.advanced = true;
                    self.focus = Focus::Workflow;
                }
            }
            Hit::Duplicate => self.workflows.view.duplicate_node(),
            Hit::AutoLayout => self.workflows.view.auto_layout(),
            Hit::Minimap => self.workflows.view.minimap = !self.workflows.view.minimap,
            Hit::ResetZoom => {
                let v = &mut self.workflows.view;
                let r = v.canvas;
                v.zoom_at(
                    (r.left + r.right) / 2.,
                    (r.top + r.bottom) / 2.,
                    1. / v.zoom,
                );
            }
            Hit::OpenArtifact(ref value) => {
                let root = self.workflows.root.as_ref().ok_or("工作区不可用")?;
                let path = Path::new(value);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.join(path)
                };
                if !path.is_file() {
                    return Err(format!("生成的文件已移动或不存在：{}", path.display()));
                }
                self.workflows_persist_draft();
                if self.open_file_from_ui(&path) {
                    self.state.view = WorkspaceView::Editor;
                }
            }
            Hit::ClosePanel => {
                self.workflows.view.history_open = false;
                self.workflows.view.settings_open = false;
                self.workflows.view.editor = None;
                self.workflows.view.selected_node = None;
                self.workflows.view.result_hidden = true;
            }
            Hit::ResultOverview => {
                let v = &mut self.workflows.view;
                v.history_open = false;
                v.selected_node = None;
                v.editor = None;
                v.result_tab = 0;
                v.result_hidden = false;
                v.panel_scroll = 0.;
                v.refresh_result_text();
            }
            Hit::ResultTab(tab) => {
                let v = &mut self.workflows.view;
                v.result_tab = tab;
                v.panel_scroll = 0.;
                v.refresh_result_text();
                if tab == 2 {
                    v.show_result_page();
                } else {
                    v.editor = None;
                }
            }
            Hit::Settings => {
                self.workflows.view.panel_scroll = 0.;
                self.workflows.view.settings_open = !self.workflows.view.settings_open;
                self.workflows.view.editor = None;
                self.workflows.view.history_open = false;
            }
            Hit::PaletteSearch => {
                self.workflows.view.searching = true;
                self.focus = Focus::Workflow;
            }
            Hit::Insert(i) => {
                self.workflows.view.insert_edge = Some(i);
                self.workflows.view.palette = true;
                self.workflows.view.scroll = 0.;
            }
            Hit::ImportFile => {
                if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                    let text = mochi_core::transfer::read_content(
                        &path,
                        mochi_core::transfer::Kind::Workflow,
                    )
                    .map_err(|e| e.to_string())?;
                    self.workflows.view.set_editor(Editor::Import, text);
                    self.focus = Focus::Workflow;
                }
            }
            Hit::ResultPage(next) => {
                let v = &mut self.workflows.view;
                let pages = v.result_text.chars().count().div_ceil(8000).max(1);
                v.result_page = if next {
                    (v.result_page + 1).min(pages - 1)
                } else {
                    v.result_page.saturating_sub(1)
                };
                v.show_result_page();
            }
            Hit::Sample(weekly) => {
                let flow = if weekly {
                    core::templates::weekly_documents()
                } else {
                    core::templates::daily_web()
                };
                let saved = self
                    .workflows
                    .store
                    .as_ref()
                    .ok_or("请先打开工作区")?
                    .save(&flow, None)?;
                self.workflows.view.open(saved);
                self.workflows.view.status =
                    "请先在 JSON 中修改 defaults 的网址、文件夹，再保存运行。".into();
            }
            Hit::CopyResult => {
                platform::copy_to_clipboard(&self.workflows.view.result_text);
                self.workflows.view.status = "完整结果已复制".into();
            }
            Hit::New => {
                let store = self.workflows.store.as_ref().ok_or("请先打开工作区")?;
                let saved = store.save(&core::Workflow::blank(), None)?;
                store
                    .move_to_folder(&saved.definition.id, self.workflows.view.folder.as_deref())?;
                self.workflows.view.open(saved);
            }
            Hit::Open(i) => {
                if let Some(saved) = self.workflows.view.workflows.get(i).cloned() {
                    self.workflows.view.open(
                        self.workflows
                            .store
                            .as_ref()
                            .ok_or("工作区不可用")?
                            .get(&saved.id)?,
                    );
                }
            }
            Hit::Back => {
                if self.workflows.view.dirty
                    || self.workflows.view.field.text() != self.workflows.view.field_before
                {
                    self.workflow_dialog(
                        "放弃未保存的修改？",
                        "已保存的版本和执行历史不受影响。",
                        DialogAction::WorkflowDiscard,
                    );
                } else {
                    let folder = self.workflows.view.folder.clone();
                    self.workflows.view = ui::State::default();
                    self.workflows.view.folder = folder;
                    self.workflows_refresh();
                }
            }
            Hit::Save => self.workflows_save()?,
            Hit::Import => {
                self.workflows
                    .view
                    .set_editor(Editor::Import, String::new());
                self.focus = Focus::Workflow;
            }
            Hit::Definition => {
                let draft = self.workflows.view.draft.as_ref().ok_or("请先选择工作流")?;
                self.workflows.view.set_editor(
                    Editor::Definition,
                    serde_json::to_string_pretty(draft).map_err(|e| e.to_string())?,
                );
                self.focus = Focus::Workflow;
            }
            Hit::Export => {
                self.workflows.view.apply_node()?;
                let draft = self.workflows.view.draft.as_ref().ok_or("请先选择工作流")?;
                let text = serde_json::to_string_pretty(draft).map_err(|e| e.to_string())?;
                if let Some(path) = platform::save_file(
                    HWND(self.hwnd_raw as *mut _),
                    &format!("{}.mochi-workflow.zip", draft.id),
                ) {
                    mochi_core::transfer::Package {
                        kind: mochi_core::transfer::Kind::Workflow,
                        name: draft.name.clone(),
                        content: text,
                    }
                    .write(&path)
                    .map_err(|e| e.to_string())?;
                    self.workflows.view.status = format!("已导出：{}", path.display());
                }
            }
            Hit::Run => {
                if self.workflows.view.editor == Some(Editor::RunInput) {
                    return self.workflow_apply();
                }
                if self.workflows.view.dirty
                    || self.workflows.view.field.text() != self.workflows.view.field_before
                {
                    self.workflows_save()?;
                }
                self.workflows.pending_input = None;
                self.workflow_confirm(true, false)?;
            }
            Hit::RunOptions => {
                if self.workflows.view.dirty
                    || self.workflows.view.field.text() != self.workflows.view.field_before
                {
                    self.workflows_save()?;
                }
                let defaults = self
                    .workflows
                    .view
                    .draft
                    .as_ref()
                    .ok_or("请选择工作流")?
                    .defaults
                    .clone();
                self.workflows.view.set_editor(
                    Editor::RunInput,
                    serde_json::to_string_pretty(&defaults).unwrap_or_default(),
                );
                self.focus = Focus::Workflow;
            }
            Hit::Authorize => {
                if self.workflows.view.dirty
                    || self.workflows.view.field.text() != self.workflows.view.field_before
                {
                    self.workflows_save()?;
                }
                self.workflow_confirm(false, true)?;
            }
            Hit::Pause => {
                let id = &self
                    .workflows
                    .view
                    .selected
                    .as_ref()
                    .ok_or("请先保存")?
                    .definition
                    .id;
                let store = self.workflows.store.as_ref().ok_or("工作区不可用")?;
                store.pause(id)?;
                self.workflows.view.selected = Some(store.get(id)?);
                self.workflows.view.status = "定时已暂停；正在运行的任务可在历史中停止".into();
            }
            Hit::History => {
                self.workflows.view.history_open = !self.workflows.view.history_open;
                self.workflows.view.panel_scroll = 0.;
                self.workflows.view.editor = None;
                self.workflows.view.selected_node = None;
                let id = &self
                    .workflows
                    .view
                    .selected
                    .as_ref()
                    .ok_or("请先保存")?
                    .definition
                    .id;
                self.workflows.view.history = self
                    .workflows
                    .store
                    .as_ref()
                    .ok_or("工作区不可用")?
                    .history(id)?;
            }
            Hit::RunHistory(i) => {
                let id = self
                    .workflows
                    .view
                    .history
                    .get(i)
                    .ok_or("记录不存在")?
                    .id
                    .clone();
                let run = self
                    .workflows
                    .store
                    .as_ref()
                    .ok_or("工作区不可用")?
                    .run(&id)?;
                self.workflows.view.show_run(run);
            }
            Hit::CloseRun => {
                self.workflows.view.palette = self.workflows.view.run_palette;
                self.workflows.view.panel_scroll = 0.;
                self.workflows.view.history_open = false;
                self.workflows.view.needs_fit = true;
                self.workflows.view.run = None;
                self.workflows.view.editor = None;
                self.workflows.view.selected_node = None;
            }
            Hit::CancelRun => {
                let run = self.workflows.view.run.as_ref().ok_or("没有运行记录")?;
                self.workflows
                    .store
                    .as_ref()
                    .ok_or("工作区不可用")?
                    .cancel(&run.id)?;
                self.workflows.view.status = "停止请求已发送；已执行的写入不会回滚".into();
            }
            Hit::Trigger => {
                let trigger = &self
                    .workflows
                    .view
                    .draft
                    .as_ref()
                    .ok_or("请选择工作流")?
                    .trigger;
                self.workflows.view.set_editor(
                    Editor::Trigger,
                    serde_json::to_string_pretty(trigger).unwrap_or_default(),
                );
                self.focus = Focus::Workflow;
                self.workflows.view.status="支持 manual、interval(minutes)、daily(time,utc_offset_minutes)、weekly(另加 weekdays:1–7)".into();
            }
            Hit::Rename => {
                let d = self.workflows.view.draft.as_ref().ok_or("请选择工作流")?;
                self.workflows.view.set_editor(
                    Editor::Rename,
                    serde_json::to_string_pretty(
                        &json!({"name":d.name,"description":d.description}),
                    )
                    .unwrap_or_default(),
                );
                self.focus = Focus::Workflow;
            }
            Hit::Apply => self.workflow_apply()?,
            Hit::CloseEditor if self.workflows.view.editor == Some(Editor::Folder) => {
                self.workflows.view.editor = None;
                self.workflows.view.field_before = self.workflows.view.field.text().into();
                self.focus = Focus::Main;
            }
            Hit::CloseEditor => {
                if self.workflows.view.editor != Some(Editor::Result)
                    && self.workflows.view.field.text() != self.workflows.view.field_before
                {
                    self.workflow_dialog(
                        "放弃当前参数编辑？",
                        "仅放弃参数面板中尚未应用的内容，画布上的其他修改仍保留。",
                        DialogAction::WorkflowDiscardEditor,
                    );
                } else {
                    self.workflows.view.editor = None;
                    self.workflows.view.selected_node = None;
                    self.focus = Focus::Main;
                }
            }
            Hit::Add => {
                self.workflows.view.insert_edge = None;
                self.workflows.view.palette = !self.workflows.view.palette;
                self.workflows.view.scroll = 0.;
            }
            Hit::Kind(_) => {}
            Hit::Undo => self.workflows.view.undo(false),
            Hit::Redo => self.workflows.view.undo(true),
            Hit::Fit => self.workflows.view.fit(),
            Hit::ZoomIn | Hit::ZoomOut => {
                let r = self.workflows.view.canvas;
                self.workflows.view.zoom_at(
                    (r.left + r.right) * 0.5,
                    (r.top + r.bottom) * 0.5,
                    if hit == Hit::ZoomIn { 1.15 } else { 1. / 1.15 },
                );
            }
            Hit::Delete => self.workflows.view.remove_selected(),
            Hit::DeleteFlow => {
                self.workflow_dialog(
                    "删除这个工作流？",
                    "执行历史仍会保留；运行中的工作流不能删除。",
                    DialogAction::WorkflowDelete,
                );
            }
            Hit::PickObject => {
                if let Some(i) = self.workflows.view.selected_node {
                    self.workflows.view.select_node(i);
                    self.workflows.view.advanced = true;
                }
                self.open_object_picker(super::super::object_picker_host::Purpose::Workflow);
            }
            _ => {}
        }
        Ok(())
    }
    fn workflow_apply(&mut self) -> Result<(), String> {
        use ui::Editor;
        let text = self.workflows.view.field.text().to_owned();
        let editor = self.workflows.view.editor;
        if text.len() > 524288 {
            return Err("JSON 超过 512 KiB".into());
        }
        match editor {
            Some(Editor::Folder) => {
                let store = self.workflows.store.as_ref().ok_or("请先打开工作区")?;
                store.save_folder(self.workflows.view.editing_folder.as_deref(), &text)?;
                self.workflows.view.editor = None;
                self.workflows.view.field_before = text;
                self.workflows_refresh();
                self.workflows.view.status = "文件夹已保存".into();
                self.focus = Focus::Main;
            }
            Some(Editor::Node | Editor::Parameter) => {
                self.workflows.view.apply_node()?;
                self.workflows.view.status = "参数已应用，保存后生效".into();
            }
            Some(Editor::Definition) => {
                let mut flow: core::Workflow =
                    serde_json::from_str(&text).map_err(|e| e.to_string())?;
                flow.id = self
                    .workflows
                    .view
                    .draft
                    .as_ref()
                    .ok_or("工作流不存在")?
                    .id
                    .clone();
                core::validate(&flow)?;
                self.workflows.view.checkpoint();
                self.workflows.view.draft = Some(flow);
                self.workflows.view.editor = None;
                self.workflows.view.selected_node = None;
            }
            Some(Editor::Import) => {
                let saved = self
                    .workflows
                    .store
                    .as_ref()
                    .ok_or("请先打开工作区")?
                    .import(&text)?;
                self.workflows
                    .store
                    .as_ref()
                    .unwrap()
                    .move_to_folder(&saved.definition.id, self.workflows.view.folder.as_deref())?;
                self.workflows.view.open(saved);
            }
            Some(Editor::Trigger) => {
                let trigger = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let mut draft = self.workflows.view.draft.clone().ok_or("请选择工作流")?;
                draft.trigger = trigger;
                core::validate(&draft)?;
                self.workflows.view.checkpoint();
                self.workflows.view.draft = Some(draft);
                self.workflows.view.editor = None;
            }
            Some(Editor::Rename) => {
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let name = value["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("名称不能为空")?;
                self.workflows.view.checkpoint();
                if let Some(d) = &mut self.workflows.view.draft {
                    d.name = name.into();
                    d.description = value["description"].as_str().unwrap_or("").into();
                }
                self.workflows.view.editor = None;
            }
            Some(Editor::RunInput) => {
                let input: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                if !input.is_object() {
                    return Err("输入应为 JSON 对象".into());
                }
                self.workflows.pending_input = Some(input);
                self.workflow_confirm(true, false)?;
            }
            _ => {}
        }
        self.workflows.view.error.clear();
        Ok(())
    }
    fn workflow_confirm(&mut self, run: bool, enable: bool) -> Result<(), String> {
        let saved = self.workflows.view.selected.clone().ok_or("请先保存")?;
        let enable = enable && !matches!(saved.definition.trigger, core::Trigger::Manual);
        if run {
            return self.workflow_start();
        }
        let store = self.workflows.store.as_ref().ok_or("工作区不可用")?;
        store.set_schedule(
            &saved.definition.id,
            saved.revision,
            enable || saved.enabled,
        )?;
        self.workflows.view.selected = Some(store.get(&saved.definition.id)?);
        self.workflows.view.status = if enable {
            "定时运行已启用"
        } else {
            "工作流已就绪"
        }
        .into();
        Ok(())
    }
    fn workflow_dialog(&mut self, title: &str, description: &str, action: DialogAction) {
        self.dialog = Some(Dialog {
            title: title.into(),
            description: "请确认操作范围。".into(),
            field: None,
            error: String::new(),
            note: Some(description.into()),
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "确认".into(),
                    kind: ButtonKind::Primary,
                    action,
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }
    pub(in crate::app) fn workflow_approve(
        &mut self,
        id: &str,
        revision: i64,
        enable: bool,
        run: bool,
    ) {
        let result = (|| {
            let store = self.workflows.store.as_ref().ok_or("工作区不可用")?;
            store.set_schedule(id, revision, enable)?;
            self.workflows.view.selected = Some(store.get(id)?);
            if run {
                self.workflow_start()?;
            }
            Ok::<_, String>(())
        })();
        self.close_dialog();
        if let Err(e) = result {
            self.workflows.view.error = e;
        }
    }
    fn workflow_start(&mut self) -> Result<(), String> {
        let id = &self
            .workflows
            .view
            .selected
            .as_ref()
            .ok_or("请先保存")?
            .definition
            .id;
        let run = self
            .workflows
            .store
            .as_ref()
            .ok_or("工作区不可用")?
            .enqueue(
                id,
                self.workflows.pending_input.take().unwrap_or(json!({})),
                "manual",
                None,
            )?
            .ok_or("该工作流正在运行")?;
        self.workflows.view.show_run(run);
        self.focus = Focus::Main;
        Ok(())
    }
    pub(in crate::app) fn workflow_delete(&mut self) {
        let result = (|| {
            let saved = self
                .workflows
                .view
                .selected
                .as_ref()
                .ok_or("请先选择工作流")?;
            self.workflows
                .store
                .as_ref()
                .ok_or("工作区不可用")?
                .delete(&saved.definition.id, saved.revision)
        })();
        self.close_dialog();
        match result {
            Ok(()) => {
                self.workflows.view = ui::State::default();
                self.workflows_refresh();
            }
            Err(e) => self.workflows.view.error = e,
        }
    }
}
