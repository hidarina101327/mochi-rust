//! 管理控制台后台任务、执行结果和关闭前的收尾检查。
use super::*;
type JobResult = std::result::Result<Value, String>;
#[cfg(test)]
#[test]
fn background_jobs_prevent_close_until_their_result_is_collected() {
    let root = std::env::temp_dir().join(format!(
        "mochi-console-close-{}",
        mochi_core::paths::random_base36(16)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
        root.join("settings.json"),
    ))))
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    app.console_jobs.jobs.insert(
        "test".into(),
        Job {
            root,
            kind: "workspace_transfer".into(),
            rx,
            result: None,
        },
    );
    assert!(!app.console_jobs_ready_for_close());
    tx.send(Ok(json!({"done":true}))).unwrap();
    assert!(app.console_jobs_ready_for_close());
    assert!(app.console_jobs.jobs["test"].result.is_some());
}
struct Job {
    root: PathBuf,
    kind: String,
    rx: Receiver<JobResult>,
    result: Option<JobResult>,
}
#[derive(Default)]
pub(in crate::app) struct SystemState {
    jobs: HashMap<String, Job>,
    pub(super) reviewing: bool,
}
impl App {
    pub(in crate::app) fn console_poll_jobs(&mut self) {
        let mut finished = Vec::new();
        for (id, job) in &mut self.console_jobs.jobs {
            if job.result.is_none() {
                match job.rx.try_recv() {
                    Ok(r) => job.result = Some(r),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        job.result = Some(Err("后台任务已中断".into()))
                    }
                    Err(_) => {}
                }
                if job.result.is_some() {
                    finished.push((id.clone(), job.kind.clone()));
                }
            }
        }
        for (id, kind) in finished {
            let _ = self.console_job_status(&kind, &id);
        }
        // 任务运行期间，用户可能切换对话。等该对话再次成为当前对话时，
        // 要根据已保存的执行回执恢复状态。
        let pending: Vec<_> = self
            .ai
            .panel
            .active
            .as_ref()
            .into_iter()
            .flat_map(|c| &c.messages)
            .filter_map(|m| m.get("pendingConsoleAction"))
            .filter(|v| v["status"] == "running")
            .filter_map(|v| v["result"]["jobId"].as_str().map(str::to_owned))
            .collect();
        for id in pending {
            let result = match self.console_jobs.jobs.get(&id) {
                Some(job) if job.result.is_none() => continue,
                Some(job) => match job.result.as_ref().unwrap() {
                    Ok(value) => json!({"jobId":id,"status":"completed","result":value}),
                    Err(error) => json!({"jobId":id,"status":"failed","error":error}),
                },
                None => {
                    json!({"jobId":id,"status":"failed","error":"运行记录已不可用，请先检查实际状态再决定是否重试"})
                }
            };
            self.console_update_job_receipts(&id, &result);
        }
    }
    pub(in crate::app) fn console_jobs_ready_for_close(&mut self) -> bool {
        self.console_poll_jobs();
        if self
            .console_jobs
            .jobs
            .values()
            .any(|job| job.result.is_none())
        {
            self.state.status_text = "控制台后台任务仍在运行，请等待完成后再关闭应用".into();
            return false;
        }
        true
    }
    fn console_job(
        &mut self,
        kind: &str,
        f: impl FnOnce() -> CResult<Value> + Send + 'static,
    ) -> CResult<Value> {
        self.console_poll_jobs();
        ensure!(
            self.console_jobs.jobs.values().all(|j| j.result.is_some()),
            "已有控制台后台任务运行中，请先查询其状态再继续"
        );
        ensure!(
            self.console_jobs.jobs.len() < 128,
            "后台任务记录已满，请重启应用后继续"
        );
        let root = self.console_root()?;
        let id = mochi_core::paths::random_base36(16);
        let (tx, rx) = std::sync::mpsc::channel();
        let hwnd_raw = self.hwnd_raw;
        std::thread::Builder::new()
            .name(format!("console-{kind}"))
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                    .map_err(|_| "后台操作异常退出".to_owned())
                    .and_then(|r| r.map_err(|e| format!("{e:#}")));
                let _ = tx.send(result);
                if hwnd_raw != 0 {
                    unsafe {
                        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(HWND(hwnd_raw as *mut _)),
                            platform::WM_APP_FILES_CHANGED,
                            windows::Win32::Foundation::WPARAM(0),
                            windows::Win32::Foundation::LPARAM(0),
                        );
                    }
                }
            })?;
        self.console_jobs.jobs.insert(
            id.clone(),
            Job {
                root,
                kind: kind.into(),
                rx,
                result: None,
            },
        );
        Ok(json!({"jobId":id,"status":"running","pollAfterMs":1000}))
    }
    fn console_job_status(&mut self, kind: &str, id: &str) -> CResult<Value> {
        let root = self.console_root()?;
        let job = self
            .console_jobs
            .jobs
            .get_mut(id)
            .context("后台任务不存在")?;
        ensure!(
            job.root == root && job.kind == kind,
            "后台任务不属于当前工作区或工具"
        );
        if job.result.is_none() {
            match job.rx.try_recv() {
                Ok(r) => job.result = Some(r),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => job.result = Some(Err("后台任务已中断".into())),
            }
        }
        let result = match &job.result {
            None => json!({"jobId":id,"status":"running","pollAfterMs":1000}),
            Some(Ok(v)) => json!({"jobId":id,"status":"completed","result":v}),
            Some(Err(e)) => json!({"jobId":id,"status":"failed","error":e}),
        };
        if result["status"] == "completed" {
            self.shell.refresh_tree();
            self.home_dirty = true;
        }
        self.console_update_job_receipts(id, &result);
        Ok(result)
    }
    // 传输操作可以明确指定工作区外的路径，但仍须遵守
    // 与工作区内操作相同的动作和目录策略。
    pub(super) fn console_external(&self, path: &str, write: bool) -> CResult<PathBuf> {
        use mochi_core::ai::permission::AiToolAction;
        let p = PathBuf::from(path);
        ensure!(p.is_absolute(), "需要绝对路径");
        let p = if p.exists() {
            p.canonicalize()?
        } else {
            p.parent()
                .context("缺少父目录")?
                .canonicalize()?
                .join(p.file_name().context("缺少文件名")?)
        };
        let perms = self.ai.permissions.as_ref().context("权限服务不可用")?;
        let normalized = p.to_string_lossy().replace('\\', "/");
        let s = if let Some(rest) = normalized.strip_prefix("//?/UNC/") {
            format!("//{rest}")
        } else {
            normalized
                .strip_prefix("//?/")
                .unwrap_or(&normalized)
                .to_owned()
        };
        perms
            .assert_tool_action_allowed(
                if write {
                    AiToolAction::WriteFile
                } else {
                    AiToolAction::ReadFile
                },
                Some(&s),
            )
            .map_err(|e| anyhow::anyhow!(e.message))?;
        if write && perms.path_requires_proposal(&s) && !self.console_automatic() {
            return Err(ApprovalRequired.into());
        }
        Ok(p)
    }
    pub(super) fn console_system(
        &mut self,
        hwnd: HWND,
        name: &str,
        action: &str,
        a: &Value,
    ) -> CResult<Value> {
        let root = self.console_root()?;
        let d = data(a);
        if action == "status" && a["id"].is_string() {
            return self.console_job_status(name, text(a, "id")?);
        }
        match name {
            "plugins_manage" => {
                let svc = mochi_core::plugins::PluginService::new(&root);
                self.console_scope(&svc.root().to_string_lossy(), action != "list")?;
                if action == "list" {
                    return Ok(json!(svc.list()?.iter().map(|p|json!({"manifest":p.manifest,"path":p.directory,"enabled":p.enabled})).collect::<Vec<_>>()));
                }
                if matches!(action, "enable" | "disable" | "uninstall") {
                    let id = text(a, "id")?;
                    ensure!(
                        svc.list()?.iter().any(|p| p.manifest.id == id),
                        "插件不存在"
                    );
                    if action == "uninstall" {
                        svc.uninstall(id)?;
                    } else {
                        svc.set_enabled(id, action == "enable")?;
                    }
                    return Ok(json!({"applied":true}));
                }
                let path = self.console_target(a, false)?;
                if action == "install" {
                    return self.console_job(name, move || {
                        mochi_core::console_system::install_plugin(&root, &path)
                    });
                }
                ensure!(action == "package", "未知插件操作");
                let dest = self.console_external(text(d, "destination")?, true)?;
                ensure!(!dest.exists(), "目标已存在");
                ensure!(
                    !dest.starts_with(path.canonicalize()?),
                    "打包目标不能位于插件目录内"
                );
                self.console_job(name, move || {
                    svc.package_dir(path, &dest)?;
                    Ok(json!({"path":dest}))
                })
            }
            "mapped_folders_manage" => {
                let svc = mochi_core::mapped_folders::Service::new(&root);
                self.console_path(
                    &mochi_core::paths::mapped_folders_file(&root).to_string_lossy(),
                    action != "list",
                )?;
                let mut config = svc.load()?;
                if action == "list" {
                    return Ok(versioned(json!(config)));
                }
                check_revision(a, &json!(config))?;
                match action {
                    "add" => {
                        let lib = self.console_path(text(d, "libraryPath")?, false)?;
                        ensure!(lib.is_dir(), "知识库不存在");
                        let parent = if let Some(p) = d["parentPath"].as_str() {
                            self.console_path(p, false)?
                        } else {
                            lib.clone()
                        };
                        let source = self.console_external(text(d, "source")?, false)?;
                        let rules = d
                            .get("rules")
                            .cloned()
                            .map(serde_json::from_value)
                            .transpose()?
                            .unwrap_or_default();
                        svc.add_at(&lib, &parent, &source, text(d, "name")?, rules)?;
                    }
                    "delete" => {
                        let id = text(a, "id")?;
                        ensure!(config.folders.iter().any(|f| f.id == id), "映射不存在");
                        config.folders.retain(|f| f.id != id);
                        svc.save(&config)?;
                    }
                    "update" => {
                        let id = text(a, "id")?;
                        let f = config
                            .folders
                            .iter_mut()
                            .find(|f| f.id == id)
                            .context("映射不存在")?;
                        if let Some(n) = d["name"].as_str() {
                            ensure!(
                                !n.trim().is_empty() && !n.contains(['/', '\\', '\0']),
                                "名称无效"
                            );
                            f.name = n.trim().into();
                        }
                        if let Some(r) = d.get("rules") {
                            f.rules = serde_json::from_value(r.clone())?;
                        }
                        if let Some(s) = d["source"].as_str() {
                            let p = self.console_external(s, false)?;
                            ensure!(
                                p.is_dir() && !f.parent().canonicalize()?.starts_with(&p),
                                "映射来源无效"
                            );
                            f.source = p.to_string_lossy().into_owned();
                        }
                        for (i, f) in config.folders.iter().enumerate() {
                            ensure!(
                                !config.folders[..i].iter().any(|o| o.library_path
                                    == f.library_path
                                    && (o.source == f.source
                                        || (o.parent() == f.parent()
                                            && o.name.eq_ignore_ascii_case(&f.name)))),
                                "映射名称或来源重复"
                            );
                        }
                        svc.save(&config)?;
                    }
                    _ => bail!("未知映射操作"),
                }
                self.shell.refresh_tree();
                Ok(versioned(json!(svc.load()?)))
            }
            "workspace_transfer" => match action {
                "inspect" => {
                    let p = self.console_external(text(a, "path")?, false)?;
                    self.console_job(name, move || {
                        mochi_core::console_system::inspect_archive(&p)
                    })
                }
                "export" => {
                    ensure!(
                        !self.shell.tabs().iter().any(|t| t.dirty()),
                        "请先保存文档后导出"
                    );
                    self.console_scope(&root.to_string_lossy(), false)?;
                    let p = self.console_external(text(a, "path")?, true)?;
                    self.console_job(name, move || {
                        mochi_core::console_system::export_workspace(&root, &p)
                    })
                }
                "import" => {
                    let p = self.console_external(text(a, "path")?, false)?;
                    let dest = self.console_external(text(d, "destination")?, true)?;
                    self.console_job(name, move || {
                        mochi_core::console_system::import_workspace(&p, &dest)
                    })
                }
                _ => bail!("未知工作区传输操作"),
            },
            "sync_manage" => {
                self.console_scope(&root.to_string_lossy(), action != "status")?;
                ensure!(
                    !matches!(action, "pull" | "resolve")
                        || !self.shell.tabs().iter().any(|t| t.dirty()),
                    "请先保存已打开文档后同步"
                );
                if action == "resolve" {
                    self.console_path(text(d, "path")?, true)?;
                }
                let action = action.to_owned();
                let d = d.clone();
                self.console_job(name, move || {
                    mochi_core::console_system::sync(&root, &action, &d)
                })
            }
            "marketplace_manage" => {
                // 每次安装都使用经过身份验证的最新目录；绝不接受模型输出中的网址或哈希值。
                if action == "install" {
                    self.console_scope(&root.to_string_lossy(), true)?;
                }
                let id = a["id"].as_str().map(str::to_owned);
                let install = action == "install";
                ensure!(
                    matches!(action, "list" | "refresh" | "install"),
                    "未知市场操作"
                );
                self.console_job(name, move || {
                    let c = mochi_core::marketplace::fetch_catalog()?;
                    if !install {
                        return Ok(json!({"release":c.release,"packages":c.packages}));
                    }
                    let p = c
                        .packages
                        .iter()
                        .find(|p| Some(p.id.as_str()) == id.as_deref())
                        .context("市场条目不存在")?;
                    let temp = std::env::temp_dir().join(format!(
                        "mochi-market-{}.zip",
                        mochi_core::paths::random_base36(16)
                    ));
                    let r = (|| {
                        mochi_core::marketplace::download(p, &temp)?;
                        let path = mochi_core::marketplace::install_archive(p, &temp, &root)?;
                        Ok(json!({"installed":true,"path":path,"package":p}))
                    })();
                    let _ = std::fs::remove_file(temp);
                    r
                })
            }
            "updates_manage" => match action {
                "get" => Ok(
                    json!({"currentVersion":env!("CARGO_PKG_VERSION"),"available":self.update_available.as_ref().map(|u|json!({"version":u.latest_version,"notes":u.notes})),"checking":self.update_rx.is_some(),"downloading":self.update_download_rx.is_some()}),
                ),
                "check" | "download" => {
                    let download = action == "download";
                    self.console_job(name,move||{let update=mochi_core::updates::check_latest_release(env!("CARGO_PKG_VERSION")).map_err(anyhow::Error::msg)?;let Some(u)=update else{return Ok(json!({"available":false}));};if download{let path=mochi_core::updates::download_installer(&u).map_err(anyhow::Error::msg)?;let fingerprint=mochi_core::console_system::fingerprint(&path)?;Ok(json!({"available":true,"version":u.latest_version,"path":path,"fingerprint":fingerprint}))}else{Ok(json!({"available":true,"version":u.latest_version,"notes":u.notes,"url":u.url}))}})
                }
                "install" => {
                    let result = self.console_job_status(name, text(a, "id")?)?;
                    ensure!(result["status"] == "completed", "下载尚未完成");
                    let p = PathBuf::from(text(&result["result"], "path")?);
                    ensure!(
                        mochi_core::console_system::fingerprint(&p)?
                            == text(&result["result"], "fingerprint")?,
                        "安装包下载后发生变化，请重新下载"
                    );
                    ensure!(self.prepare_close(), "当前内容保存失败，不能安装更新");
                    ensure!(!hwnd.is_invalid(), "应用窗口不可用");
                    launch_installer_after_exit(&p).map_err(anyhow::Error::msg)?;
                    self.desktop.exit_requested = true;
                    unsafe {
                        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                            Some(hwnd),
                            windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                            windows::Win32::Foundation::WPARAM(0),
                            windows::Win32::Foundation::LPARAM(0),
                        )?;
                    }
                    Ok(json!({"installerStarted":true,"exiting":true}))
                }
                _ => bail!("未知更新操作"),
            },
            _ => bail!("未知控制台工具：{name}"),
        }
    }
}
