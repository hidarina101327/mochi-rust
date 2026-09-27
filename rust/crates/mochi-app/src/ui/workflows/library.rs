//! 绘制工作流库及其列表内容。
use super::painting::button;
use super::*;
use crate::ui::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    text,
    theme::Palette,
};

pub fn paint(list: &mut DrawList, area: Rect, s: &mut State, p: &Palette) {
    let compact = area.width() < 760.;
    let top = area.top + if compact { 76. } else { 20. };
    let mut x = if compact {
        area.left + 24.
    } else {
        area.right - 352.
    };
    for (label, hit, width, primary) in [
        ("导入工作流", Hit::Import, 96., false),
        ("＋ 文件夹", Hit::NewFolder, 104., false),
        ("＋ 新建工作流", Hit::New, 120., true),
    ] {
        button(
            list,
            s,
            Rect::from_size(x, top, width, 36.),
            label,
            hit,
            p,
            primary,
        );
        x += width + 8.;
    }
    let nav_y = area.top + if compact { 134. } else { 92. };
    if s.folder.is_some() {
        button(
            list,
            s,
            Rect::from_size(area.left + 24., nav_y, 112., 32.),
            "‹ 全部工作流",
            Hit::Folder(None),
            p,
            false,
        );
        let name = s
            .folders
            .iter()
            .find(|f| Some(&f.id) == s.folder.as_ref())
            .map(|f| f.name.as_str())
            .unwrap_or("文件夹");
        list.text(
            Rect::new(area.left + 152., nav_y, area.right - 24., nav_y + 32.),
            name,
            TextStyle::Label,
            p.foreground,
        );
    } else {
        list.text(
            Rect::new(area.left + 24., nav_y, area.right - 24., nav_y + 32.),
            format!(
                "我的工作流    {} 个流程 · {} 个文件夹",
                s.workflows.len(),
                s.folders.len()
            ),
            TextStyle::Caption,
            p.muted,
        );
    }
    let body = Rect::new(
        area.left + 24.,
        nav_y + 48.,
        area.right - 24.,
        area.bottom - 24.,
    );
    let cols = ((body.width() + 16.) / 300.).floor().max(1.) as usize;
    let width = (body.width() - (cols - 1) as f32 * 16.) / cols as f32;
    let folders = if s.folder.is_none() {
        s.folders.clone()
    } else {
        vec![]
    };
    let flows: Vec<_> = s
        .workflows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.folder_id == s.folder)
        .map(|(i, w)| (i, w.clone()))
        .collect();
    let total = folders.len() + flows.len();
    s.library_height = total.div_ceil(cols) as f32 * 206.;
    s.scroll = s
        .scroll
        .clamp(0., (s.library_height - body.height()).max(0.));
    list.push_clip(body);
    for idx in 0..total {
        let r = Rect::from_size(
            body.left + (idx % cols) as f32 * (width + 16.),
            body.top + (idx / cols) as f32 * 206. - s.scroll,
            width,
            190.,
        );
        let clipped = r.intersect(&body);
        if clipped.is_empty() {
            continue;
        }
        let folder = folders.get(idx);
        let flow = if folder.is_none() {
            flows.get(idx - folders.len())
        } else {
            None
        };
        let hit = folder
            .map(|f| Hit::Folder(Some(f.id.clone())))
            .unwrap_or_else(|| Hit::Open(flow.unwrap().0));
        let hot = s.hover.as_ref() == Some(&hit);
        list.rounded_rect(r, 12., p.surface_elevated);
        list.rounded_border(r, 12., if hot { p.accent } else { p.border });
        let badge = Rect::from_size(r.left + 18., r.top + 18., 36., 36.);
        list.rounded_rect(badge, 9., p.surface_muted);
        list.icon_centered(
            badge,
            if folder.is_some() {
                Icon::FOLDER
            } else {
                Icon::LINK2
            },
            18.,
            p.foreground,
        );
        let (name, description, footer, menu) = if let Some(f) = folder {
            let count = s
                .workflows
                .iter()
                .filter(|w| w.folder_id.as_deref() == Some(&f.id))
                .count();
            (
                f.name.clone(),
                "集中管理相关工作流".to_string(),
                format!("{count} 个工作流    ·    打开文件夹 ›"),
                Hit::FolderMenu(f.id.clone()),
            )
        } else {
            let (i, w) = flow.unwrap();
            list.text(
                Rect::new(r.left + 66., r.top + 24., r.right - 50., r.top + 46.),
                if w.enabled {
                    "定时运行"
                } else {
                    "手动运行"
                },
                TextStyle::Tiny,
                p.muted,
            );
            (
                w.name.clone(),
                if w.description.is_empty() {
                    "添加节点与变量，构建你的自动化流程。".into()
                } else {
                    w.description.clone()
                },
                format!("{} 个节点    ·    打开工作流 ›", w.node_count),
                Hit::WorkflowMenu(*i),
            )
        };
        list.text(
            Rect::new(r.left + 18., r.top + 68., r.right - 18., r.top + 96.),
            text::ellipsize(&name, TextStyle::Label, width - 36.),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(r.left + 18., r.top + 102., r.right - 18., r.top + 126.),
            text::ellipsize(
                &description.replace('\n', " "),
                TextStyle::Caption,
                width - 36.,
            ),
            TextStyle::Caption,
            p.muted,
        );
        list.hline(r.left + 18., r.right - 18., r.bottom - 46., p.border);
        list.text(
            Rect::new(r.left + 18., r.bottom - 38., r.right - 18., r.bottom - 12.),
            footer,
            TextStyle::Tiny,
            p.muted,
        );
        s.hits.push((clipped, hit));
        let mr = Rect::from_size(r.right - 46., r.top + 18., 28., 28.);
        if mr.top >= body.top && mr.bottom <= body.bottom {
            button(list, s, mr, "⋯", menu, p, false);
        }
    }
    if total == 0 {
        list.icon_centered(
            Rect::from_size(
                (body.left + body.right) / 2. - 24.,
                body.top + 64.,
                48.,
                48.,
            ),
            Icon::LINK2,
            28.,
            p.muted,
        );
        list.text_aligned(
            Rect::new(body.left, body.top + 130., body.right, body.top + 158.),
            "让重复的工作自动完成",
            TextStyle::Title,
            p.foreground,
            crate::ui::draw::Align::Center,
        );
        list.text_aligned(
            Rect::new(body.left, body.top + 164., body.right, body.top + 194.),
            "新建工作流，或导入已有流程到这里",
            TextStyle::Caption,
            p.muted,
            crate::ui::draw::Align::Center,
        );
        button(
            list,
            s,
            Rect::from_size(
                (body.left + body.right) / 2. - 70.,
                body.top + 220.,
                140.,
                38.,
            ),
            "＋ 新建工作流",
            Hit::New,
            p,
            true,
        );
    }
    list.pop_clip();
    if s.library_height > body.height() && body.height() > 0. {
        let height = (body.height() * body.height() / s.library_height).max(28.);
        let y = body.top + (body.height() - height) * s.scroll / (s.library_height - body.height());
        list.rounded_rect(
            Rect::from_size(area.right - 8., y, 3., height),
            1.5,
            p.border,
        );
    }
    if s.editor == Some(Editor::Folder) {
        let r = Rect::from_size(
            area.left + 24.,
            nav_y + 44.,
            (area.width() - 48.).min(500.),
            156.,
        );
        list.rounded_rect(r, 12., p.surface_elevated);
        list.rounded_border(r, 12., p.border);
        s.hits.push((r, Hit::Parameters));
        list.text(
            Rect::new(r.left + 16., r.top + 12., r.right - 16., r.top + 38.),
            if s.editing_folder.is_some() {
                "重命名文件夹"
            } else {
                "新建文件夹"
            },
            TextStyle::Label,
            p.foreground,
        );
        s.field_rect = Rect::new(r.left + 16., r.top + 48., r.right - 16., r.top + 88.);
        s.field.paint(
            list,
            s.field_rect,
            true,
            p,
            crate::ui::widgets::FieldLook::dialog(p),
        );
        button(
            list,
            s,
            Rect::from_size(r.right - 100., r.bottom - 50., 84., 34.),
            "保存",
            Hit::Apply,
            p,
            true,
        );
        button(
            list,
            s,
            Rect::from_size(r.right - 192., r.bottom - 50., 84., 34.),
            "取消",
            Hit::CloseEditor,
            p,
            false,
        );
    }
}
