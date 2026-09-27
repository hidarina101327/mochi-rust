//! 绘制工作流画布、节点、检查器和运行状态。
use super::*;
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    theme::Palette,
};

pub(super) fn button(
    list: &mut DrawList,
    s: &mut State,
    r: Rect,
    label: &str,
    hit: Hit,
    p: &Palette,
    primary: bool,
) {
    if primary {
        list.rounded_rect(
            r,
            8.,
            if s.hover.as_ref() == Some(&hit) {
                p.muted
            } else {
                p.foreground
            },
        );
    } else if s.hover.as_ref() == Some(&hit) {
        list.rounded_rect(r, 8., p.surface_muted);
    }
    list.text_aligned(
        r,
        label,
        TextStyle::Caption,
        if primary { p.surface } else { p.foreground },
        Align::Center,
    );
    s.hits.push((r, hit));
}
pub fn paint(list: &mut DrawList, area: Rect, s: &mut State, focused: bool, p: &Palette) {
    let neutral = super::super::theme::workflow_palette(p);
    let p = &neutral;
    s.area = area;
    s.hits.clear();
    s.canvas = Rect::ZERO;
    s.field_rect = Rect::ZERO;
    s.search_rect = Rect::ZERO;
    if s.needs_fit && area.width() < 1100. {
        s.palette = false;
    }
    list.rect(area, p.background);
    list.push_clip(area);
    s.tooltips.clear();
    s.panel_body = Rect::ZERO;
    if s.draft.is_none() {
        list.text(
            Rect::from_size(area.left + 24., area.top + 12., 240., 28.),
            "工作流",
            TextStyle::Title,
            p.foreground,
        );
        list.text(
            Rect::from_size(area.left + 24., area.top + 42., 500., 22.),
            "将重复的工作交给流程，结果留在工作区。",
            TextStyle::Caption,
            p.muted,
        );
        library(list, area, s, p);
        list.pop_clip();
        return;
    }
    let top = super::chrome::header(list, area, s, p) - 42.;
    let width = if s.editor == Some(Editor::Definition) || s.editor == Some(Editor::Import) {
        (area.width() * 0.52).max(320.)
    } else {
        s.panel_width
    }
    .min((area.width() - 240.).max(220.));
    let show_panel = s.editor.is_some()
        || s.history_open
        || s.settings_open
        || (s.run.is_some() && !s.result_hidden);
    s.inspector = if show_panel {
        Rect::new(area.right - width, top + 43., area.right, area.bottom - 32.)
    } else {
        Rect::ZERO
    };
    let docked = s.palette && area.width() >= 1100.;
    s.palette_rect = if docked {
        Rect::new(area.left, top + 43., area.left + 238., area.bottom - 32.)
    } else {
        Rect::ZERO
    };
    s.canvas = Rect::new(
        if docked {
            s.palette_rect.right
        } else {
            area.left
        },
        top + 43.,
        if show_panel {
            s.inspector.left
        } else {
            area.right
        },
        area.bottom - 32.,
    );
    if s.needs_fit {
        s.fit();
        if s.zoom < 0.55 {
            if let Some(bounds) = s.bounds() {
                s.zoom = 0.65;
                s.pan = (
                    36. - bounds.left * s.zoom,
                    (s.canvas.height() - bounds.height() * s.zoom).max(80.) * 0.5
                        - bounds.top * s.zoom,
                );
                s.minimap = true;
            }
        }
        s.needs_fit = false;
    }
    super::modern::graph(list, s, p);
    if show_panel {
        inspector(list, s, focused, p);
        s.hits.push((
            Rect::new(
                s.inspector.left - 3.,
                s.inspector.top,
                s.inspector.left + 3.,
                s.inspector.bottom,
            ),
            Hit::Resize,
        ));
    }
    if s.palette {
        super::forms::palette(list, s, p);
    } else {
        let r = Rect::from_size(s.canvas.left + 14., s.canvas.top + 14., 36., 36.);
        list.rounded_rect(r, 8., p.surface_elevated);
        list.rounded_border(r, 8., p.border);
        super::chrome::icon_button(
            list,
            s,
            r,
            crate::ui::icons::Icon::BOXES,
            "展开节点库",
            Hit::Add,
            p,
        );
    }
    if let Some(Drag::Template(i, sx, sy)) = s.drag {
        if (s.pointer.0 - sx).abs() + (s.pointer.1 - sy).abs() > 6. {
            if let Some(kind) = mochi_core::workflows::catalog::KINDS.get(i) {
                let r = Rect::from_size(s.pointer.0 + 14., s.pointer.1 + 12., 150., 38.);
                list.rounded_rect(r, 8., p.surface_elevated);
                list.rounded_border(r, 8., p.accent);
                list.text_aligned(
                    r,
                    mochi_core::workflows::catalog::label(kind),
                    TextStyle::Label,
                    p.foreground,
                    Align::Center,
                );
            }
        }
    }
    let note = format!(
            "{} 节点 · {} 连线  |  拖动平移 · Shift 多选/框选 · Ctrl+A 全选 · 滚轮缩放 · Delete 删除 · Ctrl+Z 撤销",
            s.graph().map_or(0, |g| g.nodes.len()),
            s.graph().map_or(0, |g| g.edges.len())
        );
    list.text(
        Rect::new(
            area.left + 20.,
            area.bottom - 30.,
            area.right - 15.,
            area.bottom - 3.,
        ),
        &note,
        TextStyle::Caption,
        p.muted,
    );
    super::chrome::tooltip(list, s, p);
    list.pop_clip();
}
fn library(list: &mut DrawList, area: Rect, s: &mut State, p: &Palette) {
    super::library::paint(list, area, s, p);
    if s.editor == Some(Editor::Import) {
        s.inspector = Rect::new(
            area.left + 24.,
            area.top + 92.,
            area.right - 24.,
            area.bottom - 35.,
        );
        inspector(list, s, true, p);
    }
}
fn inspector(list: &mut DrawList, s: &mut State, focused: bool, p: &Palette) {
    let r = s.inspector;
    list.rect(r, p.surface);
    list.border_left(r, p.border);
    list.push_clip(r);
    if matches!(s.editor, Some(Editor::Node | Editor::Parameter))
        && !s.advanced
        && s.run.is_none()
        && !s.history_open
    {
        super::forms::inspector(list, s, focused, p);
        list.pop_clip();
        return;
    }
    if s.history_open {
        super::results::history(list, s, p);
    } else if s.run.is_some() {
        super::results::paint(list, s, p);
    } else if let Some(editor) = s.editor {
        let label = match editor {
            Editor::Folder => "文件夹名称",
            Editor::Node | Editor::Parameter => "节点参数",
            Editor::Definition => "工作流 JSON",
            Editor::Import => "导入工作流 · ZIP / JSON",
            Editor::Trigger => "触发规则",
            Editor::RunInput => "本次输入",
            Editor::Rename => "名称与描述",
            Editor::Result => "节点执行结果",
        };
        list.text(
            Rect::new(r.left + 16., r.top + 15., r.right - 50., r.top + 42.),
            label,
            TextStyle::Label,
            p.foreground,
        );
        button(
            list,
            s,
            Rect::from_size(r.right - 42., r.top + 9., 32., 32.),
            "×",
            Hit::CloseEditor,
            p,
            false,
        );
        s.field_rect = Rect::new(r.left + 12., r.top + 54., r.right - 12., r.bottom - 66.);
        s.field
            .paint_multiline(list, s.field_rect, focused && editor != Editor::Result, p);
        if editor != Editor::Result {
            button(
                list,
                s,
                Rect::from_size(
                    r.left + 12.,
                    r.bottom - 52.,
                    if editor == Editor::Node {
                        (r.width() - 40.) / 3.
                    } else {
                        76.
                    },
                    34.,
                ),
                if editor == Editor::RunInput {
                    "运行"
                } else {
                    "应用"
                },
                Hit::Apply,
                p,
                true,
            );
        } else {
            button(
                list,
                s,
                Rect::from_size(r.left + 12., r.bottom - 52., 84., 34.),
                "复制结果",
                Hit::CopyResult,
                p,
                false,
            );
        }
        if editor == Editor::Node {
            button(
                list,
                s,
                Rect::from_size(
                    r.left + 20. + (r.width() - 40.) / 3.,
                    r.bottom - 52.,
                    (r.width() - 40.) / 3.,
                    34.,
                ),
                "引用对象",
                Hit::PickObject,
                p,
                false,
            );
            button(
                list,
                s,
                Rect::from_size(
                    r.right - 12. - (r.width() - 40.) / 3.,
                    r.bottom - 52.,
                    (r.width() - 40.) / 3.,
                    34.,
                ),
                "删除",
                Hit::Delete,
                p,
                false,
            );
        }
        if editor == Editor::Import {
            button(
                list,
                s,
                Rect::from_size(r.left + 104., r.bottom - 52., 108., 34.),
                "从文件读取",
                Hit::ImportFile,
                p,
                false,
            );
        }
        if editor == Editor::Result && s.result_text.chars().count() > 8000 {
            let count = s.result_text.chars().count().div_ceil(8000);
            button(
                list,
                s,
                Rect::from_size(r.left + 102., r.bottom - 52., 28., 34.),
                "‹",
                Hit::ResultPage(false),
                p,
                false,
            );
            list.text_aligned(
                Rect::from_size(r.left + 132., r.bottom - 52., 44., 34.),
                &format!("{}/{}", s.result_page + 1, count),
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
            button(
                list,
                s,
                Rect::from_size(r.left + 178., r.bottom - 52., 28., 34.),
                "›",
                Hit::ResultPage(true),
                p,
                false,
            );
        }
    } else {
        super::chrome::settings(list, s, p);
    }
    list.pop_clip();
}
pub(super) fn status(value: &str) -> &str {
    match value {
        "queued" => "排队中",
        "pending" => "等待",
        "running" => "执行中",
        "succeeded" => "成功",
        "failed" => "失败",
        "skipped" => "已跳过",
        "cancelled" => "已取消",
        "interrupted" => "已中断",
        "partial" => "部分失败",
        _ => value,
    }
}
