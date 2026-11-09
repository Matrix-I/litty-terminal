# Performance & Memory Footprint

When the 65,536-entry grapheme table fills, collection retries are bounded to
avoid scanning scrollback for every new cluster. Existing clusters remain
available; new clusters may temporarily lose combining marks until a later
collection reclaims unused entries.

Measured on 2026-11-09: Litty 0.11.0 release build (`cargo build --release`),
Apple M1 Pro, macOS 27.2, one 2590 × 1492 px Retina window.

| Metric | Result | How it was measured |
| :--- | :---: | :--- |
| Idle footprint, 1 window | **61 MB** | `litty +bench-ram`, step 1 (1 window, 1 tab, idle shell) |
| Idle RSS, 1 window | 92 MB | Same run |
| Idle footprint, all windows closed | **28 MB** | Shell exits so the only window closes; `footprint` 20 s later, 4 runs |
| Idle CPU | **0.0%** | 1 window, idle shell: no CPU time used over 30 s (`ps -o time`) |
| Startup to first frame | **136–148 ms** p50 warm, 130–175 ms first run (2 runs of 10) | `litty +bench-first-frame 10`: process spawn → GPU-completed first frame |
| Keystroke to frame | **3.1–3.6 ms** p50, 8.3–9.0 ms p95 (3 runs) | `litty +bench-input-latency`: synthetic key → GPU-completed frame showing the echo; excludes the OS input stack and display scanout |
| Binary size | **2.2 MB** | `target/release/litty` |

Two drawables at Retina size (15 MB each) make up about half of the
1-window footprint; they are freed when the window is occluded or closed.
Startup is dominated by `init` (~106 ms p50: font loading and the Metal
device, overlapped). Comparisons with other terminals are not listed: they
have not been measured on this setup.

> [!NOTE]
> **Measuring Memory Footprint on macOS**: macOS Activity Monitor displays **Physical Footprint** (`task_vm_info.phys_footprint`: actual private dirty memory + compressed memory), which accurately reflects the real memory cost attributed to the process. The legacy Unix `ps -o rss` metric includes shared read-only framework pages (Metal, AppKit, and system dylib caches) shared across all macOS GUI applications.
>
> Run from the raw binary outside a benchmark, Litty also carries an 8 MB Dock icon bitmap that a bundled `Litty.app` does not; `+bench-ram` skips it so it measures what bundled users get.

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
