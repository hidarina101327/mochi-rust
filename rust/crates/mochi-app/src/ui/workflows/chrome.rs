//! 绘制工作流编辑器的标题、分区和按钮外观。
use super::*;
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    text,
    theme::Palette,
};

pub(super) fn icon_button(
    list: &mut DrawList,
    s: &mut State,
    r: Rect,
    icon: Icon,
    label: &str,
    hit: Hit,
    p: &Palette,
) {
    if s.hover.as_ref() == Some(&hit) {
        list.rounded_rect(r, 7., p.surface_muted);
    }
    list.icon_centered(r, icon, 17., p.foreground);
    s.tooltips.push((r, label.into()));
    s.hits.push((r, hit));
}

pub(super) fn header(list: &mut DrawList, area: Rect, s: &mut State, p: &Palette) -> f32 {
    let snapshot = s.run.is_some();
    let active = s
        .run
        .as_ref()
        .is_some_and(|r| matches!(r.status.as_str(), "queued" | "running"));
    let mut actions = if snapshot {
        vec![
            (Icon::FILE_TEXT, "运行结果", Hit::ResultOverview),
            (Icon::HISTORY, "运行历史", Hit::History),
            (Icon::MAXIMIZE2, "显示完整流程", Hit::Fit),
        ]
    } else {
        vec![
            (Icon::ROTATE_CCW, "撤销 · Ctrl+Z", Hit::Undo),
            (Icon::ROTATE_CW, "重做 · Ctrl+Y", Hit::Redo),
            (Icon::NETWORK, "自动布局", Hit::AutoLayout),
            (Icon::SAVE, "保存 · Ctrl+S", Hit::Save),
            (Icon::CHECK_CIRCLE2, "检查流程", Hit::Validate),
            (Icon::HISTORY, "运行历史", Hit::History),
            (Icon::MORE_HORIZONTAL, "更多操作", Hit::More),
        ]
    };
    if active {
        actions.push((Icon::SQUARE, "停止运行", Hit::CancelRun));
    }
    let action_width = actions.len() as f32 * 36. + if snapshot { 0. } else { 84. };
    let narrow = area.width() < action_width + 310.;
    let height = if narrow { 108. } else { 64. };
    list.rect(
        Rect::from_size(area.left, area.top, area.width(), height),
        p.surface,
    );
    let back = Rect::from_size(area.left + 14., area.top + 14., 36., 36.);
    list.rounded_border(back, 8., p.border);
    icon_button(
        list,
        s,
        back,
        Icon::ARROW_LEFT,
        if snapshot {
            "返回编辑画布"
        } else {
            "返回工作流总览"
        },
        if snapshot { Hit::CloseRun } else { Hit::Back },
        p,
    );
    let title_right = if narrow {
        area.right - 20.
    } else {
        area.right - action_width - 32.
    };
    let title = s.graph().map(|g| g.name.as_str()).unwrap_or("工作流");
    list.text(
        Rect::new(area.left + 64., area.top + 10., title_right, area.top + 34.),
        text::ellipsize(title, TextStyle::Title, title_right - area.left - 64.),
        TextStyle::Title,
        p.foreground,
    );
    let subtitle = if let Some(run) = &s.run {
        format!(
            "{} · {}",
            super::results::source_label(&run.source),
            super::painting::status(&run.status)
        )
    } else if s.dirty {
        "有未保存修改".into()
    } else if s.selected.as_ref().is_some_and(|s| s.enabled) {
        "定时运行已启用".into()
    } else {
        "工作流编辑器".into()
    };
    list.text(
        Rect::new(area.left + 64., area.top + 34., title_right, area.top + 54.),
        subtitle,
        TextStyle::Tiny,
        p.muted,
    );
    let mut x = if narrow {
        area.left + 14.
    } else {
        area.right - action_width - 16.
    };
    let y = area.top + if narrow { 64. } else { 14. };
    for (icon, label, hit) in actions {
        icon_button(
            list,
            s,
            Rect::from_size(x, y, 32., 36.),
            icon,
            label,
            hit,
            p,
        );
        x += 36.;
    }
    if !snapshot {
        let r = Rect::from_size(x + 8., y, 76., 36.);
        list.rounded_rect(
            r,
            8.,
            if s.hover == Some(Hit::Run) {
                p.muted
            } else {
                p.foreground
            },
        );
        list.icon_centered(
            Rect::from_size(r.left + 10., y, 20., 36.),
            Icon::PLAY,
            15.,
            p.surface,
        );
        list.text(
            Rect::from_size(r.left + 35., y, 35., 36.),
            "运行",
            TextStyle::Caption,
            p.surface,
        );
        s.hits.push((r, Hit::Run));
    }
    list.hline(area.left, area.right, area.top + height, p.border);
    area.top + height
}

pub(super) fn tooltip(list: &mut DrawList, s: &State, p: &Palette) {
    if s.drag.is_some() {
        return;
    }
    if let Some((r, label)) = s
        .tooltips
        .iter()
        .rev()
        .find(|(r, _)| r.contains(s.pointer.0, s.pointer.1))
    {
        let w = text::measure(label, TextStyle::Caption) + 24.;
        let x = r.left.clamp(
            s.area.left + 8.,
            (s.area.right - w - 8.).max(s.area.left + 8.),
        );
        let y = if r.bottom + 42. < s.area.bottom {
            r.bottom + 6.
        } else {
            r.top - 38.
        };
        let rect = Rect::from_size(x, y, w, 30.);
        list.rounded_rect(rect, 6., p.foreground);
        list.text_aligned(rect, label, TextStyle::Caption, p.surface, Align::Center);
    }
}

pub(super) fn panel_header(
    list: &mut DrawList,
    s: &mut State,
    icon: Icon,
    title: &str,
    p: &Palette,
) {
    let r = s.inspector;
    list.icon_centered(
        Rect::from_size(r.left + 16., r.top + 16., 24., 24.),
        icon,
        17.,
        p.muted,
    );
    list.text(
        Rect::new(r.left + 50., r.top + 14., r.right - 88., r.top + 42.),
        text::ellipsize(title, TextStyle::Title, r.width() - 138.),
        TextStyle::Title,
        p.foreground,
    );
    icon_button(
        list,
        s,
        Rect::from_size(r.right - 44., r.top + 12., 32., 32.),
        Icon::X,
        "关闭面板",
        Hit::ClosePanel,
        p,
    );
    list.hline(r.left, r.right, r.top + 56., p.border);
}

pub(super) fn row(
    list: &mut DrawList,
    s: &mut State,
    r: Rect,
    icon: Icon,
    title: &str,
    detail: &str,
    hit: Hit,
    danger: bool,
    p: &Palette,
) {
    let color = if danger { p.danger } else { p.foreground };
    if s.hover.as_ref() == Some(&hit) {
        list.rounded_rect(r, 8., p.surface_muted);
    }
    list.icon_centered(
        Rect::from_size(r.left + 10., r.top + 14., 24., 24.),
        icon,
        17.,
        color,
    );
    list.text(
        Rect::new(r.left + 46., r.top + 7., r.right - 30., r.top + 30.),
        text::ellipsize(title, TextStyle::Label, r.width() - 80.),
        TextStyle::Label,
        color,
    );
    list.text(
        Rect::new(r.left + 46., r.top + 31., r.right - 30., r.top + 49.),
        text::ellipsize(detail, TextStyle::Tiny, r.width() - 80.),
        TextStyle::Tiny,
        p.muted,
    );
    list.icon_centered(
        Rect::from_size(r.right - 26., r.top + 16., 20., 20.),
        Icon::CHEVRON_RIGHT,
        14.,
        p.muted,
    );
    s.hits.push((r.intersect(&s.panel_body), hit));
}

pub(super) fn settings(list: &mut DrawList, s: &mut State, p: &Palette) {
    panel_header(list, s, Icon::SETTINGS2, "流程设置", p);
    let r = s.inspector;
    s.panel_body = Rect::new(r.left, r.top + 57., r.right, r.bottom);
    let enabled = s.selected.as_ref().is_some_and(|v| v.enabled);
    let rows = [
        (
            Icon::PENCIL_LINE,
            "名称与描述",
            "编辑流程信息",
            Hit::Rename,
            false,
        ),
        (
            Icon::CLOCK,
            "定时触发",
            "设置执行时间与频率",
            Hit::Trigger,
            false,
        ),
        (
            if enabled { Icon::PAUSE } else { Icon::PLAY },
            if enabled {
                "暂停定时"
            } else {
                "启用定时"
            },
            "仅影响后续定时任务",
            if enabled { Hit::Pause } else { Hit::Authorize },
            false,
        ),
        (
            Icon::CODE,
            "工作流 JSON",
            "查看或编辑完整定义",
            Hit::Definition,
            false,
        ),
        (
            Icon::FILE_DOWN,
            "导出工作流",
            "保存为可拖入导入的 ZIP 包",
            Hit::Export,
            false,
        ),
        (
            Icon::TRASH2,
            "删除工作流",
            "移除此流程及其配置",
            Hit::DeleteFlow,
            true,
        ),
    ];
    s.panel_height = rows.len() as f32 * 64. + 24.;
    s.panel_scroll = s
        .panel_scroll
        .clamp(0., (s.panel_height - s.panel_body.height()).max(0.));
    list.push_clip(s.panel_body);
    for (i, (icon, title, detail, hit, danger)) in rows.into_iter().enumerate() {
        let row_rect = Rect::from_size(
            r.left + 10.,
            s.panel_body.top + 10. + i as f32 * 64. - s.panel_scroll,
            r.width() - 20.,
            58.,
        );
        row(list, s, row_rect, icon, title, detail, hit, danger, p);
    }
    list.pop_clip();
}
