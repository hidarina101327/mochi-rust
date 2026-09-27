//! 处理桌面卡片组件的计时器、AI 请求、事件和打开目标。
use super::*;
use model::studio::{Action as WidgetAction, Kind, Trigger};
impl App {
    pub(super) fn desktop_widget_live(&self) -> desktop_window::widgets::Live {
        let ai = &self.ai.panel;
        let mut messages: Vec<_> = ai
            .active
            .as_ref()
            .map(|c| {
                c.messages
                    .iter()
                    .filter(|m| matches!(m.role(), "user" | "assistant") && !m.content().is_empty())
                    .map(|m| (m.role().to_owned(), m.content().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(stream) = &ai.streaming {
            if !stream.content.is_empty() {
                messages.push(("assistant".into(), stream.content.clone()));
            }
        }
        desktop_window::widgets::Live {
            now: chrono::Local::now().timestamp(),
            timer: pomodoro::format_duration(self.panels.pomodoro.remaining_now()),
            running: self.panels.pomodoro.running,
            messages,
            sessions: ai
                .sessions
                .iter()
                .map(|s| (s.id.clone(), s.title.clone()))
                .collect(),
            streaming: ai.is_streaming(),
            error: if ai.provider_missing {
                "请先在墨池设置中配置 AI 模型".into()
            } else {
                ai.error.clone()
            },
            title: ai
                .active
                .as_ref()
                .map(|c| c.title.clone())
                .unwrap_or_else(|| "新对话".into()),
        }
    }
    pub(super) fn desktop_timer_action(&mut self, reset: bool) {
        let t = &mut self.panels.pomodoro;
        if reset {
            t.reset();
        } else if t.running {
            t.pause();
        } else {
            t.start();
        }
        self.desktop_sync_windows();
        self.invalidate_main();
    }
    pub(super) fn desktop_ai_send(&mut self, card: &str, text: String) {
        if self.ai.panel.is_streaming() {
            return;
        }
        let saved = self.ai.panel.input.text().to_owned();
        let images = std::mem::take(&mut self.ai.panel.pending_images);
        let files = std::mem::take(&mut self.ai.panel.pending_files);
        let selections = std::mem::take(&mut self.ai.panel.pending_selections);
        self.ai.panel.input.set_text(&text);
        self.ai_send(HWND(self.hwnd_raw as *mut _));
        if self.ai.panel.is_streaming() {
            self.desktop.windows.clear_composer(card);
        }
        self.ai.panel.input.set_text(&saved);
        self.ai.panel.pending_images = images;
        self.ai.panel.pending_files = files;
        self.ai.panel.pending_selections = selections;
        self.desktop_sync_windows();
    }
    pub(super) fn desktop_widget_event(&mut self, index: usize, id: &str, trigger: Trigger) {
        let Some(page) = self.desktop.config.cards[index].active_page().cloned() else {
            return;
        };
        let gesture = matches!(trigger, Trigger::Click | Trigger::DoubleClick);
        let Some(node) = page.studio.nodes.iter().find(|n| n.id == id) else {
            return;
        };
        if trigger == Trigger::DoubleClick && node.kind == Kind::Shortcut {
            self.desktop_open_target(&node.target);
        }
        let mut queue: VecDeque<_> = node
            .events
            .iter()
            .filter(|b| {
                b.trigger == trigger
                    && (trigger != Trigger::Interval
                        || self.desktop.widget_second % b.seconds.max(1) as u64 == 0)
            })
            .cloned()
            .collect();
        let mut count = 0;
        let mut changed = false;
        while let Some(b) = queue.pop_front() {
            count += 1;
            if count > 64 {
                break;
            }
            match b.action {
                WidgetAction::OpenTarget if gesture => self.desktop_open_target(&b.target),
                WidgetAction::OpenModule if gesture => {
                    self.desktop_row_action(
                        &page,
                        model::Row {
                            id: String::new(),
                            title: String::new(),
                            detail: String::new(),
                            checked: None,
                            action: Some(model::Action::OpenModule(b.target)),
                            meta: Default::default(),
                        },
                        false,
                    );
                }
                WidgetAction::SwitchPage if gesture => {
                    if self.desktop.config.cards[index]
                        .pages
                        .iter()
                        .any(|p| p.id == b.target)
                    {
                        self.desktop.config.cards[index].active_page = b.target;
                        changed = true;
                    }
                }
                WidgetAction::ToggleTimer if gesture => self.desktop_timer_action(false),
                WidgetAction::ResetTimer if gesture => self.desktop_timer_action(true),
                WidgetAction::SetText | WidgetAction::ToggleVisible => {
                    if let Some(p) = self.desktop.config.cards[index]
                        .pages
                        .iter_mut()
                        .find(|p| p.id == page.id)
                    {
                        if let Some(n) = p.studio.nodes.iter_mut().find(|n| n.id == b.target) {
                            if b.action == WidgetAction::SetText {
                                n.title = b
                                    .value
                                    .replace(
                                        "${time}",
                                        &chrono::Local::now().format("%H:%M:%S").to_string(),
                                    )
                                    .replace(
                                        "${date}",
                                        &chrono::Local::now().format("%Y-%m-%d").to_string(),
                                    );
                            } else {
                                n.hidden = !n.hidden;
                            }
                            changed = true;
                        }
                    }
                }
                WidgetAction::Emit => {
                    for n in &page.studio.nodes {
                        queue.extend(
                            n.events
                                .iter()
                                .filter(|e| e.trigger == Trigger::Custom && e.name == b.target)
                                .cloned(),
                        );
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.desktop_config_changed();
        }
    }
    pub(super) fn desktop_open_target(&mut self, target: &str) {
        self.desktop_queue_target(target, None);
    }

    pub(super) fn desktop_open_folder_target(
        &mut self,
        folder: &model::folder::FolderConfig,
        target: &str,
    ) {
        self.desktop_queue_target(target, Some(folder.clone()));
    }

    fn desktop_queue_target(&mut self, target: &str, folder: Option<model::folder::FolderConfig>) {
        let value = if folder.is_some() {
            target
        } else {
            target.trim()
        };
        if !(value.starts_with("https://")
            || value.starts_with("http://")
            || Path::new(value).is_absolute())
        {
            return;
        }
        self.desktop_take_target_results();
        if self
            .desktop
            .target_jobs
            .iter()
            .any(|(target, _)| target == value)
        {
            return;
        }
        if self.desktop.target_jobs.len() >= 4 {
            self.desktop_error("正在打开其他项目，请稍后重试".into());
            return;
        }
        let value = value.to_owned();
        let target = value.clone();
        let hwnd = self.hwnd_raw;
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("desktop-open-target".into())
            .spawn(move || {
                use windows::Win32::System::Com::*;
                let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
                // 在后台重新校验映射范围，失效条目和替换为链接的条目不能继续打开。
                let opened = if let Some(config) = folder.as_ref() {
                    model::folder::resolve_entry(config, &target)
                        .is_ok_and(|path| platform::open_external(&path.to_string_lossy()))
                } else {
                    platform::open_external(&target)
                };
                if com {
                    unsafe {
                        CoUninitialize();
                    }
                }
                let _ = tx.send(opened);
                if hwnd != 0 {
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd as *mut _)),
                            desktop_window::shortcut_icon::READY,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            });
        match spawned {
            Ok(_) => self.desktop.target_jobs.push((value, rx)),
            Err(error) => self.desktop_error(format!("无法打开项目：{error}")),
        }
    }
    pub(super) fn desktop_take_target_results(&mut self) {
        let mut failed = false;
        self.desktop
            .target_jobs
            .retain(|(_, rx)| match rx.try_recv() {
                Ok(true) => false,
                Ok(false) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    failed = true;
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
            });
        if failed {
            self.desktop_error("无法打开快捷方式，请检查原文件是否仍存在".into());
        }
    }
    pub(super) fn desktop_widget_intervals(&mut self) {
        if self.desktop.paused {
            return;
        }
        let seconds = chrono::Local::now().timestamp().max(0) as u64;
        if self.desktop.widget_second == seconds {
            return;
        }
        self.desktop.widget_second = seconds;
        let mut events = vec![];
        for (i, c) in self
            .desktop
            .config
            .cards
            .iter()
            .enumerate()
            .filter(|(_, c)| c.enabled)
        {
            if let Some(p) = c.active_page() {
                for n in &p.studio.nodes {
                    if n.events.iter().any(|b| {
                        b.trigger == Trigger::Interval && seconds % b.seconds.max(1) as u64 == 0
                    }) {
                        events.push((i, n.id.clone()));
                    }
                }
            }
        }
        for (i, id) in events {
            self.desktop_widget_event(i, &id, Trigger::Interval);
        }
    }
}
