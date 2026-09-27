//! 真实剪贴板内容会在进入文档时转换。表单字段、源码和代码编辑
//! 始终保留原文；普通粘贴不会读取图片。
use super::*;

impl App {
    fn clipboard_rich_target(&self) -> bool {
        if self.focus == Focus::TableCell {
            return true;
        }
        if !self.editing_file() || self.content() != MainContent::Document {
            return false;
        }
        let Some(buffer) = self.shell.active().and_then(|tab| tab.buffer()) else {
            return false;
        };
        let parsed = crate::ui::document::parse_ranged(buffer.text());
        let (start, end) = buffer.selection();
        !parsed.block_at(buffer.cursor()).is_some_and(|index| {
            let block = &parsed.blocks[index];
            let replacing_block = start < end && start <= block.start && end >= block.end;
            !replacing_block
                && matches!(
                    block.block,
                    crate::ui::document::Block::Code { .. } | crate::ui::document::Block::Math(_)
                )
        })
    }

    pub(super) fn clipboard_paste_mode(&mut self, plain: bool) -> bool {
        let rich_target = self.clipboard_rich_target();
        if !plain && rich_target && self.focus != Focus::TableCell {
            if let Some(text) = platform::read_clipboard_text() {
                if crate::ui::latex_paste::formula_block(&text).is_some() {
                    return self.insert_clipboard_content(&text, false);
                }
            }
        }
        if !plain && rich_target {
            if let Some(html) = platform::read_clipboard_html() {
                if let Some(source) = crate::ui::paste::from_html(&html) {
                    if !source.is_empty() {
                        return self.insert_formatted_clipboard_content(&source);
                    }
                }
            }
        }
        if !plain && self.focus != Focus::TableCell {
            if self.paste_ai_image_with(platform::read_clipboard_image) {
                return true;
            }
            if rich_target && self.paste_editor_image_with(platform::read_clipboard_image) {
                return true;
            }
        }
        if self.focus != Focus::TableCell
            && !self.editing_file()
            && self.focused_field_mut().is_none()
        {
            return false;
        }
        let Some(text) = platform::read_clipboard_text() else {
            return true;
        };
        self.insert_clipboard_content(&text, plain)
    }

    fn insert_formatted_clipboard_content(&mut self, source: &str) -> bool {
        self.insert_clipboard_content(source, false)
    }

    fn separate_clipboard_blocks(&self, source: &str) -> String {
        let mut value = source.to_owned();
        if self.focus != Focus::TableCell && self.clipboard_rich_target() {
            if let Some(buffer) = self.shell.active().and_then(|tab| tab.buffer()) {
                let (start, end) = buffer.selection();
                let parsed = crate::ui::document::parse_ranged(source);
                let standalone = |block: &crate::ui::document::RangedBlock| {
                    !matches!(
                        block.block,
                        crate::ui::document::Block::Paragraph(_)
                            | crate::ui::document::Block::Blank
                    )
                };
                let left = buffer.text()[..start].rsplit('\n').next().unwrap_or("");
                let right = buffer.text()[end..].split('\n').next().unwrap_or("");
                // 在现有段落中间粘贴时，列表、表格和围栏代码
                // 必须各自占据独立的源码行。
                if parsed.blocks.first().is_some_and(standalone) && !left.trim().is_empty() {
                    value.insert_str(0, "\n\n");
                }
                if parsed.blocks.last().is_some_and(standalone) && !right.trim().is_empty() {
                    value.push_str("\n\n");
                }
            }
        }
        value
    }

    /// 剪贴板和菜单输入共用同一条路径，并留有纯数据测试接口，方便编写回归
    /// 测试。生产读取流程不会使用测试夹具提供方。
    pub(super) fn insert_clipboard_content(&mut self, text: &str, plain: bool) -> bool {
        if text.is_empty() {
            return true;
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let rich_target = self.clipboard_rich_target();
        let source = if plain && rich_target {
            crate::ui::paste::plain_text_source(&normalized)
        } else if rich_target && self.focus != Focus::TableCell {
            let source = crate::ui::latex_paste::formula_block(&normalized).unwrap_or(normalized);
            self.separate_clipboard_blocks(&source)
        } else {
            normalized
        };
        if self.focus == Focus::TableCell {
            if let Some(edit) = self.table_editing.as_mut() {
                edit.field.buffer.break_undo_group();
                crate::ui::rich::insert(&mut edit.field.buffer, &source);
                edit.field.buffer.break_undo_group();
            }
            return true;
        }
        if self.editing_file() {
            let rich = self.content() == MainContent::Document
                && !crate::ui::editor_preferences::current().live_line_source;
            if let Some(buffer) = self.shell.active_buffer_mut() {
                buffer.break_undo_group();
                let source = if buffer.text().contains("\r\n") {
                    source.replace('\n', "\r\n")
                } else {
                    source
                };
                if rich {
                    crate::ui::rich::insert(buffer, &source);
                } else {
                    buffer.insert(&source);
                }
                buffer.break_undo_group();
            }
            self.after_edit(true);
            return true;
        }
        let multiline = self.dialog.as_ref().is_some_and(|d| d.is_math_editor())
            || matches!(
                self.focus,
                Focus::AiInput | Focus::CommentCompose | Focus::CanvasText
            )
            || self.agenda_multiline_focused();
        let source = if multiline {
            source
        } else {
            source.replace('\n', " ")
        };
        let Some(field) = self.focused_field_mut() else {
            return false;
        };
        field.buffer.break_undo_group();
        field.buffer.insert(&source);
        field.buffer.break_undo_group();
        self.after_field_edit();
        true
    }

    pub(super) fn clipboard_copy_formatted_with(
        &mut self,
        cut: bool,
        write: impl FnOnce(&str, Option<&str>) -> bool,
    ) -> bool {
        let (plain, source) = if self.focus == Focus::TableCell {
            let Some(edit) = self.table_editing.as_ref() else {
                return false;
            };
            if !edit.field.buffer.has_selection() {
                return true;
            }
            (
                edit.selected_text(),
                crate::ui::rich::selected_markdown(&edit.field.buffer),
            )
        } else {
            let Some(buffer) = self.shell.active().and_then(|tab| tab.buffer()) else {
                return false;
            };
            if !buffer.has_selection() {
                return true;
            }
            (
                self.doc.selected_text(buffer),
                crate::ui::rich::selected_markdown(buffer),
            )
        };
        let options = pulldown_cmark::Options::ENABLE_TABLES
            | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
            | pulldown_cmark::Options::ENABLE_TASKLISTS;
        let mut html = String::new();
        pulldown_cmark::html::push_html(
            &mut html,
            pulldown_cmark::Parser::new_ext(&source, options),
        );
        if !write(&plain, Some(&html)) {
            self.state.status_text = "复制失败，原内容已保留".into();
            return true;
        }
        if cut {
            if self.focus == Focus::TableCell {
                if let Some(edit) = self.table_editing.as_mut() {
                    crate::ui::rich::insert(&mut edit.field.buffer, "");
                }
            } else {
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    crate::ui::rich::insert(buffer, "");
                }
                self.after_edit(true);
            }
        }
        self.show_global_notice(if cut { "已剪切" } else { "已复制" });
        true
    }

    pub(super) fn clipboard_menu_items() -> Vec<MenuItem<MenuAction>> {
        vec![
            MenuItem::new("粘贴", MenuAction::EditorPaste),
            MenuItem::new("纯文本粘贴", MenuAction::EditorPastePlain),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_editor(source: &str, test: impl FnOnce(&mut App)) {
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
        };
        struct Com(bool);
        impl Drop for Com {
            fn drop(&mut self) {
                if self.0 {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }
        let _com = Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok());
        let root = std::env::temp_dir().join(format!(
            "mochi-paste-{}",
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
            std::fs::write(&path, source).unwrap();
            assert!(app.shell.open_file(&path));
            app.state.view = WorkspaceView::Editor;
            app.focus = Focus::Main;
            app.editor_engaged = true;
            app.sync_state();
            test(&mut app);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn formatted_copy_restores_partial_marks_and_failed_cut_keeps_source() {
        with_editor("前 **中文😀加粗** 后", |app| {
            let buffer = app.shell.active_buffer_mut().unwrap();
            let start = buffer.text().find("中文").unwrap();
            buffer.set_cursor(start, false);
            buffer.set_cursor(start + "中文😀".len(), true);
            let before = buffer.text().to_owned();
            let area = Rect::new(0.0, 0.0, 800.0, 600.0);
            app.doc
                .ensure(area, app.shell.active_tab(), Some(&before), None);
            assert!(app.clipboard_copy_formatted_with(true, |plain, html| {
                assert_eq!(plain, "中文😀");
                assert!(html.unwrap().contains("<strong>中文😀</strong>"));
                false
            }));
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), before);
            assert!(app.clipboard_copy_formatted_with(true, |_, _| true));
            assert_eq!(app.status_bar.toast.message, "已剪切");
            assert!(app.status_bar.timer.is_some());
            assert!(!app
                .shell
                .active()
                .unwrap()
                .buffer()
                .unwrap()
                .text()
                .contains("中文😀"));
            assert!(app.shell.active_buffer_mut().unwrap().undo());
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), before);
        });
    }

    #[test]
    fn repeated_global_copy_notice_rearms_its_timer() {
        with_editor("x", |app| {
            app.show_global_notice("已复制");
            assert!(app
                .take_timer_requests()
                .iter()
                .any(|(id, delay)| *id == platform::TIMER_TOAST && *delay >= 500));
            app.show_global_notice("已复制");
            assert!(app
                .take_timer_requests()
                .iter()
                .any(|(id, delay)| *id == platform::TIMER_TOAST && *delay >= 500));
            assert_eq!(app.status_bar.toast.message, "已复制");
        });
    }

    #[test]
    fn multiline_paste_preserves_spacing_and_undo_for_lf_and_crlf() {
        for newline in ["\n", "\r\n"] {
            let original = format!("前文后文{newline}");
            with_editor(&original, |app| {
                for (text, plain, html) in [
                    ("甲\n乙\n\n丙", false, false),
                    ("甲\r\n乙\r\n\r\n丙", true, false),
                    ("<p>甲</p><p>乙</p><p><br></p><p>丙</p>", false, true),
                ] {
                    app.shell
                        .active_buffer_mut()
                        .unwrap()
                        .set_cursor("前文".len(), false);
                    if html {
                        let source = crate::ui::paste::from_html(text).unwrap();
                        assert!(app.insert_formatted_clipboard_content(&source));
                    } else {
                        assert!(app.insert_clipboard_content(text, plain));
                    }
                    let buffer = app.shell.active_buffer_mut().unwrap();
                    assert_eq!(
                        buffer.text(),
                        format!("前文甲{newline}乙{newline}{newline}丙后文{newline}")
                    );
                    assert!(buffer.undo());
                    assert_eq!(buffer.text(), original);
                }
            });
        }
    }

    #[test]
    fn rich_paste_preserves_html_structure_and_is_one_undo_step() {
        with_editor("原文\r\n", |app| {
            app.shell.active_buffer_mut().unwrap().set_cursor(0, false);
            let source = crate::ui::paste::from_html(
                "<p><strong>中文</strong> <em>格式</em></p><ul><li>条目</li></ul>",
            )
            .unwrap();
            assert!(app.insert_formatted_clipboard_content(&source));
            let buffer = app.shell.active_buffer_mut().unwrap();
            assert!(crate::ui::text::parse_inline(buffer.text())
                .iter()
                .any(|run| run.text.contains("中文")
                    && run.emphasis.base() == crate::ui::text::Emphasis::Bold));
            assert!(buffer.text().contains("条目"));
            assert!(!buffer.text().replace("\r\n", "").contains('\n'));
            assert!(buffer.undo());
            assert_eq!(buffer.text(), "原文\r\n");
        });
    }

    #[test]
    fn formula_paste_creates_a_block_between_paragraphs_and_undo_restores_crlf() {
        with_editor("前文后文\r\n", |app| {
            app.shell
                .active_buffer_mut()
                .unwrap()
                .set_cursor("前文".len(), false);
            let tex = r"S = \sum\_{i=1}^{n} w\_i s\_i, \qquad \sum\_{i=1}^{n} w\_i = 1";
            assert!(app.insert_clipboard_content(tex, false));
            let buffer = app.shell.active_buffer_mut().unwrap();
            let parsed = crate::ui::document::parse_ranged(buffer.text());
            assert!(parsed.blocks.iter().any(|block| matches!(&block.block,
                crate::ui::document::Block::Math(value) if value.contains(r"\sum_{i=1}^{n} w_i"))));
            assert!(buffer.text().starts_with("前文\r\n\r\n$$\r\n"));
            assert!(buffer.text().ends_with("\r\n$$\r\n\r\n后文\r\n"));
            assert!(buffer.undo());
            assert_eq!(buffer.text(), "前文后文\r\n");
        });
    }

    #[test]
    fn formula_paste_leaves_plain_source_code_and_form_fields_literal() {
        let tex = r"\sum_{i=1}^{n} w_i";
        with_editor("", |app| {
            app.insert_clipboard_content(tex, true);
            assert!(!crate::ui::document::parse_ranged(
                app.shell.active().unwrap().buffer().unwrap().text()
            )
            .blocks
            .iter()
            .any(|block| matches!(block.block, crate::ui::document::Block::Math(_))));
        });
        with_editor("```latex\n\n```", |app| {
            app.shell.active_buffer_mut().unwrap().set_cursor(9, false);
            app.insert_clipboard_content(tex, false);
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().text(),
                format!("```latex\n{tex}\n```")
            );
        });
        with_editor("", |app| {
            if let TabKind::File { source_mode, .. } = &mut app.shell.active_mut().unwrap().kind {
                *source_mode = true;
            }
            app.insert_clipboard_content(tex, false);
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), tex);
            app.focus = Focus::AiInput;
            app.insert_clipboard_content(tex, false);
            assert_eq!(app.ai.panel.input.text(), tex);
        });
    }

    fn open_formula(app: &mut App) {
        app.paint(HWND::default()).unwrap();
        let rect = app
            .list
            .cmds()
            .iter()
            .find_map(|cmd| match cmd {
                crate::ui::draw::DrawCmd::Math { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("rendered formula");
        app.on_click(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
        assert!(app
            .dialog
            .as_ref()
            .is_some_and(|dialog| dialog.is_math_editor()));
    }

    #[test]
    fn formula_dialog_previews_multiline_drafts_saves_and_cancels_without_losing_source() {
        let original = "$$\r\nx^2\r\n$$\r\n";
        with_editor(original, |app| {
            let snapshot = app.renderer.prepare_snapshot(1280, 900, 96.0).unwrap();
            open_formula(app);
            app.dialog
                .as_mut()
                .unwrap()
                .field
                .as_mut()
                .unwrap()
                .select_all();
            let draft = "\\begin{aligned}\nS &= \\sum_{i=1}^{n} w_i s_i \\\\\n\\sum_{i=1}^{n} w_i &= 1\n\\end{aligned}";
            app.insert_clipboard_content(draft, false);
            assert_eq!(
                app.dialog.as_ref().unwrap().field.as_ref().unwrap().text(),
                draft
            );
            assert!(app.on_edit_key(13, false, false));
            assert_eq!(
                app.dialog.as_ref().unwrap().field.as_ref().unwrap().text(),
                format!("{draft}\n")
            );
            app.paint(HWND::default()).unwrap();
            assert!(app.list.cmds().iter().any(|cmd| matches!(cmd,
                crate::ui::draw::DrawCmd::Math { tex, .. } if tex == draft)));
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().text(),
                original
            );
            let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/formula-editor-acceptance");
            std::fs::create_dir_all(&folder).unwrap();
            app.renderer
                .save_snapshot(&snapshot, &folder.join("formula-editor-wide.png"))
                .unwrap();
            assert!(app.on_edit_key(13, false, true));
            assert!(app.dialog.is_none());
            let saved = format!("$$\r\n{}\r\n$$\r\n", draft.replace('\n', "\r\n"));
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), saved);
            open_formula(app);
            app.dialog
                .as_mut()
                .unwrap()
                .field
                .as_mut()
                .unwrap()
                .set_text(r"\frac{");
            app.paint(HWND::default()).unwrap();
            assert!(app.on_edit_key(27, false, false));
            assert_eq!(app.shell.active().unwrap().buffer().unwrap().text(), saved);
            assert!(app.shell.active_buffer_mut().unwrap().undo());
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().text(),
                original
            );

            let snapshot = app.renderer.prepare_snapshot(640, 700, 96.0).unwrap();
            open_formula(app);
            app.dialog
                .as_mut()
                .unwrap()
                .field
                .as_mut()
                .unwrap()
                .set_text(draft);
            app.paint(HWND::default()).unwrap();
            app.renderer
                .save_snapshot(&snapshot, &folder.join("formula-editor-narrow.png"))
                .unwrap();
        });
    }

    #[test]
    fn formula_dialog_supports_pointer_selection_scrolling_and_keyboard_navigation() {
        with_editor("$$\nx^2\n$$", |app| {
            let _snapshot = app.renderer.prepare_snapshot(1200, 900, 96.0).unwrap();
            open_formula(app);
            let lines = (0..50)
                .map(|i| format!("x_{i} + y_{i}\n"))
                .collect::<String>();
            app.dialog
                .as_mut()
                .unwrap()
                .field
                .as_mut()
                .unwrap()
                .set_text(&lines);
            app.on_edit_key(36, false, true); // Ctrl+Home
            app.paint(HWND::default()).unwrap();
            let rect = app
                .dialog
                .as_ref()
                .unwrap()
                .field_rect(app.renderer.viewport())
                .unwrap();
            app.on_click(rect.left + 12.0, rect.top + 12.0);
            app.on_mouse_move(rect.left + 80.0, rect.top + 60.0);
            assert!(app
                .dialog
                .as_ref()
                .unwrap()
                .field
                .as_ref()
                .unwrap()
                .buffer
                .has_selection());
            app.drag = None;
            app.on_edit_key(36, false, true);
            app.on_edit_key(40, false, false);
            assert_eq!(
                app.dialog
                    .as_ref()
                    .unwrap()
                    .field
                    .as_ref()
                    .unwrap()
                    .buffer
                    .cursor(),
                lines.find('\n').unwrap() + 1
            );
            app.on_wheel(rect.left + 30.0, rect.top + 30.0, -480);
            app.paint(HWND::default()).unwrap();
            app.on_click(rect.left + 12.0, rect.top + 12.0);
            assert!(
                app.dialog
                    .as_ref()
                    .unwrap()
                    .field
                    .as_ref()
                    .unwrap()
                    .buffer
                    .cursor()
                    > 40
            );
            app.on_edit_key(27, false, false);
            assert!(app.drag.is_none());
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().text(),
                "$$\nx^2\n$$"
            );
        });
    }

    #[test]
    fn details_header_title_and_empty_space_toggle_but_menu_still_opens() {
        let source = "<details open>\n<summary>标题</summary>\n\n内容\n\n</details>\n";
        with_editor(source, |app| {
            let _snapshot = app.renderer.prepare_snapshot(1200, 900, 96.0).unwrap();
            app.paint(HWND::default()).unwrap();
            let area = app.editor_area;
            let x = area.left + crate::ui::editor_preferences::current().padding_left + 80.0;
            let y = (area.top as i32..area.bottom as i32)
                .map(|y| y as f32)
                .find(|y| {
                    app.doc
                        .container_at(area, app.shell.active_scroll(), x, *y)
                        .is_some()
                })
                .unwrap();
            app.on_click(x, y);
            assert!(
                !crate::ui::containers::scan(app.shell.active().unwrap().buffer().unwrap().text())
                    [0]
                .open
            );
            assert!(app.menu.is_none());
            app.paint(HWND::default()).unwrap();
            let right = area.left
                + crate::ui::editor_preferences::current().padding_left
                + crate::ui::document::content_width(area);
            app.on_click(right - 70.0, y);
            assert!(
                crate::ui::containers::scan(app.shell.active().unwrap().buffer().unwrap().text())
                    [0]
                .open
            );
            app.paint(HWND::default()).unwrap();
            app.on_click(right - 18.0, y);
            assert!(app.menu.is_some());
            assert!(
                crate::ui::containers::scan(app.shell.active().unwrap().buffer().unwrap().text())
                    [0]
                .open
            );
        });
    }

    #[test]
    fn plain_paste_is_literal_in_rendered_document_but_code_keeps_raw_bytes() {
        with_editor("", |app| {
            assert!(app.insert_clipboard_content("# 标题\n**文字**", true));
            let buffer = app.shell.active().unwrap().buffer().unwrap();
            assert!(!matches!(
                crate::ui::document::parse_ranged(buffer.text()).blocks[0].block,
                crate::ui::document::Block::Heading { .. }
            ));
            assert!(buffer.text().contains("文字"));
        });
        with_editor("```text\n\n```", |app| {
            app.shell.active_buffer_mut().unwrap().set_cursor(8, false);
            assert!(app.insert_clipboard_content("**literal**", true));
            assert!(app
                .shell
                .active()
                .unwrap()
                .buffer()
                .unwrap()
                .text()
                .contains("\n**literal**\n"));
        });
    }

    #[test]
    fn source_and_form_paste_are_plain_and_menu_exposes_both_actions() {
        with_editor("", |app| {
            if let TabKind::File { source_mode, .. } = &mut app.shell.active_mut().unwrap().kind {
                *source_mode = true;
            }
            app.insert_clipboard_content("**source**", true);
            assert_eq!(
                app.shell.active().unwrap().buffer().unwrap().text(),
                "**source**"
            );
            app.focus = Focus::AiInput;
            app.insert_clipboard_content("# 原样\n第二行", true);
            assert_eq!(app.ai.panel.input.text(), "# 原样\n第二行");
        });
        let items = App::clipboard_menu_items();
        assert!(items
            .iter()
            .any(|item| matches!(item.action, MenuAction::EditorPaste)));
        assert!(items
            .iter()
            .any(|item| matches!(item.action, MenuAction::EditorPastePlain)));
    }

    #[test]
    fn one_character_paste_does_not_merge_with_adjacent_typing() {
        with_editor("", |app| {
            app.shell.active_buffer_mut().unwrap().insert("a");
            app.insert_clipboard_content("b", true);
            app.shell.active_buffer_mut().unwrap().insert("c");
            let buffer = app.shell.active_buffer_mut().unwrap();
            assert!(buffer.undo());
            assert_eq!(buffer.text(), "ab");
            assert!(buffer.undo());
            assert_eq!(buffer.text(), "a");
        });
    }

    #[test]
    fn rich_paste_renders_with_the_native_document_renderer() {
        with_editor("", |app| {
            let html = r#"<h2>格式粘贴验证</h2><p>普通文字 <strong>粗体</strong> <em>斜体</em> <u>下划线</u> <span style="color:#de4a49">颜色</span></p><ul><li>保留列表结构</li><li>中文与 emoji 😀</li></ul><pre><code class="language-rust">let message = "保留代码缩进";
    println!("{}", message);</code></pre><table><tr><th>项目</th><th>状态</th></tr><tr><td>格式粘贴</td><td><strong>保留格式</strong></td></tr></table>"#;
            let source = crate::ui::paste::from_html(html).unwrap();
            assert!(app.insert_formatted_clipboard_content(&source));
            let snapshot = app.renderer.prepare_snapshot(1200, 1050, 96.0).unwrap();
            app.paint(HWND::default()).unwrap();
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/paste-block-acceptance/rich-paste.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            app.renderer.save_snapshot(&snapshot, &path).unwrap();
        });
    }

    #[test]
    fn formatted_block_paste_keeps_surrounding_paragraphs_and_one_undo() {
        with_editor("前文后文", |app| {
            app.shell
                .active_buffer_mut()
                .unwrap()
                .set_cursor("前文".len(), false);
            let source = crate::ui::paste::from_html(
                "<pre><code class='language-rust'>let x = 1;</code></pre>",
            )
            .unwrap();
            assert!(app.insert_formatted_clipboard_content(&source));
            let buffer = app.shell.active_buffer_mut().unwrap();
            let parsed = crate::ui::document::parse_ranged(buffer.text());
            assert!(parsed.blocks.iter().any(|block|matches!(&block.block,crate::ui::document::Block::Code { lang, lines } if lang=="rust" && lines==&vec!["let x = 1;".to_owned()])));
            assert!(buffer.text().starts_with("前文\n\n"));
            assert!(buffer.text().ends_with("\n\n后文"));
            assert!(buffer.undo());
            assert_eq!(buffer.text(), "前文后文");
        });
    }

    #[test]
    fn table_context_paste_targets_the_clicked_cell_and_undo_restores_document() {
        let original = "| 名称 | 状态 |\n| --- | --- |\n| 文档 | 待办 |\n";
        with_editor(original, |app| {
            let _snapshot = app.renderer.prepare_snapshot(1200, 900, 96.0).unwrap();
            app.paint(HWND::default()).unwrap();
            let cell = app
                .doc
                .table_cells(app.editor_area, original, app.shell.active_scroll())
                .pop()
                .unwrap();
            app.on_right_click(cell.rect.right - 8.0, cell.rect.top + 12.0);
            assert_eq!(app.focus, Focus::TableCell);
            assert!(app
                .menu
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|item| matches!(item.action, MenuAction::EditorPastePlain)));
            app.table_editing
                .as_mut()
                .unwrap()
                .field
                .buffer
                .select_all();
            assert!(app.insert_clipboard_content("**原样** | $x$", true));
            assert!(app.commit_table_cell());
            let buffer = app.shell.active_buffer_mut().unwrap();
            let parsed = crate::ui::document::parse_ranged(buffer.text());
            let crate::ui::document::Block::Table { rows, .. } = &parsed.blocks[0].block else {
                panic!("paste must preserve table structure")
            };
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[1].len(), 2);
            let visible = crate::ui::text::parse_inline(&rows[1][1])
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>();
            assert_eq!(visible, "**原样** | $x$");
            assert!(buffer.undo());
            assert_eq!(buffer.text(), original);
        });
    }

    #[test]
    fn native_context_menus_hit_real_function_blocks_and_plain_paste() {
        with_editor("```rust\nlet n = 1;\n```\n\n---\n\n$$\nx^2\n$$\n", |app| {
            let snapshot = app.renderer.prepare_snapshot(1200, 900, 96.0).unwrap();
            app.state.sidebar_visible = false;
            app.state.ai_panel_open = false;
            app.paint(HWND::default()).unwrap();
            let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/paste-block-acceptance/native-context-menu.png");
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            app.renderer.save_snapshot(&snapshot, &target).unwrap();
            let mut hits = std::collections::BTreeMap::new();
            let area = app.editor_area;
            for y in (area.top as usize..area.bottom as usize).step_by(3) {
                let x = area.left + 100.0;
                if let Some((_, start, false)) =
                    app.doc
                        .block_handle(area, app.shell.active_scroll(), x, y as f32)
                {
                    hits.entry(start).or_insert((x, y as f32));
                }
            }
            let starts = crate::ui::document::parse_ranged(
                app.shell.active().unwrap().buffer().unwrap().text(),
            )
            .blocks
            .into_iter()
            .filter(|block| {
                matches!(
                    block.block,
                    crate::ui::document::Block::Code { .. }
                        | crate::ui::document::Block::Divider
                        | crate::ui::document::Block::Math(_)
                )
            })
            .map(|block| block.start)
            .collect::<Vec<_>>();
            assert_eq!(starts.len(), 3);
            for start in starts {
                let (x,y)=*hits.get(&start).unwrap_or_else(||panic!("function block {start} must be visible and hittable; actual {hits:?}; area {area:?}"));
                app.shell.active_buffer_mut().unwrap().set_cursor(0, false);
                app.menu = None;
                app.on_right_click(x, y);
                let menu = app
                    .menu
                    .as_ref()
                    .expect("right-click must open a native menu");
                assert!(menu
                    .items
                    .iter()
                    .any(|item| matches!(item.action,MenuAction::Block(s,false) if s==start)));
                assert!(menu
                    .items
                    .iter()
                    .any(|item| matches!(item.action,MenuAction::Block(s,true) if s==start)));
            }
            app.menu = None;
            app.shell.active_buffer_mut().unwrap().select_all();
            app.on_right_click(area.right - 30.0, area.bottom - 30.0);
            assert!(app
                .menu
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|item| matches!(item.action, MenuAction::EditorPastePlain)));
            app.paint(HWND::default()).unwrap();
            app.renderer.save_snapshot(&snapshot, &target).unwrap();
        });
    }
}
