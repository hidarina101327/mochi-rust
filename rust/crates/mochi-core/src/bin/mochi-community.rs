//! 用于单个社区扩展包的命令行入口。
use anyhow::{bail, Context, Result};
use mochi_core::marketplace;

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        println!("mochi-community install <plugin|workflow|template> <name> [--workspace <folder>]\n从官方市场安装独立资源包。默认安装到当前工作区。");
        return Ok(());
    }
    if args.len() != 3 && args.len() != 5 {
        bail!("用法：mochi-community install <plugin|workflow|template> <name> [--workspace <folder>]");
    }
    if args[0] != "install" || !matches!(args[1].as_str(), "plugin" | "workflow" | "template") {
        bail!("不支持的命令或资源类型");
    }
    let workspace = if args.len() == 5 {
        if args[3] != "--workspace" {
            bail!("未知选项：{}", args[3]);
        }
        std::path::PathBuf::from(&args[4])
    } else {
        std::env::current_dir()?
    };
    let workspace = workspace.canonicalize().context("工作区目录不存在")?;
    let catalog = marketplace::fetch_catalog()?;
    let package = catalog
        .packages
        .iter()
        .find(|p| p.kind == args[1] && p.id == args[2])
        .context("官方市场中没有此资源")?;
    println!("正在下载 {} {}…", package.title, package.version);
    let folder = std::env::temp_dir().join(format!(
        "mochi-market-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::create_dir(&folder)?;
    let archive = folder.join(&package.asset);
    let result = marketplace::download(package, &archive)
        .and_then(|_| marketplace::install_archive(package, &archive, &workspace));
    let _ = std::fs::remove_dir_all(&folder);
    println!("已安装：{}", result?.display());
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("安装失败：{error:#}");
        std::process::exit(1);
    }
}
