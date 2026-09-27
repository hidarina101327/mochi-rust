//! 处理窗口关闭、尺寸变化、DPI 变化和初始大小。
use super::*;

impl App {
    pub fn prepare_close(&mut self) -> bool {
        if !self.console_jobs_ready_for_close() {
            return false;
        }
        if !self.desktop_flush_before_exit() {
            return false;
        }
        self.workflows_persist_draft();
        if !self.stop_commands_for_close() {
            return false;
        }
        if !self.commit_title() {
            return false;
        }
        self.editor_ai.invalidate();
        if !self.commit_table_cell() {
            return false;
        }
        if let Err(error) = self.shell.save_dirty_tabs() {
            self.state.status_text = format!("保存失败，窗口保持打开：{error}");
            return false;
        }
        self.sync_document_format_changes();
        self.ai_cancel();
        self.ai.memory_jobs.clear();
        self.save_notifications();
        if let Some(store) = &self.workflows.store {
            let _ = store.set_dirty_paths(&[]);
        }
        true
    }

    pub fn resize(&self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }

    /// 绘制、命中和输入法光标始终使用当前窗口的同一套 DIP 坐标。
    pub fn on_dpi_changed(&mut self, dpi_x: u32, dpi_y: u32) {
        self.renderer.set_window_dpi(dpi_x, dpi_y);
        self.doc.invalidate();
        self.source.invalidate();
        self.split.doc.invalidate();
        self.split.source.invalidate();
    }

    /// 首帧的 DPI 调整只做一次。返回 true 表示这次该做。
    pub fn claim_initial_size(&mut self) -> bool {
        if self.initial_sized {
            return false;
        }
        self.initial_sized = true;
        true
    }
}
