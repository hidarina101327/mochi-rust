//! Validated, reviewable folder operations for the desktop folder card.
//!
//! Copy and move execution is delegated to the Windows shell host. This module plans
//! bounded transfers and verifies staged output before it can be published.

use super::folder::{self, FolderConfig, MAX_FOLDER_ENTRIES};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ffi::CString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAX_TRANSFER_ENTRIES: usize = 8192;
pub const MAX_TRANSFER_DEPTH: usize = 32;
pub const MAX_TRANSFER_BYTES: u64 = 512 * 1024 * 1024;

static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferKind {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferLimits {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_bytes: u64,
}

impl Default for TransferLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_TRANSFER_ENTRIES,
            max_depth: MAX_TRANSFER_DEPTH,
            max_bytes: MAX_TRANSFER_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManifestKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestEntry {
    /// Empty for the transfer root; otherwise relative to the root.
    pub relative_path: PathBuf,
    pub kind: ManifestKind,
    pub bytes: u64,
    /// Lowercase SHA-256 for files; empty for directories.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlannedTransfer {
    pub source: PathBuf,
    pub target: PathBuf,
    pub staged_target: PathBuf,
    pub manifest: Vec<ManifestEntry>,
    pub entries: usize,
    pub bytes: u64,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationResult {
    pub source: PathBuf,
    pub target: Option<PathBuf>,
    pub succeeded: bool,
    pub error: Option<String>,
}

impl OperationResult {
    fn success(source: PathBuf, target: PathBuf) -> Self {
        Self {
            source,
            target: Some(target),
            succeeded: true,
            error: None,
        }
    }

    fn failure(source: PathBuf, target: Option<PathBuf>, error: impl ToString) -> Self {
        Self {
            source,
            target,
            succeeded: false,
            error: Some(error.to_string()),
        }
    }
}

impl Default for OperationResult {
    fn default() -> Self {
        Self {
            source: PathBuf::new(),
            target: None,
            succeeded: false,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferPlan {
    pub kind: TransferKind,
    /// Canonical mapped directory at planning time.
    pub root: PathBuf,
    /// A unique, not-yet-created child of root. The host creates it after approval.
    pub staging_root: PathBuf,
    pub limits: TransferLimits,
    pub items: Vec<PlannedTransfer>,
    /// Sources that could not be safely planned. Valid items remain usable.
    pub failures: Vec<OperationResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrganizeAction {
    pub source: PathBuf,
    pub category: String,
    pub target: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrganizePlan {
    pub root: PathBuf,
    pub actions: Vec<OrganizeAction>,
    pub failures: Vec<OperationResult>,
    pub truncated: bool,
}

/// Create a direct child folder using an exclusive filesystem operation.
pub fn create_subdirectory(config: &FolderConfig, name: &str) -> Result<PathBuf> {
    let root = current_root(config)?;
    validate_entry_name(name)?;
    let target = root.join(name);
    fs::create_dir(&target)
        .with_context(|| format!("无法创建文件夹（名称已存在或无写入权限）: {name}"))?;
    ensure_created_child(&root, &target)?;
    Ok(target)
}

/// Rename one direct child. The mapping root itself cannot be renamed here.
pub fn rename_entry(config: &FolderConfig, target: &str, new_name: &str) -> Result<PathBuf> {
    let current = folder::current_config(config)?;
    let root = current_root(&current)?;
    validate_entry_name(new_name)?;
    let source = folder::resolve_entry(&current, target)?;
    if crate::paths::paths_equal(&source, &root) {
        bail!("不能在映射文件夹内部重命名映射根目录");
    }
    let destination = root.join(new_name);
    if crate::paths::paths_equal(&source, &destination) {
        return Ok(source);
    }
    ensure_absent(&destination)?;
    ensure_no_reparse_ancestors(&source)?;
    no_replace_rename(&source, &destination)
        .with_context(|| format!("重命名失败，目标可能已存在: {new_name}"))?;
    Ok(destination)
}

/// Build a bounded, per-source transfer plan. Invalid sources are returned in
/// failures; other sources continue to be planned.
pub fn plan_transfer(
    config: &FolderConfig,
    sources: &[PathBuf],
    kind: TransferKind,
    requested_limits: TransferLimits,
) -> Result<TransferPlan> {
    let current = folder::current_config(config)?;
    let root = current_root(&current)?;
    let limits = effective_limits(requested_limits)?;
    let staging_root = unique_staging_root(&root)?;
    let mut plan = TransferPlan {
        kind,
        root: root.clone(),
        staging_root,
        limits,
        items: Vec::new(),
        failures: Vec::new(),
    };
    let mut budget = ManifestBudget {
        entries_left: limits.max_entries,
        bytes_left: limits.max_bytes,
    };
    let mut targets = Vec::<PathBuf>::new();

    for source in sources {
        match plan_one_transfer(
            &root,
            &plan.staging_root,
            source,
            limits,
            &mut budget,
            &targets,
        ) {
            Ok(item) => {
                targets.push(item.target.clone());
                plan.items.push(item);
            }
            Err(error) => plan
                .failures
                .push(OperationResult::failure(source.clone(), None, error)),
        }
    }
    Ok(plan)
}

/// Validate plan bounds and path layout before creating the shared staging directory.
/// Source contents and destination collisions are checked per item by
/// `validate_transfer_item` so one stale source does not cancel the rest of a batch.
pub fn validate_transfer_plan(config: &FolderConfig, plan: &TransferPlan) -> Result<()> {
    anyhow::ensure!(
        effective_limits(plan.limits)? == plan.limits,
        "传输计划预算超出允许范围"
    );
    anyhow::ensure!(
        plan.items.len() <= plan.limits.max_entries,
        "传输计划条目数超过预算"
    );
    let root = validate_plan_root(config, plan)?;
    validate_staging_root_path(&root, &plan.staging_root)?;
    ensure_absent(&plan.staging_root).context("传输暂存目录已被占用，请重新生成计划")?;
    let mut targets: Vec<&Path> = Vec::with_capacity(plan.items.len());
    for (index, item) in plan.items.iter().enumerate() {
        validate_transfer_paths(&root, &plan.staging_root, index, item)?;
        if targets
            .iter()
            .any(|target| crate::paths::paths_equal(target, &item.target))
        {
            bail!("传输计划包含重复的目标路径");
        }
        targets.push(item.target.as_path());
    }
    Ok(())
}

/// Create the reviewed plan's staging directory exclusively after user approval.
pub fn prepare_transfer_staging(config: &FolderConfig, plan: &TransferPlan) -> Result<PathBuf> {
    validate_transfer_plan(config, plan)?;
    fs::create_dir(&plan.staging_root)
        .with_context(|| format!("无法安全创建传输暂存目录: {}", plan.staging_root.display()))?;
    ensure_created_child(&plan.root, &plan.staging_root)?;
    Ok(plan.staging_root.clone())
}

/// Revalidate one source, target, and empty staging slot immediately before copy.
pub fn validate_transfer_item(
    config: &FolderConfig,
    plan: &TransferPlan,
    index: usize,
) -> Result<()> {
    let root = validate_plan_root(config, plan)?;
    validate_staging_root_path(&root, &plan.staging_root)?;
    let stage_metadata =
        fs::symlink_metadata(&plan.staging_root).context("传输暂存目录不存在，请先准备暂存目录")?;
    if is_reparse_point(&stage_metadata) || !stage_metadata.is_dir() {
        bail!("传输暂存路径不是安全文件夹");
    }
    let item = plan
        .items
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("传输计划条目编号无效"))?;
    validate_transfer_paths(&root, &plan.staging_root, index, item)?;
    verify_source_manifest(item, plan.limits)?;
    ensure_absent(&item.target).context("目标已存在，拒绝覆盖")?;
    ensure_absent(&item.staged_target).context("暂存目标已存在，拒绝覆盖")?;
    Ok(())
}

/// Verify a staged copy against the reviewed manifest, then atomically publish it
/// without replacing an existing destination.
pub fn publish_staged_item(
    config: &FolderConfig,
    plan: &TransferPlan,
    index: usize,
) -> Result<PathBuf> {
    let root = validate_plan_root(config, plan)?;
    let item = plan
        .items
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("传输计划条目编号无效"))?;
    validate_transfer_paths(&root, &plan.staging_root, index, item)?;
    verify_source_manifest(item, plan.limits)?;
    verify_manifest_matches(
        &item.staged_target,
        &item.manifest,
        item.entries,
        item.bytes,
        plan.limits,
    )
    .context("暂存副本未通过完整性校验")?;
    ensure_absent(&item.target).context("目标已存在，拒绝覆盖")?;
    ensure_no_reparse_ancestors(&item.staged_target)?;
    no_replace_rename(&item.staged_target, &item.target)
        .with_context(|| format!("无法发布暂存项目: {}", item.target.display()))?;
    Ok(item.target.clone())
}

/// Revalidate a source immediately before the host removes it for a move operation.
pub fn validate_source_manifest(
    config: &FolderConfig,
    plan: &TransferPlan,
    index: usize,
) -> Result<()> {
    let root = validate_plan_root(config, plan)?;
    let item = plan
        .items
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("传输计划条目编号无效"))?;
    validate_transfer_paths(&root, &plan.staging_root, index, item)?;
    verify_source_manifest(item, plan.limits)?;
    verify_manifest_matches(
        &item.target,
        &item.manifest,
        item.entries,
        item.bytes,
        plan.limits,
    )
    .context("已发布目标在移除来源前未通过完整性校验")?;
    Ok(())
}

/// Remove the staging directory only when it is empty. Failed staged items are left
/// intact for recovery and are never recursively deleted by this helper.
pub fn cleanup_transfer_staging(config: &FolderConfig, plan: &TransferPlan) -> Result<()> {
    let root = validate_plan_root(config, plan)?;
    validate_staging_root_path(&root, &plan.staging_root)?;
    match fs::remove_dir(&plan.staging_root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
            bail!(
                "暂存目录仍包含未发布或未清理项目，请保留以便恢复: {}",
                plan.staging_root.display()
            )
        }
        Err(error) => Err(error).context("无法移除空的传输暂存目录"),
    }
}

/// Propose moving direct files into folders grouped by file type. No files are changed.
pub fn plan_organize(config: &FolderConfig) -> Result<OrganizePlan> {
    let current = folder::current_config(config)?;
    let root = current_root(&current)?;
    let mut plan = OrganizePlan {
        root: root.clone(),
        actions: Vec::new(),
        failures: Vec::new(),
        truncated: false,
    };
    let entries = fs::read_dir(&root)
        .with_context(|| format!("无法读取映射文件夹进行整理: {}", root.display()))?;
    let mut seen = 0usize;
    for entry in entries {
        if seen >= MAX_FOLDER_ENTRIES {
            plan.truncated = true;
            break;
        }
        seen += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                plan.failures.push(OperationResult::failure(
                    root.clone(),
                    None,
                    format!("读取目录项失败：{error}"),
                ));
                continue;
            }
        };
        let source = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(error) => {
                plan.failures.push(OperationResult::failure(
                    source,
                    None,
                    format!("读取目录项元数据失败：{error}"),
                ));
                continue;
            }
        };
        if metadata.is_dir() && is_sensitive_name(&name) {
            continue;
        }
        if is_reparse_point(&metadata) {
            plan.failures.push(OperationResult::failure(
                source,
                None,
                "不整理符号链接或 junction",
            ));
            continue;
        }
        if !current.show_hidden && is_hidden(&name, &metadata) {
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let category = category_for(&source);
        let target = root.join(&category).join(&name);
        match path_exists(&target) {
            Ok(true) => {
                plan.failures.push(OperationResult::failure(
                    source,
                    Some(target),
                    "分类目录中已有同名项目，拒绝覆盖",
                ));
                continue;
            }
            Err(error) => {
                plan.failures
                    .push(OperationResult::failure(source, Some(target), error));
                continue;
            }
            Ok(false) => {}
        }
        if !plan
            .actions
            .iter()
            .any(|action| crate::paths::paths_equal(&action.target, &target))
        {
            plan.actions.push(OrganizeAction {
                source,
                category,
                target,
            });
        }
    }
    plan.actions
        .sort_by(|left, right| left.source.cmp(&right.source));
    Ok(plan)
}

/// Apply a reviewed organization plan. Each item is independent; a conflict or I/O
/// error for one item does not stop the remaining moves.
pub fn apply_organize(config: &FolderConfig, plan: &OrganizePlan) -> Vec<OperationResult> {
    let mut results = plan.failures.clone();
    let root = match current_root(config) {
        Ok(root) if crate::paths::paths_equal(&root, &plan.root) => root,
        Ok(_) => {
            return plan
                .actions
                .iter()
                .map(|action| {
                    OperationResult::failure(
                        action.source.clone(),
                        Some(action.target.clone()),
                        "映射目录已变化，拒绝应用过期整理计划",
                    )
                })
                .chain(results)
                .collect()
        }
        Err(error) => {
            return plan
                .actions
                .iter()
                .map(|action| {
                    OperationResult::failure(
                        action.source.clone(),
                        Some(action.target.clone()),
                        &error,
                    )
                })
                .chain(results)
                .collect()
        }
    };

    for action in &plan.actions {
        let attempt = apply_organize_action(config, &root, action);
        results.push(match attempt {
            Ok(()) => OperationResult::success(action.source.clone(), action.target.clone()),
            Err(error) => {
                OperationResult::failure(action.source.clone(), Some(action.target.clone()), error)
            }
        });
    }
    results
}

fn apply_organize_action(
    config: &FolderConfig,
    root: &Path,
    action: &OrganizeAction,
) -> Result<()> {
    validate_entry_name(&action.category)?;
    let source = folder::resolve_entry(config, &action.source.to_string_lossy())?;
    if crate::paths::paths_equal(&source, root) {
        bail!("不能整理映射根目录本身");
    }
    let metadata = fs::symlink_metadata(&source)?;
    if is_reparse_point(&metadata) || !metadata.is_file() {
        bail!("整理来源不再是普通文件");
    }
    let name = source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("整理来源缺少文件名"))?;
    let expected = root.join(&action.category).join(name);
    if !crate::paths::paths_equal(&expected, &action.target) {
        bail!("整理计划目标已被修改");
    }
    let category_dir = root.join(&action.category);
    match fs::create_dir(&category_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let category_metadata = fs::symlink_metadata(&category_dir)?;
            if is_reparse_point(&category_metadata) || !category_metadata.is_dir() {
                bail!("分类目标不是普通目录");
            }
        }
        Err(error) => return Err(error).context("无法创建分类目录"),
    }
    ensure_absent(&action.target).context("分类目录中已有同名项目，拒绝覆盖")?;
    ensure_no_reparse_ancestors(&source)?;
    ensure_no_reparse_ancestors(&category_dir)?;
    no_replace_rename(&source, &action.target)
        .with_context(|| format!("无法移动到分类目录: {}", action.target.display()))
}

fn plan_one_transfer(
    root: &Path,
    staging_root: &Path,
    source: &Path,
    limits: TransferLimits,
    budget: &mut ManifestBudget,
    planned_targets: &[PathBuf],
) -> Result<PlannedTransfer> {
    validate_local_absolute(source)?;
    ensure_no_reparse_ancestors(source)?;
    let source_metadata = fs::symlink_metadata(source)
        .with_context(|| format!("传输来源不存在或不可读取: {}", source.display()))?;
    if is_reparse_point(&source_metadata) {
        bail!("不允许传输符号链接或 junction: {}", source.display());
    }
    if !source_metadata.is_file() && !source_metadata.is_dir() {
        bail!("只允许传输普通文件或文件夹: {}", source.display());
    }
    let canonical_source = canonicalize_without_extended_prefix(source)?;
    if crate::paths::paths_equal(root, &canonical_source)
        || crate::paths::path_is_within(&canonical_source, root)
    {
        bail!("不能把映射目录本身、其祖先或其内部项目再次导入");
    }
    let name = canonical_source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("传输来源名称不是有效文本"))?;
    validate_entry_name(name)?;
    let name = name.to_owned();
    let target = root.join(&name);
    if planned_targets
        .iter()
        .any(|planned| crate::paths::paths_equal(planned, &target))
    {
        bail!("批量传输中存在同名目标: {name}");
    }
    ensure_absent(&target).with_context(|| format!("目标已存在，拒绝覆盖: {name}"))?;
    let (manifest, entries, bytes, depth) = build_manifest(&canonical_source, limits, budget)?;
    Ok(PlannedTransfer {
        source: canonical_source,
        target,
        staged_target: staging_root.join(&name),
        manifest,
        entries,
        bytes,
        depth,
    })
}

#[derive(Debug)]
struct ManifestBudget {
    entries_left: usize,
    bytes_left: u64,
}

fn build_manifest(
    root: &Path,
    limits: TransferLimits,
    budget: &mut ManifestBudget,
) -> Result<(Vec<ManifestEntry>, usize, u64, usize)> {
    let mut pending = vec![(PathBuf::new(), 0usize)];
    let mut manifest = Vec::new();
    let mut item_bytes = 0u64;
    let mut item_depth = 0usize;

    while let Some((relative, depth)) = pending.pop() {
        if depth > limits.max_depth {
            bail!(
                "传输目录超过深度上限 {}：{}",
                limits.max_depth,
                root.display()
            );
        }
        if budget.entries_left == 0 {
            bail!("批量传输超过 {} 项上限", limits.max_entries);
        }
        let path = if relative.as_os_str().is_empty() {
            root.to_path_buf()
        } else {
            root.join(&relative)
        };
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("读取传输来源元数据失败: {}", path.display()))?;
        if is_reparse_point(&metadata) {
            bail!("传输树中含有符号链接或 junction: {}", path.display());
        }
        let file_type = metadata.file_type();
        let kind = if file_type.is_dir() {
            ManifestKind::Directory
        } else if file_type.is_file() {
            ManifestKind::File
        } else {
            bail!("传输树中含有非普通文件系统项目: {}", path.display());
        };
        budget.entries_left -= 1;
        item_depth = item_depth.max(depth);

        let (bytes, sha256) = if kind == ManifestKind::File {
            let size = metadata.len();
            if size > budget.bytes_left {
                bail!("传输超过 {} 字节上限", limits.max_bytes);
            }
            budget.bytes_left -= size;
            item_bytes = item_bytes.saturating_add(size);
            let hash = sha256_file(&path, size)
                .with_context(|| format!("读取传输来源失败: {}", path.display()))?;
            (size, hash)
        } else {
            (0, String::new())
        };
        manifest.push(ManifestEntry {
            relative_path: relative.clone(),
            kind,
            bytes,
            sha256,
        });

        if kind == ManifestKind::Directory {
            let mut children = Vec::new();
            let mut entries = fs::read_dir(&path)
                .with_context(|| format!("无法枚举传输目录: {}", path.display()))?
                .peekable();
            while let Some(entry) = entries.next() {
                if children.len() >= budget.entries_left {
                    bail!("传输超过 {} 项上限", limits.max_entries);
                }
                let entry =
                    entry.with_context(|| format!("枚举传输目录失败: {}", path.display()))?;
                children.push(entry.file_name());
            }
            children.sort();
            for name in children.into_iter().rev() {
                pending.push((relative.join(name), depth + 1));
            }
        }
    }
    manifest.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let entries = manifest.len();
    Ok((manifest, entries, item_bytes, item_depth))
}

fn verify_source_manifest(item: &PlannedTransfer, limits: TransferLimits) -> Result<()> {
    verify_manifest_matches(
        &item.source,
        &item.manifest,
        item.entries,
        item.bytes,
        limits,
    )
    .with_context(|| format!("传输来源自计划后发生变化: {}", item.source.display()))
}

fn verify_manifest_matches(
    root: &Path,
    expected: &[ManifestEntry],
    expected_entries: usize,
    expected_bytes: u64,
    limits: TransferLimits,
) -> Result<()> {
    let mut budget = ManifestBudget {
        entries_left: limits.max_entries,
        bytes_left: limits.max_bytes,
    };
    let (actual, entries, bytes, _) = build_manifest(root, limits, &mut budget)?;
    anyhow::ensure!(
        entries == expected_entries && bytes == expected_bytes && actual == expected,
        "文件、目录结构或内容与已审核计划不一致"
    );
    Ok(())
}

fn sha256_file(path: &Path, expected_size: u64) -> Result<String> {
    let before = fs::symlink_metadata(path)?;
    if is_reparse_point(&before) || !before.is_file() || before.len() != expected_size {
        bail!("文件在读取前发生变化");
    }
    let mut file = open_file_without_following_link(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut read = 0u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        read = read.saturating_add(count as u64);
        if read > expected_size {
            bail!("文件在读取期间增长");
        }
        hasher.update(&buffer[..count]);
    }
    let after_path = fs::symlink_metadata(path)?;
    let after_file = file.metadata()?;
    if is_reparse_point(&after_path)
        || !after_file.is_file()
        || read != expected_size
        || after_path.len() != expected_size
        || after_file.len() != expected_size
    {
        bail!("文件在读取期间发生变化");
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn open_file_without_following_link(path: &Path) -> Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .with_context(|| format!("无法打开文件: {}", path.display()))?;
    #[cfg(windows)]
    {
        let metadata = file.metadata()?;
        if is_reparse_point(&metadata) {
            bail!("不允许读取重解析点: {}", path.display());
        }
    }
    Ok(file)
}

fn effective_limits(mut limits: TransferLimits) -> Result<TransferLimits> {
    if limits.max_entries == 0 || limits.max_depth == 0 || limits.max_bytes == 0 {
        bail!("传输预算必须大于 0");
    }
    limits.max_entries = limits.max_entries.min(MAX_TRANSFER_ENTRIES);
    limits.max_depth = limits.max_depth.min(MAX_TRANSFER_DEPTH);
    limits.max_bytes = limits.max_bytes.min(MAX_TRANSFER_BYTES);
    Ok(limits)
}

fn current_root(config: &FolderConfig) -> Result<PathBuf> {
    let current = folder::current_config(config)?;
    let path = Path::new(&current.path);
    validate_local_absolute(path)?;
    ensure_no_reparse_ancestors(path)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("映射文件夹不可用: {}", path.display()))?;
    if is_reparse_point(&metadata) || !metadata.is_dir() {
        bail!("当前映射位置不是普通文件夹: {}", path.display());
    }
    canonicalize_without_extended_prefix(path)
}

fn validate_plan_root(config: &FolderConfig, plan: &TransferPlan) -> Result<PathBuf> {
    let root = current_root(config)?;
    if !crate::paths::paths_equal(&root, &plan.root) {
        bail!("映射目录已变化，拒绝应用过期传输计划");
    }
    Ok(root)
}

fn validate_transfer_paths(
    root: &Path,
    staging_root: &Path,
    index: usize,
    item: &PlannedTransfer,
) -> Result<()> {
    validate_staging_root_path(root, staging_root)?;
    let name = item
        .source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("传输来源缺少文件名"))?;
    validate_entry_name(
        name.to_str()
            .ok_or_else(|| anyhow::anyhow!("传输来源名称不是有效文本"))?,
    )?;
    if !crate::paths::paths_equal(&item.target, &root.join(name))
        || !crate::paths::paths_equal(&item.staged_target, &staging_root.join(name))
        || index >= MAX_TRANSFER_ENTRIES
    {
        bail!("传输计划路径与审核结果不一致");
    }
    if crate::paths::paths_equal(root, &item.source)
        || crate::paths::path_is_within(&item.source, root)
    {
        bail!("传输来源与映射目录存在递归关系");
    }
    Ok(())
}

fn validate_staging_root_path(root: &Path, staging_root: &Path) -> Result<()> {
    let name = staging_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("传输暂存目录名称无效"))?;
    validate_entry_name(name)?;
    if !staging_root
        .parent()
        .is_some_and(|parent| crate::paths::paths_equal(parent, root))
        || !name.starts_with(".mochi-transfer-staging-")
    {
        bail!("传输暂存目录必须是映射目录内的专用直接子项");
    }
    Ok(())
}

fn unique_staging_root(root: &Path) -> Result<PathBuf> {
    for _ in 0..64 {
        let sequence = STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let candidate = root.join(format!(
            ".mochi-transfer-staging-{}-{millis}-{sequence}",
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法检查暂存路径: {}", candidate.display()))
            }
        }
    }
    bail!("无法分配唯一的传输暂存目录")
}

fn validate_local_absolute(path: &Path) -> Result<()> {
    let text = path.to_string_lossy();
    if text.is_empty()
        || text.chars().any(char::is_control)
        || text.starts_with(r"\\")
        || text.starts_with("//")
        || !path.is_absolute()
    {
        bail!("路径必须是本地绝对路径且不能包含控制字符或 UNC 前缀");
    }
    #[cfg(windows)]
    {
        let mut components = path.components();
        match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => {}
            _ => bail!("路径不能使用 UNC 或设备前缀"),
        }
        if !matches!(components.next(), Some(Component::RootDir)) {
            bail!("路径必须包含磁盘根目录");
        }
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("路径不能包含 ..");
    }
    Ok(())
}

fn validate_entry_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(char::is_control)
        || name.chars().any(|ch| matches!(ch, '/' | '\\' | ':'))
        || is_sensitive_name(name)
    {
        bail!("名称为空、包含非法路径字符或使用了保留目录名");
    }
    #[cfg(windows)]
    {
        if name.ends_with('.')
            || name.ends_with(' ')
            || name.chars().any(|ch| "<>\"|?*".contains(ch))
            || windows_reserved_name(name)
        {
            bail!("名称不是有效的 Windows 文件或文件夹名称");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_reserved_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or_default();
    matches!(
        base.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn ensure_created_child(root: &Path, target: &Path) -> Result<()> {
    ensure_no_reparse_ancestors(target)?;
    let metadata = fs::symlink_metadata(target)?;
    if is_reparse_point(&metadata) || !metadata.is_dir() {
        bail!("新建目标不是普通目录: {}", target.display());
    }
    let canonical = canonicalize_without_extended_prefix(target)?;
    if !crate::paths::path_is_within(root, &canonical) {
        bail!("新建目标越出映射目录");
    }
    Ok(())
}

fn ensure_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("目标已存在: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("无法检查目标: {}", path.display())),
    }
}

fn path_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("无法检查目标: {}", path.display())),
    }
}

fn is_sensitive_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(".mochi") || name.eq_ignore_ascii_case(".git")
}

fn is_hidden(name: &str, metadata: &fs::Metadata) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x2 != 0;
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return metadata.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn ensure_no_reparse_ancestors(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    let mut rooted = false;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => {
                current.push(component.as_os_str());
                rooted = true;
            }
            Component::CurDir => {}
            Component::ParentDir => bail!("路径不能包含 .."),
            Component::Normal(part) => {
                current.push(part);
                if !rooted {
                    continue;
                }
            }
        }
        if !rooted {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_reparse_point(&metadata) => {
                bail!("路径经过符号链接或 junction: {}", current.display())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法检查路径组件: {}", current.display()))
            }
        }
    }
    Ok(())
}

fn canonicalize_without_extended_prefix(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if rest.starts_with("UNC\\") || rest.starts_with("UNC/") {
                bail!("网络路径不受支持");
            }
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(canonical)
}

fn category_for(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "heic" => "图片",
        "mp4" | "mov" | "mkv" | "avi" | "webm" => "视频",
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => "音频",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" => "文档",
        "zip" | "7z" | "rar" | "tar" | "gz" => "压缩包",
        "rs" | "c" | "h" | "cpp" | "hpp" | "py" | "js" | "ts" | "jsx" | "tsx" | "html" | "css"
        | "json" | "toml" | "yaml" | "yml" | "go" | "java" | "kt" | "swift" => "代码",
        _ => "其他",
    }
    .into()
}

#[cfg(windows)]
fn no_replace_rename(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileW(existing: *const u16, new: *const u16) -> i32;
    }
    let moved = unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) };
    if moved != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn no_replace_rename(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    const AT_FDCWD: i32 = -100;
    const RENAME_NOREPLACE: u32 = 1;
    unsafe extern "C" {
        fn renameat2(
            olddirfd: i32,
            oldpath: *const std::ffi::c_char,
            newdirfd: i32,
            newpath: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let moved = unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            destination.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if moved == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn no_replace_rename(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    const RENAME_EXCL: u32 = 0x0000_0004;
    unsafe extern "C" {
        fn renamex_np(
            oldpath: *const std::ffi::c_char,
            newpath: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let moved = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), RENAME_EXCL) };
    if moved == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn no_replace_rename(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "当前平台没有可用的原子 no-replace rename",
    ))
}

#[cfg(test)]
#[path = "folder_operations_tests.rs"]
mod tests;
