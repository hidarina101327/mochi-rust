//! 管理数据表视图、详情弹窗及其交互。
use super::*;

impl App {
    pub(super) fn base_detail_open(&self) -> bool {
        self.state.view == WorkspaceView::Editor
            && matches!(self.viewer_tab(), Some((_, viewer::Content::Base(state))) if state.detail.is_some())
    }
    pub(super) fn base_interaction_area(&self) -> Rect {
        if self.base_detail_open() {
            self.renderer.viewport()
        } else {
            self.editor_area
        }
    }
    pub(super) fn paint_base_modal(&mut self, palette: &theme::Palette) {
        if !self.base_detail_open() {
            return;
        }
        let viewport = self.renderer.viewport();
        if let Some(TabKind::Viewer {
            content: viewer::Content::Base(state),
            ..
        }) = self.shell.active().map(|tab| &tab.kind)
        {
            base_view::paint_modal(&mut self.list, viewport, state, palette);
        }
    }
    pub(super) fn open_base_dialog(&mut self, parent: PathBuf) {
        self.dialog = Some(Dialog {
            title: "新建多维表格".into(),
            description: "创建通用 .mcb 文件，添加字段、记录与视图".into(),
            field: Some(TextField::new("文件名称")),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "创建".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::CreateBase(parent),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }
    pub(super) fn create_base_from_dialog(&mut self, parent: &Path) {
        let value = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().trim())
            .unwrap_or("");
        let name = match sidebar::finalize_name(value, true) {
            Ok(name) => {
                if name.to_ascii_lowercase().ends_with(".mcb") {
                    name
                } else {
                    format!("{name}.mcb")
                }
            }
            Err(error) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = error.into();
                }
                return;
            }
        };
        match self.shell.create_file(parent, &name) {
            Ok(path) => {
                self.close_dialog();
                self.open_file_from_ui(&path);
                self.state.view = WorkspaceView::Editor;
                self.invalidate_main();
                self.sync_state();
            }
            Err(error) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = error.to_string();
                }
            }
        }
    }
    pub(super) fn on_base_click(&mut self, hit: base_view::Hit) {
        if hit == base_view::Hit::Automations {
            self.open_automations();
            return;
        }
        if hit == base_view::Hit::Save {
            self.save_active();
            self.sync_state();
            return;
        }
        let Some((path, viewer::Content::Base(state))) = self.viewer_tab() else {
            return;
        };
        let path = path.to_path_buf();
        let original =
            mochi_core::base::serialize_base_document(&state.document).unwrap_or_default();
        let prompt = match self.viewer_content_mut() {
            Some(viewer::Content::Base(s)) => s.activate(hit),
            _ => None,
        };
        let action = match self.viewer_content_mut() {
            Some(viewer::Content::Base(state)) => state.action.take(),
            _ => None,
        };
        if let Some(action) = action {
            match action {
                base_view::Action::ExportXlsx => {
                    self.export_base_xlsx();
                    return;
                }
                base_view::Action::Automations => {
                    self.open_automations();
                    return;
                }
                base_view::Action::Open(url) => {
                    self.open_link(&url);
                    return;
                }
                base_view::Action::CopyRecord(record, field) => {
                    if let Some((path, viewer::Content::Base(state))) = self.viewer_tab() {
                        if let Some(record) = state.table().records.get(record) {
                            let root = self
                                .shell
                                .workspace()
                                .map(|workspace| workspace.root.as_path());
                            let field = field
                                .and_then(|field| state.table().fields.get(field))
                                .map(|field| field.id.as_str());
                            let url = mochi_url::build_mochi_record_url(
                                path,
                                &state.table().id,
                                &record.id,
                                field,
                                root,
                            );
                            let copied = platform::copy_to_clipboard(&url);
                            self.show_global_notice(if copied {
                                "记录链接已复制"
                            } else {
                                "复制失败，请重试"
                            });
                        }
                    }
                }
                base_view::Action::CreateTemporaryDocument => {
                    match mochi_core::object_reference::temporary_documents_dir(&path).and_then(
                        |folder| {
                            std::fs::create_dir_all(&folder)?;
                            Ok(folder)
                        },
                    ) {
                        Ok(folder) => self.open_create_dialog(folder, false),
                        Err(error) => {
                            self.state.status_text = format!("创建临时文档目录失败：{error}")
                        }
                    }
                    return;
                }
                base_view::Action::LoadReferences(record, field) => {
                    self.pick_base_references(record, field);
                    // 此交互由共享对象选择器处理，不要
                    // 继续进入下方旧版的提示框或菜单流程。
                    return;
                }
            }
        }
        if let Some(prompt) = prompt {
            let destructive_conversion = matches!(prompt.edit, base_view::Edit::FieldType(..));
            let field = prompt.value.map(|text| {
                let mut field = TextField::new("");
                field.set_text(&text);
                field.style = TextStyle::Table;
                field
            });
            self.dialog = Some(Dialog {
                title: prompt.title,
                description: prompt.description,
                field,
                error: String::new(),
                note: None,
                buttons: vec![
                    DialogButton {
                        label: "取消".into(),
                        kind: ButtonKind::Ghost,
                        action: DialogAction::Dismiss,
                    },
                    DialogButton {
                        label: if destructive_conversion {
                            "更改并清空"
                        } else {
                            "确定"
                        }
                        .into(),
                        kind: if destructive_conversion {
                            ButtonKind::Danger
                        } else {
                            ButtonKind::Primary
                        },
                        action: DialogAction::BaseEdit {
                            path,
                            edit: prompt.edit,
                            original,
                        },
                    },
                ],
                dismiss: DialogAction::Dismiss,
                hover: None,
            });
            self.focus = Focus::Dialog;
        }
        self.schedule_autosave();
        self.sync_state();
    }
    pub(super) fn submit_base_dialog(
        &mut self,
        path: &Path,
        edit: base_view::Edit,
        original: &str,
    ) {
        let value = self
            .dialog
            .as_ref()
            .and_then(|d| d.field.as_ref())
            .map(|f| f.text().to_owned())
            .unwrap_or_default();
        let result = match self.viewer_content_for(path) {
            Some(viewer::Content::Base(s)) => {
                if mochi_core::base::serialize_base_document(&s.document)
                    .is_ok_and(|raw| raw == original)
                {
                    s.submit(edit, &value)
                } else {
                    Err("数据表已变化，请关闭对话框后重新编辑".into())
                }
            }
            _ => Err("文件已关闭".into()),
        };
        match result {
            Ok(()) => {
                self.close_dialog();
                self.schedule_autosave();
                self.sync_state();
            }
            Err(error) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = error;
                }
            }
        }
    }

    pub(super) fn on_base_pointer_down(&mut self, x: f32, y: f32) -> bool {
        let area = self.base_interaction_area();
        match self.viewer_content_mut() {
            Some(viewer::Content::Base(state)) => state.pointer_down(area, x, y),
            _ => false,
        }
    }
    pub(super) fn on_base_pointer(&mut self, x: f32, y: f32) -> bool {
        if self.state.view != WorkspaceView::Editor
            || self.dialog.is_some()
            || self.menu.is_some()
            || self.search.is_some()
            || self.command.is_some()
        {
            return false;
        }
        let area = self.base_interaction_area();
        match self.viewer_content_mut() {
            Some(viewer::Content::Base(state)) => state.pointer(area, x, y),
            _ => false,
        }
    }
    pub(super) fn reveal_base(&mut self, location: mochi_core::base::BaseLocation) {
        if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
            if let Err(error) = state.reveal(location) {
                state.error = error;
            }
        }
    }
    pub(super) fn open_base_schedule_reference(
        &mut self,
        _kind: ResourceKind,
        id: &str,
        path: &Path,
    ) {
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            return;
        };
        let norm = |p: &Path| {
            p.to_string_lossy()
                .replace('\\', "/")
                .trim_end_matches('/')
                .to_lowercase()
        };
        let target = norm(path);
        let owned = [
            mochi_core::paths::SCHEDULE_DIR_NAME,
            mochi_core::agenda::store::DIR_NAME,
        ]
        .iter()
        .any(|dir| norm(&root.join(dir)) == target);
        if !owned {
            self.state.status_text = "此日程链接不属于当前工作区".into();
            return;
        }
        self.agenda_open(id);
    }
}
