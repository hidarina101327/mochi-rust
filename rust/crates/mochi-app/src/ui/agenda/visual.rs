//! 日程待办的颜色语言。
//!
//! - 种类只用色相区分：日程蓝、任务靛、重复绿、愿望琥珀、目标紫；
//! - 状态只用形态区分：已执行变淡带勾、未执行加删除线、待确认琥珀描边、正在进行加粗描边。

use super::*;

#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub entry: u32,
    pub task: u32,
    pub routine: u32,
    pub wish: u32,
    pub goal: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
    pub muted: u32,
    pub now: u32,
}

pub fn colors(p: &Palette) -> Colors {
    let mut colors = if theme::is_dark(p) {
        Colors {
            entry: 0x60A5FA,
            task: 0x818CF8,
            routine: 0x34D399,
            wish: 0xFBBF24,
            goal: 0xA78BFA,
            success: 0x4ADE80,
            warning: 0xFBBF24,
            danger: 0xF87171,
            muted: 0x94A3B8,
            now: 0xF87171,
        }
    } else {
        Colors {
            entry: 0x2563EB,
            task: 0x4F46E5,
            routine: 0x059669,
            wish: 0xD97706,
            goal: 0x7C3AED,
            success: 0x16A34A,
            warning: 0xD97706,
            danger: 0xDC2626,
            muted: 0x64748B,
            now: 0xEF4444,
        }
    };
    colors.entry = crate::ui::settings_values::color("schedule.entryColor", colors.entry);
    colors.task = crate::ui::settings_values::color("schedule.taskColor", colors.task);
    colors.routine = crate::ui::settings_values::color("schedule.routineColor", colors.routine);
    colors.wish = crate::ui::settings_values::color("schedule.wishColor", colors.wish);
    colors.goal = crate::ui::settings_values::color("schedule.goalColor", colors.goal);
    colors.success = crate::ui::settings_values::color("schedule.successColor", colors.success);
    colors.warning = crate::ui::settings_values::color("schedule.warningColor", colors.warning);
    colors.danger = crate::ui::settings_values::color("schedule.dangerColor", colors.danger);
    colors.muted = crate::ui::settings_values::color("schedule.mutedColor", colors.muted);
    colors.now = crate::ui::settings_values::color("schedule.nowColor", colors.now);
    colors
}

/// 用户可选的块颜色。
pub const SWATCHES: &[(&str, &str)] = &[
    ("#3B82F6", "蓝"),
    ("#6366F1", "靛"),
    ("#10B981", "绿"),
    ("#F59E0B", "琥珀"),
    ("#EF4444", "红"),
    ("#EC4899", "粉"),
    ("#8B5CF6", "紫"),
    ("#64748B", "灰"),
];

pub fn parse_color(value: &str) -> Option<u32> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

pub fn kind_color(c: &Colors, kind: Kind) -> u32 {
    match kind {
        Kind::Wish => c.wish,
        Kind::Goal => c.goal,
        Kind::Task => c.task,
        Kind::Entry => c.entry,
        Kind::Routine => c.routine,
        Kind::Project => c.muted,
    }
}

pub fn kind_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Wish => Icon::SPARKLES,
        Kind::Goal => Icon::TARGET,
        Kind::Task => Icon::LIST_TODO,
        Kind::Entry => Icon::CALENDAR_DAYS,
        Kind::Routine => Icon::REPEAT,
        Kind::Project => Icon::FOLDER_KANBAN,
    }
}

/// 时间块的底色：自定义颜色 > 项目颜色 > 按来源（任务 / 重复 / 独立日程）。
pub fn slot_color(data: &AgendaData, slot: &query::Slot, c: &Colors) -> u32 {
    if let Some(color) = slot.color.as_deref().and_then(parse_color) {
        return color;
    }
    if let Some(color) = slot
        .project_id
        .as_deref()
        .and_then(|id| data.project(id))
        .and_then(|p| p.color.as_deref())
        .and_then(parse_color)
    {
        return color;
    }
    if slot.routine_id.is_some() {
        c.routine
    } else if slot.task_id.is_some() {
        c.task
    } else {
        c.entry
    }
}

pub fn entry_status_color(c: &Colors, status: EntryStatus) -> u32 {
    match status {
        EntryStatus::Planned => c.entry,
        EntryStatus::Done => c.success,
        EntryStatus::Skipped => c.danger,
        EntryStatus::Cancelled => c.muted,
    }
}

pub fn priority_color(c: &Colors, priority: Priority) -> Option<u32> {
    match priority {
        Priority::Urgent => Some(c.danger),
        Priority::High => Some(c.warning),
        _ => None,
    }
}

/// 小胶囊，返回占用的宽度（含右侧间距）。
pub fn badge(list: &mut DrawList, x: f32, y: f32, label: &str, color: u32, p: &Palette) -> f32 {
    let w = text::measure(label, TextStyle::Tiny) + 12.0;
    let r = Rect::new(x, y, x + w, y + 18.0);
    list.rounded_rect(r, 9.0, theme::mix(color, p.surface, 0.16));
    list.text_aligned(r, label, TextStyle::Tiny, color, Align::Center);
    w + 6.0
}

/// 圆形勾选框。
pub fn check_circle(list: &mut DrawList, r: Rect, checked: bool, color: u32, p: &Palette) {
    let size = r.width().min(r.height()).min(16.0);
    let c = Rect::from_size(
        r.left + (r.width() - size) / 2.0,
        r.top + (r.height() - size) / 2.0,
        size,
        size,
    );
    if checked {
        list.rounded_rect(c, size / 2.0, color);
        list.icon_centered(c, Icon::CHECK, size - 5.0, p.background);
    } else {
        list.rounded_border(c, size / 2.0, color);
    }
}

/// 细进度条。
pub fn progress_bar(list: &mut DrawList, r: Rect, ratio: f32, color: u32, p: &Palette) {
    list.rounded_rect(r, r.height() / 2.0, p.surface_muted);
    let w = r.width() * ratio.clamp(0.0, 1.0);
    if w > 0.5 {
        list.rounded_rect(
            Rect::new(r.left, r.top, r.left + w, r.bottom),
            r.height() / 2.0,
            color,
        );
    }
}
