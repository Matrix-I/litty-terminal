# Performance review — 2026-11-11

Reviewed at commit `ac5a861` (v0.11.1). No source was changed by this review.
Follows [the 2026-11-10 review](2026-11-10-perf-review.md), which covered the
parser and grid. Fix plan: [2026-11-11-perf-review-fixes](../plans/2026-11-11-perf-review-fixes.md).

## Scope

Read end to end: the PTY reader (`src/terminal/pty.rs`), grid storage and
resize (`src/terminal/grid/storage.rs`, `resize.rs`), the frame builder
(`src/render/renderer/frame.rs`, `pane_row.rs`), the glyph atlas lookup
(`src/render/atlas/mod.rs`), the event loop and redraw path
(`src/app/mod.rs`, `window.rs`, `window_event.rs`, `terminal_io.rs`) and
find-in-terminal (`src/terminal/search.rs`, `src/app/search_ui.rs`).

Not reviewed in depth: the Vulkan / Linux backend, `src/terminal/history.rs`,
`src/ai/`, `src/terminal/graphics/`.

## Method

- Temporary tests in `src/terminal/grid/tests.rs` (removed afterwards),
  `cargo test --profile fast-release`, Apple M1 Pro, macOS 27.2.
- `fast-release` uses thin LTO; the shipped `release` profile uses fat LTO.
  Treat the numbers as relative.
- No in-app benchmark (`+bench-pty-throughput`, `+bench-input-latency`,
  `+bench-first-frame`, `+bench-ram`) was run.

## Findings

### 1. A width change reflows every pane of every tab on the UI thread

- Where: `sync_size` (`src/app/window.rs:569`) loops over `s.tabs`, so each
  `WindowEvent::Resized` (`src/app/window_event.rs:67`) calls `Grid::resize`
  for the panes of background tabs too. A column change goes through
  `reflow_main` (`src/terminal/grid/resize.rs:95`), which copies the whole
  scrollback three times (`logical_cells`, `out`, `cells`).
- Measured, 200 columns, 150-character lines, 20 alternating resizes:

  | Scrollback | 200 ↔ 199 columns | Rows only |
  | ---: | ---: | ---: |
  | 5,000 lines (default) | 7.5 ms / pane | 0.02 ms |
  | 100,000 lines (`scrollback-limit`) | 233 ms / pane | 0.08 ms |

- A window drag sends one resize per step. With ten single-pane tabs at the
  default scrollback, each step costs about 75 ms on the UI thread
  (extrapolated from the per-pane number, not measured in the app).
- The rows-only fast path added after the 2026-11-10 review is effective.
- Suggested fix: resize only the visible tab on each event; bring background
  tabs to the new size once, after the burst settles or when they are shown.
- Not addressed by that fix: the visible pane with a very large scrollback
  still pays the full reflow on every step. The fix plan makes the default
  `scrollback-limit` a 10,000,000-byte budget, so the default cost no longer
  grows with the width; a larger configured limit still pays in proportion.

### 2. Occlusion handling is dead code: covered windows keep rendering

- Where: no platform emits `WindowEvent::Occluded`. The variant exists
  (`src/platform/gui.rs:596`) and `src/app/window_event.rs:124` handles it,
  but nothing in `src/platform/macos/` or `src/platform/linux/` sends it; the
  emitter went away with the winit removal (`62b2672`). `State::occluded` is
  therefore always `false`.
- Effect:
  - A covered or minimized window whose shell is printing rebuilds its
    instance rows and asks for a drawable on every wake (`AppEvent::WakeUp`,
    `src/app/mod.rs:378`; the reader notifies at most every 3 ms).
  - `Renderer::shrink_drawables` never runs for a covered window, so
    `docs/development/performance.md` ("they are freed when the window is
    occluded or closed") holds only for a closed window.
- Confirmed by reading the code and by `grep -rni occlu src/platform`. CPU and
  memory cost not measured.
- An earlier draft of this review blamed the missing `s.occluded` check in the
  `WakeUp` path. That check would change nothing while the flag is never set.
- Suggested fix: emit occlusion from the platform layer (AppKit window
  occlusion state on macOS; the Wayland / X11 counterpart or an explicit
  no-op on Linux), then skip rendering in the `WakeUp` path. Needs its own
  plan; tracked in `ROADMAP.md` under "Verification debt".

### 3. Styled ASCII glyphs miss the direct atlas cache

- Where: `Atlas::glyph_slots` (`src/render/atlas/mod.rs:471`) adds
  `STYLE_BOLD` / `STYLE_ITALIC` to the key, so a bold or italic ASCII glyph
  has a key ≥ 128 and `Atlas::slot` looks it up in
  `map: HashMap<u32, u16>` (SipHash) instead of the `ascii` array. Box
  drawing, CJK and wide glyphs take the same path.
- The lookup runs once per cell in `build_pane_row`, with the grid lock held.
- Measured, 8,000 lookups (one 160 × 50 screen), 600 entries:

  | Lookup | Time |
  | :--- | ---: |
  | `HashMap<u32, u16>` with SipHash | 232 µs |
  | Direct array index | 19 µs |

- Low priority: about 0.2 ms per fully styled frame.
- Suggested fix: a multiplicative hasher for the `u32` keys (no new
  dependency).

## Read and fine

- PTY reader: 64 KiB reads, 8 KiB feed chunks, one grid lock per chunk, image
  decoding outside the lock, at most 3 ms of batching before a wake.
- `Waker` coalesces a burst of output into one pending `WakeUp`.
- `render_panes` checks `nothing_changed` before acquiring a drawable.
- Row scratch buffers, `render_panes` and `pane_rects` are reused across
  frames; no per-frame heap allocation found by reading.
- Find-in-terminal copies the text under the lock and matches outside it.
- Main-screen scroll is an O(1) ring rotation.

## Suggested order

1. Finding 1 (in the fix plan).
2. Finding 3 (in the fix plan).
3. Finding 2 (separate plan: it needs platform work on both targets).
