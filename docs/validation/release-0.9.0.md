# Litty Terminal — Pre-Release Performance & Stability Verification Report

Archived v0.9.0 release report; original results and limitations are preserved.

Measured figures from this run (macOS arm64):

- Footprint: 75 MB (1w/1t), 77 MB (1w/3t), 126 MB (2w/6t), 65 MB after close (delta -10 MB)
- PTY DSR roundtrip P99: 0.051 ms
- 20k-line stress: 427,560 lines/s, 0 frame stalls (worst 0.96 ms)
- FZF 10k search 1.671 ms; regex 5k scan 0.197 ms
- Binary size: 6,310,336 bytes (6.3 MB)

- **Execution Date**: 2026-11-05 20:42:49 +0700
- **Platform**: Darwin arm64 (Kernel 27.2.0)
- **Git Commit**: efe0eee + version bump and release docs, committed as the v0.9.0 release commit (`git rev-parse v0.9.0^{commit}`; no code changed afterwards)
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
  - Average: $\le$ 1.0 ms
  - P99: $\le$ 3.0 ms
- **File Reading Ingestion**:
  - 4 KB Config: $\le$ 1.0 ms
  - 64 KB Source code: $\le$ 15.0 ms
  - 1 MB Data stream: $\le$ 150 ms
- **Image & Extras Stream Ingestion**:
  - 256 KB Kitty Graphics APC: $\le$ 30.0 ms
  - 256 KB Sixel Graphics DCS: $\le$ 30.0 ms
  - Saturated Extras GC (500 distinct URLs): $\le$ 30.0 ms
- **Process Lifecycle & Unicode Layout**:
  - PTY Shell Spawn (macOS/Linux): $\le$ 25.0 ms
  - BiDi Grapheme Cluster Fast-Reject (500 rows): $\le$ 10.0 ms
- **Status**: **PASS**

## 3. Large Log Stress Test & Frame Stalls
- **Test Workload**: 20,000 lines of standard ANSI server logs (ISO timestamps, colors, log levels, JSON payloads).
- **Throughput**: $\ge$ 20,000 lines/sec.
- **Frame Stalls (> 16.6 ms)**: Zero severe stalls, Worst stall $\le$ 25 ms.
- **Post-Stress Responsiveness (Unblock Probe)**: $\le$ 1.0 ms (terminal immediately responsive, no lockups/hangs).
- **Status**: **PASS**

## 4. Critical Features Regression
- **FZF Fuzzy History Search**: Query over 10,000 commands ($\le$ 2.5 ms / query).
- **Find in Terminal (Regex Search)**: Regex scan across 5,000 scrollback lines ($\le$ 1 ms).
- **Split Panes Engine**: 1,000 vertical/horizontal splits, resize, zoom, and close operations without panicking.
- **Bracketed Paste Security**: Escape injection mitigation (`\x1b[201~`) and safe shell quotation.
- **Status**: **PASS**

---
*Report generated automatically by `scripts/pre-release-check.sh`.*
