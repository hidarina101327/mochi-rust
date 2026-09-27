//! 格式化状态栏中的运行模式、资源用量和状态说明。
use super::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::{self, Palette},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMode {
    Software,
    /// D2D 的 DEFAULT 目标会优先尝试硬件，但驱动不可用时可能自行回退。
    #[default]
    HardwarePreferred,
}

impl RenderMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Software => "软件渲染",
            Self::HardwarePreferred => "硬件优先",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Software => "D2D 软件渲染（低内存模式）",
            Self::HardwarePreferred => "D2D 硬件优先（默认目标；驱动不可用时可能回退软件）",
        }
    }
}

/// 状态栏使用的资源快照。采样层故意不依赖这个 UI 类型，便于单独测试 Win32 读取。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResourceUsage {
    pub cpu_percent: Option<f32>,
    pub working_set_bytes: Option<u64>,
    pub private_bytes: Option<u64>,
    pub thread_count: Option<u32>,
    pub render_mode: RenderMode,
}

impl ResourceUsage {
    /// 状态栏上的短文案；私有内存留在悬浮信息中，避免挤压字数和 AI 状态。
    pub fn compact_label(self) -> String {
        format!(
            "CPU {} · RAM {} · {} · {}",
            format_cpu(self.cpu_percent),
            format_megabytes(self.working_set_bytes),
            self.thread_count
                .map(|value| format!("{value}线程"))
                .unwrap_or_else(|| "线程 --".to_owned()),
            self.render_mode.label(),
        )
    }

    /// 可供窗口层接入悬浮提示的完整信息，不包含任何会误导用户的 GPU 利用率。
    pub fn detail(self) -> String {
        format!(
            "CPU {} · 工作集 {} · 私有内存 {} · {} · {}",
            format_cpu(self.cpu_percent),
            format_megabytes(self.working_set_bytes),
            format_megabytes(self.private_bytes),
            self.thread_count
                .map(|value| format!("线程 {value}"))
                .unwrap_or_else(|| "线程未知".to_owned()),
            self.render_mode.detail(),
        )
    }
}

fn format_cpu(value: Option<f32>) -> String {
    match value.filter(|value| value.is_finite()) {
        Some(value) if value < 10.0 => format!("{value:.1}%"),
        Some(value) => format!("{value:.0}%"),
        None => "--".to_owned(),
    }
}

fn format_megabytes(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "--".to_owned();
    };
    let megabytes = value as f64 / (1024.0 * 1024.0);
    if megabytes >= 1024.0 {
        format!("{:.1}G", megabytes / 1024.0)
    } else {
        format!("{megabytes:.0}M")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Source,
    BlockMode,
    InlinePrediction,
    Annotations,
    Ai,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InlinePredictionStatus {
    pub enabled: bool,
    pub running: bool,
    pub available: bool,
}

#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    /// 资源短文案的盒子；不参与点击，供窗口层做悬浮详情命中。
    pub resource_rect: Option<Rect>,
    pub resource_detail: Option<String>,
    /// 字数盒子只用于布局验证和悬浮命中后的重排检查。
    pub counts_rect: Option<Rect>,
    /// 位于 CPU 资源信息左侧的预测运行状态。
    pub prediction_status_rect: Option<Rect>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }

    pub fn resource_detail_at(&self, x: f32, y: f32) -> Option<&str> {
        self.resource_rect
            .filter(|rect| rect.contains(x, y))
            .and(self.resource_detail.as_deref())
    }
}

/// 没有资源快照时保留原有调用接口，测试和离屏验证无需构造 Win32 数据。
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    counts: Option<(usize, usize)>,
    source: Option<bool>,
    annotations: Option<bool>,
    ai: bool,
    p: &Palette,
) -> Layout {
    paint_with_resources(list, area, counts, source, annotations, ai, None, None, p)
}

/// 绘制状态栏。资源区与字数区从 AI 按钮左侧反向分配，窄窗口空间不足时优先保留
/// 资源短文案并隐藏字数，任何情况下都不让文本盒子越过彼此或覆盖 AI 按钮。
pub fn paint_with_resources(
    list: &mut DrawList,
    area: Rect,
    counts: Option<(usize, usize)>,
    source: Option<bool>,
    annotations: Option<bool>,
    ai: bool,
    inline_prediction: Option<InlinePredictionStatus>,
    resources: Option<&ResourceUsage>,
    p: &Palette,
) -> Layout {
    let mut layout = Layout::default();
    list.push_clip(area);
    list.hline(area.left, area.right, area.top, p.border);
    let mut x = area.left + 16.0;
    let mut controls = Vec::new();
    if let Some(source) = source {
        controls.extend([
            ("源码", Hit::Source, source),
            (
                if super::editor_preferences::current().live_line_source {
                    "块源码"
                } else {
                    "所见即所得"
                },
                Hit::BlockMode,
                !source,
            ),
        ]);
        if let Some(status) = inline_prediction {
            controls.push((
                if status.enabled {
                    "AI预测：开"
                } else {
                    "AI预测：关"
                },
                Hit::InlinePrediction,
                status.enabled,
            ));
        }
    }
    if let Some(visible) = annotations {
        controls.push((
            if visible {
                "隐藏笔迹"
            } else {
                "显示笔迹"
            },
            Hit::Annotations,
            visible,
        ));
    }
    for (label, hit, on) in controls {
        let width = text::measure(label, TextStyle::Caption) + 12.0;
        let r = Rect::new(x, area.top + 1.0, x + width, area.bottom);
        // 给右侧的资源、字数和 AI 至少留出一块区域；窗口再窄时右侧会自行隐藏。
        if r.right > (area.right - 220.0).max(area.left) {
            break;
        }
        if on {
            list.rounded_rect(r, 3.0, theme::mix(p.accent, p.area_main_default, 0.10));
        }
        list.text_aligned(
            r,
            label,
            TextStyle::Caption,
            if on { p.accent } else { p.muted },
            Align::Center,
        );
        layout.entries.push((r, hit));
        x += width + 6.0;
    }

    let ai_left = (area.right - 43.0).max(area.left);
    let ai_right = (area.right - 12.0).max(ai_left);
    let ai_rect = Rect::new(ai_left, area.top, ai_right, area.bottom);
    layout.entries.push((ai_rect, Hit::Ai));
    list.text_aligned(
        ai_rect,
        "AI",
        TextStyle::Caption,
        if ai { p.accent } else { p.muted },
        Align::Center,
    );

    let counts_label = counts
        .map(|(words, chars)| format!("{words} 字 / {chars} 字符    UTF-8"))
        .unwrap_or_default();
    let left_edge = (x + 8.0).min(area.right);
    let mut right = (ai_rect.left - 8.0).max(left_edge);

    // 先计算所需宽度，空间不够时让资源区占满可用空间并把字数盒子隐藏。
    if let Some(resource) = resources {
        let label = resource.compact_label();
        let desired = (text::measure(&label, TextStyle::Caption) + 12.0).clamp(78.0, 230.0);
        let counts_desired = if counts_label.is_empty() {
            0.0
        } else {
            (text::measure(&counts_label, TextStyle::Caption) + 8.0).min(190.0)
        };
        let available = (right - left_edge).max(0.0);
        let show_both = !counts_label.is_empty() && available >= desired + counts_desired + 8.0;
        let reserved_for_counts = if show_both { counts_desired + 8.0 } else { 0.0 };
        let resource_max = (available - reserved_for_counts).max(0.0);
        if resource_max >= 78.0 {
            let width = desired.min(resource_max);
            let rect = Rect::new(right - width, area.top, right, area.bottom);
            if !rect.is_empty() && rect.left >= left_edge {
                list.text_aligned(
                    rect,
                    text::ellipsize(&label, TextStyle::Caption, rect.width()),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
                layout.resource_rect = Some(rect);
                layout.resource_detail = Some(resource.detail());
                right = (rect.left - 8.0).max(left_edge);
            }
        }
    }

    // 预测状态紧贴在 CPU 资源区左侧。即使预测关闭也保留明确状态，避免用户
    // 把“没有补全结果”误认为请求卡住。
    if let Some(status) = inline_prediction {
        let label = if !status.enabled {
            "AI 预测关闭"
        } else if status.running {
            "AI 预测中…"
        } else if status.available {
            "AI 预测就绪"
        } else {
            "AI 预测空闲"
        };
        let desired = text::measure(label, TextStyle::Caption) + 8.0;
        let available = (right - left_edge).max(0.0);
        if available >= 64.0 {
            let width = desired.min(available);
            let rect = Rect::new(right - width, area.top, right, area.bottom);
            list.text_aligned(
                rect,
                text::ellipsize(label, TextStyle::Caption, rect.width()),
                TextStyle::Caption,
                if status.running || status.available {
                    p.accent
                } else {
                    p.muted
                },
                Align::Trailing,
            );
            layout.prediction_status_rect = Some(rect);
            right = (rect.left - 8.0).max(left_edge);
        }
    }

    if !counts_label.is_empty() && right > left_edge {
        let rect = Rect::new(left_edge, area.top, right, area.bottom);
        if !rect.is_empty() {
            list.text_aligned(
                rect,
                text::ellipsize(&counts_label, TextStyle::Caption, rect.width()),
                TextStyle::Caption,
                p.muted,
                Align::Trailing,
            );
            layout.counts_rect = Some(rect);
        }
    }
    list.pop_clip();
    layout
}

/// 资源短文案的悬浮详情。调用方应在普通状态栏绘制之后调用，
/// 这样详情框可以浮在状态栏上方而不会被状态栏自己的 clip 截掉。
pub fn paint_resource_tooltip(
    list: &mut DrawList,
    viewport: Rect,
    anchor: Rect,
    detail: &str,
    p: &Palette,
) {
    const HEIGHT: f32 = 34.0;
    const GAP: f32 = 6.0;
    let available = (viewport.width() - 16.0).max(0.0);
    if available < 80.0 || detail.is_empty() {
        return;
    }
    let width = (text::measure(detail, TextStyle::Caption) + 24.0)
        .min(520.0)
        .min(available)
        .max(80.0);
    let left_min = viewport.left + 8.0;
    let left_max = (viewport.right - 8.0 - width).max(left_min);
    let left = (anchor.right - width).clamp(left_min, left_max);
    let top = anchor.top - HEIGHT - GAP;
    if top < viewport.top {
        return;
    }
    let rect = Rect::new(left, top, left + width, top + HEIGHT);
    list.rounded_rect(rect, 6.0, p.surface_elevated);
    list.rounded_border(rect, 6.0, p.border);
    list.text_aligned(
        Rect::new(rect.left + 12.0, rect.top, rect.right - 12.0, rect.bottom),
        text::ellipsize(detail, TextStyle::Caption, rect.width() - 24.0),
        TextStyle::Caption,
        p.foreground,
        Align::Center,
    );
}

#[derive(Default)]
pub struct Toast {
    pub message: String,
    pub expires: i64,
    /// `Some` 表示进行中的任务：不走自动消失计时器，直到任务自己给出结果。
    pub progress: Option<(usize, usize)>,
}

impl Toast {
    pub fn progress(message: impl Into<String>, processed: usize, total: usize) -> Self {
        Self {
            message: message.into(),
            expires: i64::MAX,
            progress: Some((processed, total)),
        }
    }

    pub fn is_progress(&self) -> bool {
        self.progress.is_some()
    }

    pub fn paint(&self, list: &mut DrawList, area: Rect, now: i64, p: &Palette) {
        if self.message.is_empty() || now >= self.expires {
            return;
        }
        let error = self.message.contains("失败")
            || self.message.contains("错误")
            || self.message.contains("无法");
        let progress = self.progress;
        // 任务提示固定为紧凑的信息卡，避免进度数字每次变化时盒子跳动；普通提示
        // 仍由内容决定宽度，且不会遮住右侧工作区。
        let max_width = (area.width() - 32.0).max(0.0).min(360.0);
        if max_width < 80.0 || area.height() < 64.0 {
            return;
        }
        let width = if progress.is_some() {
            288.0_f32.min(max_width)
        } else {
            (self
                .message
                .lines()
                .map(|line| text::measure(line, TextStyle::Label))
                .fold(0.0_f32, f32::max)
                + 60.0)
                .clamp(144.0_f32.min(max_width), max_width)
        };
        let lines: Vec<_> = self
            .message
            .lines()
            .flat_map(|line| {
                if line.is_empty() {
                    vec![Vec::new()]
                } else {
                    text::wrap_source(line, TextStyle::Label, width - 60.0)
                }
            })
            .collect();
        let padding = if progress.is_some() { 60.0 } else { 24.0 };
        let max_lines = (((area.height() - 32.0 - padding) / 22.0).floor() as usize).min(6);
        if max_lines == 0 {
            return;
        }
        let line_count = lines.len().max(1).min(max_lines);
        let message_height = line_count as f32 * 22.0;
        let height = if progress.is_some() {
            60.0 + message_height
        } else {
            24.0 + message_height
        };
        let r = Rect::new(
            area.right - 16.0 - width,
            area.top + 16.0,
            area.right - 16.0,
            area.top + 16.0 + height,
        );
        let tone = if error { p.danger } else { p.muted };
        list.rounded_rect_alpha(
            Rect::new(r.left - 2.0, r.top + 2.0, r.right + 2.0, r.bottom + 4.0),
            12.0,
            0x000000,
            0.025,
        );
        list.rounded_rect_alpha(
            Rect::new(r.left - 1.0, r.top + 1.0, r.right + 1.0, r.bottom + 3.0),
            10.0,
            0x000000,
            0.04,
        );
        list.rounded_rect(r, 10.0, p.surface_elevated);
        list.rounded_border(r, 10.0, theme::mix(p.border, p.surface_elevated, 0.35));
        let badge = Rect::new(r.left + 12.0, r.top + 11.0, r.left + 36.0, r.top + 35.0);
        list.icon_centered(
            badge,
            if error {
                Icon::X
            } else if progress.is_some() {
                Icon::LOADER2
            } else {
                Icon::INFO
            },
            17.0,
            tone,
        );
        for (index, runs) in lines.iter().take(line_count).enumerate() {
            let top = r.top + 12.0 + index as f32 * 22.0;
            let mut line: String = runs.iter().map(|run| run.text.as_str()).collect();
            if index + 1 == line_count && lines.len() > line_count {
                line = text::ellipsize(&(line + "…"), TextStyle::Label, width - 60.0);
            }
            list.text(
                Rect::new(r.left + 46.0, top, r.right - 14.0, top + 22.0),
                line,
                TextStyle::Label,
                p.foreground,
            );
        }
        if let Some((processed, total)) = progress {
            let detail = format!("{processed} / {total}");
            list.text(
                Rect::new(
                    r.left + 14.0,
                    r.bottom - 40.0,
                    r.right - 14.0,
                    r.bottom - 21.0,
                ),
                detail,
                TextStyle::Caption,
                p.muted,
            );
            let track = Rect::new(
                r.left + 14.0,
                r.bottom - 13.0,
                r.right - 14.0,
                r.bottom - 8.0,
            );
            let fraction = if total == 0 {
                1.0
            } else {
                processed as f32 / total as f32
            }
            .clamp(0.0, 1.0);
            list.rounded_rect_alpha(track, 3.0, p.foreground, 0.10);
            if fraction > 0.0 {
                list.rounded_rect(
                    Rect::new(
                        track.left,
                        track.top,
                        track.left + track.width() * fraction,
                        track.bottom,
                    ),
                    3.0,
                    tone,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_expires_and_status_hits_stay_inside_bar() {
        let p = theme::tokens().palette(false);
        let mut list = DrawList::new();
        let area = Rect::new(0.0, 0.0, 800.0, 600.0);
        let toast = Toast {
            message: "已保存".into(),
            expires: 100,
            progress: None,
        };
        toast.paint(&mut list, area, 100, p);
        assert!(list.cmds().is_empty());
        let bar = Rect::new(0.0, 576.0, 800.0, 600.0);
        let layout = paint(&mut list, bar, Some((12, 24)), Some(false), None, true, p);
        assert!(layout
            .entries
            .iter()
            .all(|(r, _)| r.top >= bar.top && r.bottom <= bar.bottom));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn toast_is_anchored_to_the_global_top_right() {
        let p = theme::tokens().palette(false);
        let mut list = DrawList::new();
        let area = Rect::new(12.0, 8.0, 812.0, 608.0);
        Toast {
            message: "已复制".into(),
            expires: 200,
            progress: None,
        }
        .paint(&mut list, area, 100, p);
        let background = list
            .cmds()
            .iter()
            .find_map(|command| match command {
                crate::ui::draw::DrawCmd::RoundedRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("visible toast background");
        assert_eq!(background.right, area.right - 16.0);
        assert_eq!(background.top, area.top + 16.0);
        assert!(background.bottom < area.top + area.height() / 2.0);
    }

    #[test]
    fn short_toast_uses_a_compact_content_sized_width() {
        let p = theme::tokens().palette(false);
        let mut list = DrawList::new();
        Toast {
            message: "已复制".into(),
            expires: 200,
            progress: None,
        }
        .paint(&mut list, Rect::new(0.0, 0.0, 800.0, 600.0), 100, p);
        let background = list
            .cmds()
            .iter()
            .find_map(|command| match command {
                crate::ui::draw::DrawCmd::RoundedRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("visible toast background");
        assert_eq!(background.width(), 144.0);
    }

    #[test]
    fn progress_toast_keeps_a_stable_card_and_draws_a_progress_track() {
        let p = theme::tokens().palette(false);
        let mut list = DrawList::new();
        Toast::progress("正在建立搜索索引", 12, 48).paint(
            &mut list,
            Rect::new(0.0, 0.0, 800.0, 600.0),
            100,
            p,
        );
        let backgrounds: Vec<_> = list
            .cmds()
            .iter()
            .filter_map(|command| match command {
                crate::ui::draw::DrawCmd::RoundedRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert!(backgrounds
            .iter()
            .any(|rect| rect.width() == 288.0 && rect.height() == 82.0));
        assert!(backgrounds
            .iter()
            .any(|rect| (rect.width() - 65.0).abs() < 0.1));
    }

    #[test]
    fn long_notice_wraps_inside_the_available_content_area() {
        let p = theme::tokens().palette(false);
        let mut list = DrawList::new();
        let area = Rect::new(0.0, 80.0, 320.0, 280.0);
        Toast {
            message: "当前按名称排序\n切换为手动排序后，可拖拽调整顺序\n".repeat(20),
            expires: 200,
            progress: None,
        }
        .paint(&mut list, area, 100, p);
        let background = list
            .cmds()
            .iter()
            .find_map(|cmd| match cmd {
                crate::ui::draw::DrawCmd::RoundedRect { rect, .. } => Some(*rect),
                _ => None,
            })
            .unwrap();
        let lines: Vec<_> = list
            .cmds()
            .iter()
            .filter_map(|cmd| match cmd {
                crate::ui::draw::DrawCmd::Text { rect, text, .. } => Some((rect, text)),
                _ => None,
            })
            .collect();
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|(_, text)| !text.contains('\n')));
        assert!(background.top >= area.top + 16.0 && background.bottom <= area.bottom - 16.0);
        assert!(lines
            .iter()
            .all(|(rect, _)| rect.bottom <= background.bottom));
        assert!(lines.last().unwrap().1.ends_with('…'));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn non_document_tabs_do_not_offer_editor_or_pdf_actions() {
        let mut list = DrawList::new();
        let layout = paint(
            &mut list,
            Rect::new(0.0, 0.0, 800.0, 24.0),
            None,
            None,
            None,
            false,
            theme::tokens().palette(false),
        );
        assert_eq!(layout.entries.len(), 1);
        assert!(layout.entries.iter().all(|(_, hit)| *hit == Hit::Ai));
    }

    #[test]
    fn resource_and_counts_boxes_do_not_overlap_in_a_wide_bar() {
        let mut list = DrawList::new();
        let usage = ResourceUsage {
            cpu_percent: Some(2.5),
            working_set_bytes: Some(48 * 1024 * 1024),
            private_bytes: Some(42 * 1024 * 1024),
            thread_count: Some(12),
            render_mode: RenderMode::Software,
        };
        let layout = paint_with_resources(
            &mut list,
            Rect::new(0.0, 0.0, 900.0, 24.0),
            Some((120, 240)),
            Some(false),
            None,
            true,
            None,
            Some(&usage),
            theme::tokens().palette(false),
        );
        if let (Some(resource), Some(counts)) = (layout.resource_rect, layout.counts_rect) {
            assert!(resource.intersect(&counts).is_empty());
        }
        assert!(layout
            .resource_detail_at(layout.resource_rect.unwrap().left, 12.0)
            .is_some());
        assert!(list.finish().is_ok());
    }

    #[test]
    fn narrow_bar_hides_or_ellipsizes_without_crossing_ai() {
        let mut list = DrawList::new();
        let usage = ResourceUsage {
            cpu_percent: None,
            working_set_bytes: Some(48 * 1024 * 1024),
            private_bytes: None,
            thread_count: Some(4),
            render_mode: RenderMode::HardwarePreferred,
        };
        let bar = Rect::new(0.0, 0.0, 300.0, 24.0);
        let layout = paint_with_resources(
            &mut list,
            bar,
            Some((120, 240)),
            Some(false),
            Some(false),
            true,
            None,
            Some(&usage),
            theme::tokens().palette(false),
        );
        let ai = layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == Hit::Ai)
            .map(|(rect, _)| *rect)
            .unwrap();
        if let Some(resource) = layout.resource_rect {
            assert!(resource.right <= ai.left);
            assert!(resource.left >= bar.left);
        }
        if let Some(counts) = layout.counts_rect {
            assert!(counts.right <= ai.left);
            assert!(counts.left >= bar.left);
        }
        assert!(list.finish().is_ok());
    }

    #[test]
    fn inline_prediction_toggle_follows_wysiwyg_and_running_state_precedes_cpu() {
        let mut list = DrawList::new();
        let usage = ResourceUsage {
            cpu_percent: Some(1.5),
            working_set_bytes: Some(64 * 1024 * 1024),
            private_bytes: Some(60 * 1024 * 1024),
            thread_count: Some(8),
            render_mode: RenderMode::HardwarePreferred,
        };
        let status = InlinePredictionStatus {
            enabled: true,
            running: true,
            available: false,
        };
        let layout = paint_with_resources(
            &mut list,
            Rect::new(0.0, 0.0, 1200.0, 24.0),
            Some((10, 20)),
            Some(false),
            None,
            true,
            Some(status),
            Some(&usage),
            theme::tokens().palette(false),
        );
        let block = layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == Hit::BlockMode)
            .map(|(rect, _)| *rect)
            .unwrap();
        let toggle = layout
            .entries
            .iter()
            .find(|(_, hit)| *hit == Hit::InlinePrediction)
            .map(|(rect, _)| *rect)
            .unwrap();
        assert!(toggle.left >= block.right);
        let prediction = layout.prediction_status_rect.unwrap();
        let resource = layout.resource_rect.unwrap();
        assert!(prediction.right <= resource.left);
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::Text { text, color, .. }
                if text == "AI 预测中…" && *color == theme::tokens().palette(false).accent
        )));
        assert_eq!(
            layout.hit(toggle.left + 1.0, 12.0),
            Some(Hit::InlinePrediction)
        );
        assert!(list.finish().is_ok());
    }
}
