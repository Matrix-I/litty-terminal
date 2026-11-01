//! Run: rustc -O --edition=2024 scripts/bench-scroll.rs -o /tmp/litty-bench-scroll && /tmp/litty-bench-scroll
#[path = "../src/scroll.rs"]
mod scroll;

use std::{hint::black_box, time::Instant};
use scroll::{Accumulator, Target};

fn main() {
    let _ = Target::History;
    for (name, deltas) in [
        ("slow 2px", vec![2.0; 1200]),
        ("12px packets", vec![12.0; 200]),
        ("whole rows", vec![24.0; 100]),
        ("threshold jitter", [11.0, 13.0].repeat(100)),
        ("reverse slow", vec![-2.0; 1200]),
    ] {
        let expected = (deltas.iter().sum::<f64>() / 24.0).trunc() as isize;
        let before: isize = deltas.iter().map(|d| (d / 24.0).round() as isize).sum();
        let mut a = Accumulator::default();
        let after: isize = deltas.iter().map(|&d| a.push(d, 24.0, Target::Pane(1), true)).sum();
        assert_eq!(after, expected);
        println!("{name}: expected={expected} rows, before={before}, after={after}");
    }
    const EVENTS: usize = 5_000_000;
    for repeat in 1..=5 {
        let start = Instant::now();
        let mut before = 0isize;
        for i in 0..EVENTS { before += (black_box((i % 13 + 1) as f64) / 24.0).round() as isize; }
        black_box(before);
        let before_ns = start.elapsed().as_nanos() as f64 / EVENTS as f64;
        let mut a = Accumulator::default();
        let start = Instant::now();
        let mut after = 0isize;
        for i in 0..EVENTS { after += a.push(black_box((i % 13 + 1) as f64), 24.0, Target::Pane(1), true); }
        black_box(after);
        let after_ns = start.elapsed().as_nanos() as f64 / EVENTS as f64;
        println!("run {repeat}: before={before_ns:.2} ns/event, after={after_ns:.2} ns/event");
    }
}
