//! In-app benchmark drivers that need a real window: `+bench-input-latency`.
//! The pure parts (stages, report, echo child) live in `crate::bench_input`.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::keyboard::handle_key;
use super::state::State;
use super::window::render_active;
use crate::bench_input::{self, Sample};
use crate::input::{Key, KeyCode, KeyInput, KeyLocation, PhysicalKey};
use crate::Waker;

/// Program (argv\[0\] is the path) that new panes run instead of the shell.
/// Set once, before the event loop starts, by a benchmark command only.
static PANE_PROGRAM: OnceLock<Vec<String>> = OnceLock::new();

pub(crate) fn set_pane_program(argv: Vec<String>) {
    let _ = PANE_PROGRAM.set(argv);
}

/// The benchmark program for new panes, if one was set.
pub(super) fn pane_program() -> Option<&'static [String]> {
    PANE_PROGRAM.get().map(Vec::as_slice).filter(|argv| !argv.is_empty())
}

/// Keys typed by the benchmark, in order; consecutive keys always differ so a
/// stale echo is never mistaken for the new one.
const KEYS: [(char, KeyCode); 4] = [('a', KeyCode::KeyA), ('s', KeyCode::KeyS), ('d', KeyCode::KeyD), ('f', KeyCode::KeyF)];

fn key_event(index: usize, pressed: bool) -> (u8, KeyInput) {
    let (ch, code) = KEYS[index % KEYS.len()];
    let text = ch.to_string();
    let ev = KeyInput {
        logical: Key::Character(text.clone()),
        unshifted: Key::Character(text.clone()),
        physical: PhysicalKey::Code(code),
        location: KeyLocation::Standard,
        text: Some(text),
        pressed,
        repeat: false,
    };
    (ch as u8, ev)
}

/// The byte in the active pane's top-left cell.
fn top_left(s: &State) -> Option<u8> {
    let pane = s.tabs.get(s.active)?.active_pane()?;
    let g = pane.grid.lock();
    let cp = g.display_row(0).first()?.codepoint;
    u8::try_from(cp).ok()
}

struct Pending {
    key: u8,
    t_key: Instant,
    t_queued: Instant,
    t_seen: Option<Instant>,
}

pub(super) enum Step {
    /// Wake again at this instant (or on the next event).
    Continue(Option<Instant>),
    /// Finished: the report line, or why the run failed.
    Done(Result<String, String>),
}

pub(super) struct InputLatencyRun {
    target: usize,
    started: Instant,
    ready: bool,
    typed: usize,
    next_at: Instant,
    seed: u64,
    pending: Option<Pending>,
    samples: Vec<Sample>,
    frames_before: u64,
}

impl InputLatencyRun {
    pub(super) fn new(target: usize) -> Self {
        let now = Instant::now();
        Self {
            target,
            started: now,
            ready: false,
            typed: 0,
            next_at: now,
            seed: 0x9E37_79B9_7F4A_7C15,
            pending: None,
            samples: Vec::with_capacity(target),
            frames_before: 0,
        }
    }

    /// Called from `about_to_wait`: wait for the echo child, then type the next key.
    pub(super) fn tick(&mut self, s: &mut State, waker: &Waker, now: Instant) -> Step {
        if !self.ready {
            if top_left(s) == Some(bench_input::READY_MARK) {
                self.ready = true;
                // Let the first frames and glyph rasterization settle.
                self.next_at = now + Duration::from_millis(500);
            } else if now.duration_since(self.started) > bench_input::READY_DEADLINE {
                return Step::Done(Err("the echo child never reported ready".into()));
            } else {
                return Step::Continue(Some(now + Duration::from_millis(10)));
            }
        }
        if let Some(p) = &self.pending {
            if now.duration_since(p.t_key) > bench_input::SAMPLE_DEADLINE {
                return Step::Done(Err(format!("no frame showed the echo of {:?} within {:?}", p.key as char, bench_input::SAMPLE_DEADLINE)));
            }
            return Step::Continue(Some(p.t_key + bench_input::SAMPLE_DEADLINE));
        }
        if now < self.next_at {
            return Step::Continue(Some(self.next_at));
        }
        let (key, press) = key_event(self.typed, true);
        let (_, release) = key_event(self.typed, false);
        self.typed += 1;
        let t_key = Instant::now();
        handle_key(s, &press, waker);
        let t_queued = Instant::now();
        handle_key(s, &release, waker);
        self.pending = Some(Pending { key, t_key, t_queued, t_seen: None });
        Step::Continue(Some(t_key + bench_input::SAMPLE_DEADLINE))
    }

    /// Called right before a draw: note when the echo first reached the grid.
    pub(super) fn before_render(&mut self, s: &State) {
        self.frames_before = s.renderer.frames_submitted();
        if let Some(p) = &mut self.pending
            && p.t_seen.is_none()
            && top_left(s) == Some(p.key)
        {
            p.t_seen = Some(Instant::now());
        }
    }

    /// Called right after a draw. A frame submitted after the echo was seen
    /// contains it: wait for the GPU and record the sample.
    pub(super) fn after_render(&mut self, s: &mut State) -> Option<Result<String, String>> {
        let p = self.pending.as_ref()?;
        let t_seen = p.t_seen?;
        if s.renderer.frames_submitted() == self.frames_before {
            // Nothing was drawn (no drawable, synchronized output): try again.
            s.window.request_redraw();
            return None;
        }
        let t_submit = Instant::now();
        s.renderer.wait_gpu_idle();
        let t_done = Instant::now();
        let ms = |a: Instant, b: Instant| b.duration_since(a).as_secs_f64() * 1000.0;
        let sample = Sample {
            input_ms: ms(p.t_key, p.t_queued),
            echo_ms: ms(p.t_queued, t_seen),
            render_ms: ms(t_seen, t_submit),
            gpu_ms: ms(t_submit, t_done),
        };
        self.pending = None;
        if self.typed > bench_input::WARMUP {
            self.samples.push(sample);
        }
        self.next_at = t_done + bench_input::next_gap(&mut self.seed);
        if self.samples.len() < self.target {
            return None;
        }
        let size = s.window.inner_size();
        let info = bench_input::RunInfo {
            width_px: size.width,
            height_px: size.height,
            scale_factor: s.window.scale_factor(),
            monitor_millihz: s.window.current_monitor().and_then(|m| m.refresh_rate_millihertz).unwrap_or(0),
        };
        Some(Ok(bench_input::report(&self.samples, &info)))
    }
}

/// Draw `s`, letting a running input-latency benchmark observe the frame.
/// Every draw of a window goes through here (redraw requests and PTY wake-ups).
pub(super) fn render_observed(s: &mut State, run: &mut Option<InputLatencyRun>, result: &mut Option<Result<String, String>>) {
    let Some(active) = run else {
        render_active(s);
        return;
    };
    active.before_render(s);
    render_active(s);
    if let Some(done) = active.after_render(s) {
        *run = None;
        *result = Some(done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_keys_never_repeat_back_to_back() {
        for i in 0..20 {
            assert_ne!(key_event(i, true).0, key_event(i + 1, true).0);
        }
    }

    #[test]
    fn key_events_carry_matching_text_and_state() {
        let (byte, press) = key_event(1, true);
        assert_eq!(byte, b's');
        assert_eq!(press.text.as_deref(), Some("s"));
        assert_eq!(press.physical, PhysicalKey::Code(KeyCode::KeyS));
        assert!(press.pressed && !press.repeat);
        assert!(!key_event(1, false).1.pressed);
    }
}
