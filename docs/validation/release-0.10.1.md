# Litty Terminal — Pre-Release Performance & Stability Verification Report

Archived v0.10.1 release report; original results and limitations are preserved.

Measured figures from this run (macOS arm64):

- Footprint: 72 MB (1w/1t), 73 MB (1w/3t), 107 MB (2w/6t), 59 MB after close (delta -13 MB)
- PTY DSR roundtrip P99: 0.052 ms
- 20k-line stress: 623,270 lines/s, 0 frame stalls (worst 0.82 ms)
- FZF 10k search 1.292 ms; regex 5k scan 0.125 ms
- Binary size: 2,118,624 bytes (2.1 MB)

Measured separately (release binary, `footprint` and `ps`, after the shell
exited and the last window closed, app idle in the Dock):

- Idle footprint: 46–48 MB (47 MB at 30 s, 46 MB at 90 s and 180 s); RSS 104 MB
- Idle CPU: 0.0% (`top -stats cpu,idlew` over 10 s: 0.0%, 0 idle wake-ups;
  CPU time 0:00.30 → 0:00.40 over 150 s)
- The v0.10.0 release binary (`litty-macos-aarch64`) measured the same way on
  the same host gives 47–48 MB, so the change from the ~33 MB published for
  v0.10.0 comes from the measurement environment, not from this release. The
  README badge and `docs/development/performance.md` now show the figure
  measured here.

Other release checks:

- `cargo check`, `cargo check --target x86_64-unknown-linux-gnu`,
  `cargo clippy --all-targets -- -D warnings`, `git diff --check`: clean.
- `cargo test`: 527 passed, 0 failed, 11 ignored (rerun on `2d2bbe9`, which
  includes `48357a6` and its two regression tests).
- The pre-release suite and the idle measurements above ran on `9768a8f`,
  before `48357a6` (macOS function-key text) landed. That fix only changes
  the text reported for AppKit function keys; the maintainer tested it
  manually and it was not rerun through the suite.
- `.deb` (Debian 13) and `.rpm` (Fedora 43) packaging, package tests and
  install/remove round trips run in the tag-triggered release workflow
  (`.github/workflows/release.yml`) instead of locally.
- Not run: `scripts/linux/test.sh` (Linux runtime smoke test); all three fixes in
  this release are macOS-only code paths. CI release workflow runs on
  `v0.10.1-rc.1` are the reference for the packaging checks. Launching the packaged desktop
  entries in a desktop session and native Wayland were not run either.

- **Execution Date**: 2026-11-08 22:14:28 +0700
- **Platform**: Darwin arm64 (Kernel 27.2.0)
- **Git Commit**: 9768a8f (suite run); release code is `2d2bbe9` = 9768a8f + `48357a6` + version bump and release docs (`git rev-parse v0.10.1^{commit}` adds only this record update)
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
