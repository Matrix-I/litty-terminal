# Full test run — Linux results (2026-11-23)

Plan: [full feature test plan](../plans/2026-11-23-full-test-plan.md)
(columns X and W hold the per-case results). Issues:
[issue note](2026-11-23-full-test-issues.md). macOS:
[macOS results](2026-11-23-macos-run-results.md).

## Status

**The Linux run is not complete.** As on macOS, only the cases that run
without a person at the keyboard were executed: 64 of 334 X11 cases (19%)
and 29 of 299 Wayland cases (10%). The manual GUI cases (keyboard, mouse,
IME by hand, windows, rendering checks by eye, …) were left out on
purpose, so exit criterion 1 of the plan is not met for Linux.

| Item | Value |
| --- | --- |
| Commit under test | `fc3d1d4` (source identical to `178357e`, the macOS run; update cases also `v0.14.0`) |
| Machine | Intel Core i5-8365U (8 threads, `powersave` governor, on AC), Intel UHD 620, Mesa Vulkan |
| OS | Ubuntu 26.04.1, kernel 7.0.0-44, GNOME Shell 50.1 Wayland session |
| Display | eDP 3072×1728 at 59.94 Hz, GNOME scale 1.25 |
| Backends | X: default X11 (XWayland); W: `LITTY_BACKEND=wayland` |
| Toolchain | rustc 1.100.0 (host), 1.99.1 (container) |
| Shells | zsh 5.9, bash 5.3.9 (fish, nu, elvish not installed) |
| Input method | IBus with Unikey (`XMODIFIERS=@im=ibus`) |
| Isolation | fresh `HOME` per run, `auto-update = off` |

Not on this machine: Fedora (P5-03), KDE and sway sessions (E-L2, E-L3),
a second monitor (E-HW), the Khronos validation layer (P2-03), `rpmbuild`.

## Totals

| | X11 (X) | Wayland (W) |
| --- | --- | --- |
| Cases in the plan | 367 | 367 |
| Not applicable (`—`) | 33 | 68 |
| Applicable cases | 334 | 299 |
| Run | 64 | 29 |
| — passed | 46 | 19 |
| — failed | 14 | 8 |
| — blocked | 4 | 2 |
| Not run (skipped) | 270 | 270 |

## Per phase and section

| Phase / section | X P/F/B/S | W P/F/B/S |
| --- | --- | --- |
| P0 preparation | 4/0/0/2 | 4/0/0/2 |
| P1 build gates | 11/0/0/0 | — |
| P2 automated suites | 6/2/1/0 | 2/0/1/0 |
| P3 performance | 9/4/0/5 | 6/1/0/5 |
| A launch, lifecycle, CLI | 4/2/0/7 | 4/2/0/7 |
| R update | 4/1/0/6 | — |
| T Linux-specific | 2/0/0/11 | 2/0/0/12 |
| U security | 2/0/0/8 | 1/0/0/8 |
| W AI assistant (offline) | 1/0/0/5 | 0/0/0/5 |
| Other P4 sections (B–Q, S, V) | 0/0/0/225 | 0/0/0/227 |
| P5 packaging, install | 3/0/2/1 | 0/0/0/4 |
| P6 documentation | 0/5/1/0 | 0/5/1/0 |

## Failed cases

| Case | Column | Issue | Summary |
| --- | --- | --- | --- |
| P2-05 | X | ISSUE-013 | `xdg-open` smoke opens real apps on a GNOME desktop; passes in the container |
| P2-08 | X | ISSUE-020 | Container suite: `rounded_corners_fade_a_premultiplied_frame` fails 3 of 3 full runs, passes alone |
| P3-01 | X | ISSUE-014, ISSUE-015 | Heap-corruption abort in its `cargo test` step; bench limits missed |
| P3-03, P3-04, P3-05a | X | ISSUE-015 | Throughput 155k–279k lines/s (limit 350k); stall, file read, APC, Sixel, FZF limits missed in some runs |
| P3-09 | W | ISSUE-016 | Native Wayland scroll at 48–50 fps on a 60 Hz panel (X11: 60 fps) |
| A-03 | X, W | ISSUE-017 | `SHELL=/nonexistent/shell` exits silently with code 0 |
| A-09 | X, W | ISSUE-004 | `--version` / `--help` start the terminal (as on macOS) |
| R-07 | X | ISSUE-018 | `+update` on the root-owned `.deb` install downloads, then fails with `Permission denied` |
| P6-01..04 | X, W | ISSUE-005..009 | Documentation issues found on macOS; same files |
| P6-06 | X, W | ISSUE-019 | `scripts/manual-test.sh` assumes macOS (REV-10) |

Passed with a logged issue: P1-04 (ISSUE-012, fails only under a fresh
`HOME` with zsh), R-03 and R-04 (ISSUE-011, need the pinned key; the
`bundle_linux.sh` binary has it).

## Blocked cases

| Case | Column | Reason |
| --- | --- | --- |
| P2-03 | X, W | Both tests pass; Khronos validation layer not installed, so VUIDs were not checked |
| P5-02 | X | `test-package-deb.sh` passes; `apt install` needs root and was not run (0.14.0 `.deb` is installed from before) |
| P5-03 | X | No Fedora machine; `rpmbuild` not installed |
| P6-05 | X, W | Needs the results of the manual P4 cases |

## Key measurements

| Case | X11 | Wayland |
| --- | --- | --- |
| P0-03 / P1-10 binary | 4,795,768 B (release asset 4,819,704 B; limit 5,000,000) | same |
| P1-04 tests | 876 unit + 1 + 14 passed, 17 ignored | — |
| P1-11 `ldd` | `libgcc_s`, `libm`, `libc`, `ld-linux` only | — |
| P2-04 glyph upload | 1024 glyphs 8.26 ms | — |
| P3-02 `+bench-ram` (RSS) | 74.9–76.3 MB, cleanup +0.69–0.72 MB | 74.2–75.5 MB, cleanup +0.66–0.69 MB |
| P3-05 DSR | mean 0.020–0.049 ms, P99 0.044–0.309 ms | — |
| P3-06 input latency | p50 9.4 ms, p95 13.7 ms | p50 10.0 ms, p95 29.0 ms |
| P3-07 first frame p50 | 113–224 ms | 67–186 ms |
| P3-08 PTY throughput (ascii, in app) | 46.1 MB/s | 20.3–26.5 MB/s |
| P3-09 scroll | 60.0 fps | 48.4–50.0 fps |
| P3-11 idle CPU, 60 s | blink on +5 ticks, off 0 | blink on +5 ticks, off 0 |
| P3-14 idle memory, 1 window | RSS 76.9 MB, PSS 28.8 MB | RSS 76.1 MB, PSS 29.0 MB |
| P5-06 release assets | 6/6 SHA-256 OK, minisign signature OK | — |

P3-14 is above the earlier reference (RSS ~67 MB, PSS ~21 MB in
`docs/development/performance.md`); the plan asks only to record it.
P3-06 now completes on Linux (the 2026-11-13 run could not finish it).

## Still to run on Linux

- All manual cases (`S manual` in X and W).
- P2-03 with `VK_LAYER_KHRONOS_validation` installed.
- P5-02 `apt install` / `apt remove` and menu launch; P5-03 on Fedora.
- KDE, sway and mixed-DPI environments (E-L2, E-L3, E-HW).
- P6-05 after the manual cases.

Raw logs are in a session scratch folder and are not kept in the
repository; the numbers above and in the issue note are the record.
