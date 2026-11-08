# Litty Terminal — Pre-Release Performance & Stability Verification Report

Archived v0.10.0 release report; original results and limitations are preserved.

Measured figures from this run (macOS arm64):

- Footprint: 72 MB (1w/1t), 73 MB (1w/3t), 107 MB (2w/6t), 59 MB after close (delta -13 MB)
- PTY DSR roundtrip P99: 0.053 ms
- 20k-line stress: 578,174 lines/s, 0 frame stalls (worst 0.93 ms)
- FZF 10k search 1.286 ms; regex 5k scan 0.123 ms
- Binary size: 2,118,624 bytes (2.1 MB)

Measured separately (release binary, `footprint` and `ps`, after the shell
exited and the last window closed, app idle in the Dock):

- Idle footprint: 33 MB (66 MB with 1 window open); RSS 104 MB
- Idle CPU: 0.0% (`top -stats cpu,idlew` over 10 s: 0.0%, 0 idle wake-ups;
  CPU time unchanged at 0:00.38 over 30 s)

Other release checks:

- `scripts/linux/test.sh` (Podman, Debian 13 aarch64, Xvfb/Openbox, software
  Vulkan): `cargo check`, clippy `-D warnings`, 493 passed / 12 ignored, PTY
  smoke and X11 wheel, keysym, window-close and lock-modifier probes passed.
- `.deb`: `package-deb.sh` and `test-package-deb.sh` passed in a Debian 13
  amd64 container; `apt install` placed `/usr/bin/litty` (byte-identical to
  `litty-linux-x86_64`) and the launcher; `apt remove` removed both.
- `.rpm`: `package-rpm.sh` and `test-package-rpm.sh` passed in a Fedora 43
  amd64 container; `dnf install`, `rpm -V`, `cmp` and `dnf check` passed;
  `dnf remove` removed the binary and launcher.
- Not run: launching the packaged desktop entries in a desktop session, the
  x86_64 binary outside the package tests, and native Wayland (no backend).

- **Execution Date**: 2026-11-08 15:50:54 +0700
- **Platform**: Darwin arm64 (Kernel 27.2.0)
- **Git Commit**: 07c737f + version bump and release docs, committed as the v0.10.0 release commit (`git rev-parse v0.10.0^{commit}`; no code changed afterwards)
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
