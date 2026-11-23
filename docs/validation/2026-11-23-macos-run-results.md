# Full test run — macOS results (2026-11-23)

Plan: [full feature test plan](../plans/2026-11-23-full-test-plan.md)
(column M holds the per-case result). Issues:
[issue note](2026-11-23-full-test-issues.md).

## Status

**The macOS run is not complete.** Only the cases that run without a
person at the keyboard were executed: 52 of 325 macOS cases (16%). The
273 manual GUI cases (keyboard, mouse, IME, windows, rendering checks
by eye, …) have not run, so exit criterion 1 of the plan (every case
has a result other than "skipped") is not met for macOS. Linux X11 and
Linux Wayland have not started.

| Item | Value |
| --- | --- |
| Commit under test | `178357e` (update cases also `v0.14.0`) |
| Machine | Apple M1 Pro, built-in Retina 3024×1964, scale 2 |
| OS | macOS 27.2.1 (26B131), arm64 |
| Toolchain | rustc 1.99.1 |
| Shells | zsh, bash 3.2.57, nu (fish, elvish not installed) |
| Isolation | fresh `HOME` per run, `auto-update = off` |

## Totals (column M)

| | Count |
| --- | --- |
| Cases in the plan | 367 |
| Not applicable on macOS (`—`) | 42 |
| macOS cases | 325 |
| Run | 52 |
| — passed | 40 |
| — failed | 9 |
| — blocked | 3 |
| Not run (skipped: manual) | 273 |

## Per phase and section

| Phase / section | Pass | Fail | Blocked | Skipped | `—` |
| --- | --- | --- | --- | --- | --- |
| P0 preparation | 4 | 0 | 0 | 2 | 0 |
| P1 build gates | 11 | 0 | 0 | 0 | 1 |
| P2 automated suites | 5 | 0 | 1 | 0 | 7 |
| P3 performance | 9 | 4 | 1 | 5 | 1 |
| A launch, lifecycle, CLI | 2 | 1 | 0 | 11 | 0 |
| B windows, tabs, splits | 0 | 0 | 0 | 23 | 0 |
| C keyboard | 0 | 0 | 0 | 25 | 3 |
| D input methods | 0 | 0 | 0 | 6 | 2 |
| E mouse, selection, clipboard | 0 | 0 | 0 | 24 | 2 |
| F scrollback, output | 0 | 0 | 0 | 10 | 0 |
| G rendering, fonts, Unicode | 0 | 0 | 0 | 24 | 0 |
| H colors, themes, config | 0 | 0 | 0 | 18 | 0 |
| I escape sequences | 0 | 0 | 0 | 16 | 0 |
| J inline images | 0 | 0 | 0 | 9 | 0 |
| K shell integration | 0 | 0 | 0 | 10 | 0 |
| L history search | 0 | 0 | 0 | 10 | 0 |
| M find in terminal | 0 | 0 | 0 | 8 | 0 |
| N links, opening files | 0 | 0 | 0 | 8 | 0 |
| O notifications, bell, title | 0 | 0 | 0 | 12 | 1 |
| P inspector, shaders | 0 | 0 | 0 | 5 | 0 |
| Q process reuse | 0 | 0 | 0 | 6 | 1 |
| R update | 4 | 0 | 0 | 9 | 1 |
| S macOS-specific | 0 | 0 | 0 | 10 | 0 |
| T Linux-specific | 0 | 0 | 0 | 0 | 16 |
| U security | 2 | 0 | 0 | 8 | 0 |
| V robustness, stress | 0 | 0 | 0 | 8 | 2 |
| W AI assistant (offline) | 1 | 0 | 0 | 5 | 0 |
| P5 packaging, install | 2 | 0 | 0 | 1 | 4 |
| P6 documentation | 0 | 4 | 1 | 0 | 1 |

## Failed cases

| Case | Issue | Summary |
| --- | --- | --- |
| P3-03, P3-04 | ISSUE-001 | Worst frame stall above 3 ms in 2 of 6 bench runs |
| P3-09 | ISSUE-003 | macOS reports no refresh rate; scroll runs at 60 fps on a 120 Hz-capable display |
| P3-11 | ISSUE-010 | Idle CPU time grows slightly with cursor blink off |
| A-09 | ISSUE-004 | `--version` / `--help` open a terminal instead of printing |
| P6-01 | ISSUE-005 | Default shortcuts missing from `shortcuts.md` |
| P6-02 | ISSUE-006, ISSUE-007 | `$EDITOR` claim; undocumented env vars and config alias |
| P6-03 | ISSUE-008 | Actions missing from `keybinding-actions.md` |
| P6-04 | ISSUE-009 | Install docs name the v0.8.0 packages |

Passed with a logged issue: P3-01 and U-10 (ISSUE-002, test needs
`~/Library/Caches`), R-03 and R-04 (ISSUE-011, binary must be built with
the pinned minisign key).

## Blocked cases

| Case | Reason |
| --- | --- |
| P2-12 | Shutdown-race part passes; the activation probe script only builds the probe and needs a manual launch |
| P3-06 | `+bench-input-latency` reports the window covered or hidden when started from automation; needs a visible, focused window |
| P6-05 | Needs the results of the manual P4 cases |

## Key measurements

| Case | Result |
| --- | --- |
| P0-03 | Raw binary 2,338,384 B; app binary 2,341,328 B |
| P1-04 | 722 unit + 1 + 14 integration tests passed, 12 ignored |
| P3-02 | `+bench-ram` footprint 30–48 MB, 3 of 3 runs pass |
| P3-05 | DSR mean 0.021 ms, P99 0.062 ms |
| P3-07 | Startup p50 161–186 ms |
| P3-08 | In-app throughput: ascii 115.8 MB/s, unicode 75.5 MB/s |
| P3-12 | Idle RAM, all windows closed: 19 MB (limit 50 MB) |
| P3-13 | Idle CPU, all windows closed: 0.0%, +0.01 s CPU in 60 s |
| P5-05 | Bundle `com.litty.terminal` 0.14.1, arm64, ad-hoc signed, `codesign --verify` OK |
| P5-06 | v0.14.1 assets: 6 of 6 SHA-256 OK, minisign signature OK |

## Still to run on macOS

- All 273 skipped manual cases: P0-04/05 fixtures and helpers, P3-10 and
  P3-15..18, sections A–W except the cases listed above, P5-07.
- P3-06 with the window visible and focused.
- P2-12 activation probe with a manual launch.
- P6-05 after the manual cases.

## Next macOS session (decided 2026-11-23)

Continue the manual cases once computer use (desktop control in the
Claude desktop app: Settings → System → Computer use) is available, so
the app can be driven and checked without a person at the keyboard.

1. Enable computer use, start a new session, grant access to `Litty.app`.
2. Check first which tier Litty gets: terminal apps are limited to
   clicks (no typing or key presses), which would block most cases. If
   so, record it and fall back to a person running
   `scripts/manual-test.sh`.
3. Run the skipped cases section by section (A–W, then P3-06, P3-10,
   P3-15..18, P2-12 activation probe, P5-07, P6-05), writing results into
   column M and issues into the issue note as before.
4. Cases checked only from screenshots (fonts, colours, images, IME
   popups) get a note saying so.

Not yet tried, can run without computer use: drive the escape-sequence
and config cases (section I, parts of H, J, K, O) with a script used as
`SHELL` inside Litty that sends queries and records the replies.

Raw logs from this run are in a session scratch folder and are not kept
in the repository; the numbers above and in the issue note are the
record.
