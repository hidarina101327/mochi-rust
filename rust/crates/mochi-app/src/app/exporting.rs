//! 处理文档导出选项，并使用当前编辑内容生成导出结果。
use super::*;
use crate::ui::export_dialog::Action;
use mochi_core::exports::enrichment;

impl App {
    pub(super) fn export_form_action(&mut self, action: Option<Action>) {
        let Some(action) = action else { return };
        let Some(form) = self.export_form.take() else {
            return;
        };
        self.focus = Focus::Main;
        if action == Action::Cancel {
            return;
        }
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let source = form.source;
        let options = form.options;
        let output = match mochi_core::exports::output_path(&root, &source, &options.format) {
            Ok(p) => p,
            Err(e) => {
                self.state.status_text = e.to_string();
                return;
            }
        };
        let content = self
            .shell
            .tabs()
            .iter()
            .find(|t| t.path() == Some(source.as_path()))
            .and_then(|t| t.buffer())
            .map(|b| b.text().to_owned());
        let active = if options.conversation != enrichment::ConversationMode::None {
            self.ai.panel.active.clone()
        } else {
            None
        };
        let typography = crate::ui::editor_preferences::current();
        self.state.status_text = "正在导出…".into();
        self.file_jobs
            .submit(source.clone(), self.hwnd_raw, move || {
                crate::ui::editor_preferences::set(typography);
                let content = content
                    .map(Ok)
                    .unwrap_or_else(|| std::fs::read_to_string(&source))?;
                let comments = if options.comments && options.format != "markdown" {
                    sidecars::load_comments(&source.to_string_lossy()).comments
                } else {
                    Vec::new()
                };
                let enriched = enrichment::prepare(&content, &options, &comments, |id| {
                    active
                        .as_ref()
                        .filter(|c| c.id == id)
                        .cloned()
                        .or_else(|| enrichment::load_session(&root, id))
                })?;
                match options.format.as_str() {
                    "pdf" => crate::gfx::export_pdf(&enriched, &output, source.parent())?,
                    "html" => mochi_core::files::FileService::new().write_file_safe(
                        &output,
                        &mochi_core::exports::html(
                            &enriched,
                            &source.file_stem().unwrap_or_default().to_string_lossy(),
                        ),
                    )?,
                    _ => {
                        mochi_core::files::FileService::new().write_file_safe(&output, &enriched)?
                    }
                }
                Ok(crate::file_runtime::Payload::Exported(output))
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exported_extras_use_live_content_without_saving_the_note() {
        let parent = std::env::temp_dir().join(format!(
            "mochi-export-extras-{}",
            mochi_core::paths::random_base36(12)
        ));
        let root = parent.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                parent.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            let source = root.join("原文.md");
            let original = "# 原文\r\n";
            std::fs::write(&source, original).unwrap();
            app.shell.open_file(&source);
            let content = "# 未保存标题\r\n\r\nmochi://ai-locate?session=live&message=answer\r\n";
            let buffer = app.shell.active_buffer_mut().unwrap();
            buffer.select_all();
            buffer.insert(content);
            let mut answer = AiStoredMessage::new("assistant", "内存中的答案");
            answer.set("id", serde_json::json!("answer"));
            app.ai.panel.active = Some(AiConversation {
                id: "live".into(),
                messages: vec![AiStoredMessage::new("user", "内存中的问题"), answer],
                ..Default::default()
            });
            let comments = vec![sidecars::DocumentComment {
                resolved: false,
                id: "comment".into(),
                parent_id: None,
                target_type: "document".into(),
                author: "作者".into(),
                content: "已发表的评论".into(),
                created_at: "2026-09-05T00:00:00Z".into(),
                updated_at: None,
                anchor: None,
                attachments: vec![],
            }];
            sidecars::save_comments(&source.to_string_lossy(), comments).unwrap();
            app.run_menu_action(MenuAction::ExportDocument(source.clone(), "html".into()));
            assert!(app.export_form.is_some());
            let before = app.shell.active_buffer_mut().unwrap().text().to_owned();
            assert!(app.on_shortcut(HWND::default(), 0x41, false, true));
            assert_eq!(app.shell.active_buffer_mut().unwrap().text(), before);
            app.export_form.as_mut().unwrap().activate(3);
            app.export_form.as_mut().unwrap().activate(6);
            app.export_form_action(Some(Action::Submit));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let output = loop {
                if let Some(event) = app.file_jobs.take().into_iter().next() {
                    match event.result.unwrap() {
                        crate::file_runtime::Payload::Exported(path) => break path,
                        _ => panic!("wrong job"),
                    }
                }
                assert!(std::time::Instant::now() < deadline, "export timed out");
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let html = std::fs::read_to_string(output).unwrap();
            for value in ["未保存标题", "内存中的问题", "内存中的答案", "已发表的评论"]
            {
                assert!(html.contains(value));
            }
            assert_eq!(std::fs::read_to_string(&source).unwrap(), original);
            assert_eq!(app.shell.active_buffer_mut().unwrap().text(), content);
            assert!(!root.join(".mochi/ai-sessions/sessions/live.json").exists());
            app.run_menu_action(MenuAction::ExportDocument(source, "markdown".into()));
            app.on_accelerator(HWND::default(), 0x73, false, false, true);
            assert!(app.export_form.is_none());
        }
        std::fs::remove_dir_all(parent).unwrap();
    }
}
