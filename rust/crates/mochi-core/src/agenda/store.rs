//! 落盘：`<工作区>/agenda/agenda.json`（整份数据，原子替换）+ `agenda/log.jsonl`（操作日志）。
//!
//! 所有「读 → 改 → 写」都在进程级可重入锁里完成，UI、定时器、AI 工具、桌面卡片
//! 同时写也不会互相覆盖。

use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::change::ChangeSet;
use super::legacy;
use super::model::AgendaData;
use super::ops::{Clock, Editor};

pub const DIR_NAME: &str = "agenda";
const DATA_FILE: &str = "agenda.json";
const LOG_FILE: &str = "log.jsonl";

static WRITE_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static LOCK_HELD: Cell<bool> = const { Cell::new(false) };
}

/// 串行化本进程内所有日程待办写入。按线程可重入。
pub fn with_write_lock<R>(f: impl FnOnce() -> R) -> R {
    if LOCK_HELD.with(Cell::get) {
        return f();
    }
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            LOCK_HELD.with(|held| held.set(false));
        }
    }
    LOCK_HELD.with(|held| held.set(true));
    let _release = Release;
    f()
}

/// 谁发起的修改，写进日志。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    User,
    Ai,
    Card,
    System,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "我",
            Self::Ai => "墨池 AI",
            Self::Card => "桌面卡片",
            Self::System => "系统",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: String,
    pub source: Source,
    pub lines: Vec<String>,
}

/// 文件指纹，用来判断磁盘上的数据是否被别处改过。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

#[derive(Debug, Clone)]
pub struct AgendaStore {
    workspace: PathBuf,
}

impl AgendaStore {
    pub fn new(workspace: impl AsRef<Path>) -> Self {
        Self {
            workspace: workspace.as_ref().to_path_buf(),
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn dir(&self) -> PathBuf {
        self.workspace.join(DIR_NAME)
    }

    pub fn data_path(&self) -> PathBuf {
        self.dir().join(DATA_FILE)
    }

    fn log_path(&self) -> PathBuf {
        self.dir().join(LOG_FILE)
    }

    pub fn stamp(&self) -> Stamp {
        std::fs::metadata(self.data_path())
            .map(|m| Stamp {
                modified: m.modified().ok(),
                len: m.len(),
            })
            .unwrap_or_default()
    }

    /// 读取数据。第一次使用时若存在旧版日程文件则迁移过来。
    pub fn load(&self) -> Result<AgendaData> {
        let path = self.data_path();
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("读取日程待办失败: {}", path.display()))?;
            if text.trim().is_empty() {
                return Ok(AgendaData::default());
            }
            return serde_json::from_str(&text)
                .with_context(|| format!("日程待办数据损坏: {}", path.display()));
        }
        let legacy_dir = self.workspace.join(crate::paths::SCHEDULE_DIR_NAME);
        if legacy::has_legacy(&legacy_dir) {
            return with_write_lock(|| {
                if path.is_file() {
                    return self.load();
                }
                let data = legacy::migrate(&legacy_dir);
                self.save(&data)?;
                self.append_log(Source::System, vec!["从旧版日程迁移数据".into()]);
                Ok(data)
            });
        }
        Ok(AgendaData::default())
    }

    pub fn save(&self, data: &AgendaData) -> Result<()> {
        let bytes = format!("{}\n", serde_json::to_string_pretty(data)?).into_bytes();
        let path = self.data_path();
        if std::fs::read(&path).is_ok_and(|existing| existing == bytes) {
            return Ok(());
        }
        atomic_write(&path, &bytes)
    }

    /// 在锁内读取最新数据、执行修改、写回，返回修改结果和变更集。
    /// 闭包出错时什么都不写。
    pub fn mutate<R>(
        &self,
        source: Source,
        f: impl FnOnce(&mut Editor<'_>) -> Result<R>,
    ) -> Result<(R, ChangeSet)> {
        with_write_lock(|| {
            let before = self.load()?;
            let mut after = before.clone();
            let result = {
                let mut editor = Editor::new(&mut after, Clock::system());
                f(&mut editor)?
            };
            let change = ChangeSet::diff(&before, &after);
            if !change.is_empty() {
                self.save(&after)?;
                self.append_log(source, change.describe(&before, &after));
            }
            Ok((result, change))
        })
    }

    /// 应用一组变更（撤销、重做、批准的 AI 提案）。`strict` 时核对冲突。
    pub fn apply(&self, source: Source, change: &ChangeSet, strict: bool) -> Result<AgendaData> {
        with_write_lock(|| {
            let before = self.load()?;
            let mut after = before.clone();
            change.apply(&mut after, strict)?;
            self.save(&after)?;
            self.append_log(source, change.describe(&before, &after));
            Ok(after)
        })
    }

    fn append_log(&self, source: Source, lines: Vec<String>) {
        let lines: Vec<String> = lines.into_iter().filter(|l| !l.is_empty()).collect();
        if lines.is_empty() {
            return;
        }
        let entry = LogEntry {
            at: crate::jstime::now(),
            source,
            lines,
        };
        let Ok(json) = serde_json::to_string(&entry) else {
            return;
        };
        let path = self.log_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // 日志超过 2 MB 时只保留后半部分。
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let lines: Vec<&str> = text.lines().collect();
                let keep = lines[lines.len() / 2..].join("\n");
                let _ = atomic_write(&path, format!("{keep}\n").as_bytes());
            }
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(file, "{json}");
        }
    }

    /// 最近的操作日志，新的在前。
    pub fn history(&self, limit: usize) -> Vec<LogEntry> {
        let Ok(text) = std::fs::read_to_string(self.log_path()) else {
            return Vec::new();
        };
        text.lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<LogEntry>(line).ok())
            .take(limit)
            .collect()
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("文件名无效: {}", path.display()))?;
    let temporary = path.with_file_name(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("创建临时文件失败: {}", temporary.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Err(error) = std::fs::rename(&temporary, path) {
            // 有些 Windows 安全软件不允许覆盖式改名：退回到直接覆盖写。
            if cfg!(windows) && path.exists() {
                std::fs::write(path, bytes)
                    .with_context(|| format!("写入失败: {}", path.display()))?;
                let _ = std::fs::remove_file(&temporary);
                return Ok(());
            }
            return Err(error).with_context(|| format!("替换文件失败: {}", path.display()));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
