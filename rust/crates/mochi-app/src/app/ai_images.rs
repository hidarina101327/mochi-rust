//! 管理 AI 对话中的图片和文件附件，包括粘贴、拖入与移除。
use super::*;
use mochi_core::ai::assets;
impl App {
    /// 将来自文件树或上下文菜单的文件加入 AI 输入区。
    ///
    /// 图片走已有的工作区图片资产管线，从而在输入区显示缩略图；其余文档保留为
    /// 文件附件卡。两者都不会把本地路径拼成一段纯文本提示。
    pub(super) fn add_ai_attachments(&mut self, paths: Vec<PathBuf>) {
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            self.ai.panel.error = "请先打开工作区再添加附件".into();
            return;
        };
        for path in paths {
            if !path.is_absolute() || !path.is_file() {
                self.ai.panel.error = "只能附加可读取的本地文件".into();
                continue;
            }
            if !mochi_core::ai::attachments::is_image(&path) {
                self.add_ai_files(vec![path]);
                continue;
            }
            if self.ai.panel.pending_images.len() >= assets::MAX_IMAGES {
                self.ai.panel.error = "一轮最多附加 20 张图片".into();
                break;
            }
            match assets::import_image(&root, &path) {
                Ok(reference) if !self.ai.panel.pending_images.contains(&reference) => {
                    self.ai.panel.pending_images.push(reference)
                }
                Ok(_) => {}
                Err(error) => {
                    self.ai.panel.error = format!("添加图片失败：{error}");
                    break;
                }
            }
        }
        self.refresh_ai_images();
    }
    pub(super) fn add_ai_files(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if self.ai.panel.pending_files.contains(&path) {
                continue;
            }
            if self.ai.panel.pending_files.len() >= mochi_core::ai::attachments::MAX_FILES {
                self.ai.panel.error = "一轮最多附加 20 个文件".into();
                break;
            }
            if !path.is_absolute() || !path.is_file() {
                self.ai.panel.error = "只能附加可读取的本地文件".into();
                continue;
            }
            self.ai.panel.pending_files.push(path);
        }
    }
    pub(super) fn paste_ai_image_with(
        &mut self,
        read: impl FnOnce() -> anyhow::Result<Option<Vec<u8>>>,
    ) -> bool {
        if self.focus != Focus::AiInput
            || !self.ai_scroll_enabled()
            || self.shell.workspace().is_none()
        {
            return false;
        }
        let data = match read() {
            Ok(None) => return false,
            Ok(Some(data)) => data,
            Err(e) => {
                self.ai.panel.error = format!("图片粘贴失败：{e}");
                return true;
            }
        };
        if self.ai.panel.pending_images.len() >= assets::MAX_IMAGES {
            self.ai.panel.error = "一轮最多附加 20 张图片".into();
            return true;
        }
        let root = self.shell.workspace().unwrap().root.clone();
        match assets::save_image(&root, &data) {
            Ok(reference) => {
                let placeholder = format!("[image {}]", self.ai.panel.next_image_index);
                self.ai.panel.next_image_index = self.ai.panel.next_image_index.saturating_add(1);
                let old = self.ai.panel.input.text();
                let text = if old.is_empty() {
                    placeholder.clone()
                } else {
                    format!("{old} {placeholder}")
                };
                self.ai.panel.input.set_text(&text);
                self.ai
                    .panel
                    .image_placeholders
                    .insert(reference.clone(), placeholder);
                self.ai.panel.pending_images.push(reference);
                self.refresh_ai_images();
            }
            Err(e) => self.ai.panel.error = format!("图片粘贴失败：{e}"),
        }
        true
    }
    pub(super) fn remove_pending_image(&mut self, index: usize) {
        if index >= self.ai.panel.pending_images.len() {
            return;
        }
        let reference = self.ai.panel.pending_images.remove(index);
        if let Some(placeholder) = self.ai.panel.image_placeholders.remove(&reference) {
            let text =
                crate::ui::ai_images::remove_placeholder(self.ai.panel.input.text(), &placeholder);
            self.ai.panel.input.set_text(&text);
        }
        self.refresh_ai_images();
    }
    pub(super) fn refresh_ai_images(&mut self) {
        let Some(root) = self.shell.workspace().map(|w| w.root.clone()) else {
            return;
        };
        let mut refs = self.ai.panel.pending_images.clone();
        if let Some(c) = &self.ai.panel.active {
            for m in &c.messages {
                if let Some(images) = m.get("images").and_then(|v| v.as_array()) {
                    refs.extend(images.iter().filter_map(|v| v.as_str().map(str::to_owned)));
                }
            }
        }
        let refs = refs.into_iter().collect::<std::collections::HashSet<_>>();
        self.ai.panel.image_previews.retain(|r, _| refs.contains(r));
        for reference in refs {
            if self.ai.panel.image_previews.contains_key(&reference) {
                continue;
            }
            if let Ok(path) = assets::local_path(&root, &reference) {
                if let Some((w, h)) = crate::ui::imginfo::dimensions(&path).filter(|(w, h)| {
                    *w > 0 && *h > 0 && u64::from(*w) * u64::from(*h) <= 64_000_000
                }) {
                    self.ai.panel.image_previews.insert(
                        reference,
                        crate::ui::ai_images::ImagePreview {
                            path: path.to_string_lossy().into_owned(),
                            width: w as f32,
                            height: h as f32,
                        },
                    );
                }
            }
        }
    }
    pub fn on_dropped_files(&mut self, paths: Vec<PathBuf>, x: f32, y: f32) {
        if self.dialog.is_some()
            || self.global_import.is_some()
            || self.object_picker.is_some()
            || self.settings_overlay.is_some()
            || self.mapped_folder.is_some()
            || self.link_create.is_some()
            || self.export_form.is_some()
            || self.template_picker.is_some()
            || self.table_picker.is_some()
            || self.commands.review.is_some()
            || self.notification_open()
            || self.desktop_manager_modal()
            || self.automation.panel.is_some()
            || self.image_preview.is_some()
        {
            return;
        }
        if self.offer_package_import(&paths) {
            return;
        }
        if self.on_tree_files_drop(&paths, x, y) {
            return;
        }
        if self.content() == MainContent::Document && self.editor_area.contains(x, y) {
            let images = paths
                .iter()
                .filter(|p| p.is_file() && mochi_core::ai::attachments::is_image(p))
                .collect::<Vec<_>>();
            // 拖入的文件中既有图片也有其他类型时，不能悄悄丢弃非图片文件。
            if !images.is_empty() && images.len() == paths.len() {
                let scroll = self.shell.active_scroll();
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    self.doc
                        .click(self.editor_area, buffer, scroll, x, y, false);
                }
                for image in images {
                    self.import_editor_image(image);
                }
                return;
            }
        }
        if self.ai_scroll_enabled()
            && !paths.iter().any(|path| path.is_dir())
            && (self
                .ai
                .layout
                .rect_of(assistant::Hit::Input)
                .is_some_and(|r| r.contains(x, y))
                || self.ai.layout.messages_rect.contains(x, y))
        {
            self.add_ai_attachments(paths);
        } else {
            self.open_global_import(paths);
        }
    }
    pub(super) fn prepare_pending_images(&self) -> anyhow::Result<Vec<String>> {
        if self.ai.panel.pending_images.is_empty() {
            return Ok(Vec::new());
        }
        let root = &self
            .shell
            .workspace()
            .ok_or_else(|| anyhow::anyhow!("没有工作区"))?
            .root;
        assets::prepare_images(root, &self.ai.panel.pending_images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_attachment_keeps_draft_queue_and_starts_no_request() {
        let root = std::env::temp_dir().join(format!(
            "mochi-file-failure-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
            settings.set(ai_runtime::KEY_BASE_URL, "http://127.0.0.1:9/v1");
            settings.set(ai_runtime::KEY_MODEL, "local");
            let mut app = App::with_settings(settings).unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            app.ai.panel.input.set_text("不要丢弃草稿");
            let missing = root.join("missing.pdf");
            app.ai.panel.pending_files.push(missing.clone());
            app.ai_send(HWND::default());
            assert!(app.ai.run.is_none());
            assert_eq!(app.ai.panel.input.text(), "不要丢弃草稿");
            assert_eq!(app.ai.panel.pending_files, vec![missing]);
            assert!(app.ai.panel.error.contains("准备附件失败"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn paste_is_scoped_preserves_text_fallback_and_removes_its_marker() {
        let root = std::env::temp_dir().join(format!(
            "mochi-paste-image-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.state.view = WorkspaceView::MochiAi;
            app.focus = Focus::Main;
            assert!(!app.paste_ai_image_with(|| panic!("wrong focus must not read clipboard")));
            app.focus = Focus::AiInput;
            app.ai.panel.input.set_text("原有文字");
            assert!(!app.paste_ai_image_with(|| Ok(None)));
            assert_eq!(app.ai.panel.input.text(), "原有文字");
            let png = crate::ui::math_layout::png("x", 24.0, 0, 2.0).unwrap();
            assert!(app.paste_ai_image_with(|| Ok(Some(png.clone()))));
            assert_eq!(app.ai.panel.input.text(), "原有文字 [image 1]");
            assert!(app.paste_ai_image_with(|| Ok(Some(png))));
            assert_eq!(app.ai.panel.pending_images.len(), 2);
            let first = app.ai.panel.pending_images[0].clone();
            app.remove_pending_image(0);
            assert_eq!(app.ai.panel.input.text(), "原有文字 [image 2]");
            assert!(assets::local_path(&root, &first).unwrap().is_file());
            let before = app.ai.panel.input.text().to_owned();
            assert!(app.paste_ai_image_with(|| anyhow::bail!("模拟剪贴板占用")));
            assert_eq!(app.ai.panel.input.text(), before);
            assert_eq!(app.ai.panel.pending_images.len(), 1);
            assert!(app.ai.panel.error.contains("模拟剪贴板占用"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_session_save_keeps_composer_and_does_not_start_model_request() {
        let root = std::env::temp_dir().join(format!(
            "mochi-image-save-fail-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
            settings.set(ai_runtime::KEY_BASE_URL, "http://127.0.0.1:9/v1");
            settings.set(ai_runtime::KEY_MODEL, "local");
            let mut app = App::with_settings(settings).unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            let reference = assets::save_image(&root, b"\x89PNG\r\n\x1a\nsynthetic").unwrap();
            app.ai.panel.pending_images.push(reference.clone());
            app.ai.panel.input.set_text("保留草稿");
            app.ai.panel.active = Some(AiConversation {
                id: "blocked".into(),
                ..Default::default()
            });
            std::fs::create_dir(root.join(".mochi/ai-sessions/sessions/blocked.json")).unwrap();
            app.ai_send(HWND::default());
            assert!(app.ai.run.is_none());
            assert_eq!(app.ai.panel.input.text(), "保留草稿");
            assert_eq!(app.ai.panel.pending_images, vec![reference]);
            assert!(app.ai.panel.active.as_ref().unwrap().messages.is_empty());
            assert!(app.ai.panel.error.contains("保存会话失败"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn dropped_image_is_saved_displayed_and_sent_without_history_images_or_tools() {
        verify_attachment_send(true, false);
    }
    #[test]
    fn file_only_send_keeps_only_paths_and_can_use_agent_tools() {
        verify_attachment_send(false, true);
    }
    #[test]
    fn mixed_text_file_and_image_send_preserves_originals() {
        verify_attachment_send(true, true);
    }
    fn verify_attachment_send(with_image: bool, with_files: bool) {
        use std::io::{Read, Write};
        let top = std::env::temp_dir().join(format!(
            "mochi-image-send-{}",
            mochi_core::paths::random_base36(12)
        ));
        let root = top.join("workspace");
        std::fs::create_dir_all(&root).unwrap();
        let source = top.join("输入图片.png");
        std::fs::write(
            &source,
            crate::ui::math_layout::png("x^2", 32.0, 0x3c5a78, 2.0).unwrap(),
        )
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let mut socket = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut received = Vec::new();
            let request = loop {
                let mut chunk = [0u8; 8192];
                let n = socket.read(&mut chunk).unwrap();
                assert!(n > 0);
                received.extend_from_slice(&chunk[..n]);
                if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&received[..end]);
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if received.len() >= end + 4 + length {
                        break serde_json::from_slice::<serde_json::Value>(
                            &received[end + 4..end + 4 + length],
                        )
                        .unwrap();
                    }
                }
            };
            let response = "data: {\"choices\":[{\"delta\":{\"content\":\"看到了图片\"}}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            request
        });
        {
            let settings = Arc::new(SettingsService::new(Some(top.join("settings.json"))));
            settings.set(ai_runtime::KEY_BASE_URL, &format!("http://{address}/v1"));
            settings.set(ai_runtime::KEY_MODEL, "vision-local");
            settings.set("app.ai.memoryAutoExtract", "false");
            let mut app = App::with_settings(settings).unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();
            app.setup_ai(HWND::default(), &root);
            app.state.view = WorkspaceView::MochiAi;
            let mut old = AiStoredMessage::new("user", "旧问题");
            old.set("images", serde_json::json!(["assets/missing.png"]));
            old.set(
                "files",
                serde_json::json!([{"fileData":"must-not-be-resent","fileName":"old.pdf"}]),
            );
            app.ai.panel.active = Some(AiConversation {
                id: "test-images".into(),
                messages: vec![old, AiStoredMessage::new("assistant", "旧回答")],
                ..Default::default()
            });
            let area = Rect::from_size(0.0, 0.0, 700.0, 800.0);
            app.ai.layout = assistant::layout(&app.ai.panel, area);
            let input = app.ai.layout.rect_of(assistant::Hit::Input).unwrap();
            if with_image {
                app.on_dropped_files(vec![source.clone()], input.left + 2.0, input.top + 2.0);
            }
            let binary = top.join("附件.pdf");
            let note = top.join("原始笔记.md");
            if with_files {
                std::fs::write(&binary, b"opaque transport fixture").unwrap();
                std::fs::write(&note, "原始正文\r\n").unwrap();
                app.on_dropped_files(
                    vec![binary.clone(), note.clone()],
                    input.left + 2.0,
                    input.top + 2.0,
                );
                assert_eq!(app.ai.panel.pending_files.len(), 2);
            }
            assert_eq!(app.ai.panel.pending_images.len(), usize::from(with_image));
            let reference = app.ai.panel.pending_images.first().cloned();
            if let Some(reference) = &reference {
                assert!(app.ai.panel.image_previews.contains_key(reference));
            }
            let pending = assistant::layout(&app.ai.panel, area);
            assert_eq!(
                pending.rect_of(assistant::Hit::RemoveImage(0)).is_some(),
                with_image
            );
            assert_eq!(
                pending.rect_of(assistant::Hit::RemoveFile(0)).is_some(),
                with_files
            );
            app.ai_send(HWND::default());
            assert!(app.ai.panel.pending_images.is_empty());
            assert!(app.ai.panel.pending_files.is_empty());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while app.ai.run.is_some() && std::time::Instant::now() < deadline {
                app.take_ai_events();
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(app.ai.run.is_none());
            assert_eq!(
                app.ai
                    .panel
                    .active
                    .as_ref()
                    .unwrap()
                    .messages
                    .last()
                    .unwrap()
                    .content(),
                "看到了图片"
            );
            let wire = server.join().unwrap();
            assert_eq!(wire.get("tools").is_none(), with_image);
            assert_eq!(wire["messages"][1]["content"], "旧问题");
            let current = wire["messages"].as_array().unwrap().last().unwrap();
            assert!(!wire.to_string().contains("must-not-be-resent"));
            if with_files {
                let expected =
                    mochi_core::ai::attachments::prepare("", &[binary.clone(), note.clone()], 0)
                        .unwrap();
                assert!(expected.files.is_empty());
                let wire_text = current["content"].to_string();
                assert!(wire_text.contains("附件.pdf"));
                assert!(wire_text.contains("原始笔记.md"));
                assert!(!wire_text.contains("原始正文"));
                assert!(!wire_text.contains("file_data"));
                assert_eq!(std::fs::read_to_string(&note).unwrap(), "原始正文\r\n");
            }
            if let Some(reference) = &reference {
                let i = 0;
                assert_eq!(current["content"][i]["type"], "image_url");
                assert_eq!(
                    current["content"][i]["image_url"]["url"],
                    assets::data_url(&root, reference).unwrap()
                );
                assert_eq!(
                    std::fs::read(&source).unwrap(),
                    std::fs::read(assets::local_path(&root, reference).unwrap()).unwrap()
                );
            }
            let stored = AiSessionService::new(&root)
                .load_session("test-images")
                .unwrap();
            if let Some(reference) = &reference {
                assert_eq!(
                    stored.messages[2].get("images").unwrap()[0],
                    reference.as_str()
                );
            }
            if with_files {
                assert!(stored.messages[2].content().contains("📎 **附件.pdf**"));
                assert!(!stored.messages[2].content().contains("原始正文"));
            } else {
                assert_eq!(stored.messages[2].content(), "");
            }
            assert!(stored.messages[2].get("files").is_none());
            assert_eq!(
                assistant::layout(&app.ai.panel, area).messages[2]
                    .images
                    .len(),
                usize::from(with_image)
            );
            let bytes =
                std::fs::read_to_string(root.join(".mochi/ai-sessions/sessions/test-images.json"))
                    .unwrap();
            assert!(!bytes.contains("base64,"));
            assert!(source.is_file());
        }
        std::fs::remove_dir_all(top).unwrap();
    }
}
