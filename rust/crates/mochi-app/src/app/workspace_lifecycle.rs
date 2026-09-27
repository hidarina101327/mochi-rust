//! 管理工作区打开、恢复、监视和更新检查。
use super::*;

impl App {
    pub fn install_capture(&mut self, hwnd: HWND) {
        self.capture = crate::capture_window::Handle::new(hwnd).ok();
        if std::env::var_os("MOCHI_VERIFY_OFFSCREEN").is_some() {
            return;
        }
        let key = crate::ui::shortcuts::binding("Ctrl+Shift+Space");
        if let Ok(chord) = crate::ui::shortcuts::Chord::parse(&key) {
            if let Err(error) = self.register_capture_key(chord) {
                if !crate::capture_window::mark_hotkey_notice() {
                    return;
                }
                self.state.status_text = format!(
                    "快速窗口快捷键 {key} 注册失败：{error}。可从收件箱打开或在设置中更换快捷键"
                );
            }
        }
    }

    pub(super) fn register_capture_key(&self, chord: crate::ui::shortcuts::Chord) -> Result<()> {
        crate::capture_window::register_hotkey(self.hwnd_raw, chord)
    }

    /// 上次工作区记忆恢复。打不开就静默留在欢迎态，不要因为路径没了就启动失败。
    ///
    /// 必须在窗口建好之后调用——文件监听要拿 HWND 往 UI 线程投消息。
    pub fn restore_last_workspace(&mut self, hwnd: HWND) {
        self.hwnd_raw = hwnd.0 as isize;
        self.load_chrome_settings();
        self.file_icons = self
            .settings
            .keys()
            .into_iter()
            .filter_map(|key| {
                let path = key.strip_prefix("file.icon:")?;
                Some((PathBuf::from(path), self.settings.get(&key)?))
            })
            .collect();
        if let Some(last) = self.settings.get("workspace.lastPath") {
            let _ = self.open_workspace(hwnd, PathBuf::from(last), false);
        }
        if let Err(error) = mochi_core::web_clipper::native::serve(Arc::clone(&self.web_clipper)) {
            self.state.status_text = format!("网页剪藏接收端启动失败：{error}");
        }
        if !crate::desktop_tray::isolated() {
            if let Err(error) = mochi_core::web_clipper::native::register(&self.settings) {
                self.state.status_text = format!("网页剪藏注册失败：{error}");
            }
        }
    }

    /// 在后台检查公开 GitHub Releases。启动检查保持静默；设置页手动检查会把结果
    /// 写入状态栏，并在发现新版本时打开原生对话框。
    pub fn start_update_check(&mut self, manual: bool) {
        if self.update_rx.is_some() {
            if manual {
                self.show_global_notice("正在检查 Rust 原生版更新…");
            }
            return;
        }
        let (tx, rx) = channel();
        self.update_rx = Some(rx);
        self.update_check_manual = manual;
        if manual {
            self.show_global_notice("正在检查 Rust 原生版更新…");
        }
        let hwnd_raw = self.hwnd_raw;
        let spawned = std::thread::Builder::new()
            .name("mochi-update-check".into())
            .spawn(move || {
                let result = mochi_core::updates::check_latest_release(env!("CARGO_PKG_VERSION"));
                if tx.send(result).is_ok() && hwnd_raw != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd_raw as *mut _)),
                            platform::WM_APP_UPDATE_READY,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            });
        if spawned.is_err() {
            self.update_rx = None;
            self.update_check_manual = false;
            if manual {
                self.show_global_notice("无法启动更新检查线程");
            }
        }
    }

    /// 消费后台更新检查结果。只由窗口消息线程调用。
    pub fn take_update_result(&mut self) {
        let Some(rx) = self.update_rx.as_ref() else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_rx = None;
                if self.update_check_manual {
                    self.update_check_manual = false;
                    self.show_global_notice("更新检查线程异常退出");
                }
                return;
            }
        };
        self.update_rx = None;
        let manual = std::mem::take(&mut self.update_check_manual);
        match result {
            Ok(Some(update)) => {
                let skipped = self.settings.get("updates.skippedVersion");
                if !manual && skipped.as_deref() == Some(update.latest_version.as_str()) {
                    return;
                }
                let version = update.latest_version.clone();
                self.show_update_dialog(update);
                if manual {
                    self.show_global_notice(format!("发现 Rust 原生版新版本 v{version}"));
                } else {
                    self.publish_notification(
                        crate::ui::notifications::Category::System,
                        "发现新版本",
                        &format!("v{version} 已可用，可在设置中查看更新。"),
                    );
                }
            }
            Ok(None) => {
                if manual {
                    self.show_global_notice(format!(
                        "当前已是最新版本 v{}，或暂时没有可用 Release",
                        env!("CARGO_PKG_VERSION")
                    ));
                }
            }
            Err(error) => {
                if manual {
                    self.show_global_notice(format!("检查更新失败：{error}"));
                }
            }
        }
    }

    pub(super) fn show_update_dialog(&mut self, update: mochi_core::updates::UpdateInfo) {
        let latest = update.latest_version.clone();
        let description = if update.release_name.trim().is_empty() {
            format!("发现 Rust 原生版新版本 v{latest}")
        } else {
            format!("{} · v{latest}", update.release_name.trim())
        };
        let note = format!(
            "当前版本：v{}  \n最新版本：v{}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            latest,
            update_notes_for_dialog(&update.notes)
        );
        self.update_available = Some(update);
        self.dialog = Some(Dialog {
            title: "发现新版本".into(),
            description,
            field: None,
            error: String::new(),
            note: Some(crate::ui::widgets::DialogNote::Markdown {
                source: note,
                scroll: 0.0,
            }),
            buttons: vec![
                DialogButton {
                    label: "忽略此版本".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::SkipAppUpdate,
                },
                DialogButton {
                    label: "下载并安装".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::DownloadAppUpdate,
                },
            ],
            dismiss: DialogAction::DismissAppUpdate,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    /// 下载经过 GitHub Release 摘要校验的安装包。下载完成后，由一个临时 PowerShell
    /// 助手等待当前进程退出、运行安装包并重新启动已安装的原生程序。
    pub(super) fn start_update_download(&mut self) {
        if self.update_download_rx.is_some() {
            self.show_global_notice("原生安装包正在下载…");
            return;
        }
        let Some(update) = self.update_available.clone() else {
            self.show_global_notice("没有可下载的更新");
            return;
        };
        let (tx, rx) = channel();
        self.update_download_rx = Some(rx);
        self.show_global_notice(format!("正在下载 Rust 原生版 v{}…", update.latest_version));
        let hwnd_raw = self.hwnd_raw;
        let spawned = std::thread::Builder::new()
            .name("mochi-update-download".into())
            .spawn(move || {
                let result = mochi_core::updates::download_installer(&update);
                if tx.send(result).is_ok() && hwnd_raw != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd_raw as *mut _)),
                            platform::WM_APP_UPDATE_DOWNLOAD_READY,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            });
        if spawned.is_err() {
            self.update_download_rx = None;
            self.show_global_notice("无法启动原生安装包下载线程");
        }
    }

    /// 消费下载结果；只有已成功保存所有内容时，才退出交给安装助手。
    pub fn take_update_download_result(&mut self) {
        let Some(rx) = self.update_download_rx.as_ref() else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_download_rx = None;
                self.show_global_notice("原生安装包下载线程异常退出");
                return;
            }
        };
        self.update_download_rx = None;
        match result {
            Ok(installer) => {
                if !self.prepare_close() {
                    self.show_global_notice("安装包已下载，但当前内容保存失败；请保存后重试更新");
                    return;
                }
                match launch_installer_after_exit(&installer) {
                    Ok(()) => {
                        self.update_available = None;
                        self.close_dialog();
                        if self.hwnd_raw != 0 {
                            unsafe {
                                let _ = DestroyWindow(HWND(self.hwnd_raw as *mut _));
                            }
                        }
                    }
                    Err(error) => self.show_global_notice(format!("无法启动更新安装助手：{error}")),
                }
            }
            Err(error) => self.show_global_notice(format!("下载或校验原生安装包失败：{error}")),
        }
    }

    pub(super) fn open_workspace(
        &mut self,
        hwnd: HWND,
        path: PathBuf,
        remember: bool,
    ) -> Result<()> {
        if !self.desktop_before_workspace_switch() {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_ABORT,
                "卡片布局尚未保存",
            ));
        }
        self.shell.save_dirty_tabs().map_err(|error| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                format!("切换工作区前保存失败：{error}"),
            )
        })?;
        self.sync_document_format_changes();
        self.export_form = None;
        self.template_picker = None;
        self.finish_object_picker(None);
        self.close_global_import();
        self.close_search();
        self.ai.memory_jobs.clear();
        // 部分回复必须在旧工作区/旧会话仍有效时保存，不能到 setup_ai 后再收尾。
        self.ai_cancel();
        self.editor_ai.invalidate();
        // HWND 不是 Send，跨线程只能搬裸指针值再重建
        let hwnd_raw = hwnd.0 as isize;
        let result = self.shell.open_workspace(&path, move || unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd_raw as *mut _)),
                platform::WM_APP_FILES_CHANGED,
                WPARAM(0),
                LPARAM(0),
            );
        });
        if result.is_ok() {
            *self
                .web_clipper
                .workspace
                .lock()
                .unwrap_or_else(|e| e.into_inner()) =
                self.shell.workspace().map(|w| w.root.clone());
            if remember {
                self.settings
                    .set("workspace.lastPath", &path.to_string_lossy());
                let _ = self.settings.flush();
            }
            self.nav = NavState::default();
            self.side = SidebarState::default();
            self.split = split::State::default();
            self.agent = AgentConfigState::default();
            self.sched = ScheduleState::default();
            self.views = ViewsState::default();
            self.links = backlinks::State::default();
            self.links_layout = backlinks::Layout::default();
            self.links_job.clear();
            self.menu = None;
            self.dialog = None;
            self.focus = Focus::Main;
            self.setup_ai(hwnd, &path);
            self.start_workspace_index(hwnd);
            self.sync_state();
            self.home.reset();
            self.start_home_analytics(hwnd, path);
            self.reload_home_dashboard();
            self.desktop_workspace_changed();
        }
        result.map_err(|e| {
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string())
        })
    }

    /// 索引会读遍全部候选文档，绝不能占用 UI 线程。首帧先展示工作区，再以 Win32
    /// 消息把进度送回绘制循环；搜索/反链在建索引期间可能稍后才出现，但窗口可继续使用。
    pub(super) fn start_workspace_index(&mut self, hwnd: HWND) {
        let Some(index) = self
            .shell
            .workspace()
            .map(|workspace| Arc::clone(&workspace.index))
        else {
            return;
        };
        self.workspace_index_generation = self.workspace_index_generation.wrapping_add(1);
        let generation = self.workspace_index_generation;
        let (tx, rx) = channel();
        self.workspace_index_rx = Some(rx);

        self.show_global_progress("正在建立搜索索引", 0, 0);

        let hwnd_raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = index.run_full_index_with_progress(|progress| {
                let _ = progress_tx.send((generation, WorkspaceIndexEvent::Progress(progress)));
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut _)),
                        platform::WM_APP_INDEX_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            });
            let _ = tx.send((
                generation,
                WorkspaceIndexEvent::Finished(result.map_err(|error| error.to_string())),
            ));
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_raw as *mut _)),
                    platform::WM_APP_INDEX_READY,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    }

    /// 清空已到达的进度消息。只在后台线程完成后读取 `indexed_file_count`，避免 UI
    /// 为了更新数字而竞争 SQLite 锁。
    pub fn take_workspace_index_updates(&mut self) {
        let events: Vec<_> = self
            .workspace_index_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for (generation, event) in events {
            if generation != self.workspace_index_generation {
                continue;
            }
            match event {
                WorkspaceIndexEvent::Progress(progress) => {
                    self.show_global_progress(
                        "正在建立搜索索引",
                        progress.processed,
                        progress.total,
                    );
                }
                WorkspaceIndexEvent::Finished(result) => {
                    self.workspace_index_rx = None;
                    match result {
                        Ok(updated) => {
                            self.shell.refresh_status();
                            self.show_global_notice(format!(
                                "搜索索引已就绪 · 本次更新 {updated} 个文件"
                            ));
                        }
                        Err(error) => {
                            self.show_global_notice(format!("搜索索引建立失败：{error}"));
                        }
                    }
                    break;
                }
            }
        }
    }

    pub fn open_workspace_dialog(&mut self, hwnd: HWND) {
        let Some(path) = platform::pick_folder(hwnd) else {
            return;
        };
        if let Err(e) = self.open_workspace(hwnd, path, true) {
            eprintln!("打开工作区失败: {e}");
        }
    }

    /// 在后台线程算首页快照。
    ///
    /// 这一步要扫全工作区、读完整个活动日志（真实工作区是 627 个文件 + 9743 条
    /// 事件），放在 UI 线程上会让窗口在打开工作区时僵住好几秒。算完投一条消息
    /// 回来，UI 线程再取结果——与文件监听同一个套路。
    pub(super) fn start_home_analytics(&mut self, hwnd: HWND, root: PathBuf) {
        let (tx, rx) = channel();
        self.home_rx = Some(rx);
        self.home_dirty = false;
        let hwnd_raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            let snapshot = mochi_core::analytics::build(&root, None, chrono::Local::now());
            // 发送失败说明 UI 侧已经换了工作区，直接丢弃，不要 panic
            if tx.send(snapshot).is_ok() {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut _)),
                        platform::WM_APP_HOME_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }

    /// 取回后台算好的首页快照。
    pub fn take_home_analytics(&mut self) {
        if let Some(snapshot) = self.home_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.home_rx = None;
            self.home.set_analytics(snapshot);
            self.reload_home_dashboard();
        }
    }

    /// 记录会影响首页统计的一次活动，并在首页打开时立即安排重新统计。
    /// 埋点写入失败不能影响用户原本的保存/发送操作。
    pub(super) fn record_home_activity(
        &mut self,
        input: mochi_core::analytics::events::ActivityInput,
    ) {
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            return;
        };
        if !mochi_core::analytics::events::record_input(&root, &input) {
            return;
        }
        self.home_dirty = true;
        if self.state.view == WorkspaceView::Home && self.home_rx.is_none() {
            self.start_home_analytics(HWND(self.hwnd_raw as *mut _), root);
        }
    }

    pub(super) fn dirty_text_save_activities(&self) -> Vec<TextSaveActivity> {
        self.shell
            .tabs()
            .iter()
            .filter(|tab| tab.dirty())
            .filter_map(|tab| {
                let path = tab.path()?.to_path_buf();
                let after_words = mochi_core::word_count::count_words(tab.buffer()?.text());
                let before_words = std::fs::read_to_string(&path)
                    .map(|content| mochi_core::word_count::count_words(&content))
                    .unwrap_or(0);
                Some(TextSaveActivity {
                    path,
                    before_words,
                    after_words,
                })
            })
            .collect()
    }

    pub(super) fn active_text_save_activity(&self) -> Option<TextSaveActivity> {
        let active_path = self.active_file_path()?;
        self.dirty_text_save_activities()
            .into_iter()
            .find(|activity| activity.path == active_path)
    }

    pub(super) fn record_text_save_activities(&mut self, activities: Vec<TextSaveActivity>) {
        for activity in activities {
            self.record_home_activity(mochi_core::analytics::events::ActivityInput {
                kind: Some(mochi_core::analytics::events::ActivityEventType::FileSave),
                path: Some(activity.path.to_string_lossy().into_owned()),
                words: Some(activity.after_words as f64),
                words_delta: Some(activity.after_words as f64 - activity.before_words as f64),
                ..Default::default()
            });
        }
    }

    /// 外部文件变更到达。
    pub fn on_files_changed(&mut self) {
        self.console_poll_jobs();
        self.console_agent_requests(HWND(self.hwnd_raw as *mut _));
        self.canvas_agent_requests();
        self.desktop_agent_requests();
        self.home_dirty = true;
        let mut new_subdocument_parents = Vec::new();
        let mut written_paths = Vec::new();
        if let Some(host) = &self.ai.host {
            for change in host.take_file_changes() {
                match change {
                    mochi_core::ai::tools::host::FileChange::Write { path } => {
                        written_paths.push(PathBuf::from(path));
                    }
                    mochi_core::ai::tools::host::FileChange::Rename { path, new_path } => {
                        self.shell
                            .accept_external_rename(Path::new(&path), Path::new(&new_path));
                        if let Some(rest) = self
                            .split
                            .other
                            .as_deref()
                            .and_then(|other| other.strip_prefix(&path).ok())
                        {
                            self.split.other = Some(PathBuf::from(&new_path).join(rest));
                        }
                        self.remember_split_active();
                    }
                    mochi_core::ai::tools::host::FileChange::Delete { path } => {
                        self.shell.accept_external_delete(Path::new(&path))
                    }
                    mochi_core::ai::tools::host::FileChange::SubDocumentCreate {
                        parent_path,
                        ..
                    } => new_subdocument_parents.push(PathBuf::from(parent_path)),
                    _ => {}
                }
            }
        }
        // `FileChange::Write` 也包括画布工具的写入。重新读取已打开的标签页，
        // 让成功的 Agent 更新立即显示，
        // 不用等用户手动关闭并重新打开文档。
        for path in written_paths {
            self.shell.reload_file(&path);
        }
        self.load_backlinks();
        let pdf_paths = self
            .shell
            .tabs()
            .iter()
            .filter(|t| t.is_pdf())
            .filter_map(|t| t.path().map(Path::to_path_buf))
            .collect::<Vec<_>>();
        for path in pdf_paths {
            let items = sidecars::load_pdf_annotations(&path.to_string_lossy()).annotations;
            if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(&path) {
                s.annotations.items = items;
                if s.annotations.selected().is_none() {
                    s.annotations.selected = None;
                }
            }
        }
        self.shell.refresh_tree();
        for parent in new_subdocument_parents {
            self.shell.expand(&parent);
        }
        self.shell.refresh_status();
        if self
            .shell
            .refresh_visible_files(self.split.other.as_deref())
        {
            self.doc.invalidate();
            self.split.doc.invalidate();
            self.source.invalidate();
            self.split.source.invalidate();
        }
        self.sync_state();
        // 伴生文件（评论/挂载/收件箱）可能被别的进程改了，当前面板重读一遍
        self.refresh_right_panel();
        // 日程文件（schedule/*.json）同理——Electron 版或 AI 工具写了就得跟上
        if self.sched.view.data.is_some() {
            self.agenda_refresh_if_stale();
        }
        // 收件箱（快速捕获窗写进来的）与最近文档（mtime 变了）
        if self.views.inbox.loaded && self.views.inbox.editing.is_none() {
            self.reload_inbox();
        }
        if self.views.recent.loaded {
            self.reload_recent();
        }
        if self.views.favorites.loaded {
            self.reload_favorites();
        }
        if self.state.view == WorkspaceView::Home {
            self.reload_home_dashboard();
        }
    }

    /// 当前面板的数据是不是针对现在这个文件。不是就重读——切标签之后调。
    pub(super) fn refresh_right_panel_if_stale(&mut self) {
        let active = self.active_file_path();
        let stale = match self.state.right_panel {
            RightPanel::VersionHistory => self.panels.version.source != active,
            RightPanel::Comments => self.panels.comments.source != active,
            RightPanel::DocumentMounts => self.panels.mounts.source != active,
            _ => false,
        };
        if stale {
            self.refresh_right_panel();
        }
    }

    /// 把 `Shell` 的状态投影到 `ChromeState`。
    ///
    /// 单向投影，不反过来——`ChromeState` 是给绘制看的快照，
    /// 把它作为第二份真实数据源，很快就会出现两处不同步等难以排查的问题。
    pub(super) fn sync_state(&mut self) {
        self.refresh_document_unread();
        self.sync_document_format_changes();
        self.reconcile_split();
        self.shell.set_protected_view_path(self.split.other.clone());
        self.load_editor_comments();
        self.status_bar.stats = None;
        self.state.workspace_name = self.shell.workspace().map(|ws| ws.name.clone());
        let mode = if gfx::low_memory_mode() {
            "软件光栅"
        } else {
            "GPU"
        };
        self.state.status_text = format!("{} · {mode}", self.shell.status());
        self.state.active_file_is_pdf = self.shell.active().map(|t| t.is_pdf()).unwrap_or(false);
        self.prune_pdf_jobs();
        let open_documents: Vec<&Path> = self
            .shell
            .tabs()
            .iter()
            .filter(|tab| tab.buffer().is_some())
            .filter_map(|tab| tab.path())
            .collect();
        let closed_documents = open_documents.len() < self.open_document_count;
        self.open_document_count = open_documents.len();
        self.doc.retain_open_documents(&open_documents);
        self.split.doc.retain_open_documents(&open_documents);
        if open_documents.is_empty() {
            self.source = SourcePane::default();
            self.split.source = SourcePane::default();
        }
        if closed_documents {
            platform::trim_process_working_set();
        }
        // 收件箱徽标。读一个小 JSON，便宜；快速捕获窗写进来后靠文件监听触发重算
        self.nav.inbox_count = self
            .shell
            .workspace()
            .map(|ws| CaptureService::new(&ws.root).count_inbox())
            .unwrap_or(0);
        self.refresh_right_panel_if_stale();
        if self.links.source != self.link_source() {
            self.load_backlinks();
        }
    }

    /// 主区此刻画什么。绘制、点击、滚轮都问这一个函数——见 [`MainContent::resolve`]。
    pub(super) fn content(&self) -> MainContent {
        if self.state.view == WorkspaceView::Editor
            && self
                .shell
                .active()
                .is_some_and(|tab| matches!(tab.kind, TabKind::Library { .. }))
        {
            return MainContent::NoTab;
        }
        let facts = self.shell.active_tab().map(|index| TabFacts {
            index,
            source_mode: self
                .shell
                .active()
                .map(|t| t.source_mode())
                .unwrap_or(false),
            special: self
                .shell
                .active()
                .map(|t| !matches!(t.kind, TabKind::File { .. }))
                .unwrap_or(false),
        });
        MainContent::resolve(&self.state, self.shell.has_workspace(), facts)
    }

    /// 导航轨里当前高亮哪个入口。照抄 TSX 的 `activeNavId` 推导。
    pub(super) fn nav_active(&self) -> Option<NavItem> {
        match self.state.view {
            WorkspaceView::Home => Some(NavItem::Home),
            WorkspaceView::Inbox => Some(NavItem::Inbox),
            WorkspaceView::Recent => Some(NavItem::Recent),
            WorkspaceView::Templates => Some(NavItem::Templates),
            WorkspaceView::Marketplace | WorkspaceView::DesktopCards => None,
            WorkspaceView::Plugin => None,
            WorkspaceView::Schedule => Some(NavItem::Schedule),
            WorkspaceView::MochiAi => Some(NavItem::MochiAi),
            WorkspaceView::QuickNote => Some(NavItem::QuickNote),
            WorkspaceView::AgentConfig => Some(NavItem::MochiAi),
            WorkspaceView::Automations => Some(NavItem::Automations),
            WorkspaceView::Editor => match self.shell.active().map(|t| &t.kind) {
                Some(TabKind::Settings { .. }) => None,
                _ if self.shell.favorites_selected() => Some(NavItem::Favorites),
                _ => Some(NavItem::Knowledge),
            },
        }
    }

    /// 导航轨的模型：全部从 `Shell` 与 `NavState` 投影。
    pub(super) fn nav_types(&self) -> (Vec<(String, String)>, Vec<(String, String)>) {
        match self.shell.workspace() {
            Some(ws) => (
                ws.library_types
                    .iter()
                    .map(|t| (t.id.clone(), t.name.clone()))
                    .collect(),
                ws.libraries
                    .iter()
                    .map(|l| (l.kind.clone(), l.name.clone()))
                    .collect(),
            ),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// 当前标签的可绘制投影。
    pub(super) fn tab_projection(&self) -> Vec<tab_bar::Tab> {
        self.shell
            .tabs()
            .iter()
            .map(|t| tab_bar::Tab {
                title: t.title.clone(),
                dirty: t.dirty(),
                pinned: t.pinned,
            })
            .collect()
    }

    /// 搭一次外壳并记下侧栏内容区。输入处理与绘制共用，两边不会算出不同的坐标。
    pub(super) fn build_chrome(&mut self) -> Chrome {
        // `outline_mode` resolves the temporary AI-open placement from settings without
        // persisting that projection. Keep the chrome snapshot and editor layout in sync.
        self.state.outline_in_ai_sidebar = self.outline_mode() == "ai-sidebar";
        let chrome = Chrome::build(&self.state, self.renderer.viewport());
        self.tree_area = chrome.tree.rect(chrome.sidebar);
        chrome
    }

    pub(super) fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    /// 小记复用普通编辑器，同时保留固定入口及升级后的文档身份。
    pub(super) fn ensure_quick_note_tab(&mut self) -> bool {
        let Some(ws) = self.shell.workspace() else {
            return false;
        };
        if self.shell.active().is_some_and(|tab| {
            tab.buffer().is_some()
                && tab.path().is_some_and(|path| {
                    path == ws.root.join("小记.md") || path == ws.root.join("小记.mc")
                })
        }) {
            return true;
        }
        let result = views::quick_note::ensure(&ws.root);
        let error = match result {
            Ok(path) if self.open_file_from_ui(&path) => {
                self.invalidate_main();
                self.sync_state();
                return true;
            }
            Ok(_) => self.shell.status().to_owned(),
            Err(error) => error.to_string(),
        };
        self.dialog = Some(Dialog {
            title: "无法打开小记".into(),
            description: error,
            field: None,
            error: String::new(),
            note: None,
            buttons: vec![DialogButton {
                label: "确定".into(),
                kind: ButtonKind::Primary,
                action: DialogAction::Dismiss,
            }],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
        false
    }
}
