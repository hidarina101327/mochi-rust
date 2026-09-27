//! 定义同步推送、拉取结果及冲突报告数据。
use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PushOutcome {
    Pushed { head: Oid },
    AlreadyUpToDate { head: Oid },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PullOutcome {
    NoRemote,
    UpToDate { head: Oid },
    LocalAhead { head: Oid, remote_head: Oid },
    FastForward { head: Oid },
    Merged { head: Oid },
    Conflict { report: ConflictReport },
}

pub type PushResult = PushOutcome;

pub type PullResult = PullOutcome;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictFile {
    pub path: String,
    pub base: Option<Vec<u8>>,
    pub ours: Option<Vec<u8>>,
    pub theirs: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictReport {
    pub base_head: Option<Oid>,
    pub ours_head: Oid,
    pub theirs_head: Oid,
    pub conflicts: Vec<ConflictFile>,
    /// 不冲突的那部分结果。冲突路径刻意留空，
    /// 直到调用方显式解决它们。
    pub proposed: Snapshot,
}

impl ConflictReport {
    pub fn resolution(&self) -> MergeResolution {
        MergeResolution::from_report(self)
    }

    pub fn conflict_paths(&self) -> impl Iterator<Item = &str> {
        self.conflicts.iter().map(|conflict| conflict.path.as_str())
    }
}

/// 由调用方持有的一组显式冲突决定。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeResolution {
    pub base_head: Option<Oid>,
    pub ours_head: Oid,
    pub theirs_head: Oid,
    pub(super) proposed: Snapshot,
    pub(super) unresolved: BTreeSet<String>,
}

impl MergeResolution {
    pub fn from_report(report: &ConflictReport) -> Self {
        Self {
            base_head: report.base_head,
            ours_head: report.ours_head,
            theirs_head: report.theirs_head,
            proposed: report.proposed.clone(),
            unresolved: report
                .conflicts
                .iter()
                .map(|conflict| conflict.path.clone())
                .collect(),
        }
    }

    pub fn unresolved_paths(&self) -> impl Iterator<Item = &str> {
        self.unresolved.iter().map(String::as_str)
    }

    pub fn proposed_snapshot(&self) -> &Snapshot {
        &self.proposed
    }

    pub fn resolve_file(&mut self, path: impl Into<String>, bytes: impl AsRef<[u8]>) -> Result<()> {
        let path = validate_snapshot_path(&path.into())?;
        ensure!(
            self.unresolved.contains(&path),
            "path is not an unresolved conflict: {path}"
        );
        upsert_snapshot_file(&mut self.proposed, SnapshotFile::new(path.clone(), bytes)?);
        self.unresolved.remove(&path);
        self.proposed.validate()
    }

    pub fn delete_file(&mut self, path: impl Into<String>) -> Result<()> {
        let path = validate_snapshot_path(&path.into())?;
        ensure!(
            self.unresolved.contains(&path),
            "path is not an unresolved conflict: {path}"
        );
        self.proposed.files.retain(|file| file.path != path);
        if !self.proposed.deleted_paths.contains(&path) {
            self.proposed.deleted_paths.push(path.clone());
        }
        self.unresolved.remove(&path);
        self.proposed.validate()
    }
}

/// 设备本地的 bare 历史 + 一个显式选定的工作区。
pub struct DeviceRepo {
    pub(super) config: DeviceConfig,
    pub(super) transport: Arc<dyn Transport>,
    pub(super) last_workspace_warning: Arc<Mutex<Option<WorkspaceCleanupWarning>>>,
}

pub type BlockRepository = DeviceRepo;

pub type DeviceRepository = DeviceRepo;

pub(super) fn merge_file_maps(
    base: &CommittedFiles,
    ours: &CommittedFiles,
    theirs: &CommittedFiles,
    block_id: &str,
) -> Result<MergeMaps> {
    let mut paths = BTreeSet::new();
    paths.extend(base.keys().cloned());
    paths.extend(ours.keys().cloned());
    paths.extend(theirs.keys().cloned());

    let mut merged = BTreeMap::new();
    let mut conflicts = Vec::new();
    for path in paths {
        if path == BLOCK_ID_COMMENT_PATH {
            continue;
        }
        let base_value = base.get(&path);
        let ours_value = ours.get(&path);
        let theirs_value = theirs.get(&path);
        if ours_value == theirs_value {
            if let Some(value) = ours_value {
                merged.insert(path, value.clone());
            }
        } else if ours_value == base_value {
            if let Some(value) = theirs_value {
                merged.insert(path, value.clone());
            }
        } else if theirs_value == base_value {
            if let Some(value) = ours_value {
                merged.insert(path, value.clone());
            }
        } else {
            conflicts.push(ConflictFile {
                path,
                base: base_value.cloned(),
                ours: ours_value.cloned(),
                theirs: theirs_value.cloned(),
            });
        }
    }
    merged.insert(
        BLOCK_ID_COMMENT_PATH.to_string(),
        block_id_comment_bytes(block_id),
    );
    validate_file_map(&merged, true)?;
    Ok((merged, conflicts))
}
