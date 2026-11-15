// This binary-only crate has no library facade. Compile its modules into a
// separate test executable so AppKit runs on main, without a production CLI flag.
#![allow(dead_code, unused_imports)]
// harness=false compiles existing unit-test modules as a normal executable;
// their source ordering is checked by the normal crate's Clippy target.
#![allow(clippy::items_after_test_module)]

// release_free_memory calls mi_collect even with the test counting allocator.
#[cfg(target_os = "macos")]
use mimalloc as _;

#[path = "../src/ai/mod.rs"] mod ai;
#[path = "../src/app/mod.rs"] mod app;
#[path = "../src/bench.rs"] pub mod bench;
#[path = "../src/bench_dsr.rs"] mod bench_dsr;
#[path = "../src/bench_input.rs"] mod bench_input;
#[path = "../src/bench_startup.rs"] mod bench_startup;
#[path = "../src/bench_throughput.rs"] mod bench_throughput;
#[path = "../src/cli.rs"] mod cli;
#[path = "../src/config.rs"] mod config;
#[path = "../src/curl_config.rs"] mod curl_config;
#[path = "../src/event.rs"] mod event;
#[path = "../src/json.rs"] mod json;
#[path = "../src/keymap.rs"] mod keymap;
#[path = "../src/mouse.rs"] mod mouse;
#[path = "../src/paste.rs"] mod paste;
#[path = "../src/platform/mod.rs"] mod platform;
#[path = "../src/render/mod.rs"] mod render;
#[path = "../src/scroll.rs"] mod scroll;
#[path = "../src/split.rs"] mod split;
#[path = "../src/tab.rs"] mod tab;
#[path = "../src/terminal/mod.rs"] mod terminal;
#[path = "../src/test_alloc.rs"] mod test_alloc;
#[path = "../src/update/mod.rs"] mod update;

pub(crate) use platform::input;
pub use event::{AppEvent, Waker};
pub use tab::{HoveredPaneLink, Pane, Tab, next_id};

fn main() {
    #[cfg(target_os = "macos")]
    app::run_native_activation_test();
    #[cfg(target_os = "linux")]
    println!("NOT RUN: macOS AppKit activation lifecycle is not available on Linux");
}
