# Scroll input and scrollbar verification — 2026-11-01

Slow wheel events previously rounded each event to a whole line, discarding small
movement and exaggerating events near half a line. The fix accumulates movement
in input units, consumes whole lines and retains the signed remainder. It resets
on gesture start/cancellation, focus loss, target changes, input-kind changes and
cell-height changes. History search uses the same accumulation.

The scrollbar is a separate 12-byte GPU instance per pane, included in the existing
instanced draw. Its width is 10 physical pixels, with antialiased round caps of
radius 5 pixels. Outer panes extend to the window's right edge through its padding;
inner panes end at their divider. Drawing and dragging share thumb geometry.
The grid Cell layout and idle event-loop scheduling are unchanged.

## Reproducible input benchmark

Run from the repository root:

```sh
rustc -O --edition=2024 scripts/bench-scroll.rs -o /tmp/litty-bench-scroll
/tmp/litty-bench-scroll
```

Measured with a 24-pixel line height and default precise-scroll multiplier:

| Input | Expected rows | Before | After |
| --- | ---: | ---: | ---: |
| 1,200 events of +2 px | 100 | 0 | 100 |
| 200 events of +12 px | 100 | 200 | 100 |
| 100 events of +24 px | 100 | 100 | 100 |
| 100 pairs of +11/+13 px | 100 | 100 | 100 |
| 1,200 events of -2 px | -100 | 0 | -100 |

Five optimized runs of 5,000,000 events each measured 15.28–16.38 ns/event after
the change (median 15.68), compared with 0.70–1.36 ns/event for the old lossy
rounding. This measures only input conversion, not render time or input-to-display
latency. An allocation-counting regression test checks 10,000 accumulated events.

## Pixel scrolling follow-up

The initial accumulator-only revision passed: native `cargo check`, Linux cross-target `cargo check`,
`cargo clippy --all-targets -- -D warnings`, `cargo build`, and the full suite
with `cargo test -- --test-threads=1` (296 passed, 3 ignored). One parallel run
hit the existing clipboard-helper deadline test's “helper must have been spawned”
assertion; that test and the complete suite passed on the sequential rerun.

Precise wheel input now goes directly to a fractional viewport position. The grid
keeps an integer anchor plus a fractional row; the shader translates cell quads
and clips them to each pane. One extra row supplies newly exposed bottom pixels.
Images and text selection/link hit-testing use the same shift. Mouse-reporting
applications and alternate-screen arrow scrolling retain discrete events. Opening
a cell-based search/inspector overlay snaps the fractional position once.

A sequence of 24 one-pixel events at 24 pixels/row now produces 24 viewport updates
instead of one whole-row jump. It rebuilds glyph instances only once across those
24 updates; intermediate updates reuse instances and change per-pane GPU metadata.
Allocation-counting tests verify zero allocations for 10,000 scroll/reverse pairs.
Grid Cells and GPU instances both remain 12 bytes, and no idle timer is added.

The explicit `precise_scroll_gpu_pixels` test renders with Metal into an offscreen
texture and reads back pixels. It verifies the additional bottom row, top/bottom
clipping, an independently stationary neighboring pane, the 10-pixel scrollbar at
the right edge, round caps, and both normal/flipped custom-shader coordinates.

```sh
cargo test precise_scroll
LITTY_SCROLL_TEST_IMAGE=/tmp/litty-scroll-gpu.png cargo test precise_scroll_gpu_pixels -- --ignored --nocapture
```

At this point only offscreen pixels had been checked; the subsequent live-window
measurement below adds swapchain submission timing. Physical trackpad input and
compositor/display presentation timestamps remain unmeasured.

The pixel-scrolling revision passed native and Linux cross-target checks, Clippy
with zero warnings, and the full sequential suite (299 passed, 4 ignored). The
GPU test is one of the normally ignored tests and passed when run explicitly.

## Live-window FPS measurement

The optimized binary now accepts `+bench-scroll-fps`. It opens a dedicated window,
loads 5,000 ASCII lines and applies synthetic precise-scroll deltas to the same
Grid/Renderer path used by local scrolling. Each of three scenarios lasts five
seconds, with the first second excluded. It automatically closes its own window.
Normal runs do not enable instrumentation or add benchmark timers.

```sh
cargo build --profile fast-release
./target/fast-release/litty +bench-scroll-fps
```

The measurement records timestamps immediately after actual swapchain `present()`
calls. It includes the real window's FIFO backpressure, grid locking, instance
updates and rendering/submission path. These are **application submission FPS**,
not hardware display presentation timestamps or an input-to-photon measurement.
The input is generated inside the application; OS trackpad delivery is excluded.

Environment: 2590 × 1492 physical pixels, 2× display scaling, monitor reporting
120 Hz, no custom shaders, one pane, optimized `fast-release` build.

First run:

| Input | Submission FPS | Frame interval p95 | Frame interval p99 | Intervals >25 ms |
| --- | ---: | ---: | ---: | ---: |
| Slow: 2 px at 120 Hz | 119.997 | 8.628 ms | 8.706 ms | 0 |
| Fast: 12 px at 120 Hz | 119.999 | 8.728 ms | 8.889 ms | 0 |
| Slow: 2 px at 60 Hz | 60.001 | 17.445 ms | 17.686 ms | 0 |

At 120 Hz, acquisition p95 was 8.380/8.196 ms and total render/submission p95
was 8.590/8.600 ms. This is consistent with FIFO pacing dominating the sampled
render duration; it is not a GPU execution-time measurement.

Raw JSON from the first run is saved in `scroll-fps-2026-11-01.jsonl`. A second
run was interrupted: it recorded a 1013.9 ms gap, then zero frames in the final
two scenarios (`scroll-fps-interrupted-2026-11-01.jsonl`). That run cannot establish
normal visible-window performance. The capture now marks any scenario invalid
when the window is reported occluded or fewer than three samples are available.
The benchmark brings its dedicated window forward at startup, without changing
normal application behavior. Capture/scheduler tests cover occlusion invalidation.
The full sequential suite passed with 302 tests and 4 ignored; the capture tests,
Clippy and Linux cross-check also pass after adding validity checks.

Validation run (all scenarios valid, zero occlusion events and zero dropped samples):

| Input | Submission FPS | Interval p95 | Interval p99 | Maximum | Late intervals |
| --- | ---: | ---: | ---: | ---: | ---: |
| Slow: 2 px at 120 Hz | 119.255 | 10.558 ms | 13.025 ms | 18.102 ms | 7 |
| Fast: 12 px at 120 Hz | 118.742 | 11.183 ms | 13.197 ms | 17.789 ms | 7 |
| Slow: 2 px at 60 Hz | 60.003 | 18.218 ms | 18.779 ms | 19.295 ms | 0 |

Raw data: `scroll-fps-validation-2026-11-01.jsonl`. Late means an interval over
12.5 ms at 120 Hz or over 25 ms at 60 Hz. The 120 Hz samples show occasional
uneven pacing despite a high average frame rate, which may contribute to perceived
stutter. This benchmark does not identify whether the OS input delivery path adds
further jitter, nor isolate compositor scheduling from application timing.
