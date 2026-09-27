//! Shell 图标提取和磁盘读写都放在有界工作线程中；绘制时只查询内存。
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock},
};
use windows::{
    core::{Result, HSTRING},
    Win32::{
        Foundation::{GENERIC_WRITE, HWND, LPARAM, WPARAM},
        Graphics::Imaging::*,
        System::Com::*,
        UI::{
            Shell::*,
            WindowsAndMessaging::{DestroyIcon, IsWindow, PostMessageW, WM_APP},
        },
    },
};
pub const READY: u32 = WM_APP + 120;
type Key = (PathBuf, String);
#[derive(Default)]
struct State {
    entries: HashMap<Key, Option<String>>,
    waiting: HashSet<Key>,
    windows: HashSet<isize>,
}
struct Cache {
    state: Arc<Mutex<State>>,
    queue: mpsc::SyncSender<Key>,
}
fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let state = Arc::new(Mutex::new(State::default()));
        let (queue, rx) = mpsc::sync_channel::<Key>(256);
        let rx = Arc::new(Mutex::new(rx));
        for index in 0..2 {
            let state = state.clone();
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("desktop-icon-{index}"))
                .spawn(move || {
                    let initialized =
                        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
                    loop {
                        let next = { rx.lock().unwrap().recv() };
                        let Ok(key) = next else {
                            break;
                        };
                        let result = load(&key.0, &key.1).ok();
                        let windows = {
                            let mut state = state.lock().unwrap();
                            state.waiting.remove(&key);
                            state.entries.insert(key, result);
                            state.windows.clone()
                        };
                        for raw in windows {
                            unsafe {
                                let _ = PostMessageW(
                                    Some(HWND(raw as *mut _)),
                                    READY,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                            }
                        }
                    }
                    if initialized {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                })
                .expect("start desktop icon worker");
        }
        Cache { state, queue }
    })
}
pub fn watch(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let mut state = cache().state.lock().unwrap();
    state
        .windows
        .retain(|raw| unsafe { IsWindow(Some(HWND(*raw as *mut _))).as_bool() });
    state.windows.insert(hwnd.0 as isize);
}
pub fn path(workspace: &Path, target: &str) -> Option<String> {
    if workspace.as_os_str().is_empty() || !Path::new(target).is_absolute() {
        return None;
    }
    let key = (workspace.to_owned(), target.to_owned());
    let cache = cache();
    let mut state = cache.state.lock().unwrap();
    if let Some(value) = state.entries.get(&key) {
        return value.clone();
    }
    if !state.waiting.contains(&key) {
        if state.entries.len() >= 4096 {
            if let Some(old) = state.entries.keys().next().cloned() {
                state.entries.remove(&old);
            }
        }
        state.waiting.insert(key.clone());
        if cache.queue.try_send(key.clone()).is_err() {
            state.waiting.remove(&key);
        }
    }
    None
}
fn cached_path(workspace: &Path, target: &str) -> PathBuf {
    // 使用固定的 FNV-1a 算法，不受进程或 Rust 版本影响。Windows 路径不区分大小写。
    let key = target.replace('\\', "/").to_lowercase();
    let hash = key.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    });
    workspace
        .join(".mochi/cache/desktop-icons/v1")
        .join(format!("{hash:016x}.png"))
}
fn load(workspace: &Path, target: &str) -> Result<String> {
    let path = cached_path(workspace, target);
    // 在访问源文件或调用 Shell 前先复用磁盘上已有的图像（离线时也可用）。
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 8) {
        return Ok(path.to_string_lossy().into_owned());
    }
    let io = |e: std::io::Error| {
        windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
    };
    std::fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    let temporary = path.with_extension(format!(
        "{}-{:?}.tmp.png",
        std::process::id(),
        std::thread::current().id()
    ));
    let result = extract(target, &temporary);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        if !path.is_file() {
            return Err(io(error));
        }
    }
    Ok(path.to_string_lossy().into_owned())
}
fn extract(target: &str, path: &std::path::Path) -> Result<String> {
    unsafe {
        let mut info = SHFILEINFOW::default();
        let value = HSTRING::from(target.replace('/', "\\"));
        if SHGetFileInfoW(
            &value,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        ) == 0
        {
            return Err(windows::core::Error::from_thread());
        }
        struct Icon(windows::Win32::UI::WindowsAndMessaging::HICON);
        impl Drop for Icon {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyIcon(self.0);
                }
            }
        }
        let icon = Icon(info.hIcon);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = factory.CreateBitmapFromHICON(icon.0)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(
            &HSTRING::from(path.to_string_lossy().as_ref()),
            GENERIC_WRITE.0,
        )?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(windows::core::Error::from_thread)?;
        frame.Initialize(None)?;
        let mut w = 0;
        let mut h = 0;
        bitmap.GetSize(&mut w, &mut h)?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        Ok(path.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_shortcut_icon_is_async_persistent_and_workspace_scoped() {
        let root = std::env::temp_dir().join(format!("mochi-icon-test-{}", std::process::id()));
        let target = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let started = std::time::Instant::now();
        assert!(path(&root, &target).is_none());
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let icon = loop {
            if let Some(icon) = path(&root, &target) {
                break icon;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(Path::new(&icon).starts_with(root.join(".mochi/cache/desktop-icons")));
        assert_eq!(&std::fs::read(&icon).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
        let before = std::fs::metadata(&icon).unwrap().modified().unwrap();
        cache()
            .state
            .lock()
            .unwrap()
            .entries
            .remove(&(root.clone(), target.clone()));
        // 内存缓存未命中时仍复用 PNG；重新调用 Shell 提取会改写它的时间戳。
        assert_eq!(load(&root, &target).unwrap(), icon);
        assert_eq!(
            std::fs::metadata(&icon).unwrap().modified().unwrap(),
            before
        );
        let missing = root.join("offline.exe").to_string_lossy().into_owned();
        let offline = cached_path(&root, &missing);
        std::fs::copy(&icon, &offline).unwrap();
        assert_eq!(load(&root, &missing).unwrap(), offline.to_string_lossy());
        assert_ne!(
            cached_path(&root, &target),
            cached_path(&root.join("other"), &target)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
