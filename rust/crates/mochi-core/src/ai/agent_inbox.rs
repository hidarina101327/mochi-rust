//! 后台写操作先存提案。operation 保留未知字段和键序；inbox.json 写入尾换行。

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use crate::json2;

pub const INBOX_REL_PATH: &str = ".mochi/ai-agent/inbox.json";
const CLAIM_LEASE_MS: i64 = 2 * 60 * 1000;
const FILE_LOCK_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(30);
const FILE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

type InboxLock = Arc<Mutex<()>>;

/// 收件箱也会被原生应用的多个部分临时创建的服务值写入。因此把闸门
/// 放在 `AgentInboxService` 之外至关重要：只挂在服务上的闸门，仍会
/// 让两个实例互相丢掉对方的「读改写」更新。
static INBOX_LOCKS: OnceLock<Mutex<HashMap<PathBuf, InboxLock>>> = OnceLock::new();
static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn inbox_locks() -> &'static Mutex<HashMap<PathBuf, InboxLock>> {
    INBOX_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(not(windows))]
fn normalize_lock_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn normalize_lock_path(path: PathBuf) -> PathBuf {
    // Windows 上 canonicalize 常返回加长路径 `\\?\` 的写法，
    // 而 absolute() 返回普通的盘符写法。把两种形式（连同 Windows
    // 大小写不敏感的路径名）统一掉，inbox 目录还没创建时建的
    // 服务才能和之后建的服务共享同一把锁。
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\UNC\")
        .map(|unc| format!(r"\\{unc}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| text.into_owned());
    PathBuf::from(text.to_lowercase())
}

/// 为进程级锁表构造稳定键，且不要求收件箱文件已经存在。
/// 顺便 canonicalize 存在的父目录，让相对工作区和它的绝对写法
/// 共用同一道闸门。
fn lock_path(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut existing = absolute.clone();
    let mut suffix = Vec::<OsString>::new();
    let canonical = loop {
        match fs::canonicalize(&existing) {
            Ok(canonical) => break canonical,
            Err(_) => {
                let Some(name) = existing.file_name() else {
                    break existing;
                };
                suffix.push(name.to_owned());
                if !existing.pop() {
                    break existing;
                }
            }
        }
    };

    let mut canonical = canonical;
    for name in suffix.iter().rev() {
        canonical.push(name);
    }
    normalize_lock_path(canonical)
}

fn shared_lock(path: &Path) -> InboxLock {
    let key = lock_path(path);
    let mut locks = inbox_locks()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn temporary_path(target: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("inbox.json");
    parent.join(format!(".{name}.tmp-{}-{sequence}", std::process::id()))
}

/// 一个极简的跨进程闸门，用于原生的收件箱变更。`create_new` 就是
/// 原子获取动作；年龄守卫负责回收已终止进程留下的锁文件。收件箱
/// 变更都很短，活锁的年龄永远不该接近过期阈值。
struct InboxFileLock {
    path: PathBuf,
    _file: File,
}

impl InboxFileLock {
    fn acquire(inbox: &Path) -> Result<Self> {
        let path = inbox.with_extension("json.lock");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let deadline = std::time::Instant::now() + FILE_LOCK_WAIT;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(
                        file,
                        "pid={} acquiredAt={}",
                        std::process::id(),
                        crate::jstime::now_millis()
                    )?;
                    file.sync_all()?;
                    return Ok(Self { path, _file: file });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age >= FILE_LOCK_STALE_AFTER);
                    if stale {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(anyhow::anyhow!(
                            "收件箱正被另一个进程使用，请稍后重试: {}",
                            inbox.display()
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("创建收件箱进程锁失败: {}", path.display()))
                }
            }
        }
    }
}

impl Drop for InboxFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    // 源和目标在同一文件系统上时，rename(2) 能原子替换已存在的文件。
    // 临时文件总是建在目标旁边，这条前提在这里始终成立。
    fs::rename(source, destination)
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    // MoveFileExW 加 REPLACE_EXISTING 用一次文件系统操作完成目标替换，
    // 目标已存在时也不例外。
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// 一条待批准的操作。
#[derive(Debug, Clone, PartialEq)]
pub struct InboxEntry {
    fields: Map<String, Value>,
}

impl InboxEntry {
    pub fn from_value(v: &Value) -> Option<Self> {
        Some(Self {
            fields: v.as_object()?.clone(),
        })
    }

    pub fn to_value(&self) -> Value {
        Value::Object(self.fields.clone())
    }

    fn s(&self, key: &str) -> &str {
        self.fields
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    fn op(&self) -> Option<&Map<String, Value>> {
        self.fields.get("operation").and_then(Value::as_object)
    }

    fn op_s(&self, key: &str) -> &str {
        self.op()
            .and_then(|o| o.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub fn id(&self) -> &str {
        self.s("id")
    }

    pub fn created_at(&self) -> i64 {
        self.fields
            .get("createdAt")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }

    pub fn task_name(&self) -> Option<&str> {
        self.fields.get("taskName").and_then(Value::as_str)
    }

    /// `write` | `overwrite` | `create-folder` | `delete-file` | `delete-folder` | `rename`
    pub fn kind(&self) -> &str {
        self.op_s("kind")
    }

    pub fn path(&self) -> &str {
        self.op_s("path")
    }

    pub fn new_path(&self) -> Option<&str> {
        self.op()
            .and_then(|o| o.get("newPath"))
            .and_then(Value::as_str)
    }

    pub fn title(&self) -> &str {
        self.op_s("title")
    }

    pub fn summary(&self) -> &str {
        self.op_s("summary")
    }

    pub fn content(&self) -> Option<&str> {
        self.op()
            .and_then(|o| o.get("content"))
            .and_then(Value::as_str)
    }
    pub fn previous_content(&self) -> Option<&str> {
        self.op()
            .and_then(|o| o.get("previousContent"))
            .and_then(Value::as_str)
    }

    pub fn status(&self) -> &str {
        self.op_s("status")
    }

    pub fn claim_token(&self) -> Option<&str> {
        self.op()
            .and_then(|operation| operation.get("approvalClaimToken").and_then(Value::as_str))
    }

    fn claimed_at(&self) -> Option<i64> {
        self.op()
            .and_then(|operation| operation.get("approvalClaimedAt"))
            .and_then(Value::as_i64)
    }

    pub fn is_pending(&self) -> bool {
        self.status() == "pending"
    }

    fn set_status(&mut self, status: &str, error: Option<&str>) {
        if let Some(op) = self
            .fields
            .get_mut("operation")
            .and_then(Value::as_object_mut)
        {
            op.insert("status".into(), Value::String(status.to_owned()));
            if status != "running" {
                op.remove("approvalClaimToken");
                op.remove("approvalClaimedAt");
            }
            match error {
                Some(e) => {
                    op.insert("error".into(), Value::String(e.to_owned()));
                }
                None => {
                    op.remove("error");
                }
            }
        }
    }

    fn set_claim(&mut self, token: &str, claimed_at: i64) {
        if let Some(operation) = self
            .fields
            .get_mut("operation")
            .and_then(Value::as_object_mut)
        {
            operation.insert("status".into(), Value::String("running".into()));
            operation.remove("error");
            operation.insert("approvalClaimToken".into(), Value::String(token.to_owned()));
            operation.insert("approvalClaimedAt".into(), Value::from(claimed_at));
        }
    }
}

fn is_stale_claim(entry: &InboxEntry, now: i64) -> bool {
    entry.status() == "running"
        && entry
            .claimed_at()
            .is_none_or(|claimed_at| now.saturating_sub(claimed_at) >= CLAIM_LEASE_MS)
}

pub struct AgentInboxService {
    path: PathBuf,
    lock: InboxLock,
}

impl AgentInboxService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        let path = workspace_path.as_ref().join(INBOX_REL_PATH);
        Self {
            lock: shared_lock(&path),
            path,
        }
    }

    fn with_lock<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        let _file_guard = InboxFileLock::acquire(&self.path)?;
        operation()
    }

    /// 读取一份合法的收件箱用于变更。收件箱缺失是正常的空状态；
    /// 但存在却损坏就是错误：悄悄把损坏当成空，下一次变更就会
    /// 覆盖掉本可以抢救回来的用户数据。
    fn read_strict(&self) -> Result<Vec<InboxEntry>> {
        let raw = match fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("读取收件箱失败: {}", self.path.display()))
            }
        };
        let value: Value = serde_json::from_str(&raw)
            .with_context(|| format!("收件箱 JSON 损坏: {}", self.path.display()))?;
        let entries = value
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("收件箱缺少有效的 entries 数组"))?;

        entries
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let entry = InboxEntry::from_value(value)
                    .ok_or_else(|| anyhow::anyhow!("收件箱第 {} 条记录不是对象", index + 1))?;
                if !entry.fields.get("id").is_some_and(Value::is_string) {
                    return Err(anyhow::anyhow!(
                        "收件箱第 {} 条记录缺少有效的 id",
                        index + 1
                    ));
                }
                if !entry.fields.get("operation").is_some_and(Value::is_object) {
                    return Err(anyhow::anyhow!(
                        "收件箱第 {} 条记录缺少有效的 operation",
                        index + 1
                    ));
                }
                Ok(entry)
            })
            .collect()
    }

    /// 列表 API 用的尽力而为读取。列出收件箱属于诊断/只读操作，
    /// 为了与渲染端兼容，损坏或读不了的存储一律呈现为空列表。
    fn read(&self) -> Vec<InboxEntry> {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            return Vec::new();
        };
        // 坏一条丢一条，不让整个收件箱失效（TS 是 Array.isArray 兜底 + 逐条用）
        v.get("entries")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(InboxEntry::from_value).collect())
            .unwrap_or_default()
    }

    fn write(&self, entries: &[InboxEntry]) -> Result<()> {
        // 这条不变量要在写入边界和每个公开的读改写入口都守住，
        // 保护以后新加的变更方法不会一不小心把损坏文件变成空收件箱。
        self.read_strict()?;

        let mut file = Map::new();
        file.insert("version".into(), Value::from(1));
        file.insert(
            "entries".into(),
            Value::Array(entries.iter().map(|e| e.to_value()).collect()),
        );
        let text = format!("{}\n", json2::serialize(&Value::Object(file))?);

        let dir = self
            .path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir)?;

        // 绝不以写入方式打开目标文件。只有当生成新内容的所有可能
        // 失败的步骤都成功之后，才把一份完整、已 sync 的同目录
        // 兄弟文件 rename 覆盖过去。
        let temporary = temporary_path(&self.path);
        let result = (|| -> std::io::Result<()> {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            output.write_all(text.as_bytes())?;
            output.sync_all()?;
            drop(output);
            atomic_replace(&temporary, &self.path)
        })();

        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(anyhow::Error::new(error)
                .context(format!("原子写入收件箱失败: {}", self.path.display())));
        }
        Ok(())
    }

    /// 追加一条提案（`addInboxEntry`）。`operation` 是 TS 形状的 `PendingFileOperation` 对象。
    /// id 形状 `inbox-<ms>-<6 位 base36>`，与 TS 一致。
    pub fn add(
        &self,
        operation: Value,
        task_id: Option<&str>,
        task_name: Option<&str>,
    ) -> Result<InboxEntry> {
        if !operation.is_object() {
            return Err(anyhow::anyhow!("收件箱 operation 必须是对象"));
        }
        let mut fields = Map::new();
        let now = crate::jstime::now_millis();
        fields.insert(
            "id".into(),
            Value::String(format!("inbox-{now}-{}", crate::paths::random_base36(6))),
        );
        fields.insert("createdAt".into(), Value::from(now));
        fields.insert(
            "taskId".into(),
            task_id
                .map(|s| Value::String(s.to_owned()))
                .unwrap_or(Value::Null),
        );
        fields.insert(
            "taskName".into(),
            task_name
                .map(|s| Value::String(s.to_owned()))
                .unwrap_or(Value::Null),
        );
        fields.insert("operation".into(), operation);
        let entry = InboxEntry { fields };
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            entries.push(entry.clone());
            self.write(&entries)?;
            Ok(entry)
        })
    }

    /// 追加一条（`addInboxEntry`）。调用方给完整的条目对象（`id/createdAt/taskId/taskName/operation`）。
    pub fn add_raw(&self, entry: &Value) -> Result<InboxEntry> {
        let entry =
            InboxEntry::from_value(entry).ok_or_else(|| anyhow::anyhow!("收件箱条目必须是对象"))?;
        if !entry.fields.get("id").is_some_and(Value::is_string)
            || !entry.fields.get("operation").is_some_and(Value::is_object)
        {
            return Err(anyhow::anyhow!("收件箱条目缺少有效的 id 或 operation"));
        }
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            entries.push(entry.clone());
            self.write(&entries)?;
            Ok(entry)
        })
    }

    /// 只列 pending 的（`listInboxEntries`）。
    pub fn list_pending(&self) -> Vec<InboxEntry> {
        let mut entries = self.read();
        if entries
            .iter()
            .any(|entry| is_stale_claim(entry, crate::jstime::now_millis()))
        {
            let _ = self.recover_stale_claims();
            entries = self.read();
        }
        entries.into_iter().filter(|e| e.is_pending()).collect()
    }

    pub fn count_pending(&self) -> usize {
        self.list_pending().len()
    }

    /// 标记为已处理（`applied` / `rejected` / `error`）。
    pub fn resolve(&self, entry_id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            if let Some(e) = entries.iter_mut().find(|e| e.id() == entry_id) {
                e.set_status(status, error);
                self.write(&entries)?;
            }
            Ok(())
        })
    }

    /// 用一次读改写周期处理一批收件箱条目的快照。
    ///
    /// UI 在把一组文档当作一个文件事务应用之后调用它。保留各条目
    /// 既保住了审计和会话身份，也避免几十次互相竞争、丢信息的
    /// `inbox.json` 重写。
    pub fn resolve_many(
        &self,
        entry_ids: &[String],
        status: &str,
        error: Option<&str>,
    ) -> Result<usize> {
        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            if wanted.is_empty() {
                return Ok(0);
            }
            let mut resolved = 0;
            for entry in &mut entries {
                if wanted.contains(entry.id()) {
                    entry.set_status(status, error);
                    resolved += 1;
                }
            }
            if resolved > 0 {
                self.write(&entries)?;
            }
            Ok(resolved)
        })
    }

    /// 只有整组 ID 的状态都仍是预期值时才整体流转。
    /// 适合「不执行、只记账」的决策，比如记录一条校验错误
    /// 而不去覆盖一个已生效的 claim。
    pub fn resolve_many_if_status(
        &self,
        entry_ids: &[String],
        expected_status: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<usize> {
        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            if wanted.is_empty() {
                return Ok(0);
            }
            let all_expected = wanted.iter().all(|id| {
                entries
                    .iter()
                    .any(|entry| entry.id() == *id && entry.status() == expected_status)
            });
            if !all_expected {
                return Ok(0);
            }
            let mut resolved = 0;
            for entry in &mut entries {
                if wanted.contains(entry.id()) {
                    entry.set_status(status, error);
                    resolved += 1;
                }
            }
            self.write(&entries)?;
            Ok(resolved)
        })
    }

    /// 把请求的全部 pending 条目作为一个事务领走。
    ///
    /// 返回的是领走时的快照（因此 `operation.status == "running"`）。
    /// 只要有一个请求的 ID 缺失或已不再是 pending，就返回空向量，
    /// 所有条目原样不动。重复的 ID 按一个请求算。
    pub fn claim_many_pending(&self, entry_ids: &[String]) -> Result<Vec<InboxEntry>> {
        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            if wanted.is_empty() {
                return Ok(Vec::new());
            }

            let all_pending = wanted.iter().all(|id| {
                entries
                    .iter()
                    .any(|entry| entry.id() == *id && entry.is_pending())
            });
            if !all_pending {
                return Ok(Vec::new());
            }

            let claimed_at = crate::jstime::now_millis();
            let token = format!("claim-{claimed_at}-{}", crate::paths::random_base36(12));
            let mut claimed = Vec::with_capacity(wanted.len());
            for entry in &mut entries {
                if wanted.contains(entry.id()) {
                    entry.set_claim(&token, claimed_at);
                    claimed.push(entry.clone());
                }
            }
            self.write(&entries)?;
            Ok(claimed)
        })
    }

    /// 只需要「领了几条」的调用方用这个便捷形式。
    pub fn claim_many_pending_count(&self, entry_ids: &[String]) -> Result<usize> {
        self.claim_many_pending(entry_ids)
            .map(|entries| entries.len())
    }

    /// 把领走的条目落到最终状态。刻意做成 `resolve` 的别名：
    /// 旧调用方可以直接调用 resolve；新的认领工作线程则使用
    /// `claim`/`release` 这套生命周期词汇。
    pub fn release(&self, entry_id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.resolve(entry_id, status, error)
    }

    /// 对一组已同时接手的条目执行批量 release。
    pub fn release_many(
        &self,
        entry_ids: &[String],
        status: &str,
        error: Option<&str>,
    ) -> Result<usize> {
        self.resolve_many(entry_ids, status, error)
    }

    /// 只有当所有请求条目仍由给定令牌持有时，才允许完结或放弃 claim。
    /// 过期的工作线程不可能覆盖更新的审批决定。
    pub fn release_many_claimed(
        &self,
        entry_ids: &[String],
        claim_token: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<usize> {
        let wanted: HashSet<&str> = entry_ids.iter().map(String::as_str).collect();
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            if wanted.is_empty() {
                return Ok(0);
            }
            let all_owned = wanted.iter().all(|id| {
                entries.iter().any(|entry| {
                    entry.id() == *id
                        && entry.status() == "running"
                        && entry.claim_token() == Some(claim_token)
                })
            });
            if !all_owned {
                return Ok(0);
            }
            let mut resolved = 0;
            for entry in &mut entries {
                if wanted.contains(entry.id()) {
                    entry.set_status(status, error);
                    resolved += 1;
                }
            }
            self.write(&entries)?;
            Ok(resolved)
        })
    }

    /// 经过一小段租期后，把已终止进程遗留的 claim 归还给可见列表。
    /// 每次重试时，文件操作都会重新校验。
    pub fn recover_stale_claims(&self) -> Result<usize> {
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            let now = crate::jstime::now_millis();
            let mut recovered = 0;
            for entry in &mut entries {
                if is_stale_claim(entry, now) {
                    entry.set_status("pending", Some("上次审批执行中断，请重新确认"));
                    recovered += 1;
                }
            }
            if recovered > 0 {
                self.write(&entries)?;
            }
            Ok(recovered)
        })
    }

    pub fn remove(&self, entry_id: &str) -> Result<()> {
        self.with_lock(|| {
            let mut entries = self.read_strict()?;
            let before = entries.len();
            entries.retain(|e| e.id() != entry_id);
            if entries.len() != before {
                self.write(&entries)?;
            }
            Ok(())
        })
    }

    /// 清掉所有已处理的条目，返回清掉几条。
    pub fn prune(&self) -> Result<usize> {
        self.with_lock(|| {
            let entries = self.read_strict()?;
            let before = entries.len();
            let kept: Vec<InboxEntry> = entries.into_iter().filter(|e| e.is_pending()).collect();
            let removed = before - kept.len();
            if removed > 0 {
                self.write(&kept)?;
            }
            Ok(removed)
        })
    }

    /// 真正执行一条已批准的操作（`applyPendingFileOperation` 的文件部分）。
    /// 返回一句话摘要，失败时返回错误文字。
    pub fn apply(&self, entry: &InboxEntry) -> Result<String, String> {
        let path = Path::new(entry.path());
        match entry.kind() {
            "write" | "overwrite" => {
                let content = entry.content().unwrap_or_default();
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                crate::files::FileService::new()
                    .write_file_safe(path, content)
                    .map_err(|e| e.to_string())?;
                Ok(format!("已写入 {}", entry.path()))
            }
            "create-folder" => {
                std::fs::create_dir_all(path).map_err(|e| e.to_string())?;
                Ok(format!("已创建文件夹 {}", entry.path()))
            }
            "delete-file" => {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                Ok(format!("已删除 {}", entry.path()))
            }
            "delete-folder" => {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())?;
                Ok(format!("已删除文件夹 {}", entry.path()))
            }
            "rename" => {
                let to = entry.new_path().ok_or("rename 缺少 newPath")?;
                std::fs::rename(path, to).map_err(|e| e.to_string())?;
                Ok(format!("已重命名为 {to}"))
            }
            other => Err(format!("未知的操作类型 {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("mochi-inbox-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    // 样本里有 `"# a\n"`，`r#"…"#` 会在 `"#` 处提前结束，所以用两个井号
    const SAMPLE: &str = r##"{
  "version": 1,
  "entries": [
    {
      "id": "inbox-1",
      "createdAt": 1756800000000,
      "taskId": "t1",
      "taskName": "每日整理",
      "operation": {
        "id": "op-1",
        "kind": "write",
        "path": "PATH",
        "title": "写入 a.md",
        "summary": "新建一篇笔记",
        "content": "# a\n",
        "reason": "permission-suggest",
        "status": "pending",
        "extraFieldFromFuture": 42
      }
    },
    {
      "id": "inbox-2",
      "createdAt": 1756800001000,
      "taskId": null,
      "taskName": null,
      "operation": {
        "id": "op-2",
        "kind": "delete-file",
        "path": "x.md",
        "title": "删除 x.md",
        "summary": "",
        "reason": "destructive",
        "status": "rejected"
      }
    }
  ]
}
"##;

    fn operation(status: &str) -> Value {
        serde_json::json!({
            "id": "op-test",
            "kind": "write",
            "path": "test.md",
            "title": "测试",
            "summary": "测试收件箱",
            "content": "content",
            "status": status,
        })
    }

    fn raw_entry(id: &str, status: &str) -> Value {
        serde_json::json!({
            "id": id,
            "createdAt": 1,
            "taskId": null,
            "taskName": null,
            "operation": operation(status),
        })
    }

    #[test]
    fn only_pending_entries_are_listed_and_unknown_fields_survive_a_round_trip() {
        let ws = temp("roundtrip");
        let file = ws.join(INBOX_REL_PATH);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, SAMPLE).unwrap();
        let svc = AgentInboxService::new(&ws);
        let pending = svc.list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].title(), "写入 a.md");
        assert_eq!(pending[0].task_name(), Some("每日整理"));

        // 写回：未知字段、键序、尾换行都要保住
        svc.resolve("inbox-1", "pending", None).unwrap();
        let after = std::fs::read_to_string(&file).unwrap();
        assert!(after.contains("\"extraFieldFromFuture\": 42"));
        assert!(after.ends_with("}\n"));
        assert_eq!(after, SAMPLE, "没有实质改动时应逐字节一致");
    }

    #[test]
    fn resolving_updates_the_status_and_prune_drops_the_settled_ones() {
        let ws = temp("resolve");
        let file = ws.join(INBOX_REL_PATH);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, SAMPLE).unwrap();
        let svc = AgentInboxService::new(&ws);
        svc.resolve("inbox-1", "error", Some("磁盘满")).unwrap();
        assert_eq!(svc.count_pending(), 0);
        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(raw.contains("\"error\": \"磁盘满\""));
        assert_eq!(svc.prune().unwrap(), 2);
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("\"entries\": []"));
    }

    #[test]
    fn resolving_many_updates_one_snapshot_and_preserves_unknown_entries() {
        let ws = temp("resolve-many");
        let file = ws.join(INBOX_REL_PATH);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut value: Value = serde_json::from_str(SAMPLE).unwrap();
        let mut second = value["entries"][0].clone();
        second["id"] = "inbox-3".into();
        second["operation"]["id"] = "op-3".into();
        value["entries"].as_array_mut().unwrap().push(second);
        let mut unrelated = value["entries"][0].clone();
        unrelated["id"] = "inbox-4".into();
        unrelated["operation"]["id"] = "op-4".into();
        value["entries"].as_array_mut().unwrap().push(unrelated);
        std::fs::write(&file, format!("{}\n", json2::serialize(&value).unwrap())).unwrap();

        let svc = AgentInboxService::new(&ws);
        assert_eq!(
            svc.resolve_many(
                &[
                    "inbox-1".to_owned(),
                    "inbox-3".to_owned(),
                    "missing".to_owned()
                ],
                "rejected",
                None,
            )
            .unwrap(),
            2
        );
        assert_eq!(
            svc.list_pending()
                .iter()
                .map(InboxEntry::id)
                .collect::<Vec<_>>(),
            vec!["inbox-4"]
        );
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(stored["entries"][0]["operation"]["status"], "rejected");
        assert_eq!(stored["entries"][1]["operation"]["status"], "rejected");
        assert_eq!(stored["entries"][2]["operation"]["status"], "rejected");
        assert_eq!(stored["entries"][3]["operation"]["status"], "pending");
    }

    #[test]
    fn applying_a_write_creates_the_file_and_a_missing_inbox_is_just_empty() {
        let ws = temp("apply");
        let svc = AgentInboxService::new(&ws);
        assert!(svc.list_pending().is_empty(), "没有文件时不该报错");
        let target = ws.join("笔记/a.md");
        let sample = SAMPLE.replace("PATH", &target.to_string_lossy().replace('\\', "/"));
        let v: Value = serde_json::from_str(&sample).unwrap();
        let entry = InboxEntry::from_value(&v["entries"][0]).unwrap();
        let summary = svc.apply(&entry).unwrap();
        assert!(summary.starts_with("已写入"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "# a\n");
    }

    #[test]
    fn corrupt_inbox_is_rejected_by_every_mutation_and_stays_unchanged() {
        let ws = temp("corrupt");
        let file = ws.join(INBOX_REL_PATH);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = "{\"version\":1,\"entries\":[";
        std::fs::write(&file, original).unwrap();
        let svc = AgentInboxService::new(&ws);

        assert!(svc.list_pending().is_empty());
        assert!(svc.add(operation("pending"), None, None).is_err());
        assert!(svc.add_raw(&raw_entry("new", "pending")).is_err());
        assert!(svc.resolve("missing", "rejected", None).is_err());
        assert!(svc
            .resolve_many(&["missing".to_owned()], "rejected", None)
            .is_err());
        assert!(svc.claim_many_pending(&["missing".to_owned()]).is_err());
        assert!(svc.remove("missing").is_err());
        assert!(svc.prune().is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    }

    #[test]
    fn claim_many_pending_is_all_or_nothing_and_release_handles_running_entries() {
        let ws = temp("claim");
        let svc = AgentInboxService::new(&ws);
        for (id, status) in [("one", "pending"), ("two", "pending"), ("done", "rejected")] {
            svc.add_raw(&raw_entry(id, status)).unwrap();
        }

        let file = ws.join(INBOX_REL_PATH);
        let before = std::fs::read_to_string(&file).unwrap();
        assert!(svc
            .claim_many_pending(&["one".to_owned(), "missing".to_owned()])
            .unwrap()
            .is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
        assert_eq!(svc.list_pending().len(), 2);

        let claimed = svc
            .claim_many_pending(&["one".to_owned(), "two".to_owned()])
            .unwrap();
        assert_eq!(claimed.len(), 2);
        assert!(claimed.iter().all(|entry| entry.status() == "running"));
        let token = claimed[0].claim_token().unwrap().to_owned();
        assert!(claimed
            .iter()
            .all(|entry| entry.claim_token() == Some(token.as_str())));
        assert!(svc.list_pending().is_empty());

        assert_eq!(
            svc.release_many_claimed(
                &["one".to_owned(), "two".to_owned()],
                "foreign-claim",
                "rejected",
                None,
            )
            .unwrap(),
            0,
            "另一个审批流程不能结束当前 claim"
        );
        assert_eq!(
            svc.release_many_claimed(
                &["one".to_owned(), "two".to_owned()],
                &token,
                "rejected",
                Some("用户拒绝"),
            )
            .unwrap(),
            2
        );
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(stored["entries"][0]["operation"]["status"], "rejected");
        assert_eq!(stored["entries"][1]["operation"]["status"], "rejected");
        assert_eq!(stored["entries"][1]["operation"]["error"], "用户拒绝");
        assert!(stored["entries"][0]["operation"]
            .get("approvalClaimToken")
            .is_none());
    }

    #[test]
    fn conditional_status_transition_never_overwrites_another_decision() {
        let ws = temp("conditional-resolve");
        let svc = AgentInboxService::new(&ws);
        svc.add_raw(&raw_entry("pending", "pending")).unwrap();
        svc.add_raw(&raw_entry("settled", "applied")).unwrap();

        assert_eq!(
            svc.resolve_many_if_status(
                &["pending".to_owned(), "settled".to_owned()],
                "pending",
                "rejected",
                None,
            )
            .unwrap(),
            0
        );
        assert_eq!(svc.list_pending().len(), 1);
        assert_eq!(
            svc.resolve_many_if_status(
                &["pending".to_owned()],
                "pending",
                "error",
                Some("验证失败"),
            )
            .unwrap(),
            1
        );
        let entries = svc.read_strict().unwrap();
        assert_eq!(entries[0].status(), "error");
        assert_eq!(entries[1].status(), "applied");
    }

    #[test]
    fn concurrent_claimers_have_one_owner_and_stale_claims_recover() {
        use std::sync::{Arc, Barrier};

        let ws = temp("claim-owner");
        AgentInboxService::new(&ws)
            .add_raw(&raw_entry("one", "pending"))
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let workspace = ws.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    AgentInboxService::new(workspace)
                        .claim_many_pending(&["one".to_owned()])
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let claims = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(claims.iter().filter(|claim| claim.len() == 1).count(), 1);
        assert_eq!(claims.iter().filter(|claim| claim.is_empty()).count(), 1);

        let file = ws.join(INBOX_REL_PATH);
        let mut stored: Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        stored["entries"][0]["operation"]["approvalClaimedAt"] = Value::from(0_u64);
        std::fs::write(&file, format!("{}\n", json2::serialize(&stored).unwrap())).unwrap();

        let pending = AgentInboxService::new(&ws).list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].status(), "pending");
        assert!(pending[0].claim_token().is_none());
        assert_eq!(
            pending[0].to_value()["operation"]["error"],
            "上次审批执行中断，请重新确认"
        );
    }

    #[test]
    fn concurrent_add_and_resolve_across_service_instances_keep_all_updates() {
        use std::sync::{Arc, Barrier};

        let ws = temp("concurrent");
        let seed = AgentInboxService::new(&ws);
        let seeded = (0..16)
            .map(|index| {
                seed.add_raw(&raw_entry(&format!("seed-{index}"), "pending"))
                    .unwrap()
                    .id()
                    .to_owned()
            })
            .collect::<Vec<_>>();

        let barrier = Arc::new(Barrier::new(32));
        let mut workers = Vec::new();
        let worker_inputs = seeded
            .iter()
            .cloned()
            .map(Some)
            .chain((16..32).map(|_| None));
        for (index, resolve_id) in worker_inputs.enumerate() {
            let workspace = ws.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                let service = AgentInboxService::new(workspace);
                if let Some(id) = resolve_id {
                    service.resolve(&id, "applied", None).unwrap();
                } else {
                    service
                        .add_raw(&raw_entry(&format!("new-{index}"), "pending"))
                        .unwrap();
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let entries = AgentInboxService::new(&ws).read_strict().unwrap();
        assert_eq!(entries.len(), 32);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.status() == "applied")
                .count(),
            16
        );
        assert_eq!(AgentInboxService::new(&ws).count_pending(), 16);
    }

    #[test]
    fn failed_atomic_replace_leaves_the_existing_inbox_intact() {
        let ws = temp("atomic");
        let file = ws.join(INBOX_REL_PATH);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let original = "old inbox\n";
        std::fs::write(&file, original).unwrap();
        let missing_temporary = file.with_file_name(".inbox.json.missing-temp");

        assert!(atomic_replace(&missing_temporary, &file).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);

        let svc = AgentInboxService::new(&ws);
        // 成功写入后也不该留下写了一半的兄弟文件。旧的、故意写坏的
        // 文件，只有先修好，才能走正常的变更路径替换。
        std::fs::write(&file, "{\"version\":1,\"entries\":[]}\n").unwrap();
        svc.add(operation("pending"), None, None).unwrap();
        let temporary_left = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().to_string_lossy().contains(".tmp-"));
        assert!(!temporary_left);
        assert!(!std::fs::read_to_string(&file).unwrap().is_empty());
    }
}
