//! 小记复用 Document 输入和保存路径，不另建编辑器。

use anyhow::{ensure, Context};
use std::path::Path;

use super::*;

impl App {
    /// 小记的真实离屏验收：所有输入都经过普通文档编辑器入口。
    ///
    /// `folder` 是快照创建的验收库目录，小记的约定位置仍然是工作区根；这
    /// 个参数只用于报告和断言，避免验收代码把固定入口误写进验收库。
    pub(super) fn verify_quick_note(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        use crate::ui::chrome::WorkspaceView;
        use crate::ui::navigation::{NavHit, NavItem};

        let root = self
            .shell
            .workspace()
            .context("小记验收缺少工作区")?
            .root
            .clone();
        ensure!(root != folder, "小记验收库不能替代工作区根");
        let markdown = root.join("小记.md");
        let compiled = root.join("小记.mc");
        let initial = "# 小记\n\n从导航打开后可以直接编辑。\n";
        std::fs::write(&markdown, initial)?;
        if compiled.exists() {
            std::fs::remove_file(&compiled)?;
        }

        // 先绘制导航，再点真实的小记导航项。导航处理会在切换视图前打开文件，
        // 因而首帧就拥有中央编辑器的文档标签。
        self.state.view = WorkspaceView::Editor;
        self.sync_state();
        self.paint(HWND::default())?;
        let nav = self
            .nav_layout
            .rect_of(NavHit::Item(NavItem::QuickNote))
            .context("小记导航项缺失")?;
        self.on_click((nav.left + nav.right) / 2.0, (nav.top + nav.bottom) / 2.0);
        ensure!(
            self.state.view == WorkspaceView::QuickNote,
            "点击小记导航未切换视图"
        );
        ensure!(self.active_file_path().as_deref() == Some(markdown.as_path()));
        ensure!(markdown.is_file() && !compiled.exists());
        ensure!(
            self.shell
                .tabs()
                .iter()
                .filter(|tab| tab.path() == Some(markdown.as_path()))
                .count()
                == 1,
            "小记导航重复打开了 Markdown 标签"
        );
        self.verify_frame(output, "opened", snapshot)?;

        // 通过渲染态正文的命中坐标点击，而不是直接把 editor_engaged 置为真。
        let at = initial.find("直接编辑").context("小记正文锚点缺失")?;
        {
            let buffer = self.shell.active_buffer_mut().context("小记缓冲区缺失")?;
            buffer.set_cursor(at, false);
        }
        self.after_doc_edit(true);
        let caret = {
            let scroll = self.shell.active_scroll();
            let buffer = self
                .shell
                .active()
                .and_then(|tab| tab.buffer())
                .context("小记光标缓冲区缺失")?;
            self.doc
                .caret_rect(self.editor_area, buffer, scroll)
                .context("小记正文光标缺失")?
        };
        self.on_click(caret.left, caret.top + caret.height() / 2.0);
        ensure!(
            self.focus == Focus::Main && self.editor_engaged,
            "点击小记正文没有获得编辑焦点"
        );
        self.verify_frame(output, "clicked", snapshot)?;

        ensure!(self.on_char('改'), "小记正文没有接收字符输入");
        let after_char = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .context("小记字符输入后缓冲区缺失")?
            .text()
            .to_owned();
        ensure!(after_char.contains('改'), "小记字符输入没有写入文档");

        self.on_ime_composition("中文输入", "中文输入".len());
        let composing = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .context("小记组合输入后缓冲区缺失")?;
        ensure!(
            composing.composition().map(|c| c.text.as_str()) == Some("中文输入"),
            "小记没有保留 IME 组合串"
        );
        ensure!(
            !composing.text().contains("中文输入"),
            "未提交的 IME 组合串写进了正文"
        );
        self.verify_frame(output, "ime-composition", snapshot)?;

        self.on_ime_commit("中文输入🙂");
        let edited = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .context("小记 IME 提交后缓冲区缺失")?
            .text()
            .to_owned();
        ensure!(
            edited.contains("中文输入🙂"),
            "小记没有写入已提交的中文输入"
        );
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.dirty()),
            "小记输入后没有标记为脏"
        );
        self.verify_frame(output, "edited", snapshot)?;

        // Ctrl+S、Ctrl+Shift+E 来回都走 App 的现有快捷键分派。
        ensure!(
            self.on_shortcut(HWND::default(), 0x53, false, true),
            "Ctrl+S 未被消费"
        );
        ensure!(
            std::fs::read_to_string(&markdown)? == edited,
            "Ctrl+S 没有保存小记正文"
        );
        self.verify_frame(output, "saved", snapshot)?;
        ensure!(
            self.on_shortcut(HWND::default(), 0x45, true, true),
            "Ctrl+Shift+E 未被消费"
        );
        ensure!(
            self.content() == MainContent::Source,
            "Ctrl+Shift+E 没有进入源码模式"
        );
        self.verify_frame(output, "source", snapshot)?;
        ensure!(
            self.on_shortcut(HWND::default(), 0x45, true, true),
            "Ctrl+Shift+E 返回未被消费"
        );
        ensure!(
            self.content() == MainContent::Document,
            "Ctrl+Shift+E 没有返回渲染编辑器"
        );
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text() == edited),
            "源码来回改写了小记正文"
        );
        self.verify_frame(output, "roundtrip", snapshot)?;

        // 使用已有颜色格式化路径触发 Markdown → MC 自动升级。
        let color_start = edited.find('改').context("小记颜色验收锚点缺失")?;
        let color_end = color_start + '改'.len_utf8();
        {
            let buffer = self
                .shell
                .active_buffer_mut()
                .context("颜色验收缓冲区缺失")?;
            buffer.set_cursor(color_start, false);
            buffer.set_cursor(color_end, true);
        }
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.apply_text_color(false, 0x3C5A78);
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text().contains("color: #3C5A78")),
            "颜色格式化没有写入小记源码"
        );
        self.verify_frame(output, "colored", snapshot)?;
        ensure!(
            self.on_shortcut(HWND::default(), 0x53, false, true),
            "颜色格式化后的 Ctrl+S 未被消费"
        );
        ensure!(
            compiled.is_file() && !markdown.exists(),
            "颜色保存没有把小记升级为 MC"
        );
        ensure!(self.active_file_path().as_deref() == Some(compiled.as_path()));
        ensure!(
            std::fs::read_to_string(&compiled)?.contains("3C5A78"),
            "升级后的 MC 没有保留颜色内容"
        );

        ensure!(
            self.on_shortcut(HWND::default(), 0x57, false, true),
            "Ctrl+W 未被消费"
        );
        ensure!(
            self.state.view == WorkspaceView::Editor,
            "关闭小记后没有返回编辑器视图"
        );
        ensure!(self.active_file_path().is_none(), "关闭小记后仍有活动标签");

        // 再次通过导航打开，ensure 必须复用 .mc，不能重建空的 .md。
        self.paint(HWND::default())?;
        let nav = self
            .nav_layout
            .rect_of(NavHit::Item(NavItem::QuickNote))
            .context("重开时小记导航项缺失")?;
        self.on_click((nav.left + nav.right) / 2.0, (nav.top + nav.bottom) / 2.0);
        ensure!(
            self.state.view == WorkspaceView::QuickNote,
            "重开小记导航未切换视图"
        );
        ensure!(self.active_file_path().as_deref() == Some(compiled.as_path()));
        ensure!(!markdown.exists(), "重开小记重新创建了 Markdown 文件");
        ensure!(
            self.shell
                .tabs()
                .iter()
                .filter(|tab| tab.path() == Some(compiled.as_path()))
                .count()
                == 1,
            "重开小记创建了第二个 MC 标签"
        );
        self.verify_frame(output, "reopened-mc", snapshot)?;
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text().contains("color: #3C5A78")),
            "重开小记没有恢复 MC 内容"
        );
        self.renderer.save_snapshot(snapshot, output)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed": true,
                "workspaceRoot": root,
                "fixtureFolder": folder,
                "clickedBody": true,
                "characterInput": "改",
                "imeCommit": "中文输入🙂",
                "sourceRoundtrip": true,
                "markdownSavedBeforeUpgrade": true,
                "upgradedToMc": true,
                "reopenedSameMc": true,
                "noSecondMarkdown": true,
            }))?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::{App, Focus};
    use crate::ui::chrome::WorkspaceView;
    use crate::ui::layout::Rect;
    use crate::ui::navigation::{self, NavHit, NavItem, NavModel};
    use mochi_core::settings::SettingsService;
    use windows::Win32::Foundation::HWND;

    fn temp_workspace(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "mochi-quick-note-{label}-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(10)
        ))
    }

    fn app_for(root: &std::path::Path) -> App {
        // Rust 测试运行器可能把这个模块放到一个之前没有任何 UI 测试初始化过的
        // 工作线程上。DirectWrite/D2D 的初始化是线程本地的。
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let settings = root.join("profile").join("settings.json");
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(settings)))).unwrap();
        app.open_workspace(HWND::default(), root.to_path_buf(), false)
            .unwrap();
        app
    }

    /// 只构建导航命中表。测试不需要真实的 HWND；所有坐标仍来自与绘制
    /// 相同的布局函数。
    fn prepare_navigation_hit(app: &mut App) -> Rect {
        let (types, libraries) = app.nav_types();
        let expanded = app.nav.expanded_types.clone();
        let model = NavModel {
            types: &types,
            libraries: &libraries,
            expanded_types: &expanded,
            creating: None,
            inbox_count: 0,
            collapsed: false,
            active_item: None,
            plugin_entries: &[],
            active_plugin: None,
            selected_library: None,
        };
        let area = Rect::new(0.0, 0.0, 280.0, 800.0);
        app.nav_layout = navigation::layout(&model, area, 0.0);
        app.nav_layout
            .rect_of(NavHit::Item(NavItem::QuickNote))
            .unwrap()
    }

    /// 准备真实的 Direct2D 目标，让 App::on_click/on_paint 走与生产窗口
    /// 相同的 chrome 命中测试和编辑器坐标。
    #[cfg(debug_assertions)]
    fn prepare_target(app: &mut App) {
        app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
    }

    #[test]
    fn quick_note_has_one_title_and_no_extra_workspace_header() {
        use crate::ui::draw::{DrawCmd, TextStyle};
        let root = temp_workspace("single-title");
        std::fs::create_dir_all(&root).unwrap();
        let original = "周四 操作系统\n\n周五 现代密码学 数据库\n\n周一 习概\n";
        let path = root.join("小记.md");
        std::fs::write(&path, original).unwrap();
        let mut app = app_for(&root);
        app.settings.set("app.editorLayout.cardEnabled", "false");
        app.load_chrome_settings();
        app.state.view = WorkspaceView::QuickNote;
        assert!(app.ensure_quick_note_tab());
        #[cfg(debug_assertions)]
        prepare_target(&mut app);
        app.paint(HWND::default()).unwrap();

        // 断言真实绘制输出，而不是隔离的标题辅助函数。这里曾经同时渲染过
        // 文档标题和工作区标题。
        let titles: Vec<_> = app
            .list
            .cmds()
            .iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::Text {
                    rect,
                    text,
                    style: TextStyle::DocumentTitle | TextStyle::Display,
                    ..
                } if text == "小记" => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(titles.len(), 1, "小记不应叠加第二套工作区标题");
        assert_eq!(
            app.editor_area.top, app.toolbar_area.bottom,
            "工具栏与编辑器之间不应再塞入 78px 页头"
        );
        assert_eq!(
            titles[0],
            app.doc
                .title_rect(app.editor_area, app.shell.active_scroll())
        );
        assert!(!app.list.cmds().iter().any(
            |cmd| matches!(cmd, DrawCmd::Text { text, .. } if text == "随手记下，慢慢整理。")
        ));
        assert_eq!(app.shell.active_buffer_mut().unwrap().text(), original);
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn quick_note_and_editor_share_geometry_in_rendered_and_source_modes() {
        let root = temp_workspace("shared-geometry");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("小记.md"), "第一行\n\n第二行\n\n第三行").unwrap();
        let mut app = app_for(&root);
        app.state.view = WorkspaceView::QuickNote;
        assert!(app.ensure_quick_note_tab());
        #[cfg(debug_assertions)]
        prepare_target(&mut app);
        let p = crate::ui::theme::configured_palette(false);
        for source_mode in [false, true] {
            if app.shell.active().unwrap().source_mode() != source_mode {
                app.toggle_source_mode();
            }
            // 保持外层 chrome 不变：切换导航入口不得在同一个编辑器面板内
            // 再插入一个偏移量。
            let chrome = app.build_chrome();
            app.state.view = WorkspaceView::Editor;
            app.list.clear();
            app.paint_main(&chrome, &p);
            let editor_area = app.editor_area;
            app.state.view = WorkspaceView::QuickNote;
            app.list.clear();
            app.paint_main(&chrome, &p);
            assert_eq!(app.editor_area, editor_area, "source_mode={source_mode}");
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn quick_note_navigation_click_typing_ime_and_reopen_use_document_editor() {
        let root = temp_workspace("editing");
        std::fs::create_dir_all(&root).unwrap();
        let mut app = app_for(&root);

        // 走真实的导航命中，而不是直接给视图赋值。App 侧的导航处理会在
        // 首帧之前打开 小记.md。
        let quick_note = prepare_navigation_hit(&mut app);
        app.on_navigation_click(quick_note.left + 8.0, quick_note.top + 8.0);
        assert_eq!(app.state.view, WorkspaceView::QuickNote);
        let markdown = root.join("小记.md");
        assert_eq!(app.active_file_path(), Some(markdown.clone()));
        assert_eq!(
            app.shell
                .tabs()
                .iter()
                .filter(|tab| tab.path() == Some(markdown.as_path()))
                .count(),
            1
        );

        #[cfg(debug_assertions)]
        prepare_target(&mut app);
        // 绘制确立 editor_area 与渲染文档几何。
        app.paint(HWND::default()).unwrap();
        assert_eq!(app.content(), super::super::MainContent::Document);

        let area = app.editor_area;
        assert!(!area.is_empty());
        let y = area.top + crate::ui::file_title::BODY_OFFSET + 12.0;
        app.on_click(area.left + 28.0, y);
        assert_eq!(app.focus, Focus::Main);
        assert!(app.editor_engaged);
        let clicked_cursor = app
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .map(|b| b.cursor())
            .unwrap();
        assert!(clicked_cursor > 0, "点击小记正文应将光标放进文档");

        assert!(app.on_char('改'));
        let after_char = app
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .unwrap()
            .text()
            .to_owned();
        assert!(after_char.contains('改'));

        // 组合串在缓冲区里可见，但在输入法提交之前不能进入源文本。
        app.on_ime_composition("输", "输".len());
        let buffer = app.shell.active().and_then(|tab| tab.buffer()).unwrap();
        assert_eq!(buffer.composition().map(|c| c.text.as_str()), Some("输"));
        assert!(!buffer.text().contains('输'));
        app.on_ime_commit("输入🙂");
        let edited = app
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .unwrap()
            .text()
            .to_owned();
        assert!(edited.contains("输入🙂"));
        assert!(app
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .unwrap()
            .dirty());

        assert!(app.shell.save_active(), "{}", app.shell.status());
        assert_eq!(std::fs::read_to_string(&markdown).unwrap(), edited);
        let tab = app.shell.active_tab().unwrap();
        app.shell.close_tab(tab);
        assert!(app.shell.open_file(&markdown));
        assert_eq!(
            app.shell
                .active()
                .and_then(|tab| tab.buffer())
                .unwrap()
                .text(),
            edited
        );
        assert!(!app
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .unwrap()
            .dirty());

        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn quick_note_reuses_the_same_mc_after_rich_save_and_does_not_create_md_again() {
        let root = temp_workspace("format-upgrade");
        std::fs::create_dir_all(&root).unwrap();
        let mut app = app_for(&root);
        app.state.view = WorkspaceView::QuickNote;
        app.ensure_quick_note_tab();
        let markdown = root.join("小记.md");
        assert_eq!(app.active_file_path(), Some(markdown.clone()));

        #[cfg(debug_assertions)]
        prepare_target(&mut app);
        app.paint(HWND::default()).unwrap();
        let area = app.editor_area;
        app.on_click(
            area.left + 28.0,
            area.top + crate::ui::file_title::BODY_OFFSET + 12.0,
        );
        let replacement = "<span style=\"color: red\">同一份小记</span>";
        let source_len = app.shell.active_buffer_mut().unwrap().text().len();
        app.shell
            .active_buffer_mut()
            .unwrap()
            .replace_range(0..source_len, replacement);
        // 富格式化是 shell 既有的 Markdown → MC 升级触发器。
        assert!(app.shell.save_active(), "{}", app.shell.status());
        let compiled = root.join("小记.mc");
        assert!(compiled.is_file());
        assert!(!markdown.exists());
        assert_eq!(app.active_file_path(), Some(compiled.clone()));

        // 从导航路由离开再回来时必须解析到已编译路径，而不是再新建一个
        // 空的 小记.md。
        app.shell.close_tab(app.shell.active_tab().unwrap());
        let quick_note = prepare_navigation_hit(&mut app);
        app.on_navigation_click(quick_note.left + 8.0, quick_note.top + 8.0);
        assert_eq!(app.active_file_path(), Some(compiled.clone()));
        assert!(!markdown.exists());
        assert_eq!(
            app.shell
                .tabs()
                .iter()
                .filter(|tab| tab.path() == Some(compiled.as_path()))
                .count(),
            1
        );
        assert_eq!(std::fs::read_to_string(compiled).unwrap(), replacement);

        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
