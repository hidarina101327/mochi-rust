//! 后台读取链接；每次替换接收端，过期工作区/标签的结果无法覆盖当前状态。
use crate::ui::backlinks::Data;
use mochi_core::metadata_index::MetadataIndexService;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{channel, Receiver},
    Arc,
};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

#[derive(Default)]
pub struct Job {
    rx: Option<Receiver<Result<Data, String>>>,
    cancel: Option<Arc<AtomicBool>>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Job {
    pub fn clear(&mut self) {
        self.rx = None;
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn start(&mut self, index: Arc<MetadataIndexService>, source: PathBuf, hwnd: isize) {
        self.clear();
        let mut i = 0;
        while i < self.workers.len() {
            if self.workers[i].is_finished() {
                let _ = self.workers.remove(i).join();
            } else {
                i += 1;
            }
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.workers.push(std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Data> {
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
                let backlinks = index.get_backlinks(&source)?;
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
                let outgoing = index.get_outgoing_links(&source)?;
                anyhow::ensure!(!cancel.load(Ordering::Relaxed), "读取已取消");
                Ok(Data {
                    backlinks,
                    outgoing,
                    mentions: index.get_unlinked_mentions(&source)?,
                })
            })()
            .map_err(|e| e.to_string());
            if !cancel.load(Ordering::Relaxed) && tx.send(result).is_ok() {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        crate::platform::WM_APP_LINKS_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        }));
    }
    pub fn take(&mut self) -> Option<Result<Data, String>> {
        let result = self.rx.as_ref()?.try_recv().ok()?;
        self.rx = None;
        Some(result)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.clear();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacing_receiver_discards_stale_completion() {
        let (old_tx, old_rx) = channel();
        let (new_tx, new_rx) = channel();
        let mut job = Job::default();
        job.rx = Some(old_rx);
        old_tx.send(Err("旧标签".into())).unwrap();
        job.rx = Some(new_rx);
        assert!(job.take().is_none());
        new_tx.send(Err("当前标签".into())).unwrap();
        assert!(matches!(job.take(), Some(Err(e)) if e == "当前标签"));
        assert!(job.take().is_none());
    }

    #[test]
    fn dropping_a_job_releases_the_index_before_returning() {
        let root = std::env::temp_dir().join(format!(
            "mochi-links-drop-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("note.md");
        std::fs::write(&source, "# note").unwrap();
        let index = Arc::new(MetadataIndexService::new(&root));
        index.open().unwrap();
        index.run_full_index().unwrap();
        let weak = Arc::downgrade(&index);
        let mut job = Job::default();
        job.start(index.clone(), source, 0);
        drop(index);
        drop(job);
        assert!(weak.upgrade().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
