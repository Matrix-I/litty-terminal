# Performance review — 2026-11-10

Measured at commit `442534c` (v0.11.0). No source was changed by this review.
Companion to [the source review](2026-11-10-source-review.md).

## Scope

Measured: the parser and grid, which run without a window.

Not measured: GPU rendering, keystroke-to-frame latency, startup time and RAM.
The machine used has no GPU, so the in-app benchmarks (`+bench-input-latency`,
`+bench-first-frame`, `+bench-pty-throughput`, `+bench-ram`) could not run.

## Method

- Temporary benchmark test (removed afterwards), `cargo test --release`,
  Linux x86_64, 4 cores.
- Grid 160×50, 32 MiB per stream kind, best of 3 runs, fed through
  `Grid::feed_bytes` as `bench_throughput::grid_only_mb_per_s` does.
- The first six stream kinds come from `bench_throughput::generate`; the rest
  were generated for this review.
- Numbers are from one machine and one session. Treat them as relative.

## Parser throughput

| Stream | MB/s |
| :--- | ---: |
| `ascii` | 410 |
| `sgr` (coloured log lines) | 346 |
| `code` | 600 |
| TUI redraw (cursor addressing, truecolor, erase line) | 500 |
| 256-colour and attribute toggles | 281 |
| ASCII with mode 2027 on | 397 |
| ASCII inside an open OSC 8 link | 332 |
| SGR 58 underline colour | 151 |
| `unicode` (Vietnamese, CJK, emoji) | 118 |
| CJK only | 121 |
| Combining marks | 97 |
| Emoji clusters | 81 |
| `region` (scroll region of 19 rows) | 110 |
| Alt screen, full-screen scroll | 27 |
| `osc8` (400 distinct links) | 2 |

Per-line cost derived from the above: about 0.15 µs per scrolled line on the
main screen, about 0.4 µs in the 19-row region, about 1.8 µs on the alt screen.

## Findings

### 1. OSC 8 hyperlinks are about 150 times slower than plain text

- Where: `src/terminal/grid/links.rs`, `intern_extra` and `osc8`.
- Cause: every link set does a linear search with string comparison over up
  to 255 table entries.
- Cost against the number of distinct links (4 MiB stream):

  | Distinct links | µs / line | MB/s |
  | ---: | ---: | ---: |
  | 1 | 1.0 | 44 |
  | 100 | 6.6 | 7.0 |
  | 250 | 14.6 | 3.2 |
  | 400 | 20.0 | 2.3 |

- Not caused by scrollback scans: 19 full scans in 90,000 lines.
- The 1 µs floor with a single link comes from heap allocation in `osc8`
  (`uri.join` and two `String`s per link). This conflicts with the "zero heap
  allocation on hot paths (PTY parsing)" invariant in `CLAUDE.md`.
- Affects output with a distinct link per line, such as `ls --hyperlink` in a
  large directory.
- Suggested fix: keep a hash per slot and compare hashes before strings;
  compare against the incoming bytes before allocating. Estimated result is
  about 1 µs per line or better. Not tried.

### 2. Resize always reflows the whole scrollback

- Where: `src/terminal/grid/resize.rs`; called per pane on every resize event
  under the grid lock (`src/app/window.rs:587`).
- Measured with 5,000 scrollback lines (the default) at 160 columns:

  | Change | Time |
  | :--- | ---: |
  | 160 → 159 columns | 12.8 ms |
  | Rows only | 6.2 ms |
  | 159 → 80 columns | 6.1 ms |

- A rows-only change needs no re-wrap but pays the full cost.
- Three transient copies of the scrollback are built (`logical_cells`, `out`,
  `cells`).
- Cost is linear in scrollback, and `scrollback-limit` allows up to 1,000,000
  lines. At that size a resize step would take seconds (extrapolated, not
  measured).
- Suggested fix: when `cols` is unchanged, skip the reflow and only move the
  screen / scrollback boundary.

### 3. Alt-screen full-screen scroll is O(rows × cols)

- Where: `src/terminal/grid/storage.rs`, `scroll_up_in`, the non-history
  branch (`copy_row` per row).
- About 1.8 µs per line against about 0.15 µs on the main screen, which uses
  an O(1) ring rotation.
- Suggested fix: a ring offset for the alt screen.
- Low priority: 540,000 lines per second is still far above the draw rate.
- Fixed (2026-11-13): the alternate screen keeps a ring offset (`alt_top`),
  so a full-screen scroll up or down rotates it and clears only the rows that
  come into view. Region scrolls still copy rows. `perf_probe alt_scroll`
  (macOS arm64, release, best of 7): 28.0 → 1.10 ns/byte.

### 4. Non-ASCII text is 3 to 5 times slower than ASCII

- Each character goes through `print` and a width lookup; ASCII is batched.
- Batching runs of same-width characters is possible. Small gain; low priority.
- Fixed for narrow text (2026-11-13): `feed_bytes` writes a run of
  one-column characters that cannot join a cluster (printable ASCII,
  Latin-1, Vietnamese precomposed letters, box drawing) once per row
  segment. Wide (CJK) and cluster characters still go one at a time.
  `perf_probe` (macOS arm64, release, best of 7):

  | Case | Before | After |
  | :--- | ---: | ---: |
  | `vietnamese` | 9.0 ns/B | 3.4 ns/B |
  | `unicode_mixed` | 8.1 ns/B | 5.0 ns/B |
  | `sgr_log` | 2.5 ns/B | 2.3 ns/B |
  | `cjk` | 7.8 ns/B | 7.8 ns/B |
  | `emoji_clusters` | 14.4 ns/B | 14.7 ns/B |

## Measured and fine

- ASCII, SGR, code and TUI redraw: 280–600 MB/s.
- Main-screen scroll: about 0.15 µs per line.
- `search_text` over 5,000 lines: 2.2 ms. Reusing the previous snapshot: 2.1 ms.
- Select all and extract text: 3 ms.
- Link detection over the whole screen (8,000 cells): 1.2 ms.
- `scroll_display` ×1000: about 1 µs in total.

## Read, not measured

- `render/renderer/frame.rs`, `pane_row.rs` and `atlas/run_cache.rs`: no
  per-frame heap allocation found by reading.

## Suggested order

1. Finding 1 (OSC 8 interning and allocation).
2. Finding 2 (rows-only resize fast path).
3. Finding 3, then 4, only if a real workload shows a need.
