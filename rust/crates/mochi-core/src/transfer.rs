//! 可搬移、类型显式声明的 Mochi ZIP 包。压缩包内的路径绝不直接解压使用。
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub const MANIFEST: &str = "mochi-package.json";
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Workflow,
    Agent,
    DesktopCards,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Workflow => "工作流",
            Self::Agent => "Agent",
            Self::DesktopCards => "桌面卡片",
        }
    }
    fn entry(self) -> &'static str {
        match self {
            Self::Workflow => "workflow.json",
            Self::Agent => "agent.md",
            Self::DesktopCards => "desktop-cards.json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    kind: Kind,
    name: String,
    entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub kind: Kind,
    pub name: String,
    pub content: String,
}

impl Package {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty() && self.name.chars().count() <= 120,
            "包名称为空或过长"
        );
        ensure!(self.content.len() <= 512 * 1024, "包内容超过 512 KiB");
        match self.kind {
            Kind::Workflow => {
                let flow = serde_json::from_str(&self.content).context("工作流格式错误")?;
                crate::workflows::validate(&flow).map_err(anyhow::Error::msg)?;
            }
            Kind::DesktopCards => {
                crate::desktop_cards::DesktopConfig::import_json(&self.content)?;
            }
            Kind::Agent => {
                let (data, body) = crate::ai::agent_config::parse_frontmatter(&self.content);
                ensure!(
                    data.iter().any(|(key, value)| key == "name"
                        && matches!(value,
                    crate::ai::agent_config::FmValue::Scalar(name) if !name.trim().is_empty()))
                        && !body.trim().is_empty(),
                    "Agent 缺少名称或提示词"
                );
            }
        }
        Ok(())
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let manifest = Manifest {
            format: "mochi-package".into(),
            version: 1,
            kind: self.kind,
            name: self.name.clone(),
            entry: self.kind.entry().into(),
        };
        // 完整构建好之后，才碰用户选定的目标位置。
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(MANIFEST, options)?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        zip.start_file(self.kind.entry(), options)?;
        zip.write_all(self.content.as_bytes())?;
        let bytes = zip.finish()?.into_inner();
        crate::files::FileService::new()
            .write_bytes_safe(path, &bytes)
            .context("无法保存导出包")
    }

    pub fn install(&self, root: &Path) -> Result<()> {
        self.validate()?;
        match self.kind {
            Kind::Workflow => {
                crate::workflows::Store::open(root)
                    .map_err(anyhow::Error::msg)?
                    .import(&self.content)
                    .map_err(anyhow::Error::msg)?;
            }
            Kind::DesktopCards => {
                let mut config = crate::desktop_cards::DesktopConfig::load(root)?;
                config.merge_import(&self.content)?;
                config.save(root)?;
            }
            Kind::Agent => {
                let directory = crate::ai::agent_config::AgentConfigService::new(root)
                    .root()
                    .join("Agents");
                let workspace = root.canonicalize()?;
                let mut existing = directory.as_path();
                while !existing.exists() {
                    existing = existing.parent().context("Agent 目录不可用")?;
                }
                ensure!(
                    existing.canonicalize()?.starts_with(&workspace),
                    "Agent 目录指向工作区之外"
                );
                fs::create_dir_all(&directory)?;
                ensure!(
                    directory.canonicalize()?.starts_with(&workspace),
                    "Agent 目录指向工作区之外"
                );
                let safe: String = self
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_control() || "<>:\"/\\|?*".contains(c) {
                            '_'
                        } else {
                            c
                        }
                    })
                    .collect();
                // 加前缀还能避开 Windows 保留设备名。始终新建一份定义。
                let file = directory.join(format!(
                    "导入-{}-{}.md",
                    safe.trim_matches([' ', '.']),
                    crate::paths::random_base36(8)
                ));
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&file)?;
                if let Err(error) = output.write_all(self.content.as_bytes()) {
                    drop(output);
                    let _ = fs::remove_file(file);
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }
}

/// 普通 ZIP 返回 None；认得出是 Mochi 包但内容损坏的，按错误处理。
pub fn inspect(path: &Path) -> Result<Option<Package>> {
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        return Ok(None);
    }
    let file = fs::File::open(path)?;
    let named_package = path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy().to_ascii_lowercase();
        [
            ".mochi-workflow.zip",
            ".mochi-agent.zip",
            ".mochi-cards.zip",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    });
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(zip) => zip,
        Err(error) => {
            ensure!(!named_package, "Mochi ZIP 包已损坏：{error}");
            return Ok(None);
        }
    };
    if !zip.file_names().any(|n| n == MANIFEST) {
        ensure!(!named_package, "Mochi 包缺少 mochi-package.json 清单");
        return Ok(None);
    }
    ensure!(
        fs::metadata(path)?.len() <= MAX_BYTES,
        "Mochi 导入包超过 2 MiB"
    );
    ensure!(zip.len() == 2, "Mochi 包必须包含清单和一个内容文件");
    let manifest: Manifest = serde_json::from_str(&read_entry(&mut zip, MANIFEST, 4096)?)
        .context("Mochi 包清单格式不支持")?;
    ensure!(
        manifest.format == "mochi-package" && manifest.version == 1,
        "不支持的 Mochi 包格式或版本"
    );
    ensure!(
        manifest.entry == manifest.kind.entry(),
        "包类型与内容文件不匹配"
    );
    let content = read_entry(&mut zip, &manifest.entry, 512 * 1024)?;
    let package = Package {
        kind: manifest.kind,
        name: manifest.name,
        content,
    };
    package.validate()?;
    Ok(Some(package))
}

fn read_entry<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<String> {
    let file = zip.by_name(name).context("包缺少内容文件")?;
    ensure!(
        !file.is_dir() && !file.is_symlink() && file.size() <= limit,
        "包内容类型或大小不支持"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "包解压内容超过限制");
    Ok(String::from_utf8(bytes)?)
}

/// 各功能的专属文件选择器继续接受旧版 JSON。
pub fn read_content(path: &Path, expected: Kind) -> Result<String> {
    if let Some(package) = inspect(path)? {
        ensure!(package.kind == expected, "请选择{}导入包", expected.label());
        return Ok(package.content);
    }
    let mut content = String::new();
    fs::File::open(path)?
        .take(512 * 1024 + 1)
        .read_to_string(&mut content)?;
    ensure!(content.len() <= 512 * 1024, "导入内容超过 512 KiB");
    Ok(content)
}

#[cfg(test)]
mod tests;
