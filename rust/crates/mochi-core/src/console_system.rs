//! 显式的工作区传输与 Git 同步，不经过 shell 拼接。
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::Path,
};
const MAX_BYTES: u64 = 1024 * 1024 * 1024;
pub fn fingerprint(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
pub fn install_plugin(workspace: &Path, archive: &Path) -> Result<Value> {
    inspect_archive(archive)?;
    let svc = crate::plugins::PluginService::new(workspace);
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut manifests = Vec::new();
    for i in 0..zip.len() {
        let name = zip.by_index(i)?.name().to_owned();
        if name == "manifest.json" || name.ends_with("/manifest.json") {
            manifests.push(name);
        }
    }
    ensure!(
        manifests.len() == 1,
        "插件包必须且只能包含一个 manifest.json"
    );
    let manifest: crate::plugins::Manifest =
        serde_json::from_reader(zip.by_name(&manifests[0])?.take(1024 * 1024))?;
    ensure!(
        !svc.list()?.iter().any(|p| p.manifest.id == manifest.id),
        "插件已安装，请先显式卸载旧版"
    );
    drop(zip);
    let installed = svc.install_zip(archive)?;
    Ok(
        json!({"manifest":installed.manifest,"path":installed.directory,"enabled":installed.enabled}),
    )
}
fn safe_entry(name: &str) -> Result<()> {
    let name = name.trim_end_matches('/');
    ensure!(
        !name.is_empty() && !name.contains(['\\', ':']) && !name.starts_with('/'),
        "ZIP 路径无效"
    );
    for p in name.split('/') {
        ensure!(
            !p.is_empty() && !matches!(p, "." | "..") && !p.ends_with(['.', ' ']),
            "ZIP 路径无效"
        );
        let stem = p.split('.').next().unwrap().to_ascii_lowercase();
        ensure!(
            ![
                "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
                "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
                "lpt9"
            ]
            .contains(&stem.as_str()),
            "ZIP 系统保留路径"
        );
    }
    Ok(())
}
pub fn inspect_archive(path: &Path) -> Result<Value> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    let mut total = 0u64;
    let mut files = Vec::new();
    let mut names = std::collections::HashSet::new();
    ensure!(zip.len() <= 100_000, "压缩包条目过多");
    for i in 0..zip.len() {
        let f = zip.by_index(i)?;
        safe_entry(f.name())?;
        ensure!(
            names.insert(f.name().trim_end_matches('/').to_lowercase()),
            "ZIP 包含重复路径"
        );
        ensure!(
            f.enclosed_name().is_some() && !f.name().contains('\\'),
            "ZIP 路径无效"
        );
        ensure!(
            f.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "不接受符号链接"
        );
        total = total.checked_add(f.size()).context("压缩包大小溢出")?;
        ensure!(total <= MAX_BYTES, "解压总量超过 1 GiB");
        files.push(json!({"path":f.name(),"bytes":f.size()}));
    }
    Ok(json!({"files":files,"totalBytes":total}))
}
pub fn export_workspace(root: &Path, destination: &Path) -> Result<Value> {
    let root = root.canonicalize()?;
    ensure!(!destination.exists(), "导出文件已存在");
    let parent = destination
        .parent()
        .context("缺少导出目录")?
        .canonicalize()?;
    ensure!(
        !parent.starts_with(&root),
        "导出目标必须在工作区外，避免把导出包递归打包"
    );
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = (|| {
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut pending = vec![root.clone()];
        let mut bytes = 0u64;
        let mut count = 0;
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(dir)? {
                let e = entry?;
                let p = e.path();
                let ty = e.file_type()?;
                ensure!(!ty.is_symlink(), "工作区包含符号链接，请单独处理映射目录");
                let relative = p.strip_prefix(&root)?.to_string_lossy().replace('\\', "/");
                if relative == ".mochi/git" || relative == ".git" {
                    continue;
                }
                if ty.is_dir() {
                    zip.add_directory(format!("{relative}/"), options)?;
                    pending.push(p);
                    continue;
                }
                if !ty.is_file() {
                    continue;
                }
                let before = e.metadata()?;
                bytes = bytes.checked_add(before.len()).context("文件大小溢出")?;
                ensure!(bytes <= MAX_BYTES, "导出超过 1 GiB");
                ensure!(p.canonicalize()?.starts_with(&root), "文件路径越界");
                zip.start_file(relative, options)?;
                let copied = std::io::copy(
                    &mut std::fs::File::open(&p)?.take(before.len() + 1),
                    &mut zip,
                )?;
                ensure!(copied == before.len(), "导出期间文件大小变化");
                let after = std::fs::metadata(&p)?;
                ensure!(
                    before.len() == after.len() && before.modified()? == after.modified()?,
                    "导出期间文件发生变化，请重试"
                );
                count += 1;
            }
        }
        zip.finish()?;
        Ok(json!({"path":destination,"files":count,"bytes":bytes}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}
pub fn import_workspace(archive: &Path, destination: &Path) -> Result<Value> {
    inspect_archive(archive)?;
    ensure!(!destination.exists(), "导入目标必须是尚不存在的新目录");
    let parent = destination
        .parent()
        .context("目标缺少父目录")?
        .canonicalize()?;
    let staging = parent.join(format!(".mochi-import-{}", crate::paths::random_base36(16)));
    std::fs::create_dir(&staging)?;
    let result = (|| {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            safe_entry(f.name())?;
            ensure!(
                f.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
                "不接受符号链接"
            );
            let relative = f.enclosed_name().context("ZIP 路径无效")?;
            let p = staging.join(relative);
            if f.is_dir() {
                std::fs::create_dir_all(p)?;
                continue;
            }
            std::fs::create_dir_all(p.parent().unwrap())?;
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(p)?;
            let copied = std::io::copy(&mut f.by_ref().take(MAX_BYTES - total + 1), &mut out)?;
            total = total.checked_add(copied).context("解压体积溢出")?;
            ensure!(total <= MAX_BYTES, "解压体积超过 1 GiB");
            out.flush()?;
        }
        std::fs::rename(&staging, destination)?;
        Ok(json!({"imported":true,"path":destination}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}
fn git(root: &Path, args: &[&str]) -> Result<String> {
    let svc = crate::git::GitService::new(root);
    ensure!(svc.is_valid(), "工作区版本库不可用，请先启用版本历史");
    let mut cmd = std::process::Command::new("git");
    cmd.arg("--git-dir")
        .arg(svc.git_dir_path())
        .arg("--work-tree")
        .arg(root)
        .args([
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=20",
            "-c",
            "core.quotepath=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .current_dir(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.env(
        "GIT_SSH_COMMAND",
        "ssh -o BatchMode=yes -o ConnectTimeout=15",
    )
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().context("无法启动 Git，请安装 Git 后重试")?;
    fn collect(
        reader: impl Read + Send + 'static,
    ) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
        std::thread::spawn(move || {
            let mut r = reader;
            let mut result = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = r.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                if result.len() < 1024 * 1024 {
                    let keep = n.min(1024 * 1024 - result.len());
                    result.extend_from_slice(&buf[..keep]);
                }
            }
            Ok(result)
        })
    }
    let stdout = collect(child.stdout.take().unwrap());
    let stderr = collect(child.stderr.take().unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Git 操作超时，已终止；请重新查询同步状态");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    let output = stdout
        .join()
        .map_err(|_| anyhow::anyhow!("Git 输出线程中断"))??;
    let error = stderr
        .join()
        .map_err(|_| anyhow::anyhow!("Git 错误线程中断"))??;
    ensure!(
        status.success(),
        "Git 操作失败：{}",
        String::from_utf8_lossy(&error)
    );
    Ok(String::from_utf8_lossy(&output).into_owned())
}
pub fn sync(root: &Path, action: &str, data: &Value) -> Result<Value> {
    let remote = data["remote"].as_str().unwrap_or("origin");
    ensure!(
        !remote.starts_with('-')
            && remote
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-./".contains(c)),
        "remote 无效"
    );
    let branch = git(root, &["symbolic-ref", "--short", "HEAD"])?;
    let branch = branch.trim();
    let status = git(root, &["status", "--porcelain=v1"])?;
    if action == "status" {
        return Ok(
            json!({"branch":branch,"status":status,"conflicts":git(root,&["diff","--name-only","--diff-filter=U"])?}),
        );
    }
    if matches!(action, "fetch" | "pull" | "push") {
        ensure!(
            git(root, &["remote"])?.lines().any(|r| r == remote),
            "remote 必须是工作区已配置的远程名称"
        );
    }
    match action {
        "fetch" => {
            git(root, &["fetch", remote])?;
        }
        "pull" => {
            ensure!(status.is_empty(), "本地存在未提交更改，请先提交再同步");
            git(root, &["pull", "--no-rebase", "--no-edit", remote, branch])?;
        }
        "push" => {
            git(root, &["push", remote, branch])?;
        }
        "resolve" => {
            let raw = data["path"].as_str().context("缺少冲突 path")?;
            let path = root.join(raw).canonicalize()?;
            let root = root.canonicalize()?;
            ensure!(path.starts_with(&root), "冲突路径越界");
            let relative = path
                .strip_prefix(&root)?
                .to_string_lossy()
                .replace('\\', "/");
            let conflicts = git(&root, &["diff", "--name-only", "--diff-filter=U"])?;
            ensure!(
                conflicts.lines().any(|p| p == relative),
                "目标不是未解决的冲突"
            );
            match data["resolution"].as_str() {
                Some("ours") => {
                    git(&root, &["checkout", "--ours", "--", &relative])?;
                }
                Some("theirs") => {
                    git(&root, &["checkout", "--theirs", "--", &relative])?;
                }
                Some("content") => crate::files::FileService::new()
                    .write_file_safe(&path, data["content"].as_str().context("缺少 content")?)?,
                _ => anyhow::bail!("resolution 需要 ours/theirs/content"),
            };
            git(&root, &["add", "--", &relative])?;
        }
        _ => anyhow::bail!("未知同步操作"),
    }
    Ok(json!({"completed":true,"branch":branch,"status":git(root,&["status","--porcelain=v1"])?}))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "mochi-console-transfer-{}",
                crate::paths::random_base36(16)
            ));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn workspace_round_trip_preserves_empty_directories_and_refuses_overwrite() {
        let t = Temp::new();
        let root = t.0.join("source");
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("note.md"), "你好\n").unwrap();
        let archive = t.0.join("out.zip");
        export_workspace(&root, &archive).unwrap();
        assert!(export_workspace(&root, &archive).is_err());
        assert!(export_workspace(&root, &root.join("recursive.zip")).is_err());
        let dest = t.0.join("imported");
        import_workspace(&archive, &dest).unwrap();
        assert_eq!(
            std::fs::read_to_string(dest.join("note.md")).unwrap(),
            "你好\n"
        );
        assert!(dest.join("empty").is_dir());
        assert!(import_workspace(&archive, &dest).is_err());
    }
    #[test]
    fn archive_rejects_traversal_aliases_and_duplicates_without_creating_destination() {
        let t = Temp::new();
        for (i, names) in [
            vec!["../escape"],
            vec!["foo:ads"],
            vec!["CON.txt"],
            vec!["a.txt", "A.txt"],
        ]
        .iter()
        .enumerate()
        {
            let archive = t.0.join(format!("{i}.zip"));
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            for name in names {
                writer
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"content").unwrap();
            }
            writer.finish().unwrap();
            let dest = t.0.join(format!("dest{i}"));
            assert!(import_workspace(&archive, &dest).is_err());
            assert!(!dest.exists());
        }
    }
    #[test]
    fn installer_fingerprint_tracks_bytes() {
        let t = Temp::new();
        let p = t.0.join("binary");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            fingerprint(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn plugin_install_validates_then_preserves_existing_installation() {
        let t = Temp::new();
        let root = t.0.join("workspace");
        let project = t.0.join("plugin");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            project.join("manifest.json"),
            r#"{"manifestVersion":2,"id":"console-test","name":"Console test","version":"1.0.0"}"#,
        )
        .unwrap();
        let archive = t.0.join("plugin.zip");
        let svc = crate::plugins::PluginService::new(&root);
        svc.package_dir(&project, &archive).unwrap();
        assert_eq!(
            install_plugin(&root, &archive).unwrap()["manifest"]["id"],
            "console-test"
        );
        let installed = svc.root().join("console-test");
        std::fs::write(installed.join("keep.txt"), "user config").unwrap();
        assert!(install_plugin(&root, &archive).is_err());
        assert!(installed.join("keep.txt").exists());
        svc.set_enabled("console-test", false).unwrap();
        assert!(!svc.list().unwrap()[0].enabled);
        svc.set_enabled("console-test", true).unwrap();
        assert!(svc.list().unwrap()[0].enabled);
    }
    #[test]
    fn sync_reads_real_git_worktree_and_rejects_unknown_remote() {
        let t = Temp::new();
        let svc = crate::git::GitService::new(&t.0);
        svc.ensure_repository().unwrap();
        std::fs::write(t.0.join("note.md"), "uncommitted").unwrap();
        let state = sync(&t.0, "status", &json!({})).unwrap();
        assert!(state["status"].as_str().unwrap().contains("note.md"));
        assert!(sync(&t.0, "push", &json!({"remote":"missing-remote"}))
            .unwrap_err()
            .to_string()
            .contains("remote"));
    }
}
