//! gitdir 使用 .mochi/git，沿用 isomorphic-git 的仓库。保存操作入队合并提交，不逐次提交。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;
use git2::{ConfigLevel, Repository, RepositoryInitOptions, Signature, Status, StatusOptions};

use crate::paths;

pub const DEFAULT_AUTHOR_NAME: &str = "Mochi User";
pub const DEFAULT_AUTHOR_EMAIL: &str = "user@mochi.local";
pub const DEFAULT_BRANCH: &str = "main";
pub const MAX_PENDING_OPERATIONS: usize = 50;

const KNOWN_PREFIXES: &[&str] = &[
    "auto",
    "ai-before",
    "ai-after",
    "manual",
    "restore",
    "rename",
    "delete",
    "import",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionOpType {
    Write,
    Rename,
    Delete,
    Mkdir,
    Copy,
    Restore,
}

/// 待版本化操作（对应旧版 `PendingVersionOperation`，队列上限 50）。
#[derive(Debug, Clone)]
pub struct PendingVersionOperation {
    pub kind: VersionOpType,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

impl PendingVersionOperation {
    pub fn write(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: VersionOpType::Write,
            path: path.into(),
            old_path: None,
        }
    }
    pub fn rename(old_path: impl Into<PathBuf>, path: impl Into<PathBuf>) -> Self {
        Self {
            kind: VersionOpType::Rename,
            path: path.into(),
            old_path: Some(old_path.into()),
        }
    }
    pub fn delete(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: VersionOpType::Delete,
            path: path.into(),
            old_path: None,
        }
    }
}

/// 提交历史条目。
#[derive(Debug, Clone)]
pub struct GitCommitInfo {
    pub oid: String,
    pub short_oid: String,
    pub message: String,
    pub kind: String,
    pub is_key_node: bool,
    pub author_name: String,
    pub timestamp_ms: i64,
}

/// 消息格式化：`${type}: ${message}`；`other` 类型不加前缀（与旧版一致）。
pub fn format_commit_message(message: &str, kind: &str) -> String {
    let clean = message.trim();
    if kind == "other" {
        clean.to_owned()
    } else {
        format!("{kind}: {clean}")
    }
}

/// 从消息前缀解析提交类型（无已知前缀 → `other`）。
pub fn get_commit_type(message: &str) -> String {
    match message.find(':') {
        Some(i) if i > 0 => {
            let prefix = message[..i].to_lowercase();
            if KNOWN_PREFIXES.contains(&prefix.as_str()) {
                prefix
            } else {
                "other".into()
            }
        }
        _ => "other".into(),
    }
}

/// 关键节点（版本浏览 UI 高亮）：manual/ai-before/ai-after/restore/rename。
pub fn is_key_node(kind: &str) -> bool {
    matches!(
        kind,
        "manual" | "ai-before" | "ai-after" | "restore" | "rename"
    )
}

#[derive(Debug, Clone)]
pub struct GitSettings {
    pub version_history_enabled: bool,
    pub auto_commit_enabled: bool,
    pub auto_commit_interval_minutes: u64,
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            version_history_enabled: true,
            auto_commit_enabled: true,
            auto_commit_interval_minutes: 10,
        }
    }
}

pub struct GitService {
    workspace_root: PathBuf,
    git_dir: PathBuf,
    pending: Mutex<Vec<PendingVersionOperation>>,
    settings: Mutex<GitSettings>,
    author: Mutex<(String, String)>,
    auto_commit: Mutex<Option<Sender<()>>>,
}

impl GitService {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let workspace_root = workspace_root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        let workspace_root = strip_unc_prefix(workspace_root);
        let git_dir = paths::git_dir(&workspace_root);
        Self {
            workspace_root,
            git_dir,
            pending: Mutex::new(Vec::new()),
            settings: Mutex::new(GitSettings::default()),
            author: Mutex::new((DEFAULT_AUTHOR_NAME.into(), DEFAULT_AUTHOR_EMAIL.into())),
            auto_commit: Mutex::new(None),
        }
    }

    pub fn git_dir_path(&self) -> &Path {
        &self.git_dir
    }

    pub fn set_settings(&self, s: GitSettings) {
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }
    pub fn set_author(&self, name: &str, email: &str) {
        let name = if name.trim().is_empty() {
            DEFAULT_AUTHOR_NAME
        } else {
            name.trim()
        };
        let email = if email.trim().is_empty() {
            DEFAULT_AUTHOR_EMAIL
        } else {
            email.trim()
        };
        *self.author.lock().unwrap_or_else(|e| e.into_inner()) = (name.into(), email.into());
    }

    pub fn settings(&self) -> GitSettings {
        self.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    // ---------- 仓库初始化 ----------

    /// 是否存在可用的 gitdir（`.mochi/git` 或传统 `.git`）。
    pub fn is_valid(&self) -> bool {
        Repository::open(&self.git_dir).is_ok() || self.legacy_repo().is_some()
    }

    /// 工作区根存在传统 `.git` 仓库则返回之（旧版回退规则）。
    fn legacy_repo(&self) -> Option<Repository> {
        if !self.workspace_root.join(".git").is_dir() {
            return None;
        }
        Repository::open(&self.workspace_root).ok()
    }

    /// 确保仓库存在。旧版回退规则：工作区根存在传统 `.git` 仓库则沿用之（不新建 `.mochi/git`）。
    pub fn ensure_repository(&self) -> Result<()> {
        let use_legacy = self.legacy_repo().is_some();

        if !use_legacy && Repository::open(&self.git_dir).is_err() {
            // **必须用分离式 init**：`.mochi/git` 本身就是 gitdir，里面直接是
            // `HEAD config objects refs …`，而**不是**再套一层 `.git/`。
            // 真实工作区（isomorphic-git 建的）就是这个形状；套一层的话
            // Electron 版去找 `.mochi/git/HEAD` 会扑空，等于两版读不了同一个仓库。
            //
            // 注意 `Repository::init(path)` 会建成 `path/.git/`——那是错的。
            // C# 版 `GitService.cs:75` 用的正是 `Repository.Init(_gitDir)`，同样踩了这个坑。
            if let Some(parent) = self.git_dir.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut opts = RepositoryInitOptions::new();
            // `workdir_path` 是必需的：libgit2 对「非裸 + gitdir 不叫 .git」的组合
            // 会直接拒绝（cannot pick working directory…）。
            opts.bare(false)
                .no_dotgit_dir(true)
                .mkpath(true)
                .workdir_path(&self.workspace_root);
            Repository::init_opts(&self.git_dir, &opts)
                .with_context(|| format!("git init 失败: {}", self.git_dir.display()))?;

            // libgit2 会在工作区根留一个 `.git` gitlink 文件指回 gitdir，
            // 但真实工作区（isomorphic-git 建的）**没有**这个文件。删掉以保持两版形态一致——
            // 仓库通过 gitdir 路径打开，工作树归属由 `core.worktree` 表达，无需 gitlink。
            let gitlink = self.workspace_root.join(".git");
            if gitlink.is_file() {
                std::fs::remove_file(&gitlink)
                    .with_context(|| format!("清理 gitlink 失败: {}", gitlink.display()))?;
            }
        }

        let repo = self.open_repository()?;
        let mut config = repo.config()?;

        if !use_legacy {
            let want = self.workspace_root.to_string_lossy().into_owned();
            let current = config.get_string("core.worktree").ok();
            let matches = current.as_deref().is_some_and(|c| {
                c.trim_end_matches(['/', '\\'])
                    .eq_ignore_ascii_case(want.trim_end_matches(std::path::MAIN_SEPARATOR))
            });
            if !matches {
                config.set_str("core.worktree", &want)?;
            }
        }

        // isomorphic-git init 用 defaultBranch:'main'；对尚未有提交的 HEAD 直接改指向
        // （首次提交即落在 main 上）
        if repo.head().is_err() {
            let target = format!("refs/heads/{DEFAULT_BRANCH}");
            let current = repo
                .find_reference("HEAD")
                .ok()
                .and_then(|r| r.symbolic_target().ok().flatten().map(str::to_owned));
            if current.as_deref() != Some(target.as_str()) {
                repo.reference_symbolic("HEAD", &target, true, "set default branch")?;
            }
        }

        // 本地作者配置（只查 Local 级；全局 user.name 存在时仍要写入）。
        // 先算完再写，避免闭包对 config 的不可变借用与 set_str 的可变借用打架。
        let name_missing = config
            .open_level(ConfigLevel::Local)
            .and_then(|c| c.get_string("user.name"))
            .is_err();
        let email_missing = config
            .open_level(ConfigLevel::Local)
            .and_then(|c| c.get_string("user.email"))
            .is_err();
        if name_missing {
            config.set_str("user.name", DEFAULT_AUTHOR_NAME)?;
        }
        if email_missing {
            config.set_str("user.email", DEFAULT_AUTHOR_EMAIL)?;
        }
        Ok(())
    }

    // ---------- 操作入队 ----------

    /// 记录一次文件变更（保存/改名/删除后调用）。
    pub fn mark_change(&self, op: PendingVersionOperation) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.push(op);
        if pending.len() > MAX_PENDING_OPERATIONS {
            let excess = pending.len() - MAX_PENDING_OPERATIONS;
            pending.drain(0..excess);
        }
    }

    pub fn mark_write(&self, path: impl Into<PathBuf>) {
        self.mark_change(PendingVersionOperation::write(path));
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    // ---------- 提交 ----------

    /// 手动提交。返回 oid；无可提交内容时返回 `None`。
    pub fn commit(&self, message: &str, kind: &str) -> Result<Option<String>> {
        // 与保留的 Electron 服务行为一致：关闭历史的同时也拦下
        // 手动/AI/退出提交；但用户明确要求的还原仍会记录。
        if !self.settings().version_history_enabled && kind != "restore" {
            return Ok(None);
        }
        let repo = self.open_repository()?;
        self.stage_pending_or_all(&repo)?;

        let mut index = repo.index()?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        // 空提交守卫：索引相对 HEAD 无差异则不提交
        let head_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let head_tree = head_commit.as_ref().and_then(|c| c.tree().ok());
        let diff = repo.diff_tree_to_index(head_tree.as_ref(), Some(&index), None)?;
        if diff.deltas().len() == 0 {
            return Ok(None);
        }

        let auto_message;
        let final_message = format_commit_message(
            if message.trim().is_empty() {
                auto_message = self.create_auto_commit_message();
                &auto_message
            } else {
                message
            },
            kind,
        );
        let author = self
            .author
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let sig = Signature::now(&author.0, &author.1)?;
        let parents: Vec<&git2::Commit> = head_commit.iter().collect();
        let oid = repo.commit(Some("HEAD"), &sig, &sig, &final_message, &tree, &parents)?;

        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(Some(oid.to_string()))
    }

    /// `定时记录 MM/DD HH:mm · N 个变更`
    pub fn create_auto_commit_message(&self) -> String {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let timestamp = Local::now().format("%m/%d %H:%M");
        let distinct: HashSet<String> = pending
            .iter()
            .map(|o| o.path.to_string_lossy().to_lowercase())
            .collect();
        let count = if distinct.is_empty() {
            pending.len().max(1)
        } else {
            distinct.len()
        };
        format!("定时记录 {timestamp} · {count} 个变更")
    }

    /// 有变更才提交，吞掉所有错误（后台任务不能崩）。
    fn try_auto_commit(&self) {
        let s = self.settings();
        if !s.version_history_enabled || !s.auto_commit_enabled {
            return;
        }
        if !self.is_valid() && self.ensure_repository().is_err() {
            return;
        }
        if !self.has_anything_to_commit() {
            return;
        }
        let _ = self.commit(&self.create_auto_commit_message(), "auto");
    }

    /// 应用退出前的兜底提交（`auto: 退出前自动记录`）。
    pub fn flush_on_exit(&self) {
        if !self.has_anything_to_commit() {
            return;
        }
        let _ = self.commit("退出前自动记录", "auto");
    }

    fn has_anything_to_commit(&self) -> bool {
        if self.pending_count() > 0 {
            return true;
        }
        match self.open_repository() {
            Ok(repo) => has_worktree_changes(&repo).unwrap_or(false),
            Err(_) => false,
        }
    }

    /// 启动定时自动提交循环（间隔分钟数，最小 1）。
    pub fn start_auto_commit(self: &Arc<Self>) {
        self.stop_auto_commit();
        let s = self.settings();
        if !s.version_history_enabled || !s.auto_commit_enabled {
            return;
        }
        let interval = Duration::from_secs(60 * s.auto_commit_interval_minutes.max(1));

        let (tx, rx) = mpsc::channel::<()>();
        *self.auto_commit.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);

        // 线程不能强持有服务，否则服务的 Drop 永远不会运行，旧工作区持续自动提交。
        let this = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("mochi-git-autocommit".into())
            .spawn(move || {
                // 收到停止信号或通道关闭即退出；超时即到点，执行一次。
                while let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(interval) {
                    let Some(service) = this.upgrade() else { break };
                    service.try_auto_commit();
                }
            })
            .ok();
    }

    pub fn stop_auto_commit(&self) {
        if let Some(tx) = self
            .auto_commit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = tx.send(());
        }
    }

    // ---------- 历史 ----------

    /// 最近提交（新→旧）。
    pub fn get_history(&self, limit: usize) -> Result<Vec<GitCommitInfo>> {
        self.get_history_filtered(limit, None, None)
    }

    /// 带过滤的历史：只看动过某个文件的提交、只看某个作者。
    ///
    /// `filepath` 传绝对路径或工作区相对路径都行。按文件过滤时逐个提交与其父提交做
    /// 比较整棵树的差异——仓库（笔记工作区）规模不大，这比维护额外索引更简单可靠。
    pub fn get_history_filtered(
        &self,
        limit: usize,
        filepath: Option<&str>,
        author: Option<&str>,
    ) -> Result<Vec<GitCommitInfo>> {
        let repo = self.open_repository()?;
        if repo.head().is_err() {
            return Ok(Vec::new());
        }
        let rel = filepath.and_then(|p| self.to_rel_path(Path::new(p)));
        let author_needle = author
            .map(|a| a.trim().to_lowercase())
            .filter(|a| !a.is_empty());

        let mut walk = repo.revwalk()?;
        walk.push_head()?;

        let mut out = Vec::new();
        for oid in walk {
            if out.len() >= limit {
                break;
            }
            let commit = repo.find_commit(oid?)?;

            if let Some(needle) = &author_needle {
                let name = commit.author().name().unwrap_or_default().to_lowercase();
                if !name.contains(needle) {
                    continue;
                }
            }
            if let Some(rel) = &rel {
                if !commit_touches(&repo, &commit, rel)? {
                    continue;
                }
            }

            let message = commit.message().unwrap_or_default().trim_end_matches('\n');
            let kind = get_commit_type(message);
            let sha = commit.id().to_string();
            out.push(GitCommitInfo {
                short_oid: sha[..8.min(sha.len())].to_owned(),
                oid: sha,
                message: message.to_owned(),
                is_key_node: is_key_node(&kind),
                kind,
                author_name: commit.author().name().unwrap_or_default().to_owned(),
                timestamp_ms: commit.author().when().seconds() * 1000,
            });
        }
        Ok(out)
    }

    /// 取某次提交里某个文件的内容。文件在该提交中不存在时返回空串——
    /// 差异的一边本来就可能是“文件尚不存在”。
    pub fn file_at_commit_or_empty(&self, oid: &str, filepath: &str) -> Result<String> {
        Ok(self.file_at_commit(oid, filepath).unwrap_or_default())
    }

    pub fn file_at_commit(&self, oid: &str, filepath: &str) -> Result<String> {
        let repo = self.open_repository()?;
        let rel = self
            .to_rel_path(Path::new(filepath))
            .ok_or_else(|| anyhow::anyhow!("路径不在工作区内: {filepath}"))?;
        let commit = repo.find_commit(git2::Oid::from_str(oid)?)?;
        let entry = commit.tree()?.get_path(Path::new(&rel))?;
        let blob = repo.find_blob(entry.id())?;
        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    /// 两次提交之间某文件的新旧内容。差异渲染交给上层。
    pub fn file_diff(
        &self,
        filepath: &str,
        old_oid: &str,
        new_oid: &str,
    ) -> Result<(String, String)> {
        Ok((
            self.file_at_commit_or_empty(old_oid, filepath)?,
            self.file_at_commit_or_empty(new_oid, filepath)?,
        ))
    }

    /// 把某次提交里的文件恢复出来。返回落盘路径。
    ///
    /// - `copy`（默认）：写到 `<名>-restored-<时间戳><扩展名>`，**不动原文件**
    /// - `overwrite`：覆盖原文件。覆盖前先自动提交一次快照，
    ///   这样用户后悔了还能回来——恢复本身是破坏性操作。
    pub fn restore_file(&self, filepath: &str, oid: &str, mode: &str) -> Result<PathBuf> {
        let rel = self
            .to_rel_path(Path::new(filepath))
            .ok_or_else(|| anyhow::anyhow!("路径不在工作区内: {filepath}"))?;
        let content = self.file_at_commit(oid, filepath)?;

        if mode == "overwrite" {
            self.commit(&format!("恢复前快照 {rel}"), "manual")?;
            let absolute = self.workspace_root.join(&rel);
            std::fs::write(&absolute, content.as_bytes())?;
            self.mark_change(PendingVersionOperation {
                kind: VersionOpType::Restore,
                path: absolute.clone(),
                old_path: None,
            });
            let short = &oid[..8.min(oid.len())];
            self.commit(&format!("{rel} from {short}"), "restore")?;
            return Ok(absolute);
        }

        // 与 TS 一致：ISO 时间戳里的 : 和 . 换成 -，避免出现非法文件名
        let stamp = jstime_stamp();
        let path = Path::new(&rel);
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let base = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let restored_rel = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(dir) => dir.join(format!("{base}-restored-{stamp}{ext}")),
            None => PathBuf::from(format!("{base}-restored-{stamp}{ext}")),
        };
        let absolute = self.workspace_root.join(restored_rel);
        if let Some(parent) = absolute.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&absolute, content.as_bytes())?;
        Ok(absolute)
    }

    // ---------- 内部 ----------

    fn open_repository(&self) -> Result<Repository> {
        if let Some(repo) = self.legacy_repo() {
            return Ok(repo);
        }
        Repository::open(&self.git_dir)
            .with_context(|| format!("打开仓库失败: {}", self.git_dir.display()))
    }

    /// 暂存策略（与旧版一致）：有待办操作 → 只暂存这些路径（rename/delete 的旧路径做移除）；
    /// 无待办 → 整库 status 扫描。
    fn stage_pending_or_all(&self, repo: &Repository) -> Result<()> {
        let snapshot: Vec<PendingVersionOperation> = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        if snapshot.is_empty() {
            return self.stage_whole_worktree(repo);
        }

        let mut index = repo.index()?;

        // 先处理移除（delete / rename 旧路径），再做新增/更新
        for op in &snapshot {
            let removal = match op.kind {
                VersionOpType::Delete => Some(&op.path),
                VersionOpType::Rename => op.old_path.as_ref(),
                _ => None,
            };
            if let Some(target) = removal {
                if let Some(rel) = self.to_rel_path(target) {
                    let _ = index.remove_path(Path::new(&rel));
                }
            }
        }

        let mut touched: HashSet<String> = HashSet::new();
        for op in &snapshot {
            for path in [op.old_path.as_ref(), Some(&op.path)].into_iter().flatten() {
                let key = path.to_string_lossy().to_lowercase();
                if !touched.insert(key) {
                    continue;
                }
                if op.kind == VersionOpType::Delete {
                    continue;
                }
                if op.kind == VersionOpType::Rename && Some(path) == op.old_path.as_ref() {
                    continue;
                }
                self.stage_add_or_update(&mut index, path);
            }
        }
        index.write()?;
        Ok(())
    }

    fn stage_add_or_update(&self, index: &mut git2::Index, absolute: &Path) {
        if !self.should_track_path(absolute) {
            return;
        }
        let Some(rel) = self.to_rel_path(absolute) else {
            return;
        };
        let rel = Path::new(&rel);
        if !absolute.is_file() {
            let _ = index.remove_path(rel);
            return;
        }
        let _ = index.add_path(rel);
    }

    fn stage_whole_worktree(&self, repo: &Repository) -> Result<()> {
        let mut index = repo.index()?;
        let mut opts = status_options();
        for entry in repo.statuses(Some(&mut opts))?.iter() {
            let Ok(rel) = entry.path() else { continue };
            let full = self.workspace_root.join(rel);
            let status = entry.status();

            if status.intersects(Status::WT_NEW | Status::WT_MODIFIED) {
                if self.should_track_path(&full) {
                    let _ = index.add_path(Path::new(rel));
                }
            } else if status.contains(Status::WT_DELETED) {
                // 删除的路径若曾被跟踪则移除（不受当前过滤影响——历史里可能已有）
                let _ = index.remove_path(Path::new(rel));
            }
        }
        index.write()?;
        Ok(())
    }

    /// 跟踪范围：排除 `.git/`、`.mochi/`、`*.backup`；其余交给 `.gitignore` 规则。
    fn should_track_path(&self, absolute: &Path) -> bool {
        match self.to_rel_path(absolute) {
            None => false,
            Some(rel) => {
                !rel.starts_with(".git/")
                    && !rel.starts_with(".mochi/")
                    && !rel.ends_with(".backup")
            }
        }
    }

    /// 绝对路径 → 相对工作区的正斜杠路径；越界或等于根返回 `None`。
    fn to_rel_path(&self, absolute: &Path) -> Option<String> {
        let full = absolute
            .canonicalize()
            .map(strip_unc_prefix)
            .unwrap_or_else(|_| absolute.to_path_buf());
        let rel = full.strip_prefix(&self.workspace_root).ok()?;
        if rel.as_os_str().is_empty() {
            return None;
        }
        Some(paths::to_forward_slashes(&rel.to_string_lossy()))
    }
}

impl Drop for GitService {
    fn drop(&mut self) {
        self.stop_auto_commit();
    }
}

/// 检查此提交是否改动了该文件：与首个父提交比较整棵树；根提交则检查树中是否包含该路径。
fn commit_touches(repo: &Repository, commit: &git2::Commit, rel: &str) -> Result<bool> {
    let tree = commit.tree()?;
    let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
    let target = Path::new(rel);
    Ok(diff
        .deltas()
        .any(|d| d.new_file().path() == Some(target) || d.old_file().path() == Some(target)))
}

/// 文件名安全的时间戳：ISO 串里的 : 和 . 换成 -（Windows 不允许冒号）。
fn jstime_stamp() -> String {
    crate::jstime::now().replace([':', '.'], "-")
}

fn status_options() -> StatusOptions {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    opts
}

pub fn has_worktree_changes(repo: &Repository) -> Result<bool> {
    let mut opts = status_options();
    Ok(repo
        .statuses(Some(&mut opts))?
        .iter()
        .any(|e| !e.status().contains(Status::IGNORED)))
}

fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p =
                std::env::temp_dir().join(format!("mochi-git-{}-{tag}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn svc(&self) -> GitService {
            GitService::new(&self.0)
        }
        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let p = self.0.join(rel);
            if let Some(d) = p.parent() {
                fs::create_dir_all(d).unwrap();
            }
            fs::write(&p, content).unwrap();
            p
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn message_prefix_round_trip() {
        assert_eq!(format_commit_message("  修改  ", "manual"), "manual: 修改");
        assert_eq!(format_commit_message("裸消息", "other"), "裸消息");
        assert_eq!(get_commit_type("auto: 定时记录"), "auto");
        assert_eq!(get_commit_type("ai-before: x"), "ai-before");
        assert_eq!(get_commit_type("未知前缀: x"), "other");
        assert_eq!(get_commit_type("没有冒号"), "other");
        assert_eq!(get_commit_type(":开头就是冒号"), "other");
    }

    #[test]
    fn disabled_history_does_not_stage_or_commit_but_explicit_restore_can() {
        let ws = TempWs::new("disabled-history");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        let path = ws.write("note.md", "draft");
        svc.mark_write(&path);
        svc.set_settings(GitSettings {
            version_history_enabled: false,
            ..Default::default()
        });
        for kind in ["manual", "auto", "ai-before", "ai-after"] {
            assert_eq!(svc.commit("disabled", kind).unwrap(), None);
        }
        assert_eq!(svc.pending_count(), 1);
        assert!(svc.open_repository().unwrap().index().unwrap().is_empty());
        assert!(svc
            .commit("requested restore", "restore")
            .unwrap()
            .is_some());
        assert_eq!(svc.pending_count(), 0);
    }

    #[test]
    fn key_nodes() {
        for k in ["manual", "ai-before", "ai-after", "restore", "rename"] {
            assert!(is_key_node(k), "{k} 应是关键节点");
        }
        for k in ["auto", "delete", "import", "other"] {
            assert!(!is_key_node(k), "{k} 不应是关键节点");
        }
    }

    /// `.mochi/git` **本身**就是 gitdir，里面直接是 HEAD/config/objects/refs，
    /// 不能再套一层 `.git/`。真实工作区（isomorphic-git 建的）就是这个形状——
    /// 套一层的话 Electron 版找不到 `.mochi/git/HEAD`，两版读不了同一个仓库。
    ///
    /// 这条断言的是**磁盘布局**而不是 `Repository::open` 能否打开：后者会自动
    /// 往里找 `.git/`，套了一层也照样成功，掩盖问题。
    #[test]
    fn mochi_git_is_the_gitdir_itself_not_a_parent_of_dot_git() {
        let ws = TempWs::new("gitdir-layout");
        ws.svc().ensure_repository().unwrap();
        let gitdir = paths::git_dir(&ws.0);

        assert!(
            !gitdir.join(".git").exists(),
            "多套了一层 .git/，Electron 版会读不了"
        );
        for entry in ["HEAD", "config", "objects", "refs"] {
            assert!(gitdir.join(entry).exists(), "gitdir 里缺 {entry}");
        }
    }

    #[test]
    fn ensure_repository_creates_mochi_git_with_worktree_and_main() {
        let ws = TempWs::new("init");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();

        assert!(svc.is_valid());
        assert!(paths::git_dir(&ws.0).is_dir(), "仓库不在 .mochi/git");
        assert!(!ws.0.join(".git").exists(), "不该在根目录建 .git");

        let repo = Repository::open(paths::git_dir(&ws.0)).unwrap();
        let cfg = repo.config().unwrap();
        // core.worktree 是打开自愈的关键，必须是绝对路径
        let wt = cfg.get_string("core.worktree").unwrap();
        assert!(
            Path::new(&wt).is_absolute(),
            "core.worktree 不是绝对路径: {wt}"
        );
        assert_eq!(cfg.get_string("user.name").unwrap(), DEFAULT_AUTHOR_NAME);

        let head = repo.find_reference("HEAD").unwrap();
        assert_eq!(head.symbolic_target().unwrap(), Some("refs/heads/main"));
    }

    #[test]
    fn ensure_repository_is_idempotent() {
        let ws = TempWs::new("init-idem");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        svc.ensure_repository().unwrap();
        assert!(svc.is_valid());
    }

    #[test]
    fn automatic_commit_worker_does_not_keep_closed_workspace_alive() {
        let ws = TempWs::new("auto-lifetime");
        let svc = Arc::new(ws.svc());
        let weak = Arc::downgrade(&svc);
        svc.start_auto_commit();
        drop(svc);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn configured_author_changes_commit_not_repository_config() {
        let ws = TempWs::new("commit-author");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        svc.set_author("测试作者", "writer@example.test");
        ws.write("note.md", "测试内容");
        let oid = svc.commit("保存", "manual").unwrap().unwrap();
        let repo = Repository::open(paths::git_dir(&ws.0)).unwrap();
        let commit = repo
            .find_commit(git2::Oid::from_str(&oid).unwrap())
            .unwrap();
        assert_eq!(commit.author().name().unwrap(), "测试作者");
        assert_eq!(commit.author().email().unwrap(), "writer@example.test");
        assert_eq!(
            repo.config().unwrap().get_string("user.name").unwrap(),
            DEFAULT_AUTHOR_NAME
        );
    }

    #[test]
    fn first_commit_lands_on_main() {
        let ws = TempWs::new("commit");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        ws.write("note.md", "hello");

        let oid = svc.commit("初次提交", "manual").unwrap();
        assert!(oid.is_some(), "应产生提交");

        let repo = Repository::open(paths::git_dir(&ws.0)).unwrap();
        assert_eq!(repo.head().unwrap().shorthand().unwrap(), "main");

        let history = svc.get_history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].message, "manual: 初次提交");
        assert_eq!(history[0].kind, "manual");
        assert!(history[0].is_key_node);
        assert_eq!(history[0].short_oid.len(), 8);
    }

    #[test]
    fn empty_commit_is_refused() {
        let ws = TempWs::new("empty");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        ws.write("a.md", "x");
        svc.commit("first", "manual").unwrap();

        // 无任何改动，再提交应返回 None
        assert_eq!(svc.commit("再来一次", "manual").unwrap(), None);
    }

    #[test]
    fn mochi_and_backup_paths_are_not_tracked() {
        let ws = TempWs::new("filter");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        ws.write(".mochi/config.json", "{}");
        ws.write("note.md.backup", "old");
        ws.write("note.md", "new");

        svc.commit("过滤检查", "manual").unwrap();

        let repo = Repository::open(paths::git_dir(&ws.0)).unwrap();
        let tree = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        assert!(tree.get_path(Path::new("note.md")).is_ok());
        assert!(
            tree.get_path(Path::new("note.md.backup")).is_err(),
            ".backup 被提交了"
        );
        assert!(
            tree.get_path(Path::new(".mochi/config.json")).is_err(),
            ".mochi 被提交了"
        );
    }

    #[test]
    fn pending_queue_is_capped_at_50() {
        let ws = TempWs::new("cap");
        let svc = ws.svc();
        for i in 0..80 {
            svc.mark_write(ws.0.join(format!("f{i}.md")));
        }
        assert_eq!(svc.pending_count(), MAX_PENDING_OPERATIONS);
    }

    #[test]
    fn auto_commit_message_counts_distinct_paths() {
        let ws = TempWs::new("msg");
        let svc = ws.svc();
        svc.mark_write(ws.0.join("a.md"));
        svc.mark_write(ws.0.join("a.md")); // 重复路径只算一个
        svc.mark_write(ws.0.join("b.md"));

        let msg = svc.create_auto_commit_message();
        assert!(msg.starts_with("定时记录 "), "{msg}");
        assert!(msg.ends_with(" · 2 个变更"), "{msg}");
    }

    #[test]
    fn auto_commit_message_shape_when_empty() {
        let ws = TempWs::new("msg-empty");
        let msg = ws.svc().create_auto_commit_message();
        // 形如 `定时记录 08/28 14:51 · 1 个变更`
        assert!(msg.ends_with(" · 1 个变更"), "{msg}");
        let stamp = &msg["定时记录 ".len()..msg.len() - " · 1 个变更".len()];
        assert_eq!(
            stamp.len(),
            11,
            "时间戳格式应为 MM/DD HH:mm，实得 {stamp:?}"
        );
    }

    #[test]
    fn pending_ops_stage_only_their_paths() {
        let ws = TempWs::new("pending-stage");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        let tracked = ws.write("tracked.md", "yes");
        ws.write("untouched.md", "not in queue");

        svc.mark_write(&tracked);
        svc.commit("只提交队列里的", "manual").unwrap();

        let repo = Repository::open(paths::git_dir(&ws.0)).unwrap();
        let tree = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        assert!(tree.get_path(Path::new("tracked.md")).is_ok());
        assert!(
            tree.get_path(Path::new("untouched.md")).is_err(),
            "队列外的文件不该被暂存"
        );
    }

    #[test]
    fn commit_clears_pending_queue() {
        let ws = TempWs::new("clear");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        svc.mark_write(ws.write("a.md", "x"));
        svc.commit("x", "manual").unwrap();
        assert_eq!(svc.pending_count(), 0);
    }

    #[test]
    fn delete_op_removes_from_index() {
        let ws = TempWs::new("delete");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        let f = ws.write("gone.md", "bye");
        svc.mark_write(&f);
        svc.commit("加进去", "manual").unwrap();

        fs::remove_file(&f).unwrap();
        svc.mark_change(PendingVersionOperation::delete(&f));
        svc.commit("删掉", "delete").unwrap();

        let repo = Repository::open(paths::git_dir(&ws.0)).unwrap();
        let tree = repo
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .tree()
            .unwrap();
        assert!(
            tree.get_path(Path::new("gone.md")).is_err(),
            "删除没落到提交里"
        );
    }

    #[test]
    fn history_is_newest_first_and_limited() {
        let ws = TempWs::new("history");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        for i in 0..3 {
            ws.write("n.md", &format!("v{i}"));
            svc.commit(&format!("第{i}次"), "manual").unwrap();
        }

        let all = svc.get_history(100).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].message, "manual: 第2次", "应新→旧");
        assert_eq!(svc.get_history(2).unwrap().len(), 2);
    }

    #[test]
    fn history_on_empty_repo_is_empty_not_error() {
        let ws = TempWs::new("history-empty");
        let svc = ws.svc();
        svc.ensure_repository().unwrap();
        assert!(svc.get_history(10).unwrap().is_empty());
    }

    #[test]
    fn legacy_dot_git_is_reused_instead_of_creating_mochi_git() {
        let ws = TempWs::new("legacy");
        Repository::init(&ws.0).unwrap();
        let svc = ws.svc();

        svc.ensure_repository().unwrap();

        assert!(svc.is_valid());
        assert!(
            !paths::git_dir(&ws.0).exists(),
            "已有 .git 时不该新建 .mochi/git"
        );
    }
}
