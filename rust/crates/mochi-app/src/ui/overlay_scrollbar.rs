//! 覆盖式滚动条只负责自身几何位置；不要从内容视口中扣除边距。
use super::{draw::DrawList, layout::Rect, theme::Palette};

pub const HOTZONE: f32 = 12.0;
const INSET: f32 = 3.0;

pub fn hotzone_width() -> f32 {
    (super::settings_values::number("scrollbar.width", 6.0) + INSET * 2.0).max(HOTZONE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub axis: Axis,
    pub hotzone: Rect,
    pub track: Rect,
    pub thumb: Rect,
    pub max: f32,
}

impl Bar {
    pub fn new(area: Rect, axis: Axis, max: f32, offset: f32, other_axis: bool) -> Option<Self> {
        if area.is_empty() || !max.is_finite() || max <= 0.01 {
            return None;
        }
        let thickness = super::settings_values::number("scrollbar.width", 6.0);
        let hotzone_size = hotzone_width();
        let corner = if other_axis { hotzone_size } else { 0.0 };
        let hotzone = match axis {
            Axis::Vertical => Rect::new(
                (area.right - hotzone_size).max(area.left),
                area.top,
                area.right,
                area.bottom - corner,
            ),
            Axis::Horizontal => Rect::new(
                area.left,
                (area.bottom - hotzone_size).max(area.top),
                area.right - corner,
                area.bottom,
            ),
        };
        let track = match axis {
            Axis::Vertical => Rect::new(
                area.right - INSET - thickness,
                hotzone.top + INSET,
                area.right - INSET,
                hotzone.bottom - INSET,
            ),
            Axis::Horizontal => Rect::new(
                hotzone.left + INSET,
                area.bottom - INSET - thickness,
                hotzone.right - INSET,
                area.bottom - INSET,
            ),
        };
        if track.is_empty() || !area.contains(track.left, track.top) {
            return None;
        }
        let viewport = match axis {
            Axis::Vertical => area.height(),
            Axis::Horizontal => area.width(),
        };
        let length = match axis {
            Axis::Vertical => track.height(),
            Axis::Horizontal => track.width(),
        };
        let size = (length * viewport / (viewport + max))
            .max(super::settings_values::number(
                "scrollbar.minThumbSize",
                24.0,
            ))
            .min(length * 0.95);
        let start = (length - size) * offset.clamp(0.0, max) / max;
        let thumb = match axis {
            Axis::Vertical => Rect::new(
                track.left,
                track.top + start,
                track.right,
                track.top + start + size,
            ),
            Axis::Horizontal => Rect::new(
                track.left + start,
                track.top,
                track.left + start + size,
                track.bottom,
            ),
        };
        Some(Self {
            axis,
            hotzone,
            track,
            thumb,
            max,
        })
    }
    fn coordinate(self, x: f32, y: f32) -> f32 {
        match self.axis {
            Axis::Vertical => y,
            Axis::Horizontal => x,
        }
    }
    fn thumb_start(self) -> f32 {
        match self.axis {
            Axis::Vertical => self.thumb.top,
            Axis::Horizontal => self.thumb.left,
        }
    }
    fn thumb_size(self) -> f32 {
        match self.axis {
            Axis::Vertical => self.thumb.height(),
            Axis::Horizontal => self.thumb.width(),
        }
    }
    fn offset_at(self, x: f32, y: f32, grab: f32) -> f32 {
        let (start, length) = match self.axis {
            Axis::Vertical => (self.track.top, self.track.height()),
            Axis::Horizontal => (self.track.left, self.track.width()),
        };
        ((self.coordinate(x, y) - start - grab) / (length - self.thumb_size()).max(1.0))
            .clamp(0.0, 1.0)
            * self.max
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interaction<K> {
    pub hover: Option<K>,
    drag: Option<(K, f32)>,
}
impl<K> Default for Interaction<K> {
    fn default() -> Self {
        Self {
            hover: None,
            drag: None,
        }
    }
}
impl<K: Copy + PartialEq> Interaction<K> {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn hit(bars: &[(K, Bar)], x: f32, y: f32) -> Option<K> {
        bars.iter()
            .rev()
            .find(|(_, b)| b.hotzone.contains(x, y))
            .map(|(id, _)| *id)
    }
    pub fn pointer(&mut self, bars: &[(K, Bar)], x: f32, y: f32) -> bool {
        let next = Self::hit(bars, x, y);
        let changed = self.hover != next;
        self.hover = next;
        changed
    }
    pub fn begin(&mut self, bars: &[(K, Bar)], x: f32, y: f32) -> Option<(K, f32)> {
        let id = Self::hit(bars, x, y)?;
        let bar = bars.iter().find(|(key, _)| *key == id)?.1;
        let pos = bar.coordinate(x, y);
        // 整条 12 DIP 的区域都可拖动滑块，不限于绘制出来的 6 DIP 细条。
        let grab = if pos >= bar.thumb_start() && pos <= bar.thumb_start() + bar.thumb_size() {
            pos - bar.thumb_start()
        } else {
            bar.thumb_size() / 2.0
        };
        self.drag = Some((id, grab));
        self.hover = Some(id);
        Some((id, bar.offset_at(x, y, grab)))
    }
    pub fn drag_to(&mut self, bars: &[(K, Bar)], x: f32, y: f32) -> Option<(K, f32)> {
        let (id, grab) = self.drag?;
        let Some((_, bar)) = bars.iter().find(|(key, _)| *key == id) else {
            self.end();
            return None;
        };
        Some((id, bar.offset_at(x, y, grab)))
    }
    pub fn end(&mut self) -> bool {
        self.drag.take().is_some()
    }
    pub fn paint(&self, list: &mut DrawList, bars: &[(K, Bar)], p: &Palette) {
        for (id, bar) in bars {
            let dragging = self.drag.is_some_and(|(key, _)| key == *id);
            if self.hover != Some(*id)
                && !dragging
                && !super::settings_values::boolean("scrollbar.alwaysVisible", false)
            {
                continue;
            }
            list.push_clip(bar.hotzone);
            list.rounded_rect_alpha(bar.track, 3.0, p.foreground, 0.06);
            list.rounded_rect_alpha(
                bar.thumb,
                3.0,
                super::settings_values::color("scrollbar.color", p.foreground),
                if dragging {
                    super::settings_values::number("scrollbar.activeOpacity", 58.0) / 100.0
                } else {
                    super::settings_values::number("scrollbar.opacity", 38.0) / 100.0
                },
            );
            list.pop_clip();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_is_hidden_away_from_edge_and_never_changes_geometry() {
        let area = Rect::new(0.0, 0.0, 600.0, 400.0);
        let bar = Bar::new(area, Axis::Vertical, 2000.0, 0.0, false).unwrap();
        let bars = [(Axis::Vertical, bar)];
        let mut state = Interaction::default();
        let p = *super::super::theme::tokens().palette(false);
        let mut list = DrawList::new();
        state.pointer(&bars, 300.0, 200.0);
        state.paint(&mut list, &bars, &p);
        assert!(list.is_empty());
        assert!(state.pointer(&bars, 599.0, 200.0));
        state.paint(&mut list, &bars, &p);
        assert!(!list.is_empty());
        assert!(list.finish().is_ok());
        assert_eq!(bar.thumb.width(), 6.0);
        assert_eq!(bar.hotzone.width(), 12.0);
        assert_eq!(
            bar,
            Bar::new(area, Axis::Vertical, 2000.0, 0.0, false).unwrap()
        );
        assert!(state.pointer(&bars, f32::NEG_INFINITY, f32::NEG_INFINITY));
    }
    #[test]
    fn dragging_both_axes_preserves_grab_and_reaches_both_ends() {
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let bar = Bar::new(
                Rect::new(0.0, 0.0, 600.0, 400.0),
                axis,
                4000.0,
                1200.0,
                true,
            )
            .unwrap();
            let bars = [(axis, bar)];
            let mut state = Interaction::default();
            let (x, y) = (bar.thumb.left + 2.0, bar.thumb.top + 2.0);
            assert!((state.begin(&bars, x, y).unwrap().1 - 1200.0).abs() < 0.01);
            assert_eq!(state.drag_to(&bars, -100.0, -100.0).unwrap().1, 0.0);
            assert_eq!(state.drag_to(&bars, 9000.0, 9000.0).unwrap().1, 4000.0);
            state.pointer(&bars, -100.0, -100.0);
            assert!(state.dragging());
            assert!(state.end());
            assert!(!state.dragging());
        }
    }
    #[test]
    fn tiny_empty_and_nonoverflowing_viewports_are_safe() {
        for area in [Rect::ZERO, Rect::new(0.0, 0.0, 4.0, 4.0)] {
            assert!(Bar::new(area, Axis::Vertical, 100.0, 0.0, false).is_none());
        }
        assert!(Bar::new(
            Rect::new(0.0, 0.0, 600.0, 400.0),
            Axis::Vertical,
            0.0,
            0.0,
            false
        )
        .is_none());
    }
}
