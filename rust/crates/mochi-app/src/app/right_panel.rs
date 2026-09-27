//! 绘制右侧栏和文件区大纲，并管理插件面板入口。
use super::*;

impl App {
    pub(super) fn paint_right_sidebar(&mut self, chrome: &Chrome, p: &Palette) {
        if chrome.tree.is_hidden(chrome.right_sidebar) {
            return;
        }
        let body = chrome.tree.rect(chrome.right_sidebar_body);
        match self.state.right_panel {
            RightPanel::Annotations => {
                let (name, items) = match self.viewer_tab() {
                    Some((path, viewer::Content::Pdf(s))) => (
                        path.file_name().map(|v| v.to_string_lossy().into_owned()),
                        s.annotations.items.clone(),
                    ),
                    _ => (None, Vec::new()),
                };
                let mut lay = pdf_annotations::layout(body, &items, self.panels.annotation_scroll);
                let scroll = self.panels.annotation_scroll.min(lay.max_scroll());
                if scroll != self.panels.annotation_scroll {
                    self.panels.annotation_scroll = scroll;
                    lay = pdf_annotations::layout(body, &items, scroll);
                }
                pdf_annotations::paint(&mut self.list, body, name.as_deref(), &items, &lay, p);
                self.panels.annotation_layout = lay;
            }
            RightPanel::Outline => {
                self.outline_area = body;
                let headings = if self.link_source().is_some() {
                    self.doc.headings()
                } else {
                    &[]
                };
                outline::paint(
                    &mut self.list,
                    body,
                    headings,
                    self.doc.active_heading(self.shell.active_scroll()),
                    self.outline_scroll,
                    p,
                );
            }
            RightPanel::VersionHistory => {
                let lay = version::layout(&self.panels.version, body);
                self.panels.version.scroll = self.panels.version.scroll.min(lay.max_scroll());
                version::paint(
                    &mut self.list,
                    body,
                    &self.panels.version,
                    &lay,
                    self.focus == Focus::VersionMessage,
                    p,
                );
                self.panels.version_layout = lay;
            }
            RightPanel::Pomodoro => {
                let lay = pomodoro::layout(body);
                pomodoro::paint(&mut self.list, body, &self.panels.pomodoro, &lay, p);
                self.panels.pomodoro_layout = lay;
            }
            RightPanel::Comments => {
                let lay = comments::layout(&self.panels.comments, body);
                self.panels.comments.scroll = self.panels.comments.scroll.min(lay.max_scroll());
                if self.focus == Focus::CommentCompose {
                    if let (Some(pending), Some(r)) = (
                        self.panels.comments.pending.as_mut(),
                        lay.rect_of(comments::Hit::ComposeField),
                    ) {
                        pending.field.sync_multiline_scroll(r);
                    }
                }
                comments::paint(
                    &mut self.list,
                    body,
                    &self.panels.comments,
                    &lay,
                    self.focus == Focus::CommentCompose,
                    p,
                );
                self.panels.comments_layout = lay;
            }
            RightPanel::DocumentMounts => {
                let lay = mounts::layout(&self.panels.mounts, body);
                self.panels.mounts.scroll = self.panels.mounts.scroll.min(lay.max_scroll());
                mounts::paint(&mut self.list, body, &self.panels.mounts, &lay, p);
                self.panels.mounts_layout = lay;
            }
            RightPanel::AgentInbox => {
                let lay = inbox::layout(&self.panels.inbox, body);
                self.panels.inbox.scroll = self.panels.inbox.scroll.min(lay.max_scroll());
                inbox::paint(&mut self.list, body, &self.panels.inbox, &lay, p);
                self.panels.inbox_layout = lay;
            }
            RightPanel::Plugin => self.paint_plugin_right_sidebar(body, p),
            RightPanel::Assistant => self.paint_assistant_surface(body, p),
        }
    }

    pub(super) fn plugin_right_slots(&self) -> Vec<mochi_core::plugins::RegisteredSlot> {
        self.shell
            .workspace()
            .and_then(|ws| {
                mochi_core::plugins::PluginService::new(&ws.root)
                    .registered_slots()
                    .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .filter(|slot| slot.contribution.slot == mochi_core::plugins::PluginSlot::RightSidebar)
            .collect()
    }

    /// 所有第三方右侧栏共用这一层宿主：先选择声明的贡献，再绘制其安全的
    /// `ui.json`。插件只提供数据，不拥有 Win32 控件、窗口生命周期或绘制上下文。
    pub(super) fn paint_plugin_right_sidebar(&mut self, body: Rect, p: &Palette) {
        self.plugin_right_area = body;
        let slots = self.plugin_right_slots();
        if slots.is_empty() {
            self.list.text(
                body.inset(crate::ui::layout::Edges::xy(20.0, 20.0)),
                "没有已启用的插件注册右侧栏。",
                TextStyle::Label,
                p.muted,
            );
            return;
        }
        let selected = self.plugin_right_panel.clone().or_else(|| {
            slots.first().map(|slot| {
                (
                    slot.plugin.manifest.id.clone(),
                    slot.contribution.title.clone(),
                )
            })
        });
        let mut y = body.top + 12.0;
        for slot in &slots {
            let row = Rect::new(body.left + 12.0, y, body.right - 12.0, y + 30.0);
            let active = selected.as_ref().is_some_and(|key| {
                key.0 == slot.plugin.manifest.id && key.1 == slot.contribution.title
            });
            if active {
                self.list.rounded_rect(
                    row,
                    6.0,
                    crate::ui::theme::mix(p.accent, p.area_assistant_default, 0.12),
                );
            }
            self.list.text(
                Rect::new(row.left + 10.0, row.top, row.right - 10.0, row.bottom),
                format!(
                    "{} · {}",
                    slot.plugin.manifest.name, slot.contribution.title
                ),
                TextStyle::Caption,
                if active { p.accent } else { p.foreground },
            );
            y += 34.0;
        }
        let Some(key) = selected else {
            return;
        };
        let Some(slot) = slots
            .into_iter()
            .find(|slot| slot.plugin.manifest.id == key.0 && slot.contribution.title == key.1)
        else {
            return;
        };
        match mochi_core::plugins::PluginService::new(
            self.shell
                .workspace()
                .expect("plugin slots require workspace")
                .root
                .clone(),
        )
        .load_slot_ui(&slot)
        {
            Ok(Some(ui)) => {
                self.list.text(
                    Rect::new(body.left + 20.0, y + 12.0, body.right - 20.0, y + 40.0),
                    ui.title,
                    TextStyle::Label,
                    p.foreground,
                );
                self.list.text(
                    Rect::new(body.left + 20.0, y + 42.0, body.right - 20.0, y + 64.0),
                    format!("{} · 原生插件界面", ui.kind),
                    TextStyle::Caption,
                    p.muted,
                );
                if let Some(section) = ui.sections.first() {
                    self.list.text(
                        Rect::new(body.left + 20.0, y + 76.0, body.right - 20.0, y + 98.0),
                        &section.title,
                        TextStyle::Caption,
                        p.foreground,
                    );
                    self.list.text(
                        Rect::new(body.left + 28.0, y + 100.0, body.right - 20.0, y + 122.0),
                        section
                            .cards
                            .iter()
                            .take(3)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("  ·  "),
                        TextStyle::Caption,
                        p.muted,
                    );
                }
            }
            Ok(None) => self.list.text(
                Rect::new(body.left + 20.0, y + 12.0, body.right - 20.0, y + 38.0),
                "该容器贡献未声明 UI。",
                TextStyle::Caption,
                p.muted,
            ),
            Err(error) => self.list.text(
                Rect::new(body.left + 20.0, y + 12.0, body.right - 20.0, y + 38.0),
                format!("无法加载插件 UI：{error}"),
                TextStyle::Caption,
                p.danger,
            ),
        }
    }

    pub(super) fn outline_mode(&self) -> String {
        let placement = app_settings::descriptor("outline.placement")
            .map(|d| self.app_settings.read(d).to_storage())
            .unwrap_or_else(|| "editor-right".into());
        if placement == "editor-right"
            && self.state.ai_panel_open
            && crate::ui::settings_values::boolean("outline.moveToSidebarWhenAIOpen", true)
        {
            "ai-sidebar".into()
        } else {
            placement
        }
    }

    pub(super) fn outline_visible_in_file_area(&self) -> bool {
        app_settings::descriptor("outline.visible").is_none_or(|descriptor| {
            matches!(self.app_settings.read(descriptor), SettingValue::Bool(true))
        })
    }

    pub(super) fn toggle_file_area_outline(&mut self) {
        let Some(descriptor) = app_settings::descriptor("outline.visible") else {
            return;
        };
        self.app_settings.write(
            descriptor,
            &SettingValue::Bool(!self.outline_visible_in_file_area()),
        );
        if let Err(error) = self.settings.flush() {
            self.state.status_text = format!("大纲显示设置保存失败：{error}");
        }
        self.invalidate_main();
    }

    pub(super) fn on_outline_click(&mut self, x: f32, y: f32) {
        if let Some(i) = outline::hit(
            self.outline_area,
            self.doc.headings().len(),
            self.outline_scroll,
            x,
            y,
        ) {
            if let Some(offset) = self.doc.heading_offset(i) {
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    buffer.set_cursor(offset, false);
                }
                self.after_doc_selection_change(true);
                return;
            }
            self.shell.set_active_scroll(
                outline::scroll_target(&self.doc.headings()[i])
                    .min(self.doc.max_scroll(self.editor_area)),
            );
        }
    }

    pub(super) fn on_link_hit(&mut self, hit: backlinks::Hit) {
        match hit {
            backlinks::Hit::Refresh => self.load_backlinks(),
            backlinks::Hit::Toggle(i) => self.links.collapsed[i] = !self.links.collapsed[i],
            backlinks::Hit::Open(path) => {
                if self.open_link_file_from_ui(&path) {
                    self.state.view = WorkspaceView::Editor;
                    self.invalidate_main();
                    self.sync_state();
                }
            }
            backlinks::Hit::Heading(_) => {}
        }
    }

    pub(super) fn on_ai_workspace_sidebar_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.ai.workspace_side.hit(x, y) else {
            return;
        };
        match hit {
            ai_workspace::Hit::Query => {
                self.focus = Focus::AiSessionQuery;
                if let Some(r) = self.ai.workspace_side.query() {
                    self.ai.workspace.query.click(x - r.left - 32.0, false);
                }
            }
            _ if self.ai.panel.is_streaming() => {}
            ai_workspace::Hit::New => self.ai_new_session(),
            ai_workspace::Hit::Open(id) => self.ai_open_session(&id),
            ai_workspace::Hit::Delete(id) => {
                self.ai_ask_delete_session(&id);
            }
        }
    }

    pub(super) fn link_source(&self) -> Option<PathBuf> {
        self.shell
            .active()
            .filter(|t| t.buffer().is_some())
            .and_then(|t| t.path())
            .map(Path::to_path_buf)
    }

    pub(super) fn load_backlinks(&mut self) {
        let source = self.link_source();
        if self.links.source != source {
            self.outline_scroll = 0;
            self.links.data = backlinks::Data::default();
            self.links.scroll = 0.0;
            self.links_layout = backlinks::Layout::default();
        }
        self.links.source = source.clone();
        self.links.hover = None;
        self.links.error.clear();
        self.links_job.clear();
        self.links.loading = false;
        if let (Some(source), Some(ws)) = (source, self.shell.workspace()) {
            self.links.loading = true;
            self.links_job
                .start(Arc::clone(&ws.index), source, self.hwnd_raw);
        }
    }

    pub fn take_backlinks(&mut self) {
        if let Some(result) = self.links_job.take() {
            self.links.loading = false;
            match result {
                Ok(data) => self.links.data = data,
                Err(error) => self.links.error = error,
            }
        }
    }

    /// 当前面板要展示的数据从磁盘读一遍。换标签、换面板、外部文件变更后调。
    ///
    /// 只刷新**当前显示**的面板：版本历史需要逐个提交比较树的差异，评论和挂载则要读取伴随文件，
    /// 五个面板每次全刷会让切标签明显变慢。
    pub(super) fn refresh_right_panel(&mut self) {
        if !self.state.right_sidebar_visible() {
            return;
        }
        let active = self.active_file_path();
        match self.state.right_panel {
            RightPanel::VersionHistory => self.load_version_history(active),
            RightPanel::Comments => {
                let s = &mut self.panels.comments;
                if s.source != active {
                    s.pending = None;
                    s.scroll = 0.0;
                }
                s.source = active.clone();
                s.comments = active
                    .as_deref()
                    .map(|p| sidecars::load_comments(&p.to_string_lossy()).comments)
                    .unwrap_or_default();
                if let Some(a) = self.settings.get("comments.author") {
                    s.author = a;
                }
            }
            RightPanel::DocumentMounts => {
                let s = &mut self.panels.mounts;
                s.source = active.clone();
                s.mounts = match (&active, self.shell.workspace()) {
                    (Some(p), Some(ws)) => AiDocumentMountService::new(&ws.root)
                        .mounts_for_document(&p.to_string_lossy()),
                    _ => Vec::new(),
                };
            }
            RightPanel::AgentInbox => {
                let s = &mut self.panels.inbox;
                s.entries = self
                    .shell
                    .workspace()
                    .map(|ws| AgentInboxService::new(&ws.root).list_pending())
                    .unwrap_or_default();
                if let Some(queue) = &self.ai.export_requests {
                    s.entries.extend(
                        queue
                            .pending()
                            .iter()
                            .map(crate::export_requests::Request::as_inbox),
                    );
                }
                s.loading = false;
            }
            _ => {}
        }
    }

    pub(super) fn load_version_history(&mut self, source: Option<PathBuf>) {
        let s = &mut self.panels.version;
        if s.source != source {
            s.scroll = 0.0;
            s.message.clear();
            s.deleted = source
                .as_deref()
                .and_then(|p| self.settings.get(&deleted_versions_key(p)))
                .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
                .map(|v| v.into_iter().collect())
                .unwrap_or_default();
        }
        s.source = source.clone();
        s.error.clear();
        s.enabled = app_settings::descriptor("git.versionHistoryEnabled")
            .map(|d| matches!(self.app_settings.read(d), SettingValue::Bool(true)))
            .unwrap_or(true);
        s.commits.clear();
        s.loading = false;
        self.panels.version_rx = None;
        let (Some(path), Some(git)) = (source, self.shell.git()) else {
            return;
        };
        let limit = crate::ui::settings_values::number("git.maxHistoryDepth", 100.0) as usize;
        let (tx, rx) = channel();
        self.panels.version_rx = Some(rx);
        self.panels.version.loading = true;
        std::thread::spawn(move || {
            let result = git
                .get_history_filtered(limit, Some(&path.to_string_lossy()), None)
                .map_err(|error| error.to_string());
            let _ = tx.send((Some(path), result));
        });
    }

    pub fn take_version_history_result(&mut self) {
        let Some(rx) = self.panels.version_rx.as_ref() else {
            return;
        };
        let Ok((source, result)) = rx.try_recv() else {
            return;
        };
        self.panels.version_rx = None;
        if self.panels.version.source != source {
            return;
        }
        self.panels.version.loading = false;
        match result {
            Ok(commits) => self.panels.version.commits = commits,
            Err(error) => self.panels.version.error = error,
        }
    }

    pub(super) fn set_right_panel(&mut self, panel: RightPanel) {
        if panel == RightPanel::Assistant {
            self.ai.float = None;
        }
        self.state.right_panel = panel;
        self.state.status_text = format!("已打开{}", panel.label());
        self.outline_scroll = 0;
        self.links.scroll = 0.0;
        if matches!(self.focus, Focus::VersionMessage | Focus::CommentCompose)
            || (panel != RightPanel::Assistant
                && matches!(
                    self.focus,
                    Focus::AiInput | Focus::AiMessageQuery | Focus::AiSessionQuery
                ))
        {
            self.focus = Focus::Main;
        }
        self.refresh_right_panel();
    }

    pub(super) fn on_right_panel_click(&mut self, x: f32, y: f32) {
        match self.state.right_panel {
            RightPanel::Plugin => {
                let index = ((y - (self.plugin_right_area.top + 12.0)) / 34.0).floor() as usize;
                if let Some(slot) = self.plugin_right_slots().get(index) {
                    self.plugin_right_panel = Some((
                        slot.plugin.manifest.id.clone(),
                        slot.contribution.title.clone(),
                    ));
                }
            }
            RightPanel::Annotations => match self.panels.annotation_layout.hit(x, y) {
                Some(pdf_annotations::Hit::Locate(id)) => self.pdf_locate_annotation(&id),
                Some(pdf_annotations::Hit::Delete(id)) => {
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
                        s.annotations.selected = Some(id);
                    }
                    self.pdf_delete_selected();
                }
                None => {}
            },
            RightPanel::Assistant => {
                let hwnd = HWND(self.hwnd_raw as *mut _);
                self.on_assistant_click(hwnd, x, y);
            }
            RightPanel::VersionHistory => self.on_version_click(x, y),
            RightPanel::Pomodoro => {
                let Some((_, hit)) = self
                    .panels
                    .pomodoro_layout
                    .iter()
                    .find(|(r, _)| r.contains(x, y))
                else {
                    return;
                };
                match hit {
                    pomodoro::Hit::Toggle => {
                        if self.panels.pomodoro.running {
                            self.panels.pomodoro.pause();
                        } else {
                            self.panels.pomodoro.start();
                        }
                    }
                    pomodoro::Hit::Reset => self.panels.pomodoro.reset(),
                }
            }
            RightPanel::Comments => self.on_comments_click(x, y),
            RightPanel::DocumentMounts => {
                let Some(hit) = self.panels.mounts_layout.hit(x, y) else {
                    return;
                };
                match hit {
                    mounts::Hit::Delete(i) => {
                        if let (Some(m), Some(ws)) =
                            (self.panels.mounts.mounts.get(i), self.shell.workspace())
                        {
                            let _ = AiDocumentMountService::new(&ws.root).remove_mount(m.id());
                        }
                        self.refresh_right_panel();
                    }
                    mounts::Hit::Open(i) => {
                        if let Some(m) = self.panels.mounts.mounts.get(i) {
                            let id = m.session_id().to_owned();
                            let message = m.message_id().map(str::to_owned);
                            self.ai_open_session(&id);
                            if self.ai.panel.active.as_ref().is_some_and(|c| c.id == id) {
                                self.state.ai_panel_open = true;
                                self.state.right_panel = RightPanel::Assistant;
                                if let Some(message) = message {
                                    let index = self.ai.panel.active.as_ref().and_then(|c| {
                                        c.messages
                                            .iter()
                                            .filter(|m| {
                                                !m.is_hidden()
                                                    && matches!(m.role(), "user" | "assistant")
                                                    && !m.content().is_empty()
                                            })
                                            .position(|m| m.id() == Some(message.as_str()))
                                    });
                                    if let Some(index) = index {
                                        let chrome = self.build_chrome();
                                        let area = chrome.tree.rect(chrome.right_sidebar_body);
                                        let lay = assistant::layout(&self.ai.panel, area);
                                        if let Some(m) = lay.messages.get(index) {
                                            self.ai.panel.scroll = m.top;
                                            self.ai.panel.stick_to_bottom = false;
                                        }
                                    }
                                }
                            } else {
                                self.state.status_text = "挂载对应的 AI 会话已不存在".into();
                            }
                        }
                    }
                }
            }
            RightPanel::AgentInbox => {
                let Some(hit) = self.panels.inbox_layout.hit(x, y) else {
                    return;
                };
                if self.shell.workspace().is_none() {
                    return;
                }
                match hit {
                    inbox::Hit::Refresh => {}
                    inbox::Hit::Open(group) => self.open_inbox_approval_group(group),
                    inbox::Hit::Approve(group) => self.approve_inbox_approval_group(group),
                    inbox::Hit::Reject(group) => self.reject_inbox_approval_group(group),
                    inbox::Hit::ApproveAll => self.approve_all_document_approvals(),
                    inbox::Hit::RejectAll => self.reject_all_inbox_approvals(),
                }
                self.shell.refresh_tree();
                self.refresh_right_panel();
            }
            _ => {}
        }
    }
}
