//! Electron 开发版、发布版与 Rust 共用的应用设置。
//! `%APPDATA%/mochi/settings.json` 保留 Electron 的设置桶；原生键经映射读写同一值。
//! 文件事务使用跨进程锁、字段增量合并与原子替换，API Key 使用 CurrentUser DPAPI。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::Result;

#[path = "settings_file.rs"]
pub(crate) mod file;
#[path = "settings_projection.rs"]
mod projection;

const DPAPI_PREFIX: &str = "dpapi:";

pub struct SettingsService {
    file_path: PathBuf,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    values: BTreeMap<String, String>,
    pending: Vec<projection::Change>,
    stamp: Option<file::Stamp>,
    revision: u64,
    needs_initialization: bool,
    checked_at: Option<std::time::Instant>,
    projected: BTreeMap<String, Option<String>>,
}

impl SettingsService {
    /// 显式路径（包括环境覆盖）保持完全隔离，不读取用户旧配置。
    pub fn new(file_path: Option<PathBuf>) -> Self {
        let migrate_default =
            file_path.is_none() && std::env::var_os("MOCHI_SETTINGS_FILE").is_none();
        let file_path = file_path.unwrap_or_else(default_settings_path);
        let svc = Self {
            file_path,
            state: Mutex::new(State::default()),
        };
        if migrate_default && !svc.file_path.exists() {
            // 别在已有的 Electron 开发配置上盖一个空 profile。
            // 迁移失败时源数据原封不动，flush 时再报错。
            svc.lock().needs_initialization = initialize_shared_profile(&svc.file_path).is_err();
        }
        let _ = svc.reload();
        svc
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Electron 把背景图存成 data URL。原生渲染用派生出的文件缓存，
    /// 设置本身仍是那个 data URL。
    pub fn background_image_path(&self) -> Result<Option<PathBuf>> {
        use base64::Engine;
        use std::hash::{Hash, Hasher};
        let Some(value) = self.get("background.imagePath") else {
            return Ok(None);
        };
        if !value.starts_with("data:") {
            return Ok(Some(PathBuf::from(value)));
        }
        let (header, encoded) = value
            .split_once(',')
            .ok_or_else(|| anyhow::anyhow!("背景图数据无效"))?;
        anyhow::ensure!(
            header.starts_with("data:image/") && header.ends_with(";base64"),
            "背景图格式不受支持"
        );
        anyhow::ensure!(encoded.len() <= 24 * 1024 * 1024, "背景图不能超过 16 MB");
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        anyhow::ensure!(bytes.len() <= 16 * 1024 * 1024, "背景图不能超过 16 MB");
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hash);
        let directory = self
            .file_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("共享设置目录无效"))?
            .join("background-cache");
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{:016x}.bin", hash.finish()));
        if !path.is_file() {
            crate::files::FileService::new().write_bytes_safe(&path, &bytes)?;
        }
        Ok(Some(path))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---------- 读写 ----------

    pub fn get(&self, key: &str) -> Option<String> {
        let mut state = self.lock();
        let _ = self.refresh_locked(&mut state, false);
        if let Some(value) = state.projected.get(key) {
            return value.clone();
        }
        let value = projection::get(&state.values, key)
            .ok()
            .flatten()
            .or_else(|| {
                state
                    .pending
                    .iter()
                    .rev()
                    .find(|change| change.key == key)
                    .and_then(|change| change.after.clone())
            });
        state.projected.insert(key.to_owned(), value.clone());
        value
    }

    pub fn set(&self, key: &str, value: &str) {
        self.change(key, Some(value.to_owned()));
    }

    pub fn remove(&self, key: &str) {
        self.change(key, None);
    }

    fn change(&self, key: &str, after: Option<String>) {
        let mut state = self.lock();
        // 与调用方看到的快照比较。基线先取好再刷新，
        // 否则别人的修改会被算到当前写入方头上。
        let before = projection::get(&state.values, key).ok().flatten();
        if before == after {
            return;
        }
        let _ = self.refresh_locked(&mut state, true);
        let change = projection::Change {
            key: key.into(),
            before,
            after,
        };
        let mut next = state.values.clone();
        if projection::apply(&mut next, &change).is_ok() {
            state.values = next;
        }
        state.pending.push(change);
        state.projected.clear();
        state.revision = state.revision.wrapping_add(1);
    }

    pub fn keys(&self) -> Vec<String> {
        let mut state = self.lock();
        let _ = self.refresh_locked(&mut state, false);
        projection::keys(&state.values)
    }

    /// 标记某键为敏感：落盘时 DPAPI 加密（前缀 `dpapi:`）。
    pub fn mark_sensitive(&self, key: &str) {
        if let Some(plain) = self.get(key) {
            if !is_encrypted(&plain) {
                let _ = self.set_secret(key, &plain);
            }
        }
    }

    /// 读取时自动解密；非敏感或未加密的原样返回。解密失败返回 `None`
    /// （换机器/换用户后 DPAPI 密文不可解，此时当作没配过）。
    pub fn get_secret(&self, key: &str) -> Option<String> {
        let raw = self.get(key)?;
        if !is_encrypted(&raw) {
            return Some(raw);
        }
        unprotect(&raw).ok()
    }

    pub fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        if let Some(existing) = self.get(key) {
            // 空表单分不清「DPAPI 密钥不可用」和「用户主动清空」。
            // 密文读不出来就先保留，直到被新值替换。
            if value.is_empty() && is_encrypted(&existing) && unprotect(&existing).is_err() {
                return Ok(());
            }
            if is_encrypted(&existing) && unprotect(&existing).ok().as_deref() == Some(value) {
                return Ok(());
            }
            if value.is_empty() && existing.is_empty() {
                return Ok(());
            }
        }
        let encrypted = protect(value)?;
        self.set(key, &encrypted);
        Ok(())
    }

    /// 是否有未落盘的改动。上层的 400ms 防抖只需据此决定要不要调 `flush`。
    pub fn is_dirty(&self) -> bool {
        !self.lock().pending.is_empty()
    }

    /// 聚焦处理靠 revision 刷新 UI 快照——即使更早的一次 get()
    /// 已经感知到外部文件变化。
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    // ---------- 持久化 ----------

    pub fn flush(&self) -> Result<()> {
        let mut state = self.lock();
        if state.needs_initialization {
            initialize_shared_profile(&self.file_path)?;
            state.needs_initialization = false;
        }
        let _guard = file::Lock::acquire(&self.file_path)?;
        let original = file::read(&self.file_path)?;
        let mut latest = if !original.contains_key(projection::VERSION_KEY) {
            projection::migrate_native(&original)?
        } else {
            original.clone()
        };
        for change in &state.pending {
            projection::apply(&mut latest, change)?;
        }
        projection::normalize_providers(&mut latest)?;
        if latest != original || !self.file_path.is_file() {
            file::write_atomic(&self.file_path, &latest)?;
        }
        // 只有完整跑完的原子事务才消费本地修改。
        state.values = latest;
        state.projected.clear();
        state.pending.clear();
        state.stamp = file::stamp(&self.file_path)?;
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }

    pub fn reload(&self) -> Result<bool> {
        self.refresh_locked(&mut self.lock(), true)
    }

    fn refresh_locked(&self, state: &mut State, force: bool) -> Result<bool> {
        // 绘制/布局每帧要读大量偏好。聚焦事件和每次修改仍然立即重载；
        // 普通读取的探测频率封顶 5 Hz。
        if !force
            && state
                .checked_at
                .is_some_and(|checked| checked.elapsed() < std::time::Duration::from_millis(200))
        {
            return Ok(false);
        }
        // 先打戳再读：并发替换最多让用户多读一次，
        // 绝不会把旧字节标成新写入的 revision。
        let stamp = file::stamp(&self.file_path)?;
        state.checked_at = Some(std::time::Instant::now());
        if !force && stamp == state.stamp {
            return Ok(false);
        }
        let mut latest = file::read(&self.file_path)?;
        for change in &state.pending {
            projection::apply(&mut latest, change)?;
        }
        let changed = latest != state.values;
        if changed {
            state.values = latest;
            state.projected.clear();
            state.revision = state.revision.wrapping_add(1);
        }
        state.stamp = stamp;
        Ok(changed)
    }
}

impl Drop for SettingsService {
    fn drop(&mut self) {
        if self
            .state
            .get_mut()
            .map(|state| !state.pending.is_empty())
            .unwrap_or(false)
        {
            let _ = self.flush();
        }
    }
}

fn default_settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MOCHI_SETTINGS_FILE") {
        return PathBuf::from(path);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("mochi").join("settings.json")
}

/// 显式、无副作用的转换入口，给迁移测试和其他宿主用。
pub fn migrate_native_values(
    native: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>> {
    projection::migrate_native(native)
}

pub fn background_image_data_url(path: &Path) -> Result<String> {
    use base64::Engine;
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 16 * 1024 * 1024,
        "背景图不能超过 16 MB"
    );
    let bytes = std::fs::read(path)?;
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        anyhow::bail!("背景图格式不受支持")
    };
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn initialize_shared_profile(target: &Path) -> Result<()> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    initialize_shared_profile_at(target, &base)
}

fn initialize_shared_profile_at(target: &Path, base: &Path) -> Result<()> {
    let _guard = file::Lock::acquire(target)?;
    if target.exists() {
        return Ok(());
    }
    let legacy_native = base.join("mochi-native/settings.json");
    let sources = [
        (
            base.join("mochi-dev/mochi-dev-settings.json"),
            "electron-dev",
        ),
        (
            base.join("mochi-desktop/mochi-settings.json"),
            "electron-release",
        ),
        (base.join("mochi/mochi-settings.json"), "electron-legacy"),
    ];
    let selected = sources.into_iter().find(|(path, _)| path.is_file());
    let (mut values, source, selected_path) = if let Some((path, source)) = selected {
        let mut values = file::read(&path)?;
        if legacy_native.is_file() {
            let native = file::read(&legacy_native)?;
            for (key, value) in native {
                if !projection::is_alias(&key) {
                    values.entry(key).or_insert(value);
                }
            }
        }
        (values, source, Some(path))
    } else if legacy_native.is_file() {
        (
            projection::migrate_native(&file::read(&legacy_native)?)?,
            "rust",
            Some(legacy_native),
        )
    } else {
        (BTreeMap::new(), "new", None)
    };
    for performance_path in [
        base.join("mochi-dev/mochi-performance.json"),
        base.join("mochi-desktop/mochi-performance.json"),
    ] {
        if !performance_path.exists() {
            continue;
        }
        let performance: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&performance_path)?.trim_start_matches('\u{feff}'),
        )?;
        if performance.is_object() {
            values.insert("electron.performance".into(), performance.to_string());
        }
        break;
    }
    projection::normalize_providers(&mut values)?;
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Some(selected_path) = selected_path {
        let directory = target
            .parent()
            .ok_or_else(|| anyhow::anyhow!("共享设置目录无效"))?
            .join("settings-backups");
        std::fs::create_dir_all(&directory)?;
        let prefix = timestamp.replace([':', '.'], "-");
        std::fs::copy(
            selected_path,
            directory.join(format!("{prefix}-{source}.json")),
        )?;
    }
    values.insert(projection::VERSION_KEY.into(), "1".into());
    values.insert(projection::SOURCE_KEY.into(), source.into());
    values.insert("__mochi_shared_settings_migrated_at".into(), timestamp);
    file::write_atomic(target, &values)
}

#[cfg(test)]
#[path = "settings_shared_tests.rs"]
mod settings_shared;

fn is_encrypted(value: &str) -> bool {
    value.starts_with(DPAPI_PREFIX)
}

// ---------- DPAPI ----------

#[cfg(windows)]
mod dpapi {
    use anyhow::{bail, Result};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// 取出输出 blob 的字节并释放其内存。
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(out.pbData as *mut _) };
        bytes
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>> {
        let input = blob(plain);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut out,
            )
        };
        if ok == 0 {
            bail!("CryptProtectData 失败");
        }
        Ok(unsafe { take(out) })
    }

    pub fn unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
        let input = blob(cipher);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut out,
            )
        };
        if ok == 0 {
            bail!("CryptUnprotectData 失败（换用户或换机器后密文不可解）");
        }
        Ok(unsafe { take(out) })
    }
}

/// 非 Windows 平台没有 DPAPI。这里**不做假加密**——宁可明说不支持，
/// 也不要让调用方以为密钥被保护了。
#[cfg(not(windows))]
mod dpapi {
    use anyhow::{bail, Result};
    pub fn protect(_: &[u8]) -> Result<Vec<u8>> {
        bail!("当前平台无 DPAPI，拒绝以明文冒充加密")
    }
    pub fn unprotect(_: &[u8]) -> Result<Vec<u8>> {
        bail!("当前平台无 DPAPI")
    }
}

fn protect(plain: &str) -> Result<String> {
    let cipher = dpapi::protect(plain.as_bytes())?;
    Ok(format!("{DPAPI_PREFIX}{}", base64_encode(&cipher)))
}

fn unprotect(stored: &str) -> Result<String> {
    let cipher = base64_decode(&stored[DPAPI_PREFIX.len()..])?;
    Ok(String::from_utf8(dpapi::unprotect(&cipher)?)?)
}

// ---------- base64（标准字母表带填充，与 Convert.ToBase64String 一致）----------

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for ch in text.bytes() {
        if ch == b'=' || ch == b'\n' || ch == b'\r' {
            continue;
        }
        let Some(v) = B64.iter().position(|c| *c == ch) else {
            anyhow::bail!("非法 base64 字符: {}", ch as char);
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_file(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "mochi-settings-{}-{tag}-{n}/settings.json",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        p
    }

    #[test]
    fn set_get_remove() {
        let svc = SettingsService::new(Some(temp_file("crud")));
        assert_eq!(svc.get("theme"), None);
        svc.set("theme", "dark");
        assert_eq!(svc.get("theme").as_deref(), Some("dark"));
        svc.remove("theme");
        assert_eq!(svc.get("theme"), None);
    }

    #[test]
    fn survives_flush_and_reload() {
        let path = temp_file("reload");
        {
            let svc = SettingsService::new(Some(path.clone()));
            svc.set("workspace.lastPath", "D:\\mochi");
            svc.set("theme", "dark");
            svc.flush().unwrap();
        }
        let svc = SettingsService::new(Some(path));
        assert_eq!(svc.get("workspace.lastPath").as_deref(), Some("D:\\mochi"));
        assert_eq!(svc.get("theme").as_deref(), Some("dark"));
    }

    #[test]
    fn file_is_lf_utf8_without_bom() {
        let path = temp_file("lf");
        let svc = SettingsService::new(Some(path.clone()));
        svc.set("名称", "墨池");
        svc.flush().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.contains(&b'\r'), "写出了 CRLF");
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"墨池\""), "中文被转义了: {text}");
    }

    #[test]
    fn corrupt_file_starts_empty_instead_of_failing() {
        let path = temp_file("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();

        let svc = SettingsService::new(Some(path));
        assert!(svc.keys().is_empty(), "损坏文件应当作空配置，而不是崩");
    }

    #[test]
    fn dirty_flag_tracks_unsaved_changes() {
        let svc = SettingsService::new(Some(temp_file("dirty")));
        assert!(!svc.is_dirty());
        svc.set("a", "1");
        assert!(svc.is_dirty());
        svc.flush().unwrap();
        assert!(!svc.is_dirty());
    }

    #[test]
    fn base64_roundtrip_matches_standard_alphabet() {
        for case in [
            &b""[..],
            b"a",
            b"ab",
            b"abc",
            b"abcd",
            &[0u8, 255, 128, 7][..],
        ] {
            let encoded = base64_encode(case);
            assert_eq!(base64_decode(&encoded).unwrap(), case, "编码: {encoded}");
        }
        // 与 .NET Convert.ToBase64String / JS btoa 一致的填充
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[cfg(windows)]
    #[test]
    fn secrets_are_encrypted_at_rest_and_decrypt_on_read() {
        let path = temp_file("secret");
        let svc = SettingsService::new(Some(path.clone()));
        svc.set_secret("ai.apiKey", "sk-super-secret-12345")
            .unwrap();
        svc.flush().unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("sk-super-secret"), "密钥以明文落盘了！");
        assert!(on_disk.contains("dpapi:"), "缺少加密前缀: {on_disk}");

        assert_eq!(
            svc.get_secret("ai.apiKey").as_deref(),
            Some("sk-super-secret-12345")
        );
    }

    #[cfg(windows)]
    #[test]
    fn secrets_survive_reload() {
        let path = temp_file("secret-reload");
        {
            let svc = SettingsService::new(Some(path.clone()));
            svc.set_secret("k", "值").unwrap();
            svc.flush().unwrap();
        }
        let svc = SettingsService::new(Some(path));
        assert_eq!(svc.get_secret("k").as_deref(), Some("值"));
    }

    #[cfg(windows)]
    #[test]
    fn mark_sensitive_encrypts_existing_plaintext() {
        let path = temp_file("mark");
        let svc = SettingsService::new(Some(path.clone()));
        svc.set("token", "plain-value");
        svc.mark_sensitive("token");
        svc.flush().unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("plain-value"), "标记后仍是明文");
        assert_eq!(svc.get_secret("token").as_deref(), Some("plain-value"));
    }

    #[cfg(windows)]
    #[test]
    fn get_secret_passes_through_unencrypted_values() {
        let svc = SettingsService::new(Some(temp_file("passthrough")));
        svc.set("theme", "dark");
        assert_eq!(svc.get_secret("theme").as_deref(), Some("dark"));
    }

    #[cfg(windows)]
    #[test]
    fn undecryptable_secret_reads_as_none_not_garbage() {
        let svc = SettingsService::new(Some(temp_file("baddec")));
        svc.set("k", "dpapi:bm90LXJlYWxseS1lbmNyeXB0ZWQ=");
        assert_eq!(svc.get_secret("k"), None, "解不开应返回 None 而不是乱码");
    }
}
