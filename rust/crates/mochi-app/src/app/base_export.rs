//! 管理数据表导出任务的状态，并生成 XLSX 文件。
use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Default)]
pub(super) struct State {
    job: Option<Receiver<std::result::Result<PathBuf, String>>>,
}

impl State {
    pub fn pending(&self) -> bool {
        self.job.is_some()
    }
}

impl App {
    pub(super) fn export_base_xlsx(&mut self) {
        if self.base_export.pending() {
            self.show_global_notice("正在导出，请稍候");
            return;
        }
        let Some((_, viewer::Content::Base(state))) = self.viewer_tab() else {
            return;
        };
        let title: String = format!("{}-{}", state.table().name, state.view().name)
            .chars()
            .map(|c| {
                if "<>:\"/\\|?*".contains(c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .take(100)
            .collect();
        let Some(path) =
            platform::save_xlsx_file(HWND(self.hwnd_raw as *mut _), &format!("{title}.xlsx"))
        else {
            return;
        };
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
        {
            self.show_global_notice("请选择 .xlsx 文件名，不会覆盖其他格式的文件");
            return;
        }
        let Some((_, viewer::Content::Base(state))) = self.viewer_tab() else {
            return;
        };
        let table = state.table().clone();
        let view = state.view().clone();
        let query = state.query.clone();
        let located = state.located.is_some();
        let (tx, rx) = mpsc::channel();
        self.base_export.job = Some(rx);
        self.show_global_notice("正在后台导出当前视图…");
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let fields = if located {
                    (0..table.fields.len()).collect()
                } else {
                    mochi_core::base::visible_field_indices(&table, &view)
                };
                let indices: std::collections::HashMap<_, _> = table
                    .records
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (r.id.as_str(), i))
                    .collect();
                let rows: Vec<usize> = if located {
                    (0..table.records.len()).collect()
                } else {
                    mochi_core::base::get_view_records(&table, &view, &query)
                        .iter()
                        .filter_map(|record| indices.get(record.id.as_str()).copied())
                        .collect()
                };
                let bytes =
                    mochi_core::base_export::export_view_xlsx(&table, &view, &rows, &fields)?;
                mochi_core::files::FileService::new().write_bytes_safe(&path, &bytes)?;
                Ok(())
            })()
            .map(|_| path)
            .map_err(|error| format!("{error:#}"));
            let _ = tx.send(result);
        });
    }

    pub(super) fn poll_base_export(&mut self) {
        let Some(job) = &self.base_export.job else {
            return;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("导出线程异常退出，目标文件未确认生成".into()),
        };
        self.base_export.job = None;
        match result {
            Ok(path) => self.show_global_notice(&format!("XLSX 已导出：{}", path.display())),
            Err(error) => self.show_global_notice(&format!("XLSX 导出失败：{error}")),
        }
    }
}
