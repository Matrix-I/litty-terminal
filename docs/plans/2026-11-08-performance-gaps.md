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
| T1 | Glyph shaping and rasterization run under the grid lock | Confirmed path, impact Hypothesis | P1 |
| T2 | CoreText shaping allocates per run, per dirty row, per frame | Confirmed path | P1 |
| T3 | OSC 8 throughput 3.3 MB/s | Hypothesis on cause | P1 |
| T4 | Region scroll 36 MB/s | Hypothesis on cause | P2 |
| T5 | Unicode/emoji path 56–75 MB/s | Hypothesis on cause | P2 |
| T6 | Grid locked 4–5 times per frame | Confirmed path | P2 |
| T7 | Terminal reply buffer reallocated each batch | Confirmed path | P3 |
| S1 | Metal shaders compiled from source at startup | Confirmed | P1 |
| S2 | ~130 ms warm startup unexplained | Unknown | P1 |
| M1 | 1-window footprint dominated by 45 MB IOSurface | Confirmed | P2 |
| M2 | ~10 MB Malloc Small retained with 0 windows | Unknown | P2 |
| M3 | Unbundled runs carry an 8 MB Dock icon | Confirmed | P3 |
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
- [ ] Implement, run 3× on macOS, record numbers in this file.
- [ ] Run on Linux X11 (container or host) or state that it was not run.

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
- [ ] Implement with the platform function on both OSes (Linux reads `/proc`).
- [ ] Run cold (first launch of a freshly built binary, which misses the system shader cache) and warm; record numbers here.

### B3 `+bench-pty-throughput`: bytes through the whole app

- Child `litty +bench-cat-child <kind> <MiB>` writes a generated stream
  (ascii, sgr, code, unicode, osc8, region) to its PTY as fast as possible;
  the app renders normally; the child reports MB/s when the last byte was
  accepted by the kernel, and the app reports when the grid consumed it and the
  last frame completed.
- Generators are shared with `grid::perf_probe` so grid-only and in-app numbers
  use identical bytes.
- [ ] Implement; record the full table (replaces the manual `cat` numbers above).

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

### S2 Explain the remaining ~130 ms warm startup

- Use B2 stage stamps. Candidates to check: font discovery (`fontdb`), atlas
  creation, shell-integration setup, config parse, `update::check_on_startup`
  (must not block), window creation.
- Each found cost becomes its own task with before/after numbers.

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

## Order of work

1. B1, B2, B3 (one branch each) and D1 decision.
2. T1 isolation → T2 → T1 fix (T2 first if isolation shows shaping dominates).
3. S1 (after the build-requirement decision), S2 from B2 data, T3.
4. D2 docs update with the new numbers.
5. T4, T5, T6, M1, M2.
6. T7, M3.

After each item: update the baseline tables above with the new numbers and
the commit they came from.
