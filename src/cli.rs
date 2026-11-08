//! Command-line dispatch for `litty +<command>` / `--<command>`.

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    ListThemes,
    /// `+toggle-quick-terminal` from before the Quick Terminal was removed.
    QuickTerminalRemoved,
    CheckUpdate,
    Update { force: bool },
    BenchPerf,
    BenchStress { lines: usize },
    BenchFeatures,
    BenchDsr { iterations: usize },
    /// Internal: the child process of `BenchDsr`, which runs on a PTY.
    BenchDsrChild { out: String, warmup: usize, iterations: usize },
    /// Keystroke → GPU-completed frame in a real window.
    BenchInputLatency { samples: usize },
    /// Internal: the echo program `BenchInputLatency` runs in its pane.
    BenchEchoChild,
    /// Process launch → first GPU-completed frame, over several launches.
    BenchFirstFrame { runs: usize },
    /// Internal: one launch of `BenchFirstFrame`; `spawn_ns` is the parent's clock at spawn.
    BenchFirstFrameChild { spawn_ns: u64 },
    /// Bytes per second through PTY, parser and drawing, per stream kind.
    BenchPtyThroughput { mib: usize },
    /// Internal: the app `BenchPtyThroughput` runs for one stream kind.
    BenchPtyThroughputGui { kind: String, mib: usize, out: String },
    /// Internal: the stream writer that app runs in its pane.
    BenchCatChild { kind: String, mib: usize, out: String },
    Run { bench_ram: bool, bench_all: bool, scroll_fps: bool },
}

pub(crate) fn parse(args: &[String]) -> Command {
    let first = args.get(1).map(String::as_str).unwrap_or("");
    let is = |names: &[&str]| names.contains(&first);
    if is(&["+list-themes", "--list-themes"]) {
        Command::ListThemes
    } else if is(&["+toggle-quick-terminal"]) {
        Command::QuickTerminalRemoved
    } else if is(&["+check-update", "--check-update"]) {
        Command::CheckUpdate
    } else if is(&["+update", "--update"]) {
        Command::Update { force: args.iter().any(|a| a == "--force" || a == "-f") }
    } else if is(&["+bench-perf", "--bench-perf"]) {
        Command::BenchPerf
    } else if is(&["+bench-stress", "--bench-stress"]) {
        Command::BenchStress { lines: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20_000) }
    } else if is(&["+bench-features", "--bench-features"]) {
        Command::BenchFeatures
    } else if is(&["+bench-dsr", "--bench-dsr"]) {
        Command::BenchDsr {
            iterations: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(crate::bench_dsr::ITERATIONS),
        }
    } else if is(&["+bench-input-latency", "--bench-input-latency"]) {
        Command::BenchInputLatency {
            samples: args.get(2).and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(crate::bench_input::DEFAULT_SAMPLES),
        }
    } else if is(&[crate::bench_input::ECHO_CHILD_FLAG]) {
        Command::BenchEchoChild
    } else if is(&["+bench-first-frame", "--bench-first-frame"]) {
        Command::BenchFirstFrame {
            runs: args.get(2).and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(crate::bench_startup::DEFAULT_RUNS),
        }
    } else if is(&["+bench-pty-throughput", "--bench-pty-throughput"]) {
        Command::BenchPtyThroughput {
            mib: args.get(2).and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or(crate::bench_throughput::DEFAULT_MIB),
        }
    } else if is(&[crate::bench_throughput::GUI_FLAG, crate::bench_throughput::CAT_CHILD_FLAG]) {
        let kind = args.get(2).cloned().unwrap_or_default();
        let mib = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(crate::bench_throughput::DEFAULT_MIB);
        let out = args.get(4).cloned().unwrap_or_default();
        if first == crate::bench_throughput::GUI_FLAG {
            Command::BenchPtyThroughputGui { kind, mib, out }
        } else {
            Command::BenchCatChild { kind, mib, out }
        }
    } else if is(&[crate::bench_startup::CHILD_FLAG]) {
        Command::BenchFirstFrameChild { spawn_ns: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0) }
    } else if is(&[crate::bench_dsr::CHILD_FLAG]) {
        let num = |i: usize| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
        Command::BenchDsrChild { out: args.get(2).cloned().unwrap_or_default(), warmup: num(3), iterations: num(4) }
    } else {
        let bench_all = is(&["+bench-all", "--bench-all"]);
        Command::Run {
            bench_ram: bench_all || is(&["+bench-ram", "--bench-ram"]),
            bench_all,
            scroll_fps: is(&["+bench-scroll-fps", "--bench-scroll-fps"]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(a: &[&str]) -> Vec<String> {
        std::iter::once("litty").chain(a.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn no_args_runs_terminal() {
        assert_eq!(parse(&argv(&[])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: false });
    }

    #[test]
    fn plus_and_dash_spellings_are_equivalent() {
        for (plus, dash, want) in [
            ("+list-themes", "--list-themes", Command::ListThemes),
            ("+check-update", "--check-update", Command::CheckUpdate),
            ("+bench-perf", "--bench-perf", Command::BenchPerf),
            ("+bench-features", "--bench-features", Command::BenchFeatures),
        ] {
            assert_eq!(parse(&argv(&[plus])), want);
            assert_eq!(parse(&argv(&[dash])), want);
        }
    }

    #[test]
    fn toggle_quick_terminal_says_it_was_removed() {
        // A desktop shortcut still running it must not start a new instance.
        assert_eq!(parse(&argv(&["+toggle-quick-terminal"])), Command::QuickTerminalRemoved);
        assert_eq!(parse(&argv(&["--toggle-quick-terminal"])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: false });
    }

    #[test]
    fn update_force_flag_anywhere() {
        assert_eq!(parse(&argv(&["+update"])), Command::Update { force: false });
        assert_eq!(parse(&argv(&["--update", "-f"])), Command::Update { force: true });
        assert_eq!(parse(&argv(&["+update", "--force"])), Command::Update { force: true });
    }

    #[test]
    fn bench_stress_line_count_defaults_and_parses() {
        assert_eq!(parse(&argv(&["+bench-stress"])), Command::BenchStress { lines: 20_000 });
        assert_eq!(parse(&argv(&["--bench-stress", "500"])), Command::BenchStress { lines: 500 });
        assert_eq!(parse(&argv(&["--bench-stress", "abc"])), Command::BenchStress { lines: 20_000 });
    }

    #[test]
    fn bench_dsr_parses_iterations_and_child_arguments() {
        assert_eq!(parse(&argv(&["+bench-dsr"])), Command::BenchDsr { iterations: 1000 });
        assert_eq!(parse(&argv(&["--bench-dsr", "50"])), Command::BenchDsr { iterations: 50 });
        assert_eq!(
            parse(&argv(&["+bench-dsr-child", "/tmp/x", "10", "20"])),
            Command::BenchDsrChild { out: "/tmp/x".into(), warmup: 10, iterations: 20 }
        );
    }

    #[test]
    fn bench_input_latency_parses_sample_count_and_child() {
        assert_eq!(parse(&argv(&["+bench-input-latency"])), Command::BenchInputLatency { samples: 1000 });
        assert_eq!(parse(&argv(&["--bench-input-latency", "200"])), Command::BenchInputLatency { samples: 200 });
        assert_eq!(parse(&argv(&["+bench-input-latency", "0"])), Command::BenchInputLatency { samples: 1000 });
        assert_eq!(parse(&argv(&["+bench-echo-child"])), Command::BenchEchoChild);
    }

    #[test]
    fn bench_first_frame_parses_runs_and_child_clock() {
        assert_eq!(parse(&argv(&["+bench-first-frame"])), Command::BenchFirstFrame { runs: 10 });
        assert_eq!(parse(&argv(&["--bench-first-frame", "3"])), Command::BenchFirstFrame { runs: 3 });
        assert_eq!(parse(&argv(&["+bench-first-frame", "0"])), Command::BenchFirstFrame { runs: 10 });
        assert_eq!(parse(&argv(&["+bench-first-frame-child", "123"])), Command::BenchFirstFrameChild { spawn_ns: 123 });
        assert_eq!(parse(&argv(&["+bench-first-frame-child"])), Command::BenchFirstFrameChild { spawn_ns: 0 });
    }

    #[test]
    fn bench_pty_throughput_parses_size_and_internal_arguments() {
        assert_eq!(parse(&argv(&["+bench-pty-throughput"])), Command::BenchPtyThroughput { mib: 64 });
        assert_eq!(parse(&argv(&["--bench-pty-throughput", "8"])), Command::BenchPtyThroughput { mib: 8 });
        assert_eq!(
            parse(&argv(&["+bench-pty-throughput-gui", "sgr", "8", "/tmp/r"])),
            Command::BenchPtyThroughputGui { kind: "sgr".into(), mib: 8, out: "/tmp/r".into() }
        );
        assert_eq!(
            parse(&argv(&["+bench-cat-child", "code", "4", "/tmp/r"])),
            Command::BenchCatChild { kind: "code".into(), mib: 4, out: "/tmp/r".into() }
        );
    }

    #[test]
    fn bench_all_implies_bench_ram() {
        assert_eq!(parse(&argv(&["+bench-all"])), Command::Run { bench_ram: true, bench_all: true, scroll_fps: false });
        assert_eq!(parse(&argv(&["--bench-ram"])), Command::Run { bench_ram: true, bench_all: false, scroll_fps: false });
        assert_eq!(parse(&argv(&["+bench-scroll-fps"])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: true });
    }
}
