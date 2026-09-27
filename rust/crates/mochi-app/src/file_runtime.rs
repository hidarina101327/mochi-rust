//! 查看器/文件操作的后台任务；所有结果由 UI 线程消费。
use std::{
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};
pub enum Payload {
    Marketplace(crate::app::marketplace::Event),
    Imported {
        root: PathBuf,
        target: PathBuf,
        report: mochi_core::external_import::Report,
    },
    CommandFinished {
        root: PathBuf,
        request: crate::export_requests::Request,
        queue: std::sync::Arc<crate::export_requests::Queue>,
        error: Option<String>,
    },
    Image {
        url: String,
        result: Result<(Vec<u8>, (u32, u32)), String>,
    },
    Exported(PathBuf),
    Office(PathBuf),
    Sheet(mochi_core::office::Workbook),
    LinkCache(mochi_core::link_files::Cache),
    Created(PathBuf),
}
pub struct Event {
    pub path: PathBuf,
    pub result: Result<Payload, String>,
}
pub struct Jobs {
    tx: Sender<Event>,
    rx: Receiver<Event>,
}
impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx }
    }
}
impl Jobs {
    pub fn submit(
        &self,
        path: PathBuf,
        hwnd: isize,
        work: impl FnOnce() -> anyhow::Result<Payload> + Send + 'static,
    ) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = work().map_err(|e| e.to_string());
            if tx.send(Event { path, result }).is_ok() {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        crate::platform::WM_APP_FILE_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }
    pub fn take(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}
