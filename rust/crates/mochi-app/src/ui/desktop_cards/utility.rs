use super::*;
use crate::ui::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    theme::Palette,
};

pub(super) fn enabled(module: ModuleKind) -> bool {
    matches!(
        module,
        ModuleKind::Weather | ModuleKind::Music | ModuleKind::Search
    )
}
pub(super) fn controls(state: &State, area: Rect) -> Vec<(Rect, Hit)> {
    if state
        .selected_page_ref()
        .is_some_and(|p| p.module != ModuleKind::Music)
    {
        vec![(
            Rect::from_size(area.left, area.top + 26.0, area.width(), 36.0),
            Hit::EditPreference(Field::Utility),
        )]
    } else {
        vec![]
    }
}
pub(super) fn paint(list: &mut DrawList, state: &mut State, area: Rect, p: &Palette) {
    let Some(page) = state.selected_page_ref().cloned() else {
        return;
    };
    let (label,value,hint)=match page.module {
        ModuleKind::Weather=>("城市",page.utility.location.as_str(),"填写城市名称后保存。天气来自 Open-Meteo，显示实时、逐小时及七日预报。"),
        ModuleKind::Search=>("搜索关键词",page.utility.query.as_str(),"搜索墨池内容和 Everything 文件索引。Everything 需单独安装并保持运行；单个来源不可用时其余结果仍会显示。"),
        _=>("Windows 媒体会话","","在支持系统媒体控制的播放器中播放音乐；卡片提供播放/暂停、上一首、下一首、进度跳转和播放来源切换。"),
    };
    list.push_clip(area);
    list.text(
        Rect::from_size(area.left, area.top, area.width(), 22.0),
        label,
        TextStyle::Caption,
        p.muted,
    );
    if page.module != ModuleKind::Music {
        super::folder::paint_input(
            list,
            state,
            Rect::from_size(area.left, area.top + 26.0, area.width(), 36.0),
            Field::Utility,
            value,
            "输入内容",
            p,
        );
    }
    let hint_top = area.top
        + if page.module == ModuleKind::Music {
            34.0
        } else {
            78.0
        };
    for (index, runs) in crate::ui::text::wrap_source(hint, TextStyle::Body, area.width().max(1.0))
        .into_iter()
        .enumerate()
    {
        let line = runs.into_iter().map(|run| run.text).collect::<String>();
        list.text(
            Rect::from_size(
                area.left,
                hint_top + index as f32 * 24.0,
                area.width(),
                24.0,
            ),
            line,
            TextStyle::Body,
            p.muted,
        );
    }
    list.pop_clip();
}
