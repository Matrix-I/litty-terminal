# Full test run — Linux results (2026-11-23)

Plan: [full feature test plan](../plans/2026-11-23-full-test-plan.md)
(columns X and W hold the per-case results). Issues:
[issue note](2026-11-23-full-test-issues.md). macOS:
[macOS results](2026-11-23-macos-run-results.md).

## Status

**The Linux run is complete except P3-10** (skipped: it opens the app;
its parts are covered by P3-03..05a). On 2026-11-23 only the cases that
run without a person at the keyboard were executed. On 2026-11-24 the
manual cases ran as well, driven by scripts: each GUI case ran in a
private headless GNOME Shell 50.1 session (`gnome-shell --headless
--virtual-monitor 1280x800`, own D-Bus session, own IBus with Unikey,
own portal and a separate dconf database), with keyboard and pointer
input injected through Mutter's RemoteDesktop API and checked through
screenshots, Wayland protocol logs, `xprop`, the shell's state, D-Bus
monitors and files the shell wrote. Nothing reached the real desktop.
Cases that need hardware, software or desktops this machine does not
have are `B` with the reason.

Limits of the harness that affect results: input is injected, not typed
on a physical keyboard (ISSUE-028 was not checked with one); all windows
share one 1280×800 virtual monitor; the private session's Telex IME
rewrites some typed words, so commands were chosen to avoid it.

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
| Isolation | fresh `HOME` per run, `auto-update = off`; GUI cases in a private headless GNOME Shell 50.1 session |

Not on this machine: Fedora (P5-03), KDE and sway sessions (E-L2, E-L3),
a second monitor (E-HW), a second user account (Q-06), the Khronos
validation layer (P2-03), `rpmbuild`, `vttest`, `tmux`.

## Totals

| | X11 (X) | Wayland (W) |
| --- | --- | --- |
| Cases in the plan | 367 | 367 |
| Not applicable (`—`) | 33 | 68 |
| Applicable cases | 334 | 299 |
| Run | 333 | 298 |
| — passed | 258 | 238 |
| — failed | 42 | 27 |
| — blocked | 33 | 33 |
| Not run (skipped) | 1 | 1 |

## Per phase and section

P/F/B/S = passed, failed, blocked, skipped; `—` = not applicable.
Section S (macOS only) is `—` on Linux.

| Phase / section | X P/F/B/S | W P/F/B/S |
| --- | --- | --- |
| P0 preparation | 6/0/0/0 | 6/0/0/0 |
| P1 build gates | 11/0/0/0 | — |
| P2 automated suites | 6/2/1/0 | 2/0/1/0 |
| P3 performance | 13/4/0/1 | 10/1/0/1 |
| A Launch, lifecycle and CLI | 11/2/0/0 | 11/2/0/0 |
| B Windows, tabs and splits | 20/2/1/0 | 21/0/2/0 |
| C Keyboard | 19/4/4/0 | 20/3/4/0 |
| D Input methods (IME) | 4/2/1/0 | 5/1/1/0 |
| E Mouse, selection and clipboard | 18/5/2/0 | 24/0/2/0 |
| F Scrollback and output | 10/0/0/0 | 10/0/0/0 |
| G Rendering, fonts and Unicode | 19/2/2/0 | 19/2/2/0 |
| H Colors, themes and config | 15/2/1/0 | 15/2/1/0 |
| I Escape-sequence conformance | 14/0/2/0 | 12/1/3/0 |
| J Inline images | 5/2/2/0 | 5/2/2/0 |
| K Shell integration | 6/2/2/0 | 7/1/2/0 |
| L History search (built-in) | 8/1/1/0 | 8/1/1/0 |
| M Find in terminal | 7/0/1/0 | 7/1/0/0 |
| N Links and opening files | 6/2/0/0 | 6/2/0/0 |
| O Notifications, bell, title and progress | 10/0/0/0 | 10/0/0/0 |
| P Terminal inspector and custom shaders | 4/1/0/0 | 4/1/0/0 |
| Q Process reuse (+reuse) | 5/0/1/0 | 7/0/0/0 |
| R Update (real GitHub releases) | 10/1/0/0 | — |
| T Linux-specific | 8/1/4/0 | 9/1/4/0 |
| U Security | 10/0/0/0 | 9/0/0/0 |
| V Robustness and stress | 4/1/5/0 | 5/0/5/0 |
| W AI assistant (offline only) | 6/0/0/0 | 5/0/0/0 |
| P5 packaging, install | 3/0/3/0 | 1/0/3/0 |
| P6 documentation | 0/6/0/0 | 0/6/0/0 |

## Failed cases

| Case | Column | Issue | Summary |
| --- | --- | --- | --- |
| P2-05 | X | ISSUE-013 | `linux_xdg_open_dispatch_smoke` opens real apps on a desktop |
| P2-08 | X | ISSUE-020 | `rounded_corners_fade_a_premultiplied_frame` fails in the container's full suite |
| P3-01 | X | ISSUE-014, ISSUE-015 | heap corruption abort in the `cargo test` step of `pre-release-check.sh`; Linux misses several `+bench-perf` / `+bench-stress` limits |
| P3-03 | X | ISSUE-015 | Linux misses several `+bench-perf` / `+bench-stress` limits |
| P3-04 | X | ISSUE-015 | Linux misses several `+bench-perf` / `+bench-stress` limits |
| P3-05a | X | ISSUE-015 | Linux misses several `+bench-perf` / `+bench-stress` limits |
| P3-09 | W | ISSUE-016 | native Wayland scroll misses every fifth frame on a 60 Hz panel |
| A-03 | X, W | ISSUE-017 | a missing `SHELL` makes Litty exit silently |
| A-09 | X, W | ISSUE-004 | `--version` and `--help` open a terminal window |
| B-08 | X | ISSUE-024 | dragging a tab out does not reliably tear it off on X11 |
| B-09 | X | ISSUE-025 | dropping a tab on another window’s tab strip makes a new window |
| C-01 | X | ISSUE-051, ISSUE-026, ISSUE-030 | Ctrl+, opens the config with xdg-open, not `$EDITOR`; GNOME default shortcuts take Ctrl+Alt+arrows and Alt+`; Copy leaves CLIPBOARD unchanged on X11 |
| C-10 | X, W | ISSUE-026 | GNOME default shortcuts take Ctrl+Alt+arrows and Alt+` |
| C-14 | X, W | ISSUE-027 | `keybind = ctrl++=…` parses but never matches |
| C-21 | X, W | ISSUE-028, ISSUE-029 | F1–F12 send no bytes on X11; modified F1 uses rxvt-style sequences |
| D-05 | X, W | ISSUE-049 | with an IME active, find box and history popup input goes to the shell |
| D-06 | X | ISSUE-050 | keys stop reaching Litty after the XIM server is killed (unconfirmed) |
| E-02 | X | ISSUE-030 | Copy leaves CLIPBOARD unchanged on X11 |
| E-06 | X | ISSUE-031 | Shift+Click does not extend a selection, and Shift snaps to the bottom, on X11 |
| E-07 | X | ISSUE-030 | Copy leaves CLIPBOARD unchanged on X11 |
| E-10 | X | ISSUE-030 | Copy leaves CLIPBOARD unchanged on X11 |
| E-20 | X | ISSUE-031 | Shift+Click does not extend a selection, and Shift snaps to the bottom, on X11 |
| G-12 | X, W | ISSUE-033 | Devanagari and some combining marks are not shaped |
| G-22 | X, W | ISSUE-032 | `window-width` / `window-height` give 2 rows fewer on Linux |
| H-13 | X, W | ISSUE-022 | default background is `#000000`, docs say `#121316` |
| H-16 | X, W | ISSUE-023 | a UTF-8 BOM breaks the first config line |
| I-14 | W | ISSUE-021 | a window title over about 4 KB kills Litty on Wayland |
| J-01 | X, W | ISSUE-035, ISSUE-036 | `manual-test.sh img shm` uses a name Litty refuses; Unicode-placeholder images show seams between rows on Wayland |
| J-07 | X, W | ISSUE-034 | kitty graphics delete with an unknown selector is silent |
| K-03 | X | ISSUE-031 | Shift+Click does not extend a selection, and Shift snaps to the bottom, on X11 |
| K-06 | X, W | ISSUE-037 | bash title shows only the cwd, not the running command |
| L-02 | X, W | ISSUE-038 | the history popup drops Space |
| M-08 | W | ISSUE-039 | the find box does not refresh while output streams |
| N-02 | X, W | ISSUE-041 | plain-text links recognise only http(s) |
| N-03 | X, W | ISSUE-040 | Ctrl+Click on a soft-wrapped path opens a prefix |
| P-02 | X, W | ISSUE-042 | the scanlines example shader makes the window translucent |
| R-07 | X | ISSUE-018 | `+update` on a package install downloads, then fails with a raw error |
| T-16 | X, W | ISSUE-047 | Linux never withdraws a notification when the window regains focus |
| V-04 | X | ISSUE-043 | X11 input starves while a pane floods output |
| P6-01 | X, W | ISSUE-005 | default shortcuts missing from `shortcuts.md` |
| P6-02 | X, W | ISSUE-006, ISSUE-007 | docs say `open_config` uses `$EDITOR`; code uses `open -t` / `xdg-open`; undocumented environment variables and config alias |
| P6-03 | X, W | ISSUE-008 | actions missing from `keybinding-actions.md` |
| P6-04 | X, W | ISSUE-009 | install docs name the v0.8.0 packages |
| P6-05 | X, W | ISSUE-037, ISSUE-040, ISSUE-036 | bash title shows only the cwd, not the running command; Ctrl+Click on a soft-wrapped path opens a prefix; Unicode-placeholder images show seams between rows on Wayland |
| P6-06 | X, W | ISSUE-019 | `scripts/manual-test.sh` assumes macOS |

Passed with a logged issue: P1-04 (ISSUE-012, fails only under a fresh
`HOME` with zsh), R-03 and R-04 (ISSUE-011, need the pinned key; the
`bundle_linux.sh` binary has it), C-01 W (ISSUE-026, ISSUE-051), W-04
(ISSUE-044, ISSUE-045), A-10 (ISSUE-046), T-08 (ISSUE-048), O-04
(partial: tab not moved first, ISSUE-024) and R-11 (ISSUE-052).

## Blocked cases

| Case | Column | Reason |
| --- | --- | --- |
| P2-03 | X, W | X: 2/2 pass; Khronos validation layer not installed, VUIDs not checked; W: offscreen test, same run as X |
| B-09 | W | harness has no window geometry on Wayland |
| B-21 | X, W | single virtual monitor |
| C-04 | X, W | prompt popup P; vim not installed |
| C-17 | X, W | nvim, helix, fish 4, kakoune not installed |
| C-24 | X, W | keyboard layouts not switchable in harness |
| C-25 | X, W | as C-24 |
| D-03 | X, W | no Japanese/Chinese IME engine installed |
| E-19 | X, W | scrollbar appears while scrolled back; thumb drag not tried |
| E-26 | X, W | X: no file manager in the private session; W: repo drop smoke cannot place the pointer in the headless session |
| G-09 | X, W | JetBrains Mono / Fira Code not installed |
| G-24 | X, W | no 120 Hz display; macOS-only ProMotion |
| H-09 | X, W | switching light/dark needs the shared org.gnome.desktop.interface color-scheme dconf key; not changed |
| I-09 | X, W | vttest not installed |
| I-15 | W | probe stopped at ISSUE-021 |
| I-16 | X, W | tmux, nvim not installed |
| J-02 | X, W | kitten, timg, chafa, yazi not installed; animation covered by J-01 |
| J-08 | X, W | tmux not installed |
| K-02 | X, W | elvish not installed |
| K-09 | X, W | oh-my-zsh, starship, powerlevel10k, bash-preexec, atuin not installed |
| L-09 | X, W | nushell, elvish not installed |
| M-08 | X | not isolated: X11 input and redraw lag during the flood, ISSUE-043; final count 1/1 |
| Q-06 | X | `0700`, owned by the user; no second user for the other-UID part |
| T-02 | W | sway not installed |
| T-05 | X, W | one 60 Hz virtual monitor only |
| T-10 | W | headless virtual monitor; no second output or fractional scale to switch |
| T-12 | X, W | one GPU; no lavapipe rendering check |
| T-14 | X | Mutter always composites; no compositor-less X server available |
| T-15 | X | twm not installed |
| V-05 | X, W | 8-hour idle run not done in this session |
| V-07 | X, W | headless GNOME has no GPU switch or reset to trigger |
| V-09 | X, W | no GPU stress tool installed; not installing one |
| V-10 | X, W | needs a high-speed camera |
| V-08 | X, W | needs a small full filesystem; mounting one needs root |
| P5-02 | X, W | `test-package-deb.sh` passes; `apt install` needs root, not run |
| P5-03 | X, W | no Fedora machine; `rpmbuild` not installed |
| P5-07 | X, W | no clean Debian 12 machine |

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

- P3-10 (opens the app; parts covered by P3-03..05a).
- The blocked cases above where the missing piece can be added:
  Khronos validation layer (P2-03), `apt install` and Fedora (P5-02,
  P5-03), KDE and sway (T-02, E-L2, E-L3), a second monitor and 120 Hz
  display, the missing apps (vim, nvim, tmux, vttest, kitten, fonts,
  shells), a CJK IME, a second user account (Q-06).
- Re-check on a physical keyboard: ISSUE-028 (F keys on X11) and
  ISSUE-050 (keys after the XIM server dies, run once).

Raw logs are in a session scratch folder and are not kept in the
repository; the numbers above and in the issue note are the record.
