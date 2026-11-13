# Litty Terminal — Pre-Release Performance & Stability Verification Report

Archived v0.12.0 release report. `scripts/pre-release-check.sh` exited 0 on
2026-11-13 (Darwin arm64, kernel 27.2.0, Apple M1 Pro), on `4a94cad` plus
the version bump and release docs only (no code change). 1-minute load
average 5–17 during the runs.

Measured figures from the suite:

- Footprint: 63 MB (1w/1t), 64 MB (1w/3t), 97 MB (2w/6t), 50 MB after close
  (delta -13 MB, reported as 0 MB)
- PTY DSR roundtrip: mean 0.028 ms, P99 0.062 ms
- 20k-line stress: 493,077 lines/s, worst frame stall 0.93 ms
- FZF 10k search 1.665 ms; regex 5k scan 0.166 ms
- `cargo test --release`: 644 + 1 + 14 passed, 0 failed, 12 ignored
- Binary size: 2,234,832 bytes (2.2 MB)

Measured separately (release binary, empty `HOME`):

- Idle CPU, 1 window: 0.0% (CPU time 0:00.26 → 0:00.27 over 30 s);
  footprint 41 MB with a `sleep` shell
- All windows closed (`SHELL` runs `sleep 3; exit 0`, `footprint` 23 s
  after launch): 32, 32, 31, 31, 31 MB in 5 runs

Other release checks (debug build): `cargo check`,
`cargo check --target x86_64-unknown-linux-gnu`,
`cargo clippy --all-targets -- -D warnings`, `git diff --check`: clean;
`cargo test`: 659 passed (644 + 1 + 14), 0 failed, 12 ignored.

Linux: the maintainer verified every `fix(linux)` commit of this release on
real Linux hardware (2026-11-13).

Not run:

- Linux container smoke test (`scripts/linux/test.sh`): the local Podman VM
  has 1 GiB of memory and the script needs at least 4 GiB. `.deb` / `.rpm`
  packaging and install/remove run in the release workflow.
- `+bench-input-latency`: no valid figure. Runs failed with "no frame
  showed the echo" whenever the benchmark window was covered (another app
  or Space in front): a debug build logged `Occluded(true)` in each failing
  run and `Occluded(false)` only in the 11 passing runs, and an occluded
  window no longer draws since `dc766fa`. Builds before `dc766fa` passed
  8 of 8 runs. A benchmark limitation, not an app bug; tracked in
  `docs/plans/0.13.0.md`. `performance.md` keeps the v0.11.1 figure.
- `+bench-pty-throughput`: not re-measured.

Startup (`+bench-first-frame`, alternating with a v0.11.1 build, load
3–12): v0.12.0 p50 216–219 ms, v0.11.1 212–214 ms; two earlier v0.12.0
runs of 10 gave 213 and 214 ms. No regression beyond noise; the v0.11.1
release figure (162–169 ms) was taken on a differently loaded machine.

---

Generated report:

- **Execution Date**: 2026-11-13 07:26:16 +0700
- **Platform**: Darwin arm64 (Kernel 27.2.0)
- **Git Commit**: 4a94cad
- **Automated Suite Status**: ✅ **ALL EXECUTED CHECKS PASS**

This suite uses the budgets in src/bench.rs. Passing it does not establish
compliance with AGENTS.md's stricter <50 MB idle RAM requirement, verify idle
CPU usage, or replace the outstanding Linux manual checks in ROADMAP.md.

---

## 1. Memory Profile (RAM Footprint)
- **1 Window, 1 Tab (Idle Baseline)**: Measures cold start footprint at idle state.
- **1 Window, 3 Tabs**: Measures memory scaling when opening multiple tabs in a single window.
- **2 Windows, 6 Tabs (3 tabs per window)**: Measures multi-window scalability and multi-GPU swapchains.
- **Post-cleanup (Tabs closed)**: Checks retained footprint growth (delta $\le$ 10 MB).
- **Suite footprint budgets**: 85 MB (1w/1t), 95 MB (1w/3t), 145 MB (2w/6t).
- **Status**: **PASS**

## 2. Interactive Latency & I/O
- **Command Execution Roundtrip (PTY Latency)**:
  - Average: $\le$ 0.05 ms
  - P99: $\le$ 0.4 ms
- **File Reading Ingestion**:
  - 4 KB Config: $\le$ 1.0 ms
  - 64 KB Source code: $\le$ 3.0 ms
  - 1 MB Data stream: $\le$ 40 ms
- **Image & Extras Stream Ingestion**:
  - 256 KB Kitty Graphics APC: $\le$ 3.0 ms
  - 256 KB Sixel Graphics DCS: $\le$ 5.0 ms
  - Saturated Extras GC (500 distinct URLs): $\le$ 4.0 ms
- **Process Lifecycle & Unicode Layout**:
  - PTY Shell Spawn (macOS/Linux): $\le$ 6.0 ms
  - BiDi Grapheme Cluster Fast-Reject (500 rows): $\le$ 0.2 ms
- **Status**: **PASS**

## 3. Large Log Stress Test & Frame Stalls
- **Test Workload**: 20,000 lines of standard ANSI server logs (ISO timestamps, colors, log levels, JSON payloads).
- **Throughput**: $\ge$ 350,000 lines/sec.
- **Frame Stalls (> 16.6 ms)**: Zero severe stalls, Worst stall $\le$ 3 ms.
- **Post-Stress Responsiveness (Unblock Probe)**: $\le$ 0.05 ms (terminal immediately responsive, no lockups/hangs).
- **Status**: **PASS**

## 4. Critical Features Regression
- **FZF Fuzzy History Search**: Query over 10,000 commands ($\le$ 2.0 ms / query).
- **Find in Terminal (Regex Search)**: Regex scan across 5,000 scrollback lines ($\le$ 0.5 ms).
- **Split Panes Engine**: 1,000 vertical/horizontal splits, resize, zoom, and close operations without panicking.
- **Bracketed Paste Security**: Escape injection mitigation (`\x1b[201~`) and safe shell quotation.
- **Status**: **PASS**

---
*Report generated automatically by `scripts/pre-release-check.sh`.*
