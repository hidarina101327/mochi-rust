//! 只接受固定仓库和安装包命名，并校验 Release 提供的 SHA-256。网络请求在后台线程执行。

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::shell::{http_request, HttpRequestOptions};

/// 默认的 GitHub Releases 仓库。部署私有更新源时可以用
/// `MOCHI_UPDATE_REPOSITORY=owner/repository` 覆盖它。
pub const RELEASES_REPOSITORY: &str = "hidarina101327/mochi-releases";
pub const UPDATE_REPOSITORY_ENV: &str = "MOCHI_UPDATE_REPOSITORY";

/// 返回本次进程使用的更新仓库。
///
/// 只接受标准的 `owner/repository` GitHub 仓库名，避免把更新下载源误配为任意 URL。
/// 无效配置会安全地回退到内置的官方仓库。
pub fn releases_repository() -> String {
    std::env::var(UPDATE_REPOSITORY_ENV)
        .ok()
        .filter(|value| is_valid_repository(value))
        .unwrap_or_else(|| RELEASES_REPOSITORY.into())
}

pub fn latest_release_url() -> String {
    latest_release_url_for(&releases_repository())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub release_name: String,
    pub notes: String,
    pub url: String,
    pub published_at: Option<String>,
    /// 发布到 GitHub Release 的原生 Windows 安装程序。
    pub installer_name: String,
    pub installer_url: String,
    pub installer_size: u64,
    /// API 已提供时直接使用；否则下载同 Release 内的 `.sha256` 文件。
    pub installer_sha256: Option<String>,
    pub checksum_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

/// 查询公开仓库的最新稳定 Release。
///
/// GitHub 在尚未创建 Release 时返回 404，这种情况不是错误，返回 `Ok(None)`。
pub fn check_latest_release(current_version: &str) -> Result<Option<UpdateInfo>, String> {
    let repository = releases_repository();
    let response = http_request(&HttpRequestOptions {
        url: latest_release_url_for(&repository),
        headers: vec![
            ("Accept".into(), "application/vnd.github+json".into()),
            ("User-Agent".into(), "Mochi-Update-Checker".into()),
        ],
        timeout_ms: Some(8_000),
        use_system_proxy: true,
        ..HttpRequestOptions::default()
    });

    if response.status == Some(404) {
        return Ok(None);
    }
    if !response.ok {
        let status = response
            .status
            .map(|value| value.to_string())
            .unwrap_or_else(|| "网络错误".into());
        let detail = response.error.or(response.status_text).unwrap_or_default();
        return Err(if detail.is_empty() {
            format!("GitHub Releases 请求失败（{status}）")
        } else {
            format!("GitHub Releases 请求失败（{status}）：{detail}")
        });
    }

    let body = response
        .body
        .ok_or_else(|| "GitHub Releases 没有返回内容".to_owned())?;
    let release: GitHubRelease = serde_json::from_str(&body)
        .map_err(|error| format!("解析 GitHub Release 失败：{error}"))?;
    parse_release(release, current_version, &repository)
}

fn parse_release(
    release: GitHubRelease,
    current_version: &str,
    repository: &str,
) -> Result<Option<UpdateInfo>, String> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let Some(latest_version) = normalized_version(&release.tag_name) else {
        return Ok(None);
    };
    if !is_newer_version(current_version, &latest_version) {
        return Ok(None);
    }
    if !is_release_page_url(&release.html_url, repository) {
        return Err("GitHub Release 链接不是受支持的 HTTPS 地址".into());
    }
    let installer_name = format!("Mochi-{latest_version}-x64-Setup.exe");
    let installer = release
        .assets
        .iter()
        .find(|asset| asset.name == installer_name)
        .ok_or_else(|| format!("Release 缺少原生安装包 {installer_name}"))?;
    if installer.size == 0 || installer.size > MAX_INSTALLER_BYTES {
        return Err("Release 中的原生安装包大小无效".into());
    }
    if !is_release_asset_url(&installer.browser_download_url, repository) {
        return Err("原生安装包链接不是受支持的 GitHub HTTPS 地址".into());
    }
    let installer_sha256 = installer.digest.as_deref().and_then(parse_sha256_digest);
    let checksum_url = if installer_sha256.is_none() {
        let checksum_name = format!("{installer_name}.sha256");
        let checksum = release
            .assets
            .iter()
            .find(|asset| asset.name == checksum_name)
            .ok_or_else(|| format!("Release 缺少安装包校验文件 {checksum_name}"))?;
        if !is_release_asset_url(&checksum.browser_download_url, repository) {
            return Err("安装包校验文件链接不是受支持的 GitHub HTTPS 地址".into());
        }
        Some(checksum.browser_download_url.clone())
    } else {
        None
    };

    Ok(Some(UpdateInfo {
        latest_version,
        release_name: release
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| release.tag_name.clone()),
        notes: release
            .body
            .filter(|body| !body.trim().is_empty())
            .unwrap_or_else(|| "此版本没有填写更新说明。".into()),
        url: release.html_url,
        published_at: release.published_at,
        installer_name,
        installer_url: installer.browser_download_url.clone(),
        installer_size: installer.size,
        installer_sha256,
        checksum_url,
    }))
}

const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 下载并校验某个已经检查过的原生安装包，返回可交给安装助手执行的本地路径。
pub fn download_installer(update: &UpdateInfo) -> Result<PathBuf, String> {
    let expected = match &update.installer_sha256 {
        Some(value) => value.clone(),
        None => {
            let url = update
                .checksum_url
                .as_deref()
                .ok_or_else(|| "Release 缺少安装包 SHA-256 校验信息".to_owned())?;
            let bytes = download_trusted_bytes(url, 16 * 1024)?;
            let text =
                String::from_utf8(bytes).map_err(|_| "安装包校验文件不是 UTF-8 文本".to_owned())?;
            parse_checksum_file(&text)
                .ok_or_else(|| "安装包校验文件中没有有效的 SHA-256 值".to_owned())?
        }
    };

    let updates_dir = std::env::temp_dir().join("Mochi Updates");
    fs::create_dir_all(&updates_dir).map_err(|error| format!("创建更新目录失败：{error}"))?;
    let installer_path = updates_dir.join(&update.installer_name);
    if installer_path.is_file() && file_sha256(&installer_path)? == expected {
        return Ok(installer_path);
    }

    let partial_path = updates_dir.join(format!("{}.part", update.installer_name));
    if partial_path.exists() {
        fs::remove_file(&partial_path).map_err(|error| format!("清理未完成安装包失败：{error}"))?;
    }
    download_trusted_file(&update.installer_url, &partial_path, update.installer_size)?;
    let actual = file_sha256(&partial_path)?;
    if actual != expected {
        let _ = fs::remove_file(&partial_path);
        return Err("安装包 SHA-256 校验失败，已丢弃下载文件".into());
    }
    if installer_path.exists() {
        fs::remove_file(&installer_path).map_err(|error| format!("替换旧安装包失败：{error}"))?;
    }
    fs::rename(&partial_path, &installer_path)
        .map_err(|error| format!("完成安装包下载失败：{error}"))?;
    Ok(installer_path)
}

fn latest_release_url_for(repository: &str) -> String {
    format!("https://api.github.com/repos/{repository}/releases/latest")
}

fn is_valid_repository(repository: &str) -> bool {
    let mut parts = repository.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && repository.len() <= 200
        && repository
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

fn is_release_page_url(url: &str, repository: &str) -> bool {
    let prefix = format!("https://github.com/{repository}/releases/");
    url.starts_with(&prefix) && !url.contains(['\r', '\n'])
}

fn is_release_asset_url(url: &str, repository: &str) -> bool {
    let prefix = format!("https://github.com/{repository}/releases/download/");
    url.starts_with(&prefix) && !url.contains(['\r', '\n'])
}

fn parse_sha256_digest(value: &str) -> Option<String> {
    let value = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn parse_checksum_file(value: &str) -> Option<String> {
    value.split_whitespace().find_map(parse_sha256_digest)
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("读取安装包失败：{error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("校验安装包失败：{error}"))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest.as_slice() {
        use std::fmt::Write as _;
        let _ = write!(&mut text, "{byte:02x}");
    }
    Ok(text)
}

fn trusted_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(UPDATE_DOWNLOAD_TIMEOUT))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .proxy(ureq::Proxy::try_from_env())
        .max_redirects(5)
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
}

fn download_trusted_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = trusted_agent()
        .get(url)
        .header("Accept", "application/octet-stream")
        .header("User-Agent", "Mochi-Updater")
        .call()
        .map_err(|error| format!("下载更新校验文件失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载更新校验文件失败（HTTP {}）",
            response.status()
        ));
    }
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| format!("读取更新校验文件失败：{error}"))
}

fn download_trusted_file(url: &str, destination: &Path, expected_size: u64) -> Result<(), String> {
    if expected_size == 0 || expected_size > MAX_INSTALLER_BYTES {
        return Err("安装包大小超出允许范围".into());
    }
    let mut response = trusted_agent()
        .get(url)
        .header("Accept", "application/octet-stream")
        .header("User-Agent", "Mochi-Updater")
        .call()
        .map_err(|error| format!("下载原生安装包失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载原生安装包失败（HTTP {}）", response.status()));
    }
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_INSTALLER_BYTES)
        .reader();
    let mut output =
        File::create(destination).map_err(|error| format!("创建安装包文件失败：{error}"))?;
    let written = std::io::copy(&mut reader, &mut output)
        .map_err(|error| format!("写入原生安装包失败：{error}"))?;
    output
        .flush()
        .map_err(|error| format!("保存安装包失败：{error}"))?;
    if written != expected_size {
        let _ = fs::remove_file(destination);
        return Err(format!(
            "安装包大小不匹配（应为 {expected_size} 字节，实际为 {written} 字节）"
        ));
    }
    Ok(())
}

fn normalized_version(raw: &str) -> Option<String> {
    let version = raw.trim().strip_prefix(['v', 'V']).unwrap_or(raw.trim());
    let version = version
        .split_once('+')
        .map(|(value, _)| value)
        .unwrap_or(version)
        .split_once('-')
        .map(|(value, _)| value)
        .unwrap_or(version);
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    if parts.iter().any(|part| part.parse::<u64>().is_err()) {
        return None;
    }
    let mut normalized = parts
        .into_iter()
        .map(|part| part.parse::<u64>().unwrap_or_default())
        .collect::<Vec<_>>();
    while normalized.len() < 3 {
        normalized.push(0);
    }
    Some(format!(
        "{}.{}.{}",
        normalized[0], normalized[1], normalized[2]
    ))
}

fn version_parts(raw: &str) -> Option<[u64; 3]> {
    if raw.trim().split_once('-').is_some() {
        return None;
    }
    let normalized = normalized_version(raw)?;
    let mut parts = normalized.split('.').map(|part| part.parse().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

pub fn is_newer_version(current: &str, latest: &str) -> bool {
    match (version_parts(current), version_parts(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_like_release_tags() {
        assert!(is_newer_version("0.1.0", "v0.1.1"));
        assert!(is_newer_version("0.1.9", "0.2.0"));
        assert!(!is_newer_version("0.2.0", "v0.2.0"));
        assert!(!is_newer_version("0.3.0", "0.2.99"));
    }

    #[test]
    fn ignores_prereleases_and_invalid_versions() {
        assert!(!is_newer_version("0.1.0", "v0.2.0-beta.1"));
        assert!(!is_newer_version("0.1.0", "nightly"));
    }

    #[test]
    fn parses_a_release_and_keeps_release_notes() {
        let release = GitHubRelease {
            tag_name: "v0.2.0".into(),
            name: Some("Rust 原生版更新".into()),
            body: Some("- 更快的启动\n- 修复设置页".into()),
            html_url: "https://github.com/hidarina101327/mochi-releases/releases/tag/v0.2.0".into(),
            draft: false,
            prerelease: false,
            published_at: Some("2026-09-15T00:00:00Z".into()),
            assets: vec![GitHubAsset {
                name: "Mochi-0.2.0-x64-Setup.exe".into(),
                browser_download_url: "https://github.com/hidarina101327/mochi-releases/releases/download/v0.2.0/Mochi-0.2.0-x64-Setup.exe".into(),
                size: 1_024,
                digest: Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            }],
        };
        let info = parse_release(release, "0.1.0", RELEASES_REPOSITORY)
            .unwrap()
            .unwrap();
        assert_eq!(info.latest_version, "0.2.0");
        assert_eq!(info.release_name, "Rust 原生版更新");
        assert!(info.notes.contains("设置页"));
        assert_eq!(info.installer_name, "Mochi-0.2.0-x64-Setup.exe");
        assert!(info.checksum_url.is_none());
    }

    #[test]
    fn reads_standard_sha256_file_format() {
        assert_eq!(
            parse_checksum_file(
                "abABabababababababababababababababababababababababababababababab  setup.exe"
            ),
            Some("abababababababababababababababababababababababababababababababab".into())
        );
    }

    #[test]
    fn accepts_only_standard_github_repository_names() {
        assert!(is_valid_repository("example-org/mochi-releases"));
        assert!(is_valid_repository("example_org/mochi.releases"));
        assert!(!is_valid_repository("https://example.com/releases"));
        assert!(!is_valid_repository("owner/repo/extra"));
        assert!(!is_valid_repository("owner/"));
    }

    #[test]
    fn builds_latest_release_api_url_for_repository() {
        assert_eq!(
            latest_release_url_for("example-org/mochi-releases"),
            "https://api.github.com/repos/example-org/mochi-releases/releases/latest"
        );
    }
}
