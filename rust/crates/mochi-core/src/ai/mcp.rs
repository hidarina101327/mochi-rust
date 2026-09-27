//! MCP JSON-RPC 客户端。stdio / Streamable HTTP，连接按需创建并在 drop 时回收子进程。
use super::{
    agent_config::McpServerDefinition,
    models::{AiToolDefinition, AiToolFunctionDef},
    permission::AiToolAction,
    tools::{ToolArgs, ToolExecutor, ToolOutcome},
};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(20);
/// 发现是可选的，且在模型请求之前完成。总体预算要短——本地服务不可用
/// 不能让助手看起来卡死；单个工具调用仍用下面的 `TIMEOUT`。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_MESSAGE: u64 = 8 * 1024 * 1024;
#[cfg(windows)]
const PROCESS_TREE_KILL_TIMEOUT: Duration = Duration::from_millis(500);

/// Windows 无法直接用 `CreateProcessW` 执行 `.cmd`/`.bat` 垫片。
/// Node 包管理器恰恰就是以这种方式暴露的，所以配置成 `npx`/`npm`
/// 的 MCP 在终端里明明能跑，启动却会报出误导性的失败。
/// 这里拼一条带引号的 `/C` 命令行，让含空格的路径和 MCP 参数里的
/// shell 元字符都能安全穿过这层命令解释器。
fn stdio_command(program: &str, args: &[String]) -> Command {
    #[cfg(windows)]
    {
        let lower = program.to_ascii_lowercase();
        let basename = lower.rsplit(['\\', '/']).next().unwrap_or(&lower);
        let shim = basename.ends_with(".cmd")
            || basename.ends_with(".bat")
            || matches!(
                basename,
                "npx" | "npm" | "pnpm" | "yarn" | "bun" | "uv" | "uvx"
            );
        if shim {
            let mut command = Command::new("cmd.exe");
            let mut command_line = quote_cmd_arg(program);
            for arg in args {
                command_line.push(' ');
                command_line.push_str(&quote_cmd_arg(arg));
            }
            command.args(["/D", "/S", "/C"]);
            // `raw_arg` 保住 `cmd /C ""path with spaces\\server.cmd" "arg&value""`
            // 需要的嵌套引号。同样的字符串若走 Command 常规的 Windows
            // 引号处理，会补上 cmd.exe 当字面字符的反斜杠。
            use std::os::windows::process::CommandExt;
            command.raw_arg(format!("\"{command_line}\""));
            return command;
        }
    }
    let mut command = Command::new(program);
    command.args(args);
    command
}

#[cfg(windows)]
fn quote_cmd_arg(value: &str) -> String {
    // 给 cmd 参数加引号，`&`、`|`、`<`、`>` 和空格就进不了命令语法。
    // 内嵌引号翻倍是 cmd 批处理文件的写法，这样这个助手对
    // JSON/配置类参数同样好用。
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// 按进程树停掉 stdio MCP。Windows 上包管理器垫片通常会在应用和 MCP
/// 之间多插一层 `cmd.exe`（再往里是 `node.exe`）；`Child::kill` 只杀掉
/// 外壳，真正的服务还在跑。`taskkill /T` 交给 OS 去清整棵后代树。
fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        if child.try_wait().ok().flatten().is_none() {
            use std::os::windows::process::CommandExt;
            let pid = child.id().to_string();
            let mut killer = Command::new("taskkill");
            killer
                .args(["/PID", pid.as_str(), "/T", "/F"])
                .creation_flags(0x08000000)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Ok(mut killer) = killer.spawn() {
                let deadline = Instant::now() + PROCESS_TREE_KILL_TIMEOUT;
                loop {
                    match killer.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        _ => {
                            let _ = killer.kill();
                            let _ = killer.wait();
                            break;
                        }
                    }
                }
            }
        }
        // taskkill 不可用、超时、或两次调用之间外壳恰好退出时，
        // 保留一次有界的直接 kill 兜底。它替代不了 `/T`——
        // 那样只能覆盖到外壳本身。
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
        }
        let _ = child.wait();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

enum Transport {
    Sse {
        agent: ureq::Agent,
        endpoint: String,
        output: Receiver<Value>,
        cancel: Arc<AtomicBool>,
    },
    Stdio {
        child: Child,
        input: ChildStdin,
        output: Receiver<Value>,
        cancel: Arc<AtomicBool>,
    },
    Http {
        agent: ureq::Agent,
        url: String,
        session: Option<String>,
        version: String,
        cancel: Arc<AtomicBool>,
    },
}
impl Drop for Transport {
    fn drop(&mut self) {
        let (Self::Sse { cancel, .. } | Self::Stdio { cancel, .. } | Self::Http { cancel, .. }) =
            self;
        cancel.store(true, Ordering::Relaxed);
        if let Self::Stdio { child, .. } = self {
            terminate_process_tree(child);
        }
    }
}
pub struct Client {
    transport: Transport,
    next_id: u64,
    pub tools: Vec<Value>,
}
impl Client {
    pub fn connect(config: &McpServerDefinition) -> Result<Self> {
        Self::connect_with_cancel(config, Arc::new(AtomicBool::new(false)))
    }

    /// 使用调用方自有的取消标志连接。Executor 并行探测多个 MCP 服务
    /// 时用它：一个死掉的进程不能劫持整个 AI 请求。
    pub fn connect_with_cancel(
        config: &McpServerDefinition,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self> {
        if !config.enabled {
            bail!("服务器未启用")
        }
        let transport = match config.transport.as_str() {
            "stdio" => {
                let program = config
                    .command
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .context("缺少 command")?;
                let mut command = stdio_command(program, &config.args);
                command
                    .envs(&config.env)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                let mut child = command.spawn().map_err(|error| {
                    anyhow::anyhow!("MCP 进程启动失败：无法启动 `{program}`（{error}）")
                })?;
                let input = child.stdin.take().context("MCP stdin 不可用")?;
                let output = child.stdout.take().context("MCP stdout 不可用")?;
                let (tx, rx) = sync_channel(256);
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(output);
                    loop {
                        let mut line = String::new();
                        use std::io::Read;
                        let result = reader.by_ref().take(MAX_MESSAGE).read_line(&mut line);
                        if !matches!(result,Ok(n) if n>0) || line.len() as u64 >= MAX_MESSAGE {
                            break;
                        }
                        if let Ok(v) = serde_json::from_str(&line) {
                            if tx.send(v).is_err() {
                                break;
                            }
                        }
                    }
                });
                Transport::Stdio {
                    child,
                    input,
                    output: rx,
                    cancel: cancel.clone(),
                }
            }
            "http" => {
                let url = config.url.clone().context("缺少 URL")?;
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    bail!("MCP URL 必须为 HTTP/HTTPS")
                }
                let agent = ureq::Agent::config_builder()
                    .timeout_global(Some(TIMEOUT))
                    .build()
                    .into();
                Transport::Http {
                    agent,
                    url,
                    session: None,
                    version: "2024-11-05".into(),
                    cancel: cancel.clone(),
                }
            }
            "sse" => {
                let url = config.url.clone().context("缺少 URL")?;
                let base = url::Url::parse(&url)?;
                if !matches!(base.scheme(), "http" | "https") {
                    bail!("MCP URL 必须为 HTTP/HTTPS")
                }
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_connect(Some(TIMEOUT))
                    .timeout_recv_body(Some(Duration::from_secs(60)))
                    .build()
                    .into();
                let stream_agent = agent.clone();
                let (tx, rx) = sync_channel::<Value>(256);
                let (endpoint_tx, endpoint_rx) = sync_channel(1);
                let stopped = cancel.clone();
                std::thread::spawn(move || {
                    let result = (|| -> Result<()> {
                        let mut response = stream_agent
                            .get(&url)
                            .header("Accept", "text/event-stream")
                            .call()?;
                        let mut reader = BufReader::new(response.body_mut().as_reader());
                        let mut event = String::new();
                        let mut data = String::new();
                        let mut endpoint_sent = false;
                        loop {
                            if stopped.load(Ordering::Relaxed) {
                                break;
                            }
                            use std::io::Read;
                            let mut line = String::new();
                            let read = reader.by_ref().take(MAX_MESSAGE).read_line(&mut line)?;
                            if read == 0 {
                                break;
                            }
                            if data.len() + line.len() > MAX_MESSAGE as usize {
                                bail!("MCP SSE 消息过大")
                            }
                            if line.trim().is_empty() {
                                if event == "endpoint" && !endpoint_sent {
                                    let endpoint = base.join(data.trim())?;
                                    if endpoint.origin() != base.origin() {
                                        bail!("MCP SSE endpoint 跨域")
                                    }
                                    let _ = endpoint_tx.send(Ok(endpoint.to_string()));
                                    endpoint_sent = true;
                                } else if let Ok(value) = serde_json::from_str(&data) {
                                    if tx.send(value).is_err() {
                                        break;
                                    }
                                }
                                event.clear();
                                data.clear();
                            } else if let Some(value) = line.strip_prefix("event:") {
                                event = value.trim().into();
                            } else if let Some(value) = line.strip_prefix("data:") {
                                data.push_str(value.trim_start());
                            }
                        }
                        Ok(())
                    })();
                    if let Err(e) = result {
                        let _ = endpoint_tx.try_send(Err(e.to_string()));
                    }
                });
                let endpoint = match endpoint_rx.recv_timeout(TIMEOUT) {
                    Ok(Ok(url)) => url,
                    other => {
                        cancel.store(true, Ordering::Relaxed);
                        bail!("MCP SSE endpoint 不可用：{other:?}")
                    }
                };
                Transport::Sse {
                    agent,
                    endpoint,
                    output: rx,
                    cancel,
                }
            }
            other => bail!("不支持的 MCP 传输方式：{other}"),
        };
        let mut client = Self {
            transport,
            next_id: 1,
            tools: Vec::new(),
        };
        let initialized=client.request("initialize",json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"mochi-native","version":env!("CARGO_PKG_VERSION")}}))?;
        if let Transport::Http { version, .. } = &mut client.transport {
            if let Some(v) = initialized["protocolVersion"].as_str() {
                *version = v.into();
            }
        }
        client.notify("notifications/initialized", json!({}))?;
        let mut cursor = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let page = client.request(
                "tools/list",
                cursor
                    .as_ref()
                    .map(|c| json!({"cursor":c}))
                    .unwrap_or(json!({})),
            )?;
            if let Some(tools) = page["tools"].as_array() {
                client.tools.extend(
                    tools
                        .iter()
                        .filter(|t| {
                            config.tools.is_empty()
                                || t["name"]
                                    .as_str()
                                    .is_some_and(|n| config.tools.iter().any(|allow| allow == n))
                        })
                        .cloned(),
                );
            }
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            let Some(c) = cursor.as_ref() else { break };
            if !seen.insert(c.clone()) || seen.len() > 100 {
                bail!("工具分页游标未推进")
            }
        }
        Ok(client)
    }
    fn is_cancelled(&self) -> bool {
        match &self.transport {
            Transport::Sse { cancel, .. }
            | Transport::Stdio { cancel, .. }
            | Transport::Http { cancel, .. } => cancel.load(Ordering::Relaxed),
        }
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.exchange(
            json!({"jsonrpc":"2.0","method":method,"params":params}),
            None,
        )
        .map(|_| ())
    }
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let response = self.exchange(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            Some(id),
        )?;
        if let Some(error) = response.get("error") {
            bail!("MCP：{}", error["message"].as_str().unwrap_or("服务器错误"))
        }
        response
            .get("result")
            .cloned()
            .context("MCP 响应缺少 result")
    }
    fn exchange(&mut self, message: Value, id: Option<u64>) -> Result<Value> {
        if self.is_cancelled() {
            bail!("MCP 连接已取消")
        }
        match &mut self.transport {
            Transport::Sse {
                agent,
                endpoint,
                output,
                cancel,
                ..
            } => {
                agent
                    .post(endpoint.as_str())
                    .config()
                    .timeout_global(Some(TIMEOUT))
                    .build()
                    .send_json(&message)?;
                let Some(id) = id else { return Ok(Value::Null) };
                let deadline = Instant::now() + TIMEOUT;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("MCP SSE 连接已取消")
                    }
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        bail!("MCP SSE 响应超时")
                    }
                    let value = match output.recv_timeout(left.min(Duration::from_millis(100))) {
                        Ok(value) => value,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            bail!("MCP SSE 连接断开或超时")
                        }
                    };
                    if value["id"].as_u64() == Some(id)
                        && (value.get("result").is_some() || value.get("error").is_some())
                    {
                        return Ok(value);
                    }
                    if value.get("method").is_some() && value.get("id").is_some() {
                        let response = if value["method"] == "ping" {
                            json!({"jsonrpc":"2.0","id":value["id"],"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Client capability not supported"}})
                        };
                        agent
                            .post(endpoint.as_str())
                            .config()
                            .timeout_global(Some(TIMEOUT))
                            .build()
                            .send_json(&response)?;
                    }
                }
            }
            Transport::Stdio {
                input,
                output,
                cancel,
                ..
            } => {
                serde_json::to_writer(&mut *input, &message)?;
                input.write_all(b"\n")?;
                input.flush()?;
                let Some(id) = id else { return Ok(Value::Null) };
                let deadline = Instant::now() + TIMEOUT;
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        bail!("MCP 进程连接已取消")
                    }
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        bail!("MCP 响应超时")
                    }
                    let v = match output.recv_timeout(left.min(Duration::from_millis(100))) {
                        Ok(value) => value,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                            bail!("MCP 响应超时或连接断开")
                        }
                    };
                    if v["id"].as_u64() == Some(id)
                        && (v.get("result").is_some() || v.get("error").is_some())
                    {
                        return Ok(v);
                    }
                    if v.get("method").is_some() && v.get("id").is_some() {
                        let response = if v["method"] == "ping" {
                            json!({"jsonrpc":"2.0","id":v["id"],"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Client capability not supported"}})
                        };
                        serde_json::to_writer(&mut *input, &response)?;
                        input.write_all(b"\n")?;
                        input.flush()?;
                    }
                }
            }
            Transport::Http {
                agent,
                url,
                session,
                version,
                cancel,
            } => {
                if cancel.load(Ordering::Relaxed) {
                    bail!("MCP HTTP 连接已取消")
                }
                let mut request = agent
                    .post(url.as_str())
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", version.as_str());
                if let Some(s) = session.as_ref() {
                    request = request.header("Mcp-Session-Id", s.as_str());
                }
                let mut response = request.send_json(&message)?;
                if let Some(s) = response
                    .headers()
                    .get("Mcp-Session-Id")
                    .and_then(|v| v.to_str().ok())
                {
                    *session = Some(s.into());
                }
                if id.is_none() {
                    return Ok(Value::Null);
                }
                let sse = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("text/event-stream"));
                if sse {
                    use std::io::Read;
                    let mut reader =
                        BufReader::new(response.body_mut().as_reader().take(MAX_MESSAGE));
                    let mut data = String::new();
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            bail!("MCP HTTP 连接已取消")
                        }
                        let mut line = String::new();
                        if reader.read_line(&mut line)? == 0 {
                            bail!("MCP SSE 在响应前关闭")
                        }
                        if line.trim().is_empty() {
                            if let Ok(v) = serde_json::from_str::<Value>(&data) {
                                if v["id"].as_u64() == id {
                                    return Ok(v);
                                }
                            }
                            data.clear();
                        } else if let Some(value) = line.strip_prefix("data:") {
                            data.push_str(value.trim());
                        }
                    }
                } else {
                    let v: Value = response.body_mut().read_json()?;
                    if v["id"].as_u64() != id {
                        bail!("MCP 响应 id 不匹配")
                    }
                    Ok(v)
                }
            }
        }
    }
}
pub fn tool_name(server: &str, name: &str) -> String {
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    format!("mcp__{}__{}", clean(server), clean(name))
}
pub struct Executor {
    clients: Mutex<Vec<(McpServerDefinition, Client)>>,
    definitions: Vec<AiToolDefinition>,
}
impl Executor {
    pub fn connect(configs: Vec<McpServerDefinition>) -> (Self, Vec<String>) {
        Self::connect_with_cancel(configs, Arc::new(AtomicBool::new(false)))
    }

    pub fn connect_with_cancel(
        configs: Vec<McpServerDefinition>,
        stop: Arc<AtomicBool>,
    ) -> (Self, Vec<String>) {
        let mut clients = Vec::new();
        let mut definitions = Vec::new();
        let mut errors = Vec::new();
        let mut names = std::collections::HashSet::new();
        let enabled = configs
            .into_iter()
            .filter(|c| c.enabled && !c.is_builtin())
            .collect::<Vec<_>>();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut cancellations = Vec::with_capacity(enabled.len());
        for (index, config) in enabled.iter().cloned().enumerate() {
            let cancel = Arc::new(AtomicBool::new(false));
            cancellations.push(cancel.clone());
            let tx = tx.clone();
            std::thread::spawn(move || {
                let result = Client::connect_with_cancel(&config, cancel);
                let _ = tx.send((index, config, result));
            });
        }
        drop(tx);

        // MCP 发现在模型请求之外是锦上添花。并发探测各服务并给出
        // 一个整体上限：某个本地进程死了或 URL 连不上，绝不能挡住
        // 其余服务失败之后对基础 AI 提供方的调用。
        let mut completed = vec![false; enabled.len()];
        let mut connected = Vec::with_capacity(enabled.len());
        let deadline = Instant::now() + CONNECT_TIMEOUT + Duration::from_secs(1);
        while completed.iter().any(|done| !done) {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left.min(Duration::from_millis(100))) {
                Ok((index, config, result)) => {
                    if let Some(done) = completed.get_mut(index) {
                        *done = true;
                    }
                    connected.push((index, config, result));
                }
                // `recv_timeout` 刻意用较短轮询间隔，取消才能被及时感知。
                // 一次轮询超时不等于发现超时：继续等，直到下面共享的
                // 截止时间。曾经把轮询超时当终局处理，结果凡是需要
                // 100 ms 以上的 MCP 服务全被当成不可用。
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        for (index, done) in completed.iter().enumerate() {
            if !done {
                cancellations[index].store(true, Ordering::Relaxed);
                errors.push(if stop.load(Ordering::Relaxed) {
                    format!("{}：MCP 连接已取消", enabled[index].name)
                } else {
                    format!(
                        "{}：MCP 连接超时（{} 秒），已跳过该服务器",
                        enabled[index].name,
                        CONNECT_TIMEOUT.as_secs()
                    )
                });
            }
        }
        connected.sort_by_key(|(index, _, _)| *index);
        for (_, config, result) in connected {
            match result {
                Ok(client) => {
                    let mapped: Vec<String> = client
                        .tools
                        .iter()
                        .filter_map(|t| t["name"].as_str())
                        .map(|n| tool_name(&config.id, n))
                        .collect();
                    if mapped
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != mapped.len()
                        || mapped.iter().any(|n| names.contains(n))
                    {
                        errors.push(format!(
                            "{}：工具名称归一化后冲突，请使用不同的服务器 id / 工具名称",
                            config.name
                        ));
                        continue;
                    }
                    names.extend(mapped);
                    for tool in &client.tools {
                        if let Some(name) = tool["name"].as_str() {
                            definitions.push(AiToolDefinition {
                                kind: "function".into(),
                                function: AiToolFunctionDef {
                                    name: tool_name(&config.id, name),
                                    description: format!(
                                        "[MCP {}] {}",
                                        config.name,
                                        tool["description"].as_str().unwrap_or(name)
                                    ),
                                    parameters: if tool["inputSchema"].is_object() {
                                        tool["inputSchema"].clone()
                                    } else {
                                        json!({"type":"object","properties":{}})
                                    },
                                },
                            });
                        }
                    }
                    clients.push((config, client));
                }
                Err(e) => errors.push(format!("{}：{e}", config.name)),
            }
        }
        (
            Self {
                clients: Mutex::new(clients),
                definitions,
            },
            errors,
        )
    }
    pub fn definitions(&self) -> Vec<AiToolDefinition> {
        self.definitions.clone()
    }
}
impl ToolExecutor for Executor {
    fn handles(&self, name: &str) -> bool {
        self.definitions.iter().any(|d| d.function.name == name)
    }
    fn required_actions(&self, _: &str, _: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        vec![
            (AiToolAction::ExecuteCommand, None),
            (AiToolAction::NetworkAccess, None),
        ]
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let mut clients = self.clients.lock().map_err(|_| "MCP 连接锁不可用")?;
        for (config, client) in clients.iter_mut() {
            if let Some(raw) = client
                .tools
                .iter()
                .filter_map(|t| t["name"].as_str())
                .find(|t| tool_name(&config.id, t) == name)
                .map(str::to_owned)
            {
                let result = client
                    .request("tools/call", json!({"name":raw,"arguments":args.value()}))
                    .map_err(|e| e.to_string())?;
                if result["isError"] == true {
                    return Err(result["content"].to_string());
                }
                return Ok(result);
            }
        }
        Err("MCP 工具未连接".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config(transport: &str) -> McpServerDefinition {
        McpServerDefinition {
            id: "test".into(),
            name: "Test".into(),
            description: None,
            transport: transport.into(),
            command: None,
            args: Vec::new(),
            url: None,
            env: Default::default(),
            tools: vec!["echo".into()],
            enabled: true,
            source_path: String::new(),
        }
    }

    #[test]
    fn legacy_sse_endpoint_multiline_events_and_rpc_call() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/sse", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: endpoint\r\ndata: /messages?session=test\r\n\r\n").unwrap();
            stream.flush().unwrap();
            for i in 0..4 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                assert!(headers.starts_with("POST /messages?session=test "));
                let n = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                let mut bytes = vec![0; n];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                write!(
                    socket,
                    "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                socket.flush().unwrap();
                if i == 1 {
                    assert_eq!(request["method"], "notifications/initialized");
                    continue;
                }
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => json!({"protocolVersion":"2024-11-05","capabilities":{}}),
                    "tools/list" => {
                        json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})
                    }
                    "tools/call" => {
                        json!({"content":[{"type":"text","text":request["params"]["arguments"]["text"]}]})
                    }
                    _ => panic!("unexpected method"),
                };
                let data = serde_json::to_string_pretty(
                    &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                )
                .unwrap();
                write!(stream, ": keepalive\r\nevent: message\r\n").unwrap();
                for line in data.lines() {
                    write!(stream, "data: {line}\r\n").unwrap();
                }
                write!(stream, "\r\n").unwrap();
                stream.flush().unwrap();
            }
        });
        let mut c = config("sse");
        c.url = Some(url);
        let mut client = Client::connect(&c).unwrap();
        assert_eq!(client.tools.len(), 1);
        let result = client
            .request(
                "tools/call",
                json!({"name":"echo","arguments":{"text":"中文 SSE"}}),
            )
            .unwrap();
        assert_eq!(result["content"][0]["text"], "中文 SSE");
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn http_handshake_session_headers_tool_filter_and_rpc_call() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for i in 0..4 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let n = headers
                    .lines()
                    .find_map(|l| {
                        l.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                let mut body = vec![0; n];
                reader.read_exact(&mut body).unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                if i > 0 {
                    assert!(headers
                        .to_lowercase()
                        .contains("mcp-session-id: test-session"));
                }
                let body = if request.get("id").is_none() {
                    String::new()
                } else {
                    let result = match request["method"].as_str().unwrap() {
                        "initialize" => {
                            json!({"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"test","version":"1"}})
                        }
                        "tools/list" => {
                            json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}},{"name":"not_allowed"}]})
                        }
                        "tools/call" => {
                            json!({"content":[{"type":"text","text":request["params"]["arguments"]["text"]}]})
                        }
                        _ => panic!("unexpected method"),
                    };
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string()
                };
                write!(socket,"HTTP/1.1 {}\r\nContent-Type: application/json\r\nMcp-Session-Id: test-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",if body.is_empty(){"202 Accepted"}else{"200 OK"},body.len(),body).unwrap();
            }
        });
        let mut c = config("http");
        c.url = Some(url);
        let mut client = Client::connect(&c).unwrap();
        assert_eq!(client.tools.len(), 1);
        let result = client
            .request(
                "tools/call",
                json!({"name":"echo","arguments":{"text":"中文"}}),
            )
            .unwrap();
        assert_eq!(result["content"][0]["text"], "中文");
        server.join().unwrap();
    }

    #[test]
    fn executor_waits_for_delayed_http_discovery() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for index in 0..3 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap();
                let mut bytes = vec![0; content_length];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                if index == 0 {
                    // 这里刻意比 Executor 的 100 ms 轮询间隔长。
                    // 一次轮询超时不能让一个健康的服务凭空消失。
                    std::thread::sleep(Duration::from_millis(350));
                }
                let body = if request.get("id").is_none() {
                    String::new()
                } else {
                    let result = match request["method"].as_str().unwrap() {
                        "initialize" => {
                            json!({"protocolVersion":"2024-11-05","capabilities":{}})
                        }
                        "tools/list" => {
                            json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]})
                        }
                        method => panic!("unexpected MCP method: {method}"),
                    };
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string()
                };
                write!(
                    socket,
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nMcp-Session-Id: delayed-test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    if body.is_empty() { "202 Accepted" } else { "200 OK" },
                    body.len(),
                    body
                ).unwrap();
                socket.flush().unwrap();
            }
        });
        let mut c = config("http");
        c.url = Some(url);
        let started = Instant::now();
        let (executor, errors) = Executor::connect(vec![c]);
        let elapsed = started.elapsed();
        server.join().unwrap();
        assert!(
            errors.is_empty(),
            "delayed MCP discovery failed: {errors:?}"
        );
        assert!(executor
            .definitions()
            .iter()
            .any(|definition| definition.function.name == "mcp__test__echo"));
        assert!(
            elapsed >= Duration::from_millis(300),
            "discovery returned before the delayed response: {elapsed:?}"
        );
    }

    #[test]
    fn executor_connect_can_be_cancelled_while_http_discovery_waits() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            let _ = socket.read(&mut request);
            // 请求保持足够久，让发现只能被共享停止标志结束，
            // 而不是被 HTTP 响应结束。
            std::thread::sleep(Duration::from_millis(600));
        });
        let mut c = config("http");
        c.url = Some(url);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let started = Instant::now();
        let probe = std::thread::spawn(move || Executor::connect_with_cancel(vec![c], worker_stop));
        std::thread::sleep(Duration::from_millis(120));
        stop.store(true, Ordering::Relaxed);
        let (executor, errors) = probe.join().unwrap();
        let elapsed = started.elapsed();
        server.join().unwrap();
        assert!(executor.definitions().is_empty());
        assert!(
            errors.iter().any(|error| error.contains("取消")),
            "{errors:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "cancellation stalled: {elapsed:?}"
        );
    }

    #[test]
    #[cfg(windows)]
    fn stdio_handshake_and_cleanup_use_a_real_subprocess() {
        let mut c = config("stdio");
        c.command = Some("powershell.exe".into());
        c.args=vec!["-NoProfile".into(),"-NonInteractive".into(),"-Command".into(),r#"while ($line = [Console]::ReadLine()) { $request = $line | ConvertFrom-Json; if ($null -ne $request.id) { $result = if ($request.method -eq 'tools/list') { @{ tools = @(@{name='echo';inputSchema=@{type='object'}}) } } else { @{ protocolVersion='2024-11-05' } }; [Console]::WriteLine((@{jsonrpc='2.0';id=$request.id;result=$result} | ConvertTo-Json -Compress -Depth 8)) } }"#.into()];
        let client = Client::connect(&c).unwrap();
        assert_eq!(client.tools[0]["name"], "echo");
        drop(client);
    }

    #[test]
    #[cfg(windows)]
    fn windows_shim_preserves_paths_spaces_and_shell_metacharacters() {
        use std::io::Read;
        let script =
            std::env::temp_dir().join(format!("mochi mcp shim {}.cmd", std::process::id()));
        std::fs::write(
            &script,
            "@echo off\r\nset \"first=%~1\"\r\nset \"second=%~2\"\r\necho \"%first%\" \"%second%\"\r\n",
        )
        .unwrap();
        let args = vec!["left&right".to_owned(), "two words".to_owned()];
        let mut command = stdio_command(script.to_string_lossy().as_ref(), &args);
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(output.trim(), "\"left&right\" \"two words\"");
        let _ = std::fs::remove_file(script);
    }

    #[test]
    fn names_are_namespaced_without_splitting_underscores() {
        assert_eq!(tool_name("foo_bar", "lookup_中"), "mcp__foo_bar__lookup__");
    }
    #[test]
    fn disabled_or_missing_command_never_starts_a_process() {
        let c = McpServerDefinition {
            id: "x".into(),
            name: "x".into(),
            description: None,
            transport: "stdio".into(),
            command: None,
            args: vec![],
            url: None,
            env: Default::default(),
            tools: vec![],
            enabled: false,
            source_path: String::new(),
        };
        assert!(Client::connect(&c).is_err());
        let c = McpServerDefinition { enabled: true, ..c };
        assert!(Client::connect(&c).is_err());
    }

    #[test]
    fn builtin_mcp_definitions_are_not_started_by_external_discovery() {
        let mut c = config("stdio");
        c.id = "schedule-local".into();
        c.name = "Local Schedule Data".into();
        c.command = Some("mochi-internal".into());
        c.tools = vec!["agenda_list".into()];
        let (executor, errors) = Executor::connect(vec![c]);
        assert!(
            errors.is_empty(),
            "built-in adapter must not be a failure: {errors:?}"
        );
        assert!(executor.definitions().is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn windows_process_tree_termination_stops_a_shim_descendant() {
        let marker =
            std::env::temp_dir().join(format!("mochi-mcp-child-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let marker_literal = marker.to_string_lossy().replace('\'', "''");
        let script = format!(
            "Start-Sleep -Milliseconds 1200; Set-Content -LiteralPath '{marker_literal}' -Value done"
        );
        let command_line =
            format!("powershell.exe -NoProfile -NonInteractive -Command \"{script}\"");
        let mut child = Command::new("cmd.exe")
            .args(["/D", "/S", "/C", &command_line])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let started = Instant::now();
        terminate_process_tree(&mut child);
        assert!(started.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "descendant survived process-tree cleanup");
        let _ = std::fs::remove_file(marker);
    }
}
