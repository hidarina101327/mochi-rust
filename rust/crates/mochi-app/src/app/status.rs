//! 维护状态栏所需的资源使用情况和字数统计状态。
use super::*;

/// 资源指标在窗口可见时每 5 秒刷新一次；不可见/最小化时降到 30 秒，
/// 避免后台窗口为了状态栏而持续唤醒消息循环。
pub(super) const RESOURCE_SAMPLE_ACTIVE_MS: u32 = 5_000;
pub(super) const RESOURCE_SAMPLE_IDLE_MS: u32 = 30_000;
/// 文字统计仍在 UI 线程计算，但让连续输入合并成一次全文扫描。
pub(super) const WORD_COUNT_DEBOUNCE_MS: u32 = 400;

pub(super) struct State {
    pub layout: crate::ui::status_bar::Layout,
    pub stats: Option<(PathBuf, usize, usize)>,
    pub toast: crate::ui::status_bar::Toast,
    pub timer: Option<u32>,
    pub(super) resources: resource_usage::Sampler,
    resource_timer: Option<u32>,
    word_count_timer: Option<u32>,
    word_count_pending: bool,
    resource_hover: bool,
    last_status: String,
}

impl Default for State {
    fn default() -> Self {
        // 内存/线程首帧就可见；CPU 会等第二次采样才有差分结果。
        let mut resources = resource_usage::Sampler::new();
        resources.sample();
        Self {
            layout: Default::default(),
            stats: None,
            toast: Default::default(),
            timer: None,
            resources,
            resource_timer: Some(RESOURCE_SAMPLE_ACTIVE_MS),
            word_count_timer: None,
            word_count_pending: false,
            resource_hover: false,
            last_status: String::new(),
        }
    }
}

impl State {
    /// 内容变了之后调用；真正的全文扫描由一次性计时器触发。
    pub(super) fn invalidate_word_count(&mut self) {
        // 保留同一文件的旧快照，连续输入时状态栏不会先闪成空白；计时器到点
        // 再用当前文本替换它。文件切换由 paint_status 另行清掉不匹配的快照。
        self.word_count_pending = true;
        // 这是对现有一次性 SetTimer 的重设请求。即使上一次请求已经被窗口层
        // 取走、但 WM_TIMER 尚未到达，这里也会让下一次 after_input 把它重置。
        self.word_count_timer = Some(WORD_COUNT_DEBOUNCE_MS);
    }

    /// 状态栏绘制时调用，兼容旧的 `stats = None` 失效点。
    pub(super) fn queue_word_count(&mut self) {
        // `word_count_timer` 被窗口层取走后，实际 Win32 计时器仍然在跑；
        // 不能因为鼠标移动触发的重绘而再次排队，把截止时间无限往后推。
        if !self.word_count_pending {
            self.word_count_pending = true;
            self.word_count_timer = Some(WORD_COUNT_DEBOUNCE_MS);
        }
    }

    pub(super) fn word_count_timer_request(&mut self) -> Option<u32> {
        self.word_count_timer.take()
    }

    fn update_resource_hover(&mut self, x: f32, y: f32) -> bool {
        let hovered = self.layout.resource_detail_at(x, y).is_some();
        let changed = hovered != self.resource_hover;
        self.resource_hover = hovered;
        changed
    }

    fn resource_timer_request(&mut self) -> Option<u32> {
        self.resource_timer.take()
    }

    fn resource_usage(&self) -> Option<crate::ui::status_bar::ResourceUsage> {
        let snapshot = self.resources.snapshot()?;
        Some(crate::ui::status_bar::ResourceUsage {
            cpu_percent: snapshot.cpu_percent,
            working_set_bytes: snapshot.working_set_bytes,
            private_bytes: snapshot.private_bytes,
            thread_count: snapshot.thread_count,
            render_mode: render_mode(),
        })
    }
}

fn render_mode() -> crate::ui::status_bar::RenderMode {
    if crate::gfx::low_memory_mode() {
        crate::ui::status_bar::RenderMode::Software
    } else {
        // D2D 的 DEFAULT 目标可能在驱动不可用时自行回退；没有 API 确认时，
        // 文案明确说“硬件优先”，不把它误报成已经使用 GPU。
        crate::ui::status_bar::RenderMode::HardwarePreferred
    }
}

fn resource_timer_delay(hwnd: HWND) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
    let background = unsafe { IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() };
    if background {
        RESOURCE_SAMPLE_IDLE_MS
    } else {
        RESOURCE_SAMPLE_ACTIVE_MS
    }
}

impl App {
    /// 显示应用范围内的临时提示。重新启动提示计时器，
    /// 这样重复执行相同操作（例如连续复制两次）时，
    /// 每次都会显示提示，不会被状态去重逻辑过滤。
    pub(super) fn show_global_notice(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.record_status_notification(&message);
        self.state.status_text = message.clone();
        self.status_bar.last_status = message.clone();
        if !self
            .notification_category_enabled(crate::ui::notifications::Category::for_status(&message))
        {
            self.status_bar.toast = Default::default();
            self.status_bar.timer = None;
            return;
        }
        let duration = crate::ui::settings_values::number("general.toastDuration", 1000.0)
            .clamp(500.0, 10000.0) as u32;
        self.status_bar.toast = crate::ui::status_bar::Toast {
            message,
            expires: Self::now_ms() + i64::from(duration),
            progress: None,
        };
        self.status_bar.timer = Some(duration);
    }

    /// 展示后台任务的常驻进度提示。它和普通通知共用同一块右上角空间，因此不会
    /// 再额外叠一条横幅；`last_status` 同步更新，避免下一帧被通用状态提示覆盖。
    pub(super) fn show_global_progress(
        &mut self,
        message: impl Into<String>,
        processed: usize,
        total: usize,
    ) {
        let message = message.into();
        self.state.status_text = format!("{message} · {processed} / {total}");
        self.status_bar.last_status = self.state.status_text.clone();
        self.status_bar.toast = crate::ui::status_bar::Toast::progress(message, processed, total);
        self.status_bar.timer = None;
    }

    /// 供窗口过程把一次性资源刷新意图落成 `SetTimer`。
    pub(super) fn resource_timer_request(&mut self) -> Option<u32> {
        self.status_bar.resource_timer_request()
    }

    /// 资源刷新计时器到点。采样失败只会让对应字段保持未知，不影响其它 UI。
    pub(super) fn on_resource_timer(&mut self, hwnd: HWND) {
        self.status_bar.resources.sample();
        self.status_bar.resource_timer = Some(resource_timer_delay(hwnd));
    }

    /// 供窗口过程把文字统计计时器到点转成一次全文扫描。
    pub(super) fn on_word_count_timer(&mut self) {
        self.status_bar.word_count_timer = None;
        self.status_bar.word_count_pending = false;
        let Some(path) = self.active_file_path() else {
            self.status_bar.stats = None;
            return;
        };
        let Some(buffer) = self.shell.active().and_then(|tab| tab.buffer()) else {
            self.status_bar.stats = None;
            return;
        };
        let text = buffer.text();
        self.status_bar.stats = Some((
            path,
            mochi_core::word_count::count_words(text),
            text.encode_utf16().count(),
        ));
    }

    /// 窗口层在鼠标移动时调用；状态栏本身没有独立的 Win32 窗口，
    /// 所以只记录是否悬停资源区并让窗口决定是否重画。
    pub(super) fn on_resource_hover(&mut self, x: f32, y: f32) -> bool {
        self.status_bar.update_resource_hover(x, y)
    }

    /// 在普通状态栏绘制完成后调用，画出工作集/私有内存/线程等完整信息。
    /// 详情框始终夹在客户区内，窗口太矮时静默隐藏，不遮挡主内容。
    pub(super) fn paint_resource_detail(&mut self, p: &Palette) {
        if self.dialog.is_some()
            || self.menu.is_some()
            || self.search.is_some()
            || self.command.is_some()
            || self.export_form.is_some()
            || self.image_preview.is_some()
            || self.commands.review.is_some()
            || self.base_detail_open()
        {
            return;
        }
        if !self.status_bar.resource_hover {
            return;
        }
        let Some((x, y)) = self.block_pointer else {
            return;
        };
        let Some(detail) = self.status_bar.layout.resource_detail_at(x, y) else {
            return;
        };
        let Some(anchor) = self.status_bar.layout.resource_rect else {
            return;
        };
        crate::ui::status_bar::paint_resource_tooltip(
            &mut self.list,
            self.renderer.viewport(),
            anchor,
            detail,
            p,
        );
    }

    pub(super) fn paint_status(&mut self, chrome: &Chrome, p: &Palette) {
        if self.status_bar.last_status != self.state.status_text {
            self.status_bar.last_status = self.state.status_text.clone();
            let text = self.state.status_text.clone();
            let error = text.contains("失败") || text.contains("错误") || text.contains("无法");
            if !text.is_empty()
                && !text.starts_with("已打开")
                && (error || (!text.ends_with(" · 软件光栅") && !text.ends_with(" · GPU")))
            {
                self.record_status_notification(&text);
                if self.notification_category_enabled(
                    crate::ui::notifications::Category::for_status(&text),
                ) {
                    let duration =
                        crate::ui::settings_values::number("general.toastDuration", 1000.0)
                            .clamp(500.0, 10000.0) as u32;
                    self.status_bar.toast = crate::ui::status_bar::Toast {
                        message: text.clone(),
                        expires: Self::now_ms() + i64::from(duration),
                        progress: None,
                    };
                    self.status_bar.timer = Some(duration);
                } else {
                    self.status_bar.toast = Default::default();
                    self.status_bar.timer = None;
                }
            }
        }
        self.status_bar.layout = Default::default();
        if chrome.tree.is_hidden(chrome.status_bar) {
            return;
        }
        let path = self.active_file_path();
        if self
            .status_bar
            .stats
            .as_ref()
            .is_some_and(|(p, _, _)| Some(p) != path.as_ref())
        {
            self.status_bar.stats = None;
            self.status_bar.word_count_pending = false;
        }
        if self.status_bar.stats.is_none()
            && path.is_some()
            && self.shell.active().and_then(|tab| tab.buffer()).is_some()
        {
            // 旧的编辑路径仍会直接把 stats 置空；在这里统一转成去抖请求。
            self.status_bar.queue_word_count();
        }
        let counts = self.status_bar.stats.as_ref().map(|(_, w, c)| (*w, *c));
        let source = self
            .shell
            .active()
            .filter(|t| t.buffer().is_some())
            .map(|t| t.source_mode());
        let annotations =
            self.shell.active().filter(|t| t.is_pdf()).map(|_| {
                crate::ui::settings_values::boolean("editorLayout.annotationsVisible", true)
            });
        let ai = ai_runtime::load_provider(&self.settings).is_some();
        let inline_prediction = source.map(|_| crate::ui::status_bar::InlinePredictionStatus {
            enabled: self.inline_completion_enabled(),
            running: self.inline_prediction_running(),
            available: self.inline_prediction_available(),
        });
        let resources = self.status_bar.resource_usage();
        self.status_bar.layout = crate::ui::status_bar::paint_with_resources(
            &mut self.list,
            chrome.tree.rect(chrome.status_bar),
            counts,
            source,
            annotations,
            ai,
            inline_prediction,
            resources.as_ref(),
            p,
        );
    }

    pub(super) fn on_status_click(&mut self, x: f32, y: f32) {
        use crate::ui::status_bar::Hit;
        match self.status_bar.layout.hit(x, y) {
            Some(Hit::Source) => self.toggle_source_mode(),
            Some(Hit::BlockMode) => {
                if self.shell.active().is_some_and(|t| t.source_mode()) {
                    self.toggle_source_mode();
                } else {
                    let enabled = !crate::ui::editor_preferences::current().live_line_source;
                    self.settings.set(
                        "app.editorLayout.liveLineSource",
                        if enabled { "true" } else { "false" },
                    );
                    let _ = self.settings.flush();
                    self.apply_setting_side_effects("editorLayout.liveLineSource");
                    self.state.status_text = if enabled {
                        "已切换为当前块源码编辑"
                    } else {
                        "已切换为所见即所得编辑"
                    }
                    .into();
                }
            }
            Some(Hit::InlinePrediction) => {
                let enabled = !self.inline_completion_enabled();
                if let Err(error) = self.set_inline_completion_enabled(enabled) {
                    self.state.status_text = format!("保存 AI 预测设置失败：{error}");
                }
            }
            Some(Hit::Annotations) => {
                let value =
                    !crate::ui::settings_values::boolean("editorLayout.annotationsVisible", true);
                self.settings.set(
                    "app.editorLayout.annotationsVisible",
                    if value { "true" } else { "false" },
                );
                let _ = self.settings.flush();
                self.apply_setting_side_effects("editorLayout.annotationsVisible");
            }
            Some(Hit::Ai) => self.open_settings("ai"),
            None => {}
        }
    }
}
