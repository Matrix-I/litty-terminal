//! In-app benchmark drivers that need a real window: `+bench-input-latency`
//! and the `+bench-first-frame` child. The pure parts (stages, reports, child
//! programs) live in `crate::bench_input` and `crate::bench_startup`.

use std::time::Instant;

use parking_lot::Mutex;

use super::keyboard::handle_key;
use super::state::State;
use super::window::render_active;
use crate::bench_input::{self, Sample};
use crate::bench_startup;
use crate::input::{Key, KeyCode, KeyInput, KeyLocation, PhysicalKey};
use crate::Waker;

/// Program (argv\[0\] is the path) the next new pane runs instead of the
/// shell. Set before the event loop starts by a benchmark command only, and
/// taken by the first pane, so later tabs and splits get a normal shell.
static PANE_PROGRAM: Mutex<Option<Vec<String>>> = Mutex::new(None);

pub(crate) fn set_pane_program(argv: Vec<String>) {
    *PANE_PROGRAM.lock() = Some(argv).filter(|argv| !argv.is_empty());
}

/// The benchmark program for this pane, if one is still waiting.
pub(super) fn take_pane_program() -> Option<Vec<String>> {
    PANE_PROGRAM.lock().take()
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
                self.next_at = now + bench_input::SETTLE_DELAY;
            } else if now.duration_since(self.started) > bench_input::READY_DEADLINE {
                return Step::Done(Err("the echo child never reported ready".into()));
            } else {
                return Step::Continue(Some(now + bench_input::READY_POLL));
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
        if !s.renderer.wait_gpu_idle(bench_input::SAMPLE_DEADLINE) {
            self.pending = None;
            return Some(Err("the GPU did not complete a frame".into()));
        }
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

/// `+bench-first-frame-child`: stamps the startup stages, then reports once
/// the GPU has finished the first submitted frame.
pub(super) struct FirstFrameRun {
    spawn_ns: u64,
    window_ns: Option<u64>,
}

impl FirstFrameRun {
    pub(super) fn new(spawn_ns: u64) -> Self {
        Self { spawn_ns, window_ns: None }
    }

    /// The first window exists (its renderer, fonts and PTY are set up).
    pub(super) fn window_created(&mut self) {
        self.window_ns = Some(bench_startup::mono_ns());
    }

    fn after_render(&mut self, s: &mut State) -> Option<Result<String, String>> {
        if s.renderer.frames_submitted() == 0 {
            return None;
        }
        let first_submit = bench_startup::mono_ns();
        if !s.renderer.wait_gpu_idle(bench_startup::GPU_DEADLINE) {
            return Some(Err("the GPU did not complete the first frame".into()));
        }
        let gpu_done = bench_startup::mono_ns();
        let stamps = bench_startup::Stamps {
            spawn: self.spawn_ns,
            main: bench_startup::main_ns(),
            window: self.window_ns.unwrap_or(first_submit),
            first_submit,
            gpu_done,
        };
        Some(Ok(bench_startup::Stages::from_stamps(&stamps).report_line()))
    }
}

/// The in-app benchmark this process runs, if any.
pub(super) enum GuiBench {
    InputLatency(InputLatencyRun),
    FirstFrame(FirstFrameRun),
    /// `+bench-pty-throughput-gui`: a normal app whose pane runs the stream
    /// child; the parent process times and closes it.
    Throughput,
}

/// Draw `s`, letting a running benchmark observe the frame. Every draw of a
/// window goes through here (redraw requests and PTY wake-ups).
pub(super) fn render_observed(s: &mut State, bench: &mut Option<GuiBench>, result: &mut Option<Result<String, String>>) {
    let done = match bench {
        None | Some(GuiBench::Throughput) => {
            render_active(s);
            return;
        }
        Some(GuiBench::InputLatency(run)) => {
            run.before_render(s);
            render_active(s);
            run.after_render(s)
        }
        Some(GuiBench::FirstFrame(run)) => {
            render_active(s);
            run.after_render(s)
        }
    };
    if let Some(done) = done {
        *bench = None;
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
