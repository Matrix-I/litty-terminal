# Performance review fixes Implementation Plan — 2026-11-11

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Fix findings 1 and 3 of
[the 2026-11-11 performance review](../validation/2026-11-11-perf-review.md)
and make the default scrollback a byte budget, so the default reflow cost no
longer grows with the window width. Finding 2 (occlusion) gets its own plan.

**Architecture:** Three independent changes, one commit each:

1. `scrollback-limit` defaults to 10,000,000 bytes (`src/terminal/grid/storage.rs`).
2. A resize event resizes only the visible tab; the other tabs follow once the burst settles (`src/app/window.rs`, `src/app/winch.rs`, `src/app/mod.rs`).
3. The glyph atlas slot map uses a multiplicative hasher instead of SipHash (`src/render/atlas/mod.rs`).

**Tech Stack:** Rust 2024, no new dependency.

**Branch:** `fix/perf-review-fixes` (from `main`).

## Confirmed Requirements

1. Background tabs are resized once, 100 ms after the last resize of a burst, or at once when they are shown. The visible tab is resized on every event. (user, 2026-11-11)
2. Scrollback stays configured in bytes through `scrollback-limit`, as in Ghostty; no line-count key is added. Its default becomes 10,000,000 bytes. (user, 2026-11-11)
3. Scrollback is limited by bytes only: no 20,000-line cap. The existing 1,000,000-line bound stays. (user, 2026-11-11, replacing an earlier request for a 20,000-line cap)
4. Finding 2 is not fixed here: no platform emits `WindowEvent::Occluded`, so the fix is an occlusion emitter in `src/platform/`, planned separately. (user, 2026-11-11)
5. Finding 3 is fixed as the review suggests. (user asked for a plan to fix the review)

## Global Constraints

From `CLAUDE.md`:

- `Cell` stays exactly 12 bytes.
- Zero heap allocation on hot paths (PTY parsing, scrolling, render loop).
- Minimize grid `Mutex` hold time; never block on I/O while holding it.
- < 50 MB idle RAM, 0% idle CPU (`ControlFlow::Wait`).
- Layers: `terminal` → nothing; `render` → `terminal`; `platform` → nothing; `app` → anything.
- Every platform-specific path has a counterpart on the other platform so both targets compile.
- Bug fixes start with a failing regression test. New logic in pure modules gets unit tests in the same file (`grid` keeps its tests in `src/terminal/grid/tests.rs`).
- Update `README.md` / the guides when config keys or user-visible behavior change.
- Verification before every commit that touches `src/`:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside the sandbox; PTY tests need `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit only when asked. Conventional Commits, `type(scope): summary`, lowercase, ≤ 72 characters. Stage files by name. No `Co-Authored-By` trailer.

From `docs/plans/2026-11-08-performance-gaps.md`:

- Every optimization ships with before/after numbers from the same benchmark, same machine, `release` profile (`lto = "fat"`), never `fast-release`.

## Technical Defaults

- The settle delay reuses `winch::SETTLE` (100 ms) — the project already treats that as the end of a resize burst.
- The settled pass resizes every tab, the visible one included — it is a no-op for a pane already at its size, and it repairs any code path that changes `s.active` without calling `sync_size`.
- Hasher: one multiply by `0x9E37_79B9_7F4A_7C15`, folded with `h ^ (h >> 32)` — the flag bits live in the high half of the key and the table indexes with the low bits.

## Risks & Rollback

| Risk | Mitigation | Revert |
| :--- | :--- | :--- |
| Default scrollback changes from 5,000 lines to 10 MB: 10,416 lines at 80 columns, 4,166 at 200 columns (fewer than today above 166 columns) | Documented in `docs/guide/configuration.md`; users set `scrollback-limit` | Revert the Task 1 commit |
| A full ring at 80 columns holds 10 MB instead of 4.8 MB per pane | The ring still grows on demand; idle RAM is unchanged. Run `litty +bench-ram` in Task 4 | Revert the Task 1 commit |
| A large configured `scrollback-limit` still makes each resize step of the visible pane slow (233 ms at 100,000 lines) | Accepted by requirement 3; the default is bounded at about 833,000 cells per pane | none needed |
| A background program learns the new size up to 100 ms late and wraps output at the old width meanwhile | Requirement 1; the grid reflows that output when the tab is resized | Revert the Task 2 commit |
| A code path makes a tab visible without `sync_size` | The settled pass resizes every tab; `render_panes` already clamps to `rect.cols.min(g.cols)` | Revert the Task 2 commit |
| Weak hash clusters keys | Unit test bounds bucket and tag load over 4,096 real keys (measured: max 7 per bucket, 35 per tag) | Revert the Task 3 commit |

No persisted data, API or message format changes. The config format is unchanged (same key, same unit).

**Commit checkpoints:** before each "Ready to commit", run the five verification commands listed under Global Constraints and quote their results; each commit must build and pass on its own.

---

## Task 0: Branch

- [x] **Step 1.** Create the branch from `main`:

  ```bash
  git switch -c fix/perf-review-fixes main
  ```

---

## Task 1: Scrollback default of 10 MB

**Files:**

- `src/terminal/grid/storage.rs:8-32`
- `src/terminal/grid/tests.rs:7`, `:313`, `:553-560`, `:1322`, `:1326-1333`
- `src/terminal/grid/mod.rs:5`, `:198`
- `src/config.rs:146`
- `docs/guide/configuration.md:50`, `:117`

**Interfaces:**

- Produces: `pub const SCROLLBACK_BYTES_DEFAULT: usize = 10_000_000;`
- Unchanged: `pub(super) const MAX_SCROLLBACK: usize = 1_000_000;`, `pub fn set_scrollback_limit(bytes: Option<usize>)`, `pub(super) fn scrollback_lines(cols: usize) -> usize`, `pub(super) fn lines_for_bytes(bytes: usize, cols: usize) -> usize`
- Removes: `pub const SCROLLBACK: usize = 5000;` (used only by the tests changed here and one doc comment)

- [x] **Step 1 — failing test.** In `src/terminal/grid/tests.rs`, change line 7 to:

  ```rust
  use super::storage::{MAX_SCROLLBACK, SCROLLBACK_BYTES_DEFAULT, lines_for_bytes};
  ```

  Replace the test `scrollback_limit_is_bytes_per_width` with:

  ```rust
  #[test]
  fn scrollback_limit_is_bytes_per_width() {
      assert_eq!(SCROLLBACK_BYTES_DEFAULT, 10_000_000, "Ghostty's default");
      assert_eq!(lines_for_bytes(SCROLLBACK_BYTES_DEFAULT, 80), 10_416);
      assert_eq!(lines_for_bytes(SCROLLBACK_BYTES_DEFAULT, 200), 4_166);
      assert_eq!(lines_for_bytes(80 * 12 * 100, 80), 100);
      assert_eq!(lines_for_bytes(80 * 12 * 100, 160), 50);
      assert_eq!(lines_for_bytes(0, 80), 0);
      assert_eq!(lines_for_bytes(usize::MAX, 1), MAX_SCROLLBACK);
  }

  #[test]
  fn a_new_grid_uses_the_default_byte_limit() {
      assert_eq!(Grid::new(80, 24).scrollback, 10_416);
      assert_eq!(Grid::new(200, 50).scrollback, 4_166);
  }
  ```

  Two tests used `SCROLLBACK` to fill a 4-column grid to its limit. At 4 columns the byte default allows 208,333 lines, too many to feed in a unit test; at 2,000 columns it allows 416. Add next to the other test helpers:

  ```rust
  /// A width whose default scrollback (416 lines) is small enough to fill in a test.
  const WIDE_COLS: usize = 2_000;
  ```

  Change line 1322 to:

  ```rust
      let limit = lines_for_bytes(SCROLLBACK_BYTES_DEFAULT, WIDE_COLS);
      assert_rows_only_resize_matches_reflow(WIDE_COLS, 5, &numbered(limit + 50), &[3, 7, 2]);
  ```

  Replace the body of `shrinking_rows_with_full_scrollback_keeps_history_within_its_limit` with:

  ```rust
      let limit = lines_for_bytes(SCROLLBACK_BYTES_DEFAULT, WIDE_COLS);
      assert_eq!(limit, 416);
      let mut g = Grid::new(WIDE_COLS, 50);
      feed(&mut g, &"x\r\n".repeat(limit + 100));
      assert_eq!(g.history, limit);
      g.resize(WIDE_COLS, 10);
      feed(&mut g, &"y\r\n".repeat(200));
      assert_eq!(g.history, limit);
  ```

- [x] **Step 2 — run, expect FAIL.**

  ```bash
  cargo test scrollback
  ```

  Expected: compile error, `unresolved import super::storage::SCROLLBACK_BYTES_DEFAULT`.

- [x] **Step 3 — implementation.** In `src/terminal/grid/storage.rs` replace lines 8–32 (from the `SCROLLBACK` doc comment through the end of `lines_for_bytes`) with:

  ```rust
  /// Default `scrollback-limit`, in bytes (Ghostty's default). A byte budget
  /// keeps the cost of re-wrapping the scrollback independent of the width.
  pub const SCROLLBACK_BYTES_DEFAULT: usize = 10_000_000;

  /// Upper bound for a configured scrollback, whatever its byte size.
  pub(super) const MAX_SCROLLBACK: usize = 1_000_000;

  /// `scrollback-limit` in bytes (Ghostty semantics). Read when a terminal is
  /// created or resized.
  pub(super) static SCROLLBACK_BYTES: AtomicUsize = AtomicUsize::new(SCROLLBACK_BYTES_DEFAULT);

  pub fn set_scrollback_limit(bytes: Option<usize>) {
      SCROLLBACK_BYTES.store(bytes.unwrap_or(SCROLLBACK_BYTES_DEFAULT), Ordering::Relaxed);
  }

  /// Scrollback lines for a terminal `cols` wide.
  pub(super) fn scrollback_lines(cols: usize) -> usize {
      lines_for_bytes(SCROLLBACK_BYTES.load(Ordering::Relaxed), cols)
  }

  pub(super) fn lines_for_bytes(bytes: usize, cols: usize) -> usize {
      (bytes / (cols.max(1) * std::mem::size_of::<Cell>())).min(MAX_SCROLLBACK)
  }
  ```

  Comment updates, no behavior:

  - `src/terminal/grid/mod.rs:5`: replace `` `cols * (SCROLLBACK + rows)` `` with `` `cols * (scrollback + rows)` ``.
  - `src/terminal/grid/mod.rs:198`: replace `instead of the full 5000+ scrollback ring` with `instead of the full scrollback ring`.
  - `src/config.rs:146`: replace the doc comment with `` /// `scrollback-limit` in bytes (Ghostty semantics); `None` = 10,000,000. ``
  - `src/terminal/grid/tests.rs:313`: the comment `Fill the scrollback ring so scrolling stops allocating too.` becomes `Grow the scrollback ring so scrolling stops allocating in the measured pass.` (at 80 columns the ring no longer fills in 120 rounds).

- [x] **Step 4 — run, expect PASS.**

  ```bash
  cargo test scrollback
  ```

  ```bash
  cargo test grid::
  ```

- [x] **Step 5 — docs.** In `docs/guide/configuration.md`:

  - Line 50, the `scrollback-limit` row: default `10000000`, description `Scrollback memory per terminal in bytes, as in Ghostty (about 10,400 lines at 80 columns)`.
  - Line 117: `# scrollback-limit = 10000000    # bytes, as in Ghostty (default)`

- [ ] **Step 6 — commit checkpoint.** Ready to commit: `src/terminal/grid/storage.rs src/terminal/grid/tests.rs src/terminal/grid/mod.rs src/config.rs docs/guide/configuration.md` — `feat(grid): default scrollback to a 10 mb byte budget`

---

## Task 2: Resize only the visible tab on each event

**Files:**

- `src/app/winch.rs` (new type after `WinchResignal`, new tests in `mod tests`)
- `src/app/state.rs:56` (new field after `winch`)
- `src/app/window.rs:205` (field init), `:569-600` (`sync_size`), new test module at the end
- `src/app/tabs.rs:136-144` (`select_tab`)
- `src/app/mod.rs:31-37` (imports), `:480-492` (`about_to_wait`)

**Interfaces:**

- Consumes: `winch::SETTLE`, `WinchResignal::resized(&mut self, pane: u64, now: Instant)`, `Tab::compute_pane_rects(&self, total_cols: usize, total_rows: usize) -> Vec<PaneRect>`, `Pty::resize(&self, cols: u16, rows: u16, cell_w: u32, cell_h: u32)`, `Grid::resize(&mut self, cols: usize, rows: usize)`
- Produces:
  - `pub(super) struct BackgroundResize` with `schedule(&mut self, now: Instant)`, `deadline(&self) -> Option<Instant>`, `take_due(&mut self, now: Instant) -> bool`
  - `fn resize_tab(tab: &Tab, size: (usize, usize), cell: (u32, u32), winch: &mut WinchResignal, now: std::time::Instant) -> bool` in `src/app/window.rs`
  - `pub(super) fn sync_all_tabs(s: &mut State)` in `src/app/window.rs`
  - `State::background_resize: super::winch::BackgroundResize`

- [x] **Step 1 — failing tests.** In `src/app/winch.rs`, add to `mod tests`:

  ```rust
  #[test]
  fn background_tabs_wait_for_the_burst_to_settle() {
      let mut b = BackgroundResize::default();
      let t0 = Instant::now();
      let ms = Duration::from_millis;
      assert_eq!(b.deadline(), None);
      assert!(!b.take_due(t0 + SETTLE * 10), "nothing scheduled");
      for step in 0..10u32 {
          b.schedule(t0 + ms(16) * step);
      }
      let last = t0 + ms(16) * 9;
      assert_eq!(b.deadline(), Some(last + SETTLE), "each resize restarts the timer");
      assert!(!b.take_due(last + SETTLE - ms(1)), "not before it settles");
      assert!(b.take_due(last + SETTLE));
      assert_eq!(b.deadline(), None);
      assert!(!b.take_due(last + SETTLE * 2), "due only once per burst");
  }
  ```

  At the end of `src/app/window.rs` add:

  ```rust
  #[cfg(test)]
  mod resize_tests {
      use super::resize_tab;
      use crate::app::winch::WinchResignal;
      use crate::terminal::grid::Grid;
      use crate::terminal::pty::Pty;
      use crate::{next_id, Pane, Tab};
      use parking_lot::Mutex;
      use std::sync::Arc;

      fn tab(cols: usize, rows: usize) -> Tab {
          let pty = Pty::spawn_program("/bin/cat", &["cat"], &[], (cols as u16, rows as u16), (8, 16)).expect("spawn pty");
          Tab::new(next_id(), Pane::new(next_id(), Arc::new(Mutex::new(Grid::new(cols, rows))), pty))
      }

      fn size(tab: &Tab) -> (usize, usize) {
          let g = tab.panes[0].grid.lock();
          (g.cols, g.rows)
      }

      #[test]
      fn resize_tab_resizes_that_tab_only_and_reports_changes() {
          let (shown, hidden) = (tab(80, 24), tab(80, 24));
          let mut winch = WinchResignal::default();
          let now = std::time::Instant::now();
          assert!(resize_tab(&shown, (100, 30), (8, 16), &mut winch, now));
          assert_eq!(size(&shown), (100, 30));
          assert_eq!(size(&hidden), (80, 24), "a background tab keeps its size until the burst settles");
          assert!(!resize_tab(&shown, (100, 30), (8, 16), &mut winch, now), "already at that size");
          assert!(resize_tab(&hidden, (100, 30), (8, 16), &mut winch, now));
          assert_eq!(size(&hidden), (100, 30));
      }
  }
  ```

- [x] **Step 2 — run, expect FAIL.**

  ```bash
  cargo test app::
  ```

  Expected: compile errors, `cannot find type BackgroundResize` and `unresolved import super::resize_tab`.

- [x] **Step 3 — implementation.**

  `src/app/winch.rs`, after the `impl WinchResignal` block:

  ```rust
  /// Background tabs are brought to the window size once a resize burst
  /// settles, not on every step: a width change re-wraps the whole scrollback
  /// of every pane on the UI thread.
  #[derive(Default)]
  pub(super) struct BackgroundResize {
      due: Option<Instant>,
  }

  impl BackgroundResize {
      /// The visible tab was just resized: (re)start the settle timer.
      pub(super) fn schedule(&mut self, now: Instant) {
          self.due = Some(now + SETTLE);
      }

      /// When the event loop must wake to resize the other tabs, if pending.
      pub(super) fn deadline(&self) -> Option<Instant> {
          self.due
      }

      /// True once, when the burst has settled by `now`.
      pub(super) fn take_due(&mut self, now: Instant) -> bool {
          if !self.due.is_some_and(|due| now >= due) {
              return false;
          }
          self.due = None;
          true
      }
  }
  ```

  `src/app/state.rs`, after the `winch` field:

  ```rust
      /// Tabs not shown are resized once a resize burst settles.
      pub(super) background_resize: super::winch::BackgroundResize,
  ```

  `src/app/window.rs:205`, after `winch: super::winch::WinchResignal::default(),`:

  ```rust
          background_resize: super::winch::BackgroundResize::default(),
  ```

  `src/app/window.rs`, replace the whole `sync_size` function with:

  ```rust
  /// Bring every pane of `tab` to its rect for a window of `size` cells.
  /// Returns whether a grid changed size.
  fn resize_tab(
      tab: &Tab,
      size: (usize, usize),
      cell: (u32, u32),
      winch: &mut super::winch::WinchResignal,
      now: std::time::Instant,
  ) -> bool {
      let (cw, ch) = cell;
      let mut any = false;
      for rect in tab.compute_pane_rects(size.0, size.1) {
          let Some(p) = tab.panes.iter().find(|p| p.id == rect.id) else { continue };
          let changed = {
              let mut g = p.grid.lock();
              g.cell_px = (cw as u16, ch as u16);
              if (g.cols, g.rows) == (rect.cols, rect.rows) {
                  false
              } else {
                  g.resize(rect.cols, rect.rows);
                  true
              }
          };
          if changed {
              p.pty.resize(rect.cols as u16, rect.rows as u16, cw, ch);
              winch.resized(p.id, now);
              any = true;
          }
      }
      any
  }

  /// Resize the visible tab now; the others follow once the burst settles
  /// (`sync_all_tabs`), so a window drag re-wraps one tab per step.
  pub(super) fn sync_size(s: &mut State) {
      let bands = s.bands();
      let size = s.renderer.grid_size_for_tabs(bands);
      let cell = s.renderer.cell_size();
      let now = std::time::Instant::now();
      if let Some(tab) = s.tabs.get(s.active) {
          resize_tab(tab, size, cell, &mut s.winch, now);
      }
      if s.tabs.len() > 1 {
          s.background_resize.schedule(now);
      }
      // Reflow cleared the matches.
      if s.search.is_open {
          refresh_search(s);
      }
  }

  /// The resize burst settled: bring every tab to the window size. A no-op
  /// for panes already there, the visible tab included.
  pub(super) fn sync_all_tabs(s: &mut State) {
      let bands = s.bands();
      let size = s.renderer.grid_size_for_tabs(bands);
      let cell = s.renderer.cell_size();
      let now = std::time::Instant::now();
      let mut visible_changed = false;
      for (i, tab) in s.tabs.iter().enumerate() {
          let changed = resize_tab(tab, size, cell, &mut s.winch, now);
          visible_changed |= changed && i == s.active;
      }
      if visible_changed {
          if s.search.is_open {
              refresh_search(s);
          }
          s.window.request_redraw();
      }
  }
  ```

  `src/app/tabs.rs`, `select_tab`: add `sync_size(s);` right after `s.tabs[idx].bell = false;` so a tab shown before the burst settled is resized at once.

  `src/app/mod.rs`: add `use window::sync_all_tabs;` next to the other `use window::…` lines. In `about_to_wait`, inside `for s in &mut self.windows {`, directly before `if let Some(panes) = s.winch.take_due(now) {`:

  ```rust
              if s.background_resize.take_due(now) {
                  sync_all_tabs(s);
              }
              if let Some(settle) = s.background_resize.deadline() {
                  deadline = Some(deadline.map_or(settle, |d| d.min(settle)));
              }
  ```

- [x] **Step 4 — run, expect PASS.**

  ```bash
  cargo test app::
  ```

- [ ] **Step 5 — commit checkpoint.** Ready to commit: `src/app/winch.rs src/app/state.rs src/app/window.rs src/app/tabs.rs src/app/mod.rs` — `perf(app): resize background tabs once a resize burst settles`

---

## Task 3: Multiplicative hasher for atlas slot keys

**Files:**

- `src/render/atlas/mod.rs:14` (imports), `:146` (field), `:251` (constructor), new type above `pub struct Atlas`
- `src/render/atlas/tests.rs` (new test at the end)

**Interfaces:**

- Consumes: key flags `STYLE_BOLD`, `STYLE_ITALIC`, `WIDE_LEFT`, `WIDE_RIGHT`, `CLUSTER_KEY`, `GLYPH_PIECE` (`src/render/atlas/mod.rs:97-129`)
- Produces: `pub(super) struct SlotKeyHasher(u64)` implementing `std::hash::Hasher` and `Default`; `type SlotMap = HashMap<u32, u16, std::hash::BuildHasherDefault<SlotKeyHasher>>;`

- [x] **Step 1 — baseline number.** Before any other change, add this temporary test at the end of `src/render/atlas/tests.rs`, run it, note the `std` line, and keep the test for Step 5:

  ```rust
  #[test]
  fn tmp_slot_map_lookup_cost() {
      use std::hash::BuildHasher;
      fn lookup_us<S: BuildHasher + Default>() -> f64 {
          let mut map: std::collections::HashMap<u32, u16, S> = std::collections::HashMap::with_capacity_and_hasher(4096, S::default());
          for cp in 0x21u32..0x7f {
              for flag in [STYLE_BOLD, STYLE_ITALIC, STYLE_BOLD | STYLE_ITALIC] {
                  map.insert(cp | flag, cp as u16);
              }
          }
          let keys: Vec<u32> = (0..8000u32).map(|i| (0x21 + (i * 7) % 94) | [STYLE_BOLD, STYLE_ITALIC][(i % 2) as usize]).collect();
          let mut sum = 0u64;
          let start = std::time::Instant::now();
          for _ in 0..1000 {
              for key in &keys {
                  sum += u64::from(*map.get(std::hint::black_box(key)).unwrap_or(&0));
              }
          }
          std::hint::black_box(sum);
          start.elapsed().as_secs_f64() * 1e6 / 1000.0
      }
      eprintln!("SLOTMAP std_us_per_8000={:.1}", lookup_us::<std::collections::hash_map::RandomState>());
  }
  ```

  ```bash
  cargo test --release tmp_slot_map_lookup_cost -- --nocapture
  ```

- [x] **Step 2 — failing test.** At the end of `src/render/atlas/tests.rs`:

  ```rust
  #[test]
  fn slot_key_hasher_spreads_style_and_width_flags() {
      use std::hash::{BuildHasher, BuildHasherDefault};
      let build = BuildHasherDefault::<SlotKeyHasher>::default();
      let flags = [0, STYLE_BOLD, STYLE_ITALIC, STYLE_BOLD | STYLE_ITALIC, WIDE_LEFT, WIDE_RIGHT, CLUSTER_KEY, GLYPH_PIECE];
      // The table indexes with the low bits and tags entries with the top 7.
      let mut buckets = [0u16; 4096];
      let mut tags = [0u16; 128];
      for cp in 0x21u32..0x221 {
          for flag in flags {
              let hash = build.hash_one(cp | flag);
              assert_eq!(hash, build.hash_one(cp | flag), "deterministic");
              buckets[(hash & 4095) as usize] += 1;
              tags[(hash >> 57) as usize] += 1;
          }
      }
      assert_eq!(buckets.iter().map(|&n| n as usize).sum::<usize>(), 4096);
      assert!(*buckets.iter().max().unwrap() <= 8, "bucket load {:?}", buckets.iter().max());
      assert!(*tags.iter().max().unwrap() <= 48, "tag load {:?}", tags.iter().max());
  }
  ```

  (Computed for this hash over these 4,096 keys: largest bucket 7, largest tag 35.)

- [x] **Step 3 — run, expect FAIL.**

  ```bash
  cargo test slot_key_hasher
  ```

  Expected: compile error, `cannot find type SlotKeyHasher`.

- [x] **Step 4 — implementation.** In `src/render/atlas/mod.rs`, above `pub struct Atlas`:

  ```rust
  /// Hasher for atlas slot keys (a code point plus flag bits). The keys are
  /// bounded by the slot count and evicted LRU, so SipHash's flood resistance
  /// buys nothing here. One multiply, folded so the flag bits in the high half
  /// of the key reach the low bits the table indexes with.
  #[derive(Default, Clone, Copy)]
  pub(super) struct SlotKeyHasher(u64);

  impl std::hash::Hasher for SlotKeyHasher {
      fn finish(&self) -> u64 {
          self.0
      }

      fn write(&mut self, bytes: &[u8]) {
          for &byte in bytes {
              self.write_u32(u32::from(byte));
          }
      }

      fn write_u32(&mut self, key: u32) {
          const GOLDEN_RATIO_64: u64 = 0x9E37_79B9_7F4A_7C15;
          let hash = (self.0 ^ u64::from(key)).wrapping_mul(GOLDEN_RATIO_64);
          self.0 = hash ^ (hash >> 32);
      }
  }

  type SlotMap = HashMap<u32, u16, std::hash::BuildHasherDefault<SlotKeyHasher>>;
  ```

  Change the field `map: HashMap<u32, u16>,` to `map: SlotMap,` and the constructor line `map: HashMap::with_capacity(n_slots as usize),` to `map: SlotMap::with_capacity_and_hasher(n_slots as usize, Default::default()),`. `cluster_ids: HashMap<Box<str>, u32>` keeps the default hasher. (`src/render/atlas/run_cache.rs` has a `PreHashed` hasher for keys that are already hashes; these keys are not, so it is not reused.)

- [x] **Step 5 — run, expect PASS; record the after number.** Append to `tmp_slot_map_lookup_cost`:

  ```rust
      eprintln!("SLOTMAP mul_us_per_8000={:.1}", lookup_us::<std::hash::BuildHasherDefault<SlotKeyHasher>>());
  ```

  ```bash
  cargo test --release tmp_slot_map_lookup_cost -- --nocapture
  ```

  ```bash
  cargo test atlas::
  ```

  Note both `SLOTMAP` lines for Task 4, then delete `tmp_slot_map_lookup_cost`. If `mul` is not faster than `std`, stop and report: do not commit this task.

- [ ] **Step 6 — commit checkpoint.** Ready to commit: `src/render/atlas/mod.rs src/render/atlas/tests.rs` — `perf(atlas): hash slot keys with one multiply instead of siphash`

---

## Task 4: Verification & Docs

- [x] **Step 1 — full verification** (each must pass; quote the result):

  ```bash
  cargo check
  ```

  ```bash
  cargo check --target x86_64-unknown-linux-gnu
  ```

  ```bash
  cargo test
  ```

  ```bash
  cargo clippy --all-targets -- -D warnings
  ```

  ```bash
  git diff --check
  ```

- [x] **Step 2 — before/after for the reflow.** Add this temporary test to `src/terminal/grid/tests.rs` without committing it, run it on the branch, then on `main` (`git stash`, `git switch main`, `git stash pop`; switch back the same way), then delete it:

  ```rust
  #[test]
  fn tmp_reflow_cost() {
      let mut g = Grid::new(200, 50);
      let line = format!("{}\r\n", "x".repeat(150));
      feed_fast(&mut g, line.repeat(25_000).as_bytes());
      let start = std::time::Instant::now();
      for i in 0..20 {
          g.resize(if i % 2 == 0 { 199 } else { 200 }, 50);
      }
      eprintln!("REFLOW history={} avg_ms={:.3}", g.history, start.elapsed().as_secs_f64() * 1000.0 / 20.0);
  }
  ```

  ```bash
  cargo test --release tmp_reflow_cost -- --nocapture
  ```

  Expected history: 5,000 on `main`, 4,166 on the branch.

- [ ] **Step 3 — end-to-end, needs a display.** _(2026-11-12: item 5 run, no regression; items 1–4 not run, they need interactive window resizing)_ `cargo build --release`, then with `target/release/litty`:

  1. Scrollback: in an 80-column window run `seq 1 30000`, scroll to the top. Expected first line near `19585` (10,416 lines kept).
  2. Background tabs: open three tabs, run `sh -c 'trap "stty size" WINCH; while :; do sleep 1; done'` in tabs 2 and 3, select tab 1, drag the window edge for two seconds. Expected: tabs 2 and 3 each print the final size once, not one line per drag step.
  3. Tab switch during a burst: resize, switch to tab 2 within 100 ms (`Cmd+2`). Expected: tab 2 is drawn at the new size.
  4. Styled text: `printf '\033[1mbold \033[3mbold italic\033[0m\n'` renders as before.
  5. `target/release/litty +bench-ram` and `target/release/litty +bench-pty-throughput 16`: no regression against `docs/development/performance.md`.

- [x] **Step 4 — record results.** Append a `## Fixes` section to `docs/validation/2026-11-11-perf-review.md` with: the commit of each task, the `SLOTMAP` before/after lines, the `REFLOW` before/after lines, the Step 3 observations, and every Step 3 item that could not run.

- [x] **Step 5 — final gate** (global rules): read the full diff for unrelated changes, leftover temporary tests and debug output; rerun Step 1 after the last edit; run the self-review checklist; get an independent review of the diff against this plan.

- [ ] **Step 6 — commit checkpoint.** Ready to commit: `docs/validation/2026-11-11-perf-review.md` — `docs(validation): record the perf review fixes`

- [x] **Step 7 — follow-up.** Leave the `ROADMAP.md` entry "Occlusion events are never emitted" unchecked: finding 2 is not fixed by this plan.

**Verification gaps the executor must report:**

- `sync_size` touching only the visible tab is checked by Step 3.2 only; the unit test covers `resize_tab` and `BackgroundResize`.
- Linux is compile-checked only unless a Linux machine with a display is available.
- Any Step 3 item skipped for lack of a display.
