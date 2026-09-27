//! 初始化 AI 会话服务，并管理会话的创建、打开、删除和清理。
use super::*;

impl App {
    /// 换工作区：建宿主、读会话索引、恢复上次打开的会话。
    pub(super) fn setup_ai(&mut self, hwnd: HWND, root: &Path) {
        if let Some(run) = self.ai.run.take() {
            run.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let permissions = Arc::new(AiPermissionService::new(root));
        if let Some(raw) = self.settings.get("ai.actionPermissions") {
            permissions.set_action_permissions(
                mochi_core::ai::permission::ActionPermissions::from_setting(&raw),
            );
        }
        let snapshot = Arc::new(Mutex::new(HostSnapshot::default()));
        let export_requests = self.settings.file_path().parent().and_then(|profile| {
            match crate::export_requests::Queue::new(profile, root) {
                Ok(queue) => Some(Arc::new(queue)),
                Err(error) => {
                    self.state.status_text = format!("导出审批队列读取失败：{error}");
                    None
                }
            }
        });
        let hwnd_raw = hwnd.0 as isize;
        let host = AppHost::new(
            root,
            Arc::clone(&snapshot),
            Arc::clone(&permissions),
            move || unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_raw as *mut _)),
                    platform::WM_APP_FILES_CHANGED,
                    WPARAM(0),
                    LPARAM(0),
                );
            },
        )
        .with_export_queue(export_requests.clone())
        .with_runtime_settings(self.settings.clone());
        self.ai = AiState {
            pending_memory: None,
            memory_jobs: Vec::new(),
            follow_up_jobs: Vec::new(),
            title_jobs: Vec::new(),
            export_requests,
            workspace: ai_workspace::State::default(),
            workspace_side: ai_workspace::Layout::default(),
            panel: assistant::State::default(),
            layout: assistant::Layout::default(),
            float: None,
            run: None,
            animation_timer_armed: false,
            snapshot,
            host: Some(Arc::new(host)),
            permissions: Some(permissions),
        };
        let svc = AiSessionService::new(root);
        let _ = svc.initialize();
        let index = svc.load_index();
        self.ai.panel.sessions = index.sessions;
        self.ai.workspace.projects = index.projects;
        // 上次打开的会话（Electron 版持久化 activeSessionId）
        if let Some(id) = self.settings.get("ai.activeSessionId") {
            self.ai.panel.active = svc.load_session(&id);
        }
        self.ai.panel.provider_missing = ai_runtime::load_provider(&self.settings).is_none();
        if let Err(error) =
            mochi_core::ai::agent_config::AgentConfigService::new(root).ensure_seeds()
        {
            self.ai.panel.error = format!("初始化 Agent 配置失败：{error}");
        }
        self.reload_ai_agents();
        self.check_agent_updates(true);
        self.recover_command_results();
        self.refresh_ai_images();
    }

    pub(super) fn ai_session_service(&self) -> Option<AiSessionService> {
        self.shell
            .workspace()
            .map(|ws| AiSessionService::new(&ws.root))
    }

    /// 新建会话：立刻登记到索引（TSX 的 `handleNewConversation`）。
    pub(super) fn ai_new_session(&mut self) {
        if self.ai.panel.is_streaming() {
            self.ai_cancel();
        }
        self.ai.panel.loaded_skills.clear();
        let Some(svc) = self.ai_session_service() else {
            return;
        };
        let now = mochi_core::jstime::now_millis();
        let conv = AiConversation {
            id: ai_session::new_conversation_id(),
            title: "新会话".into(),
            created_at: now,
            updated_at: now,
            messages: Vec::new(),
        };
        let _ = svc.save_session(&conv);
        let mut index = svc.load_index();
        index.sessions.insert(
            0,
            AiSessionMeta {
                id: conv.id.clone(),
                title: conv.title.clone(),
                created_at: now,
                updated_at: now,
                ..Default::default()
            },
        );
        let _ = svc.save_index(&index);
        self.ai.panel.sessions = index.sessions;
        self.settings.set("ai.activeSessionId", &conv.id);
        self.ai.panel.active = Some(conv);
        self.ai_reset_message_view();
        self.ai.panel.scroll = 0.0;
        self.ai.panel.show_conversations = false;
        self.ai.panel.error.clear();
        self.refresh_ai_images();
    }

    pub(super) fn ai_open_session(&mut self, id: &str) {
        if self.ai.panel.active.as_ref().is_some_and(|c| c.id == id) {
            self.ai.panel.show_conversations = false;
            return;
        }
        if self.ai.panel.active.as_ref().is_some_and(|c| c.id != id) {
            self.ai_cancel();
        }
        let Some(svc) = self.ai_session_service() else {
            return;
        };
        if let Some(c) = svc.load_session(id) {
            self.ai.panel.loaded_skills.clear();
            self.settings.set("ai.activeSessionId", id);
            self.ai.panel.active = Some(c);
            self.ai_reset_message_view();
            self.ai.panel.scroll = 0.0;
            self.ai.panel.stick_to_bottom = true;
        }
        self.ai.panel.show_conversations = false;
        self.refresh_ai_images();
    }

    pub(super) fn ai_delete_session(&mut self, id: &str) {
        self.cancel_title_job(id);
        self.ai_clear_text_selection();
        if self.ai.panel.active.as_ref().is_some_and(|c| c.id == id) {
            self.ai_cancel();
        }
        let Some(svc) = self.ai_session_service() else {
            return;
        };
        let _ = svc.delete_session(id);
        let mut index = svc.load_index();
        index.sessions.retain(|s| s.id != id);
        let _ = svc.save_index(&index);
        self.ai.panel.sessions = index.sessions;
        if self
            .ai
            .panel
            .active
            .as_ref()
            .map(|c| c.id == id)
            .unwrap_or(false)
        {
            self.ai.panel.active = None;
            self.settings.remove("ai.activeSessionId");
        }
        if let Some(ws) = self.shell.workspace() {
            let _ = AiDocumentMountService::new(&ws.root).remove_session_mounts(id);
        }
    }

    /// 清空当前会话的消息（`handleClear`）。
    pub(super) fn ai_clear_session(&mut self) {
        self.ai_clear_text_selection();
        if self.ai.panel.is_streaming() {
            return;
        }
        if let Some(id) = self.ai.panel.active.as_ref().map(|c| c.id.clone()) {
            self.cancel_title_job(&id);
        }
        if let Some(c) = self.ai.panel.active.as_mut() {
            c.messages.clear();
            c.updated_at = mochi_core::jstime::now_millis();
        }
        self.ai_persist_active();
    }

    /// 把当前会话写回磁盘并更新索引里的元数据。
    pub(super) fn ai_persist_active(&mut self) -> bool {
        let Some(svc) = self.ai_session_service() else {
            return false;
        };
        let Some(c) = self.ai.panel.active.as_ref() else {
            return false;
        };
        if let Err(e) = svc.save_session(c) {
            self.ai.panel.error = format!("保存会话失败：{e}");
            return false;
        }
        let mut index = svc.load_index();
        let count = c.messages.iter().filter(|m| !m.is_hidden()).count() as i32;
        match index.sessions.iter_mut().find(|s| s.id == c.id) {
            Some(meta) => {
                meta.title = c.title.clone();
                meta.updated_at = c.updated_at;
                meta.message_count = count;
            }
            None => index.sessions.insert(
                0,
                AiSessionMeta {
                    id: c.id.clone(),
                    title: c.title.clone(),
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                    message_count: count,
                    ..Default::default()
                },
            ),
        }
        if let Err(e) = svc.save_index(&index) {
            self.ai.panel.error = format!("保存会话索引失败：{e}");
            return false;
        }
        self.ai.panel.sessions = index.sessions;
        true
    }
}
