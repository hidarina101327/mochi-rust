//! 桌面会话卡片共用 Mochi Markdown 渲染器。
use super::*;
use crate::ui::{
    ai_markdown,
    draw::{Align, TextStyle},
    icons::Icon,
    layout::Rect,
    text, theme,
};
use std::rc::Rc;
struct Message {
    rect: Rect,
    user: bool,
    layout: Rc<ai_markdown::Layout>,
}
fn messages(v: &View, body: Rect) -> (Vec<Message>, f32) {
    let mut y = body.top;
    let mut rows = Vec::new();
    for (role, source) in &v.live.messages {
        let user = role == "user";
        let x = body.left + if user { 22.0 } else { 0.0 };
        let width = (body.right - x - 24.0).max(32.0);
        let layout = ai_markdown::layout(source, width, true);
        let height = layout.height + 44.0;
        rows.push(Message {
            rect: Rect::from_size(x, y, width + 24.0, height),
            user,
            layout,
        });
        y += height + 16.0;
    }
    if v.live.streaming {
        y += 38.0;
    }
    (rows, y - body.top)
}
pub fn viewport(b: Rect) -> Rect {
    Rect::new(b.left, b.top + 32.0, b.right, b.bottom - 92.0)
}
pub fn max(v: &View, b: Rect) -> usize {
    let viewport = viewport(b);
    (messages(v, viewport).1 - viewport.height())
        .max(0.0)
        .ceil() as usize
}
pub fn paint(list: &mut DrawList, spec: &Spec, v: &View, a: Rect, offset: usize) {
    let (p, _item_bg) = painting::item_palette(&painting::palette(spec), v, "page");
    let b = widgets::chat_body(spec, v, a);
    let viewport = viewport(b);
    list.text(
        Rect::from_size(b.left, b.top, (b.width() - 65.0).max(1.0), 22.0),
        text::ellipsize(
            &v.live.title,
            TextStyle::Caption,
            (b.width() - 65.0).max(1.0),
        ),
        TextStyle::Caption,
        p.muted,
    );
    let history = Rect::from_size(b.right - 62.0, b.top, 62.0, 24.0);
    list.rounded_rect(history, 7.0, p.surface_muted);
    list.icon_centered(
        Rect::from_size(history.left + 6.0, history.top, 18.0, 24.0),
        Icon::CLOCK,
        13.0,
        p.muted,
    );
    list.text_aligned(
        Rect::new(
            history.left + 24.0,
            history.top,
            history.right - 4.0,
            history.bottom,
        ),
        "历史",
        TextStyle::Caption,
        p.foreground,
        Align::Center,
    );
    list.push_clip(viewport);
    let (rows, height) = messages(v, viewport);
    if rows.is_empty() && !v.live.streaming {
        let cy = viewport.top + viewport.height() * 0.38;
        list.icon_centered(
            Rect::from_size(viewport.left, cy - 32.0, viewport.width(), 32.0),
            Icon::BOT,
            25.0,
            p.muted,
        );
        list.text_aligned(
            Rect::from_size(viewport.left, cy + 10.0, viewport.width(), 28.0),
            "有什么想聊的？",
            TextStyle::Large,
            p.foreground,
            Align::Center,
        );
    }
    for (index, m) in rows.into_iter().enumerate() {
        let r = Rect::new(
            m.rect.left,
            m.rect.top - offset as f32,
            m.rect.right,
            m.rect.bottom - offset as f32,
        );
        if r.intersect(&viewport).is_empty() {
            continue;
        }
        if m.user {
            list.rounded_rect(r, 12.0, p.surface_muted);
        }
        list.text(
            Rect::from_size(r.left + 12.0, r.top + 8.0, r.width() - 24.0, 20.0),
            if m.user { "你" } else { "墨池 AI" },
            TextStyle::Caption,
            p.muted,
        );
        m.layout.paint_scrolled(
            list,
            (r.left + 12.0, r.top + 34.0),
            viewport.intersect(&r),
            &p,
            None,
            false,
            v.chat_offsets.get(&index),
            None,
        );
    }
    if v.live.streaming {
        let y = viewport.top + height - 34.0 - offset as f32;
        list.spinner(
            Rect::from_size(viewport.left + 12.0, y, 20.0, 20.0),
            (chrono::Local::now().timestamp_millis() / 120) as u8,
            p.muted,
        );
        list.text(
            Rect::from_size(viewport.left + 40.0, y, viewport.width() - 40.0, 22.0),
            "正在思考…",
            TextStyle::Caption,
            p.muted,
        );
    }
    list.pop_clip();
    let input = composer::rect(b);
    list.rounded_rect(input, 12.0, theme::mix(p.surface_muted, p.surface, 0.5));
    list.rounded_border(input, 12.0, p.border);
    let send = composer::send_rect(b);
    list.rounded_rect(send, 8.0, p.foreground);
    list.icon_centered(
        send,
        if v.live.streaming {
            Icon::SQUARE
        } else {
            Icon::ARROW_UP
        },
        17.0,
        p.surface,
    );
    list.text(
        Rect::new(
            input.left + 12.0,
            input.bottom - 26.0,
            send.left - 6.0,
            input.bottom - 4.0,
        ),
        "Enter 发送 · Shift+Enter 换行",
        TextStyle::Tiny,
        p.muted,
    );
    if !v.live.error.is_empty() {
        list.text(
            Rect::from_size(b.left, input.top - 26.0, b.width(), 22.0),
            text::ellipsize(&v.live.error, TextStyle::Caption, b.width()),
            TextStyle::Caption,
            p.danger,
        );
    }
}
pub fn hit(spec: &Spec, v: &View, a: Rect, offset: usize, x: f32, y: f32) -> Option<Hit> {
    let b = widgets::chat_body(spec, v, a);
    let viewport = viewport(b);
    if !viewport.contains(x, y) {
        return None;
    }
    for (index, m) in messages(v, viewport).0.into_iter().enumerate() {
        let lx = x - m.rect.left - 12.0;
        let ly = y - m.rect.top + offset as f32 - 34.0;
        for region in &m.layout.scroll_regions {
            if region.max_x() > 0.0 && region.track.contains(lx, ly) {
                let fraction = ((lx - region.track.left) / region.track.width()).clamp(0.0, 1.0);
                return Some(Hit::AiScroll(
                    index,
                    region.id,
                    (fraction * region.max_x()) as u32,
                ));
            }
        }
        if let Some(link) = m.layout.link_at(lx, ly, v.chat_offsets.get(&index)) {
            return Some(Hit::AiLink(link));
        }
        if let Some(i) = m
            .layout
            .copy_targets()
            .find(|(_, r, _)| r.contains(lx, ly))
            .map(|(i, _, _)| i)
        {
            if let Some(value) = m.layout.copy_payload(i) {
                return Some(Hit::AiCopy(value.text.clone()));
            }
        }
    }
    None
}

/// Shift+滚轮和水平滚轮会操作指针所在的代码块或表格。
pub fn scroll(
    spec: &Spec,
    v: &mut View,
    a: Rect,
    offset: usize,
    x: f32,
    y: f32,
    delta: f32,
) -> bool {
    if !widgets::has_chat(v) {
        return false;
    }
    let viewport = viewport(widgets::chat_body(spec, v, a));
    if !viewport.contains(x, y) {
        return false;
    }
    for (index, m) in messages(v, viewport).0.into_iter().enumerate() {
        let lx = x - m.rect.left - 12.0;
        let ly = y - m.rect.top + offset as f32 - 34.0;
        for region in &m.layout.scroll_regions {
            if region.max_x() > 0.0
                && (region.viewport.contains(lx, ly) || region.track.contains(lx, ly))
            {
                let offsets = v.chat_offsets.entry(index).or_default();
                let next = (region.offset(Some(offsets)) + delta).clamp(0.0, region.max_x());
                offsets.insert(region.id, next);
                return true;
            }
        }
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_ai_wide_markdown_has_operable_scrollbars() {
        let spec = Spec {
            appearance: Default::default(),
            id: "ai".into(),
            title: "AI".into(),
            x: 0,
            y: 0,
            width: 400,
            height: 480,
            locked: false,
            dark: false,
        };
        let mut view = View {
            module: Some(mochi_core::desktop_cards::Module::Ai),
            ..Default::default()
        };
        view.live.messages.push((
            "assistant".into(),
            format!("```rust\nlet message = \"{}\";\n```", "content ".repeat(60)),
        ));
        let a = Rect::from_size(0.0, 0.0, 400.0, 480.0);
        let vp = viewport(widgets::chat_body(&spec, &view, a));
        let rows = messages(&view, vp).0;
        let m = &rows[0];
        let region = &m.layout.scroll_regions[0];
        assert!(region.max_x() > 0.0);
        let x = m.rect.left + 12.0 + region.viewport.left + 10.0;
        let y = m.rect.top + 34.0 + region.viewport.top + 10.0;
        assert!(scroll(&spec, &mut view, a, 0, x, y, 48.0));
        assert_eq!(view.chat_offsets[&0][&region.id], 48.0);
        assert!(scroll(&spec, &mut view, a, 0, x, y, -100.0));
        assert_eq!(view.chat_offsets[&0][&region.id], 0.0);
        let x = m.rect.left + 12.0 + region.track.right - 2.0;
        let y = m.rect.top + 34.0 + region.track.top + 2.0;
        assert!(matches!(hit(&spec,&view,a,0,x,y),Some(Hit::AiScroll(0,_,amount)) if amount>0));
    }
}
