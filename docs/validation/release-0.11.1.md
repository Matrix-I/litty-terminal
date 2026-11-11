# Litty Terminal — Pre-Release Performance & Stability Verification Report

Archived v0.11.1 release report. `scripts/pre-release-check.sh` exited 0 on
2026-11-11 (Darwin arm64, kernel 27.2.0, Apple M1 Pro, AC power, Low Power
Mode off), on `86967b8` plus the version bump only (no code change).

Measured figures from the suite:

- Footprint: 61 MB (1w/1t), 78 MB (1w/3t), 126 MB (2w/6t), 64 MB after close
  (delta +3 MB)
- PTY DSR roundtrip P99: 0.043 ms
- 20k-line stress: 568,567 lines/s, worst frame stall 2.33 ms
- FZF 10k search 1.243 ms; regex 5k scan 0.121 ms
- `cargo test --release`: 591 + 1 + 14 passed, 0 failed, 12 ignored
- Binary size: 2,185,136 bytes (2.2 MB)

Measured separately (release binary):

- Idle CPU, 1 window: 0.0% (CPU time 0:00.32 → 0:00.32 over 30 s)
- All windows closed (shell exits, `footprint` 20 s later): 28 MB in 10 of
  10 final runs. See the intermittent result below.
- `+bench-first-frame 10`, 2 runs: warm p50 162 / 169 ms, first run 160 /
  150 ms, `init` p50 138 / 145 ms
- `+bench-input-latency`, 3 runs: p50 6.6–7.1 ms, p95 15.8–16.0 ms, p99
  18.5–19.4 ms

Measurement note (latency and startup): the machine was under load (load
average 4–11 from other work). Alternating `+bench-input-latency` runs of the
v0.11.0 build (`70c6aa9`) and this build in the same conditions gave p50
6.35–6.75 ms vs 6.12–6.90 ms and p95 15.6–16.0 ms for both, so the
difference from the v0.11.0 figures (3.1–3.6 ms p50) is the environment, not
a regression.

## Intermittent 0-window footprint (not reproduced)

In the first measurements after the suite, the 0-window footprint of this
build stayed at 47 MB in 7 of 18 runs (other runs 28–33 MB). 47 MB is the
level from before `79d44ef` freed a closed window's drawable. Eight
alternating runs of a v0.11.0 build in the same period gave 28–32 MB.

Follow-up, alternating, `footprint` 20 s after the window closed:

| Build | Runs | 47 MB results | Range otherwise |
| --- | ---: | ---: | --- |
| `d37a085` | 16 | 0 | 28–32 MB |
| `a0f7ebc` | 16 | 0 | 28–33 MB |
| `70c6aa9` (v0.11.0) | 12 | 0 | 29–31 MB |
| `15bca71` | 12 | 0 | 28–31 MB |
| `86967b8` | 12 | 0 | 29–31 MB |
| `86967b8` + version bump | 10 | 0 | 28 MB |

A `git bisect run` (8 runs per step) named `a0f7ebc`, on one 47 MB result in
5 runs; that commit only touches OSC 0/2 title parsing, which the measurement
shell (`sleep 3; exit 0`) never exercises, and the 16-run follow-up above did
not reproduce it. The cause is unknown; timing under system load is
suspected. Tracked in `ROADMAP.md` under verification debt. Still under the
50 MB idle budget in every run.

Other release checks:

- `cargo check`, `cargo check --target x86_64-unknown-linux-gnu`,
  `cargo clippy --all-targets -- -D warnings`, `git diff --check`: clean.
- `cargo test`: 606 passed (591 + 1 + 14), 0 failed, 12 ignored.

Not run:

- Linux runtime smoke test (`scripts/linux/test.sh`). `.deb` / `.rpm`
  packaging and install/remove run in the release workflow.
