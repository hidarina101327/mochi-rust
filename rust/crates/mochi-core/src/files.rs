//! 保存前写同级 .backup；失败按 100ms × attempt 退避重试，内容未变则跳过。

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};

use crate::domain::FileNode;
use crate::jstime;

/// 忽略的目录名（`build_file_tree` 与 watcher 共用）。
pub fn ignored_dirs() -> &'static HashSet<&'static str> {
    static DIRS: std::sync::OnceLock<HashSet<&'static str>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        ["node_modules", "dist", "dist-electron", "release", "assets"]
            .into_iter()
            .collect()
    })
}

const BASE36: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// 与 save.ts 的哈希兼容：遍历 UTF-16 码元，32 位回绕，base36 保留负号。
pub fn hash_content(content: &str) -> String {
    let mut hash: i32 = 0;
    for unit in content.encode_utf16() {
        hash = hash
            .wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(unit as i32);
    }
    to_base36(hash)
}

fn to_base36(value: i32) -> String {
    if value == 0 {
        return "0".into();
    }
    // i32::MIN 的绝对值溢出 i32，先升到 i64
    let mut v = (value as i64).unsigned_abs();
    let mut buf = Vec::with_capacity(8);
    while v > 0 {
        buf.push(BASE36[(v % 36) as usize]);
        v /= 36;
    }
    if value < 0 {
        buf.push(b'-');
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 字母表是 ASCII")
}

#[derive(Default)]
pub struct FileService {
    /// 串行化保存队列（与旧版顺序保存语义一致）。
    save_gate: Mutex<()>,
}

impl FileService {
    pub fn new() -> Self {
        Self::default()
    }

    /// 安全写文件：确保父目录 → 写 `.backup` → 原子替换目标 → 删 `.backup`；
    /// 失败重试 ×3，全部失败时返回最后一个错误（目标文件保持旧内容，`.backup` 留作恢复）。
    pub fn write_file_safe(&self, file_path: impl AsRef<Path>, content: &str) -> Result<()> {
        self.write_bytes_safe(file_path, content.as_bytes())
    }

    pub fn write_bytes_safe(&self, file_path: impl AsRef<Path>, content: &[u8]) -> Result<()> {
        let file_path = file_path.as_ref();
        if let Some(dir) = file_path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }

        let backup = append_ext(file_path, ".backup");
        if file_path.exists() {
            fs::copy(file_path, &backup)
                .with_context(|| format!("备份失败: {}", file_path.display()))?;
        }

        let mut last_err = None;
        for attempt in 1..=3u32 {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let temporary = append_ext(
                file_path,
                &format!(
                    ".tmp-{}-{}",
                    std::process::id(),
                    SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ),
            );
            let result = (|| -> std::io::Result<()> {
                use std::io::Write;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                output.write_all(content)?;
                output.sync_all()?;
                drop(output);
                replace_file(&temporary, file_path)
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            match result {
                Ok(()) => {
                    if backup.exists() {
                        let _ = fs::remove_file(&backup);
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 3 {
                        std::thread::sleep(Duration::from_millis(100 * attempt as u64));
                    }
                }
            }
        }
        Err(last_err.unwrap()).with_context(|| format!("写入失败: {}", file_path.display()))
    }

    /// 串行化保存。返回实际是否落盘（内容未变则跳过）。
    pub fn enqueue_save(&self, file_path: impl AsRef<Path>, content: &str) -> Result<bool> {
        let file_path = file_path.as_ref();
        let _guard = self.save_gate.lock().unwrap_or_else(|e| e.into_inner());

        if file_path.exists() {
            if let Ok(disk) = fs::read_to_string(file_path) {
                if hash_content(&disk) == hash_content(content) {
                    return Ok(false);
                }
            }
        }
        self.write_file_safe(file_path, content)?;
        Ok(true)
    }

    /// 构建文件树：跳过 `.mochi`/点目录/构建目录；`*.link.json` 变为 link 节点（名字剥后缀）；
    /// 目录优先，同类按系统区域排序。
    pub fn build_file_tree(&self, root: impl AsRef<Path>) -> Result<Vec<FileNode>> {
        let root = root.as_ref();
        if !root.is_dir() {
            bail!("目录不存在: {}", root.display());
        }
        build_children(root)
    }

    /// 首屏只枚举一层名称和文件类型；子目录在用户展开时再枚举。
    /// 不读取任何文件正文，适合包含海量文件的知识库。
    pub fn build_file_tree_shallow(&self, root: impl AsRef<Path>) -> Result<Vec<FileNode>> {
        let root = root.as_ref();
        if !root.is_dir() {
            bail!("目录不存在: {}", root.display());
        }
        build_children_shallow(root)
    }
}

/// 用同目录临时文件替换目标。
///
/// `std::fs::rename` 在 Unix 可以覆盖已有文件，但 Windows 会直接报
/// `AlreadyExists`。这会让任何已有笔记的自动保存失败（例如插入图片后写入
/// Markdown 引用）。Windows 使用 `ReplaceFileW`，既保留替换语义，也不会先
/// 删除原文件；其他平台保留标准库的原子 rename。
#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    if !destination.exists() {
        return fs::rename(temporary, destination);
    }
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

fn build_children(dir: &Path) -> Result<Vec<FileNode>> {
    let mut nodes = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if should_ignore(&name) {
                continue;
            }
            nodes.push(FileNode {
                id: path.to_string_lossy().into_owned(),
                name,
                path: path.to_string_lossy().into_owned(),
                kind: "directory".into(),
                children: Some(build_children(&path)?),
                ..Default::default()
            });
            continue;
        }

        if name.starts_with('.') {
            continue;
        }

        let meta = entry.metadata()?;
        let size = Some(meta.len() as i64);
        let mtime = meta.modified().ok().map(system_time_to_iso);

        if name.to_ascii_lowercase().ends_with(".link.json") {
            nodes.push(FileNode {
                id: path.to_string_lossy().into_owned(),
                name: name[..name.len() - ".link.json".len()].to_owned(),
                path: path.to_string_lossy().into_owned(),
                kind: "link".into(),
                size,
                mtime,
                ..Default::default()
            });
            continue;
        }

        nodes.push(FileNode {
            id: path.to_string_lossy().into_owned(),
            name,
            path: path.to_string_lossy().into_owned(),
            kind: "file".into(),
            size,
            mtime,
            created_at: meta.created().ok().map(system_time_to_iso),
            ..Default::default()
        });
    }

    nodes.sort_by(|a, b| {
        let a_dir = if a.is_directory() { 0 } else { 1 };
        let b_dir = if b.is_directory() { 0 } else { 1 };
        a_dir
            .cmp(&b_dir)
            .then_with(|| locale_compare(&a.name, &b.name))
    });
    Ok(nodes)
}

fn build_children_shallow(dir: &Path) -> Result<Vec<FileNode>> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let ty = entry.file_type()?;
        if ty.is_dir() {
            if should_ignore(&name) {
                continue;
            }
            nodes.push(FileNode {
                id: path.to_string_lossy().into_owned(),
                name,
                path: path.to_string_lossy().into_owned(),
                kind: "directory".into(),
                children: None,
                lazy: true,
                ..Default::default()
            });
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata()?;
        let size = Some(meta.len() as i64);
        let mtime = meta.modified().ok().map(system_time_to_iso);
        nodes.push(FileNode {
            id: path.to_string_lossy().into_owned(),
            name: name.strip_suffix(".link.json").unwrap_or(&name).to_owned(),
            path: path.to_string_lossy().into_owned(),
            kind: if name.to_ascii_lowercase().ends_with(".link.json") {
                "link"
            } else {
                "file"
            }
            .into(),
            size,
            mtime,
            created_at: meta.created().ok().map(system_time_to_iso),
            ..Default::default()
        });
    }
    nodes.sort_by(|a, b| {
        let a_dir = if a.is_directory() { 0 } else { 1 };
        let b_dir = if b.is_directory() { 0 } else { 1 };
        a_dir
            .cmp(&b_dir)
            .then_with(|| locale_compare(&a.name, &b.name))
    });
    Ok(nodes)
}

fn should_ignore(name: &str) -> bool {
    name.starts_with('.') || ignored_dirs().contains(name)
}

fn system_time_to_iso(t: std::time::SystemTime) -> String {
    jstime::from(DateTime::<Utc>::from(t))
}

/// 冲突命名：`"name 1.ext"`、`"name 2.ext"`…（上限 1000，与旧版一致）。
pub fn get_unique_file_path(directory: impl AsRef<Path>, desired_name: &str) -> Result<PathBuf> {
    let directory = directory.as_ref();
    let candidate = directory.join(desired_name);
    if !candidate.exists() {
        return Ok(candidate);
    }

    // 用字符串切分而非 Path::extension，才能正确处理 ".link.json" 这类复合后缀之外的
    // 常规情形，并与 C#/TS 的 GetFileNameWithoutExtension 语义一致（只剥最后一段）。
    let (base, ext) = match desired_name.rfind('.') {
        Some(i) if i > 0 => (&desired_name[..i], &desired_name[i..]),
        _ => (desired_name, ""),
    };

    for counter in 1..=1000 {
        let candidate = directory.join(format!("{base} {counter}{ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("无法生成唯一文件名")
}

fn append_ext(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// 区域敏感的字符串比较。中文按拼音而非码点排序，必须走系统排序规则，
/// 否则文件树顺序与 Electron 版（V8 的 `localeCompare`）对不上。
pub fn compare_names(a: &str, b: &str) -> Ordering {
    locale_compare(a, b)
}

#[cfg(windows)]
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    use windows_sys::Win32::Globalization::{CompareStringEx, CSTR_EQUAL, CSTR_GREATER_THAN};

    let wa: Vec<u16> = a.encode_utf16().collect();
    let wb: Vec<u16> = b.encode_utf16().collect();
    // LOCALE_NAME_USER_DEFAULT = null → 用户当前区域
    let r = unsafe {
        CompareStringEx(
            std::ptr::null(),
            0,
            wa.as_ptr(),
            wa.len() as i32,
            wb.as_ptr(),
            wb.len() as i32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    match r {
        0 => a.cmp(b), // 调用失败：退回码点序，至少保证是个全序
        CSTR_EQUAL => Ordering::Equal,
        CSTR_GREATER_THAN => Ordering::Greater,
        _ => Ordering::Less,
    }
}

#[cfg(not(windows))]
pub(crate) fn locale_compare(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn failed_atomic_replace_leaves_original_bytes_intact() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = TempDir::new("atomic-locked");
        let path = dir.0.join("note.md");
        fs::write(&path, "原文").unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        assert!(FileService::new()
            .write_file_safe(&path, "替换内容")
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "原文");
        drop(locked);
    }
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrd};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, AtomicOrd::Relaxed);
            let p = std::env::temp_dir().join(format!("mochi-fs-{}-{tag}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 期望值全部由 `node -e` 跑 save.ts 的原函数得出，逐字抄写。
    #[test]
    fn hash_matches_js_vectors() {
        assert_eq!(hash_content(""), "0");
        assert_eq!(hash_content("a"), "2p");
        assert_eq!(hash_content("hello world"), "to5x38");
        assert_eq!(hash_content("墨池笔记"), "bn142c");
        assert_eq!(hash_content("知识库/计算机通识"), "dacxbn");
        assert_eq!(hash_content("🎉 emoji"), "v104s3");
    }

    /// 负哈希必须带 `-` 前缀——C# 版的 Math.Abs 会在这里失配。
    #[test]
    fn negative_hashes_keep_their_sign() {
        assert_eq!(hash_content("The quick brown fox"), "-srk09p");
        assert_eq!(hash_content(&"x".repeat(1000)), "-sdbd34");
    }

    #[test]
    fn base36_edges_match_js() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(i32::MAX), "zik0zj");
        assert_eq!(to_base36(i32::MIN), "-zik0zk");
    }

    /// emoji 是验证"UTF-16 码元而非码点"的关键用例：
    /// `chars()` 会把 🎉 当 1 个单位，JS 当 2 个，结果必然不同。
    #[test]
    fn hash_iterates_utf16_units_not_code_points() {
        let by_code_point = {
            let mut h: i32 = 0;
            for c in "🎉 emoji".chars() {
                h = h.wrapping_shl(5).wrapping_sub(h).wrapping_add(c as i32);
            }
            to_base36(h)
        };
        assert_ne!(
            by_code_point,
            hash_content("🎉 emoji"),
            "两种遍历竟然一致？用例失效了"
        );
        assert_eq!(hash_content("🎉 emoji"), "v104s3");
    }

    #[test]
    fn write_creates_parents_and_removes_backup() {
        let d = TempDir::new("write");
        let svc = FileService::new();
        let f = d.0.join("a").join("b").join("note.md");

        svc.write_file_safe(&f, "内容").unwrap();

        assert_eq!(fs::read_to_string(&f).unwrap(), "内容");
        assert!(!append_ext(&f, ".backup").exists(), "备份没清掉");
    }

    #[test]
    fn write_is_utf8_without_bom() {
        let d = TempDir::new("bom");
        let svc = FileService::new();
        let f = d.0.join("n.md");
        svc.write_file_safe(&f, "墨池").unwrap();
        assert_eq!(fs::read(&f).unwrap(), "墨池".as_bytes());
    }

    #[test]
    fn write_replaces_existing_content() {
        let d = TempDir::new("replace-existing");
        let f = d.0.join("note.mc");
        fs::write(&f, "插入图片前").unwrap();

        FileService::new()
            .write_file_safe(&f, "插入图片后\n\n![](./assets/paste.png)")
            .unwrap();

        assert_eq!(
            fs::read_to_string(&f).unwrap(),
            "插入图片后\n\n![](./assets/paste.png)"
        );
        assert!(!append_ext(&f, ".backup").exists(), "成功后不应遗留备份");
    }

    #[test]
    fn enqueue_save_skips_identical_content() {
        let d = TempDir::new("skip");
        let svc = FileService::new();
        let f = d.0.join("n.md");

        assert!(svc.enqueue_save(&f, "v1").unwrap(), "首次应落盘");
        assert!(!svc.enqueue_save(&f, "v1").unwrap(), "内容未变应跳过");
        assert!(svc.enqueue_save(&f, "v2").unwrap(), "内容变了应落盘");
        assert_eq!(fs::read_to_string(&f).unwrap(), "v2");
    }

    #[test]
    fn file_tree_ignores_dot_and_build_dirs() {
        let d = TempDir::new("tree-ignore");
        for sub in [".mochi", "node_modules", "dist", "正常目录"] {
            fs::create_dir_all(d.0.join(sub)).unwrap();
        }
        fs::write(d.0.join(".hidden"), "x").unwrap();
        fs::write(d.0.join("a.md"), "x").unwrap();

        let tree = FileService::new().build_file_tree(&d.0).unwrap();
        let names: Vec<_> = tree.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["正常目录", "a.md"], "目录优先且忽略规则未生效");
    }

    #[test]
    fn link_json_becomes_link_node_with_suffix_stripped() {
        let d = TempDir::new("link");
        fs::write(d.0.join("我的收藏.link.json"), "{}").unwrap();

        let tree = FileService::new().build_file_tree(&d.0).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "我的收藏");
        assert_eq!(tree[0].kind, "link");
        assert!(tree[0].is_link());
    }

    #[test]
    fn file_tree_recurses_and_fills_metadata() {
        let d = TempDir::new("tree-deep");
        fs::create_dir_all(d.0.join("库").join("子")).unwrap();
        fs::write(d.0.join("库").join("子").join("x.md"), "hello").unwrap();

        let tree = FileService::new().build_file_tree(&d.0).unwrap();
        let lib = &tree[0];
        assert!(lib.is_directory());
        let sub = &lib.children.as_ref().unwrap()[0];
        let file = &sub.children.as_ref().unwrap()[0];
        assert_eq!(file.name, "x.md");
        assert_eq!(file.size, Some(5));
        assert!(file.mtime.as_ref().unwrap().ends_with('Z'));
    }

    #[test]
    fn directories_sort_before_files() {
        let d = TempDir::new("sort");
        fs::write(d.0.join("aaa.md"), "x").unwrap();
        fs::create_dir_all(d.0.join("zzz")).unwrap();

        let tree = FileService::new().build_file_tree(&d.0).unwrap();
        assert_eq!(tree[0].name, "zzz", "目录必须排在文件前面");
        assert_eq!(tree[1].name, "aaa.md");
    }

    #[test]
    fn unique_path_appends_counter() {
        let d = TempDir::new("unique");
        assert_eq!(
            get_unique_file_path(&d.0, "note.md").unwrap(),
            d.0.join("note.md")
        );

        fs::write(d.0.join("note.md"), "x").unwrap();
        assert_eq!(
            get_unique_file_path(&d.0, "note.md").unwrap(),
            d.0.join("note 1.md")
        );

        fs::write(d.0.join("note 1.md"), "x").unwrap();
        assert_eq!(
            get_unique_file_path(&d.0, "note.md").unwrap(),
            d.0.join("note 2.md")
        );
    }

    #[test]
    fn unique_path_handles_extensionless_names() {
        let d = TempDir::new("unique-noext");
        fs::create_dir_all(d.0.join("文件夹")).unwrap();
        assert_eq!(
            get_unique_file_path(&d.0, "文件夹").unwrap(),
            d.0.join("文件夹 1")
        );
    }

    #[test]
    fn locale_compare_is_a_total_order() {
        // 不断言具体的拼音顺序（依赖系统区域），只保证自反、反对称
        assert_eq!(locale_compare("阿", "阿"), Ordering::Equal);
        let ab = locale_compare("阿", "把");
        let ba = locale_compare("把", "阿");
        assert_ne!(ab, Ordering::Equal);
        assert_ne!(ab, ba);
    }
}
