# v0.14.1 Release Validation

Status: prepared 2026-11-21; not yet tagged.
Source: `42320d7` plus v0.14.1 version metadata and documentation only.
Release commit: `chore(release): prepare v0.14.1` (SHA filled in after the
last amend).

## Verification

### macOS arm64 (Apple Silicon, macOS 27.2, 2026-11-21 18:20 +0700)

`scripts/pre-release-check.sh` passed (all three sections). The run does not
record its commit; its test list includes the regression tests of all five
v0.14.1 fixes, so it ran at `42320d7` or a tree with the same source. Figures
below are from the maintainer's console log of that run; the generated
`scripts/release-test-report.md` stayed on the Mac.

| Check | Result |
| --- | --- |
| `cargo build --release` | PASS |
| `cargo test --release` | PASS: 722 unit + 1 + 14, 12 ignored; activation shell test PASS |
| RAM, 1 window / 1 tab | 61 MB footprint (budget 85 MB) |
| RAM, 1 window / 3 tabs | 63 MB (budget 95 MB) |
| RAM, 2 windows / 6 tabs | 96 MB (budget 145 MB) |
| RAM, after benchmark cleanup | 49 MB; no positive retained delta (budget 10 MB) |
| PTY DSR roundtrip | Mean 0.027 ms, P99 0.062 ms (budgets 0.05 / 0.4 ms) |
| PTY shell spawn | 2.11 ms avg (budget 6 ms) |
| Kitty APC / Sixel 256 KB | 1.19 ms / 3.35 ms (budgets 3 / 5 ms) |
| 20k-line stress | 610,940 lines/s; worst chunk stall 1.19 ms; 0 stalls > 16 ms |
| FZF 10k search / regex 5k scan | 1.64 ms / 0.165 ms (budgets 2.0 / 0.5 ms) |
| Split stress, bracketed paste sanitization | PASS |

Closed-window idle sample (2026-11-21 22:22 +0700, macOS 27.2.1, at
`42320d7`): packaged app from `bundle_macos.sh --no-install`, isolated
HOME/XDG config and `SHELL=/usr/bin/true`; the shell exited and the window
closed. Single sample.

| Metric | Result |
| --- | --- |
| Idle RAM, all windows closed | `footprint` 21 MB, about 55 s after launch |
| Idle CPU | CPU time 0:00.29 → 0:00.34 over 30 s (~0.17% of one core); `ps` displays 0.0% |
| Binary size | `litty-macos-aarch64` 2,354,896 bytes; app binary 2,341,328 bytes (< 5,000,000) |

Idle CPU is about 5× the v0.13.0 sample (0:00.23 → 0:00.24, ~0.033%). With
`ps` time resolution of 0.01 s and one sample each, this is not confirmed as
a regression; the badge keeps 0.0% as displayed by `ps`, the same convention
as v0.13.0.

### Linux x86_64 (Ubuntu host, 2026-11-21, at the release commit)

| Command | Result |
| --- | --- |
| `cargo check` | PASS |
| `cargo check --target x86_64-unknown-linux-gnu` | PASS |
| `cargo clippy --all-targets -- -D warnings` | PASS, zero warnings |
| `cargo test` outside sandbox | PASS: 876 unit + 1 + 14 shell tests, 17 ignored |
| `git diff --check` | PASS |

## Known limitations

- The Linux release binary size was not measured locally; the CI binary is
  to be checked against the 5,000,000-byte budget before publication.
- `scripts/linux/test.sh` (container smoke suite) was not run.
- The ignored tests were not enabled (live AI providers, GPU pixel probes,
  manual benchmarks).
