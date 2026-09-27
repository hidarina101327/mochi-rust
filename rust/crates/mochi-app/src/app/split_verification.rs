//! 先分屏同一文档，再替换左侧文档，覆盖共享缓冲区到独立面板的切换。
use super::*;
use anyhow::{ensure, Context};

impl App {
    fn split_verify_frame(
        &mut self,
        output: &Path,
        name: &str,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        self.paint(HWND::default())?;
        let target = output.with_file_name(format!(
            "{}-{name}.png",
            output.file_stem().unwrap_or_default().to_string_lossy()
        ));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.renderer.save_snapshot(snapshot, &target)?;
        Ok(())
    }

    pub(super) fn verify_split_right(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
    ) -> anyhow::Result<()> {
        let left_path = folder.join("拆分左文档.md");
        let right_path = folder.join("拆分右文档.md");
        let left_source = format!(
            "# 左侧文档\n\n{}",
            (0..180)
                .map(|i| format!("左侧滚动行 {i}：拆分后仍由同一个缓冲区提供。\n"))
                .collect::<String>()
        );
        let right_source = format!(
            "# 右侧文档\n\n{}",
            (0..180)
                .map(|i| format!("右侧文档行 {i}：切换标签不应改变另一栏。\n"))
                .collect::<String>()
        );
        std::fs::write(&left_path, &left_source)?;
        std::fs::write(&right_path, &right_source)?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 强制追加让这个隔离夹具不受用户 openFileBehavior 偏好影响。
        // 真实的 UI 路径在下面通过点击标签栏的分屏按钮和第二个页签来覆盖。
        ensure!(
            self.shell.open_file_with_mode(&left_path, true),
            "left fixture did not open"
        );
        ensure!(
            self.shell.open_file_with_mode(&right_path, true),
            "right fixture did not open"
        );
        let left_index = self
            .shell
            .tabs()
            .iter()
            .position(|tab| tab.path() == Some(left_path.as_path()))
            .context("left fixture tab missing")?;
        let right_index = self
            .shell
            .tabs()
            .iter()
            .position(|tab| tab.path() == Some(right_path.as_path()))
            .context("right fixture tab missing")?;
        self.shell.select_tab(left_index);
        self.state.view = WorkspaceView::Editor;
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.sync_state();
        self.paint(HWND::default())?;

        // Electron 固定的标签栏按钮作用于活动页签。从左侧文档开始，
        // 两个分屏就都刻意显示同一个共享 shell 缓冲，且新建的右侧分屏
        // 获得焦点。
        let chrome = self.build_chrome();
        let tab_bar = chrome.tree.rect(chrome.tab_bar);
        let split_button = crate::ui::tab_bar::split_button_rect(tab_bar);
        ensure!(!split_button.is_empty(), "split button geometry missing");
        self.on_click(
            (split_button.left + split_button.right) / 2.0,
            (split_button.top + split_button.bottom) / 2.0,
        );
        self.paint(HWND::default())?;
        ensure!(
            self.split.right,
            "split button did not focus the right pane"
        );
        ensure!(
            self.active_file_path().as_deref() == Some(left_path.as_path()),
            "split button changed the active document"
        );
        ensure!(
            self.split.other.as_deref() == Some(left_path.as_path()),
            "right pane did not retain the active document"
        );
        ensure!(
            self.shell
                .tabs()
                .iter()
                .filter(|tab| tab.path() == Some(left_path.as_path()))
                .count()
                == 1,
            "same-document split created a second shell buffer"
        );

        // 在另一侧持有焦点时修改活动侧，改动必须仍然可见——
        // 证明两个分屏解析到的是同一个 OpenTab 缓冲。
        let same_doc_marker = "\n同一文档双栏共享缓冲区\n";
        {
            let buffer = self
                .shell
                .active_buffer_mut()
                .context("same-document split has no active buffer")?;
            buffer.set_cursor(buffer.text().len(), false);
            buffer.insert(same_doc_marker);
        }
        let expected_left = format!("{left_source}{same_doc_marker}");
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text() == expected_left),
            "same-document edit was not kept in the shared buffer"
        );
        self.split_verify_frame(output, "same-document-right-focused", snapshot)?;

        // 非活动的左侧头部是分屏焦点的交互入口。点击它交换分屏归属，
        // 文档和缓冲保持不变。
        let left_header = self.split.header;
        ensure!(
            !left_header.is_empty(),
            "inactive left header geometry missing"
        );
        self.on_click(
            (left_header.left + left_header.right) / 2.0,
            (left_header.top + left_header.bottom) / 2.0,
        );
        self.paint(HWND::default())?;
        ensure!(!self.split.right, "left header did not focus the left pane");
        ensure!(
            self.active_file_path().as_deref() == Some(left_path.as_path()),
            "left header changed the document"
        );
        ensure!(
            self.shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.text() == expected_left),
            "shared buffer edit disappeared after pane focus"
        );

        // 在使用全局页签栏之前，让两个视图的滚动位置明显不同。
        // 长夹具让两个位置都能在绘制期钳制之后保留下来。
        self.shell.set_active_scroll(36.0);
        self.split.scroll = 84.0;
        self.paint(HWND::default())?;
        let left_scroll = self.shell.active_scroll();
        let right_scroll = self.split.scroll;
        ensure!(
            (left_scroll - right_scroll).abs() > 1.0,
            "split panes do not retain independent scroll positions"
        );

        // 通过 App::on_click 使用的同一投影几何点击第二个页签。
        // 由于左分屏当前持有焦点，这只会改变左侧文档；
        // 右分屏保持为编辑过的左侧文档。
        let chrome = self.build_chrome();
        let tab_bar = chrome.tree.rect(chrome.tab_bar);
        let content_area = crate::ui::tab_bar::tabs_content_area(tab_bar);
        let tabs = self.tab_projection();
        let tab_rect = crate::ui::tab_bar::layout(
            Rect::new(
                content_area.left - self.tab_scroll,
                content_area.top,
                content_area.right,
                content_area.bottom,
            ),
            &tabs,
            self.state.compact_tab_bar,
        )
        .into_iter()
        .find(|tab| tab.index == right_index)
        .context("right fixture tab geometry missing")?;
        let tab_x = (tab_rect.rect.left + tab_rect.rect.right) / 2.0;
        let tab_y = (tab_rect.rect.top + tab_rect.rect.bottom) / 2.0;
        ensure!(
            crate::ui::tab_bar::hit_scrolled(
                content_area,
                &tabs,
                self.state.compact_tab_bar,
                self.tab_scroll,
                tab_x,
                tab_y,
            ) == Some(crate::ui::tab_bar::Hit::Select(right_index)),
            "second fixture tab is not clickable"
        );
        self.on_click(tab_x, tab_y);
        self.paint(HWND::default())?;
        ensure!(
            self.active_file_path().as_deref() == Some(right_path.as_path()),
            "second tab did not become the left document"
        );
        ensure!(
            !self.split.right,
            "second tab unexpectedly moved to the right pane"
        );
        ensure!(
            self.split.other.as_deref() == Some(left_path.as_path()),
            "switching the left tab lost the right document"
        );
        ensure!(
            (self.shell.active_scroll() - self.split.scroll).abs() > 1.0,
            "switching tabs merged the two pane scroll positions"
        );

        // 在分隔条扩大的命中区内拖动它，验证存储的比例发生变化
        // 且始终在 Electron 的范围之内。
        let divider = self.split.divider_hit;
        ensure!(!divider.is_empty(), "split divider hit geometry missing");
        let before_ratio = self.split.ratio;
        let full = self.split.full;
        self.on_click(
            (divider.left + divider.right) / 2.0,
            (divider.top + divider.bottom) / 2.0,
        );
        self.on_mouse_move(
            full.left + full.width() * 0.64,
            (full.top + full.bottom) / 2.0,
        );
        self.end_drag();
        self.paint(HWND::default())?;
        ensure!(
            (self.split.ratio - before_ratio).abs() > 0.01,
            "divider drag did not update split ratio"
        );
        ensure!(
            (0.2..=0.8).contains(&self.split.ratio),
            "split ratio escaped its persisted bounds"
        );
        self.split_verify_frame(output, "two-documents", snapshot)?;

        // 这种状态下关闭按钮属于非活动的右侧分屏。关闭它必须让两个
        // shell 页签保持完好、左侧文件保持活动，与 Electron 的
        // closeSplitView 一致。
        let close = self.split.close;
        ensure!(!close.is_empty(), "split close geometry missing");
        self.on_click(
            (close.left + close.right) / 2.0,
            (close.top + close.bottom) / 2.0,
        );
        ensure!(
            self.split.other.is_none(),
            "closing the split kept a right pane"
        );
        ensure!(
            self.active_file_path().as_deref() == Some(right_path.as_path()),
            "closing the split did not return focus to the left file"
        );
        ensure!(
            self.shell
                .tabs()
                .iter()
                .any(|tab| tab.path() == Some(left_path.as_path()))
                && self
                    .shell
                    .tabs()
                    .iter()
                    .any(|tab| tab.path() == Some(right_path.as_path())),
            "closing the split discarded one of the documents"
        );

        // 显式保存两个文件，再核对磁盘上的结果。这样即便隔离夹具中
        // 自动保存被禁用或延迟，最终的快照/报告依然有意义。
        let save_indices = self
            .shell
            .tabs()
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                (tab.path() == Some(left_path.as_path())
                    || tab.path() == Some(right_path.as_path()))
                .then_some(index)
            })
            .collect::<Vec<_>>();
        ensure!(save_indices.len() == 2, "dual-document save tabs missing");
        for index in save_indices {
            ensure!(
                self.shell.save_tab(index),
                "fixture document failed to save"
            );
        }
        ensure!(
            std::fs::read_to_string(&left_path)? == expected_left,
            "left document was lost"
        );
        ensure!(
            std::fs::read_to_string(&right_path)? == right_source,
            "right document was lost"
        );
        self.split_verify_frame(output, "closed-preserves-documents", snapshot)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "passed": true,
                "sameDocumentSharedBuffer": true,
                "rightPaneFocusedAfterButton": true,
                "leftTabSwitchedIndependently": true,
                "independentScroll": true,
                "dividerRatio": self.split.ratio,
                "closedPreservesDocuments": true,
                "left": left_path,
                "right": right_path,
            }))?,
        )?;
        Ok(())
    }
}
