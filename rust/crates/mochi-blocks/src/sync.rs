//! 只同步调用方明确列出的路径和内容，不扫描或自动上传工作区。
//! LocalTransport 只复制本地对象；网络传输须显式选择 GitCliTransport。

mod configuration;
mod git_command;
mod git_objects;
mod git_transport;
mod merge;
mod repository;
mod snapshot;
mod transport;
mod workspace_transaction;

use configuration::absolute_for_compare;
pub use configuration::DeviceConfig;
use git_command::{
    configure_git_command, endpoint_is_network, parse_ls_remote_head, validate_git_endpoint,
};
use git_objects::{
    block_id_comment_bytes, commit_files, copy_reachable_objects, ensure_distinct_repositories,
    ensure_workspace_root, init_new_bare, map_to_tree, parse_block_id, ref_target,
    set_tracking_ref, transport_signature, update_ref_if_current, validate_branch, CommittedFiles,
    MergeMaps,
};
pub use git_transport::{GitCliTransport, GitCliTransportConfig};
use merge::merge_file_maps;
pub use merge::{
    BlockRepository, ConflictFile, ConflictReport, DeviceRepo, DeviceRepository, MergeResolution,
    PullOutcome, PullResult, PushOutcome, PushResult,
};
pub use repository::WorkspaceCleanupWarning;
use snapshot::{snapshot_from_files, snapshot_to_map, upsert_snapshot_file, validate_file_map};
pub use snapshot::{validate_snapshot_path, Snapshot, SnapshotFile};
use transport::NoTransport;
pub use transport::{
    FetchRequest, FetchResponse, HttpTransport, LocalTransport, NetworkTransportConfig,
    PushRequest, PushResponse, SshTransport, TransportCapabilities, TransportKind,
    UnavailableNetworkTransport,
};
#[cfg(test)]
use workspace_transaction::PendingWorkspaceWrite;
use workspace_transaction::WorkspaceTransaction;

use anyhow::{bail, ensure, Context, Result};
use git2::{Commit, ErrorCode, ObjectType, Oid, Repository, Signature, Tree};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 每个块提交里都会生成的元数据文件。
///
/// 它是一个纯文本注释头，后跟完整的块 ID。ID 不做任何截短、归一，
/// 也不只放在 commit subject 里。这条路径为本模块保留，
/// 不接受用户文件占用。
pub const BLOCK_ID_COMMENT_PATH: &str = ".mochi-block-id";

const BLOCK_ID_COMMENT_HEADER: &[u8] = b"# mochi block id\n";
const TRACKING_REMOTE: &str = "mochi";
const DEFAULT_BRANCH: &str = "main";
const DEFAULT_AUTHOR_NAME: &str = "Mochi Blocks";
const DEFAULT_AUTHOR_EMAIL: &str = "blocks@localhost";

/// 可替换的对象传输。
///
/// fetch 必须把请求的远端 commit 以及从它可达的全部对象复制进
/// request.local_bare，然后返回远端分支头。push 必须先复制对象，
/// 通过 expected-head 和 fast-forward 校验之后才更新远端分支。
/// 适配器绝不强推远端分支。设备只在收到成功响应之后
/// 才更新自己的本地 tracking ref。
pub trait Transport: Send + Sync {
    fn capabilities(&self) -> TransportCapabilities;
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse>;
    fn push(&self, request: &PushRequest) -> Result<PushResponse>;
}

#[cfg(test)]
mod tests;
