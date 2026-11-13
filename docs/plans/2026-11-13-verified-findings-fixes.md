# Verified Audit Findings Fix Plan

**Status:** Planned (unscheduled; linked from [v0.13.0](./0.13.0.md)).
**Baseline:** `main` at `cc7e046` (clean tree, 2026-11-13, audited on macOS).
**Scope:** Only findings confirmed as True Positives or requiring Linux runtime verification. Refuted speculative claims from earlier scans are recorded under [Verified Non-Issues](#verified-non-issues-do-not-fix) so they are not revisited.

## Global Constraints

- `CLAUDE.md` / `AGENTS.md` apply in full: 12-byte `Cell`, no blocking I/O under grid locks, no hot-path heap allocations, macOS/Linux parity, no new dependencies.
- Release builds set `panic = "abort"` (`Cargo.toml:40`), so any reachable panic terminates the terminal process.
- Ask the user before altering user-facing config validation errors/limits (`adjust-cell-width`, `adjust-cell-height`, `font-size`) if clamping changes semantics or breaks existing config files.
- Required verification gate after any fix commit/PR: `cargo check`, `cargo check --target x86_64-unknown-linux-gnu`, `cargo test` (outside sandbox), `cargo clippy --all-targets -- -D warnings`, `git diff --check`.

---

## Task 1: Bound `MetricAdjust` and `Atlas` cell dimensions (Medium)

**Files:** `src/config.rs`, `src/render/atlas/mod.rs`, `src/render/atlas/tests.rs`, `src/app/window.rs`, `docs/guide/configuration.md` (if config bounds are documented/changed).

**Baseline evidence:**
- `src/config.rs:497-502` parses `adjust-cell-width` and `adjust-cell-height` via `MetricAdjust::parse` (`src/render/atlas/mod.rs:57-64`), which accepts any `i32` pixel offset or any finite `f32` percentage.
- `Atlas::new` (`src/render/atlas/mod.rs:251-262`) caps the atlas texture side `size` at `4096`, then computes `slots_per_row = size / cell_w` and `n_slots = (slots_per_row * (size / cell_h)).min(u16::MAX as u32)`.
- When `cell_w > 4096` (e.g., `adjust-cell-width = 4097` or `50000%`), `slots_per_row = 0` and `n_slots = 0`. `Atlas::preload_ascii` (`src/render/atlas/mod.rs:411-412`) runs `total_slots.div_ceil(self.slots_per_row)` (`0u32.div_ceil(0)`), panicking with `attempt to divide by zero` (aborting in release builds).
- Huge `cell_w` and `cell_h` also overflow `u32` at `src/render/atlas/mod.rs:288,295-297` (`cell_w * cell_h`, `* 4`, `2 * cell_w * cell_h * 4`), panicking in debug builds or allocating truncated scratch buffers in release builds.
- Note on reachability: values come only from `~/.litty/config`, not PTY escape sequences. Reached via startup and config reload (`src/app/window.rs:35,460-469`). Also note `window.rs:65` clamps initial `cfg.font_size.clamp(6.0, 72.0)`, while `reload_config` (`window.rs:465`) and `ZoomReset` (`window.rs:647`) copy `font_size` (`1.0..=255.0` per `config.rs:343`) directly into `s.font_pt`.
- Reproduced with standalone logic mirror (`test_atlas_div0.rs` matching `MetricAdjust::apply` / `Atlas::new` / `preload_ascii`): release binary `-O -C panic=abort` exited `134` (`attempt to divide by zero`) for `cell_w = 4100`; debug binary with `i32::MAX` exited `101` (`attempt to multiply with overflow`). Live GPU `Atlas::new` launch was not run.

**Implementation steps:**
- [ ] Decide safe upper limit for cell pixel dimensions in an atlas (max atlas texture is `4096 × 4096`; wide-glyph scratch is `2 * cell_w * cell_h * 4` bytes). Clarify whether `MetricAdjust::parse` should reject extreme config values with a config error or clamp `cell_w`/`cell_h` inside `Atlas::new` / `MetricAdjust::apply` (or both, matching `font-size` range rejection in `src/config.rs:343`).
- [ ] Extract a pure cell/slot geometry helper in `src/render/atlas/mod.rs` that checks/clamps `cell_w` and `cell_h`, uses checked multiplications for scratch buffer sizes, and guarantees `slots_per_row >= 1` and `n_slots >= 1` (plus an early `if self.slots_per_row == 0 || total_slots == 0 { return; }` guard in `preload_ascii` as defense-in-depth).
- [ ] Review `font_pt` clamping consistency between `build_window` (`window.rs:65`), `reload_config` (`window.rs:465`), and `zoom` (`window.rs:647`).
- [ ] Add unit tests in `src/render/atlas/tests.rs` (testing the pure geometry helper without mutating global `CELL_ADJUST`, plus an `Atlas::new` integration test at boundary sizes).

---

## Task 2: Handle negative `bmp.pitch` row order and checked alloc in `rasterize_color` (Low)

**Files:** `src/render/atlas/linux.rs`

**Baseline evidence:**
- Direct C PoC against FreeType (`libfreetype.dylib`: `FT_Bitmap_Copy`, `FT_Bitmap_Embolden` with `pitch = -4`) proved `bmp.buffer` always points to the lowest address of the bitmap buffer, so `bmp.buffer.add(y * bmp.pitch.unsigned_abs())` (`src/render/atlas/linux.rs:763-766`) does **not** read out-of-bounds.
- `rasterize_color` only opens the local system `"Noto Color Emoji"` face (`linux.rs:720-721`), not untrusted PTY input.
- Remaining minor gap: unlike `blit_ft_bitmap` (`linux.rs:594-598`), `rasterize_color` (`linux.rs:766`) does not reverse row indexing when `bmp.pitch < 0` (resulting in a vertically flipped glyph if a strike uses negative pitch), checks `sw * sh * 4` (`linux.rs:764`) without `checked_mul`, and assumes `(sw as usize) * 4 <= pitch`.

**Implementation steps:**
- [ ] Extract a pure BGRA-to-RGBA conversion helper in `src/render/atlas/linux.rs` (or a shared cross-platform helper if appropriate, keeping `layering_tests.rs` rules intact) that checks `sw.checked_mul(sh)` / `checked_mul(4)`, validates `(sw as usize) * 4 <= pitch`, and indexes row `if bmp.pitch >= 0 { y } else { sh as usize - 1 - y }`.
- [ ] Add unit tests covering positive pitch, negative pitch, and dimension overflow without requiring a live X11/Vulkan session.

---

## Task 3: Evaluate Kitty `t=s` `read()` vs `mmap()` on Linux `/dev/shm` (Low)

**Files:** `src/terminal/graphics/kitty.rs`, `src/terminal/graphics/tests.rs`

**Baseline evidence:**
- `read_shm_medium` (`src/terminal/graphics/kitty.rs:417-454`) maps POSIX shared memory with `MAP_SHARED` for both macOS and Linux.
- Tested on macOS (`test_shm_sigbus.c`): macOS rejects `ftruncate(fd, 0)` on the open SHM object with `EINVAL` (read succeeds, `exit=0`), and `MAP_PRIVATE` fails immediately on `mmap` with `EINVAL` (`exit=1`) — **keep `MAP_SHARED` on macOS**.
- Linux `/dev/shm` (`tmpfs`) was **not** tested on this macOS host (`cargo check --target x86_64-unknown-linux-gnu` passed, Linux binary not executed). On Linux, a hostile same-UID process truncating the SHM object between `fstat` and `slice.to_vec()` could trigger `SIGBUS` under either `MAP_SHARED` or `MAP_PRIVATE` (same-UID threat model already has `ptrace`/`kill`).

**Implementation steps:**
- [ ] On a real Linux host, verify two behaviors first: (1) whether concurrent `ftruncate(fd, 0)` on `/dev/shm/*` induces `SIGBUS` on `mmap`, and (2) whether `read()` / `pread()` on `shm_open` descriptors works cleanly on Linux `tmpfs`.
- [ ] If supported cleanly on Linux, gate `mmap(..., MAP_SHARED, ...)` under `#[cfg(target_os = "macos")]` and use `#[cfg(not(target_os = "macos"))]` bounded `File::from_raw_fd(fd)` + `read_range` (already at `kitty.rs:511-519`) so Linux avoids mapping externally shrinkable `/dev/shm` objects.
- [ ] Add a Linux-only test in `src/terminal/graphics/tests.rs` exercising `t=s` when the writer truncates the object concurrently.

---

## Task 4: Confirm or trim `apc_buf` 64 KiB capacity retention (Low)

**Files:** `src/terminal/grid/images.rs`, `src/terminal/grid/mod.rs`

**Baseline evidence:**
- `apc_dispatch` (`src/terminal/grid/images.rs:33-43`) drops `apc_buf` when `capacity() > 65536`, otherwise retains up to 64 KiB until the next `ESC _` (`src/terminal/grid/mod.rs:839` clears it with `self.apc_buf.clear()` before reuse). Zero data leakage or correctness impact.

**Implementation steps:**
- [ ] Benchmark/confirm whether retaining `<= 64 KiB` prevents allocator churn during multi-chunk Kitty transfers; if not needed on `reset()` / alt-screen exit or after `m=0` completion, shrink/clear when idle without adding hot-path allocations.

---

## Verified Non-Issues (Do Not Fix)

These items from prior scans were verified on `HEAD` (`cc7e046`) and confirmed **False Positive** or already fixed:

- `src/render/atlas/linux.rs:763-766` OOB read / remote DoS claim: refuted against FreeType (`bmp.buffer` points to lowest address even when `pitch < 0`; only local system `"Noto Color Emoji"` is opened).
- `src/terminal/graphics/kitty.rs:432` `MAP_SHARED` -> `MAP_PRIVATE` recommendation on macOS: switching to `MAP_PRIVATE` breaks Kitty `t=s` on macOS (`mmap` fails with `EINVAL`).
- `src/terminal/graphics/animation.rs:258` `Instant::sub` panic claim: refuted by PoC (`test_anim_panic.rs` with `offset_ms = 549755813672` / `z = i32::MAX`, `exit=0`).
- `src/terminal/graphics/kitty.rs:251-263` (`d=I` ignoring `p=`): already fixed on `HEAD` (`image_id == id && (pid == 0 || placement_id == pid)`).
- `src/app/gui_bench.rs:125-135` (occlusion check race): already fixed on `HEAD` (`tick()` calls `set_occluded` and checks `occlusion_error()` before and after `ready = true`).
