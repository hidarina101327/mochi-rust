//! 单事件循环管理去抖截止时间；写入稳定后再合并推送，避免旧 Timer 事件迟到。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use notify::{EventKind, RecursiveMode, Watcher};

/// 写入稳定等待（chokidar `awaitWriteFinish.stabilityThreshold`）。
pub const STABILITY_DELAY: Duration = Duration::from_millis(500);
/// 合并窗口（`main.ts` 的 120ms flush 周期）。
pub const COALESCE_WINDOW: Duration = Duration::from_millis(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchChangeType {
    Add,
    Change,
    Unlink,
    AddDir,
    UnlinkDir,
}

impl WatchChangeType {
    fn is_dir_event(self) -> bool {
        matches!(self, Self::AddDir | Self::UnlinkDir)
    }
}

/// 合并后的变更事件（`path` 为绝对路径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    pub kind: WatchChangeType,
    pub path: PathBuf,
}

impl WatchEvent {
    pub fn new(kind: WatchChangeType, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
        }
    }
}

/// 去抖 + 合并的纯逻辑。不碰文件系统、不碰线程，时间全部由调用方喂进来。
pub struct EventPipeline {
    stability_delay: Duration,
    coalesce_window: Duration,
    /// 路径（大小写归一）→ (稳定截止时间, 最新事件)
    stability: HashMap<String, (Instant, WatchEvent)>,
    pending: Vec<WatchEvent>,
    coalesce_deadline: Option<Instant>,
}

impl EventPipeline {
    pub fn new(stability_delay: Duration, coalesce_window: Duration) -> Self {
        Self {
            stability_delay,
            coalesce_window,
            stability: HashMap::new(),
            pending: Vec::new(),
            coalesce_deadline: None,
        }
    }

    /// 收到一个原始事件。目录事件立即入队；文件事件走稳定等待（等写入完成再上报）。
    pub fn push(&mut self, now: Instant, event: WatchEvent) {
        if event.kind.is_dir_event() {
            self.enqueue(now, event);
            return;
        }
        // 同一路径的新事件重置稳定计时（awaitWriteFinish 语义）
        let key = normalize_key(&event.path);
        self.stability
            .insert(key, (now + self.stability_delay, event));
    }

    fn enqueue(&mut self, now: Instant, event: WatchEvent) {
        self.pending.push(event);
        if self.coalesce_deadline.is_none() {
            self.coalesce_deadline = Some(now + self.coalesce_window);
        }
    }

    /// 下一次需要被唤醒的时刻；`None` 表示完全空闲。
    pub fn next_deadline(&self) -> Option<Instant> {
        self.stability
            .values()
            .map(|(at, _)| *at)
            .chain(self.coalesce_deadline)
            .min()
    }

    /// 推进到 `now`。返回 `Some(batch)` 表示合并窗口到期、应当推送这批事件。
    pub fn tick(&mut self, now: Instant) -> Option<Vec<WatchEvent>> {
        let matured: Vec<String> = self
            .stability
            .iter()
            .filter(|(_, (at, _))| *at <= now)
            .map(|(k, _)| k.clone())
            .collect();
        for key in matured {
            if let Some((_, event)) = self.stability.remove(&key) {
                self.enqueue(now, event);
            }
        }

        match self.coalesce_deadline {
            Some(at) if at <= now => {
                self.coalesce_deadline = None;
                if self.pending.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut self.pending))
                }
            }
            _ => None,
        }
    }
}

/// 忽略规则：只看工作区内的相对路径段——点目录/文件、构建目录
/// （`.mochi` 以点开头自然命中）。
pub fn is_ignored(watched_root: &Path, full_path: &Path) -> bool {
    let rel = full_path.strip_prefix(watched_root).unwrap_or(full_path);
    rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || crate::files::ignored_dirs().contains(s.as_ref())
    })
}

fn normalize_key(path: &Path) -> String {
    path.to_string_lossy().to_uppercase()
}

type EventSink = Arc<dyn Fn(Vec<WatchEvent>) + Send + Sync>;

pub struct WatcherService {
    watched_path: PathBuf,
    sink: Mutex<Option<EventSink>>,
    running: Mutex<Option<Running>>,
}

struct Running {
    stop: Sender<()>,
    /// 持有 watcher 句柄；drop 即停止底层监听。
    _watcher: notify::RecommendedWatcher,
    worker: std::thread::JoinHandle<()>,
}

impl WatcherService {
    pub fn new(root_path: impl AsRef<Path>) -> Self {
        Self {
            watched_path: root_path.as_ref().to_path_buf(),
            sink: Mutex::new(None),
            running: Mutex::new(None),
        }
    }

    pub fn watched_path(&self) -> &Path {
        &self.watched_path
    }

    /// 合并窗口到期后触发（在监听线程上调用，不是 UI 线程）。
    pub fn on_events(&self, sink: impl Fn(Vec<WatchEvent>) + Send + Sync + 'static) {
        *self.sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(sink));
    }

    pub fn start(&self) -> Result<()> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if running.is_some() {
            return Ok(());
        }

        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let root = self.watched_path.clone();

        let root_for_cb = root.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                for path in &event.paths {
                    if is_ignored(&root_for_cb, path) {
                        continue;
                    }
                    let Some(kind) = classify(&event.kind, path) else {
                        continue;
                    };
                    let _ = raw_tx.send(WatchEvent::new(kind, path.clone()));
                }
            })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;

        let sink = self.sink.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let worker = std::thread::Builder::new()
            .name("mochi-watcher".into())
            .spawn(move || {
                let mut pipeline = EventPipeline::new(STABILITY_DELAY, COALESCE_WINDOW);
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    // 没有待办时用一个长超时挂着，有待办时睡到最近的截止时间
                    let timeout = pipeline
                        .next_deadline()
                        .map(|at| at.saturating_duration_since(Instant::now()))
                        .unwrap_or(Duration::from_millis(250));

                    match raw_rx.recv_timeout(timeout) {
                        Ok(event) => pipeline.push(Instant::now(), event),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    if let Some(batch) = pipeline.tick(Instant::now()) {
                        if let Some(sink) = &sink {
                            sink(batch);
                        }
                    }
                }
            })?;

        *running = Some(Running {
            stop: stop_tx,
            _watcher: watcher,
            worker,
        });
        Ok(())
    }

    pub fn stop(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(Running {
            stop,
            _watcher,
            worker,
        }) = running
        {
            let _ = stop.send(());
            drop(_watcher);
            // join 之前先释放互斥锁；回调自己可能也会停掉 watcher，
            // 而回调不能 join 自己所在的线程。
            if worker.thread().id() != std::thread::current().id() {
                let _ = worker.join();
            }
        }
    }

    pub fn restart(&self) -> Result<()> {
        self.stop();
        self.start()
    }

    pub fn is_running(&self) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

impl Drop for WatcherService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// notify 的事件种类 → 本地语义。目录/文件的区分靠当下的磁盘状态判断：
/// 删除事件发生时路径已不存在，只能退回「按文件处理」，与 chokidar 的表现一致。
fn classify(kind: &EventKind, path: &Path) -> Option<WatchChangeType> {
    let is_dir = path.is_dir();
    match kind {
        EventKind::Create(_) => Some(if is_dir {
            WatchChangeType::AddDir
        } else {
            WatchChangeType::Add
        }),
        EventKind::Remove(_) => Some(WatchChangeType::Unlink),
        EventKind::Modify(_) => Some(if is_dir {
            WatchChangeType::AddDir
        } else {
            WatchChangeType::Change
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline() -> EventPipeline {
        EventPipeline::new(Duration::from_millis(500), Duration::from_millis(120))
    }

    #[test]
    fn dir_events_bypass_stability_wait() {
        let t0 = Instant::now();
        let mut p = pipeline();
        p.push(t0, WatchEvent::new(WatchChangeType::AddDir, "D:/ws/新建"));

        assert!(p.tick(t0).is_none(), "合并窗口还没到");
        let batch = p.tick(t0 + Duration::from_millis(120)).expect("应推送");
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].kind, WatchChangeType::AddDir);
    }

    #[test]
    fn file_events_wait_for_write_stability() {
        let t0 = Instant::now();
        let mut p = pipeline();
        p.push(t0, WatchEvent::new(WatchChangeType::Change, "D:/ws/a.md"));

        // 500ms 稳定期内不该冒出来
        assert!(p.tick(t0 + Duration::from_millis(400)).is_none());
        // 稳定期满 → 入队；再等 120ms 合并窗口 → 推送
        assert!(p.tick(t0 + Duration::from_millis(500)).is_none());
        let batch = p.tick(t0 + Duration::from_millis(620)).expect("应推送");
        assert_eq!(batch.len(), 1);
    }

    /// 连续保存应只上报一次——这正是 800ms 自动保存要配合的行为。
    #[test]
    fn repeated_writes_reset_the_stability_timer() {
        let t0 = Instant::now();
        let mut p = pipeline();
        for i in 0..5 {
            p.push(
                t0 + Duration::from_millis(i * 200),
                WatchEvent::new(WatchChangeType::Change, "D:/ws/a.md"),
            );
            assert!(p.tick(t0 + Duration::from_millis(i * 200)).is_none());
        }
        // 最后一次写在 800ms，稳定期到 1300ms，成熟后合并窗口再走 120ms
        assert!(p.tick(t0 + Duration::from_millis(1200)).is_none());
        assert!(
            p.tick(t0 + Duration::from_millis(1300)).is_none(),
            "刚成熟入队，合并窗口才开始计时"
        );
        let batch = p.tick(t0 + Duration::from_millis(1420)).expect("应推送");
        assert_eq!(batch.len(), 1, "连续写入应合并成一条: {batch:?}");
    }

    #[test]
    fn events_within_the_window_are_coalesced_into_one_batch() {
        let t0 = Instant::now();
        let mut p = pipeline();
        p.push(t0, WatchEvent::new(WatchChangeType::AddDir, "D:/ws/a"));
        p.push(
            t0 + Duration::from_millis(50),
            WatchEvent::new(WatchChangeType::AddDir, "D:/ws/b"),
        );

        let batch = p.tick(t0 + Duration::from_millis(120)).expect("应推送");
        assert_eq!(batch.len(), 2, "同一窗口内的事件应合并成一批");
    }

    #[test]
    fn different_paths_keep_separate_stability_timers() {
        let t0 = Instant::now();
        let mut p = pipeline();
        p.push(t0, WatchEvent::new(WatchChangeType::Change, "D:/ws/a.md"));
        p.push(
            t0 + Duration::from_millis(300),
            WatchEvent::new(WatchChangeType::Change, "D:/ws/b.md"),
        );

        // a 在 500ms 成熟，b 在 800ms；a 成熟后开的窗口在 620ms 推送
        assert!(p.tick(t0 + Duration::from_millis(500)).is_none());
        let first = p.tick(t0 + Duration::from_millis(620)).expect("a 应先推送");
        assert_eq!(first.len(), 1);
        assert!(first[0].path.ends_with("a.md"));

        assert!(p.tick(t0 + Duration::from_millis(800)).is_none());
        let second = p.tick(t0 + Duration::from_millis(950)).expect("b 应后推送");
        assert!(second[0].path.ends_with("b.md"));
    }

    #[test]
    fn empty_window_produces_no_batch() {
        let t0 = Instant::now();
        let mut p = pipeline();
        assert!(p.tick(t0 + Duration::from_secs(10)).is_none());
    }

    #[test]
    fn next_deadline_is_none_when_idle() {
        let p = pipeline();
        assert!(p.next_deadline().is_none());
    }

    #[test]
    fn next_deadline_tracks_the_earliest_pending_work() {
        let t0 = Instant::now();
        let mut p = pipeline();
        p.push(t0, WatchEvent::new(WatchChangeType::Change, "D:/ws/a.md"));
        let d = p.next_deadline().expect("应有截止时间");
        assert!(d >= t0 + Duration::from_millis(499) && d <= t0 + Duration::from_millis(501));
    }

    #[test]
    fn dot_dirs_and_build_dirs_are_ignored() {
        let root = Path::new("D:/ws");
        assert!(is_ignored(root, Path::new("D:/ws/.mochi/index.db")));
        assert!(is_ignored(root, Path::new("D:/ws/.git/HEAD")));
        assert!(is_ignored(root, Path::new("D:/ws/node_modules/x/y.js")));
        assert!(is_ignored(root, Path::new("D:/ws/dist/out.js")));
        assert!(is_ignored(root, Path::new("D:/ws/知识库/.hidden.md")));

        assert!(!is_ignored(root, Path::new("D:/ws/知识库/正常.md")));
        assert!(!is_ignored(root, Path::new("D:/ws/a.md")));
    }

    /// 路径大小写在 Windows 上不区分，去抖必须按归一化后的键合并。
    #[test]
    fn stability_key_is_case_insensitive() {
        let t0 = Instant::now();
        let mut p = pipeline();
        p.push(t0, WatchEvent::new(WatchChangeType::Change, "D:/ws/A.md"));
        p.push(t0, WatchEvent::new(WatchChangeType::Change, "D:/WS/a.md"));

        assert!(p.tick(t0 + Duration::from_millis(500)).is_none());
        let batch = p.tick(t0 + Duration::from_millis(620)).expect("应推送");
        assert_eq!(batch.len(), 1, "同一文件的大小写变体应合并: {batch:?}");
    }

    #[test]
    fn start_and_stop_are_idempotent() {
        let dir = std::env::temp_dir().join(format!("mochi-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let svc = WatcherService::new(&dir);
        svc.start().unwrap();
        svc.start().unwrap();
        assert!(svc.is_running());
        svc.stop();
        svc.stop();
        assert!(!svc.is_running());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stop_waits_for_an_inflight_callback_to_release_resources() {
        let root = std::env::temp_dir().join(format!(
            "mochi-watch-join-{}",
            crate::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let svc = Arc::new(WatcherService::new(&root));
        let (started_tx, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let wait = Mutex::new(wait);
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mark = finished.clone();
        svc.on_events(move |_| {
            started_tx.send(()).unwrap();
            wait.lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            mark.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        svc.start().unwrap();
        std::fs::write(root.join("note.md"), "trigger").unwrap();
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let worker_svc = svc.clone();
        let (done_tx, done) = mpsc::channel();
        let stopper = std::thread::spawn(move || {
            worker_svc.stop();
            done_tx.send(()).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while svc.is_running() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(done.recv_timeout(Duration::from_millis(30)).is_err());
        release.send(()).unwrap();
        stopper.join().unwrap();
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        drop(svc);
        std::fs::remove_dir_all(root).unwrap();
    }
}
