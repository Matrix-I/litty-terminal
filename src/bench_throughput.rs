//! `+bench-pty-throughput`: bytes per second through the whole app.
//!
//! For each stream kind the parent starts the app as
//! `+bench-pty-throughput-gui <kind> <MiB> <report>`. That app's first pane runs
//! `+bench-cat-child`, which generates the stream in memory (untimed) and then
//! writes it to its terminal as fast as the kernel accepts it, like `cat`. The
//! app reads, parses and draws it normally, so a slow reader or a long grid
//! lock shows up as back-pressure on the child's writes. The child times the
//! write and leaves `<bytes> <nanoseconds>` in the report file; the parent then
//! closes the app. The timer stops when the kernel accepts the last byte, so
//! the last PTY buffer's worth (a few KB) is parsed and drawn after it: use
//! streams of many MiB, where that tail is negligible.
//!
//! The parent also feeds the same bytes to a bare `Grid` (no PTY, no drawing)
//! so each kind reports the in-app rate next to the parser-only rate.

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::bench_dsr::write_all;
use crate::terminal::grid::Grid;

pub const CAT_CHILD_FLAG: &str = "+bench-cat-child";
pub const GUI_FLAG: &str = "+bench-pty-throughput-gui";
pub const DEFAULT_MIB: usize = 64;
/// Upper bound for the stream size; the parent and the child each hold one copy.
pub const MAX_MIB: usize = 4096;
pub const KINDS: [&str; 6] = ["ascii", "sgr", "code", "unicode", "osc8", "region"];
const RUN_DEADLINE: Duration = Duration::from_secs(180);
const GRID_RUNS: usize = 3;
/// How often the parent looks for the child's report.
const REPORT_POLL: Duration = Duration::from_millis(20);

/// Lines of one stream kind, repeated until the stream holds at least `bytes`.
/// Lines end in `\r\n`, so the terminal driver's output processing has no
/// newline translation to do.
pub fn generate(kind: &str, bytes: usize) -> Option<Vec<u8>> {
    let mut s = String::with_capacity(bytes + 256);
    if kind == "region" {
        // A scroll region (as vim, less and tmux use): every newline scrolls rows 2..=20.
        s.push_str("\x1b[2;20r\x1b[20;1H");
    }
    let mut i = 0usize;
    while s.len() < bytes {
        match kind {
            "ascii" => s.push_str("lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor\r\n"),
            "sgr" => {
                let _ = write!(
                    s,
                    "\x1b[2m2026-11-08T12:00:{:02}Z\x1b[0m \x1b[1;32mINFO\x1b[0m \x1b[36mhttp\x1b[0m req={i} path=/api/v1/items status=200\r\n",
                    i % 60
                );
            }
            "code" => s.push_str("fn main() { let x: Vec<u32> = (0..10).map(|i| i * 2).collect(); if x.len() >= 3 && x[0] != 1 { println!(\"{:?}\", x); } } // => -> !=\r\n"),
            "unicode" => s.push_str("Tiếng Việt có dấu ở đây — 日本語のテキスト 한국어 😀👍 mixed ascii text\r\n"),
            "osc8" => {
                let _ = write!(s, "\x1b]8;;https://perf.test/{}\x1b\\link {i}\x1b]8;;\x1b\\\r\n", i % 400);
            }
            "region" => {
                let _ = write!(s, "region scroll line {i} alpha beta gamma delta\r\n");
            }
            _ => return None,
        }
        i += 1;
    }
    if kind == "region" {
        s.push_str("\x1b[r");
    }
    Some(s.into_bytes())
}

/// The report the child leaves: bytes written and nanoseconds it took.
pub fn parse_report(text: &str) -> Result<(u64, u64), String> {
    let mut parts = text.split_whitespace().map(str::parse::<u64>);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(bytes)), Some(Ok(ns)), None) if ns > 0 => Ok((bytes, ns)),
        _ => Err(format!("malformed child report {text:?}")),
    }
}

pub fn mb_per_s(bytes: u64, ns: u64) -> f64 {
    if ns == 0 {
        return 0.0;
    }
    bytes as f64 / (1024.0 * 1024.0) / (ns as f64 / 1e9)
}

/// Write the report through a temporary file and `rename`, so the parent never
/// reads a half-written one. `create_new` + `O_NOFOLLOW`: never write through a
/// pre-planted file or symlink.
fn write_report(out: &Path, bytes: u64, ns: u64) -> std::io::Result<()> {
    let tmp = out.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    writeln!(f, "{bytes} {ns}")?;
    drop(f);
    std::fs::rename(&tmp, out)
}

/// Child side, run on the pane's PTY. Returns the process exit code.
pub fn run_cat_child(kind: &str, mib: usize, out: &Path) -> i32 {
    let Some(data) = generate(kind, mib * 1024 * 1024) else { return 2 };
    let start = Instant::now();
    if !write_all(1, &data) {
        return 5;
    }
    let ns = start.elapsed().as_nanos().max(1) as u64;
    if write_report(out, data.len() as u64, ns).is_err() { 6 } else { 0 }
}

/// Parser-only rate for the same bytes: best of a few runs on a 160×50 grid.
fn grid_only_mb_per_s(data: &[u8]) -> f64 {
    let mut best = u64::MAX;
    for _ in 0..GRID_RUNS {
        let mut grid = Grid::new(160, 50);
        let mut parser = vte::Parser::new();
        let start = Instant::now();
        let mut off = 0;
        while off < data.len() {
            off += grid.feed_bytes(&mut parser, &data[off..]);
        }
        std::hint::black_box(&grid);
        best = best.min(start.elapsed().as_nanos().max(1) as u64);
    }
    mb_per_s(data.len() as u64, best)
}

/// A fresh directory only this user can enter, for one run's report. Creating
/// it fails if the name exists, so nobody can plant a report in advance.
fn private_dir(kind: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!(
        "litty-bench-throughput-{}-{kind}-{}",
        std::process::id(),
        crate::bench_startup::mono_ns()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Run the app on one stream kind and wait (bounded) for the child's report.
fn run_kind(exe: &str, kind: &str, mib: usize) -> Result<(u64, u64), String> {
    let dir = private_dir(kind)?;
    let result = run_kind_in(exe, kind, mib, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn run_kind_in(exe: &str, kind: &str, mib: usize, dir: &Path) -> Result<(u64, u64), String> {
    let out = dir.join("report.txt");
    let out_str = out.to_str().ok_or("report path is not UTF-8")?.to_string();
    let mut app = Command::new(exe)
        .args([GUI_FLAG, kind, &mib.to_string(), &out_str])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot start {exe}: {e}"))?;
    let deadline = Instant::now() + RUN_DEADLINE;
    let read_report = || std::fs::read_to_string(&out).ok().map(|text| parse_report(&text));
    let result = loop {
        if let Some(report) = read_report() {
            break report;
        }
        match app.try_wait() {
            // The child reports before it exits, and the app may exit with it
            // (Linux closes the app with its last window): look once more.
            Ok(Some(status)) => {
                break read_report().unwrap_or_else(|| Err(format!("the app exited with {status} before the child reported")));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(REPORT_POLL),
            Ok(None) => break Err(format!("no report within {}s", RUN_DEADLINE.as_secs())),
            Err(e) => break Err(format!("waiting for the app failed: {e}")),
        }
    };
    // Close the app on every path; its pane child has already exited or is hung up.
    let _ = app.kill();
    let _ = app.wait();
    result
}

/// Parent side: one JSON line per stream kind; `Err` on the first failure.
pub fn run(mib: usize) -> Result<(), String> {
    let exe = crate::bench_input::current_exe()?;
    for kind in KINDS {
        let (bytes, ns) = run_kind(&exe, kind, mib).map_err(|e| format!("{kind}: {e}"))?;
        let data = generate(kind, mib * 1024 * 1024).ok_or("unknown stream kind")?;
        let in_app = mb_per_s(bytes, ns);
        let grid_only = grid_only_mb_per_s(&data);
        println!(
            "{{\"benchmark\":\"pty_throughput\",\"kind\":\"{kind}\",\"mib\":{:.1},\"seconds\":{:.3},\"in_app_mb_s\":{in_app:.1},\"grid_only_mb_s\":{grid_only:.1},\"in_app_share_of_grid\":{:.3}}}",
            bytes as f64 / (1024.0 * 1024.0),
            ns as f64 / 1e9,
            if grid_only > 0.0 { in_app / grid_only } else { 0.0 },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_reaches_the_requested_size() {
        for kind in KINDS {
            let data = generate(kind, 10_000).unwrap();
            assert!(data.len() >= 10_000, "{kind}: {}", data.len());
            assert!(data.len() < 10_000 + 512, "{kind} overshoots: {}", data.len());
            assert!(std::str::from_utf8(&data).is_ok(), "{kind} must be valid UTF-8");
        }
        assert!(generate("nope", 10).is_none());
    }

    #[test]
    fn streams_carry_their_kind_specific_sequences() {
        let text = |k| String::from_utf8(generate(k, 4096).unwrap()).unwrap();
        assert!(text("ascii").is_ascii() && !text("ascii").contains('\x1b'));
        assert!(text("sgr").contains("\x1b[1;32m"));
        assert!(text("unicode").contains("日本語"));
        assert!(text("osc8").contains("\x1b]8;;https://perf.test/"));
        let region = text("region");
        assert!(region.starts_with("\x1b[2;20r") && region.ends_with("\x1b[r"));
    }

    #[test]
    fn streams_end_every_line_with_crlf() {
        let data = generate("code", 4096).unwrap();
        let lf = data.iter().filter(|&&b| b == b'\n').count();
        let crlf = data.windows(2).filter(|w| w == b"\r\n").count();
        assert_eq!(lf, crlf);
    }

    #[test]
    fn report_parses_bytes_and_nanoseconds() {
        assert_eq!(parse_report("1048576 500000000\n"), Ok((1_048_576, 500_000_000)));
        assert!(parse_report("").is_err());
        assert!(parse_report("12").is_err());
        assert!(parse_report("12 0").is_err());
        assert!(parse_report("12 34 56").is_err());
        assert!((mb_per_s(1_048_576, 500_000_000) - 2.0).abs() < 1e-9);
        assert_eq!(mb_per_s(10, 0), 0.0);
    }

    #[test]
    fn report_file_is_written_whole_and_never_through_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("litty-tput-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let out = dir.join("r.txt");
        write_report(&out, 7, 9).unwrap();
        assert_eq!(parse_report(&std::fs::read_to_string(&out).unwrap()), Ok((7, 9)));
        std::fs::write(out.with_extension("tmp"), "planted").unwrap();
        assert!(write_report(&out, 1, 2).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grid_only_rate_is_positive() {
        assert!(grid_only_mb_per_s(&generate("ascii", 64 * 1024).unwrap()) > 0.0);
    }
}
