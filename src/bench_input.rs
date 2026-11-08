//! `+bench-input-latency`: keystroke → GPU-completed frame, inside the real app.
//!
//! The app's first pane runs this binary as an echo child ([`run_echo_child`])
//! instead of a shell, so prompt themes and line editors add no noise. The app
//! injects a synthetic key press into its own keyboard handler, and each
//! sample is split into stages on the main thread's monotonic clock:
//!
//! - `input`: the keyboard handler (keymap, write queued for the PTY writer);
//! - `echo`: PTY writer, child echo, reader thread, grid, wake-up, until the
//!   main thread sees the echoed byte in the grid right before a draw;
//! - `render`: that draw, from the grid check to the GPU submit;
//! - `gpu`: submit until the GPU reports the frame complete.
//!
//! Not measured: the OS input stack before the app sees an event, compositor
//! and display scanout (up to one more refresh interval).

use std::os::fd::RawFd;
use std::time::Duration;

use crate::bench_dsr::summarize;

pub const ECHO_CHILD_FLAG: &str = "+bench-echo-child";
pub const DEFAULT_SAMPLES: usize = 1000;
pub const WARMUP: usize = 50;
/// The echo child prints this at the top-left cell once its terminal is raw.
pub const READY_MARK: u8 = b'#';
/// Upper bound of the random pause between samples, so samples do not lock
/// onto the display refresh phase.
pub const MAX_GAP_MS: u64 = 16;
/// A sample that takes longer than this aborts the benchmark.
pub const SAMPLE_DEADLINE: Duration = Duration::from_secs(2);
/// Time allowed for the echo child to start and report ready.
pub const READY_DEADLINE: Duration = Duration::from_secs(10);

/// This executable's path as UTF-8, for re-running it as a benchmark child.
pub fn current_exe() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate the litty executable: {e}"))?;
    exe.into_os_string().into_string().map_err(|_| "the litty executable path is not UTF-8".to_string())
}

/// One keystroke, in milliseconds per stage.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sample {
    pub input_ms: f64,
    pub echo_ms: f64,
    pub render_ms: f64,
    pub gpu_ms: f64,
}

impl Sample {
    pub fn total_ms(&self) -> f64 {
        self.input_ms + self.echo_ms + self.render_ms + self.gpu_ms
    }
}

/// Window and display facts printed next to the numbers.
pub struct RunInfo {
    pub width_px: u32,
    pub height_px: u32,
    pub scale_factor: f64,
    pub monitor_millihz: u32,
}

/// Pseudo-random pause before the next key, `0..=MAX_GAP_MS` (xorshift; the
/// seed is fixed so runs are comparable).
pub fn next_gap(seed: &mut u64) -> Duration {
    let mut x = *seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    Duration::from_millis(x % (MAX_GAP_MS + 1))
}

fn stats_json(name: &str, values: &[f64]) -> String {
    let s = summarize(values).unwrap_or_default();
    format!(
        "\"{name}\":{{\"min\":{:.3},\"mean\":{:.3},\"p50\":{:.3},\"p95\":{:.3},\"p99\":{:.3},\"max\":{:.3}}}",
        s.min, s.mean, s.median, s.p95, s.p99, s.max
    )
}

/// One JSON line with every stage summarized (milliseconds).
pub fn report(samples: &[Sample], info: &RunInfo) -> String {
    let stage = |f: fn(&Sample) -> f64| samples.iter().map(f).collect::<Vec<_>>();
    format!(
        concat!(
            "{{\"benchmark\":\"input_latency\",\"endpoint\":\"gpu_complete\",\"samples\":{},\"warmup\":{},",
            "\"width_px\":{},\"height_px\":{},\"scale_factor\":{},\"monitor_millihz\":{},{},{},{},{},{}}}"
        ),
        samples.len(),
        WARMUP,
        info.width_px,
        info.height_px,
        info.scale_factor,
        info.monitor_millihz,
        stats_json("total_ms", &stage(Sample::total_ms)),
        stats_json("input_ms", &stage(|s| s.input_ms)),
        stats_json("echo_ms", &stage(|s| s.echo_ms)),
        stats_json("render_ms", &stage(|s| s.render_ms)),
        stats_json("gpu_ms", &stage(|s| s.gpu_ms)),
    )
}

fn write_all(fd: RawFd, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        // SAFETY: `data` is a valid readable slice.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// Put `fd` (a terminal) in raw mode: no kernel echo, byte-at-a-time reads.
pub(crate) fn make_raw(fd: RawFd) -> bool {
    // SAFETY: `tio` is a plain C struct filled in by `tcgetattr`.
    unsafe {
        let mut tio: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut tio) != 0 {
            return false;
        }
        libc::cfmakeraw(&mut tio);
        tio.c_cc[libc::VMIN] = 1;
        tio.c_cc[libc::VTIME] = 0;
        libc::tcsetattr(fd, libc::TCSANOW, &tio) == 0
    }
}

/// Echo each input byte from `input` to the top-left cell on `output` until
/// EOF or an error. Prints [`READY_MARK`] there first.
pub(crate) fn echo_loop(input: RawFd, output: RawFd) -> i32 {
    if !write_all(output, &[b'\x1b', b'[', b'2', b'J', b'\x1b', b'[', b'H', READY_MARK]) {
        return 5;
    }
    let mut frame = *b"\x1b[H ";
    loop {
        let mut byte = 0u8;
        // SAFETY: reading one byte into a valid local.
        let n = unsafe { libc::read(input, (&mut byte as *mut u8).cast(), 1) };
        if n == 0 {
            return 0;
        }
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // EIO: the terminal hung up when the app closed the pane.
            return 0;
        }
        frame[3] = byte;
        if !write_all(output, &frame) {
            return 0;
        }
    }
}

/// Child side, run on the pane's PTY. Returns the process exit code.
pub fn run_echo_child() -> i32 {
    if !make_raw(0) {
        return 6;
    }
    echo_loop(0, 1)
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

    fn read_some(fd: RawFd) -> Vec<u8> {
        let mut buf = [0u8; 64];
        // SAFETY: valid writable buffer.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        buf[..n.max(0) as usize].to_vec()
    }

    #[test]
    fn echo_loop_marks_ready_then_echoes_each_byte_home() {
        let (in_r, in_w) = pipe();
        let (out_r, out_w) = pipe();
        let child = std::thread::spawn(move || echo_loop(in_r, out_w));
        assert_eq!(read_some(out_r), b"\x1b[2J\x1b[H#");
        assert!(write_all(in_w, b"a"));
        assert_eq!(read_some(out_r), b"\x1b[Ha");
        assert!(write_all(in_w, b"s"));
        assert_eq!(read_some(out_r), b"\x1b[Hs");
        // SAFETY: closing descriptors this test opened.
        unsafe { libc::close(in_w) };
        assert_eq!(child.join().unwrap(), 0);
        unsafe {
            libc::close(in_r);
            libc::close(out_r);
            libc::close(out_w);
        }
    }

    #[test]
    fn total_is_the_sum_of_the_stages() {
        let s = Sample { input_ms: 0.1, echo_ms: 1.0, render_ms: 0.5, gpu_ms: 2.0 };
        assert!((s.total_ms() - 3.6).abs() < 1e-9);
    }

    #[test]
    fn gaps_stay_in_range_and_vary() {
        let mut seed = 0x9E37_79B9_7F4A_7C15;
        let gaps: Vec<u128> = (0..200).map(|_| next_gap(&mut seed).as_millis()).collect();
        assert!(gaps.iter().all(|&g| g <= MAX_GAP_MS as u128));
        assert!(gaps.iter().any(|&g| g != gaps[0]));
    }

    #[test]
    fn report_summarizes_each_stage() {
        let samples: Vec<Sample> = (1..=4)
            .map(|i| Sample { input_ms: 0.0, echo_ms: i as f64, render_ms: 1.0, gpu_ms: 0.0 })
            .collect();
        let info = RunInfo { width_px: 800, height_px: 600, scale_factor: 2.0, monitor_millihz: 120_000 };
        let json = report(&samples, &info);
        assert!(json.starts_with("{\"benchmark\":\"input_latency\""));
        assert!(json.contains("\"samples\":4"));
        assert!(json.contains("\"echo_ms\":{\"min\":1.000,\"mean\":2.500,\"p50\":2.000,\"p95\":4.000,\"p99\":4.000,\"max\":4.000}"));
        assert!(json.contains("\"total_ms\":{\"min\":2.000"));
        assert!(json.ends_with("}}"));
        assert!(crate::json::Json::parse(&json).is_ok(), "report must be valid JSON: {json}");
    }
}
