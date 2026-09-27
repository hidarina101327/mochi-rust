//! 把对话框操作分派到对应的应用处理逻辑。
use super::*;

impl App {
    pub(super) fn run_dialog_action(&mut self, action: DialogAction) {
        match action {
            DialogAction::DesktopFiles {
                card,
                page,
                command,
                paths,
                move_files,
            } => self.desktop_folder_confirm(&card, &page, command, paths, move_files),
            DialogAction::ImportPackage {
                workspace,
                package,
                remaining,
            } => {
                self.confirm_package_import(workspace, *package, remaining);
            }
            DialogAction::RememberFolderDrop => {
                if let Some(dialog) = &mut self.dialog {
                    for button in &mut dialog.buttons {
                        if let ButtonKind::Checkbox(checked) = &mut button.kind {
                            *checked = !*checked;
                        }
                    }
                }
            }
            DialogAction::ImportDropped {
                paths,
                target,
                mapping,
            } => {
                let remember = self.dialog.as_ref().is_some_and(|dialog| {
                    dialog
                        .buttons
                        .iter()
                        .any(|b| b.kind == ButtonKind::Checkbox(true))
                });
                if remember {
                    if let Some(descriptor) = app_settings::descriptor("sidebar.folderDropAction") {
                        let choice = if mapping {
                            "创建映射文件夹"
                        } else {
                            "粘贴文件夹"
                        };
                        self.app_settings
                            .write(descriptor, &SettingValue::Text(choice.into()));
                        self.load_chrome_settings();
                    }
                }
                self.close_dialog();
                self.start_external_import(paths, target, mapping);
            }
            DialogAction::WorkflowApprove {
                id,
                revision,
                enable,
                run,
            } => self.workflow_approve(&id, revision, enable, run),
            DialogAction::WorkflowDelete => self.workflow_delete(),
            DialogAction::WorkflowDiscardEditor => {
                let v = &mut self.workflows.view;
                v.field.set_text(&v.field_before.clone());
                v.editor = None;
                v.error.clear();
                self.close_dialog();
            }
            DialogAction::WorkflowDiscard => {
                if let Some(store) = &self.workflows.store {
                    let _ = store.save_editor_draft(None);
                }
                self.close_dialog();
                self.workflows.view = crate::ui::workflows::State::default();
                self.workflows_refresh();
            }
            DialogAction::QuickNavAddItem => {
                let target = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                if target.is_empty() {
                    if let Some(dialog) = self.dialog.as_mut() {
                        dialog.error = "请输入网址或本地路径".into();
                    }
                    return;
                }
                let result = self.shell.workspace().map(|ws| {
                    let service = mochi_core::quick_navigation::Service::new(&ws.root);
                    let mut model = service.load()?;
                    let selected_group = model
                        .groups
                        .iter()
                        .any(|group| group.id == model.ui.selected)
                        .then(|| model.ui.selected.clone());
                    let kind = if target.starts_with("http://") || target.starts_with("https://") {
                        "web"
                    } else if std::path::Path::new(&target).is_dir() {
                        "folder"
                    } else {
                        "file"
                    };
                    service.upsert_item(
                        &mut model,
                        mochi_core::quick_navigation::Item {
                            id: String::new(),
                            name: String::new(),
                            target,
                            kind: kind.into(),
                            group_id: selected_group,
                            note: String::new(),
                            arguments: Vec::new(),
                            working_directory: String::new(),
                            source_path: String::new(),
                            favorite: false,
                            order: 0,
                            open_count: 0,
                            last_opened_at: None,
                            created_at: String::new(),
                            updated_at: String::new(),
                            background_color: None,
                            custom_icon: None,
                        },
                    )
                });
                match result {
                    Some(Ok(_)) => {
                        self.close_dialog();
                        self.show_global_notice("快捷方式已添加");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::QuickNavAddGroup => {
                let name = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let parent = self.quick_nav_group_parent.clone();
                let result = self.shell.workspace().map(|ws| {
                    let service = mochi_core::quick_navigation::Service::new(&ws.root);
                    let mut model = service.load()?;
                    service.add_group(&mut model, &name, parent.as_deref())
                });
                match result {
                    Some(Ok(_)) => {
                        self.quick_nav_group_parent = None;
                        self.close_dialog();
                        self.show_global_notice("快捷导航分组已创建");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::QuickNavEditItem(id) => {
                let input = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let mut parts = input.splitn(4, '|').map(str::trim);
                let name = parts.next().unwrap_or_default();
                let target = parts.next().unwrap_or_default();
                let note = parts.next().unwrap_or_default();
                let background = parts.next().unwrap_or_default();
                if target.is_empty() {
                    if let Some(dialog) = self.dialog.as_mut() {
                        dialog.error = "请按“名称 | 位置 | 备注 | 颜色”填写，位置不能为空".into();
                    }
                    return;
                }
                if !background.is_empty() && quick_nav_color(background).is_none() {
                    if let Some(dialog) = self.dialog.as_mut() {
                        dialog.error = "卡片颜色必须是 #RRGGBB，例如 #EAF4FF".into();
                    }
                    return;
                }
                let result = self.shell.workspace().map(|ws| {
                    let service = mochi_core::quick_navigation::Service::new(&ws.root);
                    let mut model = service.load()?;
                    let mut item = model
                        .items
                        .iter()
                        .find(|item| item.id == id)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("快捷方式不存在"))?;
                    item.name = name.to_owned();
                    item.target = target.to_owned();
                    item.note = note.to_owned();
                    item.background_color = (!background.is_empty()).then(|| background.to_owned());
                    item.kind = if target.starts_with("http://") || target.starts_with("https://") {
                        "web".into()
                    } else if std::path::Path::new(target).is_dir() {
                        "folder".into()
                    } else {
                        "file".into()
                    };
                    service.upsert_item(&mut model, item)
                });
                match result {
                    Some(Ok(_)) => {
                        self.close_dialog();
                        self.show_global_notice("快捷方式已更新");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::QuickNavRenameGroup(id) => {
                let name = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let result = self.shell.workspace().map(|ws| {
                    let service = mochi_core::quick_navigation::Service::new(&ws.root);
                    let mut model = service.load()?;
                    service.rename_group(&mut model, &id, &name)
                });
                match result {
                    Some(Ok(())) => {
                        self.close_dialog();
                        self.show_global_notice("分组已重命名");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::QuickNavDeleteGroup(id) => {
                let result = self.shell.workspace().map(|ws| {
                    let service = mochi_core::quick_navigation::Service::new(&ws.root);
                    let mut model = service.load()?;
                    service.delete_group(&mut model, &id)
                });
                match result {
                    Some(Ok(())) => {
                        self.close_dialog();
                        self.show_global_notice("分组已删除，快捷方式已移至未分组");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::QuickNavSetBrowser => {
                let input = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let (mode, path) = input
                    .split_once('|')
                    .map(|(mode, path)| (mode.trim(), path.trim()))
                    .unwrap_or(("system", ""));
                if mode != "system" && mode != "custom" {
                    if let Some(dialog) = self.dialog.as_mut() {
                        dialog.error = "打开方式只能是 system 或 custom".into();
                    }
                    return;
                }
                if mode == "custom" && path.is_empty() {
                    if let Some(dialog) = self.dialog.as_mut() {
                        dialog.error = "custom 模式需要填写浏览器 exe 路径".into();
                    }
                    return;
                }
                let result = self.shell.workspace().map(|ws| {
                    let service = mochi_core::quick_navigation::Service::new(&ws.root);
                    let mut model = service.load()?;
                    model.settings.browser.mode = mode.to_owned();
                    model.settings.browser.executable_path = path.to_owned();
                    service.save(&model)
                });
                match result {
                    Some(Ok(())) => {
                        self.close_dialog();
                        self.show_global_notice("网页浏览器设置已保存");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::EnglishAddWord => {
                let input = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let (word, meaning) = input
                    .split_once('|')
                    .map(|(word, meaning)| (word.trim(), meaning.trim()))
                    .unwrap_or((input.trim(), ""));
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::english_lab::Service::new(&ws.root).add_word(word, meaning)
                });
                match result {
                    Some(Ok(_)) => {
                        self.close_dialog();
                        self.show_global_notice("词条已加入 English Lab");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::EnglishAddArticle => {
                let input = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let mut parts = input.splitn(3, '|').map(str::trim);
                let title = parts.next().unwrap_or_default();
                let content = parts.next().unwrap_or_default();
                let translation = parts.next().unwrap_or_default();
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::english_lab::Service::new(&ws.root).add_article(
                        title,
                        content,
                        translation,
                        3,
                        "user",
                    )
                });
                match result {
                    Some(Ok(_)) => {
                        self.close_dialog();
                        self.show_global_notice("阅读文章已添加");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::EnglishAddSentence => {
                let input = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let (content, translation) = input
                    .split_once('|')
                    .map(|(content, translation)| (content.trim(), translation.trim()))
                    .unwrap_or((input.trim(), ""));
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::english_lab::Service::new(&ws.root).add_sentence(
                        content,
                        translation,
                        3,
                        "user",
                    )
                });
                match result {
                    Some(Ok(_)) => {
                        self.close_dialog();
                        self.show_global_notice("听力句子已添加");
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::EnglishRemoveDictionary(book_id) => {
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::english_lab::Service::new(&ws.root).remove_dictionary(&book_id)
                });
                match result {
                    Some(Ok(plan)) => {
                        self.close_dialog();
                        self.show_global_notice(format!(
                            "已移除《{}》，保留 {} 个已有学习记录的词",
                            plan.name, plan.retained_words
                        ));
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::EnglishGradeDictation { sentence_id } => {
                let answer = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().trim().to_owned())
                    .unwrap_or_default();
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::english_lab::Service::new(&ws.root).grade_dictation(
                        sentence_id,
                        &answer,
                        0,
                    )
                });
                match result {
                    Some(Ok(outcome)) => {
                        self.close_dialog();
                        let label = if outcome.exact {
                            "听写完全正确"
                        } else if outcome.score >= 0.6 {
                            "大部分听出来了"
                        } else {
                            "建议慢速重听"
                        };
                        self.show_global_notice(format!(
                            "{label} · {:.0}% · 原句已保存到训练历史",
                            outcome.score * 100.0,
                        ));
                        self.invalidate_main();
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::CreateTemplate(content) => {
                let name = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().to_owned())
                    .unwrap_or_default();
                let group = if self.views.templates.selected_group.is_empty() {
                    mochi_core::templates::UNGROUPED.to_owned()
                } else {
                    self.views.templates.selected_group.clone()
                };
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::templates::TemplateService::new(&ws.root)
                        .create_in_group(&name, &content, &group)
                });
                match result {
                    Some(Ok(template)) => {
                        self.close_dialog();
                        self.reload_templates();
                        if self.open_file_from_ui(&template.path) {
                            self.state.view = WorkspaceView::Editor;
                            self.sync_state();
                        }
                        self.show_global_notice("模板已保存，可直接编辑模板内容");
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::CreateTemplateGroup => {
                let name = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().to_owned())
                    .unwrap_or_default();
                let result = self.shell.workspace().map(|ws| {
                    mochi_core::templates::TemplateService::new(&ws.root).create_group(&name)
                });
                match result {
                    Some(Ok(())) => {
                        self.close_dialog();
                        self.reload_templates();
                        self.views.templates.selected_group = name;
                        self.show_global_notice("模板分组已创建");
                    }
                    Some(Err(error)) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                    None => {}
                }
            }
            DialogAction::CreateBase(parent) => self.create_base_from_dialog(&parent),
            DialogAction::AutomationText(edit) => self.submit_automation_text(edit),
            DialogAction::BaseEdit {
                path,
                edit,
                original,
            } => self.submit_base_dialog(&path, edit, &original),
            DialogAction::DeleteAiRound => self.ai_confirm_delete_round(),
            DialogAction::ConflictReload(path) => {
                self.ai_cancel();
                self.close_dialog();
                if path.is_file() {
                    self.shell.reload_file(&path);
                } else {
                    self.shell.forget_tabs_under(&path);
                }
                self.invalidate_main();
                self.sync_state();
            }
            DialogAction::ConflictOverwrite(path) => {
                self.ai_cancel();
                if self.shell.force_save_file(&path) {
                    self.close_dialog();
                    self.invalidate_main();
                    self.sync_state();
                } else {
                    self.state.status_text = self.shell.status().into();
                }
            }
            DialogAction::ConflictCopy(path) => self.conflict_copy(&path),
            DialogAction::SetShortcut(default) => {
                let value = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().trim())
                    .filter(|v| !v.is_empty())
                    .unwrap_or(default)
                    .to_owned();
                let result = crate::ui::shortcuts::prepare(default, &value).and_then(|json| {
                    if default == "Ctrl+Shift+Space" {
                        let old = crate::ui::shortcuts::Chord::parse(
                            &crate::ui::shortcuts::binding(default),
                        )?;
                        let next = crate::ui::shortcuts::Chord::parse(&value)?;
                        if let Err(e) = self.register_capture_key(next) {
                            let _ = self.register_capture_key(old);
                            return Err(format!("无法注册此快捷键：{e}"));
                        }
                    }
                    self.settings.set("keyboard.shortcuts", &json);
                    self.settings.flush().map_err(|e| e.to_string())?;
                    crate::ui::shortcuts::load(Some(&json));
                    Ok(())
                });
                match result {
                    Ok(()) => self.close_dialog(),
                    Err(error) => {
                        if let Some(d) = self.dialog.as_mut() {
                            d.error = error;
                        }
                    }
                }
            }
            DialogAction::EnableAutomaticAiEdits => {
                if let Some(d) = app_settings::descriptor("ai.editApplyMode") {
                    self.app_settings
                        .write(d, &SettingValue::Text("auto".into()));
                    let _ = self.app_settings.flush();
                    self.apply_setting_side_effects("ai.editApplyMode");
                }
                self.close_dialog();
            }
            DialogAction::FileIcon(path) => {
                let value = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().trim().to_owned())
                    .unwrap_or_default();
                if !value.is_empty()
                    && crate::ui::icons::named(&value).is_none()
                    && value.chars().count() > 2
                {
                    if let Some(d) = self.dialog.as_mut() {
                        d.error = "请输入有效图标名称或单个符号".into();
                    }
                    return;
                }
                self.save_file_icon(path, (!value.is_empty()).then_some(value));
                self.close_dialog();
            }
            DialogAction::CreateSubdocument(parent) => {
                let value = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text())
                    .unwrap_or("");
                let name = match sidebar::finalize_name(value, false) {
                    Ok(n) => n,
                    Err(e) => {
                        if let Some(d) = self.dialog.as_mut() {
                            d.error = e.into();
                        }
                        return;
                    }
                };
                if let Some(ws) = self.shell.workspace() {
                    match mochi_core::sub_documents::create(
                        &ws.root,
                        &parent.to_string_lossy(),
                        &name,
                        "",
                    ) {
                        Ok(path) => {
                            self.close_dialog();
                            self.shell.refresh_tree();
                            self.shell.expand(&parent);
                            self.open_file_from_ui(Path::new(&path));
                            self.invalidate_main();
                            self.sync_state();
                        }
                        Err(e) => {
                            if let Some(d) = self.dialog.as_mut() {
                                d.error = e.to_string();
                            }
                        }
                    }
                }
            }
            DialogAction::ExamText {
                path,
                block,
                question,
                note,
            } => {
                let text = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().to_owned())
                    .unwrap_or_default();
                if let Some(viewer::Content::Exam(s)) = self.viewer_content_for(&path) {
                    let id = question
                        .and_then(|q| s.question(block, q))
                        .and_then(|q| q["id"].as_str())
                        .unwrap_or("global")
                        .to_owned();
                    if note {
                        if let Some(notes) = s.notes.get_mut(block) {
                            notes.insert(id, text.into());
                        }
                    } else if !s.locked(block) {
                        if let Some(answers) = s.answers.get_mut(block) {
                            answers.insert(id, text.into());
                        }
                    }
                }
                self.close_dialog();
                self.remember_exam_drafts();
            }
            DialogAction::DeleteProvider(id) => {
                self.cancel_provider_test();
                self.cancel_provider_models();
                match mochi_core::ai::providers::remove(&self.settings, &id) {
                    Ok(()) => {
                        self.close_dialog();
                        self.reload_providers();
                    }
                    Err(e) => {
                        if let Some(d) = self.dialog.as_mut() {
                            d.error = e.to_string();
                        }
                    }
                }
            }
            DialogAction::DeleteAiSession(id) => {
                if !self.ai.panel.is_streaming() {
                    self.ai_delete_session(&id);
                }
                self.close_dialog();
            }
            DialogAction::RenameAiSession(id) => self.ai_submit_session_name(&id, false),
            DialogAction::NewAiSessionProject(id) => self.ai_submit_session_name(&id, true),
            DialogAction::ClearAiSession(id) => {
                if self.ai.panel.active.as_ref().is_some_and(|c| c.id == id) {
                    self.ai_clear_session();
                }
                self.close_dialog();
            }
            DialogAction::ReplaceAiDraft { session_id, text } => {
                if self
                    .ai
                    .panel
                    .active
                    .as_ref()
                    .is_some_and(|c| c.id == session_id)
                {
                    self.ai.panel.input.set_text(&text);
                    self.focus = Focus::AiInput;
                }
                self.close_dialog();
            }
            DialogAction::PdfPage => {
                let page = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .and_then(|f| f.text().trim().parse::<usize>().ok());
                let body = self.viewer_layout.body;
                if let (Some(page), Some(viewer::Content::Pdf(s))) =
                    (page, self.viewer_content_mut())
                {
                    if page > 0 && page <= s.page_count() {
                        let rects = viewer::pdf_page_rects(body, s);
                        s.scroll = (rects[page - 1].top - body.top - viewer::PDF_LABEL_H).max(0.0);
                        self.close_dialog();
                        self.remember_pdf_page();
                        return;
                    }
                }
                if let Some(d) = self.dialog.as_mut() {
                    d.error = "请输入有效页码".into();
                }
            }
            DialogAction::CodeTitle(start) => {
                let title = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().to_owned())
                    .unwrap_or_default();
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    self.doc.set_code_title(buffer, start, title);
                    self.after_doc_edit(false);
                }
                self.close_dialog();
                self.editor_engaged = false;
            }
            DialogAction::ContainerTitle {
                path,
                start,
                original,
            } => self.finish_container_title(&path, start, &original),
            DialogAction::PickEditorImage => {
                if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                    self.close_dialog();
                    self.import_editor_image(&path);
                }
            }
            DialogAction::ImageWidth {
                path,
                start,
                original,
            } => self.finish_image_width(&path, start, &original),
            DialogAction::InsertEditorLink {
                image,
                path,
                range,
                original,
                label,
            } => {
                let value = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().trim().to_owned())
                    .unwrap_or_default();
                if value.is_empty() {
                    if let Some(d) = self.dialog.as_mut() {
                        d.error = "请输入链接地址".into();
                    }
                    return;
                }
                if self.active_file_path().as_ref() != Some(&path) {
                    return;
                }
                let Some(buffer) = self.shell.active_buffer_mut() else {
                    return;
                };
                if buffer.text().get(range.0..range.1) != Some(original.as_str()) {
                    if let Some(d) = self.dialog.as_mut() {
                        d.error = "选区内容已变化，请重新插入".into();
                    }
                    return;
                }
                let label = if label.is_empty() {
                    if image {
                        String::new()
                    } else {
                        value.clone()
                    }
                } else {
                    label
                };
                let label = label
                    .replace('[', "\\[")
                    .replace(']', "\\]")
                    .replace('\n', " ");
                let url = value
                    .replace(' ', "%20")
                    .replace('(', "%28")
                    .replace(')', "%29");
                let mut markdown = format!("{}[{label}]({url})", if image { "!" } else { "" });
                if image {
                    if range.0 > 0 && !buffer.text()[..range.0].ends_with('\n') {
                        markdown.insert_str(0, "\n\n");
                    }
                    if range.1 < buffer.text().len() && !buffer.text()[range.1..].starts_with('\n')
                    {
                        markdown.push_str("\n\n");
                    }
                }
                buffer.replace_range(range.0..range.1, &markdown);
                self.close_dialog();
                self.focus = Focus::Main;
                self.editor_engaged = true;
                self.after_doc_edit(true);
            }
            DialogAction::EditEditorMath {
                path,
                range,
                original,
                display,
            } => {
                let value = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().trim().to_owned())
                    .unwrap_or_default();
                if self.active_file_path().as_ref() != Some(&path) {
                    return;
                }
                let Some(buffer) = self.shell.active_buffer_mut() else {
                    return;
                };
                if buffer.text().get(range.0..range.1) != Some(original.as_str()) {
                    return;
                }
                let markdown = if display {
                    let newline = if buffer.text().contains("\r\n") {
                        "\r\n"
                    } else {
                        "\n"
                    };
                    let value = value.replace("\r\n", "\n").replace('\n', newline);
                    format!("$${newline}{value}{newline}$$")
                } else {
                    format!("${}$", value.replace(['\r', '\n'], " "))
                };
                buffer.replace_range(range.0..range.1, &markdown);
                self.close_dialog();
                self.focus = Focus::Main;
                self.editor_engaged = true;
                self.after_doc_edit(true);
            }
            DialogAction::PdfText => {
                let text = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.field.as_ref())
                    .map(|f| f.text().trim().to_owned())
                    .unwrap_or_default();
                if text.is_empty() {
                    if let Some(d) = self.dialog.as_mut() {
                        d.error = "标注文字不能为空".into();
                    }
                    return;
                }
                if let Some((path, mut a)) = self.pdf_text_pending.clone() {
                    a.text = Some(text);
                    a.updated_at = Self::now_ms();
                    let mut items =
                        sidecars::load_pdf_annotations(&path.to_string_lossy()).annotations;
                    if let Some(existing) = items.iter_mut().find(|v| v.id == a.id) {
                        *existing = a.clone();
                    } else {
                        items.push(a.clone());
                    }
                    if self.pdf_save_annotations(&path, items) {
                        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(&path) {
                            s.annotations.selected = Some(a.id);
                        }
                        self.close_dialog();
                    } else if let Some(d) = self.dialog.as_mut() {
                        d.error = self.state.status_text.clone();
                    }
                }
            }
            DialogAction::DownloadAppUpdate => self.start_update_download(),
            DialogAction::SkipAppUpdate => {
                if let Some(update) = self.update_available.take() {
                    self.settings
                        .set("updates.skippedVersion", &update.latest_version);
                    if let Err(error) = self.settings.flush() {
                        self.show_global_notice(format!("保存跳过版本失败：{error}"));
                    } else {
                        self.show_global_notice(format!(
                            "已忽略 Rust 原生版 v{}，后续版本仍会提醒",
                            update.latest_version
                        ));
                    }
                }
                self.close_dialog();
            }
            DialogAction::DismissAppUpdate => {
                self.update_available = None;
                self.close_dialog();
            }
            DialogAction::Dismiss => self.close_dialog(),
            DialogAction::RenameFile(path) => {
                let raw = self
                    .dialog
                    .as_ref()
                    .and_then(|dialog| dialog.field.as_ref())
                    .map(|field| field.text().to_owned())
                    .unwrap_or_default();
                let extension = path
                    .extension()
                    .map(|extension| format!(".{}", extension.to_string_lossy()))
                    .unwrap_or_default();
                let name = match sidebar::finalize_name_with_extension(&raw, false, &extension) {
                    Ok(name) => name,
                    Err(error) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.into();
                        }
                        return;
                    }
                };
                match self.shell.rename(&path, &name) {
                    Ok(to) => {
                        if let Some(rest) = self
                            .split
                            .other
                            .as_deref()
                            .and_then(|other| other.strip_prefix(&path).ok())
                        {
                            self.split.other = Some(to.join(rest));
                        }
                        self.remember_split_active();
                        self.close_dialog();
                        self.invalidate_main();
                        self.sync_state();
                    }
                    Err(error) => {
                        if let Some(dialog) = self.dialog.as_mut() {
                            dialog.error = error.to_string();
                        }
                    }
                }
            }
            DialogAction::RestoreBlockDocument(path, expected) => {
                self.restore_block_document(&path, &expected)
            }
            DialogAction::CreateInDialog { parent, folder } => {
                let Some(dialog) = self.dialog.as_mut() else {
                    return;
                };
                let raw = dialog
                    .field
                    .as_ref()
                    .map(|f| f.text().to_owned())
                    .unwrap_or_default();
                match sidebar::finalize_name(&raw, folder) {
                    Err(e) => {
                        // 对话框版的文案是「文件名不能为空」
                        dialog.error =
                            e.replace("名称", if folder { "文件夹名" } else { "文件名" });
                    }
                    Ok(name) => {
                        let result = if folder {
                            self.shell.create_folder(&parent, &name).map(|_| None)
                        } else {
                            self.shell.create_file(&parent, &name).map(Some)
                        };
                        match result {
                            Ok(created) => {
                                self.close_dialog();
                                // TSX 的 handleNewFile 建完就打开它
                                if let Some(path) = created {
                                    if self.open_file_from_ui(&path) {
                                        self.invalidate_main();
                                    }
                                }
                                self.sync_state();
                            }
                            Err(e) => {
                                if let Some(d) = self.dialog.as_mut() {
                                    d.error = e.to_string();
                                }
                            }
                        }
                    }
                }
            }
            DialogAction::DeleteToTrash(path) => {
                if !self.prepare_file_deletion(&path) {
                    return;
                }
                self.close_dialog();
                let result = platform::move_to_trash(HWND(self.hwnd_raw as *mut _), &path);
                if result.is_ok() {
                    self.shell.after_external_delete(&path);
                }
                self.invalidate_main();
                self.sync_state();
                self.state.status_text = result
                    .map(|_| "系统删除操作已完成".into())
                    .unwrap_or_else(|e| format!("移到回收站失败或已取消：{e}"));
            }
            DialogAction::DeletePermanently(path) => {
                if !self.prepare_file_deletion(&path) {
                    return;
                }
                self.close_dialog();
                let result = self.shell.delete(&path);
                self.invalidate_main();
                self.sync_state();
                self.state.status_text = result
                    .map(|_| "已永久删除".into())
                    .unwrap_or_else(|e| format!("删除失败：{e}"));
            }
            DialogAction::RestoreVersion { path, oid } => {
                self.close_dialog();
                let Some(git) = self.shell.git() else { return };
                match git.restore_file(&path.to_string_lossy(), &oid, "overwrite") {
                    Ok(_) => {
                        // 打开着的标签要重新读盘，否则编辑器里还是旧内容
                        self.shell.reload_file(&path);
                    }
                    Err(e) => self.panels.version.error = e.to_string(),
                }
                self.load_version_history(Some(path));
                self.invalidate_main();
            }
            DialogAction::ReviewScheduleDiff { raw, approve } => {
                self.apply_schedule_review(raw, approve);
            }
            DialogAction::ReviewConsoleAction { raw, approve } => {
                self.apply_console_review(raw, approve)
            }
        }
    }
}
