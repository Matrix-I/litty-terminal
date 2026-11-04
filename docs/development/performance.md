# Performance & Memory Footprint

When the 65,536-entry grapheme table fills, collection retries are bounded to
avoid scanning scrollback for every new cluster. Existing clusters remain
available; new clusters may temporarily lose combining marks until a later
collection reclaims unused entries.

Tested on macOS Sequoia / Sonoma (Apple Silicon M-series, empty idle shell):

| Terminal Emulator | Idle RAM (Activity Monitor) | Idle RAM (RSS) | Idle CPU | Startup Time | Binary Size | GPU Backend |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **Litty** | **~36 MB** | **~85 – 108 MB** | **0.0%** | **< 30 ms** | **6.3 MB** | Metal (Direct) |
| Ghostty | ~70 – 85 MB | ~110 – 125 MB | 0.0% | ~50 ms | ~45 MB | Metal |
| Alacritty | ~75 – 95 MB | ~115 – 135 MB | 0.0% | ~60 ms | ~18 MB | OpenGL / Metal |
| WezTerm | ~130 – 160 MB | ~170 – 210 MB | 0.0% – 0.5% | ~120 ms | ~95 MB | OpenGL / Vulkan |

> [!NOTE]
> **Measuring Memory Footprint on macOS**: macOS Activity Monitor displays **Physical Footprint** (`task_vm_info.phys_footprint`: actual private dirty memory + compressed memory), which accurately reflects the real memory cost attributed to the process (**~36 MB** for Litty). The legacy Unix `ps -o rss` metric includes shared read-only framework pages (Metal, AppKit, and system dylib caches) shared across all macOS GUI applications.

## 🚀 Measured Engine Latencies & Microbenchmarks

| Component / Subsystem | Optimization / Architecture | Measured Latency | Throughput / Speedup |
| :--- | :--- | :---: | :---: |
| **Glyph Atlas Direct Cache** | Direct `[u16; 128]` ASCII cache bypasses hash maps | **39.78 µs** / 8,000 cells | **38.8x faster** (97.4% less time vs SipHash) |
| **FZF History Search** | Zero-heap-allocation slice tokens & smart-case matching | **23.34 ms** / 10,000 cmds | Sub-frame fuzzy interactive filtering |
| **PTY Keystroke Drain** | Non-blocking `poll(timeout=0)` stream drain | **1.0 µs** / keystroke | **99.9% reduction** (~1.0 ms saved per key) |
| **Kernel Syscall Overhead** | Throttled CWD & Tab title queries with `OnceLock` | **188.0 ns** / frame | **10.2x reduction** (90.2% syscall reduction) |
| **GPU Swapchain Passes** | Coalesced single-pass rendering per wake event | **1 pass** / WakeUp | **50% fewer** swapchain acquires & V-Sync waits |
| **Grid Cell Packing** | Packed `Cell` (`codepoint`, packed `fg`, packed `bg`) | **12 bytes** / cell | Zero runtime heap allocation on scroll/output |

---

Back to the [README](../../README.md).
