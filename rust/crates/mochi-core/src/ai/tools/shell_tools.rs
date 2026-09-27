//! 先检查 execute_command；非免审白名单命令只生成提案，用户确认后才执行。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use super::host::ToolHost;
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::shell::{self, HttpRequestOptions};
use crate::{jstime, paths};

const TOOL_NAMES: &[&str] = &["shell_run", "web_search", "http_request"];
const WEB_SEARCH_ENDPOINT: &str = "https://www.bing.com/search";

pub struct ShellToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl ShellToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    fn run(&self, args: &ToolArgs) -> ToolOutcome {
        let command = args.str_required("command")?.trim().to_owned();
        if command.is_empty() {
            return Err("命令不能为空".into());
        }

        // cwd **允许指向工作区外**（对齐 TS）——命令本来就可能要在别处跑；
        // 但缺省锁在工作区根，免得模型随手乱跑。
        let workspace = paths::to_forward_slashes(&self.host.workspace_root().to_string_lossy());
        let cwd = args
            .str_opt("cwd")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| workspace.clone());
        let timeout_ms = args
            .i64_opt("timeoutMs")
            .filter(|ms| *ms > 0)
            .map(|ms| ms as u64);
        let program = extract_program_name(&command);

        if self.host.shell_whitelist().contains(&program) {
            let result = shell::run_command(&command, &PathBuf::from(&cwd), timeout_ms);
            let data = json!({
                "command": command,
                "cwd": cwd,
                "exitCode": result.exit_code,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "autoApproved": true,
            });
            // 命令跑完但退出码非 0 时，TS 会把 ok 设成 false 并同时带上 data。
            // 本层只有 Ok/Err 两态，非零退出走 Ok——stdout/stderr/exitCode 都在里面，
            // 模型能自己判断成败；包成 Err 反而会丢掉输出。
            return Ok(match result.error {
                Some(error) => merge(data, json!({ "ok": false, "error": error })),
                None => merge(data, json!({ "ok": result.ok })),
            });
        }

        // 白名单之外：只返回提案，**绝不执行**
        self.host.propose_shell_command(json!({
            "pendingShellCommand": {
                "id": format!("cmd-{}-{}", jstime::now_millis(), paths::random_base36(6)),
                "command": command,
                "cwd": cwd,
                "summary": args
                    .str_opt("summary")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("执行命令：{command}")),
                "program": program,
                "timeoutMs": timeout_ms,
                "status": "pending",
            }
        }))
    }

    fn http(&self, args: &ToolArgs) -> ToolOutcome {
        let url = args.str_required("url")?.trim().to_owned();
        if url.is_empty() {
            return Err("URL 不能为空".into());
        }

        let headers = match args.get("headers") {
            Some(Value::Object(map)) => map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect(),
            _ => Vec::new(),
        };

        let result = shell::http_request(&HttpRequestOptions {
            url: url.clone(),
            method: args.str_opt("method").map(str::to_owned),
            headers,
            body: args.str_opt("body").map(str::to_owned),
            timeout_ms: args
                .i64_opt("timeoutMs")
                .filter(|ms| *ms > 0)
                .map(|ms| ms as u64),
            use_system_proxy: false,
        });

        // 连不上/超时/协议不支持 → 失败；HTTP 4xx/5xx 是**成功拿到了响应**，
        // 状态码和响应体照样给模型，它据此决定下一步。
        if let Some(error) = result.error {
            return Err(error);
        }
        Ok(json!({
            "url": url,
            "status": result.status,
            "statusText": result.status_text,
            "headers": result.headers,
            "body": result.body,
        }))
    }

    fn web_search(&self, args: &ToolArgs) -> ToolOutcome {
        let query = args.str_required("query")?.trim();
        if query.is_empty() {
            return Err("搜索词不能为空".into());
        }
        let max_results = args.i64_opt("maxResults").unwrap_or(5).clamp(1, 10) as usize;
        let domains = args
            .get("domains")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|domain| {
                domain
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or("")
            })
            .filter(|domain| !domain.is_empty())
            .collect::<Vec<_>>();
        let scoped = if domains.is_empty() {
            query.to_owned()
        } else {
            format!(
                "{query} ({})",
                domains
                    .iter()
                    .map(|domain| format!("site:{domain}"))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            )
        };
        let result = shell::http_request(&web_search_request(&scoped));
        if let Some(error) = result.error {
            return Err(error);
        }
        let results = parse_search_rss(result.body.as_deref().unwrap_or_default(), max_results);
        if results.is_empty() {
            return Err(format!(
                "搜索服务未返回可解析结果（HTTP {}）",
                result
                    .status
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ));
        }
        Ok(json!({ "query": query, "results": results }))
    }
}

fn web_search_request(scoped_query: &str) -> HttpRequestOptions {
    let encoded = url::form_urlencoded::byte_serialize(scoped_query.as_bytes()).collect::<String>();
    HttpRequestOptions {
        url: format!("{WEB_SEARCH_ENDPOINT}?format=rss&q={encoded}"),
        method: Some("GET".into()),
        headers: vec![(
            "Accept".into(),
            "application/rss+xml, application/xml;q=0.9, text/xml;q=0.8".into(),
        )],
        body: None,
        timeout_ms: Some(30_000),
        // 此 URL 固定由应用提供。通用 http_request 工具不支持
        // 系统代理或 TUN。
        use_system_proxy: true,
    }
}

fn xml_text(value: &str) -> String {
    static TAGS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let value = value
        .strip_prefix("<![CDATA[")
        .and_then(|value| value.strip_suffix("]]>"))
        .unwrap_or(value);
    TAGS.get_or_init(|| regex::Regex::new(r"<[^>]+>").unwrap())
        .replace_all(value, " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tag_text(item: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    item.find(&open)
        .and_then(|start| {
            let body = &item[start + open.len()..];
            body.find(&close).map(|end| xml_text(&body[..end]))
        })
        .unwrap_or_default()
}

fn parse_search_rss(body: &str, max_results: usize) -> Vec<Value> {
    let mut rest = body;
    let mut results = Vec::new();
    while results.len() < max_results {
        let Some(start) = rest.find("<item") else {
            break;
        };
        rest = &rest[start..];
        let Some(open_end) = rest.find('>') else {
            break;
        };
        let Some(end) = rest[open_end + 1..].find("</item>") else {
            break;
        };
        let item = &rest[open_end + 1..open_end + 1 + end];
        let url = tag_text(item, "link");
        if url.starts_with("http://") || url.starts_with("https://") {
            results.push(json!({
                "title": tag_text(item, "title"),
                "url": url,
                "snippet": tag_text(item, "description"),
            }));
        }
        rest = &rest[open_end + 1 + end + "</item>".len()..];
    }
    results
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(target), Some(source)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    base
}

/// 从命令行里取出程序名：去掉路径与 `.exe/.cmd/.bat/.ps1/.sh` 后缀，转小写。
/// 对齐 TS 的 `extractProgramName`——白名单比对的是这个归一化后的名字。
pub fn extract_program_name(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("");
    let without_path = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let lower = without_path.to_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".sh"] {
        if let Some(stem) = lower.strip_suffix(suffix) {
            return stem.to_owned();
        }
    }
    lower
}

impl ToolExecutor for ShellToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    fn required_actions(
        &self,
        name: &str,
        _args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        match name {
            // execute_command 是唯一默认关闭的动作。注意**提案也要过这道闸**：
            // 权限关着的时候，连"要不要执行这条命令"的卡片都不该弹给用户。
            "shell_run" => vec![(AiToolAction::ExecuteCommand, None)],
            _ => vec![(AiToolAction::NetworkAccess, None)],
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "shell_run" => self.run(args),
            "web_search" => self.web_search(args),
            "http_request" => self.http(args),
            other => Err(format!("未知工具: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{ActionPermissions, AiPermissionService};
    use crate::ai::tools::ToolRegistry;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct WhitelistHost {
        root: PathBuf,
        whitelist: Vec<String>,
    }

    impl ToolHost for WhitelistHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn shell_whitelist(&self) -> Vec<String> {
            self.whitelist.clone()
        }
    }

    struct Fixture {
        root: PathBuf,
        registry: ToolRegistry,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn run(&self, name: &str, args: Value) -> Value {
            let call = AiToolCall {
                id: "c".into(),
                kind: "function".into(),
                function: AiToolFunction {
                    name: name.into(),
                    arguments: args.to_string(),
                },
            };
            serde_json::from_str(&self.registry.execute(&call)).unwrap()
        }
        fn ok(&self, name: &str, args: Value) -> Value {
            let out = self.run(name, args);
            assert_eq!(out["ok"], true, "{out}");
            out["data"].clone()
        }
        fn err(&self, name: &str, args: Value) -> String {
            let out = self.run(name, args);
            assert_eq!(out["ok"], false, "{out}");
            out["error"].as_str().unwrap().to_owned()
        }
    }

    /// `execute_command` 默认关闭，测试里按需打开。
    fn fixture(tag: &str, whitelist: &[&str], allow_exec: bool) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-shelltool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let host = Arc::new(WhitelistHost {
            root: root.clone(),
            whitelist: whitelist.iter().map(|s| s.to_string()).collect(),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let mut actions = ActionPermissions::default();
        actions.set(AiToolAction::ExecuteCommand, allow_exec);
        perms.set_action_permissions(actions);

        let registry = ToolRegistry::new(perms).with(Arc::new(ShellToolExecutor::new(host)));
        Fixture { root, registry }
    }

    fn echo(text: &str) -> String {
        if cfg!(windows) {
            format!("Write-Output '{text}'")
        } else {
            format!("echo '{text}'")
        }
    }

    fn echo_program() -> &'static str {
        if cfg!(windows) {
            "write-output"
        } else {
            "echo"
        }
    }

    #[test]
    fn program_names_are_normalized_for_whitelist_matching() {
        assert_eq!(extract_program_name("git status"), "git");
        assert_eq!(
            extract_program_name("  C:\\Tools\\Git\\bin\\git.EXE log "),
            "git"
        );
        assert_eq!(extract_program_name("/usr/bin/ls -la"), "ls");
        assert_eq!(extract_program_name("build.cmd"), "build");
        assert_eq!(extract_program_name("deploy.sh --now"), "deploy");
        assert_eq!(extract_program_name(""), "");
    }

    #[test]
    fn web_search_rss_is_reduced_to_citable_results() {
        let rss = r#"<rss><channel>
          <item><title><![CDATA[Official &amp; current]]></title><link>https://example.com/docs</link><description>Read <b>this</b>.</description></item>
          <item><title>Second</title><link>https://example.org/2</link><description>More</description></item>
        </channel></rss>"#;
        let results = parse_search_rss(rss, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["title"], "Official & current");
        assert_eq!(results[0]["url"], "https://example.com/docs");
        assert_eq!(results[0]["snippet"], "Read this .");
    }

    #[test]
    fn web_search_proxy_mode_is_limited_to_the_fixed_application_endpoint() {
        let request = web_search_request("owasp site:owasp.org");
        assert!(request.use_system_proxy);
        assert!(request.url.starts_with(WEB_SEARCH_ENDPOINT));
        assert!(request.url.contains("format=rss"));
        assert!(request.url.contains("site%3Aowasp.org"));
    }

    #[test]
    #[ignore = "requires a live public network; run explicitly for native release verification"]
    fn live_web_search_uses_the_machine_proxy_and_returns_citable_urls() {
        let f = fixture("live-web-search", &[], true);
        let data = f.ok(
            "web_search",
            json!({ "query": "OWASP Top 10", "domains": ["owasp.org"], "maxResults": 3 }),
        );
        let results = data["results"].as_array().expect("results array");
        assert!(!results.is_empty());
        assert!(results.iter().all(|result| result["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("http"))));
    }

    /// 带空格的路径按空白切会取到半截（TS 的 `split(/\s+/)[0]` 同样如此）。
    /// 后果是**匹配不上白名单**——于是转成待批准提案，失败方向是安全的那一侧，
    /// 所以照搬而不是自作聪明地解析引号。
    #[test]
    fn a_quoted_path_with_spaces_falls_out_of_the_whitelist() {
        assert_eq!(
            extract_program_name("\"C:\\Program Files\\Git\\git.exe\" log"),
            "program"
        );
        assert_eq!(
            extract_program_name("C:\\Program Files\\Git\\git.exe log"),
            "program"
        );
    }

    /// 白名单之外只出提案，绝不执行。
    #[test]
    fn a_non_whitelisted_command_only_produces_a_proposal() {
        let f = fixture("propose", &[], true);
        let data = f.ok(
            "shell_run",
            json!({ "command": "git status", "summary": "看看改了什么" }),
        );

        let pending = &data["pendingShellCommand"];
        assert_eq!(pending["command"], "git status");
        assert_eq!(pending["program"], "git");
        assert_eq!(pending["status"], "pending");
        assert_eq!(pending["summary"], "看看改了什么");
        assert!(pending["id"].as_str().unwrap().starts_with("cmd-"));
        assert!(data["stdout"].is_null(), "提案阶段不该有输出——命令根本没跑");
    }

    #[test]
    fn the_proposal_summary_defaults_to_the_command() {
        let f = fixture("summary", &[], true);
        let data = f.ok("shell_run", json!({ "command": "git status" }));
        assert_eq!(
            data["pendingShellCommand"]["summary"],
            "执行命令：git status"
        );
    }

    #[test]
    fn the_working_directory_defaults_to_the_workspace_root() {
        let f = fixture("cwd", &[], true);
        let data = f.ok("shell_run", json!({ "command": "git status" }));
        let cwd = data["pendingShellCommand"]["cwd"].as_str().unwrap();
        assert!(
            cwd.ends_with(&f.root.file_name().unwrap().to_string_lossy().to_string()),
            "{cwd}"
        );
    }

    #[test]
    fn a_whitelisted_command_runs_immediately() {
        let f = fixture("whitelist", &[echo_program()], true);
        let data = f.ok("shell_run", json!({ "command": echo("你好") }));

        assert_eq!(data["autoApproved"], true);
        assert_eq!(data["exitCode"], 0);
        assert!(data["stdout"].as_str().unwrap().contains("你好"));
        assert!(
            data["pendingShellCommand"].is_null(),
            "白名单命令不该再出提案"
        );
    }

    /// 退出码非 0 仍返回输出：包成错误会把 stdout/stderr 一起丢掉。
    #[test]
    fn a_failing_whitelisted_command_still_returns_its_output() {
        let f = fixture("exitcode", &["exit"], true);
        let data = f.ok("shell_run", json!({ "command": "exit 3" }));
        assert_eq!(data["exitCode"], 3);
        assert_eq!(data["ok"], false, "内层 ok 标记命令本身失败了");
    }

    /// 权限关着时连提案都不该出现——否则等于把决定权推给用户去点一个本不该有的卡片。
    #[test]
    fn without_execute_permission_not_even_a_proposal_appears() {
        let f = fixture("noperm", &[], false);
        let err = f.err("shell_run", json!({ "command": "git status" }));
        assert!(err.contains("执行命令"), "{err}");
    }

    #[test]
    fn an_empty_command_is_refused() {
        let f = fixture("empty", &[], true);
        assert!(f.err("shell_run", json!({})).contains("command"));
        assert!(f
            .err("shell_run", json!({ "command": "   " }))
            .contains("不能为空"));
    }

    #[test]
    fn http_requires_a_url() {
        let f = fixture("nourl", &[], true);
        assert!(f.err("http_request", json!({})).contains("url"));
    }

    /// file:// 能读本地任意文件，必须挡住。
    #[test]
    fn http_refuses_non_http_schemes() {
        let f = fixture("scheme", &[], true);
        let err = f.err(
            "http_request",
            json!({ "url": "file:///C:/Windows/win.ini" }),
        );
        assert!(err.contains("仅支持 http/https"), "{err}");
    }

    #[test]
    fn http_needs_the_network_action() {
        let f = fixture("nonet", &[], true);
        let mut actions = ActionPermissions::default();
        actions.set(AiToolAction::NetworkAccess, false);
        f.registry.permissions().set_action_permissions(actions);

        let err = f.err("http_request", json!({ "url": "https://example.com" }));
        assert!(!err.is_empty(), "关掉网络权限后应被拒");
    }

    /// 连不上时如实报错，而不是假装拿到了空响应。
    #[test]
    fn an_unreachable_host_becomes_a_tool_error() {
        let f = fixture("unreachable", &[], true);
        let err = f.err(
            "http_request",
            json!({ "url": "http://192.0.2.1:9/", "timeoutMs": 600 }),
        );
        assert!(!err.is_empty());
    }
}
