# Performance gaps — measurement and fix plan — 2026-11-08

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the gaps between Litty's measured performance and its stated
targets (`docs/plans/0.10.0.md` §1, `CLAUDE.md` invariants), starting with
the benchmarks that are still missing so every later fix has a before/after
number.

**Scope:** Every issue found in the 2026-11-08 architecture review and
measurement session. Each item says whether its cause is **Confirmed** (code
path read end to end and backed by a measurement) or a **Hypothesis**
(mechanism is real, impact not isolated yet). Hypotheses start with an
isolation step and are dropped if that step shows no effect.

**Global constraints** (from `CLAUDE.md`, apply to every task):

- One task = one branch (`feat/`, `fix/`, `perf/`, `refactor/`) and one logical commit per change.
- `Cell` stays 12 bytes; no heap allocation added on PTY parsing, scrolling or the render loop.
- Never perform blocking I/O while holding the grid `Mutex`.
- Platform code lives in `src/platform/{macos,linux}/` with identical `pub fn` names.
- Every optimization ships with before/after numbers from the same benchmark, same machine, release profile (`lto = "fat"`), never `fast-release`.
- Verification before every commit: `cargo check`, `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `git diff --check`.

## Baseline (2026-11-08)

Environment: Apple M1 Pro, macOS 27.2, commit `b636d9d`, `cargo build --release`
(binary 2,118,624 bytes). Inputs were 64 MiB generated files (plain ASCII
words, SGR-colored log lines, Rust-like code with dense punctuation,
Vietnamese/CJK/emoji text).

### Throughput

| Input | Grid only (`grid::perf_probe`) | In app (`cat` through PTY → grid → render) | Share of raw PTY (~220 MB/s) |
|---|---|---|---|
| ASCII | 1.06 ns/B ≈ 950 MB/s | 137 MB/s | 62% |
| SGR log | 2.32 ns/B ≈ 430 MB/s | 103 MB/s | 47% |
| Code (punctuation) | not measured | 64 MB/s | 29% |
| Unicode mixed | 13.3 ns/B ≈ 75 MB/s | 50 MB/s | 23% |
| Emoji clusters | 17.7 ns/B ≈ 56 MB/s | not measured | — |
| Region scroll (`DECSTBM`) | 28.0 ns/B ≈ 36 MB/s | not measured | — |
| Alt-screen redraw | 0.47 ns/B | not measured | — |
| OSC 8 links | **299 ns/B ≈ 3.3 MB/s** | not measured | — |

Raw PTY ceiling: a Python reader on `pty.fork()` + `cat` read 215–228 MB/s for
both ASCII and code files, so the kernel path does not distinguish them.

Headless benchmarks all pass their existing SLAs: `+bench-stress 200000`
97 MB/s, worst stall 0.57 ms; `+bench-perf` DSR mean 0.017 ms / P99 0.049 ms;
`+bench-features` FZF 1.26 ms, regex 0.12 ms.

### Memory (`footprint`, `.app` bundle unless noted)

| State | Footprint | Notes |
|---|---|---|
| 1 window, after 128 MiB output, 6 s idle | 77 MB | IOSurface 45 MB (5 regions), Malloc Small 11 MB, IOAccelerator ~10 MB |
| 0 windows, after output | 22 MB | Malloc Small 11 MB |
| 0 windows, no output | 20 MB | Malloc Small 9.8 MB |
| Raw binary (not bundled), 0 windows | 34 MB | +8 MB `CG Image` from `set_dock_icon` (`src/platform/macos/app.rs:144`) |

One "1 window, no output" sample read 18 MB with no IOSurface; the window was
probably never drawn, so that sample is not used.

### Startup and idle

| Metric | Result |
|---|---|
| Launch → shell exec, cold (first run of a new binary) | 1,344 ms |
| Launch → shell exec, warm (median of 5) | ~200 ms, ~150 ms after subtracting the probe's own `python3` startup |
| `MTLCreateSystemDefaultDevice` | 18 ms every launch |
| `newLibraryWithSource` on `shader.metal` | 311 ms cold, 1 ms warm (system shader cache) |
| Idle CPU, 1 window and 0 windows | 0.0% |

Not measured: keystroke-to-screen latency, launch → first presented frame.

## Issue index

| ID | Issue | Status | Priority |
|---|---|---|---|
| B1 | No input-latency benchmark | Confirmed gap | P0 |
| B2 | No first-frame benchmark | Confirmed gap | P0 |
| B3 | No end-to-end PTY throughput benchmark | Confirmed gap | P0 |
| T1 | Glyph shaping and rasterization run under the grid lock | Measured; deferred after T2 (≤ 3% left) | P3 |
| T2 | CoreText shaping allocates per run, per dirty row, per frame | Done | P1 |
| T3 | OSC 8 throughput 3.3 MB/s | Done (scan skipped when provably full) | P1 |
| T4 | Region scroll 36 MB/s | Closed: cost is the row copy itself (no change) | P2 |
| T5 | Unicode/emoji path 56–75 MB/s | Hypothesis on cause | P2 |
| T6 | Grid locked 4–5 times per frame | Confirmed path | P2 |
| T7 | Terminal reply buffer reallocated each batch | Confirmed path | P3 |
| S1 | Metal shaders compiled from source at startup | Done (overlapped, not precompiled) | P1 |
| S2 | ~130 ms warm startup unexplained | Explained; font load overlapped | P1 |
| M1 | 1-window footprint dominated by 45 MB IOSurface | Confirmed | P2 |
| M2 | ~10 MB Malloc Small retained with 0 windows | Unknown | P2 |
| M3 | Unbundled runs carry an 8 MB Dock icon | Done (set once, skipped in benchmarks) | P3 |
| D1 | Targets that cannot be met as defined | Confirmed | P0 (decision) |
| D2 | README and `performance.md` claims disagree with measurements | Confirmed | P1 |

## P0 — Benchmarks first

### B1 `+bench-input-latency`: keystroke → GPU-completed frame

Decided with the user (2026-11-08): in-process measurement, numbers only, no
PASS/FAIL threshold until a baseline exists.

- **Measured path:** synthetic `KeyInput` injected into `app::keyboard::handle_key`
  → `keymap` → PTY writer queue → child echo → `pty-reader` → grid → frame
  build → submit → GPU completion of the frame that contains the echoed byte.
- **Excluded (state in the output):** OS input stack before Litty sees the
  event, compositor and display scanout (+0–1 vsync).
- **Child:** `litty +bench-echo-child`, raw mode, reads one byte and writes it
  back. No shell, so prompt themes and line editors do not add noise. Same
  pattern as `src/bench_dsr.rs` (`CHILD_FLAG`, `Pty::spawn_program`).
- **Timestamps, monotonic `Instant`:** `t_key` (inject), `t_pty_write`,
  `t_echo_in_grid` (reader saw the byte), `t_submit`, `t_gpu_done`. Report
  each stage and the total.
- **GPU completion:** a bench-only wait after `present_and_submit`. Metal:
  `InFlight::wait` / `waitUntilCompleted` (`src/render/backend/metal/pass.rs:21`).
  Vulkan: wait on the frame fence or `vkQueueWaitIdle`. Expose one
  `CommandEncoder::present_and_submit_wait` on both backends (Metal already has
  it at `pass.rs:88`; Linux needs the counterpart).
- **Frame detection:** the bench types a known sequence (e.g. cycling `a..z`)
  and checks the grid cell at the cursor before stamping `t_submit`, so a frame
  that does not yet contain the echo is not counted.
- **Pacing:** one key at a time, the next only after the previous frame
  completed, plus a randomized 0–16 ms gap so samples do not phase-lock to
  vsync. Warmup 50 samples discarded; default 1,000 samples.
- **Output:** one JSON line like `+bench-scroll-fps`, using `bench_dsr::summarize`
  (min/mean/p50/p95/p99/max) for the total and each stage, plus window size,
  scale factor and refresh rate.
- **Files:** `src/cli.rs` (new commands + parse tests), `src/main.rs`,
  `src/app/mod.rs` (bench driver in `about_to_wait`, like `scroll_fps_run`),
  `src/render/frame_metrics.rs` (sample collection), both backends' `pass.rs`.
- **Tests:** CLI parse tests; unit test for the stage aggregation (pure);
  echo child roundtrip test over a real PTY (no window). The GUI run itself is
  verified by running it and quoting its JSON.
- [x] Implement, run 3× on macOS, record numbers in this file (`1bca032`).
- [ ] Run on Linux X11 (container or host). **Not run yet**: only `cargo check` / `clippy` for `x86_64-unknown-linux-gnu`.
- Deviations from the design above: the reader-side stamp (`t_echo_in_grid`)
  is not taken; `echo` ends when the main thread first sees the byte in the
  grid right before a draw, so it includes the wake-up. A frame that picks up
  the echo between that check and its own grid read is not counted, so a
  sample can be one frame late, never early. `render` includes waiting for a
  free drawable (`nextDrawable`).

Result, 2026-11-09 (M1 Pro, window 2590×1492 px, scale 2, 1,000 samples after
50 warmup, three runs):

| Stage | p50 ms | p95 ms | p99 ms |
|---|---|---|---|
| total | 3.6–3.9 | 9.8–10.2 | 12.3–14.7 |
| input | 0.012–0.013 | 0.027–0.034 | 0.04–0.33 |
| echo | 0.39–0.44 | 3.3–4.1 | 8.1–10.0 |
| render | 0.30–0.35 | 4.4–5.4 | 7.8–8.8 |
| gpu | 2.45–2.52 | 4.7–4.9 | 6.0–6.1 |

Reading: the median is dominated by GPU completion (~2.5 ms for a frame
that changes one cell), the tail by `echo` and `render`; both tails exceed
one 120 Hz frame. These are new findings for P1/P2, not yet investigated.

### B2 `+bench-first-frame`: launch → first GPU-completed frame

- **Parent** `litty +bench-first-frame [runs]` spawns `litty +bench-first-frame-child`
  `runs` times (default 10) and summarizes; reports run 1 separately (cold) and
  runs 2..n (warm).
- **Child** records `Instant` at the top of `main`, plus process start from the
  kernel (`platform::process_start_time()`: macOS `sysctl` `KERN_PROC_PID`
  `p_starttime`; Linux `/proc/self/stat` field 22 + `btime`). It prints
  `exec_to_main_ms`, `main_to_window_ms`, `window_to_first_submit_ms`,
  `first_submit_to_gpu_done_ms`, then exits.
- Kernel start time is wall clock, so only the `exec_to_main` difference uses
  `SystemTime`; every other stage uses `Instant`. The stage split is a pure
  function with unit tests.
- **Stage stamps** also answer S2: device creation, atlas/font load, pipeline
  build, PTY spawn, first frame.
- [x] Implement (`ebfc8da`). Deviation: instead of a kernel process-start
  query per OS, the parent passes its `CLOCK_MONOTONIC` reading at spawn and
  the child subtracts it (that clock is system-wide), so no new platform
  function was needed. Stages are `exec`, `init` (main → window created),
  `first_submit`, `gpu`; finer stamps inside `init` are still needed for S2.
- [ ] Cold run. **Not achieved**: run 1 below was not cold (the shader
  cache was already warm from earlier runs of the same shader source).
- [x] Warm runs, 2026-11-09 (M1 Pro, 10 launches):

| Stage | Run 1 ms | Runs 2–10 p50 ms | min–max ms |
|---|---|---|---|
| total | 196.9 | 211.4 | 203.0–224.1 |
| exec | 6.7 | 12.3 | 7.4–25.9 |
| init | 176.9 | 183.4 | 172.3–194.4 |
| first_submit | 8.8 | 9.2 | 8.2–12.0 |
| gpu | 4.6 | 4.6 | 3.3–5.0 |

Reading: `init` (config, fonts, GPU device, window, PTY spawn) is ~87% of
startup; that is where S2 must look.

### B3 `+bench-pty-throughput`: bytes through the whole app

- Child `litty +bench-cat-child <kind> <MiB>` writes a generated stream
  (ascii, sgr, code, unicode, osc8, region) to its PTY as fast as possible;
  the app renders normally; the child reports MB/s when the last byte was
  accepted by the kernel, and the app reports when the grid consumed it and the
  last frame completed.
- Generators are shared with `grid::perf_probe` so grid-only and in-app numbers
  use identical bytes.
- [x] Implement (`399a777`). Deviation: the generators live in
  `src/bench_throughput.rs`, not shared with `grid::perf_probe`; instead the
  parent measures a parser-only rate on the exact same bytes.
- [x] Result, 2026-11-09 (M1 Pro, 64 MiB per kind, one run each):

| Kind | In app MB/s | Grid only MB/s | In app / grid |
|---|---|---|---|
| ascii | 116.3 | 1,029.3 | 0.11 |
| sgr | 98.8 | 488.3 | 0.20 |
| code | **56.0** | 1,566.1 | **0.04** |
| unicode | 48.1 | 76.2 | 0.63 |
| osc8 | 4.8 | 2.6 | 1.87 |
| region | 48.2 | 79.3 | 0.61 |

Reading: for `code` the parser is the fastest of all kinds yet the app is
28× slower than it, which supports T1/T2 (work outside the parser,
under or around the grid lock). `osc8` runs faster in the app than in the
parser-only pass; the cause is unknown (the in-app grid has a different
size, and both use the default 5,000-line scrollback) and belongs to T3's
isolation step.

### D1 Decide what the targets measure (user decision, blocks P2 memory work)

| Target | Why it cannot be met as written | Proposal |
|---|---|---|
| PTY throughput > 2.5 GB/s | macOS PTY delivers ~220 MB/s to a plain reader | Keep 2.5 GB/s for **grid-only** (`perf_probe`); set the in-app target as a share of the raw PTY ceiling, e.g. ≥ 80% |
| Idle RAM 1 window < 22 MB | Two BGRA8 drawables at Retina size are ~25 MB each | Measure 1-window RAM excluding IOSurface, or set the target per window size |
| `CLAUDE.md` < 50 MB with windows closed | Met (20–22 MB) | Keep |

- [ ] Ask the user; update `docs/plans/0.10.0.md` §1 targets and `CLAUDE.md` if changed.

## P1 — Throughput and startup

### T1 Move shaping and rasterization out of the grid lock

- **Where:** `src/render/renderer/frame.rs:269` locks the pane grid and calls
  `build_pane_row` (`src/render/renderer/pane_row.rs:51`), which calls
  `atlas.shape_row`, `atlas.slot`, `glyph_slots`, `cluster_slots`. On a cache
  miss these rasterize with CoreText/FreeType and upload to the atlas texture
  while the PTY reader waits for the lock.
- **Evidence:** code text runs at 64 MB/s vs 137 MB/s for plain ASCII in the
  app, with the same raw PTY speed for both. Not yet isolated from T2.
- **Isolation step:** add lock-wait time to the reader (time spent in
  `grid.lock()` in `feed_output`, `src/terminal/pty.rs:111`) behind the bench
  flag; run B3 with ascii vs code.
- **Fix:** copy the dirty rows (cells + needed cluster text/extras) into a
  reused per-renderer scratch buffer under the lock, release the lock, then
  shape, rasterize and build instances from the copy. Scratch buffers are
  sized once per resize so the frame stays allocation-free.
- **Tests:** existing renderer tests; a test that the reader can take the
  lock while a frame build is shaping (same style as the image-decode lock
  test in `pty.rs:752`).
- **Acceptance:** B3 code/ascii ratio and reader lock-wait before/after.
- [x] Isolation, 2026-11-09 (M1 Pro, 512 MiB `code` stream, temporary
  counters in `feed_output` and `collect_pane_instances`, not committed).
  Per second of streaming:

| Build | Reader waits for lock | Reader parses | Render holds lock | Of which `shape_row` |
|---|---|---|---|---|
| before T2, synthetic `code` line | 560 ms | 45 ms | 560 ms | 550 ms |
| after T2, synthetic `code` line | 18 ms | 107 ms | 12 ms | 4 ms |
| before T2, real source (800 KB of `src/**/*.rs`, cycled) | 110 ms | 255 ms | 105 ms | 92 ms |
| after T2, real source | 35 ms | 290 ms | 30 ms | 21 ms |

  Shaping was the whole lock hold; T2 removed most of it.
- [ ] Fix. **Deferred**: after T2 the render holds the lock 1–3% of the
  time, so snapshotting rows would win at most ~3% throughput for a large
  refactor of `build_pane_row`. The reader is now idle ~60% of the time
  waiting on the PTY, not on the lock; the next throughput gain is on the
  read side (read size, wakeups), not here. Revisit if a cache-miss-heavy
  workload shows the lock hold above ~10%.

### T2 Cache shaping results; stop allocating in CoreText per frame

- **Where:** `Shaper::shape_row` (`src/render/atlas/shaper.rs:55`) calls
  `shape_ascii_run` (`src/render/atlas/coretext.rs:386`) for every run with
  punctuation in every dirty row, every frame. Each call creates a
  `CFString`, `CFDictionary` and `CTLine`. This breaks the zero-allocation
  hot-path invariant.
- **Fix:** bounded cache keyed by (run bytes hash, style, px) → shaped glyph
  pieces, e.g. fixed-capacity open-addressing table with LRU; reuse the
  attribute dictionary per font/style instead of rebuilding it. Skip shaping
  entirely when the font has no GSUB ligature features enabled (detect once at
  atlas build).
- **Tests:** shaping output identical with and without cache (property test
  over random punctuation runs); allocation counter test using `src/test_alloc.rs`
  showing zero allocations on a steady-state frame with code text.
- **Acceptance:** B3 code throughput, frame `render_ms` p95 from `+bench-scroll-fps`.
- [x] Fix, branch `fix/shaping-cache`:
  - `RunCache` (`src/render/atlas/run_cache.rs`) maps (style, run text) to
    the run's output words at one size and cell width. Two generations of
    4,096 entries / 64 KiB of key bytes each: when the current one fills it
    becomes the previous one, and previous-generation hits are copied forward,
    so runs in use survive a flood of one-off runs. All-zero outputs (every
    run with a font without ligatures) store no words. One randomly keyed
    hash per lookup; collisions are checked against the stored key. Cleared
    on a piece-id reset and on a size change. A warm hit never calls CoreText
    or HarfBuzz and never allocates.
  - `NativeFont::lookup_glyph_index` (macOS) asked `font_at(16.0)` between
    `font_at(px)` calls, so the one-entry sized-font cache created a new
    `CTFont` for every shaped glyph. It now uses any cached font (glyph ids
    do not depend on size), and the cache keeps up to 4 sizes so windows at
    different scale factors sharing one font do not evict each other. Linux
    was not affected.
  - Found on the way: `set_font_features` held `FONT_FEATURES` while taking
    `FONT_CACHE`, the reverse of `load_font` → `parse_font`; the parallel
    test run with `LITTY_LIGATURE_FONT` set deadlocked 10/10. It now releases
    `FONT_FEATURES` first (regression test included).
  - Not done: reusing the attribute dictionary and skipping fonts without
    GSUB features. With the cache, misses cost ~21 ms/s on real source; not
    worth the extra platform code yet.
- [x] Tests: `RunCache` unit tests (stored words per style and text,
  collision is a miss, hot runs survive a flood, bounded, size change clears,
  warm lookup allocation-free); cached vs fresh shaper output equal (default
  font, and a ligature font via `LITTY_LIGATURE_FONT`, run with JetBrains
  Mono); repeated runs shaped once; cache cleared on piece reset; glyph lookup
  keeps the cached font size; several font sizes cached.
- [x] Result, 2026-11-09 (M1 Pro, `+bench-pty-throughput 64`, one run each,
  release builds of `b6f322e` and of this branch):

| Kind | Before MB/s | After MB/s |
|---|---|---|
| ascii | 149.6 | 150.2 |
| sgr | 119.0 | 152.4 |
| code | **58.5** | **150.9** |
| unicode | 53.7 | 52.4 |
| osc8 | 4.9 | 4.9 |
| region | 56.9 | 55.8 |

  Real source stream (temporary generator, 512 MiB, instrumented builds of
  the first cache version): 114 → 136 MB/s.
  `+bench-scroll-fps` `render_p95_ms`: slow 60 Hz 1.00 → 0.53; the 120 Hz
  scenarios are bound by drawable acquire (~9 ms p95) before and after.
  ascii/unicode/region changes are within run-to-run noise (single runs);
  sgr has punctuation runs (`req=`, `/api/v1`), so it gains too.

### T3 OSC 8 throughput (299 ns/byte)

- **Hypothesis:** `intern_extra` (`src/terminal/grid/links.rs:161`) does a
  linear `position` over up to 256 `Option<CellExtra>` with string compares on
  every OSC 8; once the 255 slots fill (the probe uses 400 distinct URIs),
  `collect_extras` scans the whole scrollback + screen on each new link.
- **Isolation step:** probe variants with 100 vs 400 distinct URIs; count
  `collect_extras` calls.
- **Fix candidates:** hash index from extra → slot; amortize `collect_extras`
  (run at most once per N new links, or per scrollback page evicted).
- **Tests:** existing OSC 8 tests; new test for slot reuse after eviction.
- **Acceptance:** `perf_probe osc8_links` before/after; no change to other cases.
- [x] Isolation, 2026-11-09 (temporary counters, `perf_probe`, 20,000 links):

  | Distinct URIs | ns/B | Scans per run | Scans that freed a slot | Links dropped |
  |---|---|---|---|---|
  | 100 | 18 | 0 | — | 0 |
  | 400 | 330–555 | 5,522 | 0 | 7,250 |

  The 255 slots fill with the first URIs, which stay in the scrollback, so the
  table never frees a slot. But every scrolled-out row and every closed link
  sets `extras_dirty`, so each new distinct link rescans the ring until it has
  seen 255 slots. The linear `position` lookup is not the bottleneck (the
  100-URI case runs at 18 ns/B).
- [x] Fix: `Grid::extras_witness` keeps, per slot, the cell where the last scan
  saw it. Before a scan, `extras_proven_full` checks 255 cells: if each slot
  is still held by its recorded cell (in a live ring row) or by a pen, the
  table is full and the scan is skipped. The scan now walks the newest rows
  first, so the recorded cells stay in the scrollback longest. Results are
  unchanged by construction (a slot is only proven used by a live cell that
  holds it). A temporary test fed both versions (with and without witnesses)
  6,000 lines that mix links, overwrites and region scrolls, and compared every
  cell's link and the slot table.
- [x] Result, 2026-11-09:

  | Measure | Before | After |
  |---|---|---|
  | `perf_probe osc8_links` | 308–555 ns/B | 32 ns/B |
  | `+bench-pty-throughput 64` osc8, in app | 4.8 MB/s | 26.5 MB/s |
  | Same, grid only | 2.5–2.6 MB/s | 31.2 MB/s |
  | Full scans, 4,000 lines × 400 URIs, 1,000-line scrollback | 1,162 | 5 |

  Other `perf_probe` cases and `+bench-pty-throughput` kinds stayed within run
  noise (two runs each). `Grid` grows by 1 KB per pane. The osc8 in-app/grid
  ratio (1.87 before) is now 0.85, in line with the other kinds.
- Independent review: no correctness finding. It checked the proof against the
  old scan on 400 random seeds × 20k operations (links, SGR 58, erases, alt
  screen, reset, insert/delete, scroll regions, resize, clear history). It
  found that no test covered the live-ring-row check; one now does, and it
  fails when that check is removed.
- Not done: links over the 255-slot limit are still dropped (an 8-bit index
  in the 12-byte `Cell`). Each OSC 8 still allocates its `OscLink` strings and
  the joined URI. At 32 ns/B this is no longer the bottleneck; measure before
  changing it.

### S1 Load precompiled Metal shaders

- **Where:** `src/render/backend/metal/device.rs:52` and `:208` use
  `newLibraryWithSource:` on `shader.metal` at startup; 311 ms on a cold
  shader cache (first launch after install or update).
- **Fix:** compile `shader.metal` to `.metallib` at build time and load it
  with `newLibraryWithData:`. Without full Xcode (`metal` compiler absent on
  this machine), options are: (a) require the Metal toolchain only for release
  builds and keep the source path as a fallback for dev builds; (b) check in
  the `.metallib` with a test that it matches the source hash. Needs a user
  decision because it changes the build requirements (`docs/development/building.md`).
- Custom shaders (`create_postfx_pipeline`) stay source-compiled.
- **Acceptance:** B2 cold first-frame before/after.
- [x] Decision, 2026-11-09 (user): no build-requirement change. Instead the
  device and shader compile start on a thread (`src/app/prewarm.rs`) before
  the platform event loop starts, overlapping AppKit's launch and the font
  load. The first window takes the result from the `SHARED_METAL` cache or
  waits on its mutex; nothing is compiled twice. Precompiling (`.metallib`)
  stays possible later; it would remove the MSL front end only, not the GPU
  back-end compile.
- [x] Result: see S2 (font and GPU prewarm were measured together).

### S2 Explain the remaining ~130 ms warm startup

- Use B2 stage stamps. Candidates to check: font discovery (`fontdb`), atlas
  creation, shell-integration setup, config parse, `update::check_on_startup`
  (must not block), window creation.
- Each found cost becomes its own task with before/after numbers.
- [x] Breakdown, 2026-11-09 (M1 Pro, unbundled release binary, temporary
  stamps from `main`, not committed; warm = runs 2–6, cold = first run of a
  new binary with a new shader source hash):

| Step (main thread, in order) | Warm ms | Cold ms |
|---|---|---|
| `main` → `resumed` (AppKit `sharedApplication`, `finishLaunching`) | 76–98 | 140 |
| `Config::load` | 0.3 | 1.7 |
| `load_font` (font discovery and CoreText faces) | 57–66 | 297 |
| OS window | 46–58 | 102 |
| `set_dock_icon` (unbundled runs only, see M3) | 31 | 35 |
| GPU device + `newLibraryWithSource` | 2–3 | **294** |
| surface, atlas, pipelines, resize | ~9 | ~8 |
| PTY spawn | 3.5 | 8 |

  Reading: AppKit's launch, the font load and the window are the warm cost;
  on a cold start the shader compile and the font load each cost ~300 ms.
  Neither the font set nor the GPU device needs a window, so both now start
  on threads at the top of every GUI run (`src/app/prewarm.rs`) while AppKit
  launches. The main thread reads the config first (`Config::load_quiet`,
  ~0.3 ms, errors left to the window's own read) and sets the process-wide
  font tweaks and features itself; the thread only calls `load_font`, so a
  late thread can never overwrite newer settings. The GPU prewarm is macOS
  only: on Linux a Vulkan driver may use Xlib while the main thread opens the
  display.
- [x] Result, 2026-11-09 (M1 Pro), prewarm on vs off (temporary build switch):
  - Cold: `+bench-first-frame 4` per build, 3 builds each, every build with
    a new shader source hash so its first run compiles the shader cold. First
    run total: off 462, 472, 531 ms; on 336, 233, 251 ms (median 472 → 251).
  - Warm: one build each, 4 alternating rounds of `+bench-first-frame 10`,
    p50 total: off 246, 251, 242, 243 ms; on 190, 197, 200, 187 ms
    (median ~244 → ~193); `init` p50 ~214 → ~159 ms.
  - `+bench-ram`: footprint unchanged (1 window 71 vs 72 MB, 2 windows 107
    vs 107 MB).
- Left: AppKit launch (~80 ms) and window creation (~50 ms) are on the main
  thread by AppKit's rules. `set_dock_icon` (31 ms) only runs unbundled (M3).

### D2 Fix published claims

- README / `docs/development/performance.md` state "< 30 ms" startup and
  "~47 MB" idle RAM; measurements are ~150 ms warm / 1.3 s cold and 77 MB with
  one window (20–22 MB with none). Update after B1–B3 exist, with the
  method stated next to each number.

## P2 — Secondary throughput and memory

### T4 Region scroll (28 ns/byte)

- **Hypothesis:** `scroll_up_in` / `scroll_down_in` (`src/terminal/grid/storage.rs:188`,
  `:252`) move a scroll region one row at a time with `copy_row`, which also
  scans the destination row for extras; a full-region scroll per linefeed
  costs rows × cols cell copies plus the scan.
- **Isolation step:** probe with extras present vs absent; profile with
  `cargo test --release` + `samply` or Instruments if available.
- **Fix candidates:** one `copy_within` per contiguous ring span; skip the
  extras scan when `extras.len() <= 1` (already partly done) or track a
  per-row "has extras" bit.
- **Acceptance:** `perf_probe region_scroll`; tmux/vim scrolling via B3 `region`.
- [x] Isolation, 2026-11-09 (temporary `perf_probe` cases, 160 × 50 grid,
  same 5,000 lines):

  | Region | Rows copied per line | ns/B |
  |---|---|---|
  | `5;9r` | 4 | 4.6 |
  | `5;45r` (`region_scroll`) | 40 | 28.6 |
  | `2;49r` | 47 | 35.7 |

  Cost grows linearly with the rows copied (~0.7 ns/B per row): about 77 KB
  of `memmove` per line, ~55 GB/s, i.e. memory bandwidth. The extras scan in
  `copy_row` does not run here (no extras), so merging rows into one
  `copy_within` would not reduce the bytes moved.
- [x] Decision, 2026-11-09 (user): close without a code change. The in-app
  rate (B3 `region`, ~52 MB/s) is far above what tmux or vim emit. The only
  real fix, a row-indirection table that turns a region scroll into an index
  rotation, would change every row access (print, display, selection,
  search, resize, images). Reopen if a real workload is shown to be slow.

### T5 Unicode and emoji paths (13–18 ns/byte)

- Profile first; candidates are width lookup, grapheme cluster interning and
  NFC normalization per character. No fix without a profile.

### T6 Lock the grid once per pane per frame

- `read_active_pane`, `nothing_changed`, `collect_pane_instances`,
  `snap_fractional_scroll` and `rebuild_after_eviction` each lock
  (`src/render/renderer/frame.rs:62`, `:184`, `:227`, `:269`, `:359`).
- Fold the dirty check and active-pane read into the T1 snapshot step.
- Acceptance: reader lock-wait (from T1 instrumentation).

### M1 1-window footprint (45 MB IOSurface, 5 regions)

- Two drawables are configured (`src/render/backend/metal/surface.rs:47`), yet
  five IOSurface regions were present after heavy output.
- **Isolation step:** list the IOSurfaces (`footprint -v`, `vmmap`) and map
  sizes to drawables, postfx targets and the atlas.
- **Fix candidates:** release drawables after an idle period
  (`shrink_drawables` already exists, `surface.rs:125`), and when occluded.
- Blocked by D1 for the target number.

### M2 ~10 MB Malloc Small with 0 windows

- Present even with no terminal output, so it is not scrollback.
- **Isolation step:** `heap` / `leaks --list` on the 0-window process; check
  global caches that outlive windows (font DB, shared Metal device and
  pipelines in `gpu::shared_device` / `shared_pipeline`, config, history).
- Fix only what the investigation names.

## P3 — Small items

### T7 Reuse the terminal reply buffer

- `std::mem::take(&mut reply)` (`src/terminal/pty.rs:520`) hands the buffer to
  the writer and allocates a new one on the next reply. Only matters for
  reply-heavy apps (DSR/DA floods). Measure with `+bench-dsr` before changing.

### M3 Dock icon in unbundled runs

- `set_dock_icon` decodes `resources/icon.png` into an 8 MB bitmap when not in
  a `.app`. Affects dev runs and `+bench-ram` numbers taken from the raw
  binary, not users. Options: smaller embedded icon, or document that RAM
  benchmarks must run from the bundle. Decide during D2.
- Also found: `build_window` called it for every window, so each new window
  decoded the icon again (~31 ms on the main thread), and
  `scripts/pre-release-check.sh` runs `+bench-ram` on the raw binary, so the
  release RAM gate included the icon.
- Decision: no asset change. The icon is set once per process, from
  `App::resumed`, and not at all in benchmark runs (`+bench-ram` and the GUI
  benchmarks), so they measure what bundled users get.
- Result (raw binary, `+bench-ram` footprint, 2 alternating rounds):

  | Step | Before | After |
  |---|---|---|
  | 1 window, 1 tab | 74 MB | 65 MB |
  | 1 window, 3 tabs | 75 MB | 66 MB |
  | 2 windows, 6 tabs | 109 MB | 100–101 MB |
  | All windows closed | 61 MB | 52 MB |

  `+bench-first-frame 8` warm p50, 3 alternating rounds: 177–188 ms before,
  151–167 ms after. That drop is the benchmark no longer paying the icon, not
  a faster start for users: bundled runs never set it, and unbundled dev runs
  still pay it once before the first frame.

## Order of work

1. B1, B2, B3 (one branch each) and D1 decision.
2. T1 isolation → T2 → T1 fix (T2 first if isolation shows shaping dominates).
3. S1 (after the build-requirement decision), S2 from B2 data, T3.
4. D2 docs update with the new numbers.
5. T4, T5, T6, M1, M2.
6. T7, M3.

After each item: update the baseline tables above with the new numbers and
the commit they came from.
