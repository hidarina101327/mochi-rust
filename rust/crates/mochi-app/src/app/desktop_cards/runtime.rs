//! 在后台执行桌面卡片操作，并收集任务结果。
use super::*;

pub(super) enum Job {
    Load(PathBuf),
    Snapshot(PathBuf, DesktopConfig),
    Save(PathBuf, DesktopConfig, u64, bool),
    Recover(PathBuf, DesktopConfig, u64),
    Import(PathBuf, DesktopConfig),
    ImportJson(String, DesktopConfig),
    Export(PathBuf, DesktopConfig),
}
pub(super) enum Done {
    Loaded(DesktopConfig),
    Snapshot(model::Snapshot),
    Saved(DesktopConfig, u64, bool),
    Imported(DesktopConfig),
    Exported,
}
pub(super) struct Output {
    epoch: u64,
    result: std::result::Result<Done, String>,
}
#[cfg(test)]
impl Output {
    pub(super) fn saved_for_test(epoch: u64, config: DesktopConfig) -> Self {
        Self {
            epoch,
            result: Ok(Done::Saved(config, 1, true)),
        }
    }
    pub(super) fn loaded_for_test(epoch: u64, config: DesktopConfig) -> Self {
        Self {
            epoch,
            result: Ok(Done::Loaded(config)),
        }
    }
}

fn execute(job: Job) -> anyhow::Result<Done> {
    match job {
        Job::Load(root) => Ok(Done::Loaded(DesktopConfig::load(root)?)),
        Job::Snapshot(root, config) => {
            Ok(Done::Snapshot(super::utilities::snapshot(&root, &config)?))
        }
        Job::Save(root, config, revision, manual) => {
            config.save(root)?;
            Ok(Done::Saved(config, revision, manual))
        }
        Job::Recover(root, config, revision) => {
            let path = DesktopConfig::path(&root);
            let mut saved_backup = None;
            if path.exists() {
                let backup =
                    path.with_file_name(format!("desktop-cards.damaged-{}.json", uuid_v4()));
                std::fs::rename(&path, &backup)?;
                saved_backup = Some(backup);
            }
            if let Err(error) = config.save(root) {
                if let Some(backup) = saved_backup {
                    let _ = std::fs::rename(backup, path);
                }
                return Err(error);
            }
            Ok(Done::Saved(config, revision, true))
        }
        Job::Import(path, mut config) => {
            let text = mochi_core::transfer::read_content(
                &path,
                mochi_core::transfer::Kind::DesktopCards,
            )?;
            config.merge_import(&text)?;
            Ok(Done::Imported(config))
        }
        Job::ImportJson(text, mut config) => {
            config.merge_import(&text)?;
            Ok(Done::Imported(config))
        }
        Job::Export(path, config) => {
            mochi_core::transfer::Package {
                kind: mochi_core::transfer::Kind::DesktopCards,
                name: if config.cards.len() == 1 {
                    config.cards[0].title.clone()
                } else {
                    "桌面卡片布局".into()
                },
                content: config.export_json()?,
            }
            .write(&path)?;
            Ok(Done::Exported)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_json_round_trip_preserves_existing_cards() {
        let mut config = DesktopConfig::new();
        config
            .cards
            .push(model::Card::new("待复制的卡片", Module::Home));
        let original = config.cards[0].clone();
        let text = config.export_json().unwrap();
        let Done::Imported(imported) = execute(Job::ImportJson(text, config)).unwrap() else {
            panic!("expected imported draft");
        };
        assert_eq!(imported.cards.len(), 2);
        assert_eq!(imported.cards[0], original);
        assert_eq!(imported.cards[1].title, original.title);
        assert_ne!(imported.cards[1].id, original.id);
        assert!(!imported.cards[1].enabled);
    }

    #[test]
    fn pasted_json_rejects_invalid_content() {
        for text in ["{broken", "", r#"{"version":999,"cards":[]}"#] {
            assert!(execute(Job::ImportJson(text.into(), DesktopConfig::new())).is_err());
        }
    }

    #[test]
    fn desktop_recovery_keeps_damaged_layout_before_replacement() {
        let root = std::env::temp_dir().join(format!("mochi-desktop-recovery-{}", uuid_v4()));
        let path = DesktopConfig::path(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"broken original layout").unwrap();
        let config = DesktopConfig::default();
        assert!(matches!(
            execute(Job::Recover(root.clone(), config, 1)).unwrap(),
            Done::Saved(_, 1, true)
        ));
        assert!(DesktopConfig::load(&root).is_ok());
        let backup = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("damaged-")
            })
            .unwrap();
        assert_eq!(std::fs::read(backup).unwrap(), b"broken original layout");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_recovery_restores_original_when_new_layout_is_invalid() {
        let root = std::env::temp_dir().join(format!("mochi-desktop-recovery-{}", uuid_v4()));
        let path = DesktopConfig::path(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"keep original").unwrap();
        let mut config = DesktopConfig::default();
        config.version = 999;
        assert!(execute(Job::Recover(root.clone(), config, 1)).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"keep original");
        std::fs::remove_dir_all(root).unwrap();
    }
}

impl App {
    pub(super) fn desktop_enqueue(&mut self, job: Job) {
        if matches!(job, Job::Snapshot(..))
            && (self.desktop.job.is_some() || !self.desktop.pending.is_empty())
        {
            self.desktop.refresh_needed = true;
            return;
        }
        // 自动保存窗口位置时会合并连续变动，用户明确发起的任务则按顺序执行。
        if matches!(job, Job::Save(_, _, _, false)) {
            self.desktop
                .pending
                .retain(|j| !matches!(j, Job::Save(_, _, _, false)));
        }
        if self.desktop.pending.len() >= 8 {
            self.desktop_error("操作仍在处理中，请稍后再试".into());
            return;
        }
        self.desktop.pending.push_back(job);
        self.desktop_start_job();
        self.desktop_timer_state();
    }

    fn desktop_start_job(&mut self) {
        if self.desktop.job.is_some() {
            return;
        }
        let Some(job) = self.desktop.pending.pop_front() else {
            return;
        };
        self.desktop.job_writes = matches!(job, Job::Save(..) | Job::Recover(..) | Job::Export(..));
        let epoch = self.desktop.epoch;
        let owner = self.hwnd_raw;
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("mochi-desktop-io".into())
            .spawn(move || {
                let result = execute(job).map_err(|e| e.to_string());
                if tx.send(Output { epoch, result }).is_ok() && owner != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(owner as *mut _)),
                            desktop_window::READY,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            });
        match spawned {
            Ok(_) => self.desktop.job = Some(rx),
            Err(e) => self.desktop_error(format!("无法启动卡片后台任务：{e}")),
        }
    }

    pub fn desktop_take_results(&mut self) {
        let Some(rx) = &self.desktop.job else {
            return;
        };
        let output = match rx.try_recv() {
            Ok(value) => value,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Output {
                epoch: self.desktop.epoch,
                result: Err("卡片后台任务已中断，请重新打开管理器".into()),
            },
        };
        self.desktop.job = None;
        self.desktop.job_writes = false;
        if output.epoch == self.desktop.epoch {
            match output.result {
                Ok(Done::Loaded(config)) => {
                    self.desktop.config = config;
                    self.desktop.loaded = true;
                    self.desktop.error = None;
                    if self.desktop.panel.is_some() {
                        let embedded = self.desktop.panel.as_ref().is_some_and(|p| p.embedded);
                        let mut panel = ui::State::new(&self.desktop.config);
                        panel.embedded = embedded;
                        self.desktop.panel = Some(panel);
                        self.desktop.editing_base = Some(self.desktop.config.clone());
                    }
                    self.desktop_sync_windows();
                    self.desktop_refresh();
                }
                Ok(Done::Snapshot(snapshot)) => {
                    self.desktop.snapshot = snapshot;
                    self.desktop.error = None;
                    self.desktop_sync_windows();
                }
                Ok(Done::Saved(config, revision, manual)) => {
                    if self.desktop.revision == revision {
                        self.desktop.unsaved = false;
                    }
                    if manual {
                        self.desktop.config = config;
                        self.desktop.revision = revision;
                        self.desktop.recovery = false;
                        self.desktop.unsaved = false;
                        self.desktop.save_at = None;
                        if self.desktop.keep_editor_open {
                            self.desktop.editing_base = Some(self.desktop.config.clone());
                            if let Some(panel) = self.desktop.panel.as_mut() {
                                panel.config = self.desktop.config.clone();
                                panel.dirty = false;
                                panel.error = None;
                            }
                        } else {
                            self.desktop.panel = None;
                        }
                        self.desktop.keep_editor_open = false;
                        self.desktop.saving_editor = false;
                        if self.desktop.panel.is_none() || self.desktop_manager_active() {
                            self.focus = if self.desktop.panel.as_ref().is_some_and(|p| !p.embedded)
                            {
                                Focus::Dialog
                            } else {
                                Focus::Main
                            };
                        }
                        self.desktop.snapshot = Default::default();
                        self.desktop_sync_windows();
                        self.desktop_refresh();
                        if let Some(reply) = self.desktop.agent_reply.take() {
                            let _ = reply.send(Ok(mochi_core::ai::tools::desktop_tools::snapshot(
                                &self.desktop.config,
                            )));
                        }
                    }
                }
                Ok(Done::Imported(config)) => {
                    self.desktop.recovery |= !self.desktop.loaded;
                    self.desktop.loaded = true;
                    self.desktop.saving_editor = false;
                    let embedded = self.desktop.panel.as_ref().is_some_and(|p| p.embedded);
                    let mut panel = ui::State::new(&config);
                    panel.embedded = embedded;
                    self.desktop.panel = Some(panel);
                    if let Some(p) = self.desktop.panel.as_mut() {
                        p.dirty = true;
                        p.error=Some("已导入到草稿。为避免突然打开多个窗口，导入的卡片默认隐藏；开启后点击应用。".into());
                    }
                }
                Ok(Done::Exported) => {
                    self.show_global_notice("卡片布局已导出，未包含笔记正文或账户信息")
                }
                Err(e) => self.desktop_error(e),
            }
        }
        self.desktop_start_job();
        self.desktop_timer_state();
        if self.desktop.close_after_jobs
            && self.desktop.job.is_none()
            && self.desktop.pending.is_empty()
        {
            self.desktop.close_after_jobs = false;
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd_raw as *mut _)),
                    windows::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    }

    pub(in crate::app) fn desktop_error(&mut self, error: String) {
        if let Some(reply) = self.desktop.agent_reply.take() {
            let _ = reply.send(Err(error.clone()));
        }
        self.desktop.saving_editor = false;
        self.desktop.error = Some(error.clone());
        if let Some(panel) = self.desktop.panel.as_mut() {
            panel.error = Some(error);
        } else {
            self.show_global_notice(format!("桌面卡片：{error}"));
        }
    }
}
