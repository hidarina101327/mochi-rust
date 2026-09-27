//! 桌面卡片协调逻辑：有界 I/O 任务、独立窗口和事务式编辑器。
mod actions;
mod agent;
pub(super) mod folder_actions;
mod manager;
mod runtime;
#[cfg(test)]
mod tests;
mod utilities;
#[cfg(debug_assertions)]
mod verification;
mod widgets;

use super::*;
use crate::ui::desktop_cards as ui;
use crate::{desktop_tray, desktop_window};
use mochi_core::desktop_cards::{self as model, DesktopConfig, Interaction, Module};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(super) struct State {
    folder_watch: Option<model::folder_watch::FolderWatcher>,
    utility_job: Option<(u64, Receiver<std::result::Result<(), String>>)>,
    file_job: Option<folder_actions::FileJob>,
    organize_plan: Option<(
        String,
        String,
        model::folder::FolderConfig,
        model::folder_operations::OrganizePlan,
    )>,
    target_jobs: Vec<(String, Receiver<bool>)>,
    pub panel: Option<ui::State>,
    agent_reply: Option<std::sync::mpsc::Sender<std::result::Result<serde_json::Value, String>>>,
    editing_base: Option<DesktopConfig>,
    root: Option<PathBuf>,
    config: DesktopConfig,
    snapshot: model::Snapshot,
    windows: desktop_window::Manager,
    tray: desktop_tray::Tray,
    epoch: u64,
    revision: u64,
    loaded: bool,
    error: Option<String>,
    job: Option<Receiver<runtime::Output>>,
    job_writes: bool,
    close_after_jobs: bool,
    unsaved: bool,
    pending: VecDeque<runtime::Job>,
    last_refresh: Option<Instant>,
    refresh_needed: bool,
    save_at: Option<Instant>,
    pub(super) saving_editor: bool,
    recovery: bool,
    keep_editor_open: bool,
    pub paused: bool,
    pub exit_requested: bool,
    pub taskbar_message: u32,
    last_dark: bool,
    widget_second: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            folder_watch: None,
            utility_job: None,
            file_job: None,
            organize_plan: None,
            target_jobs: Vec::new(),
            panel: None,
            agent_reply: None,
            editing_base: None,
            root: None,
            config: DesktopConfig::default(),
            snapshot: Default::default(),
            windows: Default::default(),
            tray: Default::default(),
            epoch: 0,
            revision: 0,
            loaded: false,
            error: None,
            job: None,
            job_writes: false,
            close_after_jobs: false,
            unsaved: false,
            pending: VecDeque::new(),
            last_refresh: None,
            refresh_needed: false,
            save_at: None,
            saving_editor: false,
            recovery: false,
            keep_editor_open: false,
            paused: false,
            exit_requested: false,
            taskbar_message: 0,
            last_dark: false,
            widget_second: 0,
        }
    }
}

impl App {
    pub(super) fn import_desktop_package(
        &mut self,
        package: &mochi_core::transfer::Package,
    ) -> anyhow::Result<()> {
        if let Some(panel) = &mut self.desktop.panel {
            panel.commit_focused_field();
        }
        anyhow::ensure!(
            !self.desktop.panel.as_ref().is_some_and(|panel| panel.dirty)
                && self.desktop.job.is_none()
                && self.desktop.pending.is_empty()
                && !self.desktop.unsaved
                && !self.desktop.saving_editor,
            "桌面卡片正在编辑或保存，请结束后重试"
        );
        let root = self
            .shell
            .workspace()
            .ok_or_else(|| anyhow::anyhow!("请先打开工作区"))?
            .root
            .clone();
        package.install(&root)?;
        self.desktop.config = DesktopConfig::load(&root)?;
        self.desktop.loaded = true;
        self.desktop.revision += 1;
        if let Some(panel) = &mut self.desktop.panel {
            let embedded = panel.embedded;
            *panel = ui::State::new(&self.desktop.config);
            panel.embedded = embedded;
            self.desktop.editing_base = Some(self.desktop.config.clone());
        }
        self.desktop.snapshot = Default::default();
        self.desktop_sync_windows();
        self.desktop.refresh_needed = true;
        Ok(())
    }
    pub(super) fn desktop_refresh_theme(&self) {
        self.desktop.windows.refresh_theme(self.state.dark);
    }

    pub fn desktop_initialize(&mut self, hwnd: HWND) {
        if desktop_tray::isolated() {
            return;
        }
        self.desktop.taskbar_message = unsafe {
            windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(windows::core::w!(
                "TaskbarCreated"
            ))
        };
        if !self.desktop.tray.ensure(hwnd) {
            self.show_global_notice("Windows 托盘暂不可用，墨池将保留主窗口入口");
        }
        self.sync_login_startup(false);
        self.desktop_timer_state();
    }

    pub fn sync_login_startup(&mut self, manual: bool) {
        if cfg!(debug_assertions) || desktop_tray::isolated() {
            if manual {
                self.show_global_notice("开机启动偏好已保存；开发和验收构建不会注册启动项");
            }
            return;
        }
        let enabled = app_settings::descriptor("general.startAtLogin")
            .is_some_and(|d| matches!(self.app_settings.read(d), SettingValue::Bool(true)));
        match platform::autostart::sync(enabled) {
            Ok(platform::autostart::SyncResult::SkippedByWindows) => {
                if manual {
                    self.show_global_notice(
                        "Windows 已禁用墨池启动项，请在系统设置 → 应用 → 启动中重新启用",
                    );
                }
            }
            Ok(_) => {
                if manual {
                    self.show_global_notice(if enabled {
                        "已开启登录 Windows 时启动墨池"
                    } else {
                        "已关闭开机自启动"
                    });
                }
            }
            Err(e) => self.show_global_notice(format!("开机自启动未生效：{e}")),
        }
    }

    pub(super) fn desktop_workspace_changed(&mut self) {
        if let Some(reply) = self.desktop.agent_reply.take() {
            let _ = reply.send(Err("工作区已切换，请重新读取卡片".into()));
        }
        self.desktop.epoch = self.desktop.epoch.wrapping_add(1);
        self.desktop.folder_watch = None;
        self.desktop.organize_plan = None;
        self.desktop.pending.clear();
        self.desktop.root = self.shell.workspace().map(|w| w.root.clone());
        self.desktop.config = DesktopConfig::default();
        self.desktop.snapshot = Default::default();
        self.desktop.panel = None;
        self.desktop.editing_base = None;
        self.desktop.error = None;
        self.desktop.loaded = false;
        self.desktop.save_at = None;
        self.desktop.refresh_needed = false;
        self.desktop.last_refresh = None;
        self.desktop.saving_editor = false;
        self.desktop.recovery = false;
        self.desktop.revision = 0;
        self.desktop.unsaved = false;
        self.desktop_sync_windows();
        if let Some(root) = self.desktop.root.clone() {
            self.desktop_enqueue(runtime::Job::Load(root));
        }
    }

    pub fn desktop_tick(&mut self) {
        if self.desktop.file_job.is_none()
            && self
                .desktop
                .folder_watch
                .as_ref()
                .is_some_and(|w| w.take_changed())
        {
            self.desktop.refresh_needed = true;
        }
        self.desktop_take_utility_result();
        self.desktop_take_file_result();
        self.desktop_take_target_results();
        if self
            .desktop
            .panel
            .as_ref()
            .is_some_and(|p| p.editor_tab == 2)
        {
            self.invalidate_main();
        }
        self.desktop_take_results();
        let hwnd = HWND(self.hwnd_raw as *mut _);
        self.desktop.windows.arrange(hwnd);
        if self.desktop.save_at.is_some_and(|at| Instant::now() >= at)
            && self.desktop.loaded
            && !self.desktop.saving_editor
        {
            self.desktop.save_at = None;
            if let Some(root) = self.desktop.root.clone() {
                self.desktop_enqueue(runtime::Job::Save(
                    root,
                    self.desktop.config.clone(),
                    self.desktop.revision,
                    false,
                ));
            }
        }
        let enabled = self.desktop.config.cards.iter().any(|c| c.enabled) && !self.desktop.paused;
        let elapsed = self
            .desktop
            .last_refresh
            .map(|t| t.elapsed())
            .unwrap_or(Duration::from_secs(60));
        if self.desktop.loaded
            && enabled
            && self.desktop.job.is_none()
            && ((self.desktop.refresh_needed && elapsed >= Duration::from_secs(2))
                || elapsed
                    >= Duration::from_secs(
                        if self.desktop.config.cards.iter().any(|c| {
                            c.enabled
                                && c.active_page().is_some_and(|p| {
                                    p.module == Module::Music
                                        || (p.module == Module::Folder && p.folder.auto_organize)
                                })
                        }) {
                            5
                        } else {
                            30
                        },
                    ))
        {
            self.desktop_refresh();
        }
        self.desktop_widget_intervals();
        if enabled || self.desktop_update_live() || self.desktop.last_dark != self.state.dark {
            self.desktop_sync_windows();
        }
        self.desktop_timer_state();
    }

    pub fn desktop_mark_stale(&mut self) {
        self.desktop.refresh_needed = true;
        self.desktop_timer_state();
    }

    fn desktop_refresh(&mut self) {
        if self.desktop.exit_requested
            || !self.desktop.loaded
            || self.desktop.paused
            || !self.desktop.config.cards.iter().any(|c| c.enabled)
        {
            return;
        }
        if let Some(root) = self.desktop.root.clone() {
            self.desktop.refresh_needed = false;
            self.desktop.last_refresh = Some(Instant::now());
            self.desktop_enqueue(runtime::Job::Snapshot(root, self.desktop.config.clone()));
        }
    }

    fn desktop_config_changed(&mut self) {
        self.desktop.revision = self.desktop.revision.wrapping_add(1);
        self.desktop.unsaved = true;
        self.desktop.save_at = Some(Instant::now() + Duration::from_millis(700));
        self.desktop_sync_windows();
        self.desktop_timer_state();
    }

    fn desktop_sync_windows(&mut self) {
        self.desktop_update_live();
        if self.hwnd_raw == 0 || desktop_tray::isolated() {
            return;
        }
        let folders = self
            .desktop
            .config
            .cards
            .iter()
            .filter(|c| c.enabled && !self.desktop.paused)
            .filter_map(|c| c.active_page())
            .filter(|p| p.module == Module::Folder && !p.folder.path.is_empty())
            .map(|p| p.folder.clone())
            .collect::<Vec<_>>();
        if folders.is_empty() {
            self.desktop.folder_watch = None;
        } else {
            if self.desktop.folder_watch.is_none() {
                self.desktop.folder_watch = model::folder_watch::FolderWatcher::new().ok();
            }
            if let Some(watcher) = &mut self.desktop.folder_watch {
                watcher.set_folders(folders);
            }
        }
        let live = self.desktop_widget_live();
        let mut views = Vec::new();
        if !self.desktop.paused {
            for card in self.desktop.config.cards.iter().filter(|c| c.enabled) {
                let Some(page) = card.active_page() else {
                    continue;
                };
                let snap = self
                    .desktop
                    .snapshot
                    .pages
                    .get(&model::page_key(&card.id, &page.id));
                let view = desktop_window::View {
                    folder_selected: Default::default(),
                    workspace: self.desktop.root.clone().unwrap_or_default(),
                    tree_rows: vec![],
                    tree_loading: vec![],
                    item_styles: page.item_styles.clone(),
                    tab_scroll: None,
                    shortcut_insertion: None,
                    widget_offsets: Default::default(),
                    chat_offsets: Default::default(),
                    widget_rows: page
                        .studio
                        .nodes
                        .iter()
                        .filter_map(|n| {
                            self.desktop
                                .snapshot
                                .pages
                                .get(&format!(
                                    "{}:node:{}",
                                    model::page_key(&card.id, &page.id),
                                    n.id
                                ))
                                .map(|snap| {
                                    (
                                        n.id.clone(),
                                        snap.rows
                                            .iter()
                                            .map(|r| desktop_window::Row {
                                                id: r.id.clone(),
                                                title: r.title.clone(),
                                                detail: r.detail.clone(),
                                                checked: r.checked,
                                                meta: r.meta.clone(),
                                            })
                                            .collect(),
                                    )
                                })
                        })
                        .collect(),
                    studio: page.studio.clone(),
                    live: if matches!(page.module, Module::Ai | Module::Pomodoro | Module::Clock)
                        || page.studio.nodes.iter().any(|n| {
                            matches!(
                                n.kind,
                                model::studio::Kind::Clock
                                    | model::studio::Kind::Date
                                    | model::studio::Kind::Timer
                                    | model::studio::Kind::AiChat
                            )
                        }) {
                        live.clone()
                    } else {
                        Default::default()
                    },
                    module: Some(page.module),
                    presentation: page.presentation.clone(),
                    collapsed: Default::default(),
                    month_offset: 0,
                    page_id: page.id.clone(),
                    tabs: card
                        .pages
                        .iter()
                        .map(|p| (p.id.clone(), p.title.clone()))
                        .collect(),
                    subtitle: snap
                        .map(|s| s.subtitle.clone())
                        .unwrap_or_else(|| page.module.label().into()),
                    rows: snap
                        .map(|s| {
                            s.rows
                                .iter()
                                .map(|r| desktop_window::Row {
                                    id: r.id.clone(),
                                    title: r.title.clone(),
                                    detail: r.detail.clone(),
                                    meta: r.meta.clone(),
                                    checked: if page.interaction == Interaction::Smart
                                        && matches!(
                                            r.action,
                                            Some(model::Action::ToggleTask { .. })
                                        ) {
                                        r.checked
                                    } else {
                                        None
                                    },
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    empty: snap
                        .map(|s| s.empty_message.clone())
                        .unwrap_or_else(|| "正在读取当前分页…".into()),
                    error: self.desktop.error.is_some(),
                    pending: false,
                };
                let spec = desktop_window::Spec {
                    appearance: card.appearance.clone(),
                    id: card.id.clone(),
                    title: card.title.clone(),
                    x: card.x,
                    y: card.y,
                    width: card.width,
                    height: card.height,
                    locked: card.locked,
                    dark: self.state.dark,
                };
                views.push((spec, view));
            }
        }
        let errors = self
            .desktop
            .windows
            .sync(HWND(self.hwnd_raw as *mut _), views);
        if let Some(error) = errors.first() {
            self.desktop.error = Some(format!("卡片窗口创建失败：{error}"));
        }
        self.desktop.last_dark = self.state.dark;
    }

    /// 计时器状态保存在当前进程中；仅凭磁盘历史无法反映其实时状态。
    fn desktop_update_live(&mut self) -> bool {
        if self.desktop.paused {
            return false;
        }
        let timer = &self.panels.pomodoro;
        let title = if timer.running {
            "专注中"
        } else if timer.is_complete() {
            "本轮已完成"
        } else if timer.remaining < pomodoro::SECONDS {
            "已暂停"
        } else {
            "准备专注"
        };
        let detail = format!(
            "{} · 点击打开番茄钟",
            pomodoro::format_duration(timer.remaining_now())
        );
        let mut changed = false;
        for card in self.desktop.config.cards.iter().filter(|c| c.enabled) {
            let Some(page) = card
                .active_page()
                .filter(|p| p.module == Module::Pomodoro && p.selected("status"))
            else {
                continue;
            };
            let Some(snapshot) = self
                .desktop
                .snapshot
                .pages
                .get_mut(&model::page_key(&card.id, &page.id))
            else {
                continue;
            };
            if let Some(row) = snapshot.rows.iter_mut().find(|r| r.id == "pomodoro:status") {
                if row.title != title || row.detail != detail {
                    row.title = title.into();
                    row.detail = detail.clone();
                    changed = true;
                }
            }
        }
        changed
    }

    fn desktop_timer_state(&self) {
        if self.hwnd_raw == 0 || desktop_tray::isolated() {
            return;
        }
        let active = !self.desktop.windows.is_empty()
            || self.desktop.utility_job.is_some()
            || self.desktop.file_job.is_some()
            || self.desktop.panel.is_some()
            || self.desktop.job.is_some()
            || !self.desktop.pending.is_empty()
            || self.desktop.save_at.is_some();
        unsafe {
            let hwnd = HWND(self.hwnd_raw as *mut _);
            if active {
                windows::Win32::UI::WindowsAndMessaging::SetTimer(
                    Some(hwnd),
                    desktop_window::TIMER,
                    1000,
                    None,
                );
            } else {
                let _ = windows::Win32::UI::WindowsAndMessaging::KillTimer(
                    Some(hwnd),
                    desktop_window::TIMER,
                );
            }
        }
    }

    pub fn desktop_display_changed(&mut self) {
        self.desktop.windows.repair_positions();
        self.desktop_handle_events();
    }
    pub fn desktop_taskbar_recreated(&mut self) {
        if !desktop_tray::isolated() {
            self.desktop.tray.recover(HWND(self.hwnd_raw as *mut _));
            self.desktop.windows.arrange(HWND(self.hwnd_raw as *mut _));
        }
    }
    pub fn desktop_taskbar_message(&self) -> u32 {
        self.desktop.taskbar_message
    }
    pub fn desktop_tray_ready(&self) -> bool {
        self.desktop.tray.available()
    }
    pub fn desktop_paused(&self) -> bool {
        self.desktop.paused
    }

    pub fn desktop_close_to_tray(&mut self) -> bool {
        if self.desktop.exit_requested
            || !self.desktop.tray.available()
            || !self.desktop.config.cards.iter().any(|c| c.enabled)
        {
            return false;
        }
        if !self.commit_title() || !self.commit_table_cell() {
            return true;
        }
        if let Err(e) = self.shell.save_dirty_tabs() {
            self.show_global_notice(format!("保存失败，主窗口保持打开：{e}"));
            return true;
        }
        self.sync_document_format_changes();
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                HWND(self.hwnd_raw as *mut _),
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }
        true
    }

    pub fn desktop_flush_before_exit(&mut self) -> bool {
        if self.desktop.job_writes || !self.desktop.pending.is_empty() {
            self.desktop.exit_requested = true;
            self.desktop.close_after_jobs = true;
            self.show_global_notice("正在完成卡片文件操作，完成后将退出墨池");
            return false;
        }
        if !self.desktop.unsaved {
            return true;
        }
        if let Some(root) = &self.desktop.root {
            if let Err(e) = self.desktop.config.save(root) {
                self.show_global_notice(format!("桌面卡片布局保存失败：{e}"));
                return false;
            }
        }
        self.desktop.save_at = None;
        self.desktop.unsaved = false;
        true
    }

    pub(super) fn desktop_before_workspace_switch(&mut self) -> bool {
        if self.desktop.job_writes {
            self.show_global_notice("卡片布局正在保存，请稍后切换工作区");
            return false;
        }
        if self.desktop.unsaved {
            if let Some(root) = &self.desktop.root {
                if let Err(e) = self.desktop.config.save(root) {
                    self.show_global_notice(format!("卡片布局未保存，保留当前工作区：{e}"));
                    return false;
                }
            }
            self.desktop.unsaved = false;
            self.desktop.save_at = None;
        }
        true
    }
}
