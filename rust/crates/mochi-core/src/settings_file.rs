//! 与 Electron 共用的设置文件，支持跨进程安全写入。
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

pub(super) type Values = BTreeMap<String, String>;
const STALE_AFTER: Duration = Duration::from_secs(120);
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    modified: SystemTime,
    length: u64,
}

pub(super) fn stamp(path: &Path) -> std::io::Result<Option<Stamp>> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(Stamp {
            modified: meta.modified()?,
            length: meta.len(),
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn read(path: &Path) -> Result<Values> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Values::new()),
        Err(error) => return Err(error).context("无法读取共享设置文件"),
    };
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .context("共享设置文件损坏，原文件已保留")
}

pub(super) fn suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Owner {
    pid: u32,
    created_at: u64,
}

pub(crate) struct Lock {
    directory: PathBuf,
}

impl Lock {
    pub(crate) fn acquire(path: &Path) -> Result<Self> {
        Self::acquire_with_timeout(path, LOCK_TIMEOUT)
    }

    fn acquire_with_timeout(path: &Path, timeout: Duration) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let directory = suffix(path, ".lock");
        let started = Instant::now();
        loop {
            match fs::create_dir(&directory) {
                Ok(()) => {
                    let owner = Owner {
                        pid: std::process::id(),
                        created_at: now_millis(),
                    };
                    if let Err(error) =
                        fs::write(directory.join("owner.json"), serde_json::to_vec(&owner)?)
                    {
                        let _ = fs::remove_dir(&directory);
                        return Err(error).context("无法记录设置文件锁");
                    }
                    return Ok(Self { directory });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    reclaim_stale_lock(&directory);
                    if started.elapsed() >= timeout {
                        bail!("共享设置正由另一进程保存，请稍后重试");
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                // 上一个进程的锁目录还在等待删除时，Windows 也可能返回 ACCESS_DENIED。
                // 先按短暂的锁冲突重试；权限问题若一直存在，最终仍会报错。
                Err(error)
                    if cfg!(windows)
                        && error.raw_os_error() == Some(5)
                        && started.elapsed() < timeout =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error).context("无法锁定共享设置文件"),
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // 只清理自己创建的锁目录，不递归删除，也不碰其他进程的文件。
        let owner = fs::read(self.directory.join("owner.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice::<Owner>(&raw).ok());
        if owner.is_some_and(|owner| owner.pid == std::process::id()) {
            let _ = fs::remove_file(self.directory.join("owner.json"));
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

fn reclaim_stale_lock(directory: &Path) {
    // 回收过期锁也要互斥。否则第一个进程清掉旧锁后，第二个进程可能凭旧判断
    // 删掉新写入者刚取得的锁。
    let recovery = suffix(directory, ".recovery");
    if fs::create_dir(&recovery).is_err() {
        return;
    }
    struct Recovery(PathBuf);
    impl Drop for Recovery {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }
    let _recovery = Recovery(recovery);
    let Ok(metadata) = fs::symlink_metadata(directory) else {
        return;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return;
    }
    let age = metadata
        .modified()
        .ok()
        .and_then(|time| time.elapsed().ok())
        .unwrap_or_default();
    let owner_path = directory.join("owner.json");
    match fs::read(&owner_path) {
        Ok(raw) => {
            let Ok(owner) = serde_json::from_slice::<Owner>(&raw) else {
                return;
            };
            if process_is_running(owner.pid) {
                return;
            }
            // 检查期间，其他进程也可能已经清掉了过期锁。
            if fs::read(&owner_path).ok().as_deref() != Some(raw.as_slice()) {
                return;
            }
            let _ = fs::remove_file(owner_path);
            let _ = fs::remove_dir(directory);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if age >= STALE_AFTER {
                let _ = fs::remove_dir(directory);
            }
        }
        Err(_) => {}
    }
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // 遇到权限拒绝等无法确认的情况，不能抢走仍在使用的锁。
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 259; // STILL_ACTIVE
        let queried = GetExitCodeProcess(handle, &mut code) != 0;
        CloseHandle(handle);
        !queried || code == 259
    }
}

#[cfg(not(windows))]
fn process_is_running(pid: u32) -> bool {
    let proc = Path::new("/proc");
    !proc.is_dir() || proc.join(pid.to_string()).exists()
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn write_atomic(path: &Path, values: &Values) -> Result<()> {
    let temporary = suffix(
        path,
        &format!(
            ".tmp-{}-{}-{}",
            std::process::id(),
            now_millis(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
    );
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(serde_json::to_string_pretty(values)?.as_bytes())?;
        file.sync_all()?;
        drop(file);
        replace(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.context("共享设置保存失败，未保存的修改已保留")
}

#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Windows 上的索引器或其他读取者可能短暂占用目标文件。
    for attempt in 0..=40 {
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) } != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(5 | 32 | 33)) || attempt == 40 {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis((5 * (attempt + 1)).min(25)));
    }
    unreachable!()
}

#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_owner_is_never_replaced_when_waiting_for_the_lock() {
        let root = std::env::temp_dir().join(format!(
            "mochi-live-settings-lock-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let path = root.join("settings.json");
        let guard = Lock::acquire(&path).unwrap();
        assert!(Lock::acquire_with_timeout(&path, Duration::from_millis(30)).is_err());
        assert!(guard.directory.join("owner.json").is_file());
        drop(guard);
        fs::remove_dir(root).unwrap();
    }
}
