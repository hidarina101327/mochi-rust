//! 调度并处理应用计时器事件。
use super::*;

#[cfg(test)]
#[path = "sidebar_tree_loading_tests.rs"]
mod sidebar_tree_loading_tests;

impl App {
    /// 窗口过程在每条输入消息之后调：把「想要一个计时器」变成真正的 `SetTimer`。
    /// `App` 不直接调 Win32 计时器——那是窗口过程的职责，这里只表达意图。
    pub fn take_timer_requests(&mut self) -> Vec<(usize, u32)> {
        *self
            .web_clipper
            .workspace
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = self.shell.workspace().map(|w| w.root.clone());
        let mut out = Vec::new();
        if self.shell.workspace().is_some() && !self.sched.timer_armed {
            self.sched.timer_armed = true;
            out.push((platform::TIMER_SCHEDULE, 15_000));
        }
        if self.shell.tree_loading_pending() {
            out.push((platform::TIMER_TREE_LOAD, 16));
        }
        if self.window_motion.animating() {
            out.push((platform::TIMER_WINDOW_MOTION, 16));
        }
        if self.prefs.providers.interaction.animating() {
            out.push((platform::TIMER_PROVIDER_INTERACTION, 16));
        }
        if let Some(delay) = self.nav.drag_timer.take() {
            out.push((platform::TIMER_NAVIGATION_DRAG, delay));
        }
        if self.base_export.pending() {
            out.push((platform::TIMER_BASE_EXPORT, 100));
        }
        self.workflows_sync();
        if self.workflows.store.is_some() && !self.workflows.armed {
            self.workflows.armed = true;
            out.push((platform::TIMER_WORKFLOWS, 1000));
        }
        if let Some(delay) = self.notifications.save_timer.take() {
            out.push((platform::TIMER_NOTIFICATIONS, delay));
        }
        if self.shell.workspace().is_some() && !self.automation.armed {
            self.automation.armed = true;
            out.push((platform::TIMER_BASE_AUTOMATION, 1500));
        }
        if let Some(delay) = self.resource_timer_request() {
            out.push((platform::TIMER_RESOURCE, delay));
        }
        if let Some(delay) = self.status_bar.word_count_timer_request() {
            out.push((platform::TIMER_WORD_COUNT, delay));
        }
        if let Some(delay) = self.status_bar.timer.take() {
            out.push((platform::TIMER_TOAST, delay));
        }
        if let Some(delay) = self.editor_ai.timer.take() {
            out.push((platform::TIMER_EDITOR_AI, delay));
        }
        if let Some(delay) = self.ai.panel.locator_timer.take() {
            out.push((platform::TIMER_AI_LOCATOR, delay));
        }
        if self.search_job.timer_pending {
            self.search_job.timer_pending = false;
            out.push((platform::TIMER_SEARCH, search::DEBOUNCE_MS));
        }
        // 番茄钟跑着就每 250ms 刷一次；不在面板里也要跑——倒计时不能因为切走就停
        if self.panels.pomodoro.running && !self.panels.pomodoro_timer_armed {
            self.panels.pomodoro_timer_armed = true;
            out.push((platform::TIMER_POMODORO, 250));
        }
        if let Some(delay) = self.autosave_request.take() {
            out.push((platform::TIMER_AUTOSAVE, delay));
        }
        if self.ai.panel.is_streaming() && !self.ai.animation_timer_armed {
            self.ai.animation_timer_armed = true;
            out.push((platform::TIMER_AI_ANIMATION, 160));
        }
        if self.panels.version_rx.is_some() {
            out.push((platform::TIMER_VERSION_JOB, 80));
        }
        if self.doc.is_loading() || self.split.doc.is_loading() {
            out.push((platform::TIMER_DOCUMENT_LAYOUT, 8));
        }
        out
    }

    /// 计时器到点。
    pub fn on_timer(&mut self, hwnd: HWND, id: usize) {
        if id == platform::TIMER_SCHEDULE {
            self.sched.timer_armed = false;
            self.schedule_tick();
            return;
        }
        if id == platform::TIMER_TREE_LOAD {
            let rename_path = self.side.editing.as_ref().and_then(|editing| {
                if let EditKind::Rename { row } = editing.kind {
                    self.shell.rows().get(row).map(|row| row.path.clone())
                } else {
                    None
                }
            });
            if self.shell.poll_tree_loads() {
                self.side.hover = None;
                if let Some(path) = rename_path {
                    if let Some(row) = self.shell.rows().iter().position(|row| row.path == path) {
                        if let Some(editing) = &mut self.side.editing {
                            editing.kind = EditKind::Rename { row };
                        }
                    } else {
                        self.side.editing = None;
                    }
                }
            }
            return;
        }
        if id == platform::TIMER_WINDOW_MOTION {
            return;
        }
        if id == platform::TIMER_PROVIDER_INTERACTION {
            if self
                .settings_tab()
                .is_some_and(|(tab, section)| Self::is_provider_settings(&tab, &section))
            {
                self.prefs.providers.interaction.tick(Self::now_ms());
            } else {
                self.prefs.providers.interaction = Default::default();
            }
            return;
        }
        if id == platform::TIMER_NAVIGATION_DRAG {
            self.navigation_library_drag_timer();
            return;
        }
        if id == platform::TIMER_BASE_EXPORT {
            self.poll_base_export();
            return;
        }
        if id == platform::TIMER_WORKFLOWS {
            self.workflows_timer();
            return;
        }
        if id == platform::TIMER_NOTIFICATIONS {
            self.save_notifications();
            return;
        }
        if id == platform::TIMER_BASE_AUTOMATION {
            self.automation_timer();
            return;
        }
        if id == platform::TIMER_DOCUMENT_LAYOUT {
            self.doc.advance_loading(self.editor_area);
            self.split.doc.advance_loading(self.split.area);
            self.finish_other_chunk_navigation();
            if let Some(cursor) = self
                .shell
                .active()
                .and_then(|tab| tab.buffer())
                .map(|buffer| buffer.display_cursor())
            {
                if self.doc.take_ready_loading_caret(cursor) {
                    self.after_doc_selection_change(true);
                }
            }
            return;
        }
        if id == platform::TIMER_RESOURCE {
            self.on_resource_timer(hwnd);
            return;
        }
        if id == platform::TIMER_WORD_COUNT {
            self.on_word_count_timer();
            return;
        }
        if id == platform::TIMER_AI_ANIMATION {
            self.ai.animation_timer_armed = false;
            let now = mochi_core::jstime::now_millis();
            let stalled = self.ai.panel.streaming.as_mut().is_some_and(|streaming| {
                streaming.animation_phase = streaming.animation_phase.wrapping_add(1) % 8;
                now.saturating_sub(streaming.last_event_at_ms.max(streaming.started_at_ms))
                    > 120_000
            });
            if stalled {
                if let Some(run) = self.ai.run.take() {
                    run.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.ai.panel.error = "AI 响应超时，已停止本次生成，请重试。".into();
                self.publish_notification(
                    crate::ui::notifications::Category::Assistant,
                    "AI 响应超时",
                    "已停止本次生成，请重试。",
                );
                self.ai_finish_streaming(None);
            }
            return;
        }
        if id == platform::TIMER_VERSION_JOB {
            self.take_version_history_result();
            return;
        }
        if id == platform::TIMER_AI_LOCATOR {
            self.ai.panel.located = None;
            self.ai.panel.locator_pending = None;
            return;
        }
        if id == platform::TIMER_TOAST {
            // 后台任务的进度提示只会在任务结束时由接收方替换，不能被先前一条
            // 普通提示遗留的计时器误清掉。
            if !self.status_bar.toast.is_progress() {
                self.status_bar.toast.message.clear();
            }
            return;
        }
        if id == platform::TIMER_EDITOR_AI {
            self.start_inline_prediction(false);
            return;
        }
        if id == platform::TIMER_POMODORO {
            self.on_pomodoro_tick();
            return;
        }
        if id == platform::TIMER_AUTOSAVE {
            self.on_autosave_timer();
            return;
        }
        if id != platform::TIMER_SEARCH {
            return;
        }
        let Some(s) = self.search.as_ref() else {
            return;
        };
        let query = s.trimmed_query();
        if query.is_empty() {
            return;
        }
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let options = s.options.clone();
        let root = ws.root.clone();
        let index = std::sync::Arc::clone(&ws.index);
        self.search_job.invalidate();
        let generation = self.search_job.generation;
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.search_job.cancel = Some(cancel.clone());
        let (tx, rx) = channel();
        self.search_job.rx = Some(rx);
        let hwnd_raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            let result =
                SearchService::new(&root, index).search_cancellable(&query, &options, &cancel);
            if !cancel.load(std::sync::atomic::Ordering::Relaxed)
                && tx.send((generation, result)).is_ok()
            {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut _)),
                        platform::WM_APP_SEARCH_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }
}
