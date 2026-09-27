//! 保存并将文档升级为 .mc 格式时，保留编辑器中的引用。
use super::*;
use mochi_core::ai::document_mounts::{self, AiDocumentMount};

fn remapped_path(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    if path == from {
        return Some(to.to_path_buf());
    }
    let old_side = PathBuf::from(mochi_core::sub_documents::sidecar_path(
        &from.to_string_lossy(),
    ));
    let new_side = PathBuf::from(mochi_core::sub_documents::sidecar_path(
        &to.to_string_lossy(),
    ));
    path.strip_prefix(old_side)
        .ok()
        .map(|rest| new_side.join(rest))
}

impl App {
    #[cfg(debug_assertions)]
    fn show_document_mode_setting(&mut self) -> anyhow::Result<Rect> {
        use anyhow::Context;
        self.open_settings("customization");
        if let Some(tab) = self.shell.active_mut() {
            if let TabKind::Settings { section, .. } = &mut tab.kind {
                *section = "editor-layout".into();
            }
        }
        self.prefs.scroll = 0.0;
        self.paint(HWND::default())?;
        let index = app_settings::descriptors()
            .iter()
            .position(|d| d.key == "editor.simpleDocumentMode")
            .context("document mode descriptor")?;
        let rect = self
            .prefs
            .content_layout
            .control_rect(index)
            .context("document mode control")?;
        self.prefs.scroll = (rect.top - self.prefs.content_layout.body.top - 200.0)
            .max(0.0)
            .min(self.prefs.content_layout.max_scroll());
        self.paint(HWND::default())?;
        self.prefs
            .content_layout
            .control_rect(index)
            .context("visible document mode control")
    }

    #[cfg(debug_assertions)]
    pub(super) fn verify_document_mode(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        use anyhow::ensure;
        let setting = app_settings::descriptor("editor.simpleDocumentMode").unwrap();
        ensure!(self.app_settings.read(setting) == SettingValue::Bool(true));
        let rect = self.show_document_mode_setting()?;
        self.verify_frame(output, "setting", snapshot)?;
        self.on_settings_content_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        ensure!(self.app_settings.read(setting) == SettingValue::Bool(false));
        self.open_create_dialog(folder.to_path_buf(), false);
        self.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("完整笔记");
        self.run_dialog_action(DialogAction::CreateInDialog {
            parent: folder.to_path_buf(),
            folder: false,
        });
        ensure!(folder.join("完整笔记.mc").is_file());
        let rect = self.show_document_mode_setting()?;
        self.on_settings_content_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        ensure!(self.app_settings.read(setting) == SettingValue::Bool(true));
        self.open_create_dialog(folder.to_path_buf(), false);
        self.dialog
            .as_mut()
            .unwrap()
            .field
            .as_mut()
            .unwrap()
            .set_text("简洁笔记");
        self.run_dialog_action(DialogAction::CreateInDialog {
            parent: folder.to_path_buf(),
            folder: false,
        });
        let plain = folder.join("简洁笔记.md");
        let rich = folder.join("简洁笔记.mc");
        ensure!(plain.is_file() && !rich.exists());
        let buffer = self.shell.active_buffer_mut().unwrap();
        buffer.set_cursor(buffer.text().len(), false);
        buffer.insert("普通段落、**加粗**和列表保持 Markdown。\n\n为这一段添加颜色。\n");
        ensure!(self.save_active());
        ensure!(
            plain.is_file() && !rich.exists(),
            "ordinary Markdown was upgraded"
        );
        self.paint(HWND::default())?;
        self.verify_frame(output, "markdown", snapshot)?;
        self.shell.toggle_favorite(&plain)?;
        self.split.other = Some(plain.clone());
        self.file_icons.insert(plain.clone(), "📄".into());
        self.ai.panel.pending_files.push(plain.clone());
        self.panels.version.source = Some(plain.clone());
        self.panels.version.message.set_text("保留本次版本说明");
        let root = self.shell.workspace().unwrap().root.clone();
        let mounts = AiDocumentMountService::new(&root);
        let mount = serde_json::json!({"id":"format-mount","scope":"session","sessionId":"format-session","documentPath":plain,"documentRelativePath":document_mounts::document_relative_path(&root.to_string_lossy(),&plain.to_string_lossy())});
        mounts.save(document_mounts::AiDocumentMountIndex {
            mounts: vec![AiDocumentMount::from_map(
                mount.as_object().unwrap().clone(),
            )],
            ..Default::default()
        })?;
        let buffer = self.shell.active_buffer_mut().unwrap();
        let start = buffer.text().find("为这一段").unwrap();
        buffer.set_cursor(start, false);
        buffer.set_cursor(start + "为这一段添加颜色。".len(), true);
        self.apply_text_color(false, 0x3c5a78);
        ensure!(
            self.save_active(),
            "rich format save failed: {}",
            self.shell.status()
        );
        ensure!(!plain.exists() && rich.is_file());
        ensure!(self.active_file_path().as_deref() == Some(rich.as_path()));
        ensure!(self.shell.is_favorite(&rich) && !self.shell.is_favorite(&plain));
        ensure!(self.split.other.as_deref() == Some(rich.as_path()));
        ensure!(self.panels.version.source.as_deref() == Some(rich.as_path()));
        ensure!(self.panels.version.message.text() == "保留本次版本说明");
        ensure!(
            self.file_icons.contains_key(&rich)
                && self.ai.panel.pending_files == vec![rich.clone()]
        );
        ensure!(mounts.mounts_for_document(&rich.to_string_lossy()).len() == 1);
        self.split.other = None;
        self.paint(HWND::default())?;
        self.verify_frame(output, "upgraded", snapshot)?;
        ensure!(std::fs::read_to_string(&rich)?.contains("color: #3C5A78"));
        self.renderer.save_snapshot(snapshot, output)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed":true,"realD2D":true,"isolatedWorkspace":true,"defaultMarkdown":true,
                "settingAppliesLive":true,"plainMarkdownPreserved":true,"richFormatUpgrades":true,
                "favoriteSplitIconAttachmentAndMountPreserved":true
            }))?,
        )?;
        Ok(())
    }

    pub(super) fn sync_document_format_changes(&mut self) -> bool {
        let changes = self.shell.take_document_format_changes();
        if changes.is_empty() {
            return false;
        }
        let mut settings_changed = false;
        for (from, to) in changes {
            if let Some(path) = self.split.other.as_mut() {
                if let Some(next) = remapped_path(path, &from, &to) {
                    *path = next;
                }
            }
            if let Some(edit) = self.title_editing.as_mut() {
                if let Some(next) = remapped_path(&edit.path, &from, &to) {
                    edit.path = next;
                }
            }
            if let Some(path) = self.panels.comments.source.as_mut() {
                if let Some(next) = remapped_path(path, &from, &to) {
                    *path = next;
                }
            }
            if let Some(path) = self.panels.version.source.as_mut() {
                if let Some(next) = remapped_path(path, &from, &to) {
                    *path = next;
                }
            }
            let icons = self
                .file_icons
                .iter()
                .filter_map(|(path, icon)| {
                    remapped_path(path, &from, &to).map(|next| (path.clone(), next, icon.clone()))
                })
                .collect::<Vec<_>>();
            for (old, next, icon) in icons {
                self.file_icons.remove(&old);
                self.settings
                    .remove(&format!("file.icon:{}", old.display()));
                self.settings
                    .set(&format!("file.icon:{}", next.display()), &icon);
                self.file_icons.insert(next, icon);
                settings_changed = true;
            }
            for path in &mut self.ai.panel.pending_files {
                if let Some(next) = remapped_path(path, &from, &to) {
                    *path = next;
                }
            }
            for value in &mut self.ai.panel.pending_selections {
                if let Some(next) = value
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|path| remapped_path(Path::new(path), &from, &to))
                {
                    value["path"] = next.to_string_lossy().replace('\\', "/").into();
                }
            }
            if let Some(workspace) = self.shell.workspace() {
                let service = AiDocumentMountService::new(&workspace.root);
                let mut index = service.load();
                let mut changed = false;
                for mount in &mut index.mounts {
                    if let Some(next) = remapped_path(Path::new(mount.document_path()), &from, &to)
                    {
                        let path = next.to_string_lossy().replace('\\', "/");
                        let mut fields = mount.fields().clone();
                        fields.insert(
                            "documentRelativePath".into(),
                            document_mounts::document_relative_path(
                                &workspace.root.to_string_lossy(),
                                &path,
                            )
                            .into(),
                        );
                        fields.insert("documentPath".into(), path.into());
                        *mount = AiDocumentMount::from_map(fields);
                        changed = true;
                    }
                }
                if changed {
                    if let Err(error) = service.save(index) {
                        self.ai.panel.error = format!("文档已升级，更新会话挂载失败：{error}");
                    }
                }
            }
        }
        self.remember_split_active();
        if settings_changed {
            let _ = self.settings.flush();
        }
        self.editor_ai.invalidate();
        self.refresh_right_panel();
        self.invalidate_main();
        true
    }
}
