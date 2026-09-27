//! 社区发行版 = 一份目录清单 + 若干互相独立、带校验和的 ZIP 资源。
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, time::Duration};

mod install;
pub use install::install_archive;
#[cfg(test)]
mod tests;

pub const REPOSITORY: &str = "hidarina101327/mochi-community";
pub const RELEASES_URL: &str = "https://github.com/hidarina101327/mochi-community/releases";
pub const INDEX_ASSET: &str = "marketplace.json";
pub const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Document,
    Base,
    Canvas,
    Workflow,
    Template,
    Agent,
    KnowledgeBase,
    Plugin,
}
impl Category {
    pub const ALL: [Self; 8] = [
        Self::Document,
        Self::Base,
        Self::Canvas,
        Self::Workflow,
        Self::Template,
        Self::Agent,
        Self::KnowledgeBase,
        Self::Plugin,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Document => "文档类",
            Self::Base => "多维表格类",
            Self::Canvas => "画布类",
            Self::Workflow => "自动化类",
            Self::Template => "模版类",
            Self::Agent => "Agent 类",
            Self::KnowledgeBase => "知识库类",
            Self::Plugin => "插件类（预留）",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub id: String,
    pub kind: String,
    pub category: Category,
    pub title: String,
    pub version: String,
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub official: bool,
    #[serde(default)]
    pub author: String,
    pub asset: String,
    pub sha256: String,
    #[serde(skip)]
    pub download_url: String,
    #[serde(skip)]
    pub size: u64,
}
impl Package {
    pub fn install_command(&self) -> Option<String> {
        matches!(self.kind.as_str(), "plugin" | "workflow" | "template")
            .then(|| format!("mochi-community install {} {}", self.kind, self.id))
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(safe_id(&self.id) && safe_id(&self.kind), "无效的市场包标识");
        ensure!(
            !self.title.trim().is_empty()
                && !self.version.trim().is_empty()
                && !self.summary.trim().is_empty(),
            "市场条目缺少标题、版本或简介"
        );
        ensure!(
            self.asset == format!("{}-{}.zip", self.id, self.kind),
            "条目必须引用独立 ZIP：{}",
            self.id
        );
        ensure!(
            self.sha256.len() == 64 && self.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "无效的 SHA-256：{}",
            self.id
        );
        if let Some(image) = &self.image {
            let url = url::Url::parse(image).context("无效的封面地址")?;
            ensure!(
                url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty(),
                "封面必须使用 HTTPS"
            );
        }
        Ok(())
    }
}

pub(crate) fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !matches!(
            value,
            "con"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub schema_version: u32,
    pub packages: Vec<Package>,
}
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    pub release: String,
    pub packages: Vec<Package>,
}
#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub assets: Vec<Asset>,
}
#[derive(Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

fn release_asset_url(value: &str) -> bool {
    value.starts_with(&format!(
        "https://github.com/{REPOSITORY}/releases/download/"
    )) && !value.contains(['?', '#', '\\'])
}

pub fn resolve_index(index: Index, release: Release) -> Result<Catalog> {
    ensure!(
        index.schema_version == 1,
        "不支持的市场索引版本：{}",
        index.schema_version
    );
    ensure!(index.packages.len() <= 10000, "市场条目过多");
    let mut seen = HashSet::new();
    let mut packages = index.packages;
    for package in &mut packages {
        package.validate()?;
        ensure!(
            seen.insert((package.kind.clone(), package.id.clone())),
            "重复的市场条目：{}",
            package.id
        );
        let mut assets = release.assets.iter().filter(|a| a.name == package.asset);
        let asset = assets
            .next()
            .with_context(|| format!("Release 缺少 {}", package.asset))?;
        ensure!(assets.next().is_none(), "Release 包含重名资源");
        ensure!(
            release_asset_url(&asset.browser_download_url),
            "无效的 Release 下载地址"
        );
        ensure!(
            asset.size > 0 && asset.size <= MAX_ARCHIVE_BYTES,
            "资源大小超出限制"
        );
        package.download_url = asset.browser_download_url.clone();
        package.size = asset.size;
    }
    Ok(Catalog {
        release: release.tag_name,
        packages,
    })
}

fn client() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .build()
        .into()
}
fn get(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut response = client()
        .get(url)
        .header("User-Agent", "Mochi-Marketplace/1")
        .header("Accept", "application/octet-stream")
        .call()
        .context("无法连接官方市场，请检查网络后重试")?;
    Ok(response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()?)
}

pub fn fetch_catalog() -> Result<Catalog> {
    let endpoint = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = match client()
        .get(&endpoint)
        .header("User-Agent", "Mochi-Marketplace/1")
        .header("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(Catalog::default()),
        Err(ureq::Error::StatusCode(403 | 429)) => bail!("GitHub 请求额度已用完，请稍后重试"),
        Err(error) => return Err(error).context("无法加载官方市场，请检查网络后重试"),
    };
    let release: Release = serde_json::from_slice(
        &response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_vec()?,
    )?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == INDEX_ASSET)
        .context("此 Release 尚未发布 marketplace.json 索引")?;
    ensure!(
        release_asset_url(&asset.browser_download_url),
        "无效的市场索引地址"
    );
    let index = serde_json::from_slice(&get(&asset.browser_download_url, 4 * 1024 * 1024)?)?;
    resolve_index(index, release)
}

pub fn verify_archive(package: &Package, bytes: &[u8]) -> Result<()> {
    use sha2::{Digest, Sha256};
    package.validate()?;
    ensure!(bytes.len() as u64 <= MAX_ARCHIVE_BYTES, "资源大小超出限制");
    ensure!(
        package.size == 0 || package.size == bytes.len() as u64,
        "下载不完整，请重试"
    );
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        digest.eq_ignore_ascii_case(&package.sha256),
        "资源校验失败，请重新下载"
    );
    Ok(())
}

/// 独占方式创建，避免覆盖用户已有的下载文件。
pub fn download(package: &Package, destination: &Path) -> Result<()> {
    use std::io::Write;
    ensure!(release_asset_url(&package.download_url), "无效的资源地址");
    let bytes = get(&package.download_url, MAX_ARCHIVE_BYTES)?;
    verify_archive(package, &bytes)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .context("无法保存下载（同名文件可能已存在）")?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(destination);
        return Err(error.into());
    }
    Ok(())
}
