//! 旧 HTML/JS 插件由用户选择的 Electron 版承载，原生主进程不加载 WebView。
use std::{
    path::{Path, PathBuf},
    process::Command,
};
pub fn repository() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .ancestors()
        .find(|p| p.join("package.json").is_file() && p.join("electron").is_dir())
        .map(Path::to_path_buf)
}
pub fn extensions(executable: Option<&Path>) -> Option<PathBuf> {
    executable
        .and_then(Path::parent)
        .map(|p| p.join("extensions"))
        .or_else(|| repository().map(|r| r.join("extensions")))
}
pub fn launch(executable: Option<&Path>, workspace: Option<&Path>) -> anyhow::Result<()> {
    let mut command = if let Some(exe) = executable {
        Command::new(exe)
    } else {
        let root = repository().ok_or_else(|| anyhow::anyhow!("请选择保留的 Electron 版程序"))?;
        let exe = root.join("node_modules/electron/dist/electron.exe");
        if !exe.is_file()
            || !root.join("dist-electron/main.js").is_file()
            || !root.join("dist/index.html").is_file()
        {
            anyhow::bail!("Electron 版尚未构建，请选择已安装的 Electron 版程序")
        }
        let mut cmd = Command::new(exe);
        cmd.arg(&root).current_dir(root);
        cmd
    };
    command.env_remove("ELECTRON_RUN_AS_NODE");
    if let Some(path) = workspace {
        command.arg(format!("--mochi-workspace={}", path.display()));
    }
    command.arg("--mochi-open-plugins");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()?;
    Ok(())
}
