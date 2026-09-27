//! 处理重命名、删除、创建文档和选择模板等对话框。
use super::*;

impl App {
    pub(super) fn open_rename_dialog(&mut self, path: PathBuf) {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut field = TextField::new("文档名称");
        field.set_text(&name);
        self.dialog = Some(Dialog {
            title: "重命名文档".into(),
            description: "扩展名将保持不变".into(),
            field: Some(field),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "重命名".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::RenameFile(path),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_delete_dialog(&mut self, path: PathBuf) {
        let is_dir = path.is_dir();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.dialog = Some(Dialog {
            title: format!("删除{}", if is_dir { "文件夹" } else { "文件" }),
            description: format!("是否将 “{name}” 移动到系统回收站？"),
            field: None,
            error: String::new(),
            note: Some(
                format!(
                    "移动到系统回收站后仍可恢复。{}",
                    if is_dir {
                        " 文件夹中的所有内容会一并移动。"
                    } else {
                        ""
                    }
                )
                .into(),
            ),
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "永久删除".into(),
                    kind: ButtonKind::Danger,
                    action: DialogAction::DeletePermanently(path.clone()),
                },
                DialogButton {
                    label: "移到系统回收站".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::DeleteToTrash(path),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn save_file_icon(&mut self, path: PathBuf, value: Option<String>) {
        let key = format!("file.icon:{}", path.to_string_lossy());
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            self.file_icons.insert(path, value.clone());
            self.settings.set(&key, &value);
        } else {
            self.file_icons.remove(&path);
            self.settings.remove(&key);
        }
        let _ = self.settings.flush();
    }

    /// 立即创建默认名称的文档；同名时使用文件服务的编号规则让路。
    pub(super) fn create_default_document(&mut self, parent: PathBuf) {
        let desired = format!("未命名文档{}", sidebar::default_document_extension());
        let result = mochi_core::files::get_unique_file_path(&parent, &desired).and_then(|path| {
            let name = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("无法生成默认文档名称"))?
                .to_string_lossy()
                .into_owned();
            self.shell.create_file(&parent, &name)
        });
        match result {
            Ok(path) => {
                if self.open_file_from_ui(&path) {
                    self.invalidate_main();
                }
                self.sync_state();
            }
            Err(error) => self.show_global_notice(format!("创建文档失败：{error}")),
        }
    }

    pub(super) fn open_template_picker(&mut self, parent: PathBuf) {
        let templates = self
            .shell
            .workspace()
            .and_then(|ws| {
                let service = mochi_core::templates::TemplateService::new(&ws.root);
                service.ensure_builtins().and_then(|_| service.list()).ok()
            })
            .unwrap_or_default();
        self.template_picker = Some(crate::ui::template_picker::Picker::new(parent, templates));
        self.focus = Focus::Dialog;
    }

    pub(super) fn on_template_picker_click(&mut self, x: f32, y: f32) {
        let Some(picker) = self.template_picker.as_ref() else {
            return;
        };
        let hit = picker.hit(self.renderer.viewport(), x, y);
        self.activate_template_picker(hit);
    }

    pub(super) fn activate_template_picker(&mut self, hit: crate::ui::template_picker::Hit) {
        let Some(picker) = self.template_picker.as_ref() else {
            return;
        };
        match hit {
            crate::ui::template_picker::Hit::Blank => {
                let parent = picker.parent.clone();
                self.template_picker = None;
                self.focus = Focus::Main;
                self.create_default_document(parent);
            }
            crate::ui::template_picker::Hit::Template(index) => {
                let parent = picker.parent.clone();
                let template = picker.templates.get(index).cloned();
                self.template_picker = None;
                self.focus = Focus::Main;
                if let Some(template) = template {
                    self.create_document_from_template(parent, template);
                }
            }
            crate::ui::template_picker::Hit::Cancel | crate::ui::template_picker::Hit::Outside => {
                self.template_picker = None;
                self.focus = Focus::Main;
            }
            crate::ui::template_picker::Hit::Inside => {}
        }
    }

    pub(super) fn create_document_from_template(
        &mut self,
        parent: PathBuf,
        template: mochi_core::templates::Template,
    ) {
        let result = self
            .shell
            .workspace()
            .ok_or_else(|| anyhow::anyhow!("请先打开一个工作区"))
            .and_then(|ws| {
                mochi_core::templates::TemplateService::new(&ws.root)
                    .instantiate(&template, &parent)
            });
        match result {
            Ok(path) => {
                self.shell.refresh_tree();
                if self.open_file_from_ui(&path) {
                    self.state.view = WorkspaceView::Editor;
                    self.invalidate_main();
                }
                self.sync_state();
                self.show_global_notice(format!(
                    "已从模板创建：{}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
            Err(error) => self.show_global_notice(format!("创建文档失败：{error}")),
        }
    }

    /// 创建一个空的空间容器；后续添加的是对象引用，而不是快照副本。
    pub(super) fn create_default_canvas(&mut self, parent: PathBuf) {
        let desired = format!("未命名画布.{}", mochi_core::canvas::EXTENSION);
        let result = mochi_core::files::get_unique_file_path(&parent, &desired).and_then(|path| {
            let name = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("无法生成默认画布名称"))?
                .to_string_lossy()
                .into_owned();
            self.shell.create_canvas(&parent, &name)
        });
        match result {
            Ok(path) => {
                if self.open_file_from_ui(&path) {
                    self.invalidate_main();
                }
                self.sync_state();
            }
            Err(error) => self.show_global_notice(format!("创建画布失败：{error}")),
        }
    }

    pub(super) fn open_create_dialog(&mut self, parent: PathBuf, folder: bool) {
        let dir_name = parent
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut field = TextField::new(if folder {
            "文件夹名".to_owned()
        } else {
            format!(
                "文件名（自动添加 {}）",
                sidebar::default_document_extension()
            )
        });
        field.style = TextStyle::Label;
        self.dialog = Some(Dialog {
            title: if folder {
                "新建文件夹"
            } else {
                "新建文件"
            }
            .into(),
            description: format!(
                "在 \"{dir_name}\" 中创建新{}",
                if folder { "文件夹" } else { "文件" }
            ),
            field: Some(field),
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
                    action: DialogAction::CreateInDialog { parent, folder },
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn on_dialog_click(&mut self, x: f32, y: f32) {
        let viewport = self.renderer.viewport();
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        if let Some(url) = dialog.note_link_at(viewport, x, y) {
            platform::open_external(&url);
            return;
        }
        match dialog.hit(viewport, x, y) {
            DialogHit::Close | DialogHit::Outside => {
                let action = dialog.dismiss.clone();
                self.run_dialog_action(action);
            }
            DialogHit::Button(i) => {
                let action = dialog.buttons[i].action.clone();
                self.run_dialog_action(action);
            }
            DialogHit::Field => {
                if dialog.is_math_editor() {
                    let rect = dialog.field_rect(viewport).unwrap();
                    dialog
                        .field
                        .as_mut()
                        .unwrap()
                        .multiline_select(rect, x, y, shift_down());
                    self.drag = Some(Drag {
                        target: DragTarget::DialogFieldSelect,
                        grab_offset: 0.0,
                    });
                    return;
                }
                let left = dialog.field_text_left(viewport);
                if let (Some(f), Some(left)) = (dialog.field.as_mut(), left) {
                    f.click(x - left, false);
                }
            }
            DialogHit::Inside => {}
        }
    }

    pub(super) fn on_link_create_click(&mut self, x: f32, y: f32) {
        let viewport = self.renderer.viewport();
        let Some(dialog) = self.link_create.as_mut() else {
            return;
        };
        let hit = dialog.hit(viewport, x, y);
        match hit {
            link_create::Hit::Close | link_create::Hit::Outside | link_create::Hit::Cancel => {
                self.close_link_create()
            }
            link_create::Hit::Create => self.create_link_from_dialog(),
            link_create::Hit::Name | link_create::Hit::Url => {
                let field = if matches!(hit, link_create::Hit::Name) {
                    link_create::Field::Name
                } else {
                    link_create::Field::Url
                };
                dialog.active = field;
                let left = dialog.field_text_left(viewport, field);
                match field {
                    link_create::Field::Name => dialog.name.click(x - left, false),
                    link_create::Field::Url => dialog.url.click(x - left, false),
                }
            }
            link_create::Hit::Inside => {}
        }
    }

    pub(super) fn close_link_create(&mut self) {
        self.link_create = None;
        self.focus = Focus::Main;
    }

    pub(super) fn create_link_from_dialog(&mut self) {
        let Some(dialog) = self.link_create.as_mut() else {
            return;
        };
        let name = dialog.name.text().trim().to_owned();
        let url = dialog.url.text().trim().to_owned();
        if name.is_empty() {
            dialog.error = "请输入显示名称".into();
            dialog.active = link_create::Field::Name;
            return;
        }
        if !url.starts_with("https://") && !url.starts_with("http://") {
            dialog.error = "请输入 HTTP/HTTPS URL".into();
            dialog.active = link_create::Field::Url;
            return;
        }
        let parent = dialog.parent.clone();
        self.file_jobs
            .submit(parent.clone(), self.hwnd_raw, move || {
                mochi_core::link_files::create(&parent, &url, Some(&name))
                    .map(crate::file_runtime::Payload::Created)
            });
        self.close_link_create();
    }

    pub(super) fn close_dialog(&mut self) {
        if self
            .drag
            .is_some_and(|drag| drag.target == DragTarget::DialogFieldSelect)
        {
            self.drag = None;
        }
        self.pending_ai_delete = None;
        self.dialog = None;
        self.pdf_text_pending = None;
        self.focus = Focus::Main;
    }
}
