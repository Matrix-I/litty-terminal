//! Preserve sub-line wheel movement without allocating or scheduling idle frames.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pane(u64),
    History,
}

#[derive(Default)]
pub struct Accumulator {
    remainder: f64,
    context: Option<(Target, bool, f64)>,
}

impl Accumulator {
    pub fn reset(&mut self) {
        self.remainder = 0.0;
        self.context = None;
    }

    /// `delta` and `unit` are both pixels for precise wheels, otherwise lines.
    /// Keep the remainder in input units to avoid dividing each tiny delta.
    pub fn push(&mut self, delta: f64, unit: f64, target: Target, precise: bool) -> isize {
        if !delta.is_finite() || !unit.is_finite() || unit <= 0.0 {
            self.reset();
            return 0;
        }
        let context = (target, precise, unit);
        if self.context != Some(context) {
            self.reset();
            self.context = Some(context);
        }
        self.remainder += delta;
        let lines = self.remainder / unit;
        // Floating-point addition can leave an exact line just below its boundary.
        let nearest = lines.round();
        let lines = if (lines - nearest).abs() <= 8.0 * f64::EPSILON * lines.abs().max(1.0) {
            nearest as isize
        } else {
            lines.trunc() as isize
        };
        self.remainder -= lines as f64 * unit;
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_pixels_are_conserved_in_both_directions() {
        for direction in [-1.0, 1.0] {
            let mut a = Accumulator::default();
            for _ in 0..11 {
                assert_eq!(a.push(direction * 2.0, 24.0, Target::Pane(1), true), 0);
            }
            assert_eq!(a.push(direction * 2.0, 24.0, Target::Pane(1), true), direction as isize);
        }
    }

    #[test]
    fn event_grouping_does_not_change_distance() {
        for chunks in [vec![2.0; 12], vec![12.0; 2], vec![24.0], vec![11.0, 13.0]] {
            let mut a = Accumulator::default();
            assert_eq!(chunks.into_iter().map(|d| a.push(d, 24.0, Target::Pane(1), true)).sum::<isize>(), 1);
        }
    }

    #[test]
    fn fractional_lines_and_reversals() {
        let mut a = Accumulator::default();
        for _ in 0..9 { assert_eq!(a.push(0.1, 1.0, Target::Pane(1), false), 0); }
        assert_eq!(a.push(0.1, 1.0, Target::Pane(1), false), 1);
        assert_eq!(a.push(0.75, 1.0, Target::Pane(1), false), 0);
        assert_eq!(a.push(-0.5, 1.0, Target::Pane(1), false), 0);
        assert_eq!(a.push(0.75, 1.0, Target::Pane(1), false), 1);
        assert_eq!(a.push(3.0, 1.0, Target::Pane(1), false), 3);
    }

    #[test]
    fn residual_does_not_leak_between_contexts_or_gestures() {
        let mut a = Accumulator::default();
        assert_eq!(a.push(20.0, 24.0, Target::Pane(1), true), 0);
        assert_eq!(a.push(4.0, 24.0, Target::Pane(2), true), 0);
        assert_eq!(a.push(20.0, 24.0, Target::History, true), 0);
        a.reset();
        assert_eq!(a.push(4.0, 24.0, Target::History, true), 0);
        assert_eq!(a.push(0.75, 1.0, Target::History, false), 0);
        assert_eq!(a.push(0.25, 1.0, Target::History, false), 1);
        assert_eq!(a.push(20.0, 24.0, Target::History, true), 0);
        assert_eq!(a.push(12.0, 32.0, Target::History, true), 0);
    }

    #[test]
    fn invalid_input_clears_residual() {
        let mut a = Accumulator::default();
        a.push(20.0, 24.0, Target::Pane(1), true);
        assert_eq!(a.push(f64::NAN, 24.0, Target::Pane(1), true), 0);
        assert_eq!(a.push(4.0, 24.0, Target::Pane(1), true), 0);
    }

    #[test]
    fn wheel_events_do_not_allocate() {
        let mut a = Accumulator::default();
        let allocations = crate::test_alloc::allocations(|| {
            for _ in 0..10_000 {
                std::hint::black_box(a.push(2.0, 24.0, Target::Pane(1), true));
            }
        });
        assert_eq!(allocations, 0);
    }
}
