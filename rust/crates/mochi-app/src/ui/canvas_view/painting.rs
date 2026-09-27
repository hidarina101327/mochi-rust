//! 计算并绘制画布网格、笔迹和文字元素。
use super::super::{draw::DrawCmd, widgets::FieldLook};
use super::*;
use mochi_core::canvas::Target;

pub fn layout(state: &State, area: Rect) -> Layout {
    if area.is_empty() {
        return Layout::default();
    }
    let mut entries = Vec::new();
    let (mut x, mut y) = (area.left + 10.0, area.top + 8.0);
    let mut add = |hit: Hit, width: f32| {
        if x + width > area.right - 8.0 && x > area.left + 10.0 {
            x = area.left + 10.0;
            y += 36.0;
        }
        entries.push((
            Rect::from_size(x, y, width.min((area.width() - 20.0).max(0.0)), 30.0),
            hit,
        ));
        x += width + 4.0;
    };
    for tool in [
        Tool::Select,
        Tool::Pen,
        Tool::Text,
        Tool::Eraser,
        Tool::Hand,
    ] {
        add(Hit::Tool(tool), 48.0);
    }
    add(Hit::AddReference, 62.0);
    for hit in [Hit::Undo, Hit::Redo, Hit::Delete, Hit::Grid] {
        add(hit, 30.0);
    }
    let mut y = y + 38.0;
    let mut x = area.left + 10.0;
    for i in 0..COLORS.len() {
        if x + 26.0 > area.right - 8.0 {
            x = area.left + 10.0;
            y += 32.0;
        }
        entries.push((Rect::from_size(x, y, 26.0, 26.0), Hit::Color(i)));
        x += 30.0;
    }
    x += 8.0;
    let text_mode = state.tool == Tool::Text || matches!(state.selected, Some(Element::Text(_)));
    for i in 0..3 {
        if x + 36.0 > area.right - 8.0 {
            x = area.left + 10.0;
            y += 32.0;
        }
        entries.push((
            Rect::from_size(x, y, 36.0, 26.0),
            if text_mode {
                Hit::FontSize(i)
            } else {
                Hit::Width(i)
            },
        ));
        x += 40.0;
    }
    if matches!(state.selected, Some(Element::Text(_)))
        && state.editor.is_none()
        && x + 60.0 < area.right
    {
        entries.push((Rect::from_size(x + 4.0, y, 56.0, 26.0), Hit::EditText));
    }
    let body = Rect::new(
        area.left,
        (y + 34.0).min(area.bottom),
        area.right,
        (area.bottom - 38.0).max(y + 34.0).min(area.bottom),
    );
    entries.insert(0, (body, Hit::Surface));
    if state.editor.is_none() && state.tool == Tool::Select {
        if let Some(Element::Text(i)) = state.selected {
            if let Some(b) = state.bounds(Element::Text(i)) {
                let rect = state.screen_rect(body, b);
                let handle = Rect::from_size(rect.right - 6.0, rect.bottom - 6.0, 12.0, 12.0)
                    .intersect(&body);
                if !handle.is_empty() {
                    entries.push((handle, Hit::Resize));
                }
            }
        }
    }
    let mut x = area.right - 10.0;
    for (hit, width) in [
        (Hit::FitContent, 54.0),
        (Hit::ZoomIn, 28.0),
        (Hit::ResetZoom, 56.0),
        (Hit::ZoomOut, 28.0),
    ] {
        x -= width;
        if x >= area.left {
            entries.push((Rect::from_size(x, area.bottom - 32.0, width, 26.0), hit));
        }
        x -= 4.0;
    }
    Layout { body, entries }
}

pub fn paint(
    list: &mut DrawList,
    area: Rect,
    state: &State,
    layout: &Layout,
    hover: Option<Hit>,
    p: &Palette,
) {
    list.rect(area, p.background);
    list.push_clip(layout.body);
    if state.grid {
        paint_grid(list, state, layout.body, p);
    }
    for (i, card) in state.document.cards.iter().enumerate() {
        let r = state.screen_rect(layout.body, state.bounds(Element::Card(i)).unwrap());
        if !list.rect_visible(r) {
            continue;
        }
        list.rounded_rect(
            r,
            9.0,
            card.color
                .map(|c| theme::mix(c, p.surface, 0.16))
                .unwrap_or(p.surface),
        );
        list.rounded_border(r, 9.0, card.color.unwrap_or(p.border));
        let zoom = state.document.viewport.zoom as f32;
        let (kind, label) = match &card.target {
            Target::Mochi { url } => mochi_core::object_reference::ObjectReference::parse(url)
                .map(|r| (r.kind.label(), r.display_label()))
                .unwrap_or(("失效引用", "无法读取的链接".into())),
            Target::PdfAnnotation { path, .. } => {
                ("PDF 批注", path.rsplit('/').next().unwrap_or(path).into())
            }
        };
        list.push_clip(r);
        for (offset, label, style, color) in [
            (12.0, kind.to_owned(), TextStyle::Small, p.accent),
            (42.0, label, TextStyle::Body, p.foreground),
            (94.0, "双击打开原始对象".into(), TextStyle::Small, p.muted),
        ] {
            let start = list.cmds().len();
            list.text(
                Rect::from_size(
                    r.left + 14.0 * zoom,
                    r.top + offset * zoom,
                    r.width() - 28.0 * zoom,
                    28.0 * zoom,
                ),
                label,
                style,
                color,
            );
            list.scale_text_since(start, zoom);
        }
        list.pop_clip();
    }
    for stroke in state.document.strokes.iter().chain(state.draft.iter()) {
        paint_stroke(list, state, layout.body, stroke, p);
    }
    for (index, note) in state.document.texts.iter().enumerate() {
        let rect = state.screen_rect(layout.body, note.bounds());
        if !list.rect_visible(rect) {
            continue;
        }
        let focused = state.editor.as_ref().is_some_and(|e| e.index == index);
        let mut field = state
            .editor
            .as_ref()
            .filter(|e| e.index == index)
            .map(|e| e.field.clone())
            .unwrap_or_else(|| TextField::new("").with_text(&note.text));
        let mut palette = p.clone();
        palette.foreground = ink_color(note.color, p);
        let mut local = DrawList::new();
        field.paint_multiline_with_look(
            &mut local,
            super::interaction::logical_text_rect(note),
            focused,
            &palette,
            FieldLook {
                radius: 0.0,
                background: None,
                border: None,
                focus_ring: None,
                padding_left: 12.0,
                padding_right: 12.0,
                leading_icon: None,
            },
        );
        let scale = note.font_size as f32 / TextStyle::Body.font_size()
            * state.document.viewport.zoom as f32;
        paint_text_commands(list, local, rect.left, rect.top, scale);
    }
    if let Some(bounds) = state.selected.and_then(|e| state.bounds(e)) {
        let rect = state.screen_rect(layout.body, bounds);
        list.rounded_border(
            Rect::new(
                rect.left - 2.0,
                rect.top - 2.0,
                rect.right + 2.0,
                rect.bottom + 2.0,
            ),
            4.0,
            p.accent,
        );
    }
    if state.document.bounds().is_none() && state.draft.is_none() {
        let cy = layout.body.top + layout.body.height() * 0.36;
        let title_height = TextStyle::Display.line_height().max(40.0);
        let body_height = TextStyle::Body.line_height().max(30.0);
        let hint_height = TextStyle::Small.line_height().max(26.0);
        let body_top = cy + title_height + 10.0;
        let hint_top = body_top + body_height + 2.0;
        list.text_aligned(
            Rect::new(
                layout.body.left + 20.0,
                cy,
                layout.body.right - 20.0,
                cy + title_height,
            ),
            "想法，随处落笔",
            TextStyle::Display,
            p.foreground,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(
                layout.body.left + 20.0,
                body_top,
                layout.body.right - 20.0,
                body_top + body_height,
            ),
            "选画笔自由涂鸦，选文字点击书写",
            TextStyle::Body,
            p.muted,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(
                layout.body.left + 20.0,
                hint_top,
                layout.body.right - 20.0,
                hint_top + hint_height,
            ),
            "也可以双击空白处，开始一条笔记",
            TextStyle::Small,
            p.muted,
            Align::Center,
        );
    }
    list.pop_clip();
    list.rect(
        Rect::new(area.left, area.top, area.right, layout.body.top),
        p.surface,
    );
    list.hline(area.left, area.right, layout.body.top - 1.0, p.border);
    list.rect(
        Rect::new(area.left, layout.body.bottom, area.right, area.bottom),
        p.surface,
    );
    list.hline(area.left, area.right, layout.body.bottom, p.border);
    for (rect, hit) in &layout.entries {
        if *hit != Hit::Surface {
            paint_button(list, *rect, *hit, state, hover == Some(*hit), p);
        }
    }
    if area.width() > 600.0 {
        let hint = hover.map(hint).unwrap_or(if state.editor.is_some() {
            "Enter 换行 · Ctrl+Enter 完成"
        } else {
            "空格拖动平移 · Ctrl+滚轮缩放"
        });
        list.text(
            Rect::new(
                area.left + 12.0,
                area.bottom - 34.0,
                area.right - 208.0,
                area.bottom - 4.0,
            ),
            hint,
            TextStyle::Small,
            p.muted,
        );
    }
}

fn paint_grid(list: &mut DrawList, state: &State, body: Rect, p: &Palette) {
    let v = &state.document.viewport;
    let grid_spacing = crate::ui::settings_values::number("canvas.gridSpacing", 28.0) as f64;
    let mut step = grid_spacing * v.zoom;
    while step < grid_spacing * (18.0 / 28.0) {
        step *= 2.0;
    }
    while step > grid_spacing * 2.0 {
        step /= 2.0;
    }
    let mut x = body.left - (v.x * v.zoom).rem_euclid(step) as f32;
    let color = crate::ui::settings_values::color(
        "canvas.gridColor",
        theme::mix(p.border, p.background, 0.65),
    );
    let dot_size = crate::ui::settings_values::number("canvas.gridDotSize", 1.5);
    while x < body.right {
        let mut y = body.top - (v.y * v.zoom).rem_euclid(step) as f32;
        while y < body.bottom {
            list.rect(Rect::from_size(x, y, dot_size, dot_size), color);
            y += step as f32;
        }
        x += step as f32;
    }
}

fn ink_color(color: u32, p: &Palette) -> u32 {
    if color == COLORS[0] {
        p.foreground
    } else {
        color
    }
}

fn paint_stroke(list: &mut DrawList, state: &State, body: Rect, stroke: &Stroke, p: &Palette) {
    let Some(bounds) = stroke.bounds() else {
        return;
    };
    if !list.rect_visible(state.screen_rect(body, bounds)) {
        return;
    }
    let v = &state.document.viewport;
    let width = (stroke.width * v.zoom) as f32;
    let color = ink_color(stroke.color, p);
    let points: Vec<_> = stroke
        .points
        .iter()
        .map(|point| {
            (
                body.left + ((point.x - v.x) * v.zoom) as f32,
                body.top + ((point.y - v.y) * v.zoom) as f32,
            )
        })
        .collect();
    list.polyline(points.clone(), color, width);
    // 用现有的原生图元实现圆角转折，并让单点点击也留下可见笔迹。
    for (x, y) in points {
        list.rounded_rect(
            Rect::from_size(x - width / 2.0, y - width / 2.0, width, width),
            width / 2.0,
            color,
        );
    }
}

fn paint_text_commands(list: &mut DrawList, local: DrawList, x: f32, y: f32, scale: f32) {
    let transform = |r: Rect| {
        Rect::from_size(
            x + r.left * scale,
            y + r.top * scale,
            r.width() * scale,
            r.height() * scale,
        )
    };
    for command in local.cmds() {
        match command {
            DrawCmd::Text {
                rect,
                text,
                style,
                color,
                ..
            } => {
                let start = list.cmds().len();
                list.text(transform(*rect), text, *style, *color);
                list.scale_text_since(start, scale);
            }
            DrawCmd::Rect { rect, color } => list.rect(transform(*rect), *color),
            DrawCmd::Caret { rect, color, .. } => list.caret(transform(*rect), *color),
            DrawCmd::PushClip { rect } => list.push_clip(transform(*rect)),
            DrawCmd::PopClip => list.pop_clip(),
            _ => {}
        }
    }
}

fn paint_button(list: &mut DrawList, rect: Rect, hit: Hit, state: &State, hot: bool, p: &Palette) {
    let selected = match hit {
        Hit::Tool(t) => state.tool == t,
        Hit::Color(i) => state.color == i,
        Hit::Width(i) => state.width == i,
        Hit::FontSize(i) => state.font_size == i,
        Hit::Grid => state.grid,
        _ => false,
    };
    let disabled = match hit {
        Hit::Undo => state.undo.is_empty() && state.editor.is_none(),
        Hit::Redo => state.redo.is_empty(),
        Hit::Delete => state.selected.is_none(),
        _ => false,
    };
    if selected || hot {
        list.rounded_rect(
            rect,
            6.0,
            if selected {
                theme::mix(p.accent, p.surface, 0.12)
            } else {
                p.surface_muted
            },
        );
    }
    let color = if disabled {
        theme::mix(p.muted, p.surface, 0.4)
    } else if selected {
        p.accent
    } else {
        p.foreground
    };
    let label = match hit {
        Hit::Tool(t) => match t {
            Tool::Select => "选择",
            Tool::Pen => "画笔",
            Tool::Text => "文字",
            Tool::Eraser => "橡皮",
            Tool::Hand => "平移",
        }
        .to_owned(),
        Hit::AddReference => "+ 关联".into(),
        Hit::ResetZoom => format!("{}%", (state.document.viewport.zoom * 100.0).round() as i32),
        Hit::FitContent => "适配".into(),
        Hit::EditText => "编辑".into(),
        Hit::FontSize(i) => format!("{}", FONT_SIZES[i]),
        Hit::Width(i) => ["细", "中", "粗"][i].into(),
        Hit::Undo => "↶".into(),
        Hit::Redo => "↷".into(),
        Hit::Grid => "▦".into(),
        Hit::Color(i) => {
            let dot = Rect::from_size(rect.left + 5.0, rect.top + 5.0, 16.0, 16.0);
            list.rounded_rect(dot, 8.0, ink_color(COLORS[i], p));
            if selected {
                list.rounded_border(rect, 13.0, p.accent);
            }
            return;
        }
        Hit::Resize => {
            list.rounded_rect(rect, 2.0, p.accent);
            return;
        }
        Hit::ZoomIn | Hit::ZoomOut | Hit::Delete => {
            list.icon_centered(
                rect,
                match hit {
                    Hit::ZoomIn => Icon::PLUS,
                    Hit::ZoomOut => Icon::MINUS,
                    _ => Icon::TRASH2,
                },
                15.0,
                color,
            );
            return;
        }
        _ => return,
    };
    list.text_aligned(rect, label, TextStyle::Small, color, Align::Center);
}

fn hint(hit: Hit) -> &'static str {
    match hit {
        Hit::Tool(Tool::Select) => "V 选择 · 拖动移动 · 双击编辑文字或打开引用",
        Hit::Tool(Tool::Pen) => "P 画笔 · 按下并拖动自由涂鸦",
        Hit::Tool(Tool::Text) => "T 文字 · 点击任意位置书写",
        Hit::Tool(Tool::Eraser) => "E 橡皮 · 擦除整条笔迹 · 可撤销",
        Hit::Tool(Tool::Hand) => "H 平移 · 拖动探索无限画布",
        Hit::Undo => "撤销 Ctrl+Z",
        Hit::Redo => "重做 Ctrl+Shift+Z / Ctrl+Y",
        Hit::Delete => "删除选中内容 Delete",
        Hit::ResetZoom => "回到原点与 100% 缩放",
        Hit::FitContent => "查看所有文字、笔迹与关联对象",
        Hit::Grid => "显示或隐藏点阵网格",
        _ => "空格拖动平移 · Ctrl+滚轮缩放",
    }
}
