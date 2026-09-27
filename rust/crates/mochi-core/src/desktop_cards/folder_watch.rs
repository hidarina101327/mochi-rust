//! Non-recursive folder notifications. All watch registration and path I/O stay off the UI thread.
use super::folder::{self, FolderConfig};
use notify::{RecursiveMode, Watcher};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
};

pub struct FolderWatcher {
    tx: mpsc::Sender<Vec<FolderConfig>>,
    changed: Arc<AtomicBool>,
    paths: Vec<(String, String)>,
}
impl FolderWatcher {
    pub fn new() -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Vec<FolderConfig>>();
        let changed = Arc::new(AtomicBool::new(false));
        let flag = changed.clone();
        std::thread::Builder::new()
            .name("desktop-folder-watch".into())
            .spawn(move || {
                let callback = flag.clone();
                let Ok(mut watcher) =
                    notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                        if event.is_ok_and(|e| !matches!(e.kind, notify::EventKind::Access(_))) {
                            callback.store(true, Ordering::Release);
                        }
                    })
                else {
                    return;
                };
                let mut watched = Vec::<PathBuf>::new();
                while let Ok(mut configs) = rx.recv() {
                    while let Ok(next) = rx.try_recv() {
                        configs = next;
                    }
                    let mut next = Vec::new();
                    for config in configs.into_iter().take(super::MAX_CARDS) {
                        if let Ok(current) = folder::current_config(&config) {
                            let path = PathBuf::from(current.path);
                            if !next.contains(&path) {
                                next.push(path);
                            }
                        }
                    }
                    for path in watched.iter().filter(|p| !next.contains(p)) {
                        let _ = watcher.unwatch(path);
                    }
                    for path in next.iter().filter(|p| !watched.contains(p)) {
                        let _ = watcher.watch(path, RecursiveMode::NonRecursive);
                    }
                    watched = next;
                }
            })?;
        Ok(Self {
            tx,
            changed,
            paths: Vec::new(),
        })
    }
    pub fn set_folders(&mut self, configs: Vec<FolderConfig>) {
        let paths = configs
            .iter()
            .map(|c| (c.path.clone(), c.subfolder.clone()))
            .collect::<Vec<_>>();
        if paths != self.paths {
            self.paths = paths;
            let _ = self.tx.send(configs);
        }
    }
    pub fn take_changed(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}
