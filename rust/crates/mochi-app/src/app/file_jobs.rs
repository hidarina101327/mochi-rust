//! 启动和收取文件相关的后台任务，包括导出和链接缓存更新。
use super::*;

impl App {
    pub(super) fn start_link_cache(&mut self) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let Some((path, viewer::Content::Link(s))) = self.viewer_tab() else {
            return;
        };
        if s.busy {
            return;
        }
        let path = path.to_path_buf();
        if let Some(viewer::Content::Link(s)) = self.viewer_content_mut() {
            s.busy = true;
        }
        self.file_jobs.submit(path.clone(), self.hwnd_raw, move || {
            mochi_core::link_files::cache(&root, &path).map(crate::file_runtime::Payload::LinkCache)
        });
    }

    pub fn take_file_jobs(&mut self) {
        for event in self.file_jobs.take() {
            match event.result {
                Ok(crate::file_runtime::Payload::Marketplace(event)) => {
                    self.finish_marketplace(event)
                }
                Ok(crate::file_runtime::Payload::Imported {
                    root,
                    target,
                    report,
                }) => {
                    if self.shell.workspace().is_some_and(|ws| ws.root == root) {
                        self.shell.expand(&target);
                        self.shell.refresh_tree();
                        self.sync_state();
                        let message = format!(
                            "已导入 {} 项，保存 {} 个引用资源",
                            report.imported.len(),
                            report.resources
                        );
                        if report.warnings.is_empty() {
                            self.show_global_notice(&message);
                        } else {
                            self.show_global_notice(&format!(
                                "{message}；{} 项未完成：{}",
                                report.warnings.len(),
                                report.warnings.join("；")
                            ));
                        }
                        self.invalidate_main();
                    }
                }
                Ok(crate::file_runtime::Payload::CommandFinished {
                    root,
                    request,
                    queue,
                    error,
                }) => self.finish_command(root, request, queue, error),
                Ok(crate::file_runtime::Payload::Image { url, result }) => {
                    self.finish_remote_image(url, result)
                }
                Ok(crate::file_runtime::Payload::Exported(path)) => {
                    self.state.status_text = format!("已导出：{}", path.display())
                }
                Ok(crate::file_runtime::Payload::Office(pdf)) => {
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(&event.path) {
                        s.document_path = Some(pdf);
                        s.converting = false;
                        s.error = None;
                    }
                }
                Ok(crate::file_runtime::Payload::Sheet(data)) => {
                    if let Some(viewer::Content::Spreadsheet(s)) =
                        self.viewer_content_for(&event.path)
                    {
                        s.data = Some(data);
                        s.loading = false;
                        s.error.clear();
                        s.scroll_x = 0.0;
                        s.scroll_y = 0.0;
                    }
                }
                Ok(crate::file_runtime::Payload::LinkCache(cache)) => {
                    if let Some(viewer::Content::Link(s)) = self.viewer_content_for(&event.path) {
                        s.title = cache.title;
                        s.description = cache.description;
                        s.cached = true;
                        s.cached_at = Some(cache.cached_at);
                        s.content = cache.content;
                        s.busy = false;
                    }
                }
                Ok(crate::file_runtime::Payload::Created(path)) => {
                    if self
                        .shell
                        .workspace()
                        .is_some_and(|ws| path.starts_with(&ws.root))
                    {
                        self.shell.refresh_tree();
                        if self.open_file_from_ui(&path) {
                            self.state.view = WorkspaceView::Editor;
                            self.invalidate_main();
                            self.sync_state();
                        }
                    }
                }
                Err(e) => {
                    match self.viewer_content_for(&event.path) {
                        Some(viewer::Content::Link(s)) => s.busy = false,
                        Some(viewer::Content::Pdf(s)) => {
                            s.error = Some(e.clone());
                            s.loading = false;
                        }
                        Some(viewer::Content::Spreadsheet(s)) => {
                            s.error = e.clone();
                            s.loading = false;
                        }
                        _ => {}
                    }
                    self.state.status_text = e;
                }
            }
        }
        if self.state.right_panel == RightPanel::AgentInbox {
            self.refresh_right_panel();
        }
    }

    pub(super) fn approve_document_export(&mut self, id: &str) -> bool {
        let Some(queue) = self.ai.export_requests.clone() else {
            return false;
        };
        let request = match queue.claim(id) {
            Ok(r) => r,
            Err(e) => {
                let error = e.to_string();
                self.state.status_text = error.clone();
                if let Some(review) = self.commands.review.as_mut() {
                    review.error = error;
                }
                return false;
            }
        };
        let validate = (|| -> std::result::Result<(), String> {
            let host = self.ai.host.as_ref().ok_or("导出宿主不可用")?;
            mochi_core::ai::tools::host::resolve_workspace_path(
                host.as_ref(),
                Some(&request.source),
                Default::default(),
            )?;
            let ws = self.shell.workspace().ok_or("工作区不可用")?;
            let expected = mochi_core::exports::output_path(
                &ws.root,
                Path::new(&request.source),
                &request.format,
            )
            .map_err(|e| e.to_string())?;
            if !expected
                .to_string_lossy()
                .replace('\\', "/")
                .eq_ignore_ascii_case(&request.output.replace('\\', "/"))
            {
                return Err("导出目标与请求不匹配".into());
            }
            let p = self.ai.permissions.as_ref().ok_or("权限服务不可用")?;
            p.assert_tool_action_allowed(
                mochi_core::ai::permission::AiToolAction::ReadFile,
                Some(&request.source),
            )
            .map_err(|e| e.to_string())?;
            p.assert_tool_action_allowed(
                mochi_core::ai::permission::AiToolAction::WriteFile,
                Some(&request.output),
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(error) = validate {
            queue.release(id);
            self.state.status_text = error.clone();
            if let Some(review) = self.commands.review.as_mut() {
                review.error = error;
            }
            return false;
        }
        let typography = crate::ui::editor_preferences::current();
        self.state.status_text = "已批准，正在导出…".into();
        self.file_jobs
            .submit(PathBuf::from(&request.source), self.hwnd_raw, move || {
                let result = (|| -> anyhow::Result<crate::file_runtime::Payload> {
                    crate::ui::editor_preferences::set(typography);
                    let content = match &request.content {
                        Some(c) => c.clone(),
                        None => std::fs::read_to_string(&request.source)?,
                    };
                    let output = PathBuf::from(&request.output);
                    let content = mochi_core::exports::enrichment::prepare(
                        &content,
                        &mochi_core::exports::enrichment::Options {
                            format: request.format.clone(),
                            ..Default::default()
                        },
                        &[],
                        |_| None,
                    )?;
                    match request.format.as_str() {
                        "pdf" => crate::gfx::export_pdf(
                            &content,
                            &output,
                            Path::new(&request.source).parent(),
                        )?,
                        "html" => mochi_core::files::FileService::new().write_file_safe(
                            &output,
                            &mochi_core::exports::html(&content, &request.source),
                        )?,
                        "markdown" => mochi_core::files::FileService::new()
                            .write_file_safe(&output, &content)?,
                        _ => anyhow::bail!("不支持的导出格式"),
                    }
                    queue.remove(&request.id)?;
                    Ok(crate::file_runtime::Payload::Exported(output))
                })();
                if result.is_err() {
                    queue.release(&request.id);
                }
                result
            });
        true
    }

    pub(super) fn start_sheet_job(&mut self, path: &Path, sheet: Option<String>) {
        if let Some(viewer::Content::Spreadsheet(s)) = self.viewer_content_for(path) {
            if s.loading && s.requested {
                return;
            }
            s.loading = true;
            s.requested = true;
            s.error.clear();
        }
        let path = path.to_path_buf();
        self.file_jobs.submit(path.clone(), self.hwnd_raw, move || {
            mochi_core::office::load_sheet(&path, sheet.as_deref())
                .map(crate::file_runtime::Payload::Sheet)
        });
    }
}
