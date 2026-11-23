# Full test run — issue note (2026-11-23)

Plan: [full feature test plan](../plans/2026-11-23-full-test-plan.md).
Status: automated macOS cases ran on 2026-11-23 (M column); manual
cases, Linux X11 and Linux Wayland have not run. Nothing below is fixed.

Issues are only logged here during the run. Fixing starts after the whole
plan has run (plan section 10).

## Run summary

Fill in at the start (environment) and at the end (counts).

| Item | macOS | Linux X11 | Linux Wayland |
| --- | --- | --- | --- |
| Commit | `178357e` (rustc 1.99.1) | | |
| OS / desktop / compositor | macOS 27.2.1 (26B131), arm64 | | |
| GPU / driver | Apple M1 Pro (Metal) | | |
| Display scales | Built-in Retina 3024×1964, scale 2 | | |
| Date run | 2026-11-23 (automated cases only) | | |

| Phase | Run | Pass | Fail | Blocked | Skipped |
| --- | --- | --- | --- | --- | --- |
| P0 preparation | M: 4 | 4 | 0 | 0 | 2 |
| P1 build gates | M: 11 | 11 | 0 | 0 | 0 |
| P2 automated suites | M: 6 | 5 | 0 | 1 | 0 |
| P3 performance | M: 14 | 9 | 4 | 1 | 5 |
| P4 functional A–W | M: 10 | 7 | 1 | 2 | 265 |
| P5 packaging/install | M: 2 | 1 | 0 | 1 | 1 |
| P6 documentation | M: 5 | 0 | 4 | 1 | 0 |

Counts are for the M column only; "Run" = pass + fail + blocked. Cases
marked `—` are not counted. "Skipped" is almost all manual GUI cases,
which this run left out on purpose (automated cases only). Shell
coverage (P2-13): zsh, bash 3.2.57 and nu; fish and elvish are not
installed on this Mac. Each run used an isolated `HOME` with
`auto-update = off` instead of backing up the real `~/.litty` (P0-02).

## How to log an issue

- One entry per distinct problem. If the same problem shows in several
  cases, list every case ID in one entry.
- IDs are `ISSUE-001`, `ISSUE-002`, … in the order found. Put the ID in the
  plan's result cell (`F ISSUE-004`).
- Severity: **S1** crash, hang, data loss or security; **S2** feature broken,
  no workaround; **S3** partly broken or has a workaround; **S4** cosmetic or
  documentation. Type: `bug`, `regression`, `docs`, `spec question`,
  `test tooling`.
- Record facts: exact steps, exact output, how often it reproduces (n of m).
  Leave the cause and the fix empty unless they are obvious from the evidence.

Template:

```markdown
### ISSUE-NNN — <short title>

- Cases: <plan IDs> · Platform: <M / X / W, OS and desktop version> · Build: <commit>
- Severity: S1–S4 · Type: bug / regression / docs / spec question / test tooling
- Reproduces: <n of m>
- Steps:
  1. …
- Expected: …
- Actual: …
- Evidence: <command output, log path, screenshot path>
- Notes: <scope, related issues; suspected area only if evidenced>
```

## Review findings before the run

Status after the automated macOS run:

| REV | Status |
| --- | --- |
| REV-01 | Open: needs the manual drop test (E-26) |
| REV-02 | Open: Linux only |
| REV-03 | Confirmed → ISSUE-006 |
| REV-04 | Confirmed → ISSUE-008 |
| REV-05 | Confirmed → ISSUE-005 |
| REV-06 | Confirmed and extended → ISSUE-007 |
| REV-07 | Confirmed → ISSUE-009 |
| REV-08 | Confirmed → ISSUE-004 |
| REV-09 | Open: needs the manual paste test (U-02) |
| REV-10 | Open: Linux only |

Found while writing the plan, by reading the source and documentation at
`9fd8ab4`. These are **not test results**: each one must be confirmed or
rejected by its plan case on a real machine, then either becomes an
`ISSUE-NNN` entry or is closed here as "not reproduced".

### REV-01 — file drag-and-drop exists only on the Wayland backend

- Cases: E-26 (also manual-test group 7) · Platform: M, X · Severity if
  confirmed: S2 · Type: regression (to confirm)
- Evidence in source: the only producer of `WindowEvent::DroppedFile` is
  `src/platform/linux/native/wayland_data.rs:71`. No dragging-destination
  code was found under `src/platform/macos/` or in the X11 backend
  (`src/platform/linux/native/x11.rs`, no XDND). The consumer
  (`src/app/window_event.rs:115`) is shared.
- History: "Drag and drop files to paste their shell-quoted paths" is marked
  shipped in `docs/plans/0.4.0.md:249`, and `scripts/manual-test.sh` group 7
  still asks to drop a Finder file. The native AppKit/X11 windowing of
  v0.10.0 may have dropped it.
- Confirm: drop a file from Finder (M) and from Nautilus/Dolphin with the
  default X11 backend; expected per the docs: its shell-quoted path is pasted.

### REV-02 — Linux without zenity: multi-line paste is always cancelled

- Cases: T-07, U-02 · Platform: X, W · Severity if confirmed: S3 · Type: bug
  (to confirm)
- Evidence in source: `platform::confirm` → `alert_choice`
  (`src/platform/linux/dialog.rs:11-19`) returns the last button (Cancel)
  when `zenity` cannot run, so `paste_text` (`src/app/actions.rs:244`) and
  the second paste path (`src/app/actions.rs:183`) drop a multi-line paste
  into a program without bracketed paste. `confirm_close`
  (`src/app/tabs.rs:121-126`) does the opposite and closes when zenity is
  missing.
- `zenity` is in neither the `.deb` `Depends`/`Recommends`
  (`scripts/linux/package-deb.sh:52-53`) nor the RPM `Requires`/`Recommends`
  (`resources/linux/litty.spec`).
- Confirm: remove zenity, copy two lines, paste into `cat`; expected per
  design: a confirmation, or the paste. Record what happens.

### REV-03 — `open_config` does not use `$EDITOR`

- Cases: H-01, P6-02 · Platform: M, X, W · Severity if confirmed: S4 ·
  Type: docs or spec question
- Evidence: `src/config.rs:935-937` opens the file with `/usr/bin/open -t`
  (macOS) or `xdg-open` (Linux). `docs/guide/configuration.md` ("opens it in
  your default `$EDITOR`") and `docs/guide/shortcuts.md` ("Opens
  `~/.litty/config` in `$EDITOR`") say `$EDITOR`.
- Decide whether the code or the docs are right.

### REV-04 — actions missing from `keybinding-actions.md`

- Cases: C-12, P6-03 · Severity: S4 · Type: docs
- `src/keymap.rs:384-388` accepts `increase_font_size`,
  `decrease_font_size`, `reset_font_size`, `scroll_page_up` and
  `scroll_page_down`; the actions guide does not list them.

### REV-05 — default shortcuts missing from `shortcuts.md`

- Cases: C-01, P6-01 · Severity: S4 · Type: docs
- In `src/keymap.rs` but not in the shortcut table: Linux `Ctrl+PageUp` /
  `Ctrl+PageDown` (previous/next tab, `resolve_named`), Linux `Ctrl+Alt+D`
  (split down), macOS `Cmd+Ctrl+=` (equalize splits, `(alt ^ ctrl)`).

### REV-06 — undocumented environment variables

- Cases: G-02, P6-02 · Severity: S4 · Type: docs
- `LITTY_FONT` (`src/render/atlas/fonts.rs:39`) and `LITTY_FONT_SIZE`
  (`src/config.rs:175`) change the font; the Environment table in
  `docs/guide/configuration.md` lists only `LITTY_BACKEND` and
  `XDG_ACTIVATION_TOKEN`.

### REV-07 — install docs point at the v0.8.0 packages

- Cases: P6-04 · Severity: S4 · Type: docs
- `README.md` (Quick Start) and `docs/guide/installation.md` name
  `litty_0.8.0_amd64.deb` and `litty-0.8.0-1.x86_64.rpm` from the v0.8.0
  release. The release workflow builds `litty_${VERSION}_amd64.deb` and
  `litty-${VERSION}-1.x86_64.rpm` for every release
  (`.github/workflows/release.yml:215`).

### REV-08 — `--version` / `--help` start the terminal

- Cases: A-09 · Severity: S4 · Type: spec question
- `src/cli.rs:91-98`: any argument that is not a known command falls
  through to `Command::Run`, so `litty --version` and `litty --help` open a
  window instead of printing. There is no way to print the version from the
  command line.

### REV-09 — multi-line paste dialog overstates the line count

- Cases: U-02 · Severity: S4 · Type: bug (cosmetic)
- `src/app/actions.rs:239` (and `:178`) uses
  `text.lines().count().max(2)`: a single line with a trailing newline
  (`"abc\n"`) is reported as "Pasting 2 lines".

### REV-10 — `scripts/manual-test.sh` assumes macOS in places

- Cases: P6-06 and every case that reuses a manual-test group on Linux ·
  Severity: S4 · Type: test tooling
- Group 23 reads the clipboard with `pbpaste` (`scripts/manual-test.sh:340`);
  prompts say `Cmd`; group 11 uses `/System/Library/Sounds/Glass.aiff`
  (`scripts/manual-test.sh:256`).
  Only group 26 checks `$OS`. On Linux these steps need manual substitution
  (`xclip -o -selection clipboard` / `wl-paste`, `Ctrl+Shift`, a local sound
  file).

## Issues found during the run

### ISSUE-001 — benchmark worst frame stall sometimes exceeds 3 ms

- Cases: P3-03, P3-04 · Platform: M, macOS 27.2.1 · Build: `178357e`
- Severity: S3 · Type: bug or test tooling (not yet known which)
- Reproduces: 2 of 6 runs (P3-03 run 3, P3-04 run 2)
- Steps:
  1. `target/release/litty +bench-perf`, three times.
  2. `target/release/litty +bench-stress 100000`, three times.
- Expected: worst stall ≤ 3 ms, exit 0.
- Actual: P3-03 run 3: `Worst Frame Stall <= 3.0 ms : FAIL (8.63 ms > 3.0 ms)`,
  throughput 350,230 lines/s (floor 350,000), exit 1. P3-04 run 2:
  `FAIL (18.35 ms > 3.0 ms)`, 1 chunk over 16 ms, 562,525 lines/s, exit 1.
  The other 4 runs pass (worst stall 0.64–0.69 ms, about 791k lines/s in
  P3-04).
- Evidence: local logs `run/p3/P3-03-r{1,2,3}.log`, `run/p3/P3-04-r{1,2,3}.log`.
- Notes: the machine was not otherwise idle (normal desktop session), so a
  host scheduling hiccup is possible; the bench has no retry or warm-up
  guard. Rerun on an idle machine before triage.

### ISSUE-002 — `poc_cwe22_macos_library_is_blocked` needs `~/Library/Caches`

- Cases: P3-01, U-10 (and P1-04 under a fresh `HOME`) · Platform: M ·
  Build: `178357e`
- Severity: S4 · Type: test tooling
- Reproduces: 1 of 1 with an isolated `HOME`; 0 of 2 with the real `HOME`
- Steps:
  1. `HOME=$(mktemp -d) cargo test security_poc`
- Expected: the test sets up what it needs and passes.
- Actual: panic at `src/security_poc.rs:124:43`: `called Result::unwrap()
  on an Err value: Os { code: 2, kind: NotFound, ... }`. With the real
  `HOME`, `cargo test security_poc` passes 16/16 and
  `scripts/pre-release-check.sh` exits 0.
- Evidence: `run/p3/P3-01.log`, `run/p3/P3-01-rerun.log`.
- Notes: the test writes into `$HOME/Library/Caches` and assumes it
  exists, which is not true on a fresh CI user or a clean `HOME`.

### ISSUE-003 — macOS reports no monitor refresh rate; scroll runs at 60 fps

- Cases: P3-09 · Platform: M, built-in 120 Hz-capable (ProMotion) display ·
  Build: `178357e`
- Severity: S3 · Type: spec question
- Reproduces: 1 of 1
- Steps:
  1. `target/release/litty +bench-scroll-fps`
- Expected (plan): record the frame rate on a 120 Hz display.
- Actual: header `"monitor_millihz":0`; every scenario submits about
  60 fps (`interval_p50_ms` ≈ 16.6); the 120 Hz-input scenarios report
  `late_intervals` 240 and 236 of about 241 frames.
- Evidence: `run/p3/P3-09.log`. `src/platform/macos/native/window.rs:618`
  sets `refresh_rate_millihertz: None`, so `src/app/mod.rs:364` never
  learns the rate on macOS.
- Notes: decide whether macOS should drive ProMotion at 120 Hz. Not
  checked: the display's refresh-rate setting in System Settings.

### ISSUE-004 — `--version` and `--help` open a terminal window

- Cases: A-09 (REV-08) · Platform: M · Build: `178357e`
- Severity: S4 · Type: spec question
- Reproduces: 1 of 1
- Steps:
  1. `timeout 4 target/release/litty --version`; same with `--help`.
- Expected: undefined in the plan ("record what happens").
- Actual: no output; the process is still running after 4 s (it starts
  the terminal), matching `src/cli.rs:88-98` (unknown arguments fall
  through to `Command::Run`).
- Notes: there is no command-line way to print the version.

### ISSUE-005 — default shortcuts missing from `shortcuts.md`

- Cases: P6-01, C-01 (REV-05) · Platform: docs · Build: `178357e`
- Severity: S4 · Type: docs
- Reproduces: n/a (document check)
- Actual: Linux `Ctrl+PageUp` / `Ctrl+PageDown` (previous/next tab), Linux
  `Ctrl+Alt+D` (split down) and macOS `Cmd+Ctrl+=` (equalize splits) are
  bound in `src/keymap.rs` (lines 136-140, 209) but not listed in
  `docs/guide/shortcuts.md`.

### ISSUE-006 — docs say `open_config` uses `$EDITOR`; code uses `open -t` / `xdg-open`

- Cases: P6-02, H-01 (REV-03) · Platform: docs · Build: `178357e`
- Severity: S4 · Type: docs or spec question
- Actual: `src/config.rs:935` opens the file with `/usr/bin/open -t` or
  `xdg-open`; `docs/guide/configuration.md` and `docs/guide/shortcuts.md`
  say `$EDITOR`.

### ISSUE-007 — undocumented environment variables and config alias

- Cases: P6-02, G-02 (REV-06, extended) · Platform: docs · Build: `178357e`
- Severity: S4 · Type: docs
- Actual, not in the user guide:
  - `LITTY_FONT` and `LITTY_FONT_SIZE` (Environment table in
    `docs/guide/configuration.md`).
  - `GITHUB_TOKEN` / `GH_TOKEN`, read by the update client
    (`src/update/client.rs:167`), not in `docs/guide/updates-and-cli.md`.
  - `background-blur-radius`, accepted as an alias of `background-blur`
    (`src/config.rs:363`).

### ISSUE-008 — actions missing from `keybinding-actions.md`

- Cases: P6-03, C-12 (REV-04) · Platform: docs · Build: `178357e`
- Severity: S4 · Type: docs
- Actual: `increase_font_size`, `decrease_font_size`, `reset_font_size`,
  `scroll_page_up`, `scroll_page_down` are accepted by `src/keymap.rs` and
  not listed in the actions guide.

### ISSUE-009 — install docs name the v0.8.0 packages

- Cases: P6-04 (REV-07) · Platform: docs · Build: `178357e`
- Severity: S4 · Type: docs
- Actual: `README.md` line 42 and `docs/guide/installation.md` lines 17-42
  name `litty_0.8.0_amd64.deb` and `litty-0.8.0-1.x86_64.rpm`; the current
  release is v0.14.1.

### ISSUE-010 — idle CPU time still grows with cursor blink off

- Cases: P3-11 · Platform: M · Build: `178357e`
- Severity: S4 · Type: bug (to confirm)
- Reproduces: 2 of 2 (small, irregular)
- Steps:
  1. Isolated `HOME`, config `cursor-style-blink = false`, `SHELL=/bin/zsh`.
  2. Start `target/release/litty`, wait 15 s at the idle prompt, read
     `ps -o time= -p <pid>`, read again 60 s later.
- Expected: CPU time unchanged.
- Actual: run 1: 0:00.18 → 0:00.27 (+0.09 s); run 2 (sampled every
  10 s): 0:00.23 → 0:00.25, the increments at about t+45 s (`%cpu` 1.6)
  and t+55 s, the rest flat. With blink on: 0:00.27 → 0:00.52 (+0.25 s,
  expected ticks).
- Evidence: local runs `idle.sh`, `idle2.sh` (scratchpad).
- Notes: the window was not focused (started from a script). The source
  of the wake-ups is not identified; P3-13 (all windows closed) shows
  +0.01 s in 60 s and 0.0% `%cpu`, which passes.
