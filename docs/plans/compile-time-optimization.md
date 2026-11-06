# Rust Compile-Time Optimization Plan

**Status:** Draft / Planned  
**Scope:** Build performance improvements, fast linking, dependency deduplication, Cargo profile tuning, and potential multi-crate modularization.

---

## 1. Context & Baseline

While `litty` already adopts exceptional practices to minimize compile overhead (zero heavy dependencies like `tokio`, `serde`, or `clap`; custom bounded JSON parser; minimal generics; hand-rolled CLI parsing; and `debug = "line-tables-only"`), several compile-time optimizations remain unapplied.

This document collects all remaining items to accelerate local development inner loops and CI build times.

---

## 2. Action Items

### Task 1: Fast Linker Setup (`.cargo/config.toml`)
* **Problem:** Default system linkers (`ld` on Linux, `ld64` on macOS) are single-threaded or slow when merging object files and debug symbols.
* **Proposal:** Provide a `.cargo/config.toml` (or documented template `.cargo/config.toml.example`) configuring fast linkers:
  ```toml
  # Linux: mold or lld
  [target.x86_64-unknown-linux-gnu]
  linker = "clang"
  rustflags = ["-C", "link-arg=-fuse-ld=mold"]

  # macOS (Apple Silicon on Rust 1.82+ with lld available)
  [target.aarch64-apple-darwin]
  rustflags = ["-C", "link-arg=-fuse-ld=lld"]
  ```
* **Notes:** Verify CI compatibility before committing directly to `.cargo/config.toml` so container/runner environments without `mold` or `clang` do not fail.

---

### Task 2: Profile Dev Tuning (`Cargo.toml`)
* **Problem:** 
  1. On macOS, debug symbol bundling via `dsymutil` can add latency if debug info is packed.
  2. Unoptimized third-party dependencies (`wgpu`, `winit`, `fontdue`, `rustybuzz`) run slower in debug builds, slowing down manual testing loops.
* **Proposal:** Add to `Cargo.toml`:
  ```toml
  [profile.dev]
  debug = "line-tables-only"
  split-debuginfo = "unpacked" # Prevent dsymutil / DWARF packaging bottlenecks

  # Compile external dependencies with opt-level 2 so the app remains fast in dev,
  # while keeping crate-internal code fast to compile with opt-level 0.
  [profile.dev.package."*"]
  opt-level = 2
  ```

---

### Task 3: Dependency Deduplication & Cleanup (`cargo tree -d`)
* **Problem:** `cargo tree -d` identifies minor version splits among transitive dependencies:
  * `miniz_oxide`: `0.8.9` (root `Cargo.toml`) vs `0.9.1` (pulled by `png -> flate2`).
  * `unicode-width`: `0.2.3` (root `Cargo.toml`) vs `0.1.14` (pulled by `wgpu -> naga -> codespan-reporting`).
* **Proposal:**
  * Bump `miniz_oxide = "0.9"` in `Cargo.toml` to unify on version 0.9 and eliminate compiling two copies of `miniz_oxide`.
  * Periodically run `cargo machete` and `cargo tree -d` to keep crate counts minimal.

---

### Task 4: Caching & Tooling Enhancements
* **Problem:** Local incremental builds rebuild dependencies when switching git branches or cleaning.
* **Proposal:**
  * Document `sccache` setup for local contributors (`export RUSTC_WRAPPER=sccache`).
  * Add a helper script or documentation for running `cargo build --timings` to identify build-pipeline bottlenecks and visualize crate compilation graphs.

---

### Task 5: Cranelift Backend Exploration (Debug Inner Loop)
* **Problem:** LLVM code generation is the heaviest stage of compilation.
* **Proposal:**
  * Evaluate `rustc_codegen_cranelift` with nightly toolchain (`cargo +nightly build -Zcodegen-backend=cranelift`) for rapid iterative testing of local UI changes.
  * Keep stable Rust as the mandatory release / CI toolchain.

---

### Task 6: Architectural Workspace Modularization (Long-term)
* **Problem:** `litty` is currently a single monolith crate. A single line edit in root modules requires re-analyzing the internal dependency graph.
* **Current status:** The codebase is already divided into strict, decoupled layers verified by `src/layering_tests.rs`:
  * `src/terminal/` (emulation core: grid, pty, vt parsing)
  * `src/render/` (GPU shaders, atlas, text layout)
  * `src/platform/` (OS windowing, clipboard, sockets)
  * `src/app/` (event loop and orchestration)
* **Proposal:** If compile times grow as features expand, migrate to a Cargo workspace:
  * `crates/litty-terminal`
  * `crates/litty-render`
  * `crates/litty-platform`
  * Root `litty` (binary)
  * *Benefit:* Cargo can compile `litty-terminal` and `litty-render` in parallel across CPU cores, and edits to UI/app code will not invalidate core terminal compilation.

---

## 3. Verification & Metrics Checklist

When implementing tasks from this plan:
1. Benchmark baseline clean build: `cargo clean && cargo build --timings`
2. Measure incremental build time after touching `src/app/window.rs`
3. Verify that all platforms pass checks:
   * `cargo check`
   * `cargo check --target x86_64-unknown-linux-gnu`
   * `cargo clippy --all-targets -- -D warnings`
   * `cargo test`
4. Confirm zero regressions on runtime memory footprint (< 50MB idle RAM) and 12-byte `Cell` invariant.
