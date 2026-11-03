//! DSR protocol roundtrip over a real PTY, through the production reader.
//!
//! The parent spawns this binary as a child on a master/slave PTY with
//! `Pty::spawn_program`, and attaches `Pty::spawn_reader` with a real `Grid`.
//! The child puts the slave in raw mode, writes `ESC[5n` to its output and waits
//! for the exact `ESC[0n` on its input. Timing starts just before that write and
//! ends when the last reply byte has been read, both on the child's monotonic
//! clock. The measured path is therefore: PTY driver, reader thread, `vte`
//! parser, `Grid::response`, the writer queue, PTY driver again. It excludes the
//! shell, command execution and rendering.
//!
//! Warmup: the first [`WARMUP`] roundtrips are discarded (thread wake-ups, page
//! faults). Every wait is bounded by [`REPLY_DEADLINE`] / [`TOTAL_DEADLINE`], and
//! the PTY, child and threads are released on success, EOF, a malformed reply and
//! timeout (dropping the `Pty` hangs up the child; its reader reaps it).

use std::io::{self, Write as _};
use std::os::fd::RawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::terminal::grid::Grid;
use crate::terminal::pty::Pty;

pub const ITERATIONS: usize = 1000;
pub const WARMUP: usize = 100;
const REQUEST: &[u8] = b"\x1b[5n";
const REPLY: &[u8] = b"\x1b[0n";
const REPLY_DEADLINE: Duration = Duration::from_secs(2);
const TOTAL_DEADLINE: Duration = Duration::from_secs(30);

pub const CHILD_FLAG: &str = "+bench-dsr-child";

/// Latency summary in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    pub samples: usize,
    pub min: f64,
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// Nearest-rank percentiles, so P99 of 1,000 samples is the 990th value.
pub fn summarize(samples_ms: &[f64]) -> Option<Stats> {
    if samples_ms.is_empty() {
        return None;
    }
    let mut v = samples_ms.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    let rank = |p: f64| v[((p * n as f64).ceil() as usize).clamp(1, n) - 1];
    Some(Stats {
        samples: n,
        min: v[0],
        mean: v.iter().sum::<f64>() / n as f64,
        median: rank(0.5),
        p95: rank(0.95),
        p99: rank(0.99),
        max: v[n - 1],
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ReplyError {
    Timeout,
    Eof,
    Malformed,
    Io,
}

impl ReplyError {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Malformed => 2,
            Self::Timeout => 3,
            Self::Eof => 4,
            Self::Io => 5,
        }
    }
}

/// Read exactly `REPLY.len()` bytes from `fd` before `deadline` and compare them.
fn read_reply(fd: RawFd, deadline: Duration) -> Result<(), ReplyError> {
    let end = Instant::now() + deadline;
    let mut buf = [0u8; 4];
    let mut got = 0;
    while got < REPLY.len() {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(ReplyError::Timeout);
        }
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: `pfd` is a valid pollfd and the count is 1.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().clamp(1, i32::MAX as u128) as i32) };
        if ready < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(ReplyError::Io);
        }
        if ready == 0 {
            return Err(ReplyError::Timeout);
        }
        // SAFETY: the slice `buf[got..]` is writable for its length.
        let n = unsafe { libc::read(fd, buf[got..].as_mut_ptr().cast(), REPLY.len() - got) };
        match n {
            0 => return Err(ReplyError::Eof),
            n if n < 0 => {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ReplyError::Io);
            }
            n => got += n as usize,
        }
    }
    if buf == *REPLY { Ok(()) } else { Err(ReplyError::Malformed) }
}

fn write_all(fd: RawFd, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        // SAFETY: `data` is a valid readable slice.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// Child side. Returns the process exit code: 0 on success, otherwise the
/// [`ReplyError`] code (2 malformed, 3 timeout, 4 EOF, 5 I/O) or 6 for setup errors.
pub fn run_child(out: &Path, warmup: usize, iterations: usize) -> i32 {
    // SAFETY: `tio` is a plain C struct filled in by `tcgetattr`.
    unsafe {
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut tio) != 0 {
            return 6;
        }
        libc::cfmakeraw(&mut tio);
        tio.c_cc[libc::VMIN] = 1;
        tio.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(0, libc::TCSANOW, &tio) != 0 {
            return 6;
        }
    }
    let mut report = String::with_capacity(iterations * 8);
    for i in 0..warmup + iterations {
        let t0 = Instant::now();
        if !write_all(1, REQUEST) {
            return ReplyError::Io.exit_code();
        }
        if let Err(e) = read_reply(0, REPLY_DEADLINE) {
            return e.exit_code();
        }
        let ns = t0.elapsed().as_nanos();
        if i >= warmup {
            report.push_str(&ns.to_string());
            report.push('\n');
        }
    }
    // `create_new` + `O_NOFOLLOW`: never write through a pre-planted file or symlink.
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(out)
        .and_then(|mut f| f.write_all(report.as_bytes()));
    if written.is_ok() { 0 } else { 6 }
}

fn report_path() -> PathBuf {
    std::env::temp_dir().join(format!("litty-bench-dsr-{}.txt", std::process::id()))
}

fn parse_report(text: &str, expected: usize) -> Result<Vec<f64>, String> {
    let samples: Vec<f64> = text
        .lines()
        .map(|l| l.parse::<u64>().map(|ns| ns as f64 / 1e6))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("malformed child report: {e}"))?;
    if samples.len() == expected {
        Ok(samples)
    } else {
        Err(format!("child reported {} of {expected} samples", samples.len()))
    }
}

/// Parent side: run `iterations` timed roundtrips (after warmup) and return them in ms.
pub fn measure(exe: &Path, iterations: usize) -> Result<Vec<f64>, String> {
    let out = report_path();
    let _ = std::fs::remove_file(&out);
    let result = run_parent(exe, &out, iterations);
    let _ = std::fs::remove_file(&out);
    result
}

fn run_parent(exe: &Path, out: &Path, iterations: usize) -> Result<Vec<f64>, String> {
    let exe_str = exe.to_str().ok_or("executable path is not UTF-8")?;
    let out_str = out.to_str().ok_or("report path is not UTF-8")?;
    let (warmup, iters) = (WARMUP.to_string(), iterations.to_string());
    let pty = Pty::spawn_program(exe_str, &["litty-bench-dsr", CHILD_FLAG, out_str, &warmup, &iters], &[], 80, 24)
        .map_err(|e| format!("PTY spawn failed: {e}"))?;
    let grid = Arc::new(Mutex::new(Grid::new(80, 24)));
    let (done_tx, done_rx) = mpsc::channel();
    pty.spawn_reader(grid, || {}, move || {
        let _ = done_tx.send(());
    })
    .map_err(|e| format!("PTY reader failed: {e}"))?;
    let finished = done_rx.recv_timeout(TOTAL_DEADLINE);
    // Hang up the child and stop the I/O threads on every path, including timeout.
    drop(pty);
    if finished.is_err() {
        return Err(format!("child did not finish within {}s", TOTAL_DEADLINE.as_secs()));
    }
    let text = std::fs::read_to_string(out).map_err(|e| {
        format!("child produced no report ({e}): timeout, EOF or malformed reply on the PTY")
    })?;
    parse_report(&text, iterations)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipe() -> (RawFd, RawFd) {
        let mut fds = [0; 2];
        // SAFETY: `fds` has room for two descriptors.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    fn close(fd: RawFd) {
        // SAFETY: closing a descriptor opened by this test.
        unsafe { libc::close(fd) };
    }

    #[test]
    fn summarize_uses_nearest_rank() {
        let v: Vec<f64> = (1..=1000).map(f64::from).collect();
        let s = summarize(&v).unwrap();
        assert_eq!((s.min, s.median, s.p95, s.p99, s.max), (1.0, 500.0, 950.0, 990.0, 1000.0));
        assert_eq!(s.mean, 500.5);
        assert_eq!(summarize(&[7.0]).unwrap().p99, 7.0);
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn reply_exact_bytes_succeed_even_when_split() {
        let (r, w) = pipe();
        assert!(write_all(w, b"\x1b["));
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            assert!(write_all(w, b"0n"));
            w
        });
        assert_eq!(read_reply(r, Duration::from_secs(2)), Ok(()));
        close(writer.join().unwrap());
        close(r);
    }

    #[test]
    fn reply_with_wrong_bytes_is_malformed() {
        let (r, w) = pipe();
        assert!(write_all(w, b"\x1b[3n"));
        assert_eq!(read_reply(r, Duration::from_secs(2)), Err(ReplyError::Malformed));
        close(r);
        close(w);
    }

    #[test]
    fn missing_reply_times_out() {
        let (r, w) = pipe();
        assert_eq!(read_reply(r, Duration::from_millis(30)), Err(ReplyError::Timeout));
        close(r);
        close(w);
    }

    #[test]
    fn closed_input_reports_eof() {
        let (r, w) = pipe();
        close(w);
        assert_eq!(read_reply(r, Duration::from_secs(2)), Err(ReplyError::Eof));
        close(r);
    }

    #[test]
    fn report_must_hold_exactly_the_expected_samples() {
        assert_eq!(parse_report("1000000\n2000000\n", 2), Ok(vec![1.0, 2.0]));
        assert!(parse_report("1000000\n", 2).is_err());
        assert!(parse_report("abc\n", 1).is_err());
    }
}
