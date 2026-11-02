//! Input polling is independent of invalidation and the next visible time change.
use std::time::{Duration, Instant};
pub(super) const FRAME_INTERVAL: Duration = Duration::from_millis(16);
pub(super) struct FrameBudget {
    dirty: bool,
    input: bool,
    next: Instant,
}
impl FrameBudget {
    pub(super) fn new(now: Instant) -> Self { Self { dirty: true, input: false, next: now } }
    pub(super) fn invalidate(&mut self) { self.dirty = true; }
    pub(super) fn input(&mut self) { self.input = true; }
    pub(super) fn due(&self, now: Instant, flowing: bool) -> bool {
        self.dirty || self.input || flowing || now >= self.next
    }
    pub(super) fn drain(&self, now: Instant, started: Instant) -> bool {
        !self.input && now.saturating_duration_since(started) < FRAME_INTERVAL
    }
    pub(super) fn submitted(&mut self, started: Instant) {
        self.dirty = false;
        self.input = false;
        self.next = started + Duration::from_secs(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_polls_coalesce_until_the_next_visible_time_change() {
        let now = Instant::now();
        let mut frames = FrameBudget::new(now);
        assert!(frames.due(now, false));
        frames.submitted(now);
        assert!(!frames.due(now + Duration::from_millis(999), false));
        assert!(frames.due(now + Duration::from_secs(1), false));
        assert!(frames.due(now, true));
        frames.invalidate();
        frames.invalidate();
        assert!(frames.due(now, false));
        assert!(frames.drain(now + Duration::from_millis(15), now));
        assert!(!frames.drain(now + FRAME_INTERVAL, now));
    }
    #[test]
    fn typed_input_forces_a_frame_before_draining_ready_background_messages() {
        let now = Instant::now();
        let mut frames = FrameBudget::new(now);
        frames.submitted(now);
        let typed = now + Duration::from_millis(2);
        let mut ui = crate::ui::Ui::new(crate::ui::demo::test_world());
        let key = crossterm::event::KeyCode::Char('x').into();
        if ui.input_event(crossterm::event::Event::Key(key)) { frames.input(); }
        assert!(frames.due(typed, false));
        assert!(!frames.drain(typed, typed), "input bypasses the receive drain");
        frames.submitted(typed);
        assert!(!frames.due(typed, false));
    }
}
