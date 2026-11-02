# Split `grid/mod.rs` — Implementation Plan (Phase 2)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Break the 5499-line `src/grid/mod.rs` (one `Grid` struct with 88 fields, 128 inherent methods, ~25 free functions and a 1777-line test module) into cohesive submodules of `src/grid/`, and group four field families into owned sub-structs (`AiTrack`, `TerminalModes`, `ClusterTable`, `Ring`), with zero behavior change and no measurable performance change.

**Architecture:** Phase 2 of the 4-phase structural refactor outlined in `plans/2026-11-01-split-main-rs.md`. Rust allows any number of `impl Grid` blocks in any module of the crate, so almost every task is a *move*: methods leave `mod.rs` as an `impl Grid { ... }` block in a sibling file and keep operating on `Grid`'s fields, which stay visible to all of `grid`'s descendants. Only Tasks 12–15 change data layout, and they do it as pure field-path renames (`self.top` → `self.ring.top`); no logic is rewritten and no method moves *into* a sub-struct. Before any code moves, Task 0 adds a release-mode throughput probe so every task can be gated against a fixed baseline binary.

**Tech Stack:** Rust edition 2024 (`Cargo.toml`), vte, bytemuck, parking_lot; cargo targets `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`. Release profile is `lto = "fat"`, `codegen-units = 1` — the performance gate must use that profile (`--release`), never `fast-release`.

## Prerequisite

- Phase 1 (`plans/2026-11-01-split-main-rs.md`) is merged to `main`. The grid's external callers that this plan edits then live in `src/app/*.rs` instead of `src/main.rs`. Every location below that says `src/main.rs:<line>` is from commit `d1d0c92` and **must be re-located with the `grep` command given next to it** — the function name is given so you know where it went (e.g. `handle_key` → `src/app/keyboard.rs`).
- All line numbers in `src/grid/mod.rs` are from commit `d1d0c92` and shift after every move. Re-locate before cutting (see "How to move an item").

## Global Constraints

- Pure move / rename refactor: no behavior, timing, locking, or allocation change. If a task tempts you to "fix" something, write it in `ROADMAP.md` instead.
- `Cell` stays exactly 12 bytes: `src/grid/cell.rs` (with its `const _: () = assert!(std::mem::size_of::<Cell>() == 12);`) is not edited by any task.
- Hot paths (`feed_byte`, `Perform::print`, `put_char`, `linefeed`, `scroll_up_in`, row addressing) must not gain heap allocations, `dyn` dispatch, bounds checks, or locking. `feed_byte`, `put_char`, `linefeed`, `char_cell`, `extend_grapheme`, `mark`, `blank` and `colors` **stay in `mod.rs`**.
- `#[inline]` / `#[inline(always)]` attributes move with their items, unchanged. Do not add or remove any. At `d1d0c92` there are exactly 23 in `src/grid/*.rs`: in `mod.rs` on `default_fg`, `default_bg`, `row_start`, `buf_mut`, `row_mut`, `display_row`, `abs_line`, `set_wrapped`, `is_selected`, `is_path_char`, `is_blank`, `trim_col_range`, `put_char` (`inline(always)`), `mark`, `blank`, `feed_byte`, `rgb`, `palette_with`, `is_wide_cp`; 2 in `cell.rs`; 2 in `selection.rs`. Note `linefeed` has **no** `#[inline]` today — leave it that way.
- Never hold the grid `Mutex` across blocking I/O. This plan does not touch any `lock()` call; the only edits outside `src/grid/` are field-path renames inside existing expressions.
- Every `#[cfg(...)]`, `#[cfg_attr(...)]`, `#[allow(...)]`, `#[ignore]` and `///` / `//!` doc comment moves with its item, verbatim.
- One task = one commit that builds and passes everything on both targets. Commit format `refactor(grid): <summary>` (lowercase, imperative, ≤ 72 chars); Task 0 is `test(grid): ...`, Task 16 is `docs(grid): ...`. Stage files explicitly; never `git add -A`.

### Standard verification ("V")

Run before every commit (from `CLAUDE.md`), outside the sandbox (PTY tests need `openpty`; the mimalloc C build needs the SDK):

```bash
cargo check
cargo check --target x86_64-unknown-linux-gnu
cargo test 2>&1 | grep -E '^test result'
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: all succeed; the `cargo test` `passed` total equals `M` and the `ignored` total equals `I` (both recorded in Task 0). Then the three structural invariants:

```bash
cat src/grid/*.rs | grep -c '#\[inline'
cat src/grid/*.rs | grep -cE '^\s*(pub(\([a-z]+\))? )?fn '
cat src/grid/*.rs | grep -c '#\[test\]'
```

Expected: `23`, `F`, `T` (recorded in Task 0; at the time of writing `F = 281`, `T = 92`). A move never changes these numbers. If one changes, an item was dropped, duplicated, or an attribute lost.

### Performance gate ("G")

Scripts and baseline binaries live in `target/grid-gate/` (ignored by git, survives across sessions). Task 0 creates them. Each gate run rebuilds, then compares the new binaries against the **fixed** Task 0 baseline in interleaved rounds, so drift accumulated over several tasks is also caught:

| Gate | Used by | Command | Pass condition |
|------|---------|---------|----------------|
| G-std | Tasks 2–8, 12 | `target/grid-gate/gate.sh 3 3` | every probe case ≤ +3 % ns/byte, stress throughput ≥ −3 % |
| G-hot | Tasks 9–11, 13, 14 | `target/grid-gate/gate.sh 5 3` | same, best of 5 rounds |
| G-ring | Task 15 | `target/grid-gate/gate.sh 7 1.5` | every case ≤ +1.5 %, stress ≥ −1.5 %, best of 7 rounds, plus the `GRID_SIZE` check in Task 15 |

The probe is very stable (three back-to-back runs at the time of writing varied by < 0.5 % per case), so ±3 % is a real signal, not noise. Rules:

- `FAIL (slower)` → re-run the same gate once. If it fails again, do not commit: `git diff -M --color-moved=dimmed-zebra` the task, look for a lost `#[inline]`, a changed expression, or (Tasks 12–15) a field read that moved to a colder cache line; fix or `git restore` and redo the task.
- `CHECK (faster)` beyond the tolerance → also suspicious: a pure move cannot make the parser > 3 % faster, so the probe may be doing less work (behavior change). Re-run; if it persists, find out why before committing.
- Tasks 1 and 16 do not change non-test code and skip G.
- `+bench-perf` (latency SLAs including PTY spawn and file I/O) is too noisy for ±3 % and mostly measures non-grid code; it is run as a pass/fail SLA check in Task 0 and Task 16 only.

## Target File Structure

```
src/grid/mod.rs        Grid struct + new(); print path (put_char, char_cell, extend_grapheme,
                       combine_with_previous, append_to_cell, previous_cell, in_line_editor,
                       wide_pictograph, colors, mark, blank); feed_byte; cursor motion
                       (linefeed, reverse_index, tabs, goto_row, clamp_cursor); set_alt;
                       full_reset; cursor_pos; mod decls and re-exports        (~780 lines)
src/grid/cell.rs       unchanged (12-byte Cell + static assert, Pen, Cursor, KittyKeyboard, ...)
src/grid/perform.rs    unchanged except imports and field-path renames
src/grid/tests.rs      #[cfg(test)] unit tests moved out of mod.rs                  (~1780)
src/grid/perf_probe.rs #[cfg(test)] throughput probe + steady-state allocation test (~140)
src/grid/colors.rs     default colors / palette globals, SGR, OSC 4/104/10-12, palette helpers (~410)
src/grid/control.rs    DCS requests (XTGETTCAP, DECRQSS), OSC payload decoders, string limits (~185)
src/grid/images.rs     ImageJob / ImageCompletion, Kitty APC dispatch, Sixel, placements (~310)
src/grid/prompt.rs     PromptMark (existing) + AiTrack + OSC 133 marks, prompt input, jumps (~420)
src/grid/selection.rs  Selection types (existing) + selection and find-in-terminal methods (~490)
src/grid/links.rs      hyperlink detection, OSC 8, cell extras (links, underline colors) (~370)
src/grid/viewport.rs   scrollback viewing, precise scrolling, clear history          (~125)
src/grid/resize.rs     resize with reflow                                            (~200)
src/grid/visual.rs     bidi visual order and column mapping                          (~80)
src/grid/width.rs      char_width, is_wide_cp, is_pictographic, macOS wcwidth        (~80)
src/grid/clusters.rs   ClusterTable + intern / lookup / GC of grapheme clusters      (~210)
src/grid/storage.rs    scrollback limits, Ring, row addressing, clear / copy / scroll rows (~280)
src/grid/modes.rs      TerminalModes, grapheme-mode default, kitty_keyboard_flags    (~70)
```

End state: `src/grid/mod.rs` ≈ 780 lines (from 5499); `Grid` has **57** top-level fields (from 88: 6 `ai_*` → `ai`, 17 mode fields → `modes`, 6 `cluster*` fields → `clusters`, 6 ring fields → `ring`).

## Visibility Rules

Privacy in Rust is per module: a private item of `grid` (mod.rs) is visible to `grid` and all its descendants (`perform`, `tests`, every new file); a private item of `grid::colors` is visible only inside `colors`.

1. **Grid fields** stay where their visibility already is. Private `Grid` fields are readable from every `impl Grid` block in every `src/grid/*.rs` file — nothing to do.
2. **Methods moved into a submodule** (`impl Grid` block in `src/grid/<file>.rs`):
   - `pub fn` stays `pub`; `pub(crate) fn` stays `pub(crate)`;
   - private `fn` becomes `pub(super) fn` (visible in `grid` and all its descendants — needed because `perform.rs`, `tests.rs` and sibling files call them). Apply this to every private method you move, even if nobody else calls it today; it keeps the rule mechanical.
3. **Free functions, statics, consts, types moved into a submodule:**
   - `pub` items stay `pub`; mod.rs re-exports the ones used outside `grid` (list per task) with `pub use <file>::{...};` so `crate::grid::<name>` paths in `renderer.rs`, `bidi.rs`, `pty.rs`, `config.rs` and `src/app/` keep compiling unchanged;
   - private items become `pub(super)`; each file that uses them imports them explicitly with `use super::<file>::{...};` (mod.rs uses `use <file>::{...};`).
   - `pub(crate)` items (`ImageJob`, `ImageCompletion`) stay `pub(crate)` and are re-exported with `pub(crate) use`.
4. **New sub-struct types:** `AiTrack` and `TerminalModes` back `pub` fields of `Grid`, so they are `pub struct` with `pub` fields where the old field was `pub` and `pub(super)` where it was private; they are re-exported from mod.rs. `ClusterTable` and `Ring` back private fields, so they are `pub(super) struct` with `pub(super)` fields and imported privately.
5. **Imports in new files:** start each new file with `use super::*;` (as `perform.rs` does) for `Grid`, `Cell`, the `cell.rs` constants and everything mod.rs imports, then add `use super::<sibling>::{...};` for sibling helpers the compiler reports as unresolved. In `tests.rs`, add `use super::<file>::{...};` lines for private helpers the tests call (e.g. `parse_x11_color`, `osc8_target`, `rgb`) when the compiler asks.
6. **Unused imports:** after each move, `cargo clippy --all-targets -- -D warnings` reports imports that became unused (e.g. `AtomicU32` in mod.rs after Task 2). Delete them; if a name is still needed only by tests, import it in `tests.rs` instead of mod.rs. Repeat until clean. A `pub use` that clippy reports as unused is dropped too.

### External call sites that change

Only Tasks 12 and 13 edit files outside `src/grid/`. Everything else keeps `crate::grid::*` paths and method names unchanged.

| Field(s) | File at `d1d0c92` (function → Phase 1 location) | Becomes |
|----------|----------------------------------------------|---------|
| `ai_request`, `ai_command`, `ai_armed`, `ai_track` | `src/main.rs:1480,1481,1496,1497,1538` (`handle_ai_requests` → `src/app/ai_bridge.rs`) | `g.ai.request`, `g.ai.command`, `g.ai.armed`, `g.ai.track` |
| `ai_armed`, `ai_request`, `ai_available`, `ai_direct`, `ai_track` | `src/main.rs:2968,2969,3006,3017,3025,3026,3036,3037,3042,3053` (`handle_key` → `src/app/keyboard.rs`) | `….ai.<name>` |
| `bracketed_paste` | `src/main.rs:1015,1280` (`handle_window_event` → `src/app/window_event.rs`), `:1531,1602` (`handle_ai_requests` → `ai_bridge.rs`), `:3312` (`handle_history_search_key` → `search_ui.rs`), `:3569` (`run_tab_action` → `actions.rs`) | `….modes.bracketed_paste` |
| `mouse_tracking` | `src/main.rs:1220` (`handle_window_event`), `:2200` (`report_mouse` → `pointer.rs`) | `g.modes.mouse_tracking` |
| `focus_events` | `src/main.rs:1345` (`handle_window_event`) | `….modes.focus_events` |
| `cursor_shape` | `src/main.rs:1834` (`cursor_blinks` → `window.rs`), `:3205` (`inspector_lines` → `inspector.rs`); `src/renderer.rs:859` | `g.modes.cursor_shape` |
| `sync_output`, `sync_started` | `src/main.rs:1936` (`render_active` → `window.rs`); `src/pty.rs:118` (`feed_output`) | `g.modes.sync_output`, `g.modes.sync_started` |
| `mouse_sgr`, `mouse_sgr_pixels` | `src/main.rs:2200` (`report_mouse` → `pointer.rs`) | `g.modes.…` |
| `alt_scroll`, `app_cursor` | `src/main.rs:2247` (`alt_screen_scroll` → `pointer.rs`) | `g.modes.…` |
| `app_cursor`, `modify_other_keys` | `src/main.rs:2983,3058,3060` (`handle_key` → `keyboard.rs`), `:3266` (`handle_key_release` → `keyboard.rs`) | `g.modes.…` |
| `cursor_visible`, `app_cursor`, `bracketed_paste`, `sync_output`, `mouse_*`, `focus_events`, `alt_scroll`, `modify_other_keys` | `src/main.rs:3217-3233` (`inspector_lines` → `inspector.rs`) | `g.modes.…` |
| `color_scheme_reports` | `src/main.rs:3636` (`apply_system_appearance` → `window.rs`) | `….modes.color_scheme_reports` |

Unchanged for external callers: `kitty_keyboard_flags()` (still a `Grid` method), `prompt_epoch`, `last_exit`, `cell_px`, `title`, `bell`, `notification`, `progress`, `pointer_shape`, `clipboard_request`, `reported_cwd`, `response`, `dirty`, `all_dirty`, `display_offset`, `scroll_fraction`, `viewport_dirty`, `alt_active`, `graphics`, `defer_images`, `image_job`, `image_epoch`, all `pub` methods, and `src/app/latency_benchmarks.rs`'s calls to `seed_dead_clusters_for_bench` / `cluster_count` (they stay `Grid` methods). `src/atlas.rs` has its **own** `clusters` / `cluster_ids` fields — Task 14's rename must not touch it.

## How to Move an Item (used by Tasks 1–11)

Re-locate items before every cut (numbers shift after each move):

```bash
grep -nE '^    (pub(\((crate|super)\))? )?fn [a-z_0-9]+' src/grid/mod.rs          # methods
grep -nE '^(pub(\((crate|super)\))? )?(fn|static|const|struct|enum|impl|type|thread_local!)[ <(!]' src/grid/mod.rs   # top-level items
grep -nE '^    // [A-Z]' src/grid/mod.rs                                          # section banners
```

A method spans from its first preceding `///` / `#[` line through its closing `    }` at 4-space indent; a top-level item from its first `///` / `#[` line through its closing `}` at column 0 (or `;` for `static` / `const`). Cut and paste with your editor (not `sed -i`, which differs between BSD and GNU). Keep the moved items in their original relative order. When a whole banner section moves (e.g. `// Find in terminal`), move the banner comment too.

New file skeleton:

```rust
//! <one-line module doc given in the task>

use super::*;

impl Grid {
    // moved methods, private ones now `pub(super)`
}

// moved free functions / statics, private ones now `pub(super)`
```

Declare the module in `src/grid/mod.rs` right after the existing `pub mod selection;` line, keeping the list alphabetical among the new private modules:

```rust
pub mod cell;
pub mod perform;
pub mod prompt;
pub mod selection;

mod clusters;
mod colors;
// ...
```

After moving, review the diff as a move, not a rewrite:

```bash
git diff -M --color-moved=dimmed-zebra HEAD -- src/grid
```

Moved lines show dimmed. The only bright `+`/`-` lines allowed are: `pub(super)` prefixes, `use` / `mod` / `pub use` lines, the `impl Grid {` / `}` wrappers, and the module `//!` doc. Anything else must be explained in the commit body or undone.

---

### Task 0: Prepare branch, add the probe, record the baseline

**Files:**
- Create: `src/grid/perf_probe.rs`
- Modify: `src/grid/mod.rs` (one `mod` line)
- Create (untracked, under `target/`): `target/grid-gate/{probe.sh,stress.sh,gate.sh,probe-base,litty-base}`

**Interfaces:**
- Produces: test `grid::perf_probe::grid_throughput_probe` (`#[ignore]`, prints `GRID_SIZE <bytes>` and seven `GRID_PROBE <case> <ns-per-byte>` lines) and test `grid::perf_probe::steady_state_print_and_scroll_allocate_nothing`.

Why a new probe: `+bench-stress` drives `parser.advance(&mut grid, b)` directly and therefore skips `Grid::feed_byte` (the APC / OSC filter that `pty.rs` actually calls), and `app::latency_benchmarks::benchmark_before_vs_after` prints before/after numbers without asserting, mostly for non-grid code. Neither can gate this refactor alone.

- [ ] **Step 1: Make sure the working tree is clean and Phase 1 is merged**

```bash
git status --short
git log --oneline main | grep -c 'refactor(app)'
ls src/app/mod.rs
```

Expected: no `git status` output; a non-zero count; `src/app/mod.rs` exists. Pause any other work that edits `src/grid/` until Task 15 lands — nearly every line of `mod.rs` moves.

- [ ] **Step 2: Create the branch**

```bash
git switch main
git switch -c refactor/split-grid
```

- [ ] **Step 3: Record the test baseline**

```bash
cargo test 2>&1 | grep -E '^test result' > target/baseline-grid-tests.txt
cat target/baseline-grid-tests.txt
wc -l src/grid/mod.rs
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu
```

Expected: every line `0 failed`; `src/grid/mod.rs` is 5499 lines (if not, the file changed since this plan was written: re-check the item lists in each task with the grep commands above before starting). Clippy and the Linux check succeed; if not, fix that in a separate commit first.

- [ ] **Step 4: Write the probe and the allocation test**

`src/grid/perf_probe.rs`:

```rust
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
        for &b in data {
            grid.feed_byte(&mut parser, b);
        }
        let ns = start.elapsed().as_nanos() as f64 / data.len() as f64;
        std::hint::black_box(grid.scroll_position());
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

fn osc8_links() -> String {
    let mut s = String::new();
    for i in 0..20_000 {
        let _ = write!(s, "\x1b]8;;https://perf.test/{}\x1b\\link {i}\x1b]8;;\x1b\\\r\n", i % 400);
    }
    s
}

#[test]
#[ignore = "release-mode performance probe; run explicitly for refactor gates"]
fn grid_throughput_probe() {
    let cases: [Case; 7] = [
        ("ascii_scroll", ascii_scroll),
        ("sgr_log", sgr_log),
        ("unicode_mixed", unicode_mixed),
        ("emoji_clusters", emoji_clusters),
        ("region_scroll", region_scroll),
        ("alt_redraw", alt_redraw),
        ("osc8_links", osc8_links),
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
```

In `src/grid/mod.rs`, directly below `pub mod selection;`:

```rust
#[cfg(test)]
mod perf_probe;
```

- [ ] **Step 5: Run the new tests**

```bash
cargo test grid::perf_probe
cargo test --release grid::perf_probe -- --ignored --nocapture --test-threads=1
```

Expected: first command `1 passed; 0 failed; 1 ignored`. Second command prints `GRID_SIZE <n>` and seven `GRID_PROBE` lines (at the time of writing on Apple Silicon: ascii_scroll ≈ 17.8, sgr_log ≈ 16.4, unicode_mixed ≈ 19.0, emoji_clusters ≈ 23.7, region_scroll ≈ 146, alt_redraw ≈ 13.1, osc8_links ≈ 396 ns/byte; `GRID_SIZE 1464`). Note the `GRID_SIZE` value as `S0`.

- [ ] **Step 6: Record the structural baseline**

Run "V". Record: `M` = `passed` total, `I` = `ignored` total (both one higher than in Step 3), `F` = fn count (281 at the time of writing), `T` = `#[test]` count (92 at the time of writing). The `#[inline` count must be 23.

- [ ] **Step 7: Commit the probe**

```bash
git add src/grid/perf_probe.rs src/grid/mod.rs
git commit -m "test(grid): add throughput probe and steady-state allocation test"
```

Commit body: "Adds a release-mode feed_byte throughput probe (ignored by default) and a zero-allocation test for steady-state printing, as the performance gate for the grid module split (plans/2026-11-01-split-grid.md)."

- [ ] **Step 8: Create the gate scripts**

```bash
mkdir -p target/grid-gate
cat > target/grid-gate/probe.sh <<'EOF'
#!/usr/bin/env bash
# Compare two grid perf-probe test binaries in interleaved rounds.
# Usage: probe.sh <base-test-binary> <new-test-binary> [rounds]   (env TOL = percent, default 3)
set -euo pipefail
base=$1; new=$2; rounds=${3:-3}; tol=${TOL:-3}
out=$(mktemp -d)
run() { "$1" grid::perf_probe::grid_throughput_probe --ignored --nocapture --test-threads=1 2>/dev/null | awk '$1 == "GRID_PROBE" { print $2, $3 }'; }
for _ in $(seq "$rounds"); do
  run "$base" >> "$out/base"
  run "$new" >> "$out/new"
done
best() { sort -k1,1 -k2,2g "$1" | awk '!seen[$1]++'; }
best "$out/base" > "$out/base.best"
best "$out/new" > "$out/new.best"
join "$out/base.best" "$out/new.best" | awk -v tol="$tol" '
  { d = ($3 - $2) / $2 * 100; flag = ""
    if (d > tol) { flag = "  FAIL (slower)"; bad = 1 } else if (d < -tol) { flag = "  CHECK (faster)" }
    printf "%-15s base %9.3f  new %9.3f ns/B  %+6.2f%%%s\n", $1, $2, $3, d, flag; n++ }
  END { if (n != 7) { print "expected 7 probe cases, got " n; exit 1 } exit bad }'
EOF
cat > target/grid-gate/stress.sh <<'EOF'
#!/usr/bin/env bash
# Compare `+bench-stress` parser throughput of two release binaries, interleaved.
# Usage: stress.sh <base-litty> <new-litty> [rounds]   (env TOL = percent, default 3)
set -euo pipefail
base=$1; new=$2; rounds=${3:-3}; tol=${TOL:-3}
tp() { { "$1" +bench-stress 200000 || true; } | perl -pe 's/\e\[[0-9;]*m//g' | awk '$2 == "Throughput" { print $4 }'; }
b=0; n=0
for _ in $(seq "$rounds"); do
  b=$(printf '%s\n%s\n' "$b" "$(tp "$base")" | sort -g | tail -1)
  n=$(printf '%s\n%s\n' "$n" "$(tp "$new")" | sort -g | tail -1)
done
awk -v b="$b" -v n="$n" -v tol="$tol" 'BEGIN {
  d = (n - b) / b * 100
  printf "bench-stress    base %9.0f  new %9.0f lines/s  %+6.2f%%%s\n", b, n, d, (d < -tol ? "  FAIL (slower)" : "")
  exit (d < -tol) }'
EOF
cat > target/grid-gate/gate.sh <<'EOF'
#!/usr/bin/env bash
# Rebuild release binaries and compare them with the Task 0 baseline.
# Usage: gate.sh <rounds> <tolerance-percent>
set -euo pipefail
rounds=${1:-3}; export TOL=${2:-3}
cargo build --release
new_probe=$(cargo test --release --no-run 2>&1 | grep -oE '[^ (]*target/release/deps/litty-[0-9a-f]+' | sed -n 1p)
target/grid-gate/probe.sh target/grid-gate/probe-base "$new_probe" "$rounds"
target/grid-gate/stress.sh target/grid-gate/litty-base target/release/litty "$rounds"
"$new_probe" grid::perf_probe::grid_throughput_probe --ignored --nocapture --test-threads=1 2>/dev/null | grep -oE 'GRID_SIZE [0-9]+'
EOF
chmod +x target/grid-gate/*.sh
```

- [ ] **Step 9: Build and keep the baseline binaries**

```bash
cargo build --release
cp target/release/litty target/grid-gate/litty-base
cp "$(cargo test --release --no-run 2>&1 | grep -oE '[^ (]*target/release/deps/litty-[0-9a-f]+' | sed -n 1p)" target/grid-gate/probe-base
target/grid-gate/gate.sh 1 3
```

Expected: seven probe lines all within ±1 % (same code on both sides), one `bench-stress` line within ±1 %, and `GRID_SIZE S0`. This proves the scripts work. Keep `target/grid-gate/` until Task 16; never `cargo clean` during this phase (it deletes the baseline).

- [ ] **Step 10: Record the latency SLA baseline**

```bash
cargo run --release -- +bench-perf > target/grid-gate/bench-perf-base.txt 2>&1; echo "exit=$?"
```

Expected: `exit=0` (all SLAs pass). This is compared again in Task 16.

---

### Task 1: Move the unit tests to `grid/tests.rs`

**Files:**
- Create: `src/grid/tests.rs`
- Modify: `src/grid/mod.rs`

**Interfaces:**
- Produces: `#[cfg(test)] mod tests;` — test paths stay `grid::tests::<name>`, so test filters in scripts keep working.

- [ ] **Step 1: Move the module body**

Cut everything from `#[cfg(test)]` / `mod tests {` (≈3722-3723) through the final `}` of the file (≈5498). Paste the **body** (from `use super::*;` through the last test, without the outer `mod tests { ... }` braces) into `src/grid/tests.rs`, de-indented by four spaces, under:

```rust
//! Unit tests for the grid: parser behavior, scrollback ring, selection,
//! clusters, images, prompts and the regressions recorded in ROADMAP.md.
```

De-indent with your editor's block-outdent (every line inside the old module starts with at least four spaces or is empty). The `#[cfg(target_os = "macos")]` test (≈4271) and the `#[ignore = "manual performance measurement"]` test keep their attributes.

At the end of `src/grid/mod.rs` add:

```rust
#[cfg(test)]
mod tests;
```

- [ ] **Step 2: Verify** — run "V". Expected: `passed` == `M`, `ignored` == `I`, counts `23` / `F` / `T`. Also:

```bash
wc -l src/grid/mod.rs src/grid/tests.rs
```

Expected: `mod.rs` ≈ 3725, `tests.rs` ≈ 1778.

No gate (test-only change).

- [ ] **Step 3: Commit**

```bash
git add src/grid/tests.rs src/grid/mod.rs
git commit -m "refactor(grid): move unit tests into grid/tests.rs"
```

---

### Task 2: Extract `grid::colors`

**Files:**
- Create: `src/grid/colors.rs`
- Modify: `src/grid/mod.rs`, `src/grid/tests.rs` (imports only)

**Interfaces:**
- Produces (re-exported from `grid`, signatures unchanged): `pub fn set_color_scheme_dark(dark: bool)`, `pub fn color_scheme_is_dark() -> bool`, `pub fn color_scheme_report() -> &'static [u8]`, `pub fn set_default_colors(fg: u32, bg: u32)`, `pub fn set_palette(pal: &[u32; 16])`. Also stays `pub` but is only used inside `grid`: `default_fg`, `default_bg`.
- Produces (crate-internal, `pub(super)`): `fn ext_color`, `fn rgb`, `fn palette`, `fn palette_with`, `fn parse_x11_color`; methods `sgr`, `osc4`, `osc104`, `recolor_pairs`, `osc_dynamic_color`, `reply_color`. `pub fn fg_color`, `bg_color`, `cursor_color` stay `pub`.

- [ ] **Step 1: Move the globals** (top of `mod.rs`)

`DEFAULT_FG_RGB` + `DEFAULT_BG_RGB` with their shared doc comment (≈50-54), `default_fg` (≈56), `default_bg` (≈61), `COLOR_SCHEME_DARK` (≈66), `set_color_scheme_dark` (≈69), `color_scheme_is_dark` (≈73), `color_scheme_report` (≈77), `set_default_colors` (≈86), `PALETTE_BASE` (≈91), `set_palette` (≈111). Delete the run of four blank lines that follows `set_palette`.

- [ ] **Step 2: Move the methods** into `impl Grid` in `colors.rs`

The `// SGR` banner and `sgr` (≈2787-2855), `fg_color` (≈2856), `bg_color` (≈2864), `cursor_color` (≈2872), `osc4` (≈2877), `osc104` (≈2911), `recolor_pairs` (≈2945), `osc_dynamic_color` (≈3002), `reply_color` (≈3019). **Do not move** `colors(&self) -> (u32, u32)` (≈2452): `put_char` calls it for every printed character.

- [ ] **Step 3: Move the free functions**

`ext_color` (≈3413), `rgb` (≈3438, keep `#[inline]`), `palette` (≈3443), `palette_with` (≈3513, keep `#[inline]`), `parse_x11_color` (≈3525).

Header:

```rust
//! Colors: configured defaults and 16-color palette (process-wide atomics),
//! SGR pen colors, OSC 4 / 104 palette overrides and OSC 10 / 11 / 12
//! dynamic colors with their xterm-style replies.
```

- [ ] **Step 4: Wire it up**

In `src/grid/mod.rs`:

```rust
mod colors;

pub use colors::{color_scheme_is_dark, color_scheme_report, set_color_scheme_dark, set_default_colors, set_palette};
use colors::palette_with;
```

(`palette_with` is still used by `placeholder_ids` until Task 4.) Then `cargo check`; for every unresolved name add the import in the file that needs it (`images` code in mod.rs needs `palette_with`; tests need `default_fg`, `default_bg`, `rgb`, `parse_x11_color`, `palette` — add `use super::colors::{...};` at the top of `tests.rs` with exactly the names the compiler lists). Prune unused imports with clippy (`AtomicU32` leaves mod.rs).

- [ ] **Step 5: Verify** — "V" (all counts unchanged), then G-std:

```bash
target/grid-gate/gate.sh 3 3
```

Expected: no `FAIL`, `GRID_SIZE S0` (no field moved).

- [ ] **Step 6: Commit**

```bash
git add src/grid/colors.rs src/grid/mod.rs src/grid/tests.rs
git commit -m "refactor(grid): move colors and palette into grid/colors.rs"
```

---

### Task 3: Extract `grid::control`

**Files:**
- Create: `src/grid/control.rs`
- Modify: `src/grid/mod.rs`, `src/grid/perform.rs` (imports only), `src/grid/tests.rs` (imports only)

**Interfaces:**
- Produces (`pub(super)`): `enum DcsRequest { XtGetTcap, Decrqss, Sixel(bool) }`, `const MAX_OSC_BYTES: usize`, `const MAX_IMAGE_STRING: usize`, `fn decode_hex(hex: &[u8]) -> Option<String>`, `fn termcap(name: &str) -> Option<&'static str>`, `fn lossy_join(parts: &[&[u8]], max: usize) -> String`, `fn parse_osc7_path(payload: &[u8]) -> Option<String>`, `fn decode_base64(data: &[u8]) -> Option<Vec<u8>>`; methods `reply_termcap`, `reply_decrqss`, `sgr_report`.

The OSC / DCS / APC collection **fields** (`osc_len`, `osc_discard`, `osc_start`, `dcs`, `dcs_buf`, `dcs_start`, `apc_state`, `apc_buf`, `apc_start`) stay on `Grid`: `feed_byte` reads them for every byte. `feed_byte` itself stays in mod.rs. `apc_dispatch` goes to `images.rs` (Task 4) because it only parses Kitty commands and queues image jobs.

- [ ] **Step 1: Move**

Methods: `reply_termcap` (≈3025), `reply_decrqss` (≈3049), `sgr_report` (≈3064). Top-level: `MAX_OSC_BYTES` with its `//` comment (≈154-155), `DcsRequest` (≈3460), `MAX_IMAGE_STRING` (≈3468), `decode_hex` (≈3471), `termcap` (≈3483), `lossy_join` (≈3507), `parse_osc7_path` (≈3629), `decode_base64` (≈3664).

Header:

```rust
//! Control strings: DCS requests (XTGETTCAP, DECRQSS) and their replies,
//! OSC payload decoders, and the size limits for OSC / DCS / APC strings.
```

- [ ] **Step 2: Wire it up**

`src/grid/mod.rs`:

```rust
mod control;

use control::{DcsRequest, MAX_IMAGE_STRING, MAX_OSC_BYTES};
```

`src/grid/perform.rs`, below `use super::*;`:

```rust
use super::control::{decode_base64, lossy_join, parse_osc7_path};
```

`cargo check`; add what the compiler lists (`tests.rs` uses `MAX_OSC_BYTES` through `use super::*`, which works because mod.rs imports it). Prune with clippy.

- [ ] **Step 3: Verify** — "V", then `target/grid-gate/gate.sh 3 3`. Expected: no `FAIL`, `GRID_SIZE S0`.

- [ ] **Step 4: Commit**

```bash
git add src/grid/control.rs src/grid/mod.rs src/grid/perform.rs src/grid/tests.rs
git commit -m "refactor(grid): move dcs replies and osc decoders into grid/control.rs"
```

(Stage only files `git status` shows as modified.)

---

### Task 4: Extract `grid::images`

**Files:**
- Create: `src/grid/images.rs`
- Modify: `src/grid/mod.rs`

**Interfaces:**
- Produces (re-exported `pub(crate)`, unchanged): `enum ImageJob { Sixel(Vec<u8>, bool), Kitty(KittyCommand, Vec<u8>) }`, `enum ImageCompletion`, `fn ImageJob::decode(self) -> ImageCompletion` — `src/pty.rs` keeps using `crate::grid::{ImageJob, ImageCompletion}`.
- Methods: `pub(crate) fn complete_image`, `pub fn prune_dead_virtuals`, `pub fn next_image_frame`, `pub fn visible_images` (unchanged); `pub(super)`: `apc_dispatch`, `queue_image`, `apply_kitty`, `place_image`, `image_cell_px`, `place_virtual`, `placeholder_ids`, `placeholder_images`, `placeholder_draw`.

- [ ] **Step 1: Move**

Top-level: `ImageJob` with its doc (≈129-135), `ImageCompletion` (≈137), `impl ImageJob` (≈142). Methods (≈2158-2433, contiguous): `apc_dispatch`, `queue_image`, `complete_image`, `apply_kitty`, `place_image` (keep `#[allow(clippy::too_many_arguments)]`), `image_cell_px`, `place_virtual`, `prune_dead_virtuals`, `next_image_frame`, `visible_images`, `placeholder_ids`, `placeholder_images`, `placeholder_draw`.

Header:

```rust
//! Inline images: Kitty graphics APC dispatch, Sixel completion, placements,
//! Unicode placeholders (virtual placements) and per-frame visible draws.
//! Decoding runs off the grid lock (`ImageJob::decode`, see `pty::feed_output`).
```

- [ ] **Step 2: Wire it up**

```rust
mod images;

pub(crate) use images::{ImageCompletion, ImageJob};
```

`images.rs` needs `use super::colors::palette_with;`; remove `use colors::palette_with;` from mod.rs if clippy now reports it unused. `cargo check`, prune.

- [ ] **Step 3: Verify** — "V", then `target/grid-gate/gate.sh 3 3`. Also confirm the PTY image path is untouched:

```bash
git diff HEAD --stat -- src/pty.rs
```

Expected: no output. Gate: no `FAIL`, `GRID_SIZE S0`.

- [ ] **Step 4: Commit**

```bash
git add src/grid/images.rs src/grid/mod.rs
git commit -m "refactor(grid): move image placement into grid/images.rs"
```

---

### Task 5: Move prompt helpers into `grid::prompt`

**Files:**
- Modify: `src/grid/prompt.rs` (existing, 13 lines: `MAX_PROMPTS`, `PromptMark`), `src/grid/mod.rs`

**Interfaces:**
- Methods (unchanged `pub`): `prompt_ready`, `full_prompt_input`, `extract_scrollback_lines`, `current_prompt_input`, `prompt_command_range`, `calculate_cursor_movement_to`, `calculate_delete_before_cursor`, `calculate_delete_after_cursor` (keep `#[allow(dead_code)]`), `jump_to_prompt`, `select_last_output`; `pub(super)`: `guess_command_start`, `cursor_line`, `record_prompt_mark`.

The `prompts` and `prompt_input_mark` fields stay on `Grid` (resize reflow remaps them; `in_line_editor`, which stays in mod.rs because `char_cell` calls it per wide pictograph, reads `prompt_input_mark`). `first_line` and `scroll_to_line` move with the viewport in Task 8.

- [ ] **Step 1: Move**

From mod.rs into a new `impl Grid` block at the end of `prompt.rs`: `prompt_ready` (≈512) through `calculate_delete_after_cursor` (≈803-819, contiguous), then `cursor_line` (≈3090), `record_prompt_mark` (≈3095), `jump_to_prompt` (≈3140), `select_last_output` (≈3164). Add `use super::*;` below the existing `//!` line. Extend the module doc:

```rust
//! Shell prompt mark tracking for OSC 133 semantic prompt jump and output selection,
//! and the prompt-input helpers used by the AI bridge and click-to-move-cursor.
```

`prompt.rs` is glob re-exported (`pub use prompt::*;`) — its `use super::*;` is private and does not leak back.

- [ ] **Step 2: Verify** — "V", then `target/grid-gate/gate.sh 3 3`. Expected: no `FAIL`, `GRID_SIZE S0`.

- [ ] **Step 3: Commit**

```bash
git add src/grid/prompt.rs src/grid/mod.rs
git commit -m "refactor(grid): move prompt helpers into grid/prompt.rs"
```

---

### Task 6: Move selection and search into `grid::selection`

**Files:**
- Modify: `src/grid/selection.rs` (existing, 116 lines: `Selection`, `SearchHits`, `SearchHit`, `Hyperlink`, `HyperlinkCandidate`), `src/grid/mod.rs`

**Interfaces:**
- Methods (unchanged `pub`): `set_selection`, `select_all`, `extend_selection`, `set_selection_end`, `is_selected` (keep `#[inline]` — the renderer calls it per cell), `select_smart_line`, `select_block`, `is_block_selected`, `select_line`, `selected_text`, `search_text`, `set_search_matches`, `clear_search`, `search_step`, `search_status`, `search_hits`; `pub(super)`: `is_blank` (keep `#[inline]`), `trim_col_range` (keep `#[inline]`), `block_range`, `reveal_search_current`.

- [ ] **Step 1: Move**

From mod.rs into an `impl Grid` block at the end of `selection.rs`: `set_selection` (≈918) through `is_selected` (≈971-979); skip `is_path_char` / `find_hyperlink_candidate_at` / `find_hyperlink_at` (≈980-1100, Task 7); then `is_blank` (≈1102) through `selected_text` (≈1268-1307); then the `// Find in terminal` banner and `search_text` (≈1309) through `search_hits` (≈1399-1408). `is_blank` and `trim_col_range` are associated functions called as `Self::is_blank(...)`; that still resolves from any `impl Grid` block. Add `use super::*;` to `selection.rs` and extend its doc:

```rust
//! Grid selection, hyperlink, and search hits representations, and the `Grid`
//! methods that select text (click, word, line, block) and find in terminal.
```

- [ ] **Step 2: Verify** — "V", then `target/grid-gate/gate.sh 3 3`. Expected: no `FAIL`, `GRID_SIZE S0`.

- [ ] **Step 3: Commit**

```bash
git add src/grid/selection.rs src/grid/mod.rs
git commit -m "refactor(grid): move selection and search into grid/selection.rs"
```

---

### Task 7: Extract `grid::links`

**Files:**
- Create: `src/grid/links.rs`
- Modify: `src/grid/mod.rs`, `src/grid/tests.rs` (imports only)

**Interfaces:**
- Methods (unchanged `pub`): `find_hyperlink_candidate_at`, `find_hyperlink_at`, `underline_color`, `describe_cell`; `pub(super)`: `is_path_char` (associated fn, keep `#[inline]`), `extra_at`, `set_underline_color`, `set_link`, `intern_extra`, `collect_extras`, `trim_extras`, `cell_link`, `osc8`.
- Free (`pub(super)`): `fn osc8_target(uri: &str) -> Option<(String, bool)>`, `fn trim_trailing_punctuation(chars: &[char], start: usize, end: usize) -> usize`.

The `extras`, `extras_dirty`, `extras_full` fields stay on `Grid` (`put_char`, `clear_cols`, `copy_row`, `scroll_up_in` set `extras_dirty` on the hot path).

- [ ] **Step 1: Move**

Methods: `is_path_char` (≈980), `find_hyperlink_candidate_at` (≈994), `find_hyperlink_at` (≈1093); the `// Extra cell attributes` banner (≈3185) and `extra_at` (≈3189) through `osc8` (≈3392-3410), **except** `kitty_keyboard_flags` (≈3180, Task 13; it sits just above the banner). Free: `osc8_target` (≈3654), `trim_trailing_punctuation` (≈3692).

Header:

```rust
//! Hyperlinks and cell extras: implicit URL / path detection under the mouse,
//! OSC 8 explicit links, SGR 58 underline colors, the interned `CellExtra`
//! table and its reclamation, and the inspector's cell description.
```

- [ ] **Step 2: Wire it up** — `mod links;` in mod.rs; `cargo check`; tests calling `osc8_target` get `use super::links::osc8_target;` in `tests.rs`. Prune.

- [ ] **Step 3: Verify** — "V", then `target/grid-gate/gate.sh 3 3` (watch `osc8_links`). Expected: no `FAIL`, `GRID_SIZE S0`.

- [ ] **Step 4: Commit**

```bash
git add src/grid/links.rs src/grid/mod.rs src/grid/tests.rs
git commit -m "refactor(grid): move hyperlinks and cell extras into grid/links.rs"
```

---

### Task 8: Extract `grid::viewport` and `grid::resize`

**Files:**
- Create: `src/grid/viewport.rs`, `src/grid/resize.rs`
- Modify: `src/grid/mod.rs`

**Interfaces:**
- `viewport.rs` methods (unchanged `pub`): `scroll_display`, `scroll_pixels`, `viewport_row`, `reset_display`, `clear_history`, `clear_screen_and_history`, `scroll_position`; `pub(super)`: `first_line`, `scroll_to_line`.
- `resize.rs`: `pub fn resize(&mut self, cols: usize, rows: usize)` (unchanged).

- [ ] **Step 1: `viewport.rs`**

Move `scroll_display` (≈819) through `clear_screen_and_history` (≈892-916, contiguous), `scroll_position` (≈3085), `first_line` (≈3124), `scroll_to_line` (≈3129). Header:

```rust
//! The viewport over scrollback: whole-row and pixel-precise scrolling,
//! hit-testing rows, and clearing history / the screen (Cmd+K).
```

- [ ] **Step 2: `resize.rs`**

Move the `// Resize (the only place the grid allocates after startup)` banner and `resize` (≈2591-2786). Header:

```rust
//! Window resize: reflow main screen + scrollback to the new width, keep the
//! cursor on screen, remap prompt marks and image placements. The only place
//! the grid allocates after startup.
```

- [ ] **Step 3: Wire up** — add `mod resize; mod viewport;` to mod.rs, then `cargo check`. `resize` calls `scrollback_lines`, `default_tab_stops`, `is_wide_cp`, `first_line`, `clamp_cursor`, `clear_search` and `prune_dead_virtuals`; all are either private items of mod.rs (visible through `use super::*`) or methods made `pub(super)` by earlier tasks, so no extra import is expected. Prune with clippy.

- [ ] **Step 4: Verify** — "V", then `target/grid-gate/gate.sh 3 3`. Expected: no `FAIL`, `GRID_SIZE S0`. Also run the reflow tests explicitly:

```bash
cargo test grid::tests:: 2>&1 | grep -E 'resize|reflow' | head
```

Expected: every listed test `ok`.

- [ ] **Step 5: Commit**

```bash
git add src/grid/viewport.rs src/grid/resize.rs src/grid/mod.rs
git commit -m "refactor(grid): move viewport scrolling and resize reflow into modules"
```

---

### Task 9: Extract `grid::width` and `grid::visual` (hot free functions)

**Files:**
- Create: `src/grid/width.rs`, `src/grid/visual.rs`
- Modify: `src/grid/mod.rs`, `src/grid/tests.rs` (imports only)

**Interfaces:**
- `width.rs` (re-exported, unchanged): `pub fn char_width(c: char) -> usize` (used by `renderer.rs`, `bidi.rs`), `#[inline] pub fn is_wide_cp(cp: u32) -> bool` (used by `renderer.rs`, `put_char`); `pub(super)`: `fn is_pictographic(cp: u32) -> bool`, `const FIRST_WIDE: u32`; macOS-only `pub(super)`: `fn is_spacing_mark_range`, `fn macos_wcwidth` (both keep `#[cfg(target_os = "macos")]`).
- `visual.rs`: `thread_local! BIDI_SCRATCH`; methods (unchanged `pub`) `bidi_order`, `logical_col`, `cursor_visual_col`.

- [ ] **Step 1: `width.rs`**

Move `char_width` (≈3557), `is_spacing_mark_range` (≈3574), `macos_wcwidth` (≈3582), `FIRST_WIDE` (≈3603), `is_wide_cp` (≈3606), `is_pictographic` (≈3622). Header:

```rust
//! Display width of characters (Unicode East Asian Width + emoji presentation,
//! macOS `wcwidth` for spacing marks) and the per-cell wide check used while printing.
```

The Linux build has no `is_spacing_mark_range` / `macos_wcwidth`; `char_width`'s `#[cfg(target_os = "macos")]` block keeps both targets compiling — `cargo check --target x86_64-unknown-linux-gnu` proves it.

- [ ] **Step 2: `visual.rs`**

Move the `thread_local!` `BIDI_SCRATCH` (≈313-316), `bidi_order` (≈1425), `logical_col` (≈1454), `cursor_visual_col` (≈1469). Header:

```rust
//! Bidirectional text: visual order of a display row and logical ↔ visual
//! column mapping for the renderer, mouse and cursor (scratch buffers are
//! thread-local, so nothing allocates per frame).
```

- [ ] **Step 3: Wire up**

```rust
mod visual;
mod width;

pub use width::{char_width, is_wide_cp};
use width::is_pictographic;
```

`cargo check`; tests may need nothing extra (they use `char_width` / `is_wide_cp` via the re-export). Prune.

- [ ] **Step 4: Verify** — "V", then G-hot:

```bash
target/grid-gate/gate.sh 5 3
```

Expected: no `FAIL` — watch `unicode_mixed` and `emoji_clusters` (they call `char_width` / `is_wide_cp` per character); `GRID_SIZE S0`. `git diff HEAD -- src/grid | grep -c '^[-+].*#\[inline'` must be even (every removed attribute re-added).

- [ ] **Step 5: Commit**

```bash
git add src/grid/width.rs src/grid/visual.rs src/grid/mod.rs src/grid/tests.rs
git commit -m "refactor(grid): move unicode width and bidi helpers into modules"
```

---

### Task 10: Move grapheme cluster code into `grid::clusters`

**Files:**
- Create: `src/grid/clusters.rs`
- Modify: `src/grid/mod.rs`

**Interfaces:**
- Methods (unchanged): `pub fn cluster(&self, cp: u32) -> Option<&str>`, `pub fn cell_char(&self, cell: &Cell) -> char`, `pub fn push_cell_text(&self, cell: &Cell, out: &mut String)`, `pub(crate) fn gc_clusters(&mut self)`, `#[cfg(test)] pub(crate) fn cluster_count(&self) -> usize`, `#[cfg(test)] pub(crate) fn seed_dead_clusters_for_bench(&mut self, count: usize)` (the last two are called from `src/app/latency_benchmarks.rs`); `pub(super) fn intern_cluster(&mut self, text: &str) -> Option<u32>`.

`append_to_cell`, `wide_pictograph`, `extend_grapheme`, `combine_with_previous` stay in mod.rs: they are the print path, and they mutate cursor / wrap state, not just the table.

- [ ] **Step 1: Move**

`cluster` (≈1493) through `gc_clusters` (≈1561-1671, contiguous: `cluster`, `cell_char`, `push_cell_text`, `intern_cluster`, `cluster_count`, `seed_dead_clusters_for_bench`, `gc_clusters`). Header:

```rust
//! Grapheme clusters: multi-codepoint graphemes are interned once and cells
//! store `CLUSTER_FLAG | id`. Unreferenced IDs are collected when the table
//! fills or after a history-sized batch of evictions, remapping live cells.
```

- [ ] **Step 2: Wire up** — `mod clusters;`; `cargo check`; prune.

- [ ] **Step 3: Verify** — "V", then `target/grid-gate/gate.sh 5 3`. Expected: no `FAIL` (watch `emoji_clusters`, `unicode_mixed`), `GRID_SIZE S0`. Also:

```bash
cargo test --release cluster -- --test-threads=1
```

Expected: all `ok` (includes `cluster_eviction_gc_is_amortized_and_preserves_live_text`, which asserts zero allocations while scrolling).

- [ ] **Step 4: Commit**

```bash
git add src/grid/clusters.rs src/grid/mod.rs
git commit -m "refactor(grid): move grapheme cluster table code into grid/clusters.rs"
```

---

### Task 11: Move ring addressing and scrolling into `grid::storage`

**Files:**
- Create: `src/grid/storage.rs`
- Modify: `src/grid/mod.rs`, `src/grid/tests.rs` (imports only)

**Interfaces:**
- Re-exported (unchanged): `pub const SCROLLBACK: usize = 5000` (only if clippy does not flag the re-export as unused; otherwise `tests.rs` imports it with `use super::storage::SCROLLBACK;`), `pub fn set_scrollback_limit(bytes: Option<usize>)` (used by app config reload).
- `pub(super)`: `const MAX_SCROLLBACK`, `static SCROLLBACK_BYTES`, `fn scrollback_lines(cols: usize) -> usize`, `fn lines_for_bytes(bytes: usize, cols: usize) -> usize`.
- Methods — keep every `#[inline]` exactly: `#[inline] pub(super) fn row_start`, `#[inline] pub(super) fn buf_mut`, `#[inline] pub(super) fn row_mut`, `#[inline] pub fn display_row`, `#[inline] pub(super) fn abs_line`, `pub(super) fn line_cells`, `pub(super) fn line_wrapped`, `#[inline] pub(super) fn set_wrapped`, `pub(super) fn clear_cols`, `pub(super) fn clear_rows`, `pub(super) fn copy_row`, `pub(super) fn grow_capacity`, `pub(super) fn scroll_up_in`, `pub(super) fn scroll_down_in`.

`linefeed` / `reverse_index` (callers of `scroll_up_in` / `scroll_down_in`), `mark`, `blank`, and `put_char` stay in mod.rs.

- [ ] **Step 1: Move**

Top-level: `SCROLLBACK` (≈25-26), `MAX_SCROLLBACK` (≈27-28), `SCROLLBACK_BYTES` (≈30-32), `set_scrollback_limit` (≈34), `scrollback_lines` (≈38), `lines_for_bytes` (≈43). Methods: the `// Row addressing` banner and `row_start` (≈420) through `set_wrapped` (≈495-501, contiguous), then `clear_cols` (≈1905) through `scroll_down_in` (≈2033-2045, contiguous). **Not** `cursor_pos` (≈503), which sits between them and stays in mod.rs.

Header:

```rust
//! Scrollback storage: one flat zero-initialized `Vec<Cell>` ring of
//! `cols * cap` cells, grown on demand up to `scrollback + rows` rows.
//! Full-screen scrolling is O(1) (advance `top`, clear one row); nothing here
//! allocates except `grow_capacity`.
```

- [ ] **Step 2: Wire up**

```rust
mod storage;

pub use storage::set_scrollback_limit;
use storage::scrollback_lines;
```

`resize.rs` needs `use super::storage::scrollback_lines;` if mod.rs no longer uses it (it does — `Grid::new`), so likely nothing. Tests using `lines_for_bytes` / `SCROLLBACK` get explicit `use super::storage::{...};`. Prune.

- [ ] **Step 3: Verify** — "V", then G-hot:

```bash
target/grid-gate/gate.sh 5 3
```

Expected: no `FAIL` — watch `ascii_scroll`, `region_scroll`, `alt_redraw` (row addressing on every print) — and `GRID_SIZE S0`. Check the inline attributes made the trip:

```bash
grep -B1 -nE 'fn (row_start|buf_mut|row_mut|display_row|abs_line|set_wrapped)\b' src/grid/storage.rs | grep -c '#\[inline\]'
```

Expected: `6`.

- [ ] **Step 4: Commit**

```bash
git add src/grid/storage.rs src/grid/mod.rs src/grid/tests.rs
git commit -m "refactor(grid): move ring addressing and scrolling into storage.rs"
```

After this task, `wc -l src/grid/mod.rs` ≈ 860; everything left is the struct, `new`, the print path, `feed_byte`, cursor motion, `set_alt`, `full_reset`, `default_tab_stops`, `dec_line_drawing`, `GRAPHEME_DEFAULT`, `kitty_keyboard_flags`.

---

### Task 12: Group AI bridge fields in `AiTrack`

**Files:**
- Modify: `src/grid/prompt.rs` (new struct), `src/grid/mod.rs` (field + initializer), `src/grid/perform.rs`, `src/grid/tests.rs`, and the app files listed in "External call sites" (`src/app/ai_bridge.rs`, `src/app/keyboard.rs` after Phase 1)

**Interfaces:**
- Produces in `src/grid/prompt.rs` (glob re-exported as `crate::grid::AiTrack`):

```rust
/// Shell-integration state for the AI bridge (OSC 778 `init` / `yo`, OSC 133 `C`).
/// The grid only records it; `crate::ai` and the app decide what it means.
#[derive(Default)]
pub struct AiTrack {
    /// AI bridge is armed only by a user submission at a known shell prompt.
    pub available: bool,
    pub direct: bool,
    pub armed: bool,
    pub track: bool,
    pub command: Option<String>,
    pub request: Option<String>,
}
```

- In `Grid`, replace the six fields `ai_available` … `ai_request` (≈164-170, including the doc comment above `ai_available`, which moved onto `available`) with:

```rust
    /// AI bridge state (see `AiTrack`).
    pub ai: AiTrack,
```

`prompt_epoch` and `last_exit` stay on `Grid` (OSC 133 data also read outside the AI path: `crate::ai::mod.rs:265` reads `last_exit`).

- [ ] **Step 1: Add the struct and replace the fields** as above.

- [ ] **Step 2: Replace the initializer** in `Grid::new` — the six lines `ai_available: false,` … `ai_request: None,` become `ai: AiTrack::default(),` (every derived default equals the old literal: `false` / `None`).

- [ ] **Step 3: Rename accesses**

```bash
files=$(grep -rlE '\.ai_(available|direct|armed|track|command|request)\b' src --include='*.rs')
echo "$files"
perl -pi -e 's/\.ai_(available|direct|armed|track|command|request)\b/.ai.$1/g' $files
cargo check
```

Expected `files`: `src/grid/perform.rs`, `src/grid/tests.rs`, `src/app/ai_bridge.rs`, `src/app/keyboard.rs` (or `src/main.rs` if the Phase 1 locations differ). `cargo check` clean.

- [ ] **Step 4: Review the external diff**

```bash
git diff --stat
git diff -- src/app | grep '^[-+]' | grep -v '\.ai\.' | grep -vE '^(\+\+\+|---)'
```

Expected: second command prints nothing (every changed app line is an `ai_*` → `ai.*` rename).

- [ ] **Step 5: Verify** — "V", then `target/grid-gate/gate.sh 3 3`. Expected: no `FAIL`; `GRID_SIZE` may differ from `S0` by padding only (≤ 8 bytes); record the new value as `S1`.

- [ ] **Step 6: Commit**

```bash
git add src/grid/prompt.rs src/grid/mod.rs src/grid/perform.rs src/grid/tests.rs src/app/ai_bridge.rs src/app/keyboard.rs
git commit -m "refactor(grid): group ai bridge fields in AiTrack"
```

(Stage exactly the files from Step 3's list.)

---

### Task 13: Group terminal mode flags in `TerminalModes`

**Files:**
- Create: `src/grid/modes.rs`
- Modify: `src/grid/mod.rs`, `src/grid/perform.rs`, `src/grid/tests.rs`, `src/renderer.rs`, `src/pty.rs`, and the app files from "External call sites" (`window_event.rs`, `ai_bridge.rs`, `window.rs`, `pointer.rs`, `keyboard.rs`, `inspector.rs`, `search_ui.rs`, `actions.rs` after Phase 1)

**Interfaces:**
- Produces in `src/grid/modes.rs` (re-exported `pub use modes::{set_grapheme_default, TerminalModes};`):

```rust
//! Terminal modes set by the application (DECSET / DECRST, IRM, DECSCUSR,
//! XTMODKEYS, Kitty keyboard, DEC 2027), reset together by RIS.

use super::*;

/// `grapheme-width-method = unicode`: new terminals start in DEC mode 2027.
pub(super) static GRAPHEME_DEFAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Set the initial state of DEC mode 2027 (grapheme clustering) for new terminals.
pub fn set_grapheme_default(on: bool) {
    GRAPHEME_DEFAULT.store(on, Ordering::Relaxed);
}

/// Mode flags of one terminal. Field docs are the old `Grid` field docs.
pub struct TerminalModes {
    pub cursor_visible: bool,
    /// Last DECSCUSR (`CSI Ps SP q`) value: 0 = config default,
    /// 1/2 = blinking/steady block, 3/4 = underline, 5/6 = bar.
    pub cursor_shape: u8,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub sync_output: bool,
    /// When synchronized output (DECSET 2026) began; rendering is deferred meanwhile.
    pub sync_started: Option<std::time::Instant>,
    pub(super) autowrap: bool,
    /// IRM: printing shifts the rest of the line right instead of overwriting.
    pub(super) insert_mode: bool,
    pub mouse_tracking: MouseTracking,
    /// SGR mouse encoding (DECSET 1006).
    pub mouse_sgr: bool,
    /// Report focus in/out (DECSET 1004).
    pub focus_events: bool,
    /// Translate the mouse wheel into arrow keys on the alternate screen (DECSET 1007).
    pub alt_scroll: bool,
    /// SGR mouse reports carry pixel instead of cell coordinates (DECSET 1016).
    pub mouse_sgr_pixels: bool,
    /// Report system light/dark appearance changes (DECSET 2031).
    pub color_scheme_reports: bool,
    /// xterm `modifyOtherKeys` level (0 off, 1, 2) set by `CSI > 4 ; level m`.
    pub modify_other_keys: u8,
    /// Kitty keyboard protocol state for the main and the alternate screen.
    pub(super) kitty_keyboard: [KittyKeyboard; 2],
    /// DEC mode 2027: emoji sequences (ZWJ, flags, skin tones, VS16) share one cell pair.
    pub(super) grapheme_mode: bool,
}

impl Grid {
    /// Active Kitty keyboard protocol flags (`keymap::KITTY_*`) of the current screen.
    pub fn kitty_keyboard_flags(&self) -> u8 {
        self.modes.kitty_keyboard[self.alt_active as usize].flags
    }
}
```

These 17 fields keep their exact types; `MouseTracking` and `KittyKeyboard` come from `cell.rs` via `use super::*`. `cell_px`, `title`, `progress`, `pointer_shape` and the other host-request fields are **not** modes and stay on `Grid`.

- [ ] **Step 1: Create `modes.rs`** with the content above, moving (not copying) `GRAPHEME_DEFAULT` (≈120-121), `set_grapheme_default` (≈123-126) and `kitty_keyboard_flags` (≈3180) out of mod.rs. In mod.rs:

```rust
mod modes;

pub use modes::{set_grapheme_default, TerminalModes};
use modes::GRAPHEME_DEFAULT;
```

- [ ] **Step 2: Replace the 17 fields in `Grid`**

Delete from `struct Grid`: `insert_mode` (≈206-207), `cursor_visible` (≈211), `cursor_shape` (≈212-214), `app_cursor`, `bracketed_paste`, `sync_output`, `sync_started` (≈215-219), `autowrap` (≈220), `mouse_tracking` through `color_scheme_reports` (≈222-232), `modify_other_keys` (≈246-247), `kitty_keyboard` (≈248-249), `grapheme_mode` (≈308-309), each with its doc comment. Insert where `cursor_visible` was:

```rust
    /// DECSET / DECRST and keyboard protocol modes (reset together by RIS).
    pub modes: TerminalModes,
```

- [ ] **Step 3: Replace the initializer** in `Grid::new` — delete the 17 field lines and add, where `cursor_visible: true,` was:

```rust
            modes: TerminalModes {
                cursor_visible: true,
                cursor_shape: 0,
                app_cursor: false,
                bracketed_paste: false,
                sync_output: false,
                sync_started: None,
                autowrap: true,
                insert_mode: false,
                mouse_tracking: MouseTracking::Off,
                mouse_sgr: false,
                focus_events: false,
                alt_scroll: true,
                mouse_sgr_pixels: false,
                color_scheme_reports: false,
                modify_other_keys: 0,
                kitty_keyboard: [KittyKeyboard::default(); 2],
                grapheme_mode: GRAPHEME_DEFAULT.load(Ordering::Relaxed),
            },
```

(These are exactly the old literals; check each one against the deleted lines before saving.)

- [ ] **Step 4: Rename accesses**

```bash
fields='cursor_visible|cursor_shape|app_cursor|bracketed_paste|sync_output|sync_started|autowrap|insert_mode|mouse_tracking|mouse_sgr_pixels|mouse_sgr|focus_events|alt_scroll|color_scheme_reports|modify_other_keys|kitty_keyboard|grapheme_mode'
files=$(grep -rlE "\.($fields)\b" src --include='*.rs')
echo "$files"
perl -pi -e "s/(?<!\.modes)\.($fields)\b(?!\s*\()/.modes.\$1/g" $files
cargo check
```

The regex skips method calls (`kitty_keyboard_flags()` is unaffected anyway: `\b` does not match inside `kitty_keyboard_flags`) and is idempotent (`(?<!\.modes)`). `full_reset` keeps its per-field assignments, now `self.modes.<field> = ...`, in the original order — do **not** collapse them into one struct assignment in this task.

- [ ] **Step 5: Review the external diff**

```bash
git diff --stat
git diff -- src/app src/renderer.rs src/pty.rs | grep '^[-+]' | grep -vE '^(\+\+\+|---)' | grep -v '\.modes\.'
```

Expected: no output from the second command. `src/renderer.rs` has exactly one changed line (`g.modes.cursor_shape`), `src/pty.rs` one (`g.modes.sync_output` in `feed_output`, lock order unchanged).

- [ ] **Step 6: Verify** — "V", then G-hot:

```bash
target/grid-gate/gate.sh 5 3
```

Expected: no `FAIL` (`put_char` now reads `self.modes.autowrap` / `self.modes.insert_mode`; watch `ascii_scroll`, `alt_redraw`), `GRID_SIZE` within 16 bytes of `S1`; record it as `S2`.

- [ ] **Step 7: Manual smoke test** (mode flags drive input encoding)

`cargo run --release`, then: arrow keys in `vim` (DECCKM `app_cursor`), paste into zsh (bracketed paste), wheel in `less` (alt scroll), click in `htop` (mouse tracking + SGR), `printf '\e[?1004h'; cat -v` then focus another app and back (prints `^[[O` / `^[[I`; Ctrl+C, then `printf '\e[?1004l'`), `printf '\e[5 q'` (bar cursor shape), Kitty keyboard in `kitten show-key -m kitty`, and the inspector overlay shows the same mode values as on `main`.

- [ ] **Step 8: Commit**

```bash
git add src/grid/modes.rs src/grid/mod.rs src/grid/perform.rs src/grid/tests.rs src/renderer.rs src/pty.rs
git add $(git diff --name-only -- src/app)
git commit -m "refactor(grid): group terminal mode flags in TerminalModes"
```

---

### Task 14: Group cluster fields in `ClusterTable`

**Files:**
- Modify: `src/grid/clusters.rs` (new struct), `src/grid/mod.rs` (field + initializer), `src/grid/storage.rs`, `src/grid/viewport.rs`, `src/grid/tests.rs` (renames)

**Interfaces:**
- Produces in `src/grid/clusters.rs`:

```rust
/// Multi-codepoint graphemes cells refer to (see `CLUSTER_FLAG`), interned,
/// plus the scratch buffers the collector reuses so it never allocates per row.
#[derive(Default)]
pub(super) struct ClusterTable {
    /// Interned cluster text, indexed by `CLUSTER_ID`.
    pub(super) texts: Vec<Box<str>>,
    pub(super) ids: std::collections::HashMap<Box<str>, u32>,
    /// Reused buffer for building a cluster.
    pub(super) buf: String,
    /// Collect at most once per history-sized batch of cluster-bearing evictions.
    pub(super) evictions: usize,
    pub(super) marks: Vec<bool>,
    pub(super) remap: Vec<u32>,
}
```

- In `Grid`, replace `clusters`, `cluster_ids`, `cluster_buf`, `cluster_evictions`, `cluster_marks`, `cluster_remap` (≈299-307 at `d1d0c92`, with docs) by:

```rust
    /// Grapheme cluster table (see `ClusterTable`).
    clusters: ClusterTable,
```

- `Grid::new`: the six initializer lines become `clusters: ClusterTable::default(),` (`HashMap::default()` is `HashMap::new()`; the others are empty / zero exactly as before).
- mod.rs: `use clusters::ClusterTable;`.

`ClusterTable` is data only. `intern_cluster`, `gc_clusters`, `cluster` stay `impl Grid` methods: GC must scan the ring cells and the alt screen, and interning triggers GC — the table cannot do either alone. No method signature changes.

- [ ] **Step 1: Add the struct, replace the fields and the initializer** as above.

- [ ] **Step 2: Rename accesses — grid files only** (`src/atlas.rs` has unrelated `clusters` / `cluster_ids` fields):

```bash
perl -pi -e 's/\.clusters\b(?!\.(texts|ids|buf|evictions|marks|remap)\b)/.clusters.texts/g; s/\.cluster_(ids|buf|evictions|marks|remap)\b/.clusters.$1/g' src/grid/*.rs
cargo check
git diff --stat
```

Expected `--stat`: only `src/grid/` files (`mod.rs`, `clusters.rs`, `storage.rs`, `viewport.rs`, `tests.rs`). In `gc_clusters`, `self.clusters.texts.retain(|text| { ... self.clusters.ids.insert(...) ... })` compiles because edition-2021+ closures capture disjoint field paths.

- [ ] **Step 3: Verify** — "V", then G-hot:

```bash
target/grid-gate/gate.sh 5 3
cargo test --release cluster -- --test-threads=1
```

Expected: no `FAIL` (watch `emoji_clusters`), all cluster tests `ok` (including the zero-allocation eviction test and `mark storage is reused`), `GRID_SIZE` within 16 bytes of `S2`; record `S3`.

- [ ] **Step 4: Commit**

```bash
git add src/grid/clusters.rs src/grid/mod.rs src/grid/storage.rs src/grid/viewport.rs src/grid/tests.rs
git commit -m "refactor(grid): group cluster table fields in ClusterTable"
```

---

### Task 15: Group ring fields in `Ring` (strictest gate)

**Files:**
- Modify: `src/grid/storage.rs` (new struct), `src/grid/mod.rs` (field + initializer), every `src/grid/*.rs` that touches the six fields (`perform.rs`, `resize.rs`, `viewport.rs`, `clusters.rs`, `images.rs`, `prompt.rs`, `selection.rs`, `links.rs`, `visual.rs`, `tests.rs` — the rename command lists them)

**Interfaces:**
- Produces in `src/grid/storage.rs`:

```rust
/// Main screen + scrollback ring. `cells` holds `cap` rows of `cols` cells;
/// visible row 0 is physical row `top`, and `history` valid scrollback rows
/// sit above it. Row arithmetic stays on `Grid` (it also needs `cols`, `rows`
/// and the alternate screen).
pub(super) struct Ring {
    /// Main screen + scrollback ring.
    pub(super) cells: Vec<Cell>,
    /// Per physical main-screen row: the line continues on the next row (soft wrap).
    pub(super) wrapped: Vec<bool>,
    /// Physical ring row of visible row 0.
    pub(super) top: usize,
    /// Ring capacity in rows (at most `scrollback + rows`).
    pub(super) cap: usize,
    /// Number of valid scrollback rows above `top`.
    pub(super) history: usize,
    /// Scrollback lines kept above the screen (`scrollback-limit`).
    pub(super) scrollback: usize,
}
```

- In `Grid`, delete `scrollback`, `cap`, `cells`, `top`, `history` (≈175-184) and `wrapped` (≈202-203) with their docs; insert where `scrollback` was:

```rust
    /// Main screen + scrollback storage (see `Ring`).
    ring: Ring,
```

`alt` (the alternate screen buffer), `lines_pushed`, `display_offset`, `cols`, `rows` stay on `Grid`.

- `Grid::new`: replace `scrollback,`, `cap,`, `cells: bytemuck::zeroed_vec(cols * cap),`, `top: 0,`, `history: 0,` and `wrapped: vec![false; cap],` with:

```rust
            ring: Ring {
                cells: bytemuck::zeroed_vec(cols * cap),
                wrapped: vec![false; cap],
                top: 0,
                cap,
                history: 0,
                scrollback,
            },
```

(The two allocations happen in the same order as before: `cells` was initialized before `wrapped`.)
- mod.rs: `use storage::Ring;`.

No method moves into `Ring`. `row_start`, `row_mut`, `display_row`, `scroll_up_in`, `grow_capacity` stay `#[inline]` / plain `impl Grid` methods in `storage.rs` exactly as Task 11 left them; only their field paths change (`self.top` → `self.ring.top`).

- [ ] **Step 1: Add the struct, replace the fields and the initializer** as above.

- [ ] **Step 2: Rename accesses — grid files only**

All accesses at `d1d0c92` go through `self.`, `g.` or `g2.` (checked with `grep -noE '[A-Za-z_0-9)\]]+\.(cells|top|cap|history|wrapped|scrollback)\b' src/grid/*.rs`); locals named `top`, `cells`, `wrapped`, `scrollback` in `resize` and the prompt helpers are not field accesses and must stay untouched, which the anchored regex guarantees:

```bash
perl -pi -e 's/\b(self|g|g2)\.(cells|top|cap|history|wrapped|scrollback)\b/$1.ring.$2/g' src/grid/*.rs
cargo check
grep -rnE '\b(self|g|g2)\.(cells|top|cap|history|wrapped|scrollback)\b' src/grid
```

Expected: `cargo check` clean; the final grep prints nothing. In `resize`, the block `self.cells = cells; self.wrapped = wrapped; ... self.cap = new_cap; self.scrollback = scrollback; self.top = new_hist; self.history = new_hist;` becomes the same assignments on `self.ring.*`, in the same order. Do not replace it with `self.ring = Ring { ... }` in this task.

- [ ] **Step 3: Verify** — "V", then the strictest gate:

```bash
target/grid-gate/gate.sh 7 1.5
```

Expected: every probe case within +1.5 % (watch `ascii_scroll`, `region_scroll`, `alt_redraw` above all), stress throughput within −1.5 %, `GRID_SIZE` within 16 bytes of `S3`. A larger `GRID_SIZE` jump means the compiler re-laid out `Grid` around `Ring`; that alone is not a failure if the probe passes, but write the before/after sizes in the commit body. If a case fails twice, try field order inside `Ring` matching hot use (`cells`, `top`, `cap` first — they are what `row_start` / `display_row` read); `Ring` is not `#[repr(C)]`, so rustc may reorder anyway — if the regression persists, `git restore` and record the finding in `ROADMAP.md` instead of committing.

Also re-run the allocation-sensitive tests in release:

```bash
cargo test --release allocat -- --test-threads=1
```

Expected: all `ok` (`steady_state_print_and_scroll_allocate_nothing`, `precise_scroll_allocates_nothing_and_preserves_cell_layout`, the cluster eviction test).

- [ ] **Step 4: Commit**

```bash
git add $(git diff --name-only -- src/grid)
git commit -m "refactor(grid): group ring buffer fields in Ring"
```

Commit body: probe numbers from the gate output (base / new per case) and the `GRID_SIZE` before / after.

---

### Task 16: Docs and final verification

**Files:**
- Modify: `CLAUDE.md`, `AGENTS.md`, `GEMINI.md` (same architecture line in all three), `ROADMAP.md`

- [ ] **Step 1: Update the architecture line** in `CLAUDE.md`, `AGENTS.md` and `GEMINI.md` (each has `` - `src/grid.rs`: 12-byte `Cell`, ring buffer, `vte::Perform`. ... ``). Replace it with:

```markdown
- `src/grid/`: `Grid` terminal state. `mod.rs` holds the struct and the print / parse hot path (`feed_byte`, `put_char`); `cell.rs` the 12-byte `Cell`; `storage.rs` the scrollback `Ring`; `perform.rs` the `vte::Perform` impl; `modes.rs`, `colors.rs`, `control.rs`, `images.rs`, `clusters.rs`, `links.rs`, `prompt.rs`, `selection.rs`, `viewport.rs`, `resize.rs`, `visual.rs`, `width.rs` one concern each; unit tests in `tests.rs`. Responses (DSR, DA) go to `Grid::response`.
```

In the same three files, extend the **Tests** bullet's "unit tests in the same file" with: "(`grid` keeps its tests in `src/grid/tests.rs`)".

- [ ] **Step 2: Update `ROADMAP.md`**

Under `### Code Health & Hardening — Audit 2026-10-31` add:

```markdown
- [x] **Phase 2 structural refactor: split `src/grid/mod.rs`** (plan: `plans/2026-11-01-split-grid.md`)
  - `mod.rs` 5499 → ~780 lines; `Grid` 88 → 57 fields (`AiTrack`, `TerminalModes`, `ClusterTable`, `Ring`). Probe and stress throughput within the ±3 % gate (Ring within ±1.5 %).
  - Follow-ups (not done, would reduce `Grid` further): group the OSC/DCS/APC collection state (`osc_*`, `dcs*`, `apc_*`, 9 fields), host requests (`title`, `title_changed`, `reported_cwd`, `clipboard_request`, `notification`, `bell`, `progress`, `pointer_shape`), cell extras (`extras*`) and search state (`selection`, `search_matches`, `search_current`); collapse `full_reset`'s per-field mode resets into one `TerminalModes` constructor.
```

Replace `~780` with the `wc -l src/grid/mod.rs` result from Step 3, and add the worst per-case probe delta from Task 15's commit body.

- [ ] **Step 3: Full verification and size check**

```bash
cargo check && cargo check --target x86_64-unknown-linux-gnu && cargo test && cargo clippy --all-targets -- -D warnings && git diff --check
wc -l src/grid/*.rs | sort -rn
awk '/^pub struct Grid \{/,/^\}/' src/grid/mod.rs | grep -cE '^    (pub(\([a-z]+\))? )?[a-z_0-9]+: '
cat src/grid/*.rs | grep -c '#\[inline'
```

Expected: all green; `tests.rs` largest, `mod.rs` ≈ 780, no other file above ~500; field count `57`; inline count `23`; `cargo test` totals `M` passed / `I` ignored.

- [ ] **Step 4: Final performance and SLA check**

```bash
target/grid-gate/gate.sh 5 3
cargo run --release -- +bench-perf > target/grid-gate/bench-perf-final.txt 2>&1; echo "exit=$?"
grep -E 'Verdict|FAIL' target/grid-gate/bench-perf-base.txt target/grid-gate/bench-perf-final.txt
```

Expected: gate passes against the Task 0 baseline (cumulative drift of the whole phase ≤ 3 %); `exit=0`; no `FAIL` lines in the final file.

- [ ] **Step 5: Release-mode smoke test**

`cargo run --release`: `cat` a large log (scrollback, wheel and pixel scroll, Cmd+K / Ctrl+Shift+K clear), `vim` and `htop` (alt screen, mouse), resize the window with a long wrapped line (reflow), Vietnamese with NFD accents and an emoji ZWJ family (clusters), `printf '\e]8;;https://example.com\e\\link\e]8;;\e\\\n'` then Cmd/Ctrl-hover and click, `kitten icat` an image (Kitty graphics), Cmd+F search, select-to-copy, OSC 133 prompt jump, the AI `yo` flow at a prompt, the inspector overlay. Check idle RAM (< 50 MB) and idle CPU (0 %).

- [ ] **Step 6: Commit**

```bash
git add CLAUDE.md AGENTS.md GEMINI.md ROADMAP.md
git commit -m "docs(grid): describe grid module layout"
```

Then delete the gate artifacts only after the branch is merged: `rm -rf target/grid-gate target/baseline-grid-tests.txt`.

---

## Where the Phase 1 outline was corrected

The outline in `2026-11-01-split-main-rs.md` ("Phase 2") was checked against the code at `d1d0c92`:

- `Grid` has 88 fields but **128** inherent methods (plus 7 `vte::Perform` methods), not ~200.
- **`Ring` cannot own `row_start`, `row_mut`, `scroll_up_in`, `grow_capacity`, `copy_row`, `clear_rows`**: they branch on `alt_active` / `alt`, need `cols` / `rows`, and `scroll_up_in` also updates `graphics`, cluster eviction counters, `extras_dirty`, `display_offset`, `lines_pushed` and `all_dirty`. `Ring` is therefore a data-only struct; the methods move as `impl Grid` into `storage.rs` (Task 11) and only their field paths change (Task 15).
- **`ClusterTable` cannot own `intern_cluster` / `gc_clusters`**: GC scans the ring and the alt screen, and interning triggers GC. Data-only struct; `cluster_count` / `seed_dead_clusters_for_bench` stay `Grid` methods because `app::latency_benchmarks` calls them.
- `colors()` (pen → cell colors) stays in mod.rs: `put_char` calls it per character. `reply_color` goes to `colors.rs` (only `osc4` / `osc_dynamic_color` call it), not `control.rs`.
- `apc_dispatch` goes to `images.rs` (it only parses Kitty commands and queues image jobs), not `control.rs`. The OSC/DCS/APC collection fields stay on `Grid` because `feed_byte` reads them on every byte; grouping them is a listed follow-up.
- `TerminalModes` has **17** fields, not ~20; `cell_px`, `title`, `progress` etc. are not modes.
- With exactly the four sub-structs of the outline, `Grid` ends at **57** fields, not ≈25. Reaching ~25 needs the follow-up groupings recorded in Task 16.
- `mod.rs` reaches ≈780 lines only because this plan also moves selection/search (into the existing `selection.rs`), viewport, resize, bidi and width code, which the outline did not list.
- The outline's gate ("`cargo test --release` latency benchmarks") does not gate the grid: `app::latency_benchmarks::benchmark_before_vs_after` prints numbers without asserting and mostly measures history search, atlas and syscalls; `+bench-stress` bypasses `feed_byte`. Task 0 adds a dedicated probe.
- `linefeed` is not `#[inline]` today; the outline's "preserve `#[inline]` on linefeed" means "keep it as it is".
- The crate is edition **2024** (Phase 1's header says 2021).
