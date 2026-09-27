//! 定义同步传输请求、响应和传输能力。
use super::*;

/// 本次构建中传输真正具备的能力。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransportKind {
    LocalBare,
    Ssh,
    Http,
    GitCli,
    Unconfigured,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransportCapabilities {
    pub kind: TransportKind,
    pub implemented: bool,
    pub network_enabled: bool,
    pub note: String,
}

/// 传给 fetch 适配器的输入。
#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub local_bare: PathBuf,
    pub branch: String,
}

#[derive(Clone, Debug)]
pub struct FetchResponse {
    /// fetch 之后的远端分支。None 表示远端分支还没建立。
    pub remote_head: Option<Oid>,
}

/// 传给 push 适配器的输入。
#[derive(Clone, Debug)]
pub struct PushRequest {
    pub local_bare: PathBuf,
    pub branch: String,
    /// 调用方上次 fetch 时观察到的远端分支。当前值一旦不同，传输
    /// 必须拒绝；这就是「不许强推」的 compare-and-swap 边界。
    pub expected_remote_head: Option<Oid>,
    pub new_head: Oid,
}

#[derive(Clone, Debug)]
pub struct PushResponse {
    pub remote_head: Oid,
    pub already_up_to_date: bool,
}

/// 本地 bare 到 bare 的对象传输。它不用 Git remote、凭据、
/// 进程当前所在的仓库，也不碰网络。
#[derive(Clone, Debug)]
pub struct LocalTransport {
    pub(super) remote_bare: PathBuf,
}

impl LocalTransport {
    pub fn new(remote_bare: impl Into<PathBuf>) -> Result<Self> {
        let remote_bare = remote_bare.into();
        Repository::open_bare(&remote_bare).with_context(|| {
            format!("open explicit local bare remote {}", remote_bare.display())
        })?;
        Ok(Self { remote_bare })
    }

    pub fn remote_bare(&self) -> &Path {
        &self.remote_bare
    }
}

impl Transport for LocalTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            kind: TransportKind::LocalBare,
            implemented: true,
            network_enabled: false,
            note: "Copies reachable Git objects between explicit bare repositories".into(),
        }
    }

    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse> {
        validate_branch(&request.branch)?;
        let local = Repository::open_bare(&request.local_bare)
            .with_context(|| format!("open local bare {}", request.local_bare.display()))?;
        let remote = Repository::open_bare(&self.remote_bare)
            .with_context(|| format!("open local remote {}", self.remote_bare.display()))?;
        ensure_distinct_repositories(&request.local_bare, &self.remote_bare)?;

        let branch_ref = format!("refs/heads/{}", request.branch);
        let remote_head = ref_target(&remote, &branch_ref)?;
        if let Some(head) = remote_head {
            copy_reachable_objects(&remote, &local, head)?;
            // 上报成功之前先校验对象图。自定义文件系统的失败
            // 不能让一个 tracking ref 假装有效。
            local
                .find_commit(head)
                .context("fetched branch head is not a commit")?;
        }
        Ok(FetchResponse { remote_head })
    }

    fn push(&self, request: &PushRequest) -> Result<PushResponse> {
        validate_branch(&request.branch)?;
        let local = Repository::open_bare(&request.local_bare)
            .with_context(|| format!("open local bare {}", request.local_bare.display()))?;
        let remote = Repository::open_bare(&self.remote_bare)
            .with_context(|| format!("open local remote {}", self.remote_bare.display()))?;
        ensure_distinct_repositories(&request.local_bare, &self.remote_bare)?;

        let branch_ref = format!("refs/heads/{}", request.branch);
        let local_head = ref_target(&local, &branch_ref)?;
        ensure!(
            local_head == Some(request.new_head),
            "push head does not match the local branch"
        );

        let current_remote = ref_target(&remote, &branch_ref)?;
        if current_remote == Some(request.new_head) {
            return Ok(PushResponse {
                remote_head: request.new_head,
                already_up_to_date: true,
            });
        }
        ensure!(
            current_remote == request.expected_remote_head,
            "remote branch changed since the last fetch; force push is disabled"
        );

        local
            .find_commit(request.new_head)
            .context("local push head is not a commit")?;
        copy_reachable_objects(&local, &remote, request.new_head)?;

        // 事务在 compare 与 update 之间一直持有 ref 锁。并发写入方
        // 赢得竞争时，对象可能已经复制过去了，但远端分支不会被覆盖，
        // 历史依旧完好。
        let mut transaction = remote.transaction()?;
        transaction.lock_ref(&branch_ref)?;
        let locked_remote = ref_target(&remote, &branch_ref)?;
        ensure!(
            locked_remote == request.expected_remote_head,
            "remote branch changed during push; no ref was overwritten"
        );
        if let Some(old) = locked_remote {
            ensure!(
                remote.graph_descendant_of(request.new_head, old)?,
                "push is not a fast-forward; force push is disabled"
            );
        }
        let signature = transport_signature()?;
        transaction.set_target(
            &branch_ref,
            request.new_head,
            Some(&signature),
            "mochi blocks push",
        )?;
        transaction.commit()?;

        Ok(PushResponse {
            remote_head: request.new_head,
            already_up_to_date: false,
        })
    }
}

/// 显式的能力占位实现，供想描述「适配器不可用」的调用方使用，
/// 且绝不触发任何实际操作。本模块真正实现了进程适配器
/// GitCliTransport。
///
/// 它总是返回错误，不开 socket、不启动进程。这是有意的：
/// 本 crate 的 git2 依赖关闭了网络 feature，这个适配器绝不能
/// 悄悄使用凭据或真实远端。
#[derive(Clone, Debug)]
pub struct UnavailableNetworkTransport {
    pub config: NetworkTransportConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkTransportConfig {
    pub kind: TransportKind,
    pub endpoint: String,
    pub executable: Option<PathBuf>,
}

impl NetworkTransportConfig {
    pub fn ssh(endpoint: impl Into<String>) -> Self {
        Self {
            kind: TransportKind::Ssh,
            endpoint: endpoint.into(),
            executable: None,
        }
    }

    pub fn http(endpoint: impl Into<String>) -> Self {
        Self {
            kind: TransportKind::Http,
            endpoint: endpoint.into(),
            executable: None,
        }
    }

    pub fn git_cli(endpoint: impl Into<String>, executable: impl Into<PathBuf>) -> Self {
        Self {
            kind: TransportKind::GitCli,
            endpoint: endpoint.into(),
            executable: Some(executable.into()),
        }
    }
}

impl UnavailableNetworkTransport {
    pub fn new(config: NetworkTransportConfig) -> Result<Self> {
        ensure!(
            matches!(
                config.kind,
                TransportKind::Ssh | TransportKind::Http | TransportKind::GitCli
            ),
            "network placeholder requires SSH, HTTP, or Git CLI kind"
        );
        Ok(Self { config })
    }
}

impl Transport for UnavailableNetworkTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            kind: self.config.kind.clone(),
            implemented: false,
            network_enabled: false,
            note: "Interface/configuration only; network adapter is intentionally unavailable in this build"
                .into(),
        }
    }

    fn fetch(&self, _request: &FetchRequest) -> Result<FetchResponse> {
        bail!(
            "{} transport is not implemented; no network connection was attempted",
            transport_kind_name(&self.config.kind)
        )
    }

    fn push(&self, _request: &PushRequest) -> Result<PushResponse> {
        bail!(
            "{} transport is not implemented; no network connection was attempted",
            transport_kind_name(&self.config.kind)
        )
    }
}

/// SSH 端点由 Git CLI 处理，沿用同一套安全边界。
pub type SshTransport = GitCliTransport;

/// HTTP(S) 端点由 Git CLI 处理，沿用同一套安全边界。
pub type HttpTransport = GitCliTransport;

fn transport_kind_name(kind: &TransportKind) -> &'static str {
    match kind {
        TransportKind::Ssh => "SSH",
        TransportKind::Http => "HTTP",
        TransportKind::GitCli => "Git CLI",
        TransportKind::LocalBare => "local",
        TransportKind::Unconfigured => "unconfigured",
    }
}

pub(super) struct NoTransport;

impl Transport for NoTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            kind: TransportKind::Unconfigured,
            implemented: false,
            network_enabled: false,
            note: "No transport configured".into(),
        }
    }

    fn fetch(&self, _request: &FetchRequest) -> Result<FetchResponse> {
        bail!("no block transport configured")
    }

    fn push(&self, _request: &PushRequest) -> Result<PushResponse> {
        bail!("no block transport configured")
    }
}
