//! Column offsets of tab-bar tabs while one is dragged or settling back.

use std::time::Instant;

use crate::render::renderer::tab_span;

/// A shift decays by `1/e` every this many seconds, so ten columns settle in about 120ms.
const SLIDE_TAU_SECS: f32 = 0.04;
/// Longest step taken for one frame, so a stalled frame does not jump the animation.
const MAX_FRAME_SECS: f32 = 0.05;
/// Step assumed for the first frame of an animation.
const FIRST_FRAME_SECS: f32 = 1.0 / 60.0;
/// Below this many columns a shift is drawn as zero, so the animation can end.
const SETTLED_COLUMNS: f32 = 0.5;

/// Per-tab shift from its equal-width slot, in columns. The dragged tab's shift
/// follows the pointer; every other shift decays to zero, which slides the tab
/// into its slot.
#[derive(Default)]
pub(super) struct TabAnim {
    offsets: Vec<f32>,
    /// `offsets` rounded to the whole columns the tab bar draws.
    shifts: Vec<i32>,
    dragged: Option<usize>,
    last_tick: Option<Instant>,
    dirty: bool,
}

impl TabAnim {
    pub(super) fn shifts(&self) -> &[i32] {
        &self.shifts
    }

    pub(super) fn dragged(&self) -> Option<usize> {
        self.dragged
    }

    /// Whether the drawn shifts changed since the last call.
    pub(super) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub(super) fn begin_drag(&mut self, index: usize) {
        if self.dragged != Some(index) {
            self.dragged = Some(index);
            self.dirty = true;
        }
    }

    /// The dragged tab keeps its current shift and starts sliding into its slot.
    pub(super) fn end_drag(&mut self) {
        if self.dragged.take().is_some() {
            self.dirty = true;
        }
    }

    pub(super) fn set_dragged_shift(&mut self, shift: i32) {
        if let Some(offset) = self.dragged.and_then(|d| self.offsets.get_mut(d)) {
            *offset = shift as f32;
        }
        self.sync_shifts();
    }

    /// The tab at `from` was moved to `to`: every tab in between slid one slot, so
    /// give each an offset that keeps it where it was drawn.
    pub(super) fn moved(&mut self, from: usize, to: usize, total_cols: usize) {
        let n = self.offsets.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let col = |i: usize| tab_span(total_cols, n, i).0 as f32;
        let carried = self.offsets[from];
        if from < to {
            for k in from + 1..=to {
                self.offsets[k - 1] = self.offsets[k] + col(k) - col(k - 1);
            }
        } else {
            for k in (to..from).rev() {
                self.offsets[k + 1] = self.offsets[k] + col(k) - col(k + 1);
            }
        }
        self.offsets[to] = carried;
        self.dragged = Some(to);
        self.dirty = true;
        self.sync_shifts();
    }

    /// Advance to `now` for a bar of `count` tabs; `true` while another frame is needed.
    pub(super) fn tick(&mut self, now: Instant, count: usize) -> bool {
        self.resize(count);
        let dt = self.last_tick.map_or(FIRST_FRAME_SECS, |t| now.duration_since(t).as_secs_f32());
        let busy = self.step(dt.min(MAX_FRAME_SECS));
        self.last_tick = busy.then_some(now);
        busy
    }

    fn step(&mut self, dt: f32) -> bool {
        let decay = (-dt / SLIDE_TAU_SECS).exp();
        let mut busy = false;
        for (i, offset) in self.offsets.iter_mut().enumerate() {
            if Some(i) == self.dragged || *offset == 0.0 {
                continue;
            }
            *offset *= decay;
            if offset.abs() < SETTLED_COLUMNS {
                *offset = 0.0;
            } else {
                busy = true;
            }
        }
        self.sync_shifts();
        busy
    }

    /// Tab indices no longer match once tabs are added or closed, so start from rest.
    fn resize(&mut self, count: usize) {
        if self.offsets.len() != count {
            self.offsets.clear();
            self.offsets.resize(count, 0.0);
            self.shifts.clear();
            self.shifts.resize(count, 0);
            self.dragged = self.dragged.filter(|&d| d < count);
            self.dirty = true;
        }
    }

    fn sync_shifts(&mut self) {
        for (shift, offset) in self.shifts.iter_mut().zip(&self.offsets) {
            let rounded = offset.round() as i32;
            if *shift != rounded {
                *shift = rounded;
                self.dirty = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::TabAnim;

    fn anim(count: usize) -> TabAnim {
        let mut a = TabAnim::default();
        a.tick(Instant::now(), count);
        a.take_dirty();
        a
    }

    /// Columns where tab `i` is drawn: slot start plus shift.
    fn drawn_left(a: &TabAnim, total: usize, i: usize) -> i32 {
        crate::render::renderer::tab_span(total, a.shifts.len(), i).0 as i32 + a.shifts[i]
    }

    #[test]
    fn a_released_shift_decays_to_rest_and_stops_asking_for_frames() {
        let mut a = anim(3);
        a.begin_drag(1);
        a.set_dragged_shift(12);
        a.end_drag();
        let mut now = Instant::now();
        let mut frames = 0;
        while a.tick(now, 3) {
            now += Duration::from_millis(16);
            frames += 1;
            assert!(frames < 60, "animation never settled");
        }
        assert_eq!(a.shifts(), [0, 0, 0]);
        assert!(frames >= 3, "a 12 column slide takes several frames, took {frames}");
        assert!(!a.tick(now, 3), "idle after settling");
    }

    #[test]
    fn the_dragged_shift_is_not_decayed() {
        let mut a = anim(3);
        a.begin_drag(2);
        a.set_dragged_shift(-9);
        let mut now = Instant::now();
        for _ in 0..30 {
            now += Duration::from_millis(16);
            assert!(!a.tick(now, 3), "a held tab needs no animation frames");
        }
        assert_eq!(a.shifts(), [0, 0, -9]);
    }

    #[test]
    fn stalled_frames_are_clamped() {
        let mut a = anim(2);
        a.begin_drag(0);
        a.set_dragged_shift(40);
        a.end_drag();
        let t0 = Instant::now();
        a.tick(t0, 2);
        a.tick(t0 + Duration::from_secs(10), 2);
        assert!(a.shifts()[0] > 0, "one stalled frame must not finish the slide");
    }

    #[test]
    fn moving_a_tab_keeps_every_other_tab_where_it_was_drawn() {
        for (from, to) in [(0, 3), (3, 0), (1, 2), (2, 1), (0, 1), (3, 2)] {
            let mut a = anim(4);
            // Tab 0 is mid-slide so carried offsets are exercised, not just slot arithmetic.
            a.offsets[0] = if from == 0 { 0.0 } else { 3.0 };
            a.sync_shifts();
            let before: Vec<i32> = (0..4).map(|i| drawn_left(&a, 82, i)).collect();
            a.begin_drag(from);
            a.moved(from, to, 82);
            assert_eq!(a.dragged(), Some(to));
            let mut expected = before.clone();
            let carried = expected.remove(from);
            expected.insert(to, carried);
            let after: Vec<i32> = (0..4).map(|i| drawn_left(&a, 82, i)).collect();
            for i in (0..4).filter(|&i| i != to) {
                assert_eq!(after[i], expected[i], "move {from}->{to}, tab now at {i}");
            }
        }
    }

    #[test]
    fn invalid_moves_and_resizes_are_safe() {
        let mut a = anim(3);
        a.moved(0, 3, 80);
        a.moved(2, 2, 80);
        assert!(!a.take_dirty());
        a.begin_drag(1);
        a.set_dragged_shift(5);
        a.tick(Instant::now(), 2);
        assert_eq!(a.shifts(), [0, 0], "a changed tab count resets the shifts");
        assert_eq!(a.dragged(), Some(1));
        a.tick(Instant::now(), 1);
        assert_eq!(a.dragged(), None, "the dragged index fell off the end");
    }

    #[test]
    fn dirty_is_reported_once_per_change() {
        let mut a = anim(2);
        a.begin_drag(0);
        assert!(a.take_dirty());
        assert!(!a.take_dirty());
        a.set_dragged_shift(0);
        assert!(!a.take_dirty(), "an unchanged shift needs no redraw");
        a.set_dragged_shift(4);
        assert!(a.take_dirty());
    }
}
