//! 调用前必须通过 tools::shell_tools 的权限和审批检查；本层只执行命令。
//! 输出最多保留 20 万字符。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_HTTP_TIMEOUT_MS: u64 = 30_000;
const MAX_OUTPUT_CHARS: usize = 200_000;

/// 轮询子进程退出状态的间隔。短到用户感觉不出延迟，又不至于空转烧 CPU。
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellRunResult {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ShellRunResult {
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            error: Some(error.into()),
        }
    }
}

/// 按字符（不是字节）截断，避免把中文切成半个。
fn truncate(value: String) -> String {
    let total = value.chars().count();
    if total <= MAX_OUTPUT_CHARS {
        return value;
    }
    let head: String = value.chars().take(MAX_OUTPUT_CHARS).collect();
    format!("{head}\n...（输出过长已截断，共 {total} 字符）")
}

/// 执行一条命令行命令。Windows 走 `powershell.exe -NoProfile -NonInteractive -Command`，
/// 其余平台走 `/bin/sh -c`——与 Electron 版同一个 shell，否则同一条命令两版行为不同。
pub fn run_command(command: &str, cwd: &Path, timeout_ms: Option<u64>) -> ShellRunResult {
    run_command_inner(command, cwd, timeout_ms, None)
}
pub fn run_command_cancellable(
    command: &str,
    cwd: &Path,
    timeout_ms: Option<u64>,
    cancel: &std::sync::atomic::AtomicBool,
) -> ShellRunResult {
    run_command_inner(command, cwd, timeout_ms, Some(cancel))
}
fn run_command_inner(
    command: &str,
    cwd: &Path,
    timeout_ms: Option<u64>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> ShellRunResult {
    let command = command.trim();
    if command.is_empty() {
        return ShellRunResult::failed("命令为空");
    }
    let mut builder = if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        c
    } else {
        let mut c = Command::new("/bin/sh");
        c.args(["-c", command]);
        c
    };
    builder.current_dir(cwd);
    run_process(&mut builder, timeout_ms, cancel, None)
}

/// 直接以可执行文件/参数运行，脚本源码绝不拼进 shell 命令行。
/// 有界的管道排水、作业树清理、超时与取消与 shell_run 共用一套。
pub(crate) fn run_process(
    builder: &mut Command,
    timeout_ms: Option<u64>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    _job_memory_limit: Option<usize>,
) -> ShellRunResult {
    let is_cancelled = || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed));
    if is_cancelled() {
        return ShellRunResult::failed("命令已取消");
    }
    #[cfg(windows)]
    let job = match ProcessJob::new(_job_memory_limit) {
        Ok(job) => job,
        Err(e) => return ShellRunResult::failed(e.to_string()),
    };

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        builder.creation_flags(0x08000000);
    }
    let mut child = match builder
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return ShellRunResult::failed(e.to_string()),
    };
    #[cfg(windows)]
    if let Err(e) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return ShellRunResult::failed(format!("无法建立命令进程组：{e}"));
    }

    // 必须**边跑边收**：管道缓冲区填满后子进程会阻塞在写上，
    // 等进程退出再读就永远等不到（经典死锁）。
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());

    let deadline =
        Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS));
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline || is_cancelled() {
                    cancelled = is_cancelled();
                    timed_out = !cancelled;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => return ShellRunResult::failed(e.to_string()),
        }
    };
    #[cfg(windows)]
    drop(job); // 关闭作业会清理命令遗留的子进程并关闭它们持有的输出管道。

    let exit_code = status.and_then(|s| s.code());
    ShellRunResult {
        ok: !timed_out && !cancelled && exit_code == Some(0),
        exit_code,
        stdout: truncate(stdout.join()),
        stderr: truncate(stderr.join()),
        timed_out,
        error: if cancelled {
            Some("命令已取消".into())
        } else {
            timed_out.then(|| "命令执行超时已被终止".to_owned())
        },
    }
}

#[cfg(windows)]
struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ProcessJob {
    fn new(memory_limit: Option<usize>) -> std::io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let job = Self(h);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(bytes) = memory_limit {
                limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.JobMemoryLimit = bytes;
            }
            if SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(job)
        }
    }
    fn assign(&self, child: &std::process::Child) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        let ok = unsafe {
            windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(
                self.0,
                child.as_raw_handle(),
            )
        };
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// 后台线程把一路管道读干，结束后经 channel 交回。
struct Drain {
    rx: Option<mpsc::Receiver<()>>,
    buffer: std::sync::Arc<std::sync::Mutex<(Vec<u8>, bool)>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drain {
    fn join(self) -> String {
        let completed = self
            .rx
            .map(|rx| rx.recv_timeout(Duration::from_secs(2)).is_ok())
            .unwrap_or(true);
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let (mut bytes, truncated) = self
            .buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if truncated {
            if let Err(error) = std::str::from_utf8(&bytes) {
                if error.error_len().is_none() && error.valid_up_to() + 4 >= bytes.len() {
                    bytes.truncate(error.valid_up_to());
                }
            }
        }
        let mut text = decode_console_output(&bytes);
        if truncated {
            text.push_str("\n...（输出过长已截断）");
        }
        if !completed {
            text.push_str("\n...（输出管道未关闭，已停止等待）");
        }
        text
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> Drain {
    let buffer = std::sync::Arc::new(std::sync::Mutex::new((Vec::new(), false)));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let Some(mut pipe) = pipe else {
        return Drain {
            rx: None,
            buffer,
            stop,
        };
    };
    let (tx, rx) = mpsc::channel();
    let output = buffer.clone();
    let cancelled = stop.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            let n = match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut out = output.lock().unwrap_or_else(|e| e.into_inner());
            let keep = n.min((MAX_OUTPUT_CHARS * 4).saturating_sub(out.0.len()));
            out.0.extend_from_slice(&chunk[..keep]);
            out.1 |= keep < n;
        }
        let _ = tx.send(());
    });
    Drain {
        rx: Some(rx),
        buffer,
        stop,
    }
}

// 旧版 PowerShell 可能输出系统代码页；UTF-8 校验失败后再尝试 ANSI，不能先有损解码。
fn decode_console_output(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => decode_ansi(bytes),
    }
}

#[cfg(windows)]
fn decode_ansi(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};

    if bytes.is_empty() {
        return String::new();
    }
    let needed = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        )
    };
    if needed <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; needed as usize];
    let written = unsafe {
        MultiByteToWideChar(
            CP_ACP,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            needed,
        )
    };
    if written <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    String::from_utf16_lossy(&wide[..written as usize])
}

#[cfg(not(windows))]
fn decode_ansi(bytes: &[u8]) -> String {
    // 非 Windows 上区域编码就是 UTF-8，走到这儿说明输出本身就是坏的
    String::from_utf8_lossy(bytes).into_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpRequestResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HttpRequestResult {
    fn failed(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            status: None,
            status_text: None,
            headers: None,
            body: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HttpRequestOptions {
    pub url: String,
    pub method: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout_ms: Option<u64>,
    /// 对可信、固定的端点使用进程/系统代理及其解析器。AI 给出的任意
    /// URL 必须保持关闭，公网地址解析器才能继续守住 SSRF 边界。
    pub use_system_proxy: bool,
}

/// 包一层，好让两条分支（带 body / 不带 body）的错误类型统一。
struct RequestBuildError(ureq::http::Error);

#[derive(Debug, Default)]
pub(crate) struct PublicInternetResolver(ureq::unversioned::resolver::DefaultResolver);

impl ureq::unversioned::resolver::Resolver for PublicInternetResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let resolved = self.0.resolve(uri, config, timeout)?;
        let mut public = self.empty();
        for address in resolved.iter().copied() {
            if is_public_ip(address.ip()) {
                public.push(address);
            }
        }
        if public.is_empty() {
            Err(ureq::Error::HostNotFound)
        } else {
            Ok(public)
        }
    }
}

#[derive(Debug)]
enum HttpResolver {
    Public(PublicInternetResolver),
    System(ureq::unversioned::resolver::DefaultResolver),
}

impl ureq::unversioned::resolver::Resolver for HttpResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        match self {
            Self::Public(resolver) => resolver.resolve(uri, config, timeout),
            Self::System(resolver) => resolver.resolve(uri, config, timeout),
        }
    }
}

fn is_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        std::net::IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(std::net::IpAddr::V4(mapped));
            }
            let segments = ip.segments();
            !(ip.is_unspecified()
                || ip.is_loopback()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xff00) == 0xff00
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

fn http_config(options: &HttpRequestOptions) -> ureq::config::Config {
    let proxy = if options.use_system_proxy {
        ureq::Proxy::try_from_env()
    } else {
        None
    };
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(
            options.timeout_ms.unwrap_or(DEFAULT_HTTP_TIMEOUT_MS),
        )))
        // mochi-core 构建时用 ureq 的 native-tls feature、不开 rustls。
        // 否则 ureq 的通用连接器会沿用 Rustls 默认的加密配置，
        // 一发 HTTPS 请求就 panic。
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        // AI 的裸 HTTP 访问不继承代理。唯一主动开启的调用方是内置搜索工具：
        // 它的目标地址由应用代码固定，不来自模型。
        .proxy(proxy)
        // 重定向直接交还调用方，发起新的、独立校验过的请求。
        .max_redirects(0)
        // 非 2xx 也要拿到响应体：模型往往就是靠错误响应判断下一步怎么走
        .http_status_as_error(false)
        .build()
}

/// 发起一次 HTTP(S) 请求。只允许 http/https——`file://` 能读本地任意文件，
/// 那会绕开工作区边界，等于给了 AI 一把万能读取钥匙。
pub fn http_request(options: &HttpRequestOptions) -> HttpRequestResult {
    let url = options.url.trim();
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http") | Some("https") => {}
        Some(other) => {
            return HttpRequestResult::failed(format!("仅支持 http/https，收到 {other}:"))
        }
        None => return HttpRequestResult::failed(format!("无效的 URL: {url}")),
    }

    let config = http_config(options);
    let resolver = if options.use_system_proxy {
        // 本地代理/TUN 客户端常会用 198.18/15 的假 IP 应答公网 DNS，
        // 并在回环地址上暴露 HTTP/SOCKS 代理。这两者都被
        // PublicInternetResolver 刻意拒绝，所以可信固定端点用普通解析器，
        // AI 的任意 URL 继续走过滤器。
        HttpResolver::System(ureq::unversioned::resolver::DefaultResolver::default())
    } else {
        HttpResolver::Public(PublicInternetResolver::default())
    };
    let agent = ureq::Agent::with_parts(
        config,
        ureq::unversioned::transport::DefaultConnector::default(),
        resolver,
    );

    let method = options
        .method
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("GET")
        .to_uppercase();

    // ureq 3 的 Agent 只暴露了 get/post/... 这些具体方法，任意方法要自己拼
    // `http::Request` 再交给 `run`。
    let mut builder = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(url);
    for (name, value) in &options.headers {
        builder = builder.header(name, value);
    }

    let sent = match &options.body {
        Some(body) => builder
            .body(body.as_str())
            .map_err(RequestBuildError)
            .map(|r| agent.run(r)),
        None => builder
            .body(())
            .map_err(RequestBuildError)
            .map(|r| agent.run(r)),
    };

    let sent = match sent {
        Ok(sent) => sent,
        Err(RequestBuildError(e)) => {
            return HttpRequestResult::failed(format!("请求构造失败: {e}"))
        }
    };

    match sent {
        Ok(mut response) => {
            let status = response.status();
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect();
            let body = response.body_mut().read_to_string().unwrap_or_default();
            HttpRequestResult {
                ok: status.is_success(),
                status: Some(status.as_u16()),
                status_text: Some(status.canonical_reason().unwrap_or_default().to_owned()),
                headers: Some(headers),
                body: Some(truncate(body)),
                error: None,
            }
        }
        Err(ureq::Error::Timeout(_)) => HttpRequestResult::failed("请求超时"),
        Err(e) => HttpRequestResult::failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_stops_a_running_command() {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let token = cancel.clone();
        let signal = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            token.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let started = Instant::now();
        let result = run_command_cancellable(
            if cfg!(windows) {
                "Start-Sleep -Seconds 20"
            } else {
                "sleep 20"
            },
            &std::env::temp_dir(),
            Some(30000),
            &cancel,
        );
        signal.join().unwrap();
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap_or("").contains("取消"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn output_buffer_is_bounded_while_the_pipe_is_drained() {
        let output = drain(Some(std::io::Cursor::new(vec![b'x'; 3_000_000]))).join();
        assert!(output.len() < MAX_OUTPUT_CHARS * 4 + 100);
        assert!(output.contains("已截断"));
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("mochi-shell-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 各平台上都能跑的「原样输出」命令。
    fn echo(text: &str) -> String {
        if cfg!(windows) {
            format!("Write-Output '{text}'")
        } else {
            format!("echo '{text}'")
        }
    }

    #[test]
    fn a_successful_command_reports_zero_and_its_output() {
        let dir = temp_dir("ok");
        let result = run_command(&echo("你好"), &dir, None);
        assert!(result.ok, "{result:?}");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("你好"), "{:?}", result.stdout);
        assert!(!result.timed_out);
        assert!(result.error.is_none());
    }

    #[test]
    fn a_failing_command_reports_its_exit_code() {
        let dir = temp_dir("fail");
        let result = run_command("exit 3", &dir, None);
        assert!(!result.ok);
        assert_eq!(result.exit_code, Some(3));
    }

    #[test]
    fn an_empty_command_is_refused_without_spawning_anything() {
        let dir = temp_dir("empty");
        let result = run_command("   ", &dir, None);
        assert!(!result.ok);
        assert_eq!(result.error.as_deref(), Some("命令为空"));
    }

    /// 超时要杀进程并如实上报，不能挂着等。
    #[test]
    fn a_slow_command_times_out_and_is_killed() {
        let dir = temp_dir("timeout");
        let sleep = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let started = Instant::now();
        let result = run_command(sleep, &dir, Some(400));

        assert!(result.timed_out, "{result:?}");
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap().contains("超时"));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "应尽快返回而不是等命令跑完"
        );
    }

    /// 管道缓冲区填满后子进程会阻塞在写上；不边跑边收就会死锁。
    #[test]
    fn large_output_does_not_deadlock() {
        let dir = temp_dir("bigout");
        let command = if cfg!(windows) {
            "1..4000 | ForEach-Object { 'x' * 100 }"
        } else {
            "for i in $(seq 1 4000); do printf 'x%.0s' $(seq 1 100); echo; done"
        };
        let result = run_command(command, &dir, Some(60_000));
        assert!(result.ok, "{:?}", result.error);
        assert!(
            result.stdout.len() > 100_000,
            "实际 {} 字节",
            result.stdout.len()
        );
    }

    #[test]
    fn the_command_runs_in_the_given_directory() {
        let dir = temp_dir("cwd");
        std::fs::write(dir.join("标记.txt"), "x").unwrap();
        let list = if cfg!(windows) {
            "Get-ChildItem -Name"
        } else {
            "ls"
        };
        let result = run_command(list, &dir, None);
        assert!(result.stdout.contains("标记.txt"), "{:?}", result.stdout);
    }

    #[test]
    fn output_is_truncated_by_characters_not_bytes() {
        let long = "中".repeat(MAX_OUTPUT_CHARS + 10);
        let truncated = truncate(long);
        assert!(truncated.contains("输出过长已截断"));
        assert!(truncated.contains(&format!("共 {} 字符", MAX_OUTPUT_CHARS + 10)));

        // 保留的正文正好是 MAX_OUTPUT_CHARS 个完整汉字，没有被切成半个
        let head: String = truncated.chars().take_while(|c| *c == '中').collect();
        assert_eq!(head.chars().count(), MAX_OUTPUT_CHARS);
        assert!(!truncated.contains('\u{fffd}'), "不该出现替换字符");
    }

    /// 中文 Windows 上 PowerShell 往管道里写的是 GBK，按 UTF-8 解会全成 `���`。
    #[test]
    fn console_output_falls_back_to_the_ansi_code_page() {
        assert_eq!(decode_console_output("你好 UTF-8".as_bytes()), "你好 UTF-8");
        assert_eq!(decode_console_output(b"plain ascii"), "plain ascii");
        assert_eq!(decode_console_output(b""), "");

        // GBK 的「你好」= C4 E3 BA C3，不是合法 UTF-8
        let gbk: Vec<u8> = vec![0xC4, 0xE3, 0xBA, 0xC3];
        assert!(std::str::from_utf8(&gbk).is_err(), "这段必须不是合法 UTF-8");
        let decoded = decode_console_output(&gbk);
        if cfg!(windows) && decoded.contains('\u{fffd}') {
            panic!("GBK 输出应能按系统代码页解出来，实得 {decoded:?}");
        }
    }

    #[test]
    fn short_output_is_left_alone() {
        assert_eq!(truncate("短".into()), "短");
    }

    /// file:// 能读本地任意文件，等于绕开工作区边界。
    #[test]
    fn non_http_schemes_are_refused() {
        for url in ["file:///C:/Windows/win.ini", "ftp://example.com/x"] {
            let result = http_request(&HttpRequestOptions {
                url: url.into(),
                ..Default::default()
            });
            assert!(!result.ok, "{url} 应被拒");
            assert!(
                result
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("仅支持 http/https"),
                "{result:?}"
            );
        }
    }

    #[test]
    fn a_malformed_url_is_refused() {
        let result = http_request(&HttpRequestOptions {
            url: "不是网址".into(),
            ..Default::default()
        });
        assert!(!result.ok);
        assert!(result.error.as_deref().unwrap().contains("无效的 URL"));
    }

    #[test]
    fn private_and_special_ip_ranges_are_not_public() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} 应被拒绝");
        }
        for ip in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} 应被允许");
        }
    }

    #[test]
    fn arbitrary_http_uses_native_tls_without_inheriting_a_proxy() {
        let config = http_config(&HttpRequestOptions::default());
        assert_eq!(
            config.tls_config().provider(),
            ureq::tls::TlsProvider::NativeTls
        );
        assert!(config.proxy().is_none());
    }

    #[test]
    fn localhost_http_requests_are_refused() {
        let result = http_request(&HttpRequestOptions {
            url: "http://localhost:9/".into(),
            timeout_ms: Some(600),
            ..Default::default()
        });
        assert!(!result.ok);
        assert!(result.error.is_some(), "{result:?}");
        assert!(result.status.is_none());
    }

    /// 不联网也要能验证「连不上时如实报错」而不是 panic。
    #[test]
    fn an_unreachable_host_reports_an_error() {
        let result = http_request(&HttpRequestOptions {
            // 保留给文档用途的 TEST-NET-1 网段，不会有人真的监听
            url: "http://192.0.2.1:9/".into(),
            timeout_ms: Some(600),
            ..Default::default()
        });
        assert!(!result.ok);
        assert!(result.error.is_some(), "{result:?}");
        assert!(result.status.is_none());
    }
}
