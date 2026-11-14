//! Entry point: `winit` event loop in `ControlFlow::Wait` mode (0% idle CPU),
//! coalesced PTY redraw wakeups, and keyboard-to-ANSI translation.

mod ai;
mod app;
pub mod bench;
mod bench_dsr;
mod bench_input;
mod bench_startup;
mod bench_throughput;
mod cli;
mod config;
mod curl_config;
mod event;
mod json;
mod keymap;
mod mouse;
mod paste;
mod platform;
mod render;
mod scroll;
#[cfg(test)]
mod security_poc;
#[cfg(test)]
mod layering_tests;
mod split;
mod tab;
mod terminal;
mod update;

pub(crate) use platform::input;
pub use event::{AppEvent, Waker};
pub use tab::{HoveredPaneLink, next_id, Pane, Tab};

#[cfg(all(target_os = "macos", not(test)))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(test)]
mod test_alloc;

fn main() {
    platform::capture_startup_activation();
    bench_startup::mark_main();
    let args: Vec<String> = std::env::args().collect();
    match cli::parse(&args) {
        cli::Command::ListThemes => {
            for name in config::bundled_theme_names() {
                println!("{name}");
            }
        }
        cli::Command::QuickTerminalRemoved => {
            eprintln!("litty: the Quick Terminal was removed in 0.10.0");
            std::process::exit(1);
        }
        cli::Command::CheckUpdate => update::cli_check_update(),
        cli::Command::Update { force } => update::cli_update(force),
        cli::Command::BenchPerf => {
            let lat = bench::run_latency_benchmark();
            let stress = bench::run_render_stress_benchmark(20_000);
            let feat = bench::run_critical_features_benchmark();
            if !lat.passed || !stress.passed || !feat.passed {
                std::process::exit(1);
            }
        }
        cli::Command::BenchStress { lines } => {
            if !bench::run_render_stress_benchmark(lines).passed {
                std::process::exit(1);
            }
        }
        cli::Command::BenchDsr { iterations } => {
            if !bench::run_dsr_benchmark(iterations).passed {
                std::process::exit(1);
            }
        }
        cli::Command::BenchDsrChild { out, warmup, iterations } => {
            std::process::exit(bench_dsr::run_child(std::path::Path::new(&out), warmup, iterations));
        }
        cli::Command::BenchInputLatency { samples } => match bench_input::current_exe() {
            Ok(exe) => app::run_input_latency(samples, vec![exe, bench_input::ECHO_CHILD_FLAG.into()]),
            Err(e) => {
                eprintln!("litty: {e}");
                std::process::exit(1);
            }
        },
        cli::Command::BenchEchoChild => std::process::exit(bench_input::run_echo_child()),
        cli::Command::BenchFirstFrame { runs } => {
            if let Err(e) = bench_startup::run(runs) {
                eprintln!("litty: first-frame benchmark failed: {e}");
                std::process::exit(1);
            }
        }
        cli::Command::BenchFirstFrameChild { spawn_ns } => app::run_first_frame_child(spawn_ns),
        cli::Command::BenchPtyThroughput { mib } => {
            if let Err(e) = bench_throughput::run(mib) {
                eprintln!("litty: PTY throughput benchmark failed: {e}");
                std::process::exit(1);
            }
        }
        cli::Command::BenchPtyThroughputGui { kind, mib, out } => match bench_input::current_exe() {
            Ok(exe) => app::run_throughput_gui(vec![exe, bench_throughput::CAT_CHILD_FLAG.into(), kind, mib.to_string(), out]),
            Err(e) => {
                eprintln!("litty: {e}");
                std::process::exit(1);
            }
        },
        cli::Command::BenchCatChild { kind, mib, out } => {
            std::process::exit(bench_throughput::run_cat_child(&kind, mib, std::path::Path::new(&out)))
        }
        cli::Command::BenchFeatures => {
            if !bench::run_critical_features_benchmark().passed {
                std::process::exit(1);
            }
        }
        cli::Command::Run { bench_ram, bench_all, scroll_fps } => {
            if bench_all {
                let lat = bench::run_latency_benchmark();
                let stress = bench::run_render_stress_benchmark(20_000);
                let feat = bench::run_critical_features_benchmark();
                if !lat.passed || !stress.passed || !feat.passed {
                    eprintln!("litty: performance/features benchmark failed");
                    std::process::exit(1);
                }
            }
            app::run(bench_ram, scroll_fps);
        }
    }
}
