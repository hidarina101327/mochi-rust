//! 收藏保存工作区相对 POSIX 路径；缺失文件不移除索引。
//! 首次导入 Electron pin 后不做双向同步；写入通过同目录临时文件刷盘后原子替换。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::paths;

const FAVORITES_FILE_NAME: &str = "favorites.json";
const FORMAT_VERSION: u64 = 1;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 一个工作区的收藏索引。
///
/// entries 始终是已经规范化的工作区相对路径，且按加入顺序保存。路径
/// 只在构造和修改入口处验证；因此 contains 和 paths 不会把一个工作区
/// 的条目误认为另一个工作区的条目。
#[derive(Debug, Clone)]
pub struct Favorites {
    root: PathBuf,
    entries: Vec<String>,
}

impl Favorites {
    /// 从 root/.mochi/favorites.json 加载收藏。
    ///
    /// 文件不存在表示该工作区还没有收藏，返回空索引；文件存在但 JSON 损坏、
    /// 版本不支持或包含越出工作区的路径时返回错误，绝不用空索引覆盖原文件。
    pub fn load(root: &Path) -> Result<Self> {
        let settings_path = shared_settings_path();
        Self::load_with_settings(root, settings_path.as_deref())
    }

    /// 读取指定的 Electron settings 文件。仅用于测试，避免测试修改进程环境。
    fn load_with_settings(root: &Path, settings_path: Option<&Path>) -> Result<Self> {
        let root = canonical_workspace_root(root)?;
        let mut favorites = Self {
            root,
            entries: Vec::new(),
        };
        let path = favorites.store_path();

        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                favorites.import_electron_pins(settings_path);
                if !favorites.entries.is_empty() {
                    favorites.save()?;
                }
                return Ok(favorites);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("读取收藏文件失败: {}", path.display()))
            }
        };

        let raw_paths = decode_paths(&text)
            .with_context(|| format!("收藏文件损坏，原文件已保留: {}", path.display()))?;

        for (index, raw_path) in raw_paths.into_iter().enumerate() {
            let relative = favorites
                .normalize_stored_path(&raw_path)
                .with_context(|| {
                    format!("收藏文件中的路径无效 (第 {} 项): {raw_path}", index + 1)
                })?;

            // 目录不能成为收藏。缺失路径则保留，供文件恢复或明确删除时使用。
            if favorites.absolute_for_relative(&relative).is_dir() {
                bail!("收藏文件中的路径是目录 (第 {} 项): {}", index + 1, raw_path);
            }
            favorites.push_unique(relative);
        }

        Ok(favorites)
    }

    /// 判断 path 是否已收藏。非法或越出当前工作区的路径返回 false。
    pub fn contains(&self, path: &Path) -> bool {
        let Ok(relative) = self.normalize_target(path) else {
            return false;
        };
        self.entries.iter().any(|entry| same_path(entry, &relative))
    }

    /// 返回收藏的绝对路径，顺序与收藏文件中的顺序一致。
    ///
    /// 缺失文件也会返回其规范化后的绝对路径；调用方可以通过 is_file 或
    /// 自己的文件树过滤掉当前不可见的条目。
    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .map(|entry| self.absolute_for_relative(entry))
            .collect()
    }
    /// 只对既有条目重排序；绝不顺手增删条目。
    pub fn reorder(&mut self, paths: &[PathBuf]) -> Result<()> {
        anyhow::ensure!(
            paths.len() == self.entries.len(),
            "收藏排序必须包含全部已有条目"
        );
        let next = paths
            .iter()
            .map(|p| self.normalize_target(p))
            .collect::<Result<Vec<_>>>()?;
        for (i, p) in next.iter().enumerate() {
            anyhow::ensure!(
                self.entries.iter().any(|e| same_path(e, p))
                    && !next[..i].iter().any(|e| same_path(e, p)),
                "收藏排序包含未知或重复条目"
            );
        }
        let previous = std::mem::replace(&mut self.entries, next);
        if let Err(error) = self.save() {
            self.entries = previous;
            return Err(error);
        }
        Ok(())
    }

    /// 切换一个文件的收藏状态，返回切换后的状态。
    ///
    /// 只有现有文件可以加入索引；不存在的路径只能取消一个已经保留的
    /// 缺失收藏。已存在的目录不能加入。若该目录已经是损坏索引中的条目，
    /// 调用仍可将其移除。
    pub fn toggle(&mut self, path: &Path) -> Result<bool> {
        let relative = self.normalize_target(path)?;
        if let Some(index) = self.index_of(&relative) {
            let previous = self.entries.remove(index);
            if let Err(error) = self.save() {
                self.entries.insert(index, previous);
                return Err(error);
            }
            return Ok(false);
        }

        let absolute = self.absolute_for_relative(&relative);
        let metadata = fs::metadata(&absolute)
            .with_context(|| format!("收藏目标不存在或无法读取: {}", path.display()))?;
        if !metadata.is_file() {
            bail!("目录不能加入收藏: {}", path.display());
        }

        self.entries.push(relative);
        if let Err(error) = self.save() {
            self.entries.pop();
            return Err(error);
        }
        Ok(true)
    }

    /// 将一个路径（包括目录下的所有收藏）映射到新路径。
    ///
    /// 旧路径本身可以已经不存在；这正是保留缺失收藏后处理重命名的场景。
    /// 若新路径已收藏，重映射后会自动去重。目录重命名会更新所有后代条目。
    pub fn remap_path(&mut self, old: &Path, new: &Path) -> Result<()> {
        let old_relative = self.normalize_target(old)?;
        let new_relative = self.normalize_target(new)?;
        if old_relative == new_relative {
            return Ok(());
        }

        let affected = self
            .entries
            .iter()
            .any(|entry| same_path(entry, &old_relative) || is_under(entry, &old_relative));
        if !affected {
            return Ok(());
        }

        let moving_directory = self.absolute_for_relative(&old_relative).is_dir()
            || self
                .entries
                .iter()
                .any(|entry| is_under(entry, &old_relative));
        let new_is_directory = self.absolute_for_relative(&new_relative).is_dir();
        if new_is_directory && !moving_directory {
            bail!("目录不能成为收藏: {}", new.display());
        }
        let before = self.entries.clone();
        let mut changed = false;
        let mut mapped = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            if same_path(entry, &old_relative) {
                mapped.push(new_relative.clone());
                changed = true;
            } else if is_under(entry, &old_relative) {
                let suffix =
                    path_suffix(entry, &old_relative).expect("is_under 已确认路径有旧路径前缀");
                mapped.push(join_relative(&new_relative, suffix));
                changed = true;
            } else {
                mapped.push(entry.clone());
            }
        }

        self.entries.clear();
        for entry in mapped {
            self.push_unique(entry);
        }

        if changed {
            if let Err(error) = self.save() {
                self.entries = before;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn remove_under(&mut self, path: &Path) -> Result<()> {
        let relative = self.normalize_target(path)?;
        let before = self.entries.clone();
        self.entries
            .retain(|entry| !(same_path(entry, &relative) || is_under(entry, &relative)));
        if self.entries == before {
            return Ok(());
        }

        if let Err(error) = self.save() {
            self.entries = before;
            return Err(error);
        }
        Ok(())
    }

    fn store_path(&self) -> PathBuf {
        paths::mochi_dir(&self.root).join(FAVORITES_FILE_NAME)
    }

    fn index_of(&self, relative: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| same_path(entry, relative))
    }

    fn push_unique(&mut self, relative: String) {
        if self.index_of(&relative).is_none() {
            self.entries.push(relative);
        }
    }

    fn absolute_for_relative(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// 把外部传入的路径解析成规范化的相对路径，并验证它位于当前工作区内。
    fn normalize_target(&self, path: &Path) -> Result<String> {
        let candidate = self.absolute_candidate(path)?;
        // 先检查字面路径，防止工作区外的 symlink 指回工作区而被当成合法输入。
        let _ = self.relative_from_absolute(&candidate)?;
        let canonical = canonicalize_with_missing(&candidate)?;
        self.relative_from_absolute(&canonical)
    }

    fn absolute_candidate(&self, path: &Path) -> Result<PathBuf> {
        let input = path.to_string_lossy();
        if input.contains('\0') {
            bail!("路径不能包含 NUL 字符");
        }

        if is_absolute_text(&input) {
            let absolute = normalize_absolute_text(&input)?;
            Ok(PathBuf::from(absolute))
        } else {
            let relative = normalize_relative_text(&input)?;
            Ok(if relative.is_empty() {
                self.root.clone()
            } else {
                self.root.join(relative)
            })
        }
    }

    fn relative_from_absolute(&self, absolute: &Path) -> Result<String> {
        let absolute = normalize_absolute_text(&absolute.to_string_lossy())?;
        let root = normalize_absolute_text(&self.root.to_string_lossy())?;
        let absolute_key = comparison_key(&absolute);
        let root_key = comparison_key(&root);

        if absolute_key == root_key {
            return Ok(String::new());
        }

        let prefix = if root.ends_with('/') {
            root.clone()
        } else {
            format!("{root}/")
        };
        let prefix_key = comparison_key(&prefix);
        if !absolute_key.starts_with(&prefix_key) {
            bail!("路径越出工作区: {absolute}");
        }

        // 用原始绝对路径保留用户/文件系统给出的大小写；Windows 只用 key 做比较。
        let relative = absolute
            .get(prefix.len()..)
            .ok_or_else(|| anyhow::anyhow!("无法解析收藏路径: {absolute}"))?
            .replace('\\', "/");
        let relative = normalize_relative_text(&relative)?;
        if relative.is_empty() {
            bail!("路径不是文件: {absolute}");
        }
        Ok(relative)
    }

    fn normalize_stored_path(&self, raw: &str) -> Result<String> {
        if is_absolute_text(raw) {
            bail!("收藏路径必须是工作区相对路径: {raw}");
        }
        let relative = normalize_relative_text(raw)?;
        if relative.is_empty() {
            bail!("收藏路径不能为空");
        }

        // 对存在的路径解析 symlink 并再次检查工作区边界；缺失路径则保留其
        // 规范化后的相对形式。
        let absolute = self.absolute_for_relative(&relative);
        let canonical = canonicalize_with_missing(&absolute)?;
        let resolved = self.relative_from_absolute(&canonical)?;
        if resolved.is_empty() {
            bail!("收藏路径不是文件: {raw}");
        }
        Ok(resolved)
    }

    fn save(&self) -> Result<()> {
        let path = self.store_path();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .expect("收藏文件总有 .mochi 父目录");
        fs::create_dir_all(parent)
            .with_context(|| format!("创建收藏目录失败: {}", parent.display()))?;

        let mut object = Map::new();
        object.insert("version".into(), Value::from(FORMAT_VERSION));
        object.insert(
            "paths".into(),
            Value::Array(self.entries.iter().cloned().map(Value::String).collect()),
        );
        let mut bytes =
            serde_json::to_vec_pretty(&Value::Object(object)).context("序列化收藏失败")?;
        bytes.push(b'\n');

        let temporary = temporary_path(&path);
        let result = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .with_context(|| format!("创建收藏临时文件失败: {}", temporary.display()))?;
            file.write_all(&bytes)
                .with_context(|| format!("写入收藏临时文件失败: {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("刷盘收藏临时文件失败: {}", temporary.display()))?;
            drop(file);
            replace_atomically(&temporary, &path)
                .with_context(|| format!("替换收藏文件失败: {}", path.display()))?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.context("收藏保存失败，内存中的修改已保留")
    }

    /// Electron 把收藏放在全局 workspace-storage 的 pinnedEntries 中。
    /// 只在工作区收藏文件尚不存在时导入，空文件因此仍表示用户明确的空集合。
    fn import_electron_pins(&mut self, settings_path: Option<&Path>) {
        let Some(settings_path) = settings_path else {
            return;
        };
        let Ok(text) = fs::read_to_string(settings_path) else {
            return;
        };
        let Ok(document) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'))
        else {
            return;
        };
        let Some(raw_bucket) = document.get("workspace-storage").and_then(Value::as_str) else {
            return;
        };
        let Ok(bucket) = serde_json::from_str::<Value>(raw_bucket) else {
            return;
        };
        let Some(state) = bucket.get("state").and_then(Value::as_object) else {
            return;
        };
        let Some(workspace_path) = state.get("workspacePath").and_then(Value::as_str) else {
            return;
        };
        if !is_absolute_text(workspace_path)
            || self
                .normalize_target(Path::new(workspace_path))
                .ok()
                .as_deref()
                != Some("")
        {
            return;
        }
        let Some(entries) = state.get("pinnedEntries").and_then(Value::as_array) else {
            return;
        };

        for entry in entries {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            if entry.get("type").and_then(Value::as_str) == Some("directory") {
                continue;
            }
            let Some(raw_path) = entry.get("path").and_then(Value::as_str) else {
                continue;
            };
            let Ok(relative) = self.normalize_target(Path::new(raw_path)) else {
                continue;
            };
            if relative.is_empty() || self.absolute_for_relative(&relative).is_dir() {
                continue;
            }
            self.push_unique(relative);
        }
    }
}

fn canonical_workspace_root(root: &Path) -> Result<PathBuf> {
    let root =
        fs::canonicalize(root).with_context(|| format!("工作区根目录无效: {}", root.display()))?;
    if !root.is_dir() {
        bail!("工作区根目录不是目录: {}", root.display());
    }
    Ok(strip_verbatim_prefix(root))
}

fn decode_paths(text: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
    match value {
        // 接受早期开发版本可能写出的纯数组，当前版本始终写对象格式。
        Value::Array(paths) => paths
            .into_iter()
            .map(|path| {
                path.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow::anyhow!("paths 必须全是字符串"))
            })
            .collect(),
        Value::Object(object) => decode_object_paths(object),
        _ => bail!("收藏文件顶层必须是对象或路径数组"),
    }
}

fn decode_object_paths(object: Map<String, Value>) -> Result<Vec<String>> {
    if let Some(version) = object.get("version") {
        let supported = version.as_u64() == Some(FORMAT_VERSION)
            || matches!(version.as_str(), Some("1") | Some("1.0"));
        if !supported {
            bail!("不支持的收藏文件版本: {version}");
        }
    }

    let value = object
        .get("paths")
        .or_else(|| object.get("favorites"))
        .ok_or_else(|| anyhow::anyhow!("收藏文件缺少 paths 字段"))?;
    let paths = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("收藏文件的 paths 必须是数组"))?;
    paths
        .iter()
        .map(|path| {
            path.as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("paths 必须全是字符串"))
        })
        .collect()
}

fn canonicalize_with_missing(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path)
            .with_context(|| format!("解析收藏路径失败: {}", path.display()));
    }

    // canonicalize 要求最终组件存在。找到最近的存在祖先后先解析它（包括
    // symlink），再把缺失组件接回去，从而既保留缺失收藏又不绕过 symlink 边界。
    let mut probe = path.to_path_buf();
    let mut missing = Vec::new();
    while !probe.exists() {
        if let Ok(metadata) = fs::symlink_metadata(&probe) {
            if metadata.file_type().is_symlink() {
                bail!("收藏路径包含无法解析的符号链接: {}", probe.display());
            }
        }
        let component = probe
            .file_name()
            .map(|name| name.to_os_string())
            .ok_or_else(|| anyhow::anyhow!("无法解析收藏路径: {}", path.display()))?;
        missing.push(component);
        if !probe.pop() {
            bail!("无法解析收藏路径: {}", path.display());
        }
    }

    let mut resolved = fs::canonicalize(&probe)
        .with_context(|| format!("解析收藏路径失败: {}", path.display()))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn normalize_relative_text(raw: &str) -> Result<String> {
    if raw.contains('\0') {
        bail!("路径不能包含 NUL 字符");
    }
    let raw = raw.replace('\\', "/");
    if is_absolute_text(&raw) {
        bail!("路径必须是工作区相对路径: {raw}");
    }
    if raw.len() >= 2 && raw.as_bytes()[1] == b':' {
        bail!("路径不能使用驱动器相对格式: {raw}");
    }

    let mut components = Vec::new();
    for component in raw.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    bail!("路径越出工作区: {raw}");
                }
            }
            component => components.push(component),
        }
    }
    Ok(components.join("/"))
}

fn normalize_absolute_text(raw: &str) -> Result<String> {
    if raw.contains('\0') {
        bail!("路径不能包含 NUL 字符");
    }
    let mut raw = raw.replace('\\', "/");
    raw = strip_verbatim_prefix_text(raw);

    let (prefix, rest) = if let Some(rest) = raw.strip_prefix("//") {
        ("//".to_owned(), rest)
    } else if let Some(rest) = raw.strip_prefix('/') {
        ("/".to_owned(), rest)
    } else if raw.len() >= 3 && raw.as_bytes()[1] == b':' && raw.as_bytes()[2] == b'/' {
        (raw[..2].to_owned(), &raw[3..])
    } else {
        bail!("路径不是绝对路径: {raw}");
    };

    let mut components = Vec::new();
    for component in rest.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    bail!("路径越出文件系统根目录: {raw}");
                }
            }
            component => components.push(component),
        }
    }

    let normalized = match prefix.as_str() {
        "/" => format!("/{}", components.join("/")),
        "//" => format!("//{}", components.join("/")),
        _ => {
            if components.is_empty() {
                format!("{prefix}/")
            } else {
                format!("{prefix}/{}", components.join("/"))
            }
        }
    };
    if normalized == "/" || (prefix != "//" && prefix != "/" && components.is_empty()) {
        Ok(normalized)
    } else {
        Ok(normalized.trim_end_matches('/').to_owned())
    }
}

fn is_absolute_text(raw: &str) -> bool {
    let raw = raw.replace('\\', "/");
    let raw = strip_verbatim_prefix_text(raw);
    raw.starts_with('/')
        || (raw.len() >= 3 && raw.as_bytes()[1] == b':' && raw.as_bytes()[2] == b'/')
}

fn strip_verbatim_prefix_text(mut path: String) -> String {
    path = path.replace('\\', "/");
    if let Some(rest) = path.strip_prefix("//?/UNC/") {
        path = format!("//{rest}");
    } else if let Some(rest) = path.strip_prefix("//?/") {
        path = rest.to_owned();
    }
    path
}

fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = strip_verbatim_prefix_text(path.to_string_lossy().into_owned());
    PathBuf::from(text)
}

fn comparison_key(path: &str) -> String {
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_owned()
    }
}

fn same_path(left: &str, right: &str) -> bool {
    comparison_key(left) == comparison_key(right)
}

fn is_under(path: &str, directory: &str) -> bool {
    if same_path(path, directory) {
        return false;
    }
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    comparison_key(path).starts_with(&comparison_key(&prefix))
}

fn path_suffix<'a>(path: &'a str, directory: &str) -> Option<&'a str> {
    if same_path(path, directory) {
        return Some("");
    }
    if directory.is_empty() {
        return Some(path);
    }
    let prefix = format!("{directory}/");
    if !comparison_key(path).starts_with(&comparison_key(&prefix)) {
        return None;
    }
    path.get(prefix.len()..)
}

fn join_relative(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_owned()
    } else if suffix.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{suffix}")
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let suffix = format!(
        ".tmp-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(suffix);
    PathBuf::from(temporary)
}

fn shared_settings_path() -> Option<PathBuf> {
    std::env::var_os("MOCHI_SETTINGS_FILE")
        .or_else(|| std::env::var_os("MOCHI_SETTINGS_PATH"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .or_else(|| Some(std::env::temp_dir()))
                .map(|base| base.join("mochi").join("settings.json"))
        })
}

#[cfg(windows)]
fn replace_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, target: *const u16, flags: u32) -> i32;
    }

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0x1 | 0x8) };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    struct TempWorkspace(PathBuf);

    impl TempWorkspace {
        fn new(tag: &str) -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "mochi-favorites-{}-{tag}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn toggles_files_and_round_trips_relative_paths() {
        let workspace = TempWorkspace::new("round-trip");
        let file = workspace.0.join("知识库").join("项目").join("笔记.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "内容").unwrap();

        let mut favorites = Favorites::load(&workspace.0).unwrap();
        assert!(favorites.toggle(&file).unwrap());
        assert!(favorites.contains(&file));
        assert_eq!(
            favorites.paths(),
            vec![strip_verbatim_prefix(fs::canonicalize(&file).unwrap())]
        );
        assert_eq!(
            fs::read_to_string(workspace.0.join(".mochi/favorites.json")).unwrap(),
            "{\n  \"version\": 1,\n  \"paths\": [\n    \"知识库/项目/笔记.md\"\n  ]\n}\n"
        );

        let loaded = Favorites::load(&workspace.0).unwrap();
        assert!(loaded.contains(&file));
        let mut loaded = loaded;
        assert!(!loaded.toggle(&file).unwrap());
        assert!(!loaded.contains(&file));
    }

    #[test]
    fn missing_files_are_retained_until_explicitly_removed() {
        let workspace = TempWorkspace::new("missing");
        let missing = workspace.0.join("知识库").join("稍后恢复.md");
        fs::create_dir_all(missing.parent().unwrap()).unwrap();
        fs::write(&missing, "初始").unwrap();
        let mut favorites = Favorites::load(&workspace.0).unwrap();
        favorites.toggle(&missing).unwrap();
        fs::remove_file(&missing).unwrap();
        assert!(favorites.contains(&missing));
        assert_eq!(
            favorites.paths(),
            vec![strip_verbatim_prefix(missing.clone())]
        );

        fs::create_dir_all(missing.parent().unwrap()).unwrap();
        fs::write(&missing, "恢复").unwrap();
        let reloaded = Favorites::load(&workspace.0).unwrap();
        assert!(reloaded.contains(&missing));
        assert_eq!(
            reloaded.paths(),
            vec![strip_verbatim_prefix(fs::canonicalize(&missing).unwrap())]
        );

        let mut reloaded = reloaded;
        reloaded.remove_under(missing.parent().unwrap()).unwrap();
        assert!(reloaded.paths().is_empty());
    }

    #[test]
    fn rejects_escape_and_directories_without_touching_the_file() {
        let workspace = TempWorkspace::new("validation");
        fs::create_dir_all(workspace.0.join("知识库")).unwrap();
        let outside = workspace.0.parent().unwrap().join("outside-favorite.md");
        let mut favorites = Favorites::load(&workspace.0).unwrap();
        assert!(favorites
            .toggle(Path::new("../outside-favorite.md"))
            .is_err());
        assert!(favorites.toggle(&workspace.0.join("知识库")).is_err());
        assert!(!workspace.0.join(".mochi/favorites.json").exists());

        fs::create_dir_all(workspace.0.join(".mochi")).unwrap();
        fs::write(workspace.0.join(".mochi/favorites.json"), "{broken").unwrap();
        assert!(Favorites::load(&workspace.0).is_err());
        assert_eq!(
            fs::read_to_string(workspace.0.join(".mochi/favorites.json")).unwrap(),
            "{broken"
        );
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn remaps_directory_and_deduplicates_existing_target() {
        let workspace = TempWorkspace::new("rename");
        let old_dir = workspace.0.join("知识库").join("旧目录");
        let new_dir = workspace.0.join("知识库").join("新目录");
        fs::create_dir_all(old_dir.join("子目录")).unwrap();
        fs::create_dir_all(new_dir.join("子目录")).unwrap();
        let old_file = old_dir.join("子目录").join("a.md");
        let new_file = new_dir.join("子目录").join("a.md");
        fs::write(&old_file, "a").unwrap();
        fs::write(&new_file, "a").unwrap();

        let mut favorites = Favorites::load(&workspace.0).unwrap();
        favorites.toggle(&old_file).unwrap();
        favorites.toggle(&new_file).unwrap();
        favorites.remap_path(&old_dir, &new_dir).unwrap();
        assert_eq!(
            favorites.paths(),
            vec![strip_verbatim_prefix(fs::canonicalize(&new_file).unwrap())]
        );
        assert!(!favorites.contains(&old_file));
        assert!(favorites.contains(&new_file));
    }

    #[test]
    fn remapping_an_unfavorited_directory_is_a_no_op() {
        let workspace = TempWorkspace::new("rename-empty");
        let old_dir = workspace.0.join("旧目录");
        let new_dir = workspace.0.join("新目录");
        fs::create_dir_all(&old_dir).unwrap();
        fs::create_dir_all(&new_dir).unwrap();

        let mut favorites = Favorites::load(&workspace.0).unwrap();
        favorites.remap_path(&old_dir, &new_dir).unwrap();
        assert!(favorites.paths().is_empty());
        assert!(!workspace.0.join(".mochi/favorites.json").exists());
    }

    #[test]
    fn workspaces_are_isolated() {
        let first = TempWorkspace::new("first");
        let second = TempWorkspace::new("second");
        let first_file = first.0.join("a.md");
        let second_file = second.0.join("a.md");
        fs::write(&first_file, "a").unwrap();
        fs::write(&second_file, "a").unwrap();

        let mut favorites = Favorites::load(&first.0).unwrap();
        favorites.toggle(&first_file).unwrap();
        assert!(favorites.contains(&first_file));
        assert!(!favorites.contains(&second_file));
        assert_eq!(
            Favorites::load(&second.0).unwrap().paths(),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn imports_file_pins_from_electron_workspace_storage_once() {
        let workspace = TempWorkspace::new("electron-import");
        let file = workspace.0.join("知识库").join("旧收藏.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "旧版").unwrap();

        let settings = workspace.0.join("electron-settings.json");
        let document = serde_json::json!({
            "workspace-storage": serde_json::json!({
                "version": 1,
                "state": {
                    "workspacePath": fs::canonicalize(&workspace.0).unwrap(),
                    "pinnedEntries": [
                        {"id": file.to_string_lossy(), "name": "旧收藏.md", "path": file.to_string_lossy(), "type": "file"},
                        {"id": "directory", "name": "知识库", "path": workspace.0.join("知识库").to_string_lossy(), "type": "directory"}
                    ]
                }
            }).to_string()
        });
        fs::write(&settings, serde_json::to_vec(&document).unwrap()).unwrap();

        let loaded = Favorites::load_with_settings(&workspace.0, Some(&settings)).unwrap();
        assert!(loaded.contains(&file));
        assert!(workspace.0.join(".mochi/favorites.json").is_file());
    }
}
