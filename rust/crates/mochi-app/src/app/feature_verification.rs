//! 通过原生 App 事件执行的 Electron 格式隔离样例验证。
use super::*;
use crate::ui::{
    code_blocks, containers,
    document::{self, Block},
};
use anyhow::{ensure, Context};

impl App {
    fn parity_source(&self) -> &str {
        self.shell.active().unwrap().buffer().unwrap().text()
    }

    fn parity_open(&mut self, folder: &Path, name: &str, source: &str) -> anyhow::Result<PathBuf> {
        let path = folder.join(format!("{name}.md"));
        std::fs::write(&path, source)?;
        ensure!(self.shell.open_file(&path));
        self.menu = None;
        self.dialog = None;
        self.table_editing = None;
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.sync_state();
        self.paint(HWND::default())?;
        Ok(path)
    }

    fn parity_menu(&mut self, label: &str) -> anyhow::Result<()> {
        let menu = self.menu.as_ref().context("visible menu missing")?;
        let index = menu
            .items
            .iter()
            .position(|item| item.label == label)
            .with_context(|| format!("menu entry missing: {label}"))?;
        let x = menu.rect.left + 30.0;
        let y = (menu.rect.top as i32..menu.rect.bottom as i32)
            .map(|y| y as f32)
            .find(|y| menu.hit(x, *y) == Some(index))
            .context("menu entry outside viewport")?;
        self.on_click(x, y);
        Ok(())
    }

    fn parity_dialog(&mut self, value: &str) -> anyhow::Result<()> {
        let dialog = self.dialog.as_mut().context("dialog missing")?;
        dialog
            .field
            .as_mut()
            .context("dialog input missing")?
            .set_text(value);
        let action = dialog
            .buttons
            .last()
            .context("dialog submit missing")?
            .action
            .clone();
        self.run_dialog_action(action);
        ensure!(self.dialog.is_none(), "dialog did not submit");
        Ok(())
    }

    fn parity_container_point(&self, start: usize, toggle: bool) -> anyhow::Result<(f32, f32)> {
        let x = self.editor_area.left
            + crate::ui::editor_preferences::current().padding_left
            + if toggle { 14.0 } else { 70.0 };
        let y = (self.editor_area.top as i32..self.editor_area.bottom as i32)
            .map(|y| y as f32)
            .find(|y| {
                self.doc
                    .container_at(self.editor_area, self.shell.active_scroll(), x, *y)
                    == Some((start, toggle))
            })
            .context("container header hit area missing")?;
        Ok((x, y))
    }

    fn parity_reopen(&mut self, path: &Path) -> anyhow::Result<()> {
        let expected = self.parity_source().to_owned();
        ensure!(self.save_active());
        let saved_path = if mochi_core::document_format::requires_mochi_format(&expected) {
            let upgraded = path.with_extension("mc");
            ensure!(!path.exists(), "upgraded Markdown source still exists");
            upgraded
        } else {
            path.to_path_buf()
        };
        ensure!(self.active_file_path().as_deref() == Some(saved_path.as_path()));
        ensure!(std::fs::read_to_string(&saved_path)? == expected);
        self.shell.close_tab(self.shell.active_tab().unwrap());
        ensure!(self.shell.open_file(&saved_path));
        self.sync_state();
        self.paint(HWND::default())?;
        ensure!(
            self.parity_source() == expected,
            "reopen changed persisted source"
        );
        Ok(())
    }

    pub(super) fn verify_feature_parity(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        let raw = "# 从 Electron 打开的笔记\n\n:::mochi-highlight color=\"blue\" title=\"提示\"\n这里的 **加粗内容** 可以直接编辑。\n:::";
        let path = self.parity_open(folder, "高亮与折叠", raw)?;
        self.on_toolbar_click(toolbar::Hit::Insert);
        for label in [
            "公式",
            "代码块",
            "表格",
            "任务列表",
            "引用",
            "高亮块",
            "折叠块",
            "分割线",
            "图片",
        ] {
            ensure!(
                self.menu
                    .as_ref()
                    .unwrap()
                    .items
                    .iter()
                    .any(|item| item.label == label),
                "insert item missing: {label}"
            );
        }
        self.verify_frame(output, "insert-menu", snapshot)?;
        self.menu = None;
        let start = raw.find(":::").unwrap();
        let (x, y) = self.parity_container_point(start, false)?;
        self.on_right_click(x, y);
        self.parity_menu("绿色")?;
        ensure!(containers::scan(self.parity_source())[0].color == "green");
        self.on_right_click(x, y);
        self.parity_menu("编辑标题")?;
        self.parity_dialog("保存并重开也保留的提示")?;
        ensure!(containers::scan(self.parity_source())[0].title == "保存并重开也保留的提示");
        let at = self.parity_source().find("加粗内容").unwrap();
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(at, false);
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.after_doc_edit(true);
        self.on_ime_commit("中文");
        ensure!(self.parity_source().contains("**中文加粗内容**"));
        self.parity_reopen(&path)?;
        self.verify_frame(output, "highlight-reopened", snapshot)?;

        let raw = "# 可折叠内容\n\n<details>\n<summary>点击箭头展开</summary>\n\n隐藏的正文\n\n</details>\n\n外部段落";
        let path = self.parity_open(folder, "折叠块", raw)?;
        let start = raw.find("<details>").unwrap();
        self.verify_frame(output, "details-closed", snapshot)?;
        let (x, y) = self.parity_container_point(start, true)?;
        self.on_click(x, y);
        ensure!(
            containers::scan(self.parity_source())[0].open,
            "details arrow did not expand"
        );
        self.verify_frame(output, "details-open", snapshot)?;
        self.parity_reopen(&path)?;
        ensure!(containers::scan(self.parity_source())[0].open);
        // 代码块属性必须能与 Electron 读到的同一注释往返转换。
        let raw = "# 代码块属性\n\n<!-- mochi-code-block title=\"Electron 标题\" collapsed=\"true\" -->\n```rust\nfn main() { println!(\"你好\"); }\n```\n";
        let path = self.parity_open(folder, "代码属性", raw)?;
        let start = raw.find("```rust").unwrap();
        let x =
            self.editor_area.left + crate::ui::editor_preferences::current().padding_left + 18.0;
        let y = (self.editor_area.top as i32..self.editor_area.bottom as i32).map(|y|y as f32)
            .find(|y| matches!(self.doc.code_header_at(self.editor_area,0.0,x,*y),Some(code_blocks::Hit::Collapse(s)) if s==start)).context("code collapse target missing")?;
        self.on_click(x, y);
        let start = self.parity_source().find("```rust").unwrap();
        ensure!(
            !code_blocks::metadata(self.parity_source(), start)
                .1
                .collapsed
        );
        self.on_click(x + 90.0, y);
        self.parity_dialog("Rust 与 Electron 共用的标题")?;
        self.parity_reopen(&path)?;
        let start = self.parity_source().find("```rust").unwrap();
        ensure!(
            code_blocks::metadata(self.parity_source(), start).1.title
                == "Rust 与 Electron 共用的标题"
        );
        self.verify_frame(output, "code-reopened", snapshot)?;
        let before = self.parity_source().to_owned();
        self.edit_block(start, false);
        ensure!(!self.parity_source().contains("mochi-code-block"));
        self.on_shortcut(HWND::default(), 0x5a, false, true);
        ensure!(self.parity_source() == before);

        let path = self.parity_open(folder, "表格与公式", "# 表格结构编辑\n\n")?;
        let end = self.parity_source().len();
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(end, false);
        self.after_doc_edit(true);
        self.on_toolbar_click(toolbar::Hit::Insert);
        self.parity_menu("表格")?;
        let cell = self
            .table_picker
            .as_ref()
            .context("table picker missing")?
            .cell_rect(4, 5);
        self.on_mouse_move(cell.left + 8.0, cell.top + 8.0);
        self.verify_frame(output, "table-picker", snapshot)?;
        self.on_click(cell.left + 8.0, cell.top + 8.0);
        let table = document::parse_ranged(self.parity_source())
            .blocks
            .into_iter()
            .find_map(|b| {
                if let Block::Table { rows, .. } = b.block {
                    Some((rows[0].len(), rows.len()))
                } else {
                    None
                }
            })
            .context("inserted table missing")?;
        ensure!(table == (5, 4), "picker dimensions disagree: {table:?}");
        self.paint(HWND::default())?;
        let cell = self
            .doc
            .table_cells(
                self.editor_area,
                self.parity_source(),
                self.shell.active_scroll(),
            )
            .get(7)
            .context("table body cell missing")?
            .clone();
        self.on_right_click(cell.rect.left + 8.0, cell.rect.top + 8.0);
        self.verify_frame(output, "table-context", snapshot)?;
        let label = table_edit::ACTIONS
            .iter()
            .find(|(_, a)| *a == table_edit::Action::ColumnAfter)
            .unwrap()
            .0;
        self.parity_menu(label)?;
        ensure!(document::parse_ranged(self.parity_source())
            .blocks
            .iter()
            .any(|b| matches!(&b.block,Block::Table{rows,..} if rows[0].len()==6)));
        self.on_shortcut(HWND::default(), 0x5a, false, true);
        self.paint(HWND::default())?;
        let (_, track, _, _) = self
            .doc
            .table_scrollbars(self.editor_area, self.shell.active_scroll())
            .into_iter()
            .next()
            .context("wide table scrollbar missing")?;
        self.on_click(track.right - 2.0, track.top + 5.0);
        self.end_drag();
        self.paint(HWND::default())?;
        let last = self.doc.table_cells(
            self.editor_area,
            self.parity_source(),
            self.shell.active_scroll(),
        )[4]
        .clone();
        ensure!(
            last.rect.right <= self.editor_area.right,
            "last column is unreachable"
        );
        self.verify_frame(output, "table-scrolled", snapshot)?;
        let start = document::parse_ranged(self.parity_source())
            .blocks
            .iter()
            .find(|b| matches!(b.block, Block::Table { .. }))
            .unwrap()
            .start;
        self.edit_block(start, true);
        self.on_toolbar_click(toolbar::Hit::Insert);
        self.parity_menu("公式")?;
        self.parity_dialog("\\frac{1}{2}+x^2")?;
        ensure!(self.parity_source().contains("$\\frac{1}{2}+x^2$"));
        self.parity_reopen(&path)?;
        self.verify_frame(output, "table-and-formula", snapshot)?;

        // 生成的样例图是真实 PNG，由正式渲染器解码。
        let mut pattern = DrawList::new();
        pattern.rect(Rect::from_size(0.0, 0.0, 1200.0, 800.0), 0xeaf3ff);
        pattern.rounded_rect(Rect::from_size(150.0, 100.0, 900.0, 600.0), 60.0, 0x93c5fd);
        self.renderer.present(HWND::default(), 0xeaf3ff, &pattern)?;
        let fixture = folder.join("image-fixture.png");
        self.renderer.save_snapshot(snapshot, &fixture)?;
        let bytes = std::fs::read(&fixture)?;
        let path = self.parity_open(folder, "正文图片", "# 正文图片\n\n")?;
        let end = self.parity_source().len();
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(end, false);
        ensure!(self.paste_editor_image_with(|| Ok(Some(bytes))));
        let start = document::parse_ranged(self.parity_source())
            .blocks
            .iter()
            .find(|b| matches!(b.block, Block::Image { .. }))
            .context("pasted image missing")?
            .start;
        self.open_image_width(start);
        self.parity_dialog("50%")?;
        let image = document::parse_ranged(self.parity_source())
            .blocks
            .into_iter()
            .find_map(|b| {
                if let Block::Image { src, width, .. } = b.block {
                    Some((src, width))
                } else {
                    None
                }
            })
            .unwrap();
        ensure!(image.1.as_deref() == Some("50%"));
        ensure!(folder.join(&image.0).exists());
        self.paint(HWND::default())?;
        let before = self.parity_source().to_owned();
        let rect = self
            .doc
            .image_rect(self.editor_area, self.shell.active_scroll(), start)
            .context("image drag geometry missing")?;
        self.on_mouse_move(rect.right - 6.0, rect.bottom - 6.0);
        self.on_click(rect.right - 6.0, rect.bottom - 6.0);
        ensure!(self.is_dragging(), "image handle did not begin drag");
        self.on_mouse_move(rect.right + 24.0, rect.bottom - 6.0);
        self.on_mouse_move(rect.right + 54.0, rect.bottom - 6.0);
        self.end_drag();
        ensure!(
            self.parity_source() != before,
            "image drag did not save width"
        );
        self.on_shortcut(HWND::default(), 0x5a, false, true);
        ensure!(
            self.parity_source() == before,
            "image resize created multiple undo steps"
        );
        self.on_shortcut(HWND::default(), 0x5a, true, true);
        self.parity_reopen(&path)?;
        self.verify_frame(output, "image-reopened", snapshot)?;
        let rect = self
            .doc
            .image_rect(self.editor_area, self.shell.active_scroll(), start)
            .unwrap();
        self.on_double_click(rect.left + 12.0, rect.top + 12.0);
        ensure!(
            self.image_preview.is_some(),
            "double click did not preview image"
        );
        let preview_source = self.parity_source().to_owned();
        self.on_char('X');
        self.on_ime_composition("中文", 6);
        self.on_ime_commit("中文");
        self.on_right_click(rect.left + 12.0, rect.top + 12.0);
        ensure!(
            self.parity_source() == preview_source && self.menu.is_none(),
            "preview allowed editing behind its overlay"
        );
        self.verify_frame(output, "image-preview", snapshot)?;
        ensure!(self.on_edit_key(0x1b, false, false));
        ensure!(self.image_preview.is_none());

        let target = folder.join("链接目标.md");
        std::fs::write(&target, "# 链接目标\n")?;
        self.shell
            .workspace()
            .unwrap()
            .index
            .index_single_file(&target)?;
        let path = self.parity_open(folder, "Wiki 链接补全", "输入链接：")?;
        let end = self.parity_source().len();
        self.shell
            .active_buffer_mut()
            .unwrap()
            .set_cursor(end, false);
        for ch in "[[链接".chars() {
            self.on_char(ch);
        }
        self.paint(HWND::default())?;
        let items = &self
            .wiki_suggestion
            .as_ref()
            .context("Wiki suggestion missing")?
            .menu
            .items;
        ensure!(
            items.iter().any(|i| i.label == "链接目标"),
            "indexed target missing from Wiki menu"
        );
        let index = items.iter().position(|i| i.label == "链接目标").unwrap();
        self.verify_frame(output, "wiki-menu", snapshot)?;
        self.wiki_suggestion.as_mut().unwrap().menu.hover = Some(index);
        ensure!(self.on_edit_key(0x0d, false, false));
        ensure!(
            self.parity_source() == "输入链接：[[链接目标]]",
            "Wiki Enter inserted a newline or lost target"
        );
        self.parity_reopen(&path)?;
        self.verify_frame(output, "wiki-reopened", snapshot)?;

        let raw = "# 行内评论\n\n这段 **选中的文字** 带有评论。";
        let path = self.parity_open(folder, "行内评论", raw)?;
        let start = raw.find("选中的文字").unwrap();
        let buffer = self.shell.active_buffer_mut().unwrap();
        buffer.set_cursor(start, false);
        buffer.set_cursor(start + "选中的文字".len(), true);
        self.after_doc_edit(true);
        self.begin_selection_comment();
        let pending = self
            .panels
            .comments
            .pending
            .as_mut()
            .context("inline comment composer missing")?;
        ensure!(pending
            .anchor
            .as_ref()
            .is_some_and(|a| a.selected_text == "选中的文字"));
        pending.field.set_text("这是一条随正文定位的评论。");
        self.submit_comment();
        ensure!(
            sidecars::load_comments(&path.to_string_lossy())
                .comments
                .len()
                == 1
        );
        self.state.ai_panel_open = false;
        self.paint(HWND::default())?;
        let (bubble, _) = self
            .comment_bubbles
            .first()
            .context("comment bubble missing")?
            .clone();
        self.verify_frame(output, "comment-mark", snapshot)?;
        self.on_click(bubble.left + 8.0, bubble.top + 8.0);
        ensure!(
            self.state.right_panel == RightPanel::Comments && self.state.ai_panel_open,
            "bubble did not open comments"
        );
        self.verify_frame(output, "comment-panel", snapshot)?;
        self.state.ai_panel_open = false;
        self.parity_reopen(&path)?;
        ensure!(self.parity_source() == raw, "comment changed note Markdown");

        self.open_settings("plugins");
        ensure!(
            matches!(&self.shell.active().context("settings tab missing")?.kind, TabKind::Settings{tab,..} if tab=="plugins"),
            "plugin settings did not open"
        );
        self.verify_frame(output, "plugins", snapshot)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({"passed":true,
            "checks":["visible insert entries","container color and title","container IME","details toggle","code metadata","code deletion undo","4x5 picker","column insertion undo","wide table scrolling","formula insertion","image paste and width","image drag undo","image preview","Wiki completion","inline comment sidecar","comment marks and bubble","save and reopen","plugin settings"]}))?,
        )?;
        Ok(())
    }
}
