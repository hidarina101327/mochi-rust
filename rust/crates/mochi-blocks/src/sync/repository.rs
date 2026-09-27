//! 管理同步仓库的创建、打开、推送、拉取和清理。
use super::*;

/// 报出这个警告时，commit 已经推进了 bare 分支。列出的临时文件
/// 会被保留，调用方可以在被打断的清理之后取回或删除它们；
/// 它们绝不会被悄悄丢弃。`DeviceRepo::take_workspace_cleanup_warning`
/// 取走最近一次警告。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCleanupWarning {
    pub recovery_paths: Vec<PathBuf>,
    pub details: String,
}

impl DeviceRepo {
    /// 初始化一个新的 bare 仓库和它的第一条元数据提交。
    pub fn init(config: DeviceConfig) -> Result<Self> {
        Self::init_with_transport(config, Arc::new(NoTransport))
    }

    pub fn init_with_transport<T>(config: DeviceConfig, transport: Arc<T>) -> Result<Self>
    where
        T: Transport + 'static,
    {
        config.validate()?;
        ensure_workspace_root(&config.workspace_path)?;
        let repo = init_new_bare(&config.bare_path)?;
        let device = Self::new(config, transport);

        let mut initial = BTreeMap::new();
        initial.insert(
            BLOCK_ID_COMMENT_PATH.to_string(),
            block_id_comment_bytes(&device.config.block_id),
        );
        let commit = device.create_commit_object(&repo, &initial, None, &[], "initial block")?;
        let mut workspace = WorkspaceTransaction::prepare(
            &device.config.workspace_path,
            &BTreeMap::new(),
            &initial,
        )?;
        workspace.apply()?;
        if let Err(error) = update_ref_if_current(
            &repo,
            &device.config.branch_ref(),
            None,
            commit,
            "initial block",
        ) {
            return match workspace.rollback() {
                Ok(()) => Err(error),
                Err(recovery) => Err(error.context(format!(
                    "initial ref update failed and workspace rollback also failed: {recovery}"
                ))),
            };
        }
        if let Some(warning) = workspace.finish() {
            device.record_workspace_warning(warning);
        }
        repo.set_head(&device.config.branch_ref())?;
        Ok(device)
    }

    /// 为新设备显式命名并创建一个空的 bare 仓库。
    /// 第一次 pull 时可以从远端块历史 fast-forward 过来。
    pub fn create_empty(config: DeviceConfig) -> Result<Self> {
        Self::create_empty_with_transport(config, Arc::new(NoTransport))
    }

    pub fn create_empty_with_transport<T>(config: DeviceConfig, transport: Arc<T>) -> Result<Self>
    where
        T: Transport + 'static,
    {
        config.validate()?;
        ensure_workspace_root(&config.workspace_path)?;
        init_new_bare(&config.bare_path)?;
        Self::open_with_transport(config, transport)
    }

    /// 打开调用方给定的 bare 仓库。适用于这样的新设备：bare 仓库
    /// 还是空的，第一条提交将由 fetch 送来。不会自动发现任何路径。
    pub fn open(config: DeviceConfig) -> Result<Self> {
        Self::open_with_transport(config, Arc::new(NoTransport))
    }

    pub fn open_with_transport<T>(config: DeviceConfig, transport: Arc<T>) -> Result<Self>
    where
        T: Transport + 'static,
    {
        config.validate()?;
        ensure_workspace_root(&config.workspace_path)?;
        let repo = Repository::open_bare(&config.bare_path).with_context(|| {
            format!(
                "open explicit bare repository {}",
                config.bare_path.display()
            )
        })?;
        let device = Self::new(config, transport);
        if let Some(head) = ref_target(&repo, &device.config.branch_ref())? {
            device.ensure_block_identity(&repo, head)?;
        }
        Ok(device)
    }

    pub fn config(&self) -> &DeviceConfig {
        &self.config
    }

    pub fn bare_path(&self) -> &Path {
        &self.config.bare_path
    }

    pub fn workspace_path(&self) -> &Path {
        &self.config.workspace_path
    }

    pub fn branch(&self) -> &str {
        &self.config.branch
    }

    pub fn block_id(&self) -> &str {
        &self.config.block_id
    }

    pub fn transport_capabilities(&self) -> TransportCapabilities {
        self.transport.capabilities()
    }

    pub fn set_transport<T>(&mut self, transport: Arc<T>)
    where
        T: Transport + 'static,
    {
        self.transport = transport;
    }

    /// 取走最近一次「提交后工作区清理警告」，如果有。
    ///
    /// 有警告意味着分支和历史都已提交；只是删除临时的备份/暂存
    /// 文件失败了。这些路径仍留在返回值里，可以恢复。
    pub fn take_workspace_cleanup_warning(&self) -> Option<WorkspaceCleanupWarning> {
        match self.last_workspace_warning.lock() {
            Ok(mut warning) => warning.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        }
    }

    pub fn head(&self) -> Result<Option<Oid>> {
        let repo = self.open_bare()?;
        ref_target(&repo, &self.config.branch_ref())
    }

    pub fn head_string(&self) -> Result<Option<String>> {
        Ok(self.head()?.map(|head| head.to_string()))
    }

    pub fn persisted_block_id(&self) -> Result<String> {
        let repo = self.open_bare()?;
        let head = self.head()?.context("device has no branch head")?;
        let files = commit_files(&repo, head)?;
        parse_block_id(
            files
                .get(BLOCK_ID_COMMENT_PATH)
                .context("block id comment file is missing")?,
        )
    }

    /// 读取已提交的快照。它读的是 Git 历史，不是工作区，
    /// 因此不可能意外带上未跟踪或私密的文件。
    pub fn snapshot(&self) -> Result<Snapshot> {
        let repo = self.open_bare()?;
        let head = self.head()?.context("device has no branch head")?;
        self.snapshot_at(&repo, head)
    }

    pub fn commit_snapshot(&self, snapshot: Snapshot, message: impl AsRef<str>) -> Result<Oid> {
        let message = message.as_ref();
        ensure!(
            !message.as_bytes().contains(&0),
            "commit message contains NUL"
        );
        let repo = self.open_bare()?;
        let old_head = ref_target(&repo, &self.config.branch_ref())?;
        let old_files = match old_head {
            Some(head) => commit_files(&repo, head)?,
            None => BTreeMap::new(),
        };
        let next_files = self.apply_snapshot(&old_files, &snapshot)?;
        if next_files == old_files {
            if old_head.is_some() {
                // 即便是 no-op 提交，也要校验物化出来的被跟踪字节。
                // 否则外部对文件的改/删，会因为「这次不用写树」被悄悄放过。
                let _ = WorkspaceTransaction::prepare(
                    &self.config.workspace_path,
                    &old_files,
                    &next_files,
                )?;
            }
            return old_head.context("unchanged snapshot on an empty device");
        }

        let commit = self.create_commit_object(&repo, &next_files, old_head, &[], message)?;
        self.finish_local_commit(&repo, old_head, &old_files, next_files, commit, message)
    }

    pub fn commit(&self, snapshot: Snapshot, message: impl AsRef<str>) -> Result<Oid> {
        self.commit_snapshot(snapshot, message)
    }

    /// 抓取显式给定的传输端点，只更新 tracking ref。
    /// 本地分支和工作区一概不动。
    pub fn fetch(&self) -> Result<FetchResponse> {
        let request = FetchRequest {
            local_bare: self.config.bare_path.clone(),
            branch: self.config.branch.clone(),
        };
        let response = self
            .transport
            .fetch(&request)
            .context("block fetch failed")?;
        let repo = self.open_bare()?;
        if let Some(head) = response.remote_head {
            repo.find_commit(head)
                .context("transport returned a remote head absent from local object store")?;
            self.ensure_block_identity(&repo, head)?;
            set_tracking_ref(&repo, &self.config.tracking_ref(), head)?;
        }
        Ok(response)
    }

    pub fn push(&self) -> Result<PushOutcome> {
        let repo = self.open_bare()?;
        let head = ref_target(&repo, &self.config.branch_ref())?
            .context("cannot push a device with no branch head")?;
        let expected = ref_target(&repo, &self.config.tracking_ref())?;
        let request = PushRequest {
            local_bare: self.config.bare_path.clone(),
            branch: self.config.branch.clone(),
            expected_remote_head: expected,
            new_head: head,
        };
        let response = self.transport.push(&request).context("block push failed")?;
        ensure!(
            response.remote_head == head,
            "transport acknowledged a different remote head"
        );
        set_tracking_ref(&repo, &self.config.tracking_ref(), head)?;
        if response.already_up_to_date {
            Ok(PushOutcome::AlreadyUpToDate { head })
        } else {
            Ok(PushOutcome::Pushed { head })
        }
    }

    /// 抓取并合并远端分支。出现冲突时，本地分支和工作区都保持
    /// 合并前原样。
    pub fn pull(&self) -> Result<PullOutcome> {
        let before_repo = self.open_bare()?;
        let ours = ref_target(&before_repo, &self.config.branch_ref())?;
        let fetch = self.fetch()?;
        let repo = self.open_bare()?;
        let theirs = fetch.remote_head;
        let Some(theirs) = theirs else {
            return Ok(PullOutcome::NoRemote);
        };
        let Some(ours) = ours else {
            let old_files = BTreeMap::new();
            let next_files = commit_files(&repo, theirs)?;
            let head = self.finish_local_commit(
                &repo,
                None,
                &old_files,
                next_files,
                theirs,
                "fast-forward block",
            )?;
            return Ok(PullOutcome::FastForward { head });
        };
        if ours == theirs {
            let files = commit_files(&repo, ours)?;
            // 就算 pull 已经是最新，也要校验物化的跟踪内容；
            // 否则调用方可能拿到成功，而工作区其实早已偏离已提交快照。
            let _ = WorkspaceTransaction::prepare(&self.config.workspace_path, &files, &files)?;
            return Ok(PullOutcome::UpToDate { head: ours });
        }

        self.ensure_block_identity(&repo, theirs)?;
        if repo.graph_descendant_of(theirs, ours)? {
            let old_files = commit_files(&repo, ours)?;
            let next_files = commit_files(&repo, theirs)?;
            let head = self.finish_local_commit(
                &repo,
                Some(ours),
                &old_files,
                next_files,
                theirs,
                "fast-forward block",
            )?;
            return Ok(PullOutcome::FastForward { head });
        }
        if repo.graph_descendant_of(ours, theirs)? {
            return Ok(PullOutcome::LocalAhead {
                head: ours,
                remote_head: theirs,
            });
        }

        let base = repo.merge_base(ours, theirs).ok();
        let base_files = match base {
            Some(base) => commit_files(&repo, base)?,
            None => BTreeMap::new(),
        };
        let ours_files = commit_files(&repo, ours)?;
        let theirs_files = commit_files(&repo, theirs)?;
        let (merged, conflicts) = merge_file_maps(
            &base_files,
            &ours_files,
            &theirs_files,
            &self.config.block_id,
        )?;
        if !conflicts.is_empty() {
            let proposed = snapshot_from_files(&merged)?;
            return Ok(PullOutcome::Conflict {
                report: ConflictReport {
                    base_head: base,
                    ours_head: ours,
                    theirs_head: theirs,
                    conflicts,
                    proposed,
                },
            });
        }

        let commit = self.create_commit_object(
            &repo,
            &merged,
            Some(ours),
            &[theirs],
            &format!("merge block branch {}", self.config.branch),
        )?;
        let head = self.finish_local_commit(
            &repo,
            Some(ours),
            &ours_files,
            merged,
            commit,
            "merge block",
        )?;
        Ok(PullOutcome::Merged { head })
    }

    pub fn pull_and_merge(&self) -> Result<PullOutcome> {
        self.pull()
    }

    /// 每个冲突路径都有调用方给的决定之后，提交这条先前上报的冲突。
    pub fn commit_resolution(
        &self,
        resolution: MergeResolution,
        message: impl AsRef<str>,
    ) -> Result<Oid> {
        ensure!(
            resolution.unresolved.is_empty(),
            "unresolved conflict paths: {}",
            resolution
                .unresolved
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        let message = message.as_ref();
        ensure!(
            !message.as_bytes().contains(&0),
            "commit message contains NUL"
        );
        let repo = self.open_bare()?;
        let ours = ref_target(&repo, &self.config.branch_ref())?;
        ensure!(
            ours == Some(resolution.ours_head),
            "local branch changed since conflict report; resolution was not committed"
        );
        let tracking = ref_target(&repo, &self.config.tracking_ref())?;
        ensure!(
            tracking == Some(resolution.theirs_head),
            "remote tracking branch changed since conflict report; resolution was not committed"
        );

        let old_files = commit_files(&repo, resolution.ours_head)?;
        let mut next_files = snapshot_to_map(&resolution.proposed)?;
        next_files.insert(
            BLOCK_ID_COMMENT_PATH.to_string(),
            block_id_comment_bytes(&self.config.block_id),
        );
        let commit = self.create_commit_object(
            &repo,
            &next_files,
            Some(resolution.ours_head),
            &[resolution.theirs_head],
            message,
        )?;
        self.finish_local_commit(
            &repo,
            Some(resolution.ours_head),
            &old_files,
            next_files,
            commit,
            message,
        )
    }

    pub fn resolve_and_commit(
        &self,
        resolution: MergeResolution,
        message: impl AsRef<str>,
    ) -> Result<Oid> {
        self.commit_resolution(resolution, message)
    }

    pub(super) fn open_bare(&self) -> Result<Repository> {
        Repository::open_bare(&self.config.bare_path).with_context(|| {
            format!(
                "open device bare repository {}",
                self.config.bare_path.display()
            )
        })
    }

    pub(super) fn new<T>(config: DeviceConfig, transport: Arc<T>) -> Self
    where
        T: Transport + 'static,
    {
        Self {
            config,
            transport,
            last_workspace_warning: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) fn record_workspace_warning(&self, warning: WorkspaceCleanupWarning) {
        match self.last_workspace_warning.lock() {
            Ok(mut slot) => *slot = Some(warning),
            Err(poisoned) => *poisoned.into_inner() = Some(warning),
        }
    }

    pub(super) fn clear_workspace_warning(&self) {
        match self.last_workspace_warning.lock() {
            Ok(mut slot) => *slot = None,
            Err(poisoned) => *poisoned.into_inner() = None,
        }
    }

    pub(super) fn snapshot_at(&self, repo: &Repository, head: Oid) -> Result<Snapshot> {
        self.ensure_block_identity(repo, head)?;
        let files = commit_files(repo, head)?;
        snapshot_from_files(&files)
    }

    pub(super) fn ensure_block_identity(&self, repo: &Repository, head: Oid) -> Result<()> {
        let files = commit_files(repo, head)?;
        let persisted = parse_block_id(
            files
                .get(BLOCK_ID_COMMENT_PATH)
                .context("commit has no Mochi block id comment file")?,
        )?;
        ensure!(
            persisted == self.config.block_id,
            "block id mismatch: expected the complete configured id, got a different block"
        );
        Ok(())
    }

    pub(super) fn apply_snapshot(
        &self,
        old_files: &BTreeMap<String, Vec<u8>>,
        snapshot: &Snapshot,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        let changes = snapshot.changes()?;
        let mut next = old_files.clone();
        for (path, bytes) in changes {
            match bytes {
                Some(bytes) => {
                    next.insert(path, bytes);
                }
                None => {
                    next.remove(&path);
                }
            }
        }
        next.insert(
            BLOCK_ID_COMMENT_PATH.to_string(),
            block_id_comment_bytes(&self.config.block_id),
        );
        validate_file_map(&next, true)?;
        Ok(next)
    }

    pub(super) fn create_commit_object(
        &self,
        repo: &Repository,
        files: &BTreeMap<String, Vec<u8>>,
        parent: Option<Oid>,
        extra_parents: &[Oid],
        message: &str,
    ) -> Result<Oid> {
        let tree_id = map_to_tree(repo, files)?;
        let tree = repo.find_tree(tree_id)?;
        let mut parent_commits = Vec::new();
        if let Some(parent) = parent {
            parent_commits.push(repo.find_commit(parent)?);
        }
        for parent in extra_parents {
            parent_commits.push(repo.find_commit(*parent)?);
        }
        let parent_refs: Vec<&Commit<'_>> = parent_commits.iter().collect();
        let signature = Signature::now(&self.config.author_name, &self.config.author_email)?;
        Ok(repo.commit(None, &signature, &signature, message, &tree, &parent_refs)?)
    }

    pub(super) fn finish_local_commit(
        &self,
        repo: &Repository,
        expected_head: Option<Oid>,
        old_files: &BTreeMap<String, Vec<u8>>,
        next_files: BTreeMap<String, Vec<u8>>,
        new_head: Oid,
        message: &str,
    ) -> Result<Oid> {
        self.clear_workspace_warning();
        let mut workspace =
            WorkspaceTransaction::prepare(&self.config.workspace_path, old_files, &next_files)?;
        workspace.apply()?;
        if let Err(error) = update_ref_if_current(
            repo,
            &self.config.branch_ref(),
            expected_head,
            new_head,
            message,
        ) {
            return match workspace.rollback() {
                Ok(()) => Err(error),
                Err(recovery) => Err(error.context(format!(
                    "ref update failed and workspace rollback also failed: {recovery}"
                ))),
            };
        }
        if let Some(warning) = workspace.finish() {
            // 分支 ref 已是提交点。保留恢复产物，上报「已提交但带警告」
            // 的状态，而不是返回一个可能被误读成回滚的错误。
            self.record_workspace_warning(warning);
        }
        Ok(new_head)
    }
}
