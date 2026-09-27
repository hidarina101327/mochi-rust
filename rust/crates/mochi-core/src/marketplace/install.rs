//! 校验并安装社区市场下载的资源包。
use super::*;
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
};

/// 写任何东西之前先校验所有路径。压缩包绝不会执行安装脚本。
pub fn install_archive(package: &Package, archive: &Path, workspace: &Path) -> Result<PathBuf> {
    ensure!(workspace.is_dir(), "工作区目录不存在");
    ensure!(
        fs::metadata(archive)?.len() <= MAX_ARCHIVE_BYTES,
        "ZIP 超出大小限制"
    );
    let bytes = fs::read(archive)?;
    verify_archive(package, &bytes)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(zip.len() <= 10000, "ZIP 文件数量过多");
    let mut names = HashSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        let name = file.name().trim_end_matches('/');
        ensure!(
            !name.is_empty() && !name.contains(['\\', ':']) && !name.starts_with('/'),
            "ZIP 路径不安全"
        );
        for part in name.split('/') {
            ensure!(
                !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' ']),
                "ZIP 路径不安全"
            );
            let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
            ensure!(
                ![
                    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6",
                    "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
                    "lpt8", "lpt9"
                ]
                .contains(&stem.as_str()),
                "ZIP 包含系统保留路径"
            );
        }
        ensure!(
            file.unix_mode()
                .is_none_or(|mode| mode & 0o170000 != 0o120000),
            "ZIP 不允许符号链接"
        );
        ensure!(names.insert(name.to_lowercase()), "ZIP 包含重复路径");
        total = total.checked_add(file.size()).context("ZIP 大小溢出")?;
        ensure!(total <= MAX_ARCHIVE_BYTES * 2, "ZIP 解压大小超出限制");
    }
    match package.kind.as_str() {
        "plugin" => {
            let manifest: crate::plugins::Manifest =
                serde_json::from_reader(zip.by_name("manifest.json")?)?;
            ensure!(
                manifest.id == package.id && manifest.version == package.version,
                "插件与市场索引不匹配"
            );
            let service = crate::plugins::PluginService::new(workspace);
            ensure!(
                !service.root().join(&manifest.id).exists(),
                "插件已安装，请先在插件设置中卸载旧版"
            );
            Ok(service.install_zip(archive)?.directory)
        }
        "workflow" => {
            let mut text = String::new();
            zip.by_name("workflow.json")?
                .take(524289)
                .read_to_string(&mut text)?;
            let store = crate::workflows::Store::open(workspace).map_err(anyhow::Error::msg)?;
            store.import(&text).map_err(anyhow::Error::msg)?;
            Ok(workspace.join(".mochi"))
        }
        "template" => {
            let root = crate::templates::TemplateService::new(workspace)
                .root()
                .to_path_buf();
            fs::create_dir_all(&root)?;
            let destination = root.join(&package.id);
            ensure!(!destination.exists(), "模版已安装，请先移除旧版");
            // create_dir 是独占的；绝不复用别的安装留下的暂存目录。
            let staging = root.join(format!(".market-{}-{}", package.id, std::process::id()));
            fs::create_dir(&staging).context("此模版安装任务已存在")?;
            let result = (|| -> Result<()> {
                let mut count = 0;
                for i in 0..zip.len() {
                    let mut entry = zip.by_index(i)?;
                    let name = entry.name().to_owned();
                    if entry.is_dir() || name == "marketplace.json" {
                        continue;
                    }
                    // TemplateService 只读一层分组；资源文件可以嵌套。
                    if !name.contains('/')
                        && matches!(
                            Path::new(&name).extension().and_then(|s| s.to_str()),
                            Some("md" | "mc" | "txt")
                        )
                    {
                        count += 1;
                    }
                    let target = staging.join(&name);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    std::io::copy(&mut entry, &mut fs::File::create(target)?)?;
                }
                ensure!(count > 0, "模版 ZIP 根目录没有 md、mc 或 txt 文件");
                ensure!(!destination.exists(), "模版已安装");
                fs::rename(&staging, &destination)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = fs::remove_dir_all(&staging);
            }
            result?;
            Ok(destination)
        }
        _ => bail!("此类别暂仅支持下载后导入"),
    }
}
