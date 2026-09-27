//! Read-only snapshots for a user-selected local folder.

use super::{Action, Page, PageSnapshot, Row, RowMeta};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

pub const MAX_FOLDER_ENTRIES: usize = 4096;
pub const MAX_FOLDER_ROWS: usize = 500;
pub const MAX_FILTER_CHARS: usize = 120;
pub const MAX_FOLDER_PATH_CHARS: usize = super::MAX_SOURCE_CHARS;
pub const MAX_FOLDER_DIRECTORY_VIEWS: usize = 64;
pub const MAX_FOLDER_STACKS: usize = 64;
pub const MAX_FOLDER_NAME_CHARS: usize = 255;
pub const MAX_FOLDER_STACK_NAME_CHARS: usize = 80;
const MAX_FOLDER_VIEW_STATE_CHARS: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderSort {
    Name,
    Modified,
    Type,
    Manual,
}

impl Default for FolderSort {
    fn default() -> Self {
        Self::Name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderConfig {
    /// The explicitly selected local absolute folder. An empty path means no selection yet.
    pub path: String,
    /// A slash-separated path from `path` to the directory currently displayed.
    pub subfolder: String,
    /// Case-insensitive substring matched against each direct entry's name.
    pub filter: String,
    pub sort: FolderSort,
    pub show_hidden: bool,
    pub auto_organize: bool,
    /// Whether non-manually-stacked entries should be grouped by folder / file extension.
    pub group_by_type: bool,
    /// Names in the current directory, used when `sort` is `Manual`.
    pub manual_order: Vec<String>,
    /// User-created display groups for entries in the current directory.
    pub stacks: Vec<FolderStack>,
    /// Per-directory ordering and stack state, keyed by root-relative subfolder.
    pub directory_views: BTreeMap<String, FolderViewState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderStack {
    pub name: String,
    /// Direct entry names; these are display-only and never interpreted as filesystem paths.
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FolderViewState {
    pub manual_order: Vec<String>,
    pub stacks: Vec<FolderStack>,
}

impl Default for FolderConfig {
    fn default() -> Self {
        Self {
            path: String::new(),
            subfolder: String::new(),
            filter: String::new(),
            sort: FolderSort::Name,
            show_hidden: false,
            auto_organize: false,
            group_by_type: false,
            manual_order: Vec::new(),
            stacks: Vec::new(),
            directory_views: BTreeMap::new(),
        }
    }
}

impl FolderConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Validate the persisted folder settings without requiring that the folder exists.
pub fn validate(config: &FolderConfig) -> Result<()> {
    if !config.path.is_empty() {
        validate_local_absolute(&config.path, "映射文件夹")?;
    } else if !config.subfolder.is_empty() {
        bail!("尚未选择映射文件夹，不能设置子目录路径");
    }
    validate_subfolder(&config.subfolder)?;
    if config.filter.chars().count() > MAX_FILTER_CHARS {
        bail!("文件夹筛选文本不能超过 {MAX_FILTER_CHARS} 个字符");
    }
    if config.filter.chars().any(char::is_control) {
        bail!("文件夹筛选文本含有控制字符");
    }
    validate_view_state(&config.manual_order, &config.stacks)?;
    if config.directory_views.len() > MAX_FOLDER_DIRECTORY_VIEWS {
        bail!("每个文件夹最多保存 {MAX_FOLDER_DIRECTORY_VIEWS} 个目录视图状态");
    }
    let mut total_chars = config.path.chars().count()
        + config.subfolder.chars().count()
        + config.filter.chars().count();
    total_chars =
        total_chars.saturating_add(view_state_chars(&config.manual_order, &config.stacks));
    for (key, state) in &config.directory_views {
        validate_subfolder(key)?;
        if normalized_subfolder(key) != *key {
            bail!("目录视图缓存路径必须使用斜线格式");
        }
        validate_view_state(&state.manual_order, &state.stacks)?;
        total_chars = total_chars.saturating_add(key.chars().count());
        total_chars =
            total_chars.saturating_add(view_state_chars(&state.manual_order, &state.stacks));
    }
    if total_chars > MAX_FOLDER_VIEW_STATE_CHARS {
        bail!("文件夹排序与叠放设置过多");
    }
    Ok(())
}

fn validate_view_state(manual_order: &[String], stacks: &[FolderStack]) -> Result<()> {
    if manual_order.len() > MAX_FOLDER_ENTRIES {
        bail!("手动排序最多记录 {MAX_FOLDER_ENTRIES} 个目录项");
    }
    let mut ordered = std::collections::HashSet::new();
    for name in manual_order {
        validate_entry_name(name, "手动排序项")?;
        if !ordered.insert(name.as_str()) {
            bail!("手动排序中有重复目录项: {name}");
        }
    }
    if stacks.len() > MAX_FOLDER_STACKS {
        bail!("最多创建 {MAX_FOLDER_STACKS} 个文件夹叠放");
    }
    let mut stack_names = std::collections::HashSet::new();
    let mut stacked_items = std::collections::HashSet::new();
    let mut item_count = 0usize;
    for stack in stacks {
        let name = stack.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_FOLDER_STACK_NAME_CHARS
            || name.chars().any(char::is_control)
        {
            bail!("叠放名称不能为空、含控制字符或超过 {MAX_FOLDER_STACK_NAME_CHARS} 个字符");
        }
        if !stack_names.insert(name) {
            bail!("叠放名称重复: {name}");
        }
        item_count = item_count.saturating_add(stack.items.len());
        for item in &stack.items {
            validate_entry_name(item, "叠放项")?;
            if !stacked_items.insert(item.as_str()) {
                bail!("同一目录项不能加入多个叠放: {item}");
            }
        }
    }
    if item_count > MAX_FOLDER_ENTRIES {
        bail!("所有叠放最多记录 {MAX_FOLDER_ENTRIES} 个目录项");
    }
    Ok(())
}

fn validate_entry_name(name: &str, label: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > MAX_FOLDER_NAME_CHARS
        || name.chars().any(char::is_control)
        || name
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
    {
        bail!("{label}名称无效");
    }
    Ok(())
}

fn view_state_chars(manual_order: &[String], stacks: &[FolderStack]) -> usize {
    manual_order
        .iter()
        .map(|name| name.chars().count())
        .sum::<usize>()
        .saturating_add(
            stacks
                .iter()
                .map(|stack| {
                    stack.name.chars().count()
                        + stack
                            .items
                            .iter()
                            .map(|name| name.chars().count())
                            .sum::<usize>()
                })
                .sum::<usize>(),
        )
}

fn validate_subfolder(value: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    if value.chars().count() > MAX_FOLDER_PATH_CHARS
        || value.chars().any(char::is_control)
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
    {
        bail!("映射子目录必须是根目录内的相对路径");
    }
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            bail!("映射子目录路径不能包含空项、. 或 ..");
        }
        if is_sensitive_name(component) {
            bail!("映射子目录不能指向 .git 或 .mochi");
        }
    }
    Ok(())
}

fn normalized_subfolder(value: &str) -> String {
    value.replace('\\', "/")
}

/// Return a temporary config whose `path` is the canonical directory currently being shown.
/// The persisted config is left untouched and retains its mapping root and view cache.
pub fn current_config(config: &FolderConfig) -> Result<FolderConfig> {
    validate(config)?;
    if config.path.is_empty() {
        bail!("尚未选择本地文件夹");
    }
    let root = canonical_directory(Path::new(&config.path))
        .with_context(|| format!("映射文件夹不可用: {}", config.path))?;
    let current = if config.subfolder.is_empty() {
        root.clone()
    } else {
        canonical_directory(&root.join(&config.subfolder))
            .with_context(|| format!("映射子目录不可用: {}", config.subfolder))?
    };
    if !path_is_within(&root, &current) {
        bail!("映射子目录必须位于所选根文件夹内");
    }
    validate_local_absolute(&current.to_string_lossy(), "当前映射文件夹")?;

    let mut current_config = config.clone();
    current_config.path = current.to_string_lossy().into_owned();
    current_config.subfolder.clear();
    Ok(current_config)
}

/// Enter one direct child directory and restore its bounded per-directory view state.
pub fn navigate(config: &FolderConfig, target: &str) -> Result<FolderConfig> {
    validate(config)?;
    let current = current_config(config)?;
    let target = resolve_entry(config, target)?;
    if same_path(Path::new(&current.path), &target) {
        bail!("已经位于该文件夹");
    }
    let metadata = fs::symlink_metadata(&target)
        .with_context(|| format!("无法读取目标文件夹: {}", target.display()))?;
    if is_link_or_junction(&metadata) || !metadata.is_dir() {
        bail!("只能进入映射目录中的直接子文件夹");
    }

    let root = canonical_directory(Path::new(&config.path))?;
    let relative = relative_subfolder(&root, &target)?;
    let mut next = transition_view(config, &relative);
    next.subfolder = relative;
    validate(&next)?;
    Ok(next)
}

/// Move up one lexical directory without ever crossing the selected mapping root.
pub fn parent(config: &FolderConfig) -> Result<FolderConfig> {
    validate(config)?;
    if config.subfolder.is_empty() {
        return Ok(config.clone());
    }
    let target = config
        .subfolder
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or_default();
    let mut next = transition_view(config, target);
    next.subfolder = target.to_owned();
    validate(&next)?;
    Ok(next)
}

/// Return to the selected mapping root, preserving the current and root view states.
pub fn root(config: &FolderConfig) -> Result<FolderConfig> {
    validate(config)?;
    if config.subfolder.is_empty() {
        return Ok(config.clone());
    }
    let mut next = transition_view(config, "");
    next.subfolder.clear();
    validate(&next)?;
    Ok(next)
}

fn transition_view(config: &FolderConfig, target_subfolder: &str) -> FolderConfig {
    let current_key = normalized_subfolder(&config.subfolder);
    let target_key = normalized_subfolder(target_subfolder);
    let mut next = config.clone();
    if current_key == target_key {
        return next;
    }

    let current_view = FolderViewState {
        manual_order: config.manual_order.clone(),
        stacks: config.stacks.clone(),
    };
    if !next.directory_views.contains_key(&current_key)
        && next.directory_views.len() >= MAX_FOLDER_DIRECTORY_VIEWS
    {
        if let Some(evict) = next
            .directory_views
            .keys()
            .find(|key| key.as_str() != current_key.as_str() && key.as_str() != target_key.as_str())
            .cloned()
        {
            next.directory_views.remove(&evict);
        }
    }
    if next.directory_views.contains_key(&current_key)
        || next.directory_views.len() < MAX_FOLDER_DIRECTORY_VIEWS
    {
        next.directory_views.insert(current_key, current_view);
    }

    if let Some(view) = next.directory_views.get(&target_key).cloned() {
        next.manual_order = view.manual_order;
        next.stacks = view.stacks;
    } else {
        next.manual_order.clear();
        next.stacks.clear();
    }
    next
}

fn relative_subfolder(root: &Path, path: &Path) -> Result<String> {
    if !path_is_within(root, path) {
        bail!("目标文件夹已超出映射根目录");
    }
    let root_depth = root.components().count();
    Ok(path
        .components()
        .skip(root_depth)
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

fn path_is_within(root: &Path, path: &Path) -> bool {
    let root_components = root.components().collect::<Vec<_>>();
    let path_components = path.components().collect::<Vec<_>>();
    if path_components.len() < root_components.len() {
        return false;
    }
    root_components
        .iter()
        .zip(path_components.iter())
        .all(|(root, path)| same_path(Path::new(root.as_os_str()), Path::new(path.as_os_str())))
}

/// Create a fresh, bounded snapshot of one directory level.
///
/// Files are inspected through filesystem metadata only. Their contents are never opened.
/// Missing and unreadable folders remain visible as explicit read errors in the snapshot.
pub fn snapshot(page: &Page, max_entries: usize, max_rows: usize) -> Result<PageSnapshot> {
    Ok(snapshot_with_scan(page, max_entries, max_rows)?.snapshot)
}

pub(crate) struct ScanSnapshot {
    pub snapshot: PageSnapshot,
    /// Raw directory entries inspected, including hidden, ignored, and link entries.
    pub entries_scanned: usize,
}

pub(crate) fn snapshot_with_scan(
    page: &Page,
    max_entries: usize,
    max_rows: usize,
) -> Result<ScanSnapshot> {
    validate(&page.folder)?;
    let mut snapshot = PageSnapshot::empty(page);
    snapshot.subtitle = folder_subtitle(page);

    if page.folder.path.is_empty() {
        snapshot.empty_message = "尚未选择本地文件夹".into();
        return Ok(ScanSnapshot {
            snapshot,
            entries_scanned: 0,
        });
    }
    let show_files = page.selected("files");
    let show_folders = page.selected("folders");
    if !show_files && !show_folders {
        snapshot.empty_message = "未选择展示内容".into();
        return Ok(ScanSnapshot {
            snapshot,
            entries_scanned: 0,
        });
    }

    let entry_limit = max_entries.min(MAX_FOLDER_ENTRIES);
    let row_limit = page_row_limit(page, max_rows);
    if entry_limit == 0 {
        push_subtitle_note(&mut snapshot, "本次刷新扫描预算已用尽，内容可能不完整");
        snapshot.empty_message = "读取失败：本次刷新文件夹扫描预算已用尽".into();
        return Ok(ScanSnapshot {
            snapshot,
            entries_scanned: 0,
        });
    }

    let mut scanned = 0usize;
    let mut visible = Vec::new();
    let mut truncated = false;
    let mut skipped_links = 0;
    let mut read_error = None;
    let filter = page.folder.filter.to_lowercase();
    let stack_groups = stack_group_map(&page.folder);
    let manual_order = manual_order_map(&page.folder.manual_order);

    let result = (|| -> Result<()> {
        let current = current_config(&page.folder)
            .with_context(|| format!("无法读取映射文件夹 {}", page.folder.path))?;
        let canonical_root = Path::new(&current.path);
        let mut entries = fs::read_dir(&canonical_root)
            .with_context(|| format!("无法读取映射文件夹 {}", page.folder.path))?;

        while scanned < entry_limit {
            let Some(entry) = entries.next() else {
                break;
            };
            scanned = scanned.saturating_add(1);
            let entry = entry
                .with_context(|| format!("读取映射文件夹 {} 的目录项失败", page.folder.path))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let entry_path = entry.path();
            let metadata = fs::symlink_metadata(&entry_path)
                .with_context(|| format!("读取映射目录项元数据失败: {}", entry_path.display()))?;

            // Sensitive directories are always hidden. Reparse points and symlinks are
            // never followed, even if the user asks to show hidden entries.
            if is_link_or_junction(&metadata) {
                skipped_links += 1;
                continue;
            }
            if is_sensitive_directory(&name, &metadata) {
                continue;
            }
            if !page.folder.show_hidden && is_hidden(&name, &metadata) {
                continue;
            }

            let file_type = metadata.file_type();
            let is_directory = file_type.is_dir();
            if (is_directory && !show_folders) || (!is_directory && !show_files) {
                continue;
            }
            if !is_directory && !file_type.is_file() {
                continue;
            }
            if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                continue;
            }
            let group = entry_group(&page.folder, &name, is_directory, &stack_groups);
            let manual_rank = manual_order.get(&name).copied();
            visible.push(folder_entry(
                page,
                entry_path,
                name,
                metadata,
                is_directory,
                group,
                manual_rank,
            ));
        }

        if scanned == entry_limit {
            match entries.next() {
                Some(Ok(_)) => truncated = true,
                Some(Err(error)) => {
                    return Err(error).context("检查映射文件夹是否还有更多条目失败")
                }
                None => {}
            }
        }
        Ok(())
    })();

    if let Err(error) = result {
        read_error = Some(error.to_string());
    }

    visible.sort_by(|left, right| compare_entries(&page.folder.sort, left, right));
    let rows_found = visible.len();
    let rows_truncated = rows_found > row_limit;
    visible.truncate(row_limit);
    snapshot.rows = visible.into_iter().map(|entry| entry.row).collect();

    if skipped_links > 0 {
        push_subtitle_note(
            &mut snapshot,
            &format!("已跳过 {skipped_links} 个链接或云占位项"),
        );
    }
    if truncated {
        push_subtitle_note(
            &mut snapshot,
            &format!("扫描达到本次 {entry_limit} 项上限，内容已截断"),
        );
    }
    if rows_truncated {
        push_subtitle_note(
            &mut snapshot,
            &format!("显示达到本次 {row_limit} 行上限，内容已截断"),
        );
    }
    if let Some(error) = read_error {
        push_subtitle_note(&mut snapshot, &format!("读取失败：{error}"));
        if snapshot.rows.is_empty() {
            snapshot.empty_message = format!("读取失败：{error}");
        }
    } else if snapshot.rows.is_empty() {
        snapshot.empty_message = if truncated {
            "已达到扫描上限，当前条件下没有可展示内容".into()
        } else {
            format!("{}暂无匹配项目", page.title)
        };
    } else if rows_truncated {
        snapshot.empty_message = format!("仅显示前 {row_limit} 项，更多内容已截断");
    }

    Ok(ScanSnapshot {
        snapshot,
        entries_scanned: scanned,
    })
}

/// Revalidate a generated folder action at click time. Only the mapping root or one of its
/// direct, non-link children can be returned.
pub fn resolve_entry(config: &FolderConfig, target: &str) -> Result<PathBuf> {
    let current = current_config(config)?;
    validate_local_absolute(target, "文件夹条目")?;

    let root = Path::new(&current.path);
    let canonical_root =
        canonical_directory(root).with_context(|| format!("映射文件夹不可用: {}", current.path))?;
    let candidate = Path::new(target);
    ensure_no_link_ancestors(candidate)?;
    let metadata = fs::symlink_metadata(candidate)
        .with_context(|| format!("映射目录项已不存在或不可读取: {target}"))?;
    if is_link_or_junction(&metadata) {
        bail!("暂不支持重解析点（链接或云占位项）: {target}");
    }
    if !metadata.is_dir() && !metadata.is_file() {
        bail!("映射目录项不是普通文件或文件夹: {target}");
    }

    let canonical_candidate = canonicalize_without_extended_prefix(candidate)
        .with_context(|| format!("映射目录项已不存在或不可读取: {target}"))?;
    if same_path(&canonical_candidate, &canonical_root) {
        return Ok(canonical_root);
    }
    if !config.show_hidden
        && is_hidden(
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
            &metadata,
        )
    {
        bail!("映射目录项当前处于隐藏状态: {target}");
    }
    if metadata.is_dir()
        && candidate
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_sensitive_name)
    {
        bail!("不允许打开敏感目录: {target}");
    }
    let parent = canonical_candidate
        .parent()
        .ok_or_else(|| anyhow::anyhow!("映射目录项没有父文件夹: {target}"))?;
    if !same_path(parent, &canonical_root) {
        bail!("只能打开当前映射目录或其直接子项: {target}");
    }
    Ok(canonical_candidate)
}

fn page_row_limit(page: &Page, max_rows: usize) -> usize {
    let requested = if page.limit == 0 {
        max_rows
    } else {
        page.limit.min(max_rows)
    };
    requested.min(MAX_FOLDER_ROWS)
}

struct FolderEntry {
    row: Row,
    name_lower: String,
    name_raw: String,
    extension_lower: String,
    is_directory: bool,
    modified: Option<SystemTime>,
    group_kind: u8,
    group_order: usize,
    group_key: String,
    manual_rank: Option<usize>,
}

struct EntryGroup {
    label: String,
    kind: u8,
    order: usize,
    key: String,
}

fn folder_entry(
    page: &Page,
    path: PathBuf,
    raw_name: String,
    metadata: fs::Metadata,
    is_directory: bool,
    group: EntryGroup,
    manual_rank: Option<usize>,
) -> FolderEntry {
    let shown_name = if is_directory || page.presentation.show_extensions {
        raw_name.clone()
    } else {
        Path::new(&raw_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or(&raw_name)
            .to_string()
    };
    let extension = if is_directory {
        String::new()
    } else {
        Path::new(&raw_name)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_string()
    };
    let modified = metadata.modified().ok();
    let mut details = Vec::new();
    if is_directory {
        details.push("文件夹".to_string());
    } else {
        details.push(format_file_size(metadata.len()));
        if !page.presentation.show_extensions && !extension.is_empty() {
            details.push(extension.clone());
        }
    }
    if page.presentation.show_modified {
        if let Some(time) = modified {
            let local: chrono::DateTime<chrono::Local> = time.into();
            details.push(local.format("%Y-%m-%d %H:%M").to_string());
        }
    }
    let absolute = path.to_string_lossy().into_owned();
    FolderEntry {
        row: Row {
            id: format!("folder-entry:{absolute}"),
            title: shown_name,
            detail: details.join(" · "),
            action: Some(Action::OpenFolderEntry(absolute.clone())),
            checked: None,
            meta: RowMeta {
                icon: if is_directory {
                    "folder".into()
                } else {
                    "file".into()
                },
                path: absolute,
                group: group.label,
                directory: is_directory,
                always_detail: page.presentation.show_modified,
                ..Default::default()
            },
        },
        name_lower: raw_name.to_lowercase(),
        name_raw: raw_name,
        extension_lower: extension.to_lowercase(),
        is_directory,
        modified,
        group_kind: group.kind,
        group_order: group.order,
        group_key: group.key,
        manual_rank,
    }
}

fn compare_entries(sort: &FolderSort, left: &FolderEntry, right: &FolderEntry) -> Ordering {
    left.group_kind
        .cmp(&right.group_kind)
        .then_with(|| match left.group_kind.cmp(&right.group_kind) {
            Ordering::Equal if left.group_kind < 2 => left.group_order.cmp(&right.group_order),
            Ordering::Equal => Ordering::Equal,
            ordering => ordering,
        })
        .then_with(|| {
            if left.group_kind == 1 {
                left.group_key.cmp(&right.group_key)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| {
            if *sort == FolderSort::Manual {
                compare_manual_rank(left.manual_rank, right.manual_rank)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| right.is_directory.cmp(&left.is_directory))
        .then_with(|| match sort {
            FolderSort::Name | FolderSort::Manual => Ordering::Equal,
            FolderSort::Modified => right.modified.cmp(&left.modified),
            FolderSort::Type => left.extension_lower.cmp(&right.extension_lower),
        })
        .then_with(|| left.name_lower.cmp(&right.name_lower))
        .then_with(|| left.name_raw.cmp(&right.name_raw))
        .then_with(|| left.row.meta.path.cmp(&right.row.meta.path))
}

fn compare_manual_rank(left: Option<usize>, right: Option<usize>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn stack_group_map(config: &FolderConfig) -> std::collections::HashMap<String, (String, usize)> {
    let mut groups = std::collections::HashMap::new();
    for (stack_index, stack) in config.stacks.iter().enumerate() {
        let label = format!("叠放：{}", stack.name);
        for item in &stack.items {
            groups.insert(item.clone(), (label.clone(), stack_index));
        }
    }
    groups
}

fn manual_order_map(order: &[String]) -> std::collections::HashMap<String, usize> {
    order
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect()
}

fn entry_group(
    config: &FolderConfig,
    name: &str,
    is_directory: bool,
    stacks: &std::collections::HashMap<String, (String, usize)>,
) -> EntryGroup {
    if let Some((label, order)) = stacks.get(name) {
        return EntryGroup {
            label: label.clone(),
            kind: 0,
            order: *order,
            key: String::new(),
        };
    }
    if config.group_by_type {
        let (key, order) = if is_directory {
            ("文件夹".to_owned(), 0)
        } else {
            let extension = Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if extension.is_empty() {
                ("无扩展名".to_owned(), 1)
            } else {
                (format!(".{extension}"), 1)
            }
        };
        return EntryGroup {
            label: format!("类型：{key}"),
            kind: 1,
            order,
            key,
        };
    }
    EntryGroup {
        label: String::new(),
        kind: 2,
        order: 0,
        key: String::new(),
    }
}

fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn folder_subtitle(page: &Page) -> String {
    if page.folder.path.is_empty() {
        "文件夹：未选择".into()
    } else if page.folder.subfolder.is_empty() {
        format!("文件夹：{}", page.folder.path)
    } else {
        format!(
            "文件夹：{}",
            Path::new(&page.folder.path)
                .join(&page.folder.subfolder)
                .display()
        )
    }
}

fn push_subtitle_note(snapshot: &mut PageSnapshot, note: &str) {
    if snapshot.subtitle.is_empty() {
        snapshot.subtitle = note.into();
    } else {
        snapshot.subtitle = format!("{note} · {}", snapshot.subtitle);
    }
}

fn is_sensitive_directory(name: &str, metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && is_sensitive_name(name)
}

fn is_sensitive_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(".mochi") || name.eq_ignore_ascii_case(".git")
}

fn is_hidden(name: &str, metadata: &fs::Metadata) -> bool {
    name.starts_with('.') || has_hidden_attribute(metadata)
}

#[cfg(windows)]
fn has_hidden_attribute(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn has_hidden_attribute(_metadata: &fs::Metadata) -> bool {
    false
}

fn is_link_or_junction(metadata: &fs::Metadata) -> bool {
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

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let drive = path.components().take(2).collect::<PathBuf>();
        let wide: Vec<u16> = drive.as_os_str().encode_wide().chain(Some(0)).collect();
        // DRIVE_REMOTE = 4. This also rejects mapped network drives such as Z:\.
        if unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) } == 4 {
            bail!("暂不支持映射网络盘: {}", path.display());
        }
    }
    ensure_no_link_ancestors(path)?;
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("映射文件夹不存在或不可读取: {}", path.display()))?;
    if is_link_or_junction(&metadata) {
        bail!("暂不支持重解析点（链接或云占位目录）: {}", path.display());
    }
    if !metadata.is_dir() {
        bail!("映射路径不是文件夹: {}", path.display());
    }
    canonicalize_without_extended_prefix(path)
        .with_context(|| format!("无法解析映射文件夹: {}", path.display()))
}

fn ensure_no_link_ancestors(path: &Path) -> Result<()> {
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
            Component::ParentDir => bail!("路径不能包含 ..: {}", path.display()),
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
            Ok(metadata) if is_link_or_junction(&metadata) => {
                bail!(
                    "路径不能经过重解析点（链接或云占位目录）: {}",
                    current.display()
                );
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

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .replace('/', "\\")
            .eq_ignore_ascii_case(&right.to_string_lossy().replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn validate_local_absolute(value: &str, label: &str) -> Result<()> {
    if value.chars().count() > MAX_FOLDER_PATH_CHARS {
        bail!("{label}路径过长");
    }
    if value.chars().any(char::is_control) {
        bail!("{label}路径含有控制字符");
    }
    if value.starts_with(r"\\") || value.starts_with("//") {
        bail!("{label}不能使用 UNC 或网络路径");
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        bail!("{label}必须是本地绝对路径");
    }
    #[cfg(windows)]
    {
        let mut components = path.components();
        match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), std::path::Prefix::Disk(_)) => {}
            _ => bail!("{label}必须是本地磁盘路径，不能使用 UNC 或设备前缀"),
        }
        if !matches!(components.next(), Some(Component::RootDir)) {
            bail!("{label}必须包含磁盘根目录");
        }
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            bail!("{label}路径不能包含 ..");
        }
    }
    Ok(())
}
