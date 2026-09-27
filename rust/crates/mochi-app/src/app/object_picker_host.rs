//! 为对象选择器提供弹层、选中值和 AI 文档挂载数据。
use super::*;
use crate::ui::object_picker as picker;
use mochi_core::object_reference::{self as objects, ObjectCandidate, ObjectKind, ObjectReference};
use serde_json::Value;

mod cache;
pub(super) use cache::Cache;
#[cfg(test)]
mod cache_flow_tests;

#[derive(Clone)]
pub(super) enum Purpose {
    DesktopSources {
        card: String,
        page: String,
        module: mochi_core::desktop_cards::Module,
    },
    Workflow,
    Attach,
    Session(String),
    Message(String, String),
    Reference {
        path: PathBuf,
        table: String,
        record: String,
        field: String,
    },
    Insert,
    Canvas(PathBuf),
}
pub(super) struct DialogState {
    pub state: picker::State,
    pub purpose: Purpose,
    pub previous_focus: Focus,
    pub loading: bool,
    workspace: PathBuf,
    base: Option<(PathBuf, mochi_core::base::BaseDocument)>,
    files_only: bool,
}

fn overlay_base(
    candidates: &mut Vec<ObjectCandidate>,
    workspace: &Path,
    base: Option<&(PathBuf, mochi_core::base::BaseDocument)>,
) {
    if let Some((path, document)) = base {
        objects::replace_record_candidates(candidates, path, workspace, document);
    }
}

impl App {
    /// 读取用于解析具体块的源文本。打开的缓冲区优先于磁盘，这样附加块时
    /// 不会悄悄把旧的已保存内容发给助手。先看活动标签页，再看其它打开的
    /// 规范路径相同的标签页。
    fn block_source(&self, path: &Path, canonical: &Path) -> anyhow::Result<String> {
        if let Some(tab) = self.shell.active() {
            if tab
                .path()
                .and_then(|open| open.canonicalize().ok())
                .as_deref()
                == Some(canonical)
            {
                if let Some(buffer) = tab.buffer() {
                    return Ok(buffer.text().to_owned());
                }
            }
        }
        for tab in self.shell.tabs() {
            let Some(open) = tab.path() else { continue };
            if open.canonicalize().ok().as_deref() != Some(canonical) {
                continue;
            }
            if let Some(buffer) = tab.buffer() {
                return Ok(buffer.text().to_owned());
            }
        }
        std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("读取文档块所在文档失败: {error}"))
    }

    /// 为一个持久化块构建既有的选中上下文协议。上下文里带稳定的块 ID 与
    /// 源码区间，显示文本则尽量取自当前编辑器快照。
    fn block_selection_value(
        &self,
        reference: &ObjectReference,
        workspace: &Path,
    ) -> anyhow::Result<(PathBuf, Value)> {
        anyhow::ensure!(reference.kind == ObjectKind::Block, "不是文档块引用");
        let root = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        let path = reference
            .resolve_path(Some(workspace))
            .ok_or_else(|| anyhow::anyhow!("文档块引用缺少可解析路径"))?;
        let canonical = path
            .canonicalize()
            .map_err(|error| anyhow::anyhow!("文档块所在文档无法读取: {error}"))?;
        anyhow::ensure!(
            mochi_core::paths::path_is_within(&root, &canonical) && canonical.is_file(),
            "文档块引用超出当前工作区"
        );
        let source = self.block_source(&path, &canonical)?;
        let document = mochi_core::document_blocks::document_from_source(&path, &source)
            .map_err(|error| anyhow::anyhow!("解析文档块失败: {error}"))?;
        let id = reference
            .block_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("文档块引用缺少 block id"))?;
        let block = document
            .find_block(id)
            .map_err(|error| anyhow::anyhow!("读取文档块失败: {error}"))?;
        anyhow::ensure!(
            block.marker_span.is_some(),
            "文档块尚未显式转换，无法作为对象引用"
        );
        let title = path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned());
        let selection = crate::ui::assistant::context::selection_from_document(
            &path.to_string_lossy(),
            title.as_deref(),
            &source,
            (block.source_span.start, block.source_span.end),
        )
        .ok_or_else(|| anyhow::anyhow!("文档块选区为空或无法建立上下文"))?;
        Ok((path, selection.to_value()))
    }

    fn ai_mount_block_session(
        &mut self,
        session_id: &str,
        reference: &ObjectReference,
        workspace: &Path,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let (path, selection) = self.block_selection_value(reference, workspace)?;
            let snippet = crate::ui::assistant::context::SelectionContext::from_value(&selection)
                .map(|selection| selection.text.chars().take(160).collect::<String>())
                .unwrap_or_default();
            let session_title = self
                .ai
                .panel
                .active
                .as_ref()
                .filter(|session| session.id == session_id)
                .map(|session| session.title.clone())
                .ok_or_else(|| anyhow::anyhow!("会话已变化，请重试"))?;
            mochi_core::ai::document_mounts::AiDocumentMountService::new(workspace)
                .add_reference_mounts(vec![
                    mochi_core::ai::document_mounts::CreateReferenceMountInput {
                        session_id: session_id.to_owned(),
                        scope: mochi_core::ai::document_mounts::MountScope::Session,
                        message_id: None,
                        user_message_id: None,
                        document_path: path.to_string_lossy().replace('\\', "/"),
                        object_url: reference.url.clone(),
                        session_title,
                        snippet,
                        created_at: None,
                    },
                ])?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.refresh_right_panel();
                self.state.status_text = "已挂载文档块，原文档内容未修改".into();
            }
            Err(error) => self.ai.panel.error = error.to_string(),
        }
    }

    fn ai_mount_block_message(
        &mut self,
        session_id: &str,
        message_id: &str,
        reference: &ObjectReference,
        workspace: &Path,
    ) {
        let result = (|| -> anyhow::Result<()> {
            let (path, selection) = self.block_selection_value(reference, workspace)?;
            let block_snippet =
                crate::ui::assistant::context::SelectionContext::from_value(&selection)
                    .map(|selection| selection.text.clone())
                    .unwrap_or_default();
            let (session_title, user_message_id, snippet) = {
                let (conversation, index) = self.ai_message(session_id, message_id)?;
                let question = conversation.messages[..index].iter().rev().find(|message| {
                    message.role() == "user" && mochi_core::ai::locator::visible(message)
                });
                (
                    conversation.title.clone(),
                    question.and_then(|message| message.id().map(str::to_owned)),
                    if block_snippet.is_empty() {
                        mochi_core::ai::message_ops::snippet(
                            question
                                .map(|message| message.content())
                                .unwrap_or(conversation.messages[index].content()),
                        )
                    } else {
                        block_snippet.chars().take(160).collect()
                    },
                )
            };
            mochi_core::ai::document_mounts::AiDocumentMountService::new(workspace)
                .add_reference_mounts(vec![
                    mochi_core::ai::document_mounts::CreateReferenceMountInput {
                        session_id: session_id.to_owned(),
                        scope: mochi_core::ai::document_mounts::MountScope::Message,
                        message_id: Some(message_id.to_owned()),
                        user_message_id,
                        document_path: path.to_string_lossy().replace('\\', "/"),
                        object_url: reference.url.clone(),
                        session_title,
                        snippet,
                        created_at: None,
                    },
                ])?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.refresh_right_panel();
                self.state.status_text = "已挂载文档块，原文档内容未修改".into();
            }
            Err(error) => self.ai.panel.error = error.to_string(),
        }
    }

    pub(super) fn open_object_picker(&mut self, purpose: Purpose) {
        self.poll_object_picker();
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let mut files_only = matches!(
            purpose,
            Purpose::Attach | Purpose::Session(_) | Purpose::Message(..)
        );
        let mut documents_only = files_only;
        let mut selected = Vec::new();
        let mut base = None;
        if let Purpose::DesktopSources { card, page, .. } = &purpose {
            documents_only = true;
            if let Some(p) = self
                .desktop
                .panel
                .as_ref()
                .and_then(|panel| panel.config.cards.iter().find(|c| &c.id == card))
                .and_then(|c| c.pages.iter().find(|p| &p.id == page))
            {
                selected = p
                    .sources
                    .iter()
                    .chain(p.source.iter())
                    .map(|s| {
                        let path = root.join(s);
                        objects::build_document_reference(
                            &path,
                            if path.is_dir() {
                                ObjectKind::Directory
                            } else {
                                ObjectKind::Document
                            },
                            Some(&root),
                            None,
                        )
                    })
                    .collect();
            }
        }

        if let Purpose::Reference {
            path,
            record,
            field,
            ..
        } = &purpose
        {
            if let Some((_, viewer::Content::Base(state))) = self.viewer_tab() {
                let field_documents = state.table().fields.iter().any(|f| {
                    &f.id == field && f.field_type == mochi_core::base::FieldType::Document
                });
                files_only |= field_documents;
                documents_only |= field_documents;
                if let Some(row) = state.table().records.iter().find(|r| &r.id == record) {
                    selected = row
                        .values
                        .get(field)
                        .and_then(|v| v.as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                }
                base = Some((path.clone(), state.document.clone()));
            }
            if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
                state.close_popup();
            }
        }
        let title = if matches!(purpose, Purpose::DesktopSources { .. }) {
            "选择知识库、文件夹或文件（可多选）"
        } else if matches!(purpose, Purpose::Canvas(_)) {
            "添加到画布"
        } else if files_only {
            "选择文档"
        } else {
            "选择 Mochi 对象"
        };
        let desktop_sources = matches!(purpose, Purpose::DesktopSources { .. });
        let cache = if desktop_sources {
            &mut self.desktop_source_cache
        } else {
            &mut self.object_picker_cache
        };
        let cached = cache.get(&root);
        let description = if cached.is_some() {
            "搜索名称或路径，多选后点击确定 · 正在检查更新…"
        } else {
            "正在读取工作区对象…"
        };
        let mut candidates = cached.cloned().unwrap_or_default();
        overlay_base(&mut candidates, &root, base.as_ref());
        filter_desktop_sources(&mut candidates, &root, &purpose);
        let mut state = picker::State::new(title, description, candidates, selected);
        if documents_only {
            state.set_documents_only(true);
        }
        self.object_picker = Some(DialogState {
            state,
            purpose,
            previous_focus: self.focus,
            loading: true,
            workspace: root.clone(),
            base,
            files_only,
        });
        if let Some(tx) = cache.begin_refresh(&root) {
            let hwnd = self.hwnd_raw;
            std::thread::spawn(move || {
                // 扫描失败时，绝不能用空列表覆盖掉还能用的快照。
                let result = std::fs::read_dir(&root).map(|_| {
                    if desktop_sources {
                        mochi_core::desktop_cards::collection::source_candidates(&root)
                    } else {
                        objects::collect_candidates(&root, None, None)
                    }
                });
                if tx.send(result).is_ok() && hwnd != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd as *mut _)),
                            platform::WM_APP_OBJECT_PICKER_READY,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            });
        }
        self.focus = Focus::Search;
        self.menu = None;
        self.drag = None;
        self.wiki_suggestion = None;
        self.table_picker = None;
    }

    pub fn poll_object_picker(&mut self) {
        if self.object_picker.as_ref().is_some_and(|dialog| {
            self.shell.workspace().map(|ws| &ws.root) != Some(&dialog.workspace)
        }) {
            self.finish_object_picker(None);
        }
        // 待处理的任务归缓存所有，关闭选择器不会丢弃正在进行的扫描。
        let updates = self
            .object_picker_cache
            .poll()
            .into_iter()
            .map(|(w, s)| (w, s, false))
            .chain(
                self.desktop_source_cache
                    .poll()
                    .into_iter()
                    .map(|(w, s)| (w, s, true)),
            )
            .collect::<Vec<_>>();
        for (workspace, success, desktop_sources) in updates {
            let Some(dialog) = self.object_picker.as_mut().filter(|d| {
                d.workspace == workspace
                    && matches!(d.purpose, Purpose::DesktopSources { .. }) == desktop_sources
            }) else {
                continue;
            };
            dialog.loading = false;
            let cache = if desktop_sources {
                &self.desktop_source_cache
            } else {
                &self.object_picker_cache
            };
            if success {
                let mut candidates = cache.get(&workspace).cloned().unwrap_or_default();
                overlay_base(&mut candidates, &workspace, dialog.base.as_ref());
                filter_desktop_sources(&mut candidates, &workspace, &dialog.purpose);
                dialog.state.update_candidates(candidates);
                let layout = picker::layout(&dialog.state, self.renderer.viewport());
                dialog.state.scroll = dialog.state.scroll.min(layout.max_scroll());
                dialog.state.description = "搜索名称或路径，多选后点击确定".into();
            } else if cache.get(&workspace).is_some() {
                dialog.state.description = "暂时无法更新，仍可搜索和选择已有对象".into();
            } else {
                dialog.state.description = "读取对象失败，可关闭后重试".into();
            }
        }
    }

    pub(super) fn object_picker_click(&mut self, x: f32, y: f32) {
        let Some(dialog) = self.object_picker.as_mut() else {
            return;
        };
        let layout = picker::layout(&dialog.state, self.renderer.viewport());
        let action = dialog.state.click(&layout, x, y);
        if layout.query.contains(x, y) {
            dialog
                .state
                .query
                .click(x - layout.query.left - 12.0, shift_down());
            self.drag = Some(Drag {
                target: DragTarget::ObjectPickerQuery,
                grab_offset: 0.0,
            });
        }
        match action {
            picker::PickerAction::Cancel => self.finish_object_picker(None),
            picker::PickerAction::Confirm(urls) => self.finish_object_picker(Some(urls)),
            _ => {}
        }
    }

    pub(super) fn object_picker_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        let Some(dialog) = self.object_picker.as_mut() else {
            return false;
        };
        if ctrl && key == b'V' as u16 {
            if let Some(text) = platform::read_clipboard_text() {
                dialog
                    .state
                    .query
                    .buffer
                    .insert(&text.replace(['\r', '\n'], " "));
                dialog.state.query_changed();
            }
            return true;
        }
        if ctrl && (key == b'C' as u16 || key == b'X' as u16) {
            if dialog.state.query.buffer.has_selection() {
                let copied = platform::copy_to_clipboard(dialog.state.query.buffer.selected_text());
                if copied && key == b'X' as u16 {
                    dialog.state.query.buffer.delete_backward();
                    dialog.state.query_changed();
                }
                self.show_global_notice(if copied {
                    if key == b'X' as u16 {
                        "已剪切"
                    } else {
                        "已复制"
                    }
                } else {
                    "复制失败，请重试"
                });
            }
            return true;
        }
        match dialog.state.key(key, shift, ctrl) {
            picker::KeyResult::Cancel => self.finish_object_picker(None),
            picker::KeyResult::Submit => {
                let urls = dialog.state.selected_urls();
                self.finish_object_picker(Some(urls));
            }
            _ => {}
        }
        true
    }

    pub(super) fn finish_object_picker(&mut self, urls: Option<Vec<String>>) {
        let Some(dialog) = self.object_picker.take() else {
            return;
        };
        self.focus = dialog.previous_focus;
        self.drag = None;
        let Some(urls) = urls else { return };
        let root = self.shell.workspace().map(|ws| ws.root.clone());
        if root.as_deref() != Some(dialog.workspace.as_path()) {
            return;
        }
        let references = urls
            .iter()
            .filter_map(|url| ObjectReference::parse(url))
            .collect::<Vec<_>>();
        let paths = references
            .iter()
            .filter(|reference| reference.is_document())
            .filter_map(|reference| reference.resolve_path(root.as_deref()))
            .collect::<Vec<_>>();
        let file_paths = references
            .iter()
            .filter(|reference| reference.is_document() && reference.kind != ObjectKind::Block)
            .filter_map(|reference| reference.resolve_path(root.as_deref()))
            .collect::<Vec<_>>();
        let block_references = references
            .iter()
            .filter(|reference| reference.kind == ObjectKind::Block)
            .collect::<Vec<_>>();
        if dialog.files_only && paths.len() != urls.len() {
            self.state.status_text = "只能选择工作区文档".into();
            return;
        }
        match dialog.purpose {
            Purpose::DesktopSources { card, page, module } => {
                let Some(root) = root else { return };
                let result = (|| -> anyhow::Result<Vec<String>> {
                    anyhow::ensure!(file_paths.len() == urls.len(), "请选择知识库、文件夹或文件");
                    let canonical = root.canonicalize()?;
                    file_paths
                        .iter()
                        .map(|path| {
                            let p = path.canonicalize()?;
                            let relative = p
                                .strip_prefix(&canonical)?
                                .to_string_lossy()
                                .replace('\\', "/");
                            mochi_core::desktop_cards::safe_source_path(&root, &relative)?;
                            anyhow::ensure!(
                                mochi_core::desktop_cards::collection::source_relative_allowed(
                                    Path::new(&relative)
                                ) && (p.is_dir()
                                    || mochi_core::desktop_cards::collection::accepts(module, &p)),
                                "请选择当前模板支持的知识库、文件夹或文件"
                            );
                            Ok(relative)
                        })
                        .collect()
                })();
                if let Some(panel) = self.desktop.panel.as_mut() {
                    match result {
                        Ok(sources) => {
                            if let Some(p) = panel
                                .config
                                .cards
                                .iter_mut()
                                .find(|c| c.id == card)
                                .and_then(|c| c.pages.iter_mut().find(|p| p.id == page))
                            {
                                p.sources = sources;
                                p.source = None;
                                panel.dirty = true;
                                panel.error = None;
                            }
                        }
                        Err(error) => panel.error = Some(error.to_string()),
                    }
                }
            }

            Purpose::Attach => {
                self.add_ai_files(file_paths);
                let Some(workspace) = root.as_deref() else {
                    self.ai.panel.error = "没有工作区，无法附加文档块".into();
                    return;
                };
                for reference in block_references {
                    match self.block_selection_value(reference, workspace) {
                        Ok((_, value)) => {
                            crate::ui::assistant::context::add_pending_selection(
                                &mut self.ai.panel.pending_selections,
                                value,
                            );
                        }
                        Err(error) => self.ai.panel.error = error.to_string(),
                    }
                }
                self.invalidate_main();
            }
            Purpose::Session(id) => {
                if self.ai.panel.active.as_ref().is_some_and(|c| c.id == id) {
                    for path in file_paths {
                        self.ai_mount_session(&path);
                    }
                    if let Some(workspace) = root.as_deref() {
                        for reference in block_references {
                            self.ai_mount_block_session(&id, reference, workspace);
                        }
                    }
                }
            }
            Purpose::Message(sid, mid) => {
                if !file_paths.is_empty() {
                    self.ai_mount_message(&sid, &mid, &file_paths);
                }
                if let Some(workspace) = root.as_deref() {
                    for reference in block_references {
                        self.ai_mount_block_message(&sid, &mid, reference, workspace);
                    }
                }
            }
            Purpose::Reference {
                path,
                table,
                record,
                field,
            } => {
                if self.active_file_path().as_deref() != Some(path.as_path()) {
                    return;
                }
                if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
                    if state.table().id != table {
                        return;
                    }
                    let row = state.table().records.iter().position(|r| r.id == record);
                    let column = state.table().fields.iter().position(|f| f.id == field);
                    if let (Some(row), Some(column)) = (row, column) {
                        if let Err(error) = state.set_reference_urls(row, column, urls) {
                            state.error = error;
                        }
                    }
                }
                self.schedule_autosave();
                self.invalidate_main();
            }
            Purpose::Insert => {
                if !urls.is_empty() {
                    if let Some(buffer) = self.shell.active_buffer_mut() {
                        buffer.insert(&format!("\n\n{}\n\n", urls.join("\n\n")));
                        self.after_doc_edit(true);
                    }
                }
            }
            Purpose::Workflow => {
                if let Some(url) = urls.first() {
                    let result = (|| {
                        let mut value: Value =
                            serde_json::from_str(self.workflows.view.field.text())
                                .map_err(|e| e.to_string())?;
                        value
                            .get_mut("inputs")
                            .and_then(Value::as_object_mut)
                            .ok_or("节点 inputs 必须是对象")?
                            .insert("object".into(), serde_json::json!(url));
                        self.workflows.view.field.set_text(
                            &serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
                        );
                        Ok::<_, String>(())
                    })();
                    if let Err(e) = result {
                        self.workflows.view.error = e;
                    }
                    self.focus = Focus::Workflow;
                }
            }
            Purpose::Canvas(path) => {
                if self.active_file_path().as_deref() != Some(path.as_path()) {
                    return;
                }
                let result = match self.viewer_content_mut() {
                    Some(viewer::Content::Canvas(state)) => state.add_references(urls),
                    _ => return,
                };
                match result {
                    Ok(count) => {
                        if self.save_canvas(&path) {
                            self.show_global_notice(format!("已添加 {count} 个关联对象到画布"));
                        }
                        self.invalidate_main();
                    }
                    Err(error) => self.show_global_notice(format!("无法添加画布对象：{error}")),
                }
            }
        }
    }

    pub(super) fn pick_base_references(&mut self, row: usize, column: usize) {
        let Some((path, viewer::Content::Base(state))) = self.viewer_tab() else {
            return;
        };
        let (Some(record), Some(field)) = (
            state.table().records.get(row),
            state.table().fields.get(column),
        ) else {
            return;
        };
        let purpose = Purpose::Reference {
            path: path.into(),
            table: state.table().id.clone(),
            record: record.id.clone(),
            field: field.id.clone(),
        };
        self.open_object_picker(purpose);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 块附加是选中上下文，不是文件附件。走的是选择器确认时同一条
    /// App/Shell 路径，活动编辑器标签页特意带着未保存文本。
    #[test]
    fn attaching_blocks_uses_dirty_source_and_keeps_same_file_blocks_separate() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-block-picker-app-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();

            let path = root.join("note.md");
            let conversion = mochi_core::document_blocks::prepare_conversion(
                &path,
                "第一块（磁盘版本）\n\n第二块（磁盘版本）\n",
            )
            .unwrap();
            std::fs::write(&path, &conversion.converted_source).unwrap();
            let persisted = mochi_core::document_blocks::document_from_source(
                &path,
                &conversion.converted_source,
            )
            .unwrap();
            assert!(persisted.blocks.len() >= 2);
            let first_id = persisted.blocks[0].id.to_string();
            let second_id = persisted.blocks[1].id.to_string();

            // 活动标签页是选择器取材的实时唯一可信来源。
            assert!(app.shell.open_file(&path));
            let dirty = conversion
                .converted_source
                .replace("第一块（磁盘版本）", "第一块（未保存块一😀）")
                .replace("第二块（磁盘版本）", "第二块（未保存块二）");
            let buffer = app.shell.active_buffer_mut().unwrap();
            buffer.replace_range(0..buffer.text().len(), &dirty);
            assert!(buffer.dirty());

            let first_url = mochi_core::object_reference::build_block_reference(
                &path,
                &first_id,
                Some(&root),
                None,
            )
            .unwrap();
            let second_url = mochi_core::object_reference::build_block_reference(
                &path,
                &second_id,
                Some(&root),
                None,
            )
            .unwrap();
            app.object_picker = Some(DialogState {
                state: picker::State::new("选择文档块", "测试", Vec::new(), Vec::<String>::new()),
                purpose: Purpose::Attach,
                previous_focus: Focus::Main,
                loading: false,
                workspace: root.clone(),
                base: None,
                files_only: true,
            });

            app.finish_object_picker(Some(vec![first_url, second_url]));
            assert!(app.object_picker.is_none());
            assert!(app.ai.panel.pending_files.is_empty());
            let selections = crate::ui::assistant::context::pending_selection_contexts(
                &app.ai.panel.pending_selections,
            );
            assert_eq!(selections.len(), 2);
            let first = selections
                .iter()
                .find(|selection| {
                    selection
                        .blocks
                        .iter()
                        .any(|block| block.id.as_str() == first_id.as_str())
                })
                .expect("first block remains its own selection");
            let second = selections
                .iter()
                .find(|selection| {
                    selection
                        .blocks
                        .iter()
                        .any(|block| block.id.as_str() == second_id.as_str())
                })
                .expect("second block remains its own selection");
            assert_eq!(first.blocks.len(), 1);
            assert_eq!(second.blocks.len(), 1);
            assert!(first.text.contains("未保存块一😀"));
            assert!(second.text.contains("未保存块二"));
            assert!(!first.text.contains("磁盘版本"));
            assert!(!second.text.contains("磁盘版本"));
        }
        let resolved = root.canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(
            resolved.starts_with(&temp)
                && resolved.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with("mochi-block-picker-app-")
                })
        );
        std::fs::remove_dir_all(resolved).unwrap();
    }
}

fn filter_desktop_sources(candidates: &mut Vec<ObjectCandidate>, root: &Path, purpose: &Purpose) {
    if let Purpose::DesktopSources { module, .. } = purpose {
        candidates.retain(|c| {
            c.kind == ObjectKind::Directory
                || c.kind == ObjectKind::Document
                    && ObjectReference::parse(&c.url)
                        .and_then(|r| r.resolve_path(Some(root)))
                        .is_some_and(|p| {
                            mochi_core::desktop_cards::collection::accepts(*module, &p)
                        })
        });
    }
}
