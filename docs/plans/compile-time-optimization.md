# Rust Compile-Time Optimization Plan

**Status:** Mostly done. Local inner loop is already ~2 s; remaining items are
small, measured-first experiments (Task 3 in progress, Task 4 open). Heavy options (Cranelift,
workspace split) are parked behind explicit thresholds (Task 5).
**Scope:** Local dev build time, release build time, CI wall time.

---

## 1. Baseline (2026-11-07)

Measured on macOS, Apple Silicon, 8 cores, `rustc 1.99.1`, branch
`feat/native-windowing` (post wgpu/winit removal). Builds used a separate
`CARGO_TARGET_DIR` so the working cache was untouched.

| Scenario | Time |
|---|---|
| Clean dev build (`cargo build`) | 16.5 s |
| Incremental, edit `src/app/window.rs` | ~2.0 s |
| Incremental, edit `src/keymap.rs` | ~1.8 s |
| `cargo check` after touching a file | 3.4 s |
| Clean `--release` (fat LTO, `codegen-units = 1`) | 34.1 s |
| Clean `--profile fast-release` (thin LTO, 16 CGUs) | 16.9 s |

Dependency graph: 24 crates on macOS, 67 on Linux; `cargo tree -d` reports
no duplicate versions.

Slowest units in the clean dev build (`cargo build --timings`):

| Unit | Time |
|---|---|
| `litty` | 6.0 s |
| `syn` | 4.9 s |
| `cc` | 4.7 s |
| `libmimalloc-sys` (build script) | 3.4 s |
| `bytemuck_derive` | 2.0 s |
| `proc-macro2` | 1.5 s |

**Conclusion:** the edit–build loop is not a bottleneck. Only pursue items
below whose payoff is shown by a before/after measurement.

---

## 2. Completed

- **Dev profile tuning** (`Cargo.toml`): `debug = "line-tables-only"`,
  `split-debuginfo = "unpacked"`, dependencies at `opt-level = 2`.
- **`fast-release` profile**: thin LTO / 16 CGUs for local performance
  testing; halves release build time (34 s → 17 s).
- **Fast linker template**: `.cargo/config.toml.example` (mold on Linux,
  opt-in lld on macOS). Expected gain is small: Linux x86_64 already links
  with `rust-lld` by default since Rust 1.90, and Apple's ld-prime is
  already parallel.
- **Dependency diet**: removing `wgpu`, `winit`, `fontdue`, `rustybuzz`,
  `png` also removed every duplicate-version split the previous revision of
  this plan listed (`miniz_oxide`, `unicode-width`). Nothing left to dedupe.

## 3. Dropped

- **sccache**: with 24 crates and a ~2 s incremental build, cache hits save
  less than the wrapper overhead; CI already uses `Swatinem/rust-cache`.

---

## 4. Open Items

### Task 1: Document which profile to use
* **Problem:** `release` costs 34 s while `fast-release` costs 17 s; the
  difference is not documented.
* **Proposal:** Add a short note to `docs/development/building.md`: use `dev` for
  the inner loop, `fast-release` for local perf / RAM checks, `release`
  only for shipping and for `README` badge numbers (binary size and RAM can
  differ between the two).
* **Done when:** the note exists and links to this table.

### Task 2: Document `cargo build --timings`
* **Proposal:** One paragraph in `docs/development/building.md` on running
  `cargo build --timings` with a scratch `CARGO_TARGET_DIR` (never
  `cargo clean` on the working target dir) and reading the HTML report.
* **Done when:** documented; no helper script needed.

### Task 3: Drop the CI mold step (Linux job) — in progress
* **Problem:** `.github/workflows/ci.yml` ran `apt-get update` and
  installed `mold clang` on every Linux run. On a 67-crate graph with a warm
  `rust-cache`, this may cost more than mold saves over the default
  `rust-lld`.
* **With mold** (9 runs on 2026-11-07, warm cache, from the GitHub Actions
  API; medians, range in brackets):

  | Job / step | Median |
  |---|---|
  | `ubuntu-latest` job | 61 s [47–85] |
  | ↳ Install mold linker | 8 s [7–18] |
  | ↳ build tests (`cargo test --no-run`, includes linking) | 6 s [5–15] |
  | `macos-latest` job | 77 s [64–102] |

  The whole `build tests` step takes ~6 s, so mold can save at most a
  fraction of that over `rust-lld`, while installing it costs ~8 s.
* **Change:** step removed on branch `chore/ci-drop-mold`. `-D warnings`
  stays via the workflow-level `RUSTFLAGS`. Removing the mold flag changes
  `RUSTFLAGS`, so the first run after merge rebuilds the `rust-cache` (cold).
* **Expected effect:** ~8 s less per Linux job (runner minutes). PR wall
  time barely changes: the macOS job is the critical path in 8 of 9 runs.
* **Done when:** ≥ 3 warm-cache runs without mold are recorded here and
  the Linux job median is not slower than 61 s. Local Linux developers can
  still opt into mold via `.cargo/config.toml.example`.

### Task 4: Evaluate dropping `bytemuck_derive`
* **Problem:** `bytemuck`'s `derive` feature pulls `syn` + `proc-macro2` +
  `bytemuck_derive` (~8 s of the clean build's serial path) for 7
  `#[derive(Pod, Zeroable)]` sites.
* **Trade-off:** replacing derives with `unsafe impl Pod/Zeroable` removes
  the compile-time padding checks the derive performs. A wrong manual impl
  is undefined behaviour in GPU upload paths.
* **Scope of gain:** macOS only. On Linux `syn` is still pulled by
  `smithay-clipboard → smithay-client-toolkit → thiserror`
  (`cargo tree -i syn --target all`), so the Linux build would only lose
  `bytemuck_derive` itself.
* **Proposal:** Only if clean / CI build time matters:
  1. Re-run `cargo tree -i syn --target all` to confirm the picture above.
  2. Replace each derive with `unsafe impl` plus a
     `const _: () = assert!(size_of::<T>() == <sum of field sizes>);`
     per type to keep the no-padding guarantee.
  3. Measure clean build before/after.
* **Done when:** measured gain ≥ 3 s on the macOS clean build, or the task is
  dropped with the numbers recorded.

### Task 5: Parked — Cranelift and workspace split
Do **not** start these unless a trigger below is hit on the baseline
machine:

| Option | Trigger |
|---|---|
| `rustc_codegen_cranelift` (nightly, debug only) | incremental dev build > 5 s |
| Cargo workspace (`litty-terminal`, `litty-render`, `litty-platform`) | incremental dev build > 10 s, or `litty` crate > 30 s in a clean build |

Notes for when a trigger fires:
* Cranelift must stay opt-in; stable Rust remains the CI and release
  toolchain. Verify the native macOS (objc/Metal FFI) path works under it.
* A workspace split must follow the layers in `src/layering_tests.rs`,
  resolve every `TOLERATED` exception first, and keep public API between
  crates minimal. It is a large refactor and needs its own plan.

---

## 5. Measurement Protocol

For every task above:
1. Use a scratch target dir: `CARGO_TARGET_DIR=<scratch> cargo build --timings`.
   Never `cargo clean` the working target dir.
2. Incremental: append a comment to `src/app/window.rs`, rebuild, revert.
3. Record before/after numbers in this file; a task without numbers is
   not done.
4. Run the project verification steps (`cargo check`, Linux
   `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`,
   `cargo clippy --all-targets -- -D warnings`, `git diff --check`).
5. Confirm no runtime regression: idle RAM < 50 MB, `Cell` still 12 bytes.
