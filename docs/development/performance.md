# Performance & Memory Footprint

When the 65,536-entry grapheme table fills, collection retries are bounded to
avoid scanning scrollback for every new cluster. Existing clusters remain
available; new clusters may temporarily lose combining marks until a later
collection reclaims unused entries.

## macOS

Measured on 2026-11-13: Litty 0.12.0 release build (`cargo build --release`),
Apple M1 Pro, macOS 27.2, one 2590 × 1492 px Retina window.

| Metric | Result | How it was measured |
| :--- | :---: | :--- |
| Idle footprint, 1 window | **63 MB** | `litty +bench-ram`, step 1 (1 window, 1 tab, idle shell) |
| Idle RSS, 1 window | 100 MB | Same run |
| Idle footprint, all windows closed | **31 MB** (31–32 MB) | Shell exits so the only window closes; `footprint` about 20 s later, 5 runs |
| Idle CPU | **0.0%** | 1 window, idle shell: no CPU time used over 30 s (`ps -o time`) |
| Startup to first frame | **213–219 ms** p50 warm (5 runs of 6–10, load 3–12; v0.11.1 gave 211–214 ms alternating in the same conditions) | `litty +bench-first-frame 10`: process spawn → GPU-completed first frame |
| Keystroke to frame | **6.6–7.1 ms** p50, 15.8–16.0 ms p95 (v0.11.1 figures, not re-measured: the benchmark window was occluded during the v0.12.0 runs and an occluded window no longer draws; 3 runs, machine under load; v0.11.0 measured 6.35–6.75 ms p50 alternating in the same conditions, 3.1–3.6 ms on an idle machine) | `litty +bench-input-latency`: synthetic key → GPU-completed frame showing the echo; excludes the OS input stack and display scanout |
| Binary size | **< 5 MB** (budget) | `target/release/litty`: 2.2 MB on macOS for 0.12.0; 4.3 MB on Linux x86_64 on 2026-11-13, of which about 1.4 MB is `naga` (custom shaders) |

Two drawables at Retina size (15 MB each) make up about half of the
1-window footprint; they are freed when the window is occluded or closed.
Startup is dominated by `init` (~175–189 ms p50 under load: font loading and the Metal
device, overlapped). Comparisons with other terminals are not listed: they
have not been measured on this setup (macOS); see the Linux section for Ptyxis.

> [!NOTE]
> **Measuring Memory Footprint on macOS**: macOS Activity Monitor displays **Physical Footprint** (`task_vm_info.phys_footprint`: actual private dirty memory + compressed memory), which accurately reflects the real memory cost attributed to the process. The legacy Unix `ps -o rss` metric includes shared read-only framework pages (Metal, AppKit, and system dylib caches) shared across all macOS GUI applications.
>
> Run from the raw binary outside a benchmark, Litty also carries an 8 MB Dock icon bitmap that a bundled `Litty.app` does not; `+bench-ram` skips it so it measures what bundled users get.

## Linux

Measured on 2026-11-13: Litty release build from `feat/wayland-window`
(`cargo build --release`), Intel Core i5-8365U, UHD Graphics 620 (Mesa, Vulkan),
Ubuntu, kernel 7.0.0, GNOME Wayland session (window through the existing
backend). Load average about 0.5.

| Metric | Result | How it was measured |
| :--- | :---: | :--- |
| Idle RSS, 1 window, 1 tab | **66.8 MB** | `litty +bench-ram`, step 1 (on Linux the benchmark's "footprint" equals RSS) |
| Idle RSS, 1 window, 3 tabs | 67.1 MB | Same run, step 2 |
| Idle RSS, 2 windows, 6 tabs | 68.0 MB | Same run, step 3 |
| After windows closed | 68.0 MB (leak delta +1.2 MB) | Same run, step 4: RSS does not shrink on Linux, unlike the macOS footprint |
| Idle PSS, 1 window, 1 tab | **20–22 MB** | `Pss` from `/proc/<pid>/smaps_rollup`, 6–40 s after launch. PSS splits shared libraries (Mesa, Wayland, libc) among the processes using them, so it is the fairer number next to RSS |
| Idle CPU | **~0.0%** | 1 window, idle shell: 1 tick (10 ms) of CPU over 30 s (`/proc/<pid>/stat`) |
| Startup to first frame | **63.5–73.8 ms** (mean 68.1, p50 67.9) | `litty +bench-first-frame 10`, 9 runs after the first (65.6 ms); `init` about 48 ms, `gpu` about 12 ms |
| Keystroke to frame | not measured | `litty +bench-input-latency` did not finish within 100 s on this setup; to be investigated |
| Binary size | **4.3 MB** | `target/release/litty` (x86_64), about 1.4 MB of it is `naga` |

RSS counts shared pages, so it overstates the cost of a GUI process on Linux;
use PSS when comparing with other terminals.

### Comparison with the Ubuntu default terminal

Ptyxis (the default on this Ubuntu release), one window, one tab, idle zsh,
measured 8 s after launch with the same `smaps_rollup` method:

| Terminal | RSS | PSS |
| :--- | :---: | :---: |
| Ptyxis (main process + `ptyxis-agent`) | ~125 MB | **~47 MB** |
| Litty | 67–69 MB | **~20–22 MB** |

This is one run per terminal on one machine, and Ptyxis was freshly launched
while the Litty process had been up longer, so treat the ratio (about 2×) as
indicative, not a benchmark.

## 🚀 Measured Engine Latencies & Microbenchmarks

| Component / Subsystem | Optimization / Architecture | Measured Latency | Throughput / Speedup |
| :--- | :--- | :---: | :---: |
| **Glyph Atlas Direct Cache** | Direct `[u16; 128]` ASCII cache bypasses hash maps | **39.78 µs** / 8,000 cells | **38.8x faster** (97.4% less time vs SipHash) |
| **FZF History Search** | Zero-heap-allocation slice tokens & smart-case matching | **23.34 ms** / 10,000 cmds | Sub-frame fuzzy interactive filtering |
| **PTY Keystroke Drain** | Non-blocking `poll(timeout=0)` stream drain | **1.0 µs** / keystroke | **99.9% reduction** (~1.0 ms saved per key) |
| **Kernel Syscall Overhead** | Throttled CWD & Tab title queries with `OnceLock` | **188.0 ns** / frame | **10.2x reduction** (90.2% syscall reduction) |
| **GPU Swapchain Passes** | Coalesced single-pass rendering per wake event | **1 pass** / WakeUp | **50% fewer** swapchain acquires & V-Sync waits |
| **Grid Cell Packing** | Packed `Cell` (`codepoint`, packed `fg`, packed `bg`) | **12 bytes** / cell | Zero runtime heap allocation on scroll/output |

## Benchmark commands

Run them on a release build (`cargo build --release`). Each in-app benchmark
opens a real window and prints one JSON line.

| Command | Measures | Notes |
| :--- | :--- | :--- |
| `litty +bench-input-latency [samples]` | Synthetic key press → GPU-completed frame that shows the echo, split into `input` / `echo` / `render` / `gpu` stages (default 1,000 samples after 50 warmup) | The pane runs an echo child, not a shell. Excludes the OS input stack and display scanout; `render` includes waiting for a free drawable |
| `litty +bench-first-frame [runs]` | Process spawn → GPU-completed first frame over `runs` launches (default 10), split into `exec` / `init` / `first_submit` / `gpu` | A normal startup with the login shell; the update check is skipped. Run 1 is reported apart because it may pay cold caches. The first frame may be drawn before the shell prints its prompt |
| `litty +bench-pty-throughput [MiB]` | MB/s of a generated stream (`ascii`, `sgr`, `code`, `unicode`, `osc8`, `region`) written by a pane child through the PTY while the app parses and draws it, next to a parser-only rate for the same bytes (default 64 MiB per kind) | Takes about 2 minutes; the `osc8` parser-only pass dominates |

---

Back to the [README](../../README.md).
