//! Entry point: `winit` event loop in `ControlFlow::Wait` mode (0% idle CPU),
//! coalesced PTY redraw wakeups, and keyboard-to-ANSI translation.

mod ai;
mod app;
mod atlas;
pub mod bench;
mod cli;
mod config;
mod event;
mod fontdb;
mod frame_metrics;
mod hotkey;
mod image_layer;
mod keymap;
mod macos;
mod mouse;
mod paste;
mod platform;
mod postfx;
mod renderer;
mod scroll;
#[cfg(test)]
mod security_poc;
mod split;
mod tab;
mod terminal;
mod update;

pub use event::{AppEvent, Waker};
pub use tab::{HoveredPaneLink, next_id, Pane, Tab};

#[cfg(all(target_os = "macos", not(test)))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(test)]
mod test_alloc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match cli::parse(&args) {
        cli::Command::ListThemes => {
            for name in config::bundled_theme_names() {
                println!("{name}");
            }
        }
        cli::Command::ToggleQuickTerminal => {
            if cfg!(target_os = "macos") {
                eprintln!("litty: on macOS, use `keybind = global:<trigger>=toggle_quick_terminal`");
                std::process::exit(1);
            }
            if let Err(e) = hotkey::send_toggle() {
                eprintln!("litty: no running Litty to toggle ({e})");
                std::process::exit(1);
            }
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
