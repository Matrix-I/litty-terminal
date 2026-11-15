# v0.13.0 Release Validation

Status: locally verified release preparation for PR review; not tagged or published.
Source: `30154c0` plus v0.13.0 version metadata and documentation only.
No application source or dependency versions changed during preparation.
Record the final release commit SHA after the preparation PR merges, before tagging.

## Verification (2026-11-15, macOS arm64)

| Command | Result |
| --- | --- |
| `cargo check` | PASS |
| `cargo check --target x86_64-unknown-linux-gnu` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS |
| `cargo test` outside sandbox | PASS: 709 unit + 1 DSR + 14 shell tests, 12 ignored; native activation harness PASS |
| `bash scripts/pre-release-check.sh` outside sandbox | PASS, exit 0; release tests 709 + 1 + 14, 12 ignored; native activation PASS |
| `bash bundle_macos.sh --no-install` | PASS; app, ZIP and DMG; no installation |
| `cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.34` | PASS; pinned public key supplied |
| `git diff --check` | PASS before independent review; final gate recorded in release preparation plan |

Shell test caveat: the release run reports fish unavailable and skips its three
bodies although the harness reports 14 passes; a focused debug rerun with `cargo test --test yo_shells -- --nocapture`
confirmed the same three skips (fish is not installed).
The 12 ignored tests were not enabled; passing suites do not establish their coverage.

## Measurements and artifact inspection

- RAM suite: 58 MB (1w/1t), 74 MB (1w/3t), 122 MB (2w/6t),
  45 MB after cleanup to one window/tab; no positive retained-footprint delta.
- PTY DSR: mean 0.016 ms, P99 0.022 ms.
- Stress: 747,692 lines/s; worst chunk stall 1.17 ms; zero chunks over 16 ms.
- FZF 10k: 1.277 ms; regex 5k: 0.130 ms.
- Packaged app with isolated HOME/XDG config and `SHELL=/usr/bin/true`:
  shell exits and window closes; after 10 s settling, CPU time
  0:00.23 → 0:00.24 over 30.020 s (~0.033% of one core), `ps` displays 0.0%.
  `footprint` exits 0: phys_footprint 20 MB, peak 23 MB. Single sample.
- macOS binary: 2,338,368 bytes; Linux x86_64 binary: 4,733,064 bytes.
  Both below the 5,000,000-byte budget; both contain `0.13.0` and the pinned key.
- App ZIP: both CFBundle versions equal `0.13.0`.
- Local macOS-only checksum manifest signature verifies against
  `resources/minisign.pub`. It is not the final combined release manifest.
- Prepared assets and release notes are in ignored `dist/v0.13.0/`.

## Known limitations and remaining publication gates

- Docker/Podman daemon unavailable; inspected Podman VM configuration is 1 GiB,
  below the documented 4 GiB suite prerequisite. Linux runtime smoke suite and
  Debian/RPM packaging/install/remove were not rerun locally. The release CI
  must build/test both packages before publishing its draft.
- Combined six-asset checksum manifest and signature are not yet generated;
  CI must produce and verify all eight release assets. Download/update checks
  require the release to exist and remain post-publication work.
- Latest Linux cross-build compiles but was not launched on a real Linux host.
  Earlier GNOME/KDE/Weston/sway validation remains historical, not a final-build run.
- No new full desktop notification/reuse or physical input run; prior maintainer
  evidence is in the linked macOS and Wayland reports. The live benchmark windows
  and native activation harness were exercised by the current suites.
- Forwarded notification foregrounding on macOS 14+, old-macOS/cross-UID runtime
  checks and remaining Wayland acceptance move to v0.14.0. See the
  [carryover plan](../plans/0.14.0.md), which also preserves Linux rasterization,
  shared-memory, APC retention, benchmark and architecture follow-ups.
- Startup/input-latency/PTY-throughput figures and additional AI provider/old
  Nushell coverage were not re-measured. Older tables remain dated historical data.

## Generated suite report

# Litty Terminal — Pre-Release Performance & Stability Verification Report

- **Execution Date**: 2026-11-15 23:48:50 +0700
- **Platform**: Darwin arm64 (Kernel 27.2.0)
- **Git Commit**: 30154c0
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
