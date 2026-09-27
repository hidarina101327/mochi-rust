//! 与文本排版和具体输入框无关的光标反馈。
use super::layout::Rect;

pub fn now_ms() -> u64 {
    static EPOCH: std::sync::LazyLock<std::time::Instant> =
        std::sync::LazyLock::new(std::time::Instant::now);
    EPOCH.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Default)]
pub struct CaretFeedback {
    focused: bool,
    activity_ms: u64,
    rect: Option<Rect>,
}

impl CaretFeedback {
    pub fn focus(&mut self, focused: bool, now_ms: u64) {
        self.focused = focused;
        self.activity(now_ms);
    }

    pub fn activity(&mut self, now_ms: u64) {
        self.activity_ms = now_ms;
    }

    pub fn geometry(&mut self, rect: Option<Rect>, now_ms: u64) {
        if self.rect != rect {
            self.activity(now_ms);
            self.rect = rect;
        }
    }

    pub fn visible(&self, now_ms: u64, period_ms: Option<u32>) -> bool {
        self.focused
            && self.rect.is_some()
            && period_ms.is_none_or(|period| {
                now_ms.saturating_sub(self.activity_ms) / u64::from(period.max(1)) % 2 == 0
            })
    }

    pub fn next_tick(&self, now_ms: u64, period_ms: Option<u32>) -> Option<u32> {
        if !self.focused || self.rect.is_none() {
            return None;
        }
        let period = period_ms?.max(1);
        Some(period - (now_ms.saturating_sub(self.activity_ms) % u64::from(period)) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_resets_on_input_and_stops_when_unfocused() {
        let mut feedback = CaretFeedback::default();
        feedback.focus(true, 0);
        feedback.geometry(Some(Rect::from_size(10.0, 20.0, 1.0, 24.0)), 0);
        assert!(feedback.visible(499, Some(500)));
        assert!(!feedback.visible(500, Some(500)));
        assert!(feedback.visible(1000, Some(500)));
        feedback.activity(600);
        assert!(feedback.visible(600, Some(500)));
        assert_eq!(feedback.next_tick(700, Some(500)), Some(400));
        feedback.focus(false, 800);
        assert!(!feedback.visible(800, Some(500)));
        assert_eq!(feedback.next_tick(800, Some(500)), None);
    }

    #[test]
    fn disabled_system_blink_is_steady_and_has_no_timer() {
        let mut feedback = CaretFeedback::default();
        feedback.focus(true, 0);
        feedback.geometry(Some(Rect::from_size(0.0, 0.0, 1.0, 20.0)), 0);
        assert!(feedback.visible(u64::MAX, None));
        assert_eq!(feedback.next_tick(0, None), None);
        feedback.geometry(None, 1);
        assert!(!feedback.visible(1, None));
    }
}
