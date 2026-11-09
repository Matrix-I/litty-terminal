# Litty Terminal — Pre-Release Performance & Stability Verification Report

Archived v0.11.0 release report. `scripts/pre-release-check.sh` exited 0 on
2026-11-09 (Darwin arm64, kernel 27.2.0, Apple M1 Pro), on `4123fb8` plus the
version bump only (no code change).

Measured figures from the suite:

- Footprint: 61 MB (1w/1t), 62 MB (1w/3t), 96 MB (2w/6t), 48 MB after close
  (delta -13 MB)
- PTY DSR roundtrip P99: 0.054 ms
- 20k-line stress: 442,067 lines/s, worst frame stall 1.42 ms
- FZF 10k search 1.637 ms; regex 5k scan 0.159 ms
- Binary size: 2,168,272 bytes (2.2 MB)

Measured separately (release binary, on AC power with Low Power Mode off):

- `+bench-ram` step 1: 61 MB footprint, 92 MB RSS
- All windows closed (shell exits, `footprint` 20 s later, 4 runs): 28 MB
- Idle CPU, 1 window: 0.0% (CPU time 0:00.39 → 0:00.39 over 30 s)
- `+bench-first-frame 10`, 2 runs: warm p50 136 / 148 ms, first run 175 /
  130 ms, `init` p50 ~106 ms
- `+bench-input-latency`, 3 runs: p50 3.1–3.6 ms, p95 8.3–9.0 ms, p99
  10.7–13.5 ms

Measurement note: a first latency run on battery with Low Power Mode on gave
p50 6.2–6.4 ms / p95 15.3–15.9 ms. An alternating A/B against a release
build of `7a48f81` (before `b2c3b8e`) under the same conditions gave the
same 6.7–6.8 ms p50, so that was the power mode, not a regression. The
published numbers are the AC-power runs above.

Other release checks:

- `cargo check`, `cargo check --target x86_64-unknown-linux-gnu`,
  `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test`: 581 passed (566 + 1 + 14), 0 failed, 11 ignored.

Not run:

- Linux runtime smoke test (`scripts/linux/test.sh`). `.deb` / `.rpm`
  packaging and install/remove run in the release workflow.
