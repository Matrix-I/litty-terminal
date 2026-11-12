//! Release-mode throughput probe for the grid hot path: `feed_byte` → vte →
//! `print` / `put_char` / `linefeed` / `scroll_up_in`, plus SGR, grapheme
//! clusters, scroll regions, the alternate screen and OSC 8 links.
//!
//! Not a pass/fail test. It prints `GRID_PROBE <case> <ns-per-byte>` (best of
//! `RUNS`) for before/after comparisons of structural refactors:
//! `cargo test --release grid::perf_probe -- --ignored --nocapture --test-threads=1`

use super::Grid;
use std::fmt::Write as _;
use std::time::Instant;

const RUNS: usize = 7;

/// A named input stream, built once outside the timed loop.
type Case = (&'static str, fn() -> String);

fn best_ns_per_byte(data: &[u8]) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..RUNS {
        let mut grid = Grid::new(160, 50);
        let mut parser = vte::Parser::new();
        let start = Instant::now();
        let mut off = 0;
        while off < data.len() {
            off += grid.feed_bytes(&mut parser, &data[off..]);
        }
        let ns = start.elapsed().as_nanos() as f64 / data.len() as f64;
        // Keep every cell write observable so the optimizer cannot drop the work.
        std::hint::black_box(&grid);
        best = best.min(ns);
    }
    best
}

fn ascii_scroll() -> String {
    let mut s = String::new();
    for i in 0..50_000 {
        let _ = write!(s, "{i:08} the quick brown fox jumps over the lazy dog 0123456789 abcdefghijklmnopqrstuvwxyz\r\n");
    }
    s
}

fn sgr_log() -> String {
    let levels = ["\x1b[32mINFO\x1b[0m", "\x1b[33mWARN\x1b[0m", "\x1b[1;31mERROR\x1b[0m", "\x1b[38;5;244mDEBUG\x1b[0m"];
    let mut s = String::new();
    for i in 0..30_000 {
        let _ = write!(
            s,
            "2026-11-01T12:{:02}:{:02}Z {} \x1b[38;2;137;220;235mapi\x1b[0m request_id={i} status=200 latency={}ms\r\n",
            (i / 60) % 60,
            i % 60,
            levels[i % 4],
            i % 997
        );
    }
    s
}

fn unicode_mixed() -> String {
    let mut s = String::new();
    for i in 0..20_000 {
        let _ = write!(s, "{i} Tiếng Việt có dấu ─┼─ 漢字かな交じり文 cafe\u{301} naïve\r\n");
    }
    s
}

fn emoji_clusters() -> String {
    let mut s = String::new();
    for i in 0..10_000 {
        let _ = write!(s, "{i} 👨\u{200D}👩\u{200D}👧 🇻🇳 ❤\u{FE0F} 👍\u{1F3FD} ok\r\n");
    }
    s
}

fn region_scroll() -> String {
    let mut s = String::from("\x1b[5;45r\x1b[45;1H");
    for i in 0..5_000 {
        let _ = write!(s, "region line {i} lorem ipsum dolor sit amet\r\n");
    }
    s.push_str("\x1b[r");
    s
}

fn alt_redraw() -> String {
    let bar = "=".repeat(120);
    let mut s = String::from("\x1b[?1049h");
    for frame in 0..200 {
        for row in 0..50 {
            let _ = write!(s, "\x1b[{};1H\x1b[7m{frame:4}\x1b[27m row {row:2} {bar}\x1b[K", row + 1);
        }
    }
    s.push_str("\x1b[?1049l");
    s
}

/// `less` / `tail -f` on the alternate screen: every line feed scrolls the
/// whole screen with no scrollback.
fn alt_scroll() -> String {
    let mut s = String::from("\x1b[?1049h\x1b[50;1H");
    for i in 0..50_000 {
        let _ = write!(s, "{i:08} the quick brown fox jumps over the lazy dog 0123456789\r\n");
    }
    s.push_str("\x1b[?1049l");
    s
}

fn osc8_links() -> String {
    let mut s = String::new();
    for i in 0..20_000 {
        let _ = write!(s, "\x1b]8;;https://perf.test/{}\x1b\\link {i}\x1b]8;;\x1b\\\r\n", i % 400);
    }
    s
}

/// `ls --hyperlink` in a big directory: distinct `file://` links sharing a
/// long prefix, so interning cannot tell them apart by the first bytes.
fn osc8_ls() -> String {
    let mut s = String::new();
    for i in 0..20_000 {
        let n = i % 400;
        let _ = write!(s, "\x1b]8;;file://build-host/home/user/projects/litty/target/debug/deps/file_{n:05}.rlib\x1b\\file_{n:05}.rlib\x1b]8;;\x1b\\\r\n");
    }
    s
}

#[test]
#[ignore = "release-mode performance probe; run explicitly for refactor gates"]
fn grid_throughput_probe() {
    let cases: [Case; 9] = [
        ("ascii_scroll", ascii_scroll),
        ("sgr_log", sgr_log),
        ("unicode_mixed", unicode_mixed),
        ("emoji_clusters", emoji_clusters),
        ("region_scroll", region_scroll),
        ("alt_redraw", alt_redraw),
        ("alt_scroll", alt_scroll),
        ("osc8_links", osc8_links),
        ("osc8_ls", osc8_ls),
    ];
    println!("GRID_SIZE {}", std::mem::size_of::<Grid>());
    for (name, build) in cases {
        let data = build();
        println!("GRID_PROBE {name} {:.4}", best_ns_per_byte(data.as_bytes()));
    }
}

/// Steady state of a live terminal: the ring is at full capacity, so printing
/// and scrolling plain, colored and wide text must not touch the allocator.
#[test]
fn steady_state_print_and_scroll_allocate_nothing() {
    let mut grid = Grid::new(80, 24);
    let mut parser = vte::Parser::new();
    let line = "\x1b[1;32mok\x1b[0m plain ascii \x1b[38;5;208mcolor\x1b[0m 漢字 Việt ─┼─\r\n";
    // Fill scrollback past its limit so the ring stops growing.
    for _ in 0..6_000 {
        for &b in line.as_bytes() {
            grid.feed_byte(&mut parser, b);
        }
    }
    let allocations = crate::test_alloc::allocations(|| {
        for _ in 0..1_000 {
            for &b in line.as_bytes() {
                grid.feed_byte(&mut parser, b);
            }
        }
    });
    assert_eq!(allocations, 0, "steady-state print / scroll must not allocate");
}

/// Prints `RESIZE_PROBE <case> <ms>` (best of `RUNS`) for one resize step of a
/// 160-column grid with a full default scrollback:
/// `cargo test --release grid::perf_probe::resize_probe -- --ignored --nocapture`
#[test]
#[ignore = "release-mode performance probe; run explicitly for refactor gates"]
fn resize_probe() {
    let fill = ascii_scroll();
    let cases: [(&str, usize, usize); 4] = [("rows_shrink", 160, 49), ("rows_grow", 160, 60), ("cols_160_to_159", 159, 50), ("cols_160_to_80", 80, 50)];
    for (name, cols, rows) in cases {
        let mut best = f64::MAX;
        for _ in 0..RUNS {
            let mut grid = Grid::new(160, 50);
            let mut parser = vte::Parser::new();
            let mut off = 0;
            while off < fill.len() {
                off += grid.feed_bytes(&mut parser, &fill.as_bytes()[off..]);
            }
            let start = Instant::now();
            grid.resize(cols, rows);
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(&grid);
        }
        println!("RESIZE_PROBE {name} {best:.4}");
    }
}
