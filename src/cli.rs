//! Command-line dispatch for `litty +<command>` / `--<command>`.

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    ListThemes,
    ToggleQuickTerminal,
    CheckUpdate,
    Update { force: bool },
    BenchPerf,
    BenchStress { lines: usize },
    BenchFeatures,
    Run { bench_ram: bool, bench_all: bool, scroll_fps: bool },
}

pub(crate) fn parse(args: &[String]) -> Command {
    let first = args.get(1).map(String::as_str).unwrap_or("");
    let is = |names: &[&str]| names.contains(&first);
    if is(&["+list-themes", "--list-themes"]) {
        Command::ListThemes
    } else if is(&["+toggle-quick-terminal"]) {
        Command::ToggleQuickTerminal
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
    fn toggle_quick_terminal_has_only_plus_spelling() {
        assert_eq!(parse(&argv(&["+toggle-quick-terminal"])), Command::ToggleQuickTerminal);
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
    fn bench_all_implies_bench_ram() {
        assert_eq!(parse(&argv(&["+bench-all"])), Command::Run { bench_ram: true, bench_all: true, scroll_fps: false });
        assert_eq!(parse(&argv(&["--bench-ram"])), Command::Run { bench_ram: true, bench_all: false, scroll_fps: false });
        assert_eq!(parse(&argv(&["+bench-scroll-fps"])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: true });
    }
}
