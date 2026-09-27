//! 暂存工作区文件写入，并在失败时回滚已应用的变更。
use super::*;

pub(super) struct PendingWorkspaceWrite {
    pub(super) target: PathBuf,
    pub(super) staged: Option<PathBuf>,
    pub(super) backup: Option<PathBuf>,
    pub(super) had_original: bool,
    pub(super) installed: bool,
}

pub(super) struct WorkspaceTransaction {
    pub(super) changes: Vec<PendingWorkspaceWrite>,
}

impl WorkspaceTransaction {
    pub(super) fn prepare(
        root: &Path,
        old_files: &BTreeMap<String, Vec<u8>>,
        next_files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        ensure_workspace_root(root)?;
        validate_file_map(old_files, true)?;
        validate_file_map(next_files, true)?;
        let mut paths = BTreeSet::new();
        paths.extend(old_files.keys().cloned());
        paths.extend(next_files.keys().cloned());
        let mut changes = Vec::new();
        for path in paths {
            let old = old_files.get(&path);
            let next = next_files.get(&path);
            let target = checked_workspace_path(root, &path)?;
            let metadata = fs::symlink_metadata(&target).ok();
            let exists = metadata.is_some();
            if let Some(metadata) = metadata {
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "workspace path is a symlink: {}",
                    target.display()
                );
                ensure!(
                    metadata.is_file(),
                    "workspace path is not a regular file: {}",
                    target.display()
                );
            }
            if let Some(expected) = old {
                ensure!(
                    exists,
                    "tracked workspace file was deleted outside sync: {}",
                    target.display()
                );
                let actual = fs::read(&target)
                    .with_context(|| format!("read tracked workspace file {}", target.display()))?;
                ensure!(
                    actual == *expected,
                    "tracked workspace file changed outside sync: {}",
                    target.display()
                );
            }
            // fast-forward 或合并可能带来与当前 commit 相同的树。
            // 上面按字节的跟踪校验照做，但未变化的路径不重写。
            if old == next {
                continue;
            }
            if next.is_some() && old.is_none() && exists {
                bail!(
                    "refusing to overwrite an untracked workspace file: {}",
                    target.display()
                );
            }

            let staged = if let Some(bytes) = next {
                let parent = target.parent().context("workspace target has no parent")?;
                fs::create_dir_all(parent)?;
                let path = unique_sibling(&target, "stage")?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                Some(path)
            } else {
                None
            };
            changes.push(PendingWorkspaceWrite {
                target,
                staged,
                backup: None,
                had_original: exists,
                installed: false,
            });
        }
        Ok(Self { changes })
    }

    pub(super) fn apply(&mut self) -> Result<()> {
        for index in 0..self.changes.len() {
            let result = self.apply_one(index);
            if let Err(error) = result {
                return match self.rollback() {
                    Ok(()) => Err(error),
                    Err(recovery) => Err(error.context(format!(
                        "workspace apply failed and rollback also failed: {recovery}"
                    ))),
                };
            }
        }
        Ok(())
    }

    pub(super) fn apply_one(&mut self, index: usize) -> Result<()> {
        let change = &mut self.changes[index];
        if change.had_original {
            let backup = unique_sibling(&change.target, "backup")?;
            fs::rename(&change.target, &backup)
                .with_context(|| format!("backup workspace file {}", change.target.display()))?;
            change.backup = Some(backup);
        }
        if let Some(staged) = change.staged.take() {
            if let Err(error) = fs::rename(&staged, &change.target) {
                // 暂存路径要和事务绑定在一起。即便临时文件自身的
                // 清理都不可用，失败的 rename 也必须保持可恢复。
                change.staged = Some(staged);
                return Err(error).with_context(|| {
                    format!("install workspace file {}", change.target.display())
                });
            }
            change.installed = true;
        }
        Ok(())
    }

    pub(super) fn rollback(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        for change in self.changes.iter_mut().rev() {
            if change.installed || change.backup.is_some() {
                if let Err(error) = remove_file_if_exists(&change.target) {
                    failures.push(format!(
                        "could not remove installed workspace path {}: {error}",
                        change.target.display()
                    ));
                } else {
                    change.installed = false;
                }
            }
            if let Some(backup) = change.backup.as_ref() {
                match fs::rename(backup, &change.target) {
                    Ok(()) => {
                        change.backup = None;
                    }
                    Err(error) => {
                        failures.push(format!(
                            "backup {} could not be restored to {}: {error}",
                            backup.display(),
                            change.target.display()
                        ));
                    }
                }
            }
            if let Some(staged) = change.staged.as_ref() {
                match remove_file_if_exists(staged) {
                    Ok(()) => {
                        change.staged = None;
                    }
                    Err(error) => failures.push(format!(
                        "staged file {} could not be removed: {error}",
                        staged.display()
                    )),
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            bail!(
                "workspace recovery paths were retained: {}",
                failures.join("; ")
            )
        }
    }

    pub(super) fn finish(&mut self) -> Option<WorkspaceCleanupWarning> {
        let mut failures = Vec::new();
        let mut recovery_paths = Vec::new();
        for change in &mut self.changes {
            if let Some(backup) = change.backup.as_ref() {
                match remove_file_if_exists(backup) {
                    Ok(()) => {
                        change.backup = None;
                    }
                    Err(error) => {
                        recovery_paths.push(backup.clone());
                        failures.push(format!(
                            "backup {} could not be removed ({error})",
                            backup.display()
                        ));
                    }
                }
            }
            if let Some(staged) = change.staged.as_ref() {
                match remove_file_if_exists(staged) {
                    Ok(()) => {
                        change.staged = None;
                    }
                    Err(error) => {
                        recovery_paths.push(staged.clone());
                        failures.push(format!(
                            "staged file {} could not be removed ({error})",
                            staged.display()
                        ));
                    }
                }
            }
        }
        if failures.is_empty() {
            None
        } else {
            Some(WorkspaceCleanupWarning {
                recovery_paths,
                details: format!(
                    "workspace cleanup failed after the commit point; recovery paths were retained: {}",
                    failures.join("; ")
                ),
            })
        }
    }
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn unique_sibling(target: &Path, label: &str) -> Result<PathBuf> {
    let parent = target.parent().context("target has no parent")?;
    let file_name = target
        .file_name()
        .context("target has no file name")?
        .to_string_lossy();
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(
            ".{file_name}.mochi-{label}-{}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!(
        "could not allocate a temporary workspace sibling for {}",
        target.display()
    )
}

fn checked_workspace_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = if relative == BLOCK_ID_COMMENT_PATH {
        relative.to_string()
    } else {
        validate_snapshot_path(relative)?
    };
    let components: Vec<&str> = relative.split('/').collect();
    let mut cursor = root.to_path_buf();
    for component in &components[..components.len().saturating_sub(1)] {
        cursor.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&cursor) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "workspace component is a symlink: {}",
                cursor.display()
            );
            ensure!(
                metadata.is_dir(),
                "workspace component is not a directory: {}",
                cursor.display()
            );
        }
    }
    cursor.push(components.last().context("empty workspace path")?);
    if let Ok(metadata) = fs::symlink_metadata(&cursor) {
        ensure!(
            !metadata.file_type().is_symlink(),
            "workspace target is a symlink: {}",
            cursor.display()
        );
        ensure!(
            metadata.is_file(),
            "workspace target is not a regular file: {}",
            cursor.display()
        );
    }
    Ok(cursor)
}
