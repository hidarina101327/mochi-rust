//! 尽力在回复后提取记忆，可单独取消，并限制各工作区的并发量。
//! 提取失败不会导致主对话失败。
use mochi_core::{
    ai::{
        memory_extract::{self, ExtractionOutcome, MemoryExtractionParams},
        models::AiProvider,
        permission::{AiPermissionService, AiToolAction},
        service::AiService,
    },
    app_settings::{self, AppSettings, SettingValue},
    memory::MemoryStore,
    settings::SettingsService,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const MAX_ACTIVE: usize = 2;
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct Exchange {
    pub root: PathBuf,
    pub session_id: String,
    pub user_text: String,
    pub provider: AiProvider,
}
pub struct Job {
    cancel: Arc<AtomicBool>,
    rx: Receiver<ExtractionOutcome>,
}
impl Job {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn finish(&self) -> Option<ExtractionOutcome> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(ExtractionOutcome {
                written: 0,
                error: Some("记忆任务已结束".into()),
            }),
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn enabled(settings: &Arc<SettingsService>) -> bool {
    let app = AppSettings::new(settings.clone());
    app_settings::descriptor("ai.memoryAutoExtract")
        .is_some_and(|d| matches!(app.read(d), SettingValue::Bool(true)))
}

pub fn spawn(
    exchange: Exchange,
    assistant_text: String,
    settings: Arc<SettingsService>,
    permissions: Arc<AiPermissionService>,
    wake: impl Fn() + Send + 'static,
) -> Option<Job> {
    if !enabled(&settings)
        || !memory_extract::worth_extracting(&exchange.user_text)
        || assistant_text.trim().is_empty()
    {
        return None;
    }
    if permissions
        .assert_tool_action_allowed(AiToolAction::WriteFile, None)
        .is_err()
    {
        return None;
    }
    ACTIVE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            (n < MAX_ACTIVE).then_some(n + 1)
        })
        .ok()?;
    let slot = Slot;
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("mochi-memory".into())
        .spawn(move || {
            let _slot = slot;
            let root = &exchange.root;
            let allowed = || {
                !flag.load(Ordering::Relaxed)
                    && root.is_dir()
                    && enabled(&settings)
                    && permissions
                        .assert_tool_action_allowed(AiToolAction::WriteFile, None)
                        .is_ok()
            };
            let outcome = if !allowed() {
                ExtractionOutcome::default()
            } else {
                match MemoryStore::open(root).map(drop) {
                    Ok(()) => {
                        let model = AiService::with_cancel(exchange.provider, flag.clone());
                        memory_extract::extract_with_writer_guarded(
                            &model,
                            &MemoryExtractionParams {
                                session_id: Some(&exchange.session_id),
                                user_text: &exchange.user_text,
                                assistant_text: &assistant_text,
                            },
                            &allowed,
                            &|input| {
                                if !allowed() {
                                    return Ok(false);
                                }
                                let store = MemoryStore::open(root).map_err(|e| e.to_string())?;
                                if !allowed() {
                                    return Ok(false);
                                }
                                store.write(input).map(|_| true).map_err(|e| e.to_string())
                            },
                        )
                    }
                    Err(error) => ExtractionOutcome {
                        written: 0,
                        error: Some(error.to_string()),
                    },
                }
            };
            drop(_slot);
            if tx.send(outcome).is_ok() {
                wake();
            }
        });
    if worker.is_err() {
        return None;
    }
    Some(Job { cancel, rx })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn fixture() -> (PathBuf, Arc<SettingsService>, Arc<AiPermissionService>) {
        let root = std::env::temp_dir().join(format!(
            "mochi-memory-job-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let settings = Arc::new(SettingsService::new(Some(root.join("settings.json"))));
        let permissions = Arc::new(AiPermissionService::new(&root));
        (root, settings, permissions)
    }
    fn exchange(root: &std::path::Path, url: String) -> Exchange {
        Exchange {
            root: root.to_path_buf(),
            session_id: "original-session".into(),
            user_text: "今后请统一使用中文回答我的技术问题".into(),
            provider: AiProvider {
                base_url: url,
                model: "memory-model".into(),
                stream: true,
                protocol: "openai-completions".into(),
                ..Default::default()
            },
        }
    }
    pub(crate) fn server() -> (
        String,
        Receiver<serde_json::Value>,
        mpsc::Sender<()>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (seen_tx, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                assert!(bytes.len() < 1024 * 1024);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let size = headers
                        .lines()
                        .find_map(|l| {
                            l.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + size {
                        break serde_json::from_slice::<serde_json::Value>(
                            &bytes[end + 4..end + 4 + size],
                        )
                        .unwrap();
                    }
                }
            };
            seen_tx.send(body).unwrap();
            wait.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let response=serde_json::json!({"choices":[{"message":{"role":"assistant","content":serde_json::json!({"memories":[{"title":"偏好中文","content":"用户希望用中文回答技术问题","tags":["preference"]}]}).to_string()},"finish_reason":"stop"}]}).to_string();
            let _=write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response);
        });
        (format!("http://{address}/v1"), seen, release, thread)
    }
    fn finish(job: &Job) -> ExtractionOutcome {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(result) = job.finish() {
                return result;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[test]
    fn real_background_request_honors_late_setting_permission_and_cancellation_changes() {
        let _guard = TEST_LOCK.lock().unwrap();
        for mode in ["write", "disabled", "denied", "cancelled"] {
            let (root, settings, permissions) = fixture();
            let (url, seen, release, server) = server();
            let job = spawn(
                exchange(&root, url),
                "好的，今后使用中文。".into(),
                settings.clone(),
                permissions.clone(),
                || {},
            )
            .unwrap();
            let body = seen
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                // 等待 API 响应时不能继续占用旧工作区的
                // SQLite 文件，否则用户切换或关闭工作区后仍无法释放它。
                let exclusive = std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(MemoryStore::db_path(&root))
                    .unwrap();
                drop(exclusive);
            }
            assert_eq!(body["stream"], false);
            assert_eq!(body["max_tokens"], 400);
            assert!(body["tools"].is_null());
            match mode {
                "disabled" => {
                    settings.set("app.ai.memoryAutoExtract", "false");
                }
                "denied" => {
                    let mut actions = permissions.action_permissions();
                    actions.set(AiToolAction::WriteFile, false);
                    permissions.set_action_permissions(actions);
                }
                "cancelled" => job.cancel(),
                _ => {}
            }
            release.send(()).unwrap();
            let result = finish(&job);
            server.join().unwrap();
            assert_eq!(result.written, usize::from(mode == "write"));
            let records = MemoryStore::open(&root).unwrap().list(None).unwrap();
            assert_eq!(records.len(), usize::from(mode == "write"));
            if let Some(record) = records.first() {
                assert_eq!(record.session_id.as_deref(), Some("original-session"));
            }
            drop(job);
            let _ = std::fs::remove_dir_all(root);
        }
        let (root, settings, permissions) = fixture();
        settings.set("app.ai.memoryAutoExtract", "false");
        assert!(spawn(
            exchange(&root, "http://127.0.0.1:9".into()),
            "回复".into(),
            settings,
            permissions,
            || {}
        )
        .is_none());
        assert!(!MemoryStore::db_path(&root).exists());
        let _ = std::fs::remove_dir_all(root);
        {
            let (root, settings, permissions) = fixture();
            let (url, seen, release, server) = server();
            let job = spawn(
                exchange(&root, url),
                "回复".into(),
                settings,
                permissions,
                || {},
            )
            .unwrap();
            seen.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            drop(job);
            release.send(()).unwrap();
            server.join().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while ACTIVE.load(Ordering::Relaxed) > 0 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(MemoryStore::open(&root)
                .unwrap()
                .list(None)
                .unwrap()
                .is_empty());
            let _ = std::fs::remove_dir_all(root);
        }
        let mut pending = Vec::new();
        for _ in 0..MAX_ACTIVE {
            let (root, settings, permissions) = fixture();
            let (url, seen, release, server) = server();
            let job = spawn(
                exchange(&root, url),
                "回复".into(),
                settings,
                permissions,
                || {},
            )
            .unwrap();
            seen.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            pending.push((root, job, release, server));
        }
        let (root, settings, permissions) = fixture();
        assert!(spawn(
            exchange(&root, "http://127.0.0.1:9".into()),
            "回复".into(),
            settings,
            permissions,
            || {}
        )
        .is_none());
        assert!(!MemoryStore::db_path(&root).exists());
        let _ = std::fs::remove_dir_all(root);
        for (root, job, release, server) in pending {
            job.cancel();
            release.send(()).unwrap();
            assert_eq!(finish(&job).written, 0);
            server.join().unwrap();
            drop(job);
            let _ = std::fs::remove_dir_all(root);
        }
        assert_eq!(ACTIVE.load(Ordering::Relaxed), 0);
    }
}
