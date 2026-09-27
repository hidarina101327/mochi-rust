//! 通过 Git 命令行执行同步传输，并配置超时和可执行文件。
use super::*;

/// Git CLI 适配器的配置。
///
/// 端点作为单独一个参数传给 git。可以是显式的本地 bare 路径、
/// file URL，或不含内嵌 userinfo 的 SSH/HTTP(S) URL。
/// timeout_ms 限定每个 git 进程的运行时长。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitCliTransportConfig {
    pub endpoint: String,
    pub executable: PathBuf,
    pub timeout_ms: u64,
}

impl GitCliTransportConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            executable: PathBuf::from("git"),
            timeout_ms: 30_000,
        }
    }

    pub fn local(path: impl AsRef<Path>) -> Self {
        Self::new(path.as_ref().to_string_lossy().into_owned())
    }

    pub fn with_executable(mut self, executable: impl Into<PathBuf>) -> Self {
        self.executable = executable.into();
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }
}

/// 真实的、非交互的 Git CLI 传输。
///
/// 刻意做成同步接口，这条边界才好推理；服务 UI 请求的调用方
/// 应放到自己的工作线程或执行器上运行。本适配器不通过命令行解释器执行命令，
/// 命令输出一律丢弃，只有有界的 ls-remote ref 查询除外。查询结果
/// 在本地解析，绝不进错误消息。超时会回收 Git 进程，Windows 上
/// 还会尝试终止其进程树；需要更强跨平台进程边界的宿主，应把
/// 需要更强跨平台进程隔离能力的宿主，应在独立任务环境中运行此适配器。
#[derive(Clone, Debug)]
pub struct GitCliTransport {
    pub(super) config: GitCliTransportConfig,
    pub(super) timeout: Duration,
    pub(super) instance: u64,
}

static GIT_CLI_INSTANCE: AtomicU64 = AtomicU64::new(1);

impl GitCliTransport {
    pub fn new(endpoint: impl Into<String>) -> Result<Self> {
        Self::from_config(GitCliTransportConfig::new(endpoint))
    }

    pub fn local(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_config(GitCliTransportConfig::local(path))
    }

    pub fn from_config(config: GitCliTransportConfig) -> Result<Self> {
        validate_git_endpoint(&config.endpoint)?;
        ensure!(
            config.timeout_ms > 0 && config.timeout_ms <= 10 * 60 * 1000,
            "Git CLI timeout must be between 1 ms and 10 minutes"
        );
        ensure!(
            !config.executable.as_os_str().is_empty(),
            "Git CLI executable must not be empty"
        );
        Ok(Self {
            timeout: Duration::from_millis(config.timeout_ms),
            config,
            instance: GIT_CLI_INSTANCE.fetch_add(1, Ordering::Relaxed),
        })
    }

    pub fn config(&self) -> &GitCliTransportConfig {
        &self.config
    }

    pub(super) fn remote_head(&self, local_bare: &Path, branch: &str) -> Result<Option<Oid>> {
        let branch_ref = format!("refs/heads/{branch}");
        let args = vec![
            OsArg::literal("--no-pager"),
            OsArg::literal("--no-optional-locks"),
            OsArg::literal("--git-dir"),
            OsArg::path(local_bare),
            OsArg::literal("ls-remote"),
            OsArg::literal("--refs"),
            OsArg::endpoint(&self.config.endpoint),
            OsArg::literal(&branch_ref),
        ];
        let output = self.run_git(&args, true)?;
        parse_ls_remote_head(&output, &branch_ref)
    }

    pub(super) fn fetch_objects(&self, local_bare: &Path, branch: &str) -> Result<()> {
        let branch_ref = format!("refs/heads/{branch}");
        let tracking_ref = format!("refs/remotes/{TRACKING_REMOTE}/{branch}");
        let refspec = format!("+{branch_ref}:{tracking_ref}");
        let args = vec![
            OsArg::literal("--no-pager"),
            OsArg::literal("--no-optional-locks"),
            OsArg::literal("--git-dir"),
            OsArg::path(local_bare),
            OsArg::literal("fetch"),
            OsArg::literal("--no-tags"),
            OsArg::endpoint(&self.config.endpoint),
            OsArg::literal(&refspec),
        ];
        self.run_git(&args, false).map(|_| ())
    }

    pub(super) fn push_objects(&self, local_bare: &Path, branch: &str) -> Result<()> {
        let branch_ref = format!("refs/heads/{branch}");
        let refspec = format!("{branch_ref}:{branch_ref}");
        let args = vec![
            OsArg::literal("--no-pager"),
            OsArg::literal("--no-optional-locks"),
            OsArg::literal("--git-dir"),
            OsArg::path(local_bare),
            OsArg::literal("push"),
            OsArg::literal("--no-tags"),
            OsArg::literal("--no-verify"),
            OsArg::literal("--porcelain"),
            OsArg::endpoint(&self.config.endpoint),
            OsArg::literal(&refspec),
        ];
        self.run_git(&args, false).map(|_| ())
    }

    pub(super) fn run_git(&self, args: &[OsArg<'_>], capture_stdout: bool) -> Result<Vec<u8>> {
        let mut command = Command::new(&self.config.executable);
        for arg in args {
            arg.add_to(&mut command);
        }
        configure_git_command(&mut command, self.instance);
        let output_file = if capture_stdout {
            let (path, file) = create_git_output_file(self.instance)?;
            command.stdout(Stdio::from(file));
            Some(GitOutputFile { path })
        } else {
            command.stdout(Stdio::null());
            None
        };
        command.stderr(Stdio::null()).stdin(Stdio::null());

        let operation = args
            .iter()
            .find_map(OsArg::display_operation)
            .unwrap_or("git");
        let mut child = command
            .spawn()
            .with_context(|| format!("spawn Git CLI for {operation}"))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    terminate_git_child(&mut child);
                    return Err(error).with_context(|| format!("poll Git CLI {operation}"));
                }
            };
            if let Some(status) = status {
                if !status.success() {
                    bail!("Git CLI {operation} failed; output was intentionally suppressed");
                }
                if let Some(output_file) = output_file.as_ref() {
                    let metadata = fs::metadata(&output_file.path)
                        .with_context(|| format!("inspect Git CLI {operation} output"))?;
                    ensure!(
                        metadata.len() <= MAX_GIT_QUERY_OUTPUT,
                        "Git CLI {operation} output exceeded the bounded query limit"
                    );
                    return fs::read(&output_file.path)
                        .with_context(|| format!("read Git CLI {operation} output"));
                }
                return Ok(Vec::new());
            }
            if Instant::now() >= deadline {
                terminate_git_child(&mut child);
                bail!("Git CLI {operation} timed out; child process cleanup was attempted");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

const MAX_GIT_QUERY_OUTPUT: u64 = 8 * 1024;

pub(super) struct GitOutputFile {
    pub(super) path: PathBuf,
}

impl Drop for GitOutputFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_git_output_file(instance: u64) -> Result<(PathBuf, std::fs::File)> {
    let directory = std::env::temp_dir();
    for attempt in 0..100u32 {
        let path = directory.join(format!(
            "mochi-blocks-gitcli-output-{}-{instance}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "create temporary Git CLI output file in {}",
                        directory.display()
                    )
                })
            }
        }
    }
    bail!(
        "could not allocate a temporary Git CLI output file in {}",
        directory.display()
    )
}

fn terminate_git_child(child: &mut Child) {
    #[cfg(windows)]
    {
        // Git 可能在我们启动的进程之下留下 git-remote-http.exe 或 ssh.exe。
        // 先用系统的 taskkill 可执行文件请求按树清理，再在下面直接
        // kill/reap 父进程。这是尽力而为：无论如何适配器都会上报超时，
        // 宿主需要硬进程边界时可以再加 Job Object/沙箱。
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            let taskkill = PathBuf::from(system_root)
                .join("System32")
                .join("taskkill.exe");
            let pid = child.id().to_string();
            let _ = Command::new(taskkill)
                .arg("/PID")
                .arg(&pid)
                .args(["/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Transport for GitCliTransport {
    fn capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            kind: TransportKind::GitCli,
            implemented: true,
            network_enabled: endpoint_is_network(&self.config.endpoint),
            note: "Non-interactive git fetch/push with an explicit endpoint, no credential URLs, bounded output, and best-effort process timeout cleanup"
                .into(),
        }
    }

    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse> {
        validate_branch(&request.branch)?;
        validate_git_endpoint(&self.config.endpoint)?;
        Repository::open_bare(&request.local_bare)
            .with_context(|| format!("open local bare {}", request.local_bare.display()))?;
        let remote_head = self.remote_head(&request.local_bare, &request.branch)?;
        if let Some(head) = remote_head {
            self.fetch_objects(&request.local_bare, &request.branch)?;
            let local = Repository::open_bare(&request.local_bare)?;
            local
                .find_commit(head)
                .context("Git CLI fetch did not install the advertised commit")?;
        }
        Ok(FetchResponse { remote_head })
    }

    fn push(&self, request: &PushRequest) -> Result<PushResponse> {
        validate_branch(&request.branch)?;
        validate_git_endpoint(&self.config.endpoint)?;
        let local = Repository::open_bare(&request.local_bare)
            .with_context(|| format!("open local bare {}", request.local_bare.display()))?;
        let branch_ref = format!("refs/heads/{}", request.branch);
        ensure!(
            ref_target(&local, &branch_ref)? == Some(request.new_head),
            "push head does not match the local branch"
        );
        let current_remote = self.remote_head(&request.local_bare, &request.branch)?;
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
        self.push_objects(&request.local_bare, &request.branch)?;
        let after = self.remote_head(&request.local_bare, &request.branch)?;
        ensure!(
            after == Some(request.new_head),
            "Git CLI push completed without installing the requested remote head"
        );
        Ok(PushResponse {
            remote_head: request.new_head,
            already_up_to_date: false,
        })
    }
}

pub(super) enum OsArg<'a> {
    Literal(&'a str),
    Path(&'a Path),
    Endpoint(&'a str),
}

impl<'a> OsArg<'a> {
    pub(super) fn literal(value: &'a str) -> Self {
        Self::Literal(value)
    }

    pub(super) fn path(value: &'a Path) -> Self {
        Self::Path(value)
    }

    pub(super) fn endpoint(value: &'a str) -> Self {
        Self::Endpoint(value)
    }

    pub(super) fn add_to(&self, command: &mut Command) {
        match self {
            Self::Literal(value) | Self::Endpoint(value) => {
                command.arg(value);
            }
            Self::Path(value) => {
                command.arg(value);
            }
        }
    }

    pub(super) fn display_operation(&self) -> Option<&'static str> {
        match self {
            Self::Literal("fetch") => Some("fetch"),
            Self::Literal("push") => Some("push"),
            Self::Literal("ls-remote") => Some("ls-remote"),
            _ => None,
        }
    }
}
