//! 每个会话独立的横向视口状态；绝不持久化进 AI 消息。
use super::{
    ai_markdown::{Offsets, ScrollId},
    assistant::Layout,
};
use std::collections::HashMap;
#[derive(Default)]
pub struct State {
    session: Option<String>,
    values: HashMap<String, Offsets>,
    pub hover: Option<(String, ScrollId)>,
    drag: Option<Drag>,
}

#[cfg(test)]
mod tests {
    use crate::ui::{assistant, layout::Rect};
    use mochi_core::ai::session::{AiConversation, AiStoredMessage};
    fn state() -> assistant::State {
        let mut s = assistant::State::default();
        s.active = Some(AiConversation {
            id: "session".into(),
            messages: vec![AiStoredMessage::new(
                "assistant",
                include_str!("../../tests/fixtures/ai-scroll-verification.md"),
            )],
            ..Default::default()
        });
        s
    }
    fn area() -> Rect {
        Rect::from_size(0.0, 0.0, 380.0, 1200.0)
    }
    #[test]
    fn wheel_is_per_block_clamped_and_resets_on_session_change() {
        let mut s = state();
        let lay = assistant::layout(&s, area());
        s.horizontal.sync(&lay);
        assert_eq!(lay.messages[0].body.scroll_regions.len(), 3);
        let m = &lay.messages[0];
        let r = &m.body.scroll_regions[0];
        let o = lay.body_origin(0, 0.0).unwrap();
        let x = o.0 + r.viewport.left + 2.0;
        let y = o.1 + r.viewport.top + 2.0;
        assert!(s.horizontal.wheel(&lay, 0.0, x, y, 1_000_000.0));
        let next = assistant::layout(&s, area());
        assert_eq!(r.offset(Some(&next.messages[0].horizontal)), r.max_x());
        assert_eq!(
            next.messages[0].body.scroll_regions[1].offset(Some(&next.messages[0].horizontal)),
            0.0
        );
        assert!(s.horizontal.wheel(&next, 0.0, x, y, -1_000_000.0));
        assert_eq!(
            s.horizontal.offsets(&lay.session_id, &m.scroll_key)[&r.id],
            0.0
        );
        s.active.as_mut().unwrap().id = "other".into();
        let next = assistant::layout(&s, area());
        s.horizontal.sync(&next);
        assert!(s.horizontal.values.is_empty());
    }
    #[test]
    fn dragging_thumb_reaches_end_and_release_stops_updates() {
        let mut s = state();
        let lay = assistant::layout(&s, area());
        let m = &lay.messages[0];
        let r = &m.body.scroll_regions[0];
        let o = lay.body_origin(0, 0.0).unwrap();
        let thumb = r.thumb(0.0);
        let y = o.1 + (thumb.top + thumb.bottom) / 2.0;
        assert!(s.horizontal.click(&lay, 0.0, o.0 + thumb.left + 3.0, y));
        assert!(s.horizontal.dragging());
        assert!(s.horizontal.drag_to(&lay, 0.0, 5000.0));
        assert_eq!(
            s.horizontal.offsets(&lay.session_id, &m.scroll_key)[&r.id],
            r.max_x()
        );
        s.horizontal.end_drag();
        assert!(!s.horizontal.drag_to(&lay, 0.0, 0.0));
        assert_eq!(
            s.horizontal.offsets(&lay.session_id, &m.scroll_key)[&r.id],
            r.max_x()
        );
        let narrow = assistant::layout(&s, Rect::from_size(0.0, 0.0, 10000.0, 1200.0));
        s.horizontal.sync(&narrow);
        assert!(s.horizontal.values.is_empty());
    }
    #[test]
    fn blocked_and_outside_hits_do_not_change_scroll() {
        let mut s = state();
        let mut lay = assistant::layout(&s, area());
        let r = &lay.messages[0].body.scroll_regions[0];
        let o = lay.body_origin(0, 0.0).unwrap();
        let x = o.0 + r.viewport.left + 2.0;
        let y = o.1 + r.viewport.top + 2.0;
        lay.popover = Some(Rect::from_size(x - 1.0, y - 1.0, 5.0, 5.0));
        assert!(!s.horizontal.wheel(&lay, 0.0, x, y, 100.0));
        assert!(!s.horizontal.wheel(&lay, 0.0, 9999.0, y, 100.0));
        assert!(s.horizontal.values.is_empty());
    }
    #[test]
    fn streaming_append_retains_the_same_block_anchor() {
        let mut s = assistant::State::default();
        s.streaming = Some(assistant::Streaming {
            content: format!("```text\n{}", "abcdef".repeat(80)),
            ..Default::default()
        });
        let lay = assistant::layout(&s, area());
        let r = &lay.messages[0].body.scroll_regions[0];
        let o = lay.body_origin(0, 0.0).unwrap();
        assert!(s.horizontal.wheel(
            &lay,
            0.0,
            o.0 + r.viewport.left + 2.0,
            o.1 + r.viewport.top + 2.0,
            80.0
        ));
        s.streaming.as_mut().unwrap().content.push_str(" append");
        let next = assistant::layout(&s, area());
        assert_eq!(next.messages[0].body.scroll_regions[0].id, r.id);
        assert_eq!(
            next.messages[0].body.scroll_regions[0].offset(Some(&next.messages[0].horizontal)),
            80.0
        );
    }
}
struct Drag {
    session: Option<String>,
    message: String,
    id: ScrollId,
    grab: f32,
}
impl State {
    pub fn offsets(&self, session: &Option<String>, message: &str) -> Offsets {
        if self.session == *session {
            self.values.get(message).cloned().unwrap_or_default()
        } else {
            Offsets::new()
        }
    }
    pub fn sync(&mut self, layout: &Layout) {
        if self.session != layout.session_id {
            self.values.clear();
            self.hover = None;
            self.session = layout.session_id.clone();
        }
        self.values.retain(|key, offsets| {
            let Some(message) = layout.messages.iter().find(|m| m.scroll_key == *key) else {
                return false;
            };
            offsets.retain(|id, value| {
                let Some(region) = message.body.scroll_regions.iter().find(|r| r.id == *id) else {
                    return false;
                };
                *value = value.clamp(0.0, region.max_x());
                true
            });
            !offsets.is_empty()
        });
    }
    fn region_at(
        layout: &Layout,
        scroll: f32,
        x: f32,
        y: f32,
        track: bool,
    ) -> Option<(usize, usize, f32)> {
        let m = layout.message_at(scroll, x, y)?;
        let origin = layout.body_origin(m, scroll)?;
        let local = (x - origin.0, y - origin.1);
        layout.messages[m]
            .body
            .scroll_regions
            .iter()
            .enumerate()
            .rev()
            .find(|(_, r)| {
                r.track.contains(local.0, local.1)
                    || (!track && r.viewport.contains(local.0, local.1))
            })
            .map(|(i, _)| (m, i, local.0))
    }
    pub fn pointer(&mut self, layout: &Layout, scroll: f32, x: f32, y: f32) -> bool {
        let next = Self::region_at(layout, scroll, x, y, false).map(|(m, r, _)| {
            (
                layout.messages[m].scroll_key.clone(),
                layout.messages[m].body.scroll_regions[r].id,
            )
        });
        let changed = self.hover != next;
        self.hover = next;
        changed
    }
    pub fn wheel(&mut self, layout: &Layout, scroll: f32, x: f32, y: f32, pixels: f32) -> bool {
        if !pixels.is_finite() {
            return false;
        }
        self.sync(layout);
        let Some((m, r, _)) = Self::region_at(layout, scroll, x, y, false) else {
            return false;
        };
        let message = &layout.messages[m];
        let region = &message.body.scroll_regions[r];
        let offsets = self.values.entry(message.scroll_key.clone()).or_default();
        let value = region.offset(Some(offsets));
        offsets.insert(region.id, (value + pixels).clamp(0.0, region.max_x()));
        self.hover = Some((message.scroll_key.clone(), region.id));
        true
    }
    pub fn click(&mut self, layout: &Layout, scroll: f32, x: f32, y: f32) -> bool {
        self.sync(layout);
        let Some((m, r, local_x)) = Self::region_at(layout, scroll, x, y, true) else {
            return false;
        };
        let message = &layout.messages[m];
        let region = &message.body.scroll_regions[r];
        let offsets = self.values.entry(message.scroll_key.clone()).or_default();
        let value = region.offset(Some(offsets));
        let thumb = region.thumb(value);
        if local_x >= thumb.left && local_x < thumb.right {
            self.drag = Some(Drag {
                session: layout.session_id.clone(),
                message: message.scroll_key.clone(),
                id: region.id,
                grab: local_x - thumb.left,
            });
        } else {
            let direction = if local_x < thumb.left { -1.0 } else { 1.0 };
            offsets.insert(
                region.id,
                (value + direction * region.viewport.width()).clamp(0.0, region.max_x()),
            );
        }
        self.hover = Some((message.scroll_key.clone(), region.id));
        true
    }
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn end_drag(&mut self) {
        self.drag = None;
    }
    pub fn drag_to(&mut self, layout: &Layout, scroll: f32, x: f32) -> bool {
        if !x.is_finite() {
            return false;
        }
        let Some(drag) = &self.drag else { return false };
        if drag.session != layout.session_id {
            return false;
        }
        let Some((m, message)) = layout
            .messages
            .iter()
            .enumerate()
            .find(|(_, m)| m.scroll_key == drag.message)
        else {
            return false;
        };
        let Some(region) = message.body.scroll_regions.iter().find(|r| r.id == drag.id) else {
            return false;
        };
        let Some(origin) = layout.body_origin(m, scroll) else {
            return false;
        };
        let width = region.thumb(0.0).width();
        let track = (region.track.width() - width).max(1.0);
        let value = ((x - origin.0 - region.track.left - drag.grab) / track * region.max_x())
            .clamp(0.0, region.max_x());
        self.values
            .entry(drag.message.clone())
            .or_default()
            .insert(region.id, value);
        true
    }
}
