//! Frame-callback starvation is an advisory visibility signal, not proof
//! that a surface is covered: a compositor or GPU stall can look the same.
use std::time::{Duration, Instant};

const FRAME_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
pub(super) struct Visibility {
    deadline: Option<Instant>,
    occluded: bool,
}
impl Visibility {
    pub(super) fn presenting(&mut self, now: Instant) {
        if !self.occluded { self.deadline = Some(now + FRAME_TIMEOUT); }
    }
    pub(super) fn skipped(&mut self) { self.deadline = None; }
    pub(super) fn deadline(&self) -> Option<Instant> { self.deadline }
    pub(super) fn expire(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|at| now >= at) {
            self.deadline = None;
            self.occluded = true;
            true
        } else { false }
    }
    /// Even an unsolicited callback restores visibility after a skipped
    /// present; only an actual callback may undo an inferred occlusion.
    pub(super) fn frame(&mut self) -> bool {
        self.deadline = None;
        std::mem::take(&mut self.occluded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_visible_idle_window_never_has_a_visibility_timer() {
        let mut state = Visibility::default();
        assert_eq!(state.deadline(), None);
        assert!(!state.expire(Instant::now()));
    }
    #[test]
    fn a_starved_frame_occludes_once_and_its_callback_restores_once() {
        let now = Instant::now();
        let mut state = Visibility::default();
        state.presenting(now);
        assert!(!state.expire(now + FRAME_TIMEOUT / 2));
        assert!(state.expire(now + FRAME_TIMEOUT));
        assert_eq!(state.deadline(), None, "no polling while hidden");
        assert!(!state.expire(now + FRAME_TIMEOUT * 2));
        assert!(state.frame());
        assert!(!state.frame());
        state.presenting(now);
        assert!(state.deadline().is_some());
    }
    #[test]
    fn normal_callbacks_and_skipped_presents_cancel_the_deadline() {
        let now = Instant::now();
        let mut state = Visibility::default();
        state.presenting(now);
        assert!(!state.frame());
        assert!(!state.expire(now + FRAME_TIMEOUT));
        state.presenting(now);
        state.skipped();
        assert!(!state.expire(now + FRAME_TIMEOUT));
    }
}
