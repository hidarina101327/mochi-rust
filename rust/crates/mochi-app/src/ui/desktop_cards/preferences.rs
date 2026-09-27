//! 定义桌面卡片可编辑的偏好项及其控件和值。
use super::{Field, Hit, ModuleKind, State};
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    theme::Palette,
    widgets::FieldLook,
};
use mochi_core::desktop_cards::CompletedBehavior;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Names,
    Modified,
    Foreground,
    Background,
    Padding,
    SingleTitle,
    TabsLeft,
    TabsRatio,
    TabsDivider,
    Dock,
    DockSpeed,
    Pinned,
    Icons,
    Extensions,
    Numbers,
    Groups,
    HeadingSize,
    Grid,
    Columns,
    Rows,
    GridLines,
    GridHeight,
    ScheduleView,
    Completed,
    Checks,
    CalendarExpanded,
    ExpandLibraries,
}

pub fn settings(state: &State) -> Vec<Setting> {
    use Setting::*;
    if state.editor_tab == 0 {
        return vec![
            Padding,
            SingleTitle,
            TabsLeft,
            TabsRatio,
            TabsDivider,
            Dock,
            DockSpeed,
            Pinned,
        ];
    }
    if state
        .selected_page_ref()
        .is_some_and(|p| super::model::interactive(p.module))
    {
        return vec![Foreground, Background];
    }
    if state
        .selected_page_ref()
        .is_some_and(|p| p.module == ModuleKind::Shortcuts)
    {
        return vec![
            Names, Columns, GridLines, GridHeight, Icons, Foreground, Background,
        ];
    }
    if state
        .selected_page_ref()
        .is_some_and(|p| p.module == ModuleKind::Folder)
    {
        return vec![
            Grid, Columns, Rows, GridLines, GridHeight, Names, Modified, Icons, Extensions,
            Numbers, Foreground, Background,
        ];
    }
    let mut rows = vec![];
    if let Some(page) = state.selected_page_ref() {
        match page.module {
            ModuleKind::Document
            | ModuleKind::Home
            | ModuleKind::Automations
            | ModuleKind::Favorites
            | ModuleKind::Recent
            | ModuleKind::Base
            | ModuleKind::Canvas
            | ModuleKind::Exam => rows.extend([Grid, Columns, Rows, GridLines, GridHeight]),
            ModuleKind::Schedule => rows.extend([
                ScheduleView,
                Completed,
                Checks,
                Groups,
                HeadingSize,
                CalendarExpanded,
            ]),
            ModuleKind::Inbox => rows.extend([Groups, HeadingSize]),
            ModuleKind::Knowledge => rows.push(ExpandLibraries),
            _ => {}
        }
    }
    if state.selected_page_ref().is_some_and(|p| {
        matches!(
            p.module,
            ModuleKind::Document
                | ModuleKind::Base
                | ModuleKind::Canvas
                | ModuleKind::Recent
                | ModuleKind::Favorites
                | ModuleKind::Exam
        )
    }) {
        rows.push(Modified);
    }
    rows.extend([Icons, Extensions, Numbers, Foreground, Background]);
    rows
}
pub fn count(state: &State) -> usize {
    settings(state).len()
        + if state
            .selected_page_ref()
            .is_some_and(|p| p.module == ModuleKind::Schedule && p.presentation.schedule_view == 2)
        {
            state.selected_page_ref().unwrap().presentation.groups.len() * 2 + 2
        } else {
            0
        }
}
pub fn controls(state: &State, row: Rect, index: usize) -> Vec<(Rect, Hit)> {
    let settings = settings(state);
    if let Some(setting) = settings.get(index) {
        if matches!(setting, Setting::Foreground | Setting::Background) {
            let bg = *setting == Setting::Background;
            return vec![
                (
                    Rect::new(
                        row.right - 168.0,
                        row.top + 5.0,
                        row.right - 40.0,
                        row.bottom - 5.0,
                    ),
                    Hit::ItemColor(bg, None),
                ),
                (
                    Rect::from_size(row.right - 36.0, row.top + 6.0, 28.0, 28.0),
                    Hit::ResetItemColor(bg, None),
                ),
            ];
        }
        let mut hits = vec![(
            Rect::new(
                row.right - 168.0,
                row.top + 5.0,
                row.right - 8.0,
                row.bottom - 5.0,
            ),
            Hit::Preference(*setting, 1),
        )];
        if matches!(
            setting,
            Setting::Padding
                | Setting::TabsRatio
                | Setting::HeadingSize
                | Setting::Columns
                | Setting::Rows
                | Setting::GridHeight
        ) {
            hits = vec![
                (
                    Rect::from_size(row.right - 168.0, row.top + 6.0, 30.0, 30.0),
                    Hit::Preference(*setting, -1),
                ),
                (
                    Rect::from_size(row.right - 38.0, row.top + 6.0, 30.0, 30.0),
                    Hit::Preference(*setting, 1),
                ),
            ];
        }
        return hits;
    }
    let index = index - settings.len();
    let groups = state
        .selected_page_ref()
        .map_or(0, |p| p.presentation.groups.len());
    if index < groups * 2 {
        let group = index / 2;
        let field = if index % 2 == 0 {
            Field::GroupName(group)
        } else {
            Field::GroupRule(group)
        };
        let mut hits = vec![(
            Rect::new(
                row.left + 78.0,
                row.top + 4.0,
                row.right - 44.0,
                row.bottom - 4.0,
            ),
            Hit::EditPreference(field),
        )];
        if index % 2 == 0 {
            hits.push((
                Rect::from_size(row.right - 36.0, row.top + 8.0, 26.0, 26.0),
                Hit::RemoveGroup(group),
            ));
        }
        return hits;
    }
    if index == groups * 2 {
        return vec![(
            Rect::from_size(row.left, row.top + 4.0, 150.0, 32.0),
            Hit::AddGroup,
        )];
    }
    vec![]
}
pub fn value(state: &State, setting: Setting) -> (&'static str, String) {
    use Setting::*;
    let card = state.selected_card_ref().unwrap();
    let a = &card.appearance;
    let p = &state.selected_page_ref().unwrap().presentation;
    let yes = |b| if b { "显示" } else { "隐藏" }.to_owned();
    match setting {
        Names => ("显示名称", yes(p.show_names)),
        Modified => ("小字修改时间", yes(p.show_modified)),
        Foreground => (
            "所有对象的文字颜色",
            p.item_foreground
                .map_or("跟随外观".into(), |c| format!("#{c:06X}")),
        ),
        Background => (
            "所有对象的背景颜色",
            p.item_background
                .map_or("无填充".into(), |c| format!("#{c:06X}")),
        ),
        Padding => ("单项上下留白", format!("{} px", a.row_padding)),
        SingleTitle => ("单分页名称", yes(a.show_single_page_name)),
        TabsLeft => ("分页位置", if a.tabs_left { "左边" } else { "顶部" }.into()),
        TabsRatio => (
            "分页 / 主视图",
            if a.tabs_ratio == 0 {
                "自动".into()
            } else {
                format!("{} : {}", a.tabs_ratio, 100 - a.tabs_ratio)
            },
        ),
        TabsDivider => ("分页分割线", yes(a.tabs_divider)),
        Checks => ("完成勾选框", yes(p.show_checks)),
        Completed => (
            "已完成的行为",
            match state.selected_page_ref().unwrap().completed_behavior() {
                CompletedBehavior::Hide => "消失",
                CompletedBehavior::Strike => "划掉",
                CompletedBehavior::Keep => "保留",
            }
            .into(),
        ),
        Dock => (
            "靠边自动收纳",
            if a.edge_dock { "开启" } else { "关闭" }.into(),
        ),
        DockSpeed => (
            "鼠标离开后收纳",
            ["立即", "快", "慢", "缓慢"][a.dock_speed.min(3) as usize].into(),
        ),
        GridLines => ("网格内框线", yes(p.grid_lines)),
        GridHeight => (
            "网格单元高度",
            if p.grid_height == 0 {
                "自动".into()
            } else {
                format!("{} px", p.grid_height)
            },
        ),
        Pinned => (
            "固定在前方",
            if a.pinned { "固定" } else { "不固定" }.into(),
        ),
        Icons => ("条目图标", yes(p.show_icons)),
        Extensions => ("文档后缀名", yes(p.show_extensions)),
        Numbers => ("列表序号", yes(p.show_numbers)),
        Groups => ("分组小标题", yes(p.show_groups)),
        HeadingSize => ("小标题字号", format!("{} px", p.heading_size)),
        Grid => ("入口布局", if p.grid { "网格" } else { "列表" }.into()),
        Columns => ("网格列数", p.columns.to_string()),
        Rows => ("网格行数", p.rows.to_string()),
        ScheduleView => (
            "日程显示形式",
            ["日期列表", "日历网格", "自定义列表"][p.schedule_view as usize].into(),
        ),
        CalendarExpanded => (
            "日历每周高度",
            if p.calendar_expanded {
                "随内容展开"
            } else {
                "固定高度收纳"
            }
            .into(),
        ),
        ExpandLibraries => (
            "点击知识库",
            if p.expand_libraries {
                "展开目录树"
            } else {
                "进入墨池知识库"
            }
            .into(),
        ),
    }
}
pub fn change(state: &mut State, setting: Setting, delta: i8) {
    use Setting::*;
    let Some(card) = state.selected_card_mut() else {
        return;
    };
    let a = &mut card.appearance;
    match setting {
        Padding => a.row_padding = (a.row_padding as i16 + delta as i16).clamp(0, 64) as u8,
        SingleTitle => a.show_single_page_name = !a.show_single_page_name,
        TabsLeft => a.tabs_left = !a.tabs_left,
        TabsRatio => a.tabs_ratio = (a.tabs_ratio as i16 + delta as i16 * 5).clamp(0, 60) as u8,
        TabsDivider => a.tabs_divider = !a.tabs_divider,
        Dock => a.edge_dock = !a.edge_dock,
        DockSpeed => a.dock_speed = (a.dock_speed + 1) % 4,
        Pinned => a.pinned = !a.pinned,
        _ => {}
    }
    if let Some(page) = state.selected_page_mut() {
        let completed_behavior = page.completed_behavior();
        let p = &mut page.presentation;
        match setting {
            Completed => {
                p.completed_behavior = Some(match completed_behavior {
                    CompletedBehavior::Hide => CompletedBehavior::Strike,
                    CompletedBehavior::Strike => CompletedBehavior::Keep,
                    CompletedBehavior::Keep => CompletedBehavior::Hide,
                })
            }
            Names => p.show_names = !p.show_names,
            Modified => p.show_modified = !p.show_modified,
            Icons => p.show_icons = !p.show_icons,
            Checks => p.show_checks = !p.show_checks,
            Extensions => p.show_extensions = !p.show_extensions,
            Numbers => p.show_numbers = !p.show_numbers,
            Groups => p.show_groups = !p.show_groups,
            HeadingSize => {
                p.heading_size = (p.heading_size as i16 + delta as i16).clamp(8, 36) as u8
            }
            GridLines => p.grid_lines = !p.grid_lines,
            GridHeight => {
                p.grid_height = (p.grid_height as i32 + delta as i32 * 8).clamp(0, 600) as u16
            }
            Grid => p.grid = !p.grid,
            Columns => p.columns = (p.columns as i16 + delta as i16).clamp(1, 12) as u8,
            Rows => p.rows = (p.rows as i16 + delta as i16).clamp(1, 12) as u8,
            ScheduleView => p.schedule_view = (p.schedule_view + 1) % 3,
            CalendarExpanded => p.calendar_expanded = !p.calendar_expanded,
            ExpandLibraries => p.expand_libraries = !p.expand_libraries,
            _ => {}
        }
    }
    state.dirty = true;
}
pub fn paint(list: &mut DrawList, state: &mut State, row: Rect, index: usize, p: &Palette) {
    let settings = settings(state);
    if let Some(setting) = settings.get(index) {
        let (label, value) = value(state, *setting);
        list.text(
            Rect::new(row.left + 8.0, row.top, row.right - 176.0, row.bottom),
            label,
            TextStyle::Label,
            p.foreground,
        );
        for (r, hit) in controls(state, row, index) {
            if state.hover == Some(hit) {
                list.rounded_rect(r, 4.0, p.surface_muted);
            }
            if let Hit::ItemColor(bg, _) = hit {
                let presentation = &state.selected_page_ref().unwrap().presentation;
                let color = if bg {
                    presentation.item_background
                } else {
                    presentation.item_foreground
                };
                list.rounded_rect(
                    Rect::from_size(r.left + 5.0, r.top + 8.0, 16.0, 16.0),
                    4.0,
                    color.unwrap_or(p.surface),
                );
                list.rounded_border(
                    Rect::from_size(r.left + 5.0, r.top + 8.0, 16.0, 16.0),
                    4.0,
                    p.border,
                );
            }
            if matches!(hit, Hit::ResetItemColor(..)) {
                list.icon_centered(r, Icon::ROTATE_CCW, 12.0, p.muted);
            }
            if let Hit::Preference(_, delta) = hit {
                if matches!(
                    setting,
                    Setting::Padding
                        | Setting::TabsRatio
                        | Setting::HeadingSize
                        | Setting::Columns
                        | Setting::Rows
                        | Setting::GridHeight
                ) {
                    list.icon_centered(
                        r,
                        if delta < 0 { Icon::MINUS } else { Icon::PLUS },
                        12.0,
                        p.muted,
                    );
                } else {
                    list.icon_centered(
                        Rect::from_size(r.right - 22.0, r.top + 8.0, 16.0, 16.0),
                        Icon::CHEVRON_RIGHT,
                        12.0,
                        p.muted,
                    );
                }
            }
        }
        list.text_aligned(
            Rect::new(row.right - 160.0, row.top, row.right - 32.0, row.bottom),
            value,
            TextStyle::Caption,
            p.foreground,
            Align::Center,
        );
        list.hline(row.left + 8.0, row.right - 8.0, row.bottom, p.border);
        return;
    }
    for (r, hit) in controls(state, row, index) {
        match hit {
            Hit::EditPreference(field) => {
                let (label, text) = match field {
                    Field::GroupName(i) => (
                        "分组名",
                        state.selected_page_ref().unwrap().presentation.groups[i]
                            .name
                            .clone(),
                    ),
                    Field::GroupRule(i) => (
                        "规则",
                        state.selected_page_ref().unwrap().presentation.groups[i]
                            .rule
                            .clone(),
                    ),
                    _ => continue,
                };
                list.text(
                    Rect::new(row.left + 8.0, row.top, r.left, row.bottom),
                    label,
                    TextStyle::Caption,
                    p.muted,
                );
                if state.focus_field == Some(field) {
                    state
                        .preference_text
                        .paint(list, r, true, p, FieldLook::dialog(p));
                } else {
                    list.rounded_border(r, 4.0, p.border);
                    list.text(
                        Rect::new(r.left + 8.0, r.top, r.right - 8.0, r.bottom),
                        crate::ui::text::ellipsize(&text, TextStyle::Label, r.width() - 16.0),
                        TextStyle::Label,
                        p.foreground,
                    );
                }
            }
            Hit::RemoveGroup(_) => list.icon_centered(r, Icon::X, 14.0, p.muted),
            Hit::AddGroup => list.text(r, "+ 添加分组", TextStyle::Label, p.accent),
            _ => {}
        }
    }
    if index
        == settings.len() + state.selected_page_ref().unwrap().presentation.groups.len() * 2 + 1
    {
        list.text(row, "属性：日期 / 优先级 / 状态 / 标题 / time.startAt\n常数：今日 / 昨日 / 明日；比较：= != < > <= >=；组合：&& ||", TextStyle::Caption, p.muted);
    }
}
