//! `+bench-first-frame`: process launch → first GPU-completed frame.
//!
//! The parent re-runs this binary `runs` times as `+bench-first-frame-child
//! <spawn_ns>`, where `spawn_ns` is its `CLOCK_MONOTONIC` reading right before
//! the spawn. That clock is system-wide, so the child can subtract it from its
//! own readings. The child starts the app normally (window, shell, renderer),
//! waits for the GPU to finish the first submitted frame, prints one JSON
//! line with its stage times and exits. The update check is skipped so no run
//! touches the network.
//!
//! Stages: `exec` (spawn → `main`: exec, dynamic loader, runtime start),
//! `init` (`main` → first window created: config, fonts, GPU device, PTY),
//! `first_submit` (window → first frame submitted), `gpu` (submit → complete).
//! Run 1 is reported separately because it may pay cold caches (shader
//! cache, page cache) that later runs do not.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::bench_dsr::summarize;
use crate::json::Json;

pub const CHILD_FLAG: &str = "+bench-first-frame-child";
pub const DEFAULT_RUNS: usize = 10;
const RUN_DEADLINE: Duration = Duration::from_secs(20);

/// `CLOCK_MONOTONIC` in nanoseconds (shared by every process on the host).
pub fn mono_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid out-pointer; CLOCK_MONOTONIC always exists.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

static MAIN_NS: AtomicU64 = AtomicU64::new(0);

/// Record when `main` started; called first thing in `main`.
pub fn mark_main() {
    MAIN_NS.store(mono_ns(), Ordering::Relaxed);
}

pub fn main_ns() -> u64 {
    MAIN_NS.load(Ordering::Relaxed)
}

/// One run, in nanoseconds on `CLOCK_MONOTONIC`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamps {
    pub spawn: u64,
    pub main: u64,
    pub window: u64,
    pub first_submit: u64,
    pub gpu_done: u64,
}

/// One run, in milliseconds per stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stages {
    pub exec_ms: f64,
    pub init_ms: f64,
    pub first_submit_ms: f64,
    pub gpu_ms: f64,
}

impl Stages {
    pub fn from_stamps(s: &Stamps) -> Self {
        let ms = |a: u64, b: u64| b.saturating_sub(a) as f64 / 1e6;
        Self {
            exec_ms: ms(s.spawn, s.main),
            init_ms: ms(s.main, s.window),
            first_submit_ms: ms(s.window, s.first_submit),
            gpu_ms: ms(s.first_submit, s.gpu_done),
        }
    }

    pub fn total_ms(&self) -> f64 {
        self.exec_ms + self.init_ms + self.first_submit_ms + self.gpu_ms
    }

    /// The child's report line.
    pub fn report_line(&self) -> String {
        format!(
            "{{\"exec_ms\":{:.3},\"init_ms\":{:.3},\"first_submit_ms\":{:.3},\"gpu_ms\":{:.3},\"total_ms\":{:.3}}}",
            self.exec_ms,
            self.init_ms,
            self.first_submit_ms,
            self.gpu_ms,
            self.total_ms()
        )
    }

    /// Parse the last non-empty line of a child's stdout.
    pub fn parse(stdout: &str) -> Result<Self, String> {
        let line = stdout.lines().rev().find(|l| !l.trim().is_empty()).ok_or("child printed no report")?;
        let json = Json::parse(line).map_err(|e| format!("malformed child report {line:?}: {e}"))?;
        let field = |k: &str| match json.get(k) {
            Some(Json::Num(n)) => n.parse::<f64>().map_err(|e| format!("bad {k} in {line:?}: {e}")),
            _ => Err(format!("child report lacks {k}: {line:?}")),
        };
        Ok(Self {
            exec_ms: field("exec_ms")?,
            init_ms: field("init_ms")?,
            first_submit_ms: field("first_submit_ms")?,
            gpu_ms: field("gpu_ms")?,
        })
    }
}

fn stats_json(name: &str, values: &[f64]) -> String {
    let s = summarize(values).unwrap_or_default();
    format!(
        "\"{name}\":{{\"min\":{:.3},\"mean\":{:.3},\"p50\":{:.3},\"max\":{:.3}}}",
        s.min, s.mean, s.median, s.max
    )
}

/// Summary line: run 1 as `first_run`, the others summarized per stage.
pub fn summary(runs: &[Stages]) -> String {
    let first = runs.first().map_or_else(|| "null".to_string(), Stages::report_line);
    let rest = runs.get(1..).unwrap_or_default();
    let stage = |f: fn(&Stages) -> f64| rest.iter().map(f).collect::<Vec<_>>();
    format!(
        "{{\"benchmark\":\"first_frame\",\"endpoint\":\"gpu_complete\",\"runs\":{},\"first_run\":{},\"later_runs\":{{{},{},{},{},{}}}}}",
        runs.len(),
        first,
        stats_json("total_ms", &stage(Stages::total_ms)),
        stats_json("exec_ms", &stage(|s| s.exec_ms)),
        stats_json("init_ms", &stage(|s| s.init_ms)),
        stats_json("first_submit_ms", &stage(|s| s.first_submit_ms)),
        stats_json("gpu_ms", &stage(|s| s.gpu_ms)),
    )
}

/// Start one child and wait (bounded) for its report.
fn run_once(exe: &str) -> Result<Stages, String> {
    let spawn = mono_ns();
    let mut child = Command::new(exe)
        .arg(CHILD_FLAG)
        .arg(spawn.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start {exe}: {e}"))?;
    let deadline = Instant::now() + RUN_DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("child did not finish within {}s", RUN_DEADLINE.as_secs()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("waiting for the child failed: {e}"));
            }
        }
    };
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_string(&mut out).map_err(|e| format!("reading the child report failed: {e}"))?;
    }
    if !status.success() {
        return Err(format!("child exited with {status}"));
    }
    Stages::parse(&out)
}

/// Parent side: print each run's line and the summary; `Err` on the first failure.
pub fn run(runs: usize) -> Result<(), String> {
    let exe = crate::bench_input::current_exe()?;
    let mut results = Vec::with_capacity(runs);
    for i in 0..runs {
        let stages = run_once(&exe).map_err(|e| format!("run {}: {e}", i + 1))?;
        println!("{{\"run\":{},\"stages\":{}}}", i + 1, stages.report_line());
        results.push(stages);
    }
    println!("{}", summary(&results));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_split_the_stamps() {
        let s = Stamps { spawn: 1_000_000, main: 3_000_000, window: 53_000_000, first_submit: 60_000_000, gpu_done: 62_500_000 };
        let st = Stages::from_stamps(&s);
        assert_eq!(st, Stages { exec_ms: 2.0, init_ms: 50.0, first_submit_ms: 7.0, gpu_ms: 2.5 });
        assert_eq!(st.total_ms(), 61.5);
    }

    #[test]
    fn out_of_order_stamps_never_go_negative() {
        let s = Stamps { spawn: 10, main: 5, window: 5, first_submit: 5, gpu_done: 5 };
        assert_eq!(Stages::from_stamps(&s).exec_ms, 0.0);
    }

    #[test]
    fn child_report_roundtrips() {
        let st = Stages { exec_ms: 1.5, init_ms: 40.25, first_submit_ms: 6.0, gpu_ms: 2.125 };
        let printed = format!("noise from the shell\n{}\n", st.report_line());
        assert_eq!(Stages::parse(&printed), Ok(st));
    }

    #[test]
    fn broken_child_reports_are_errors() {
        assert!(Stages::parse("").is_err());
        assert!(Stages::parse("not json").is_err());
        assert!(Stages::parse("{\"exec_ms\":1}").is_err());
    }

    #[test]
    fn summary_reports_run_one_apart_from_the_rest() {
        let run = |t: f64| Stages { exec_ms: 0.0, init_ms: t, first_submit_ms: 0.0, gpu_ms: 0.0 };
        let json = summary(&[run(500.0), run(10.0), run(30.0)]);
        assert!(json.contains("\"runs\":3"));
        assert!(json.contains("\"first_run\":{\"exec_ms\":0.000,\"init_ms\":500.000"));
        assert!(json.contains("\"init_ms\":{\"min\":10.000,\"mean\":20.000,\"p50\":10.000,\"max\":30.000}"));
        assert!(Json::parse(&json).is_ok(), "summary must be valid JSON: {json}");
    }

    #[test]
    fn mono_clock_moves_forward() {
        let a = mono_ns();
        let b = mono_ns();
        assert!(b >= a && a > 0);
    }
}
