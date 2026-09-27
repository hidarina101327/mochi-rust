//! 维护工作流编辑器状态，并协调刷新、计时和草稿保存。
mod actions;
mod library;
#[cfg(test)]
mod tests;
use super::*;
use crate::ui::workflows as ui;
use mochi_core::workflows::{self as core, Store};

#[derive(Default)]
pub(super) struct State {
    pub view: ui::State,
    pub root: Option<PathBuf>,
    pub store: Option<Store>,
    runtime: Option<core::runtime::Runtime>,
    host: Option<Arc<core::native_host::NativeHost>>,
    pub armed: bool,
    pub pending_input: Option<serde_json::Value>,
    dirty_heartbeat: i64,
    persisted_draft: Option<serde_json::Value>,
}
impl App {
    pub(super) fn workflows_sync(&mut self) {
        let root = self.shell.workspace().map(|w| w.root.clone());
        if self.workflows.root != root {
            self.workflows_persist_draft();
            self.workflows = State::default();
            self.workflows.root = root.clone();
            if let Some(root) = root {
                match Store::open(&root).and_then(|store| {
                    core::native_host::NativeHost::new(&root, self.settings.clone())
                        .map(|host| (store, Arc::new(host)))
                }) {
                    Ok((store, host)) => {
                        self.workflows.runtime =
                            Some(core::runtime::Runtime::start(store.clone(), host.clone()));
                        self.workflows.store = Some(store);
                        self.workflows.host = Some(host);
                        self.workflows_refresh();
                        self.workflows_restore_draft();
                    }
                    Err(e) => self.workflows.view.error = e,
                }
            }
        }
        if let Some(host) = &self.workflows.host {
            if let Ok(mut paths) = host.blocked_paths.lock() {
                let current = self
                    .shell
                    .tabs()
                    .iter()
                    .filter(|t| t.dirty())
                    .filter_map(|t| t.path().map(Path::to_path_buf))
                    .collect::<Vec<_>>();
                if *paths != current || core::now() - self.workflows.dirty_heartbeat > 10_000 {
                    if let Some(store) = &self.workflows.store {
                        let _ = store.set_dirty_paths(&current);
                    }
                    self.workflows.dirty_heartbeat = core::now();
                }
                *paths = current;
            }
        }
    }
    pub(super) fn workflows_timer(&mut self) {
        self.refresh_document_unread();
        self.workflows.armed = false;
        self.workflows_persist_draft();
        let Some(store) = self.workflows.store.clone() else {
            return;
        };
        let messages = self
            .workflows
            .runtime
            .as_ref()
            .map(|r| r.events.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        let completed = !messages.is_empty();
        for message in messages.into_iter().filter(|m| !m.starts_with("run_")) {
            self.workflows.view.error = message;
        }
        if completed {
            self.reload_schedule();
        }
        if let Ok(notices) = store.take_notices() {
            for (title, message) in notices {
                self.publish_notification(
                    crate::ui::notifications::Category::Automation,
                    &title,
                    &message,
                );
            }
        }
        if self.state.view == WorkspaceView::Automations {
            if self.workflows.view.draft.is_none() {
                self.workflows_refresh();
            }
            if let Some(id) = self
                .workflows
                .view
                .selected
                .as_ref()
                .map(|s| s.definition.id.clone())
            {
                if let Ok(history) = store.history(&id) {
                    self.workflows.view.history = history;
                }
            }
            if let Some(id) = self
                .workflows
                .view
                .run
                .as_ref()
                .filter(|r| matches!(r.status.as_str(), "queued" | "running"))
                .map(|r| r.id.clone())
            {
                if let Ok(run) = store.run(&id) {
                    self.workflows.view.run = Some(run);
                    self.workflows.view.refresh_result_text();
                }
            }
        }
    }
    pub(super) fn workflows_persist_draft(&mut self) {
        let Some(store) = &self.workflows.store else {
            return;
        };
        let v = &self.workflows.view;
        let value = if v.draft.is_some()
            && (v.dirty
                || (v.editor.is_some()
                    && v.editor != Some(ui::Editor::Result)
                    && v.field.text() != v.field_before))
        {
            Some(
                serde_json::json!({"workflow":v.draft,"selected":v.selected,"node":v.selected_node,"editor":v.editor,"field":v.field.text(),"field_before":v.field_before,"parameter":v.parameter,"advanced":v.advanced}),
            )
        } else {
            None
        };
        if value != self.workflows.persisted_draft {
            match store.save_editor_draft(value.as_ref()) {
                Ok(()) => self.workflows.persisted_draft = value,
                Err(e) => self.workflows.view.error = e,
            }
        }
    }
    fn workflows_restore_draft(&mut self) {
        let Some(value) = self
            .workflows
            .store
            .as_ref()
            .and_then(|s| s.editor_draft().ok().flatten())
        else {
            return;
        };
        let v = &mut self.workflows.view;
        v.draft = serde_json::from_value(value["workflow"].clone()).ok();
        v.selected = serde_json::from_value(value["selected"].clone()).ok();
        v.selected_node = value["node"].as_u64().map(|i| i as usize);
        v.editor = serde_json::from_value(value["editor"].clone()).ok();
        v.field.set_text(value["field"].as_str().unwrap_or(""));
        v.field_before = value["field_before"].as_str().unwrap_or("").into();
        v.parameter = value["parameter"].as_str().unwrap_or("").into();
        v.advanced = value["advanced"].as_bool().unwrap_or(false);
        v.dirty = true;
        v.status = "已恢复未保存的工作流草稿".into();
        self.workflows.persisted_draft = Some(value);
    }
    pub(super) fn workflows_refresh(&mut self) {
        if let Some(store) = &self.workflows.store {
            match store.folders() {
                Ok(folders) => self.workflows.view.folders = folders,
                Err(e) => self.workflows.view.error = e,
            }
            match store.summaries() {
                Ok(list) => self.workflows.view.workflows = list,
                Err(e) => self.workflows.view.error = e,
            }
        }
    }
    pub(super) fn workflows_open(&mut self) {
        self.workflows_sync();
        self.open_auxiliary_page(WorkspaceView::Automations);
        self.workflows_refresh();
        self.focus = Focus::Main;
        self.invalidate_main();
    }
    pub(super) fn workflows_active(&self) -> bool {
        self.state.view == WorkspaceView::Automations
            && self.dialog.is_none()
            && self.object_picker.is_none()
            && self.settings_overlay.is_none()
            && self.search.is_none()
            && self.command.is_none()
            && self.menu.is_none()
            && !self.notification_open()
    }
    pub(super) fn paint_workflows(&mut self, area: Rect, p: &Palette) {
        let error = std::mem::take(&mut self.workflows.view.error);
        let status = std::mem::take(&mut self.workflows.view.status);
        if !error.is_empty() {
            self.show_global_notice(format!("工作流：{error}"));
        } else if !status.is_empty() {
            self.show_global_notice(status);
        }
        if !self.prefs.providers.loaded {
            self.reload_providers();
        }
        self.workflows.view.provider_labels = self
            .prefs
            .providers
            .all
            .iter()
            .map(|provider| {
                (
                    provider.id.clone(),
                    format!("{} · {}", provider.name, provider.model),
                )
            })
            .collect();
        ui::paint(
            &mut self.list,
            area,
            &mut self.workflows.view,
            self.focus == Focus::Workflow,
            p,
        );
    }
    pub(super) fn workflows_click(&mut self, x: f32, y: f32) {
        self.workflows.view.shift = shift_down();
        if !self.workflows.view.search_rect.contains(x, y) {
            self.workflows.view.searching = false;
        }
        if self.workflows.view.field_rect.contains(x, y)
            && self.workflows.view.editor != Some(ui::Editor::Result)
        {
            self.focus = Focus::Workflow;
            let v = &mut self.workflows.view;
            v.field.multiline_click(v.field_rect, x, y);
            return;
        }
        self.focus = Focus::Main;
        let hit = self.workflows.view.hit(x, y);
        if matches!(
            self.workflows.view.editor,
            Some(
                ui::Editor::Definition
                    | ui::Editor::Rename
                    | ui::Editor::Trigger
                    | ui::Editor::Import
            )
        ) && self.workflows.view.field.text() != self.workflows.view.field_before
            && !matches!(
                hit,
                Some(
                    ui::Hit::Apply
                        | ui::Hit::Save
                        | ui::Hit::ImportFile
                        | ui::Hit::Back
                        | ui::Hit::CloseEditor
                ) | None
            )
        {
            self.workflows.view.error = "请先应用或保存当前编辑内容，再切换操作。".into();
            return;
        }
        if matches!(
            hit,
            Some(
                ui::Hit::Node(_)
                    | ui::Hit::Run
                    | ui::Hit::Save
                    | ui::Hit::Add
                    | ui::Hit::Definition
                    | ui::Hit::Trigger
                    | ui::Hit::Rename
            )
        ) {
            if let Err(e) = self.workflows.view.apply_node() {
                self.workflows.view.error = e;
                return;
            }
        }
        if let Some(h) = &hit {
            if let Err(e) = self.workflows_action(h.clone()) {
                self.workflows.view.error = e;
                return;
            }
        }
        self.workflows.view.begin(hit.as_ref(), x, y);
    }
    pub(super) fn workflows_wheel(&mut self, x: f32, y: f32, delta: i16) {
        let v = &mut self.workflows.view;
        let step = delta as f32 / 120.;
        if v.field_rect.contains(x, y)
            && (v.advanced
                || v.editor != Some(ui::Editor::Parameter)
                || v.field.text().lines().count() as f32 * 24. > v.field_rect.height())
        {
            v.field.scroll_multiline(v.field_rect, step * 54.);
        } else if v.inspector.contains(x, y)
            && matches!(v.editor, Some(ui::Editor::Node | ui::Editor::Parameter))
            && !v.advanced
        {
            v.form_scroll = (v.form_scroll - step * 64.)
                .clamp(0., (v.form_height - (v.inspector.height() - 202.)).max(0.));
        } else if v.inspector.contains(x, y)
            && (v.history_open || v.run.is_some() || v.settings_open)
        {
            v.panel_scroll = (v.panel_scroll - step * 64.)
                .clamp(0., (v.panel_height - v.panel_body.height()).max(0.));
        } else if (v.palette && v.palette_rect.contains(x, y)) || v.draft.is_none() {
            let (content, visible) = if v.palette {
                (
                    core::catalog::KINDS.len() as f32 * 58. + 6. * 28.,
                    (v.canvas.height() - 144.).max(0.),
                )
            } else if v.history_open {
                (
                    v.history.len() as f32 * 66.,
                    (v.inspector.height() - 56.).max(0.),
                )
            } else {
                (
                    v.library_height,
                    (v.area.height() - if v.area.width() < 760. { 206. } else { 164. }).max(0.),
                )
            };
            v.scroll = (v.scroll - step * 64.).clamp(0., (content - visible).max(0.));
        } else if v.canvas.contains(x, y) {
            v.zoom_at(x, y, 1.12_f32.powf(step));
        }
    }
    pub(super) fn workflows_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        if self.focus == Focus::Workflow && self.workflows.view.searching {
            if key == 0x1b {
                self.workflows.view.palette = false;
                self.workflows.view.searching = false;
                self.focus = Focus::Main;
                return true;
            }
            if ctrl && matches!(key, 0x43 | 0x56 | 0x58) {
                return false;
            }
            self.workflows.view.search.key(key, shift, ctrl);
            self.workflows.view.scroll = 0.;
            return true;
        }
        if self.focus == Focus::Workflow {
            if ctrl && key == 0x53 {
                if let Err(e) = self.workflows_action(ui::Hit::Save) {
                    self.workflows.view.error = e;
                }
                return true;
            }
            if key == 0x1b {
                self.focus = Focus::Main;
                return true;
            }
            let v = &mut self.workflows.view;
            if ctrl && (key == 0x5a || key == 0x59) {
                if key == 0x59 || shift {
                    v.field.buffer.redo();
                } else {
                    v.field.buffer.undo();
                }
                return true;
            }
            if ctrl && matches!(key, 0x43 | 0x56 | 0x58) {
                return false;
            }
            v.field.multiline_key(v.field_rect, key, shift, ctrl);
            return true;
        }
        if ctrl && key == 0x41 && self.workflows.view.run.is_none() {
            let v = &mut self.workflows.view;
            if let Err(error) = v.apply_node() {
                v.error = error;
                return true;
            }
            v.group = (0..v.draft.as_ref().map_or(0, |g| g.nodes.len())).collect();
            v.selected_node = None;
            v.selected_edge = None;
            v.editor = None;
            return true;
        }
        let hit = match (key, ctrl) {
            (0x53, true) => Some(ui::Hit::Save),
            (0x5a, true) => Some(if shift { ui::Hit::Redo } else { ui::Hit::Undo }),
            (0x59, true) => Some(ui::Hit::Redo),
            (0x2e, false) => Some(ui::Hit::Delete),
            (0x1b, false) => {
                self.workflows.view.palette = false;
                self.workflows.view.drag = None;
                None
            }
            _ => None,
        };
        if let Some(hit) = hit {
            if let Err(e) = self.workflows_action(hit) {
                self.workflows.view.error = e;
            }
            return true;
        }
        false
    }
}
