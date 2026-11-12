# macOS verification close-out — 2026-11-12

Re-runs the open macOS measurements from `ROADMAP.md` (verification debt) and
the v0.7.4, v0.8.0 and v0.9.0 follow-ups.

## Setup

- Apple M1 Pro, 16 GB, macOS 27.2.1.
- Commit `a307298` (branch `fix/macos-occlusion`, v0.11.1 plus the occlusion
  fix), `cargo build --release`, raw binary (not the `.app` bundle).
- The machine was busy with other work: 1-minute load average 10–26 during the
  runs. No attempt was made to quiet it.

## Memory, 20 alternating rounds

Each round: (a) start Litty with an empty `HOME` and `SHELL` set to a script
that runs `sleep 3; exit 0`, so the only window closes on its own; read
`footprint -p` 23 s after launch (about 20 s after the window closed), then
kill the process; (b) run `+bench-ram` and keep `/tmp/litty_ram_bench.json`.

### All windows closed

| Footprint | Runs | 1-minute load at the sample |
| ---: | ---: | --- |
| 31 MB | 16 | 10–26 |
| 33 MB | 2 | 14.6, 20.4 |
| 40 MB | 2 | 22.9, 26.0 (the two highest of the session) |
| 47 MB | 0 | — |

The 47 MB level seen in 7 of 18 runs during v0.11.1 validation did not occur.
The two 40 MB results came at the highest load, which fits the "timing under
load" suspicion but does not prove it. Every run was under the 50 MB idle
budget.

### `+bench-ram`

| Step | Min | Median | Max |
| --- | ---: | ---: | ---: |
| 1 window, 1 tab | 46 MB | 62 MB | 62 MB |
| 1 window, 3 tabs | 43 MB | 48 MB | 63 MB |
| 2 windows, 6 tabs | 51 MB | 66 MB | 96 MB |
| After closing | 34 MB | 35 MB | 64 MB |
| Leak delta | 0 MB | 0 MB | 7 MB |

- `Memory Leak <= 10 MB`: passed in 20 of 20 runs (deltas: 16 × 0, 2, 3, 3,
  7 MB). All four budgets passed in 20 of 20.
- 2 windows / 6 tabs: 126 MB (v0.9.0, v0.11.1) did not occur; the maximum was
  96 MB, the same as v0.11.0. Values fall into steps about 15 MB apart (46/61,
  48/63, 51/66/81/96 MB), which suggests the number of drawables each window
  holds at the moment of the sample rather than retained heap. This was not
  confirmed with a per-category `footprint` breakdown.
- The leak delta is one sample after cleanup minus the 1-window baseline; both
  move by one of those steps, which explains a non-zero delta without a leak.

## DSR roundtrip, `+bench-dsr 1000`, 3 runs

Endpoints and warmup are documented in `src/bench_dsr.rs`: timing starts before
the child writes `ESC[5n` and ends when it has read the full `ESC[0n`, through
the PTY driver, reader thread, parser, `Grid::response` and writer queue; 100
roundtrips are discarded first.

| Run | Mean | Median | P95 | P99 | Max |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 0.028 ms | 0.023 ms | 0.051 ms | 0.088 ms | 0.330 ms |
| 2 | 0.024 ms | 0.021 ms | 0.045 ms | 0.083 ms | 0.136 ms |
| 3 | 0.025 ms | 0.021 ms | 0.048 ms | 0.076 ms | 0.140 ms |

All within the mean ≤ 1 ms / P99 ≤ 3 ms target. Linux was not run.

## Not covered

- No per-category `footprint` breakdown at the 2-window step.
- Per-session memory after `yo`, live non-Anthropic providers and Nushell older
  than 0.116.1: not measured here (see `ROADMAP.md`).
