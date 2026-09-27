//! 编辑器中的块转换、恢复和对象操作。
use super::*;
use mochi_core::document_blocks;

impl App {
    /// 读取块的边界时不改动文档源码。内嵌
    /// Mochi 块 ID 已弃用：编辑器或 AI 执行操作时，绝不能
    /// 顺带把标识写入用户的 Markdown 文档。
    pub(super) fn ensure_active_document_blocks(
        &mut self,
    ) -> anyhow::Result<mochi_blocks::model::Document> {
        let path = self
            .active_file_path()
            .ok_or_else(|| anyhow::anyhow!("当前没有可转换的文档"))?;
        let source = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .map(|buffer| buffer.text().to_owned())
            .ok_or_else(|| anyhow::anyhow!("当前标签不是可编辑文档"))?;

        document_blocks::document_from_source(&path, &source)
    }

    pub(super) fn convert_active_blocks(&mut self) {
        self.state.status_text = "文档块已停用：不会再向源文件写入 Mochi 标识".into();
    }

    pub(super) fn active_block_action(&mut self, start: usize, assistant: bool) {
        // 在插入标识的过程中跟踪用户的源码位置。
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(start, false);
        }
        let result = self.ensure_active_document_blocks().and_then(|document| {
            let cursor = self
                .shell
                .active()
                .and_then(|t| t.buffer())
                .unwrap()
                .cursor();
            let block = document
                .blocks
                .iter()
                .filter(|b| b.source_span.start <= cursor && cursor <= b.source_span.end)
                .min_by_key(|b| b.source_span.end - b.source_span.start)
                .ok_or_else(|| anyhow::anyhow!("此位置没有可引用的块"))?;
            Ok((block.id.to_string(), block.source_span))
        });
        let (id, span) = match result {
            Ok(value) => value,
            Err(error) => {
                self.state.status_text = format!("块操作失败：{error}");
                return;
            }
        };
        if assistant {
            if let Some(buffer) = self.shell.active_buffer_mut() {
                buffer.set_cursor(span.start, false);
                buffer.set_cursor(span.end, true);
            }
            if let Some(selection) = self.ai_current_selection() {
                assistant::context::add_pending_selection(
                    &mut self.ai.panel.pending_selections,
                    selection,
                );
            }
            self.state.ai_panel_open = true;
            self.set_right_panel(RightPanel::Assistant);
            self.focus = Focus::AiInput;
        } else {
            let _ = id;
            self.show_global_notice("块引用已停用，避免向源文档写入 Mochi 标识");
        }
    }

    pub(super) fn open_block_restore(&mut self) {
        let Some(path) = self.active_file_path() else {
            return;
        };
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        if buffer.dirty() {
            self.state.status_text = "请先保存当前编辑，再恢复转换前版本".into();
            return;
        }
        let expected = buffer.text().to_owned();
        let backup = document_blocks::conversion_backup_path(&path);
        if !backup.exists() {
            self.state.status_text = "该文档没有块转换备份".into();
            return;
        }
        self.dialog = Some(Dialog { title: "恢复块转换前版本".into(), description: "将恢复启用文档块之前的正文。当前版本会另存为 .blocks-restore-backup，已有块引用可能失效。".into(), field: None, error: String::new(), note: None,
            buttons: vec![DialogButton { label: "取消".into(), kind: ButtonKind::Ghost, action: DialogAction::Dismiss }, DialogButton { label: "保留当前备份并恢复".into(), kind: ButtonKind::Primary, action: DialogAction::RestoreBlockDocument(path, expected) }], dismiss: DialogAction::Dismiss, hover: None });
        self.focus = Focus::Dialog;
    }

    pub(super) fn restore_block_document(&mut self, path: &Path, expected: &str) {
        let result = (|| -> anyhow::Result<String> {
            let current = self
                .shell
                .tabs()
                .iter()
                .find(|t| t.path() == Some(path))
                .and_then(|t| t.buffer())
                .map(|b| b.text());
            anyhow::ensure!(
                current == Some(expected) && std::fs::read_to_string(path)? == expected,
                "文档已变化，请重新打开恢复确认"
            );
            let recovery = PathBuf::from(format!("{}.blocks-restore-backup", path.display()));
            use std::io::Write;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&recovery)
            {
                Ok(mut file) => {
                    file.write_all(expected.as_bytes())?;
                    file.sync_all()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    anyhow::ensure!(
                        std::fs::read_to_string(&recovery)? == expected,
                        "已有不同的恢复备份，请先保留或移动该备份：{}",
                        recovery.display()
                    );
                }
                Err(error) => return Err(error.into()),
            }
            Ok(document_blocks::restore_file(path, expected)?.source)
        })();
        match result {
            Ok(source) => {
                self.shell.accept_written_text(path, &source);
                self.close_dialog();
                self.invalidate_main();
                self.sync_state();
                self.state.status_text = "已恢复转换前版本，并保留恢复前备份".into();
            }
            Err(error) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.error = error.to_string();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_markers_are_hidden_without_changing_rendered_content() {
        for source in [
            "# 标题\n\n**中文😀** 和普通文字\n\n> 引用内容\n\n- [ ] 待办\n",
            "---\r\ntitle: metadata\r\n---\r\n\r\n正文😀\r\n\r\n```rust\r\nlet x = 1;\r\n```\r\n",
            ":::mochi-highlight color=\"blue\" title=\"提示\"\n内容😀\n:::\n\n后文\n",
        ] {
            let converted = document_blocks::prepare_conversion(Path::new("test.mc"), source)
                .unwrap()
                .converted_source;
            let original = crate::ui::live::layout(source, None, 720.0, &|_| None);
            let next = crate::ui::live::layout(&converted, None, 720.0, &|_| None);
            assert!(
                (original.layout.height - next.layout.height).abs() < 0.01,
                "height changed: {source}"
            );
            let mut before = crate::ui::editor::TextBuffer::new(source);
            before.select_all();
            let mut after = crate::ui::editor::TextBuffer::new(&converted);
            after.select_all();
            assert_eq!(
                original.selected_text(&before),
                next.selected_text(&after),
                "clipboard changed: {source}"
            );
            assert!(!next.selected_text(&after).contains("<!-- mochi:block"));
            let context = assistant::context::selection_from_document(
                "test.mc",
                None,
                &converted,
                after.selection(),
            )
            .unwrap();
            assert!(!context.blocks.is_empty());
            let value = context.to_value();
            assert_eq!(
                assistant::context::SelectionContext::from_value(&value).unwrap(),
                context
            );
            let prompt =
                assistant::context::build_selection_prompt(&[value], "润色第一块").unwrap();
            assert!(prompt.contains(context.blocks[0].id.as_str()));
            assert!(prompt.contains("document_range_propose_edit"));
        }
    }

    #[test]
    fn real_shell_save_reopen_keeps_document_source_verbatim() {
        let dir = std::env::temp_dir().join(format!(
            "mochi-block-editor-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "原文😀\n").unwrap();
        let mut shell = Shell::new();
        assert!(shell.open_file(&path));
        let buffer = shell.active_buffer_mut().unwrap();
        buffer.set_cursor(buffer.text().len(), false);
        buffer.insert("\n新的块\n");
        assert!(shell.save_active(), "{}", shell.status());
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, "原文😀\n\n新的块\n");
        assert!(!saved.contains("mochi:block"));
        assert!(!document_blocks::conversion_backup_path(&path).exists());
        let mut reopened = Shell::new();
        assert!(reopened.open_file(&path));
        assert_eq!(reopened.active().unwrap().buffer().unwrap().text(), saved);
        drop(shell);
        drop(reopened);
        let resolved = dir.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mochi-block-editor-"));
        std::fs::remove_dir_all(resolved).unwrap();
    }

    #[test]
    fn save_does_not_rewrite_or_reject_legacy_block_marker_text() {
        let dir = std::env::temp_dir().join(format!(
            "mochi-block-save-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        let original = "正常正文\n";
        std::fs::write(&path, original).unwrap();
        let mut shell = Shell::new();
        assert!(shell.open_file(&path));
        let malformed = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001\n未闭合标记\n";
        shell
            .active_buffer_mut()
            .unwrap()
            .replace_range(0..original.len(), malformed);
        assert!(shell.save_active(), "{}", shell.status());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
        let buffer = shell.active().unwrap().buffer().unwrap();
        assert_eq!(buffer.text(), malformed);
        assert!(!buffer.dirty());
        drop(shell);
        let resolved = dir.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mochi-block-save-"));
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
