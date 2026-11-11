//! A second SIGWINCH once a burst of resizes settles. Programs can miss the
//! last signal of a burst: vim reads the size, and a signal that lands before
//! it clears its "resized" flag is lost, leaving it drawn for a stale size.
//! A single resize (font zoom, a split) is not signalled again: a shell would
//! only redraw its prompt once more.

use std::time::{Duration, Instant};

/// Quiet time after the last resize before panes are signalled again: long
/// enough to skip the steps of a window drag, short enough to look immediate.
pub(super) const SETTLE: Duration = Duration::from_millis(100);

/// Panes resized since the last re-signal with how often, and when to send it.
#[derive(Default)]
pub(super) struct WinchResignal {
    due: Option<Instant>,
    panes: Vec<(u64, u32)>,
}

impl WinchResignal {
    /// Pane `pane` was just resized: (re)start the settle timer.
    pub(super) fn resized(&mut self, pane: u64, now: Instant) {
        match self.panes.iter_mut().find(|(id, _)| *id == pane) {
            Some((_, count)) => *count = count.saturating_add(1),
            None => self.panes.push((pane, 1)),
        }
        self.due = Some(now + SETTLE);
    }

    /// When the event loop must wake to send the re-signal, if one is pending.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// The panes resized more than once in a burst that has settled by `now`.
    pub(super) fn take_due(&mut self, now: Instant) -> Option<Vec<u64>> {
        if !self.due.is_some_and(|due| now >= due) {
            return None;
        }
        self.due = None;
        let bursts = self.panes.iter().filter(|&&(_, count)| count > 1).map(|&(id, _)| id).collect();
        self.panes.clear();
        Some(bursts)
    }
}

/// Background tabs are brought to the window size once a resize burst
/// settles, not on every step: a width change re-wraps the whole scrollback
/// of every pane on the UI thread.
#[derive(Default)]
pub(super) struct BackgroundResize {
    due: Option<Instant>,
}

impl BackgroundResize {
    /// The visible tab was just resized: (re)start the settle timer.
    pub(super) fn schedule(&mut self, now: Instant) {
        self.due = Some(now + SETTLE);
    }

    /// When the event loop must wake to resize the other tabs, if pending.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.due
    }

    /// True once, when the burst has settled by `now`.
    pub(super) fn take_due(&mut self, now: Instant) -> bool {
        if !self.due.is_some_and(|due| now >= due) {
            return false;
        }
        self.due = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_due_without_a_resize() {
        let mut w = WinchResignal::default();
        let t0 = Instant::now();
        assert_eq!(w.deadline(), None);
        assert_eq!(w.take_due(t0 + SETTLE * 10), None);
    }

    #[test]
    fn signals_resized_panes_once_the_burst_settles() {
        let mut w = WinchResignal::default();
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        // A drag: pane 1 every 16 ms, pane 2 twice in the middle.
        for step in 0..10u32 {
            w.resized(1, t0 + ms(16) * step);
            if (3..5).contains(&step) {
                w.resized(2, t0 + ms(16) * step);
            }
        }
        let last = t0 + ms(16) * 9;
        assert_eq!(w.deadline(), Some(last + SETTLE), "each resize restarts the timer");
        assert_eq!(w.take_due(last + SETTLE - ms(1)), None, "not before it settles");
        assert_eq!(w.take_due(last + SETTLE), Some(vec![1, 2]), "each pane once");
        assert_eq!(w.deadline(), None);
        assert_eq!(w.take_due(last + SETTLE * 2), None, "signalled only once per burst");
    }

    #[test]
    fn a_single_resize_is_not_signalled_again() {
        let mut w = WinchResignal::default();
        let t0 = Instant::now();
        w.resized(1, t0);
        w.resized(2, t0);
        w.resized(2, t0);
        assert_eq!(w.take_due(t0 + SETTLE), Some(vec![2]), "pane 1 changed size once");
    }

    #[test]
    fn a_later_burst_signals_again() {
        let mut w = WinchResignal::default();
        let t0 = Instant::now();
        w.resized(3, t0);
        w.resized(3, t0);
        assert_eq!(w.take_due(t0 + SETTLE), Some(vec![3]));
        w.resized(3, t0 + SETTLE * 5);
        w.resized(3, t0 + SETTLE * 5);
        assert_eq!(w.take_due(t0 + SETTLE * 6), Some(vec![3]), "counts restart per burst");
    }

    #[test]
    fn background_tabs_wait_for_the_burst_to_settle() {
        let mut b = BackgroundResize::default();
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(b.deadline(), None);
        assert!(!b.take_due(t0 + SETTLE * 10), "nothing scheduled");
        for step in 0..10u32 {
            b.schedule(t0 + ms(16) * step);
        }
        let last = t0 + ms(16) * 9;
        assert_eq!(b.deadline(), Some(last + SETTLE), "each resize restarts the timer");
        assert!(!b.take_due(last + SETTLE - ms(1)), "not before it settles");
        assert!(b.take_due(last + SETTLE));
        assert_eq!(b.deadline(), None);
        assert!(!b.take_due(last + SETTLE * 2), "due only once per burst");
    }
}
