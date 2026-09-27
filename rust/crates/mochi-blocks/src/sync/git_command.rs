//! 校验 Git 端点，并构造和解析 Git 命令。
use super::*;

pub(super) fn validate_git_endpoint(endpoint: &str) -> Result<()> {
    ensure!(!endpoint.is_empty(), "Git endpoint must not be empty");
    ensure!(
        endpoint.trim() == endpoint,
        "Git endpoint must not have surrounding whitespace"
    );
    ensure!(
        !endpoint.starts_with('-'),
        "Git endpoint option injection is not allowed"
    );
    ensure!(
        !endpoint
            .as_bytes()
            .iter()
            .any(|byte| *byte == 0 || *byte < 0x20 || *byte == 0x7f),
        "Git endpoint contains a control character"
    );
    ensure!(
        !endpoint.contains('@'),
        "Git endpoint userinfo or credential URLs are not allowed"
    );
    ensure!(
        !endpoint.contains('%'),
        "encoded Git endpoint components are not allowed"
    );
    ensure!(
        !endpoint.contains('?') && !endpoint.contains('#'),
        "Git endpoint query and fragment components are not allowed"
    );
    let lower = endpoint.to_ascii_lowercase();
    ensure!(
        !lower.starts_with("ext::") && !lower.contains("::"),
        "Git ext and helper protocols are not allowed"
    );
    if let Some(separator) = endpoint.find("://") {
        let scheme = &lower[..separator];
        ensure!(
            matches!(scheme, "file" | "ssh" | "http" | "https"),
            "Git endpoint protocol is not allowed: {scheme}"
        );
        let authority_start = separator + 3;
        let authority_end = endpoint[authority_start..]
            .find(['/', '\\'])
            .map(|offset| authority_start + offset)
            .unwrap_or(endpoint.len());
        ensure!(
            authority_end > authority_start,
            "Git endpoint authority must not be empty"
        );
        ensure!(
            !endpoint[authority_start..authority_end].contains('@'),
            "Git endpoint userinfo is not allowed"
        );
    }
    Ok(())
}

pub(super) fn endpoint_is_network(endpoint: &str) -> bool {
    let lower = endpoint.to_ascii_lowercase();
    if lower.starts_with("ssh://") || lower.starts_with("http://") || lower.starts_with("https://")
    {
        return true;
    }
    // Git 也接受 scp 风格的 `host:path` 写法。带盘符的 Windows 路径
    //（`C:\\...`）是本地的；其他含冒号的非 URL 端点会被 Git
    // 当成 SSH 风格的远端名。
    !lower.contains("://")
        && lower.as_bytes().iter().enumerate().any(|(index, byte)| {
            *byte == b':' && !(index == 1 && lower.as_bytes()[0].is_ascii_alphabetic())
        })
}

pub(super) fn configure_git_command(command: &mut Command, instance: u64) {
    let temp = std::env::temp_dir();
    let private_prefix = temp.join(format!(
        "mochi-blocks-gitcli-private-{}-{instance}",
        std::process::id()
    ));
    let hooks = private_prefix.join("hooks-disabled");
    let global = private_prefix.join("global-config-do-not-read");
    let system = private_prefix.join("system-config-do-not-read");
    command
        // 这些环境变量可以换掉传输助手、注入 Git 配置或改道对象存储。
        // 移除继承值，让显式端点和裸路径成为仅有的输入。
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_SSH")
        .env_remove("GIT_PROXY_COMMAND")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_EXEC_PATH")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_TEMPLATE_DIR")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_DIFF_OPTS")
        .env_remove("GIT_TRACE")
        .env_remove("GIT_TRACE_PACKET")
        .env_remove("GIT_TRACE_PERFORMANCE")
        .env_remove("GIT_TRACE_SETUP")
        .env_remove("GIT_TRACE_SHALLOW")
        .env_remove("GIT_TRACE_CURL")
        .env_remove("GIT_REPLACE_REF_BASE")
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &global)
        .env("GIT_CONFIG_SYSTEM", &system)
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "credential.helper")
        .env("GIT_CONFIG_VALUE_0", "")
        .env("GIT_CONFIG_KEY_1", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_1", &hooks)
        .env("GIT_CONFIG_KEY_2", "protocol.ext.allow")
        .env("GIT_CONFIG_VALUE_2", "never")
        .env("GIT_ALLOW_PROTOCOL", "file:ssh:http:https");
}

pub(super) fn parse_ls_remote_head(output: &[u8], branch_ref: &str) -> Result<Option<Oid>> {
    let text = std::str::from_utf8(output).context("Git CLI returned invalid ls-remote output")?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(line) = lines.next() else {
        return Ok(None);
    };
    ensure!(
        lines.next().is_none(),
        "ls-remote returned more than one branch ref"
    );
    let mut fields = line.split_whitespace();
    let oid = fields
        .next()
        .context("ls-remote output omitted object id")?;
    let reference = fields.next().context("ls-remote output omitted ref name")?;
    ensure!(
        reference == branch_ref,
        "ls-remote returned an unexpected branch ref"
    );
    ensure!(
        fields.next().is_none(),
        "ls-remote returned an unexpected extra field"
    );
    Ok(Some(Oid::from_str(oid)?))
}
