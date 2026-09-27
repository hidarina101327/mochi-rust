//! 计算 AI 服务提供方界面中指针悬停和按压的动画状态。
use super::*;

const DURATION: i64 = 140;

#[derive(Default)]
pub struct Interaction {
    pub hover: Option<Hit>,
    fades: Vec<(Hit, f32, f32, i64)>,
    pressed: Option<(Hit, i64)>,
    pub now: i64,
    pub reduce_motion: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_motion_retargets_without_jumps_and_stops_after_leave() {
        let mut state = Interaction::default();
        state.pointer(Some(Hit::Save), 100);
        state.tick(150);
        let (hit, from, to, start) = state.fades[0];
        let before = Interaction::sample(from, to, start, state.now);
        state.pointer(Some(Hit::Cancel), 150);
        assert_eq!(state.fades[0].0, hit);
        assert_eq!(state.fades[0].1, before);
        state.press(Hit::Cancel, 150);
        assert!(state.animating());
        state.pointer(None, 160);
        state.tick(400);
        assert!(!state.animating());
        assert!(state.fades.is_empty());
    }
}

impl Interaction {
    fn sample(from: f32, to: f32, start: i64, now: i64) -> f32 {
        let t = ((now - start) as f32 / DURATION as f32).clamp(0.0, 1.0);
        from + (to - from) * (1.0 - (1.0 - t).powi(3))
    }
    pub fn pointer(&mut self, hit: Option<Hit>, now: i64) -> bool {
        self.now = now;
        let hit = hit.filter(|h| !matches!(h, Hit::Field(_)));
        if self.hover == hit {
            return false;
        }
        self.hover = hit;
        for (h, from, to, start) in &mut self.fades {
            *from = Self::sample(*from, *to, *start, now);
            *to = if Some(*h) == hit { 1.0 } else { 0.0 };
            *start = now;
        }
        if let Some(h) = hit.filter(|h| !self.fades.iter().any(|(v, ..)| v == h)) {
            self.fades.push((h, 0.0, 1.0, now));
        }
        true
    }
    pub fn press(&mut self, hit: Hit, now: i64) {
        self.now = now;
        if !matches!(hit, Hit::Field(_)) {
            self.pressed = Some((hit, now));
        }
    }
    pub fn tick(&mut self, now: i64) {
        self.now = now;
        self.fades
            .retain(|(_, _, to, start)| *to > 0.0 || now - *start < DURATION);
        if self
            .pressed
            .is_some_and(|(_, start)| now - start >= DURATION)
        {
            self.pressed = None;
        }
    }
    pub fn animating(&self) -> bool {
        self.pressed.is_some()
            || self
                .fades
                .iter()
                .any(|(_, from, to, start)| from != to && self.now - start < DURATION)
    }
    pub fn button(
        &self,
        list: &mut DrawList,
        rect: Rect,
        hit: Hit,
        label: &str,
        primary: bool,
        p: &Palette,
    ) {
        let hover = self
            .fades
            .iter()
            .find(|(h, ..)| *h == hit)
            .map(|(_, from, to, start)| Self::sample(*from, *to, *start, self.now))
            .unwrap_or(0.0);
        let press = self
            .pressed
            .filter(|(h, _)| *h == hit)
            .map(|(_, start)| 1.0 - ((self.now - start) as f32 / DURATION as f32).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let inset = if self.reduce_motion { 0.0 } else { press * 1.3 };
        let r = Rect::new(
            rect.left + inset,
            rect.top + inset,
            rect.right - inset,
            rect.bottom - inset,
        );
        let danger = matches!(hit, Hit::Delete(_));
        let accent = if danger { p.danger } else { p.accent };
        if primary {
            list.glass_button(r, 7.0, p, hover.max(press) > 0.3);
        } else {
            let highlight = super::super::theme::mix(accent, p.surface, 0.10);
            let color = super::super::theme::mix(highlight, p.surface, hover.max(press));
            list.rounded_rect(r, 7.0, color);
            list.rounded_border(
                r,
                7.0,
                super::super::theme::mix(accent, p.border, hover.max(press) * 0.7),
            );
        }
        let foreground = if primary {
            p.button_foreground()
        } else if danger {
            p.danger
        } else {
            p.foreground
        };
        list.text_aligned(
            r,
            text::ellipsize(label, TextStyle::Caption, (r.width() - 12.0).max(0.0)),
            TextStyle::Caption,
            foreground,
            Align::Center,
        );
    }
}
