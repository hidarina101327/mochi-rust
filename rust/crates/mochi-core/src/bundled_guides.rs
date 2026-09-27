//! 带版本号的离线文档，安装进用户选定的工作区。
use anyhow::{ensure, Context, Result};
use std::{fs, path::Path};

const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const FILES: &[(&str, &[u8])] = &[
    (
        "墨池-新手使用说明.mc",
        include_bytes!("../assets/release-guides/墨池-新手使用说明.mc"),
    ),
    (
        "墨池-全部说明.md",
        include_bytes!("../assets/release-guides/墨池-全部说明.md"),
    ),
    (
        "墨池-快捷键.md",
        include_bytes!("../assets/release-guides/墨池-快捷键.md"),
    ),
    (
        "墨池AI的一百个事实.mc",
        include_bytes!("../assets/release-guides/墨池AI的一百个事实.mc"),
    ),
    (
        "墨池-常用场景.mc",
        include_bytes!("../assets/release-guides/墨池-常用场景.mc"),
    ),
    (
        "assets/1789444337553-87132-0.png",
        include_bytes!("../assets/release-guides/assets/1789444337553-87132-0.png"),
    ),
    (
        "assets/1789444481718-87132-1.png",
        include_bytes!("../assets/release-guides/assets/1789444481718-87132-1.png"),
    ),
    (
        "assets/1789457707548-37072-0.png",
        include_bytes!("../assets/release-guides/assets/1789457707548-37072-0.png"),
    ),
    (
        "assets/1789460939350-40272-0.png",
        include_bytes!("../assets/release-guides/assets/1789460939350-40272-0.png"),
    ),
    (
        "assets/1789482073801-31720-0.png",
        include_bytes!("../assets/release-guides/assets/1789482073801-31720-0.png"),
    ),
    (
        "assets/1789992360505-81992-0.png",
        include_bytes!("../assets/release-guides/assets/1789992360505-81992-0.png"),
    ),
    (
        "assets/1789992689176-81992-1.png",
        include_bytes!("../assets/release-guides/assets/1789992689176-81992-1.png"),
    ),
    (
        "assets/1789993397820-81992-2.png",
        include_bytes!("../assets/release-guides/assets/1789993397820-81992-2.png"),
    ),
    (
        "assets/1789993442453-81992-3.png",
        include_bytes!("../assets/release-guides/assets/1789993442453-81992-3.png"),
    ),
    (
        "assets/1789993752590-72872-0.png",
        include_bytes!("../assets/release-guides/assets/1789993752590-72872-0.png"),
    ),
    (
        "assets/1789993976491-72872-1.png",
        include_bytes!("../assets/release-guides/assets/1789993976491-72872-1.png"),
    ),
    (
        "assets/1789994059730-72872-2.png",
        include_bytes!("../assets/release-guides/assets/1789994059730-72872-2.png"),
    ),
    (
        "assets/1789994762294-72872-3.png",
        include_bytes!("../assets/release-guides/assets/1789994762294-72872-3.png"),
    ),
    (
        "assets/1789994961534-72872-4.png",
        include_bytes!("../assets/release-guides/assets/1789994961534-72872-4.png"),
    ),
    (
        "assets/1789995453673-72872-6.png",
        include_bytes!("../assets/release-guides/assets/1789995453673-72872-6.png"),
    ),
    (
        "assets/1789995547599-72872-7.png",
        include_bytes!("../assets/release-guides/assets/1789995547599-72872-7.png"),
    ),
    (
        "assets/1789995870497-72872-9.png",
        include_bytes!("../assets/release-guides/assets/1789995870497-72872-9.png"),
    ),
    (
        "assets/1789996754400-65764-0.png",
        include_bytes!("../assets/release-guides/assets/1789996754400-65764-0.png"),
    ),
    (
        "assets/1789997082245-65764-1.png",
        include_bytes!("../assets/release-guides/assets/1789997082245-65764-1.png"),
    ),
    (
        "assets/1789997838340-65764-2.png",
        include_bytes!("../assets/release-guides/assets/1789997838340-65764-2.png"),
    ),
    (
        "assets/1789998106090-65764-3.png",
        include_bytes!("../assets/release-guides/assets/1789998106090-65764-3.png"),
    ),
    (
        "assets/1789998215775-65764-4.png",
        include_bytes!("../assets/release-guides/assets/1789998215775-65764-4.png"),
    ),
    (
        "assets/1789998416374-65764-5.png",
        include_bytes!("../assets/release-guides/assets/1789998416374-65764-5.png"),
    ),
    (
        "assets/1789998867495-65764-6.png",
        include_bytes!("../assets/release-guides/assets/1789998867495-65764-6.png"),
    ),
];

/// 每个版本只覆盖一次内置文件，或每次显式安装时覆盖。
/// 标记最后写入：更新中途失败，下次打开还能重试。
pub fn install(root: &Path, force: bool) -> Result<()> {
    ensure!(root.is_dir(), "工作区不存在");
    let root = root.canonicalize()?;
    let marker = root.join(".mochi/bundled-guides-version");
    checked_path(&root, &marker)?;
    fs::create_dir_all(root.join(".mochi"))?;
    let _lock = crate::settings::file::Lock::acquire(&marker)?;
    if !force && fs::read_to_string(&marker).ok().as_deref() == Some(VERSION) {
        return Ok(());
    }
    let destination = root.join("知识库/墨池");
    for (name, bytes) in FILES {
        let path = destination.join(name);
        checked_path(&root, &path)?;
        crate::files::FileService::new()
            .write_bytes_safe(&path, bytes)
            .with_context(|| format!("写入内置说明失败：{}", path.display()))?;
    }
    crate::files::FileService::new().write_file_safe(&marker, VERSION)?;
    Ok(())
}

fn checked_path(root: &Path, path: &Path) -> Result<()> {
    for ancestor in path.ancestors().take_while(|p| *p != root) {
        if fs::symlink_metadata(ancestor).is_ok() {
            ensure!(
                ancestor.canonicalize()?.starts_with(root),
                "说明文件目标指向工作区外"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installs_overwrites_and_preserves_unrelated_files() {
        let root = std::env::temp_dir().join(format!(
            "mochi-guides-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(root.join("知识库/墨池")).unwrap();
        let guide = root.join("知识库/墨池").join(FILES[0].0);
        let other = root.join("知识库/墨池/个人笔记.md");
        fs::write(&guide, "旧说明").unwrap();
        fs::write(&other, "保留").unwrap();
        install(&root, false).unwrap();
        for (name, bytes) in FILES {
            assert_eq!(
                fs::read(root.join("知识库/墨池").join(name)).unwrap(),
                *bytes
            );
        }
        fs::write(&guide, "本版本内编辑").unwrap();
        install(&root, false).unwrap();
        assert_eq!(fs::read_to_string(&guide).unwrap(), "本版本内编辑");
        install(&root, true).unwrap();
        assert_eq!(fs::read(&guide).unwrap(), FILES[0].1);
        fs::write(root.join(".mochi/bundled-guides-version"), "0.1.1").unwrap();
        fs::write(&guide, "上一版本").unwrap();
        install(&root, false).unwrap();
        assert_eq!(fs::read(&guide).unwrap(), FILES[0].1);
        assert_eq!(fs::read_to_string(other).unwrap(), "保留");
        fs::remove_dir_all(root).unwrap();
    }
}
