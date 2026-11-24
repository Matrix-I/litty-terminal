# Full test run — issue note (2026-11-23)

Plan: [full feature test plan](../plans/2026-11-23-full-test-plan.md).
Status: automated cases ran on 2026-11-23 on macOS (M column) and on
Linux with the X11 and Wayland backends (X and W columns). On 2026-11-24
the Linux manual cases ran too, driven by scripts in a private headless
GNOME Shell 50.1 session (input injected through Mutter RemoteDesktop);
macOS manual cases have not run. Nothing below is fixed.

Issues are only logged here during the run. Fixing starts after the whole
plan has run (plan section 10).

## Run summary

Fill in at the start (environment) and at the end (counts).

| Item | macOS | Linux X11 | Linux Wayland |
| --- | --- | --- | --- |
| Commit | `178357e` (rustc 1.99.1) | `fc3d1d4` (rustc 1.100.0; container 1.99.1) | same as X11 |
| OS / desktop / compositor | macOS 27.2.1 (26B131), arm64 | Ubuntu 26.04.1, kernel 7.0.0-44, GNOME Shell 50.1 Wayland session (XWayland) | same machine, native Wayland (`LITTY_BACKEND=wayland`) |
| GPU / driver | Apple M1 Pro (Metal) | Intel UHD 620 (Whiskey Lake), Mesa Vulkan; CPU i5-8365U, `powersave` governor, on AC | same |
| Display scales | Built-in Retina 3024×1964, scale 2 | eDP 3072×1728 at 59.94 Hz, GNOME scale 1.25 (window reports 2 under XWayland native scaling) | same panel, window scale 1.25 |
| Date run | 2026-11-23 (automated cases only) | 2026-11-23 (automated); 2026-11-24 (manual cases, headless GNOME session) | same as X11 |

| Phase | Platform | Run | Pass | Fail | Blocked | Skipped |
| --- | --- | --- | --- | --- | --- | --- |
| P0 preparation | M | 4 | 4 | 0 | 0 | 2 |
| | X | 6 | 6 | 0 | 0 | 0 |
| | W | 6 | 6 | 0 | 0 | 0 |
| P1 build gates | M | 11 | 11 | 0 | 0 | 0 |
| | X | 11 | 11 | 0 | 0 | 0 |
| | W | — | — | — | — | — |
| P2 automated suites | M | 6 | 5 | 0 | 1 | 0 |
| | X | 9 | 6 | 2 | 1 | 0 |
| | W | 3 | 2 | 0 | 1 | 0 |
| P3 performance | M | 14 | 9 | 4 | 1 | 5 |
| | X | 17 | 13 | 4 | 0 | 1 |
| | W | 11 | 10 | 1 | 0 | 1 |
| P4 functional A–W | M | 10 | 9 | 1 | 0 | 265 |
| | X | 278 | 219 | 30 | 29 | 0 |
| | W | 268 | 219 | 20 | 29 | 0 |
| P5 packaging/install | M | 2 | 2 | 0 | 0 | 1 |
| | X | 6 | 3 | 0 | 3 | 0 |
| | W | 4 | 1 | 0 | 3 | 0 |
| P6 documentation | M | 5 | 0 | 4 | 1 | 0 |
| | X | 6 | 0 | 6 | 0 | 0 |
| | W | 6 | 0 | 6 | 0 | 0 |

"Run" = pass + fail + blocked. Cases marked `—` are not counted.
Linux X and W counts are taken from the plan's result cells after the
2026-11-24 manual run; only P3-10 is still skipped. Linux manual cases ran
in a private headless GNOME Shell 50.1 session (`gnome-shell --headless
--virtual-monitor 1280x800`) with keyboard and pointer injected through
Mutter RemoteDesktop, so nothing reached the real desktop; a physical
keyboard and real dock or file manager were not available there, which is
why several cases are `B`. On macOS, "Skipped" is almost all manual GUI
cases, which that run left out on purpose. The 2026-11-23 Linux automated
pass had covered only the P4 cases a script running as `SHELL` inside
Litty can check (terminal replies, stderr, `xprop`, process state). P3-10
is skipped because it opens the app. Each run used an isolated `HOME` with
`auto-update = off` instead of backing up the real `~/.litty` (P0-02).

Shell coverage (P2-13): macOS zsh, bash 3.2.57 and nu (fish and elvish
not installed); Linux zsh 5.9 and bash 5.3.9 (fish, nu and elvish not
installed, skipped by the test). Linux runs used the release binary from
`bundle_linux.sh` (pinned minisign key) for CLI and update cases, and
`target/release/litty` for benchmarks. Documentation cases (P6) are the
same files for both Linux backends, so W repeats X.

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

Status after the automated macOS and Linux runs:

| REV | Status |
| --- | --- |
| REV-01 | Open: E-26 blocked on X11 (no file manager in the headless session); Wayland drops pass (P2-10) |
| REV-02 | Open: Linux only |
| REV-03 | Confirmed → ISSUE-006 |
| REV-04 | Confirmed → ISSUE-008 |
| REV-05 | Confirmed → ISSUE-005 |
| REV-06 | Confirmed and extended → ISSUE-007 |
| REV-07 | Confirmed → ISSUE-009 |
| REV-08 | Confirmed → ISSUE-004 |
| REV-09 | Open: U-02 ran on Linux with a 2-line paste; the single line plus newline count was not checked |
| REV-10 | Confirmed → ISSUE-019 |

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

### ISSUE-011 — update cases fail with a plain release build (no pinned key)

- Cases: R-03, R-04 (plan steps of R-02/R-03 say only "release build") ·
  Platform: M · Build: `178357e`, `v0.14.0`
- Severity: S4 · Type: test tooling
- Reproduces: 3 of 3 without the key; 0 of 3 with it
- Steps:
  1. `cargo build --release` (no `LITTY_MINISIGN_PUBKEY`) for `v0.14.0`
     and for `178357e`; copy each binary to a user-writable folder.
  2. `litty +update` on the v0.14.0 copy; `+update --force` and
     `+update -f` on the v0.14.1 copies.
- Expected: R-03 updates to v0.14.1; R-04 reinstalls v0.14.1.
- Actual: `Error downloading/verifying update: this build has no pinned
  minisign key; refusing an unverifiable update`, exit 1, binary unchanged.
  Rebuilt with
  `LITTY_MINISIGN_PUBKEY="$(tail -n 1 resources/minisign.pub)"`, all three
  succeed: `SHA-256 checksum verified`, `Successfully updated Litty to
  v0.14.1!`, exit 0, `+check-update` then reports `Current version:
  v0.14.1`.
- Evidence: `src/update/verify.rs:205` (`option_env!("LITTY_MINISIGN_PUBKEY")`)
  and `:243-245` (refusal without a key). P5-06 verified the same release
  with `minisign -Vm` (signature and trusted comment OK) and 6/6 hashes.
- Notes: refusing is the intended secure behavior; the plan steps should
  say to build with the pinned key (as `bundle_macos.sh` does). The
  updater output names only the SHA-256 check; the minisign check is
  implied by the code path above, not printed.

### ISSUE-012 — PTY test fails under a fresh `HOME` when `SHELL` is zsh

- Cases: P1-04 (fresh `HOME`) · Platform: X, Ubuntu 26.04, zsh 5.9 ·
  Build: `fc3d1d4`
- Severity: S4 · Type: test tooling
- Reproduces: 4 of 4 with a fresh `HOME` and `SHELL=/usr/bin/zsh`; 0 of 1
  with `SHELL=/bin/bash`; 0 of 1 with an empty `$HOME/.zshrc`
- Steps:
  1. `HOME=$(mktemp -d) cargo test shell_exit_is_reported_and_reaped`
- Expected: the test passes whatever the user's shell and `HOME`.
- Actual: panic at `src/terminal/pty.rs:1255:61`: `on_exit fires: Timeout`
  after 10 s. The full suite: 875 passed, 1 failed. With the real `HOME`
  the suite passes (876 + 1 + 14).
- Evidence: `printf 'exit\r' | HOME=$(mktemp -d) script -qc zsh /dev/null`
  prints zsh's `zsh-newuser-install` menu, which reads the `exit` line as
  a menu answer, so the shell never exits.
- Notes: the test spawns the user's `$SHELL` (`Pty::spawn_in`), so its
  result depends on the developer's shell and dotfiles. Same family as
  ISSUE-002.

### ISSUE-013 — `linux_xdg_open_dispatch_smoke` opens real apps on a desktop

- Cases: P2-05 · Platform: X, GNOME 50 Wayland session · Build: `fc3d1d4`
- Severity: S4 · Type: test tooling
- Reproduces: 3 of 3 (also with `XDG_CURRENT_DESKTOP`, `DESKTOP_SESSION`
  and `DBUS_SESSION_BUS_ADDRESS` unset)
- Steps:
  1. `cargo test -- --ignored linux_xdg_open_dispatch_smoke --nocapture`
     in a GNOME session.
- Expected (plan): pass.
- Actual: `assertion left == right failed: executable/script paths must
  open their containing directory`, `left: []`; stdout shows `Opening in
  existing browser session.` The files and URL were opened in the
  desktop's real handlers (text editor, file manager, browser) instead of
  the stub `$BROWSER`.
- Evidence: `sh -x /usr/bin/xdg-open` with the variables above unset still
  selects `DE=gnome3` (it asks `org.gnome.SessionManager` on the session
  bus at its default path) and calls `gio open`, which ignores `$BROWSER`.
- Notes: the test is marked "run via `scripts/linux/run.sh`" (container,
  no desktop); the plan lists it as a real-hardware case. Either run it
  only in the container or isolate it from the desktop session.

### ISSUE-014 — heap corruption abort in the `cargo test` step of `pre-release-check.sh`

- Cases: P3-01 · Platform: X, Ubuntu 26.04, GNOME 50, `XMODIFIERS=@im=ibus`
  · Build: `fc3d1d4`
- Severity: S3 · Type: bug (test-only so far)
- Reproduces: 1 of 1 `pre-release-check.sh` run; 0 of 3 other full host
  `cargo test` runs this session (P1-04 twice, P1-05); not seen in the 3
  container runs (P2-08)
- Steps:
  1. `bash scripts/pre-release-check.sh` (isolated `HOME`).
- Expected: exit 0.
- Actual: `malloc(): smallbin double linked list corrupted`, test binary
  killed by `SIGABRT`, `✘ One or more cargo tests failed!`, exit 1. The
  RAM part of the same run passed.
- Notes: matches the host-only intermittent heap corruption with
  `ibus-x11` already recorded in
  [release-0.14.0](release-0.14.0.md) ("still unexplained"). Not seen
  outside tests.

### ISSUE-015 — Linux misses several `+bench-perf` / `+bench-stress` limits

- Cases: P3-01, P3-03, P3-04, P3-05a · Platform: X, Intel Core i5-8365U
  (8 threads, `powersave` governor, on AC), UHD 620, Ubuntu 26.04 ·
  Build: `fc3d1d4` (release, no other heavy apps open)
- Severity: S3 · Type: bug or spec question (limits may be macOS-calibrated)
- Reproduces: throughput 6 of 6 runs; others vary (below)
- Steps:
  1. `litty +bench-perf`, `+bench-stress 100000`, `+bench-features`,
     three times each, isolated `HOME`.
- Expected: exit 0 with every limit met.
- Actual (failing limits only):
  - Throughput ≥ 350,000 lines/s: 222,957 / 154,734 / 278,755 (P3-03),
    223,363 / 194,174 / 277,847 (P3-04), 162,590 (P3-01).
  - Worst frame stall ≤ 3 ms: 3.75 ms (P3-03 r2), 3.34 and 4.10 ms
    (P3-04 r1, r2), 3.75 ms (P3-01).
  - 64 KB file read ≤ 3 ms: 3.57, 4.89 ms; 1 MB file read ≤ 40 ms:
    50.31 ms (P3-03 r1, r2); 68.89 ms (P3-01).
  - 256 KB Kitty APC ≤ 3 ms: 3.12, 3.91 ms; 256 KB Sixel ≤ 5 ms: 6.91,
    9.01, 5.95 ms (P3-03 r1–r3).
  - 500 BiDi clusters ≤ 0.2 ms: 0.22 ms (P3-03 r2), 0.21 ms (P3-01).
  - FZF 10k ≤ 2 ms: 2.18, 2.65 ms (P3-03), 3.57 ms (P3-05a r1), 2.60–
    2.83 ms (P3-01); Regex 5k ≤ 0.5 ms: 0.716 ms (P3-05a r1).
  - Passing: DSR mean/P99, PTY spawn (1.24 ms), 500 URLs, post-stress
    probe, P3-05a runs 2 and 3.
- Notes: `docs/development/performance.md` has no Linux numbers for these
  benches. Related to ISSUE-001 (macOS stall). Not checked: the
  `performance` governor.

### ISSUE-016 — native Wayland scroll misses every fifth frame on a 60 Hz panel

- Cases: P3-09 (W) · Platform: W, GNOME 50, eDP 3072×1728 at 59.999 Hz,
  scale 1.25 · Build: `fc3d1d4`
- Severity: S3 · Type: bug (to confirm)
- Reproduces: 1 of 1 run (3 scenarios)
- Steps:
  1. `LITTY_BACKEND=wayland litty +bench-scroll-fps`
- Expected: about 60 fps, as the X11 backend gives on the same panel.
- Actual: `submission_fps` 48.4 / 49.2 / 50.0; `interval_p50_ms` 16.75
  but `interval_p95_ms` 33.5; `intervals_over_25ms` 46 / 43 / 40 of
  about 194 frames; `render_p95_ms` 0.26, `acquire_p95_ms` 0.06. X11
  (`P3-09-x11`): 60.0 fps, p95 17.2 ms, 0 intervals over 25 ms.
- Notes: rendering is fast, so the lost frames are in frame pacing (frame
  callback / present), not drawing. Wayland `+bench-input-latency` p95 is
  29.0 ms against 13.7 ms on X11 (echo p95 21.5 ms), possibly the same
  cause.

### ISSUE-017 — a missing `SHELL` makes Litty exit silently

- Cases: A-03 · Platform: X and W, GNOME 50 · Build: `fc3d1d4`
- Severity: S3 · Type: bug
- Reproduces: 3 of 3 (X), 1 of 1 (W)
- Steps:
  1. `SHELL=/nonexistent/shell litty`
- Expected: a clear error or a fallback shell, never a hang or crash.
- Actual: the process exits 0 after about 0.35 s with nothing on stderr;
  no window stays open to show an error. `SHELL=/bin/sh` and bash start
  normally.
- Notes: `Pty::spawn_shell` (`src/terminal/pty.rs:318`) uses `$SHELL` as
  given; the child's exec error goes to the PTY and the child exits 127,
  so the only tab closes and the app quits. Not checked by eye whether the
  window flashes.

### ISSUE-018 — `+update` on a package install downloads, then fails with a raw error

- Cases: R-07 · Platform: X, Ubuntu 26.04, `litty` 0.14.0 `.deb`
  (`/usr/bin/litty`, root-owned) · Build: `v0.14.0` package
- Severity: S4 · Type: bug or spec question
- Reproduces: 1 of 1
- Steps:
  1. As a normal user: `/usr/bin/litty +update`
- Expected: refused with a clear message; nothing changed.
- Actual: downloads and verifies v0.14.1, then `Installing update...
  failed.` / `Error applying update: cannot copy binary: Permission denied
  (os error 13)`, exit 1. `/usr/bin/litty` is unchanged (same SHA-256)
  and no stray file is left in `/usr/bin`.
- Notes: nothing is damaged, but the message does not say the binary is
  managed by a package manager (use `apt`/`dnf`), and the download happens
  first. Checked on the v0.14.0 package only.

### ISSUE-019 — `scripts/manual-test.sh` assumes macOS

- Cases: P6-06 (REV-10) · Platform: X, W · Build: `fc3d1d4`
- Severity: S4 · Type: test tooling
- Reproduces: n/a (script check)
- Actual: unchanged since REV-10: `pbpaste` (`scripts/manual-test.sh:340`),
  `/System/Library/Sounds/Glass.aiff` (`:256`), `Cmd` wording in about 25
  prompts (for example `:154`, `:198`, `:275`); only group 26 checks `$OS`.
  No Linux substitutions are documented.

### ISSUE-020 — `rounded_corners_fade_a_premultiplied_frame` fails in the container's full suite

- Cases: P2-08 · Platform: container (Debian 13, Mesa lavapipe, Xvfb),
  host Ubuntu 26.04 with Podman · Build: `fc3d1d4`
- Severity: S3 · Type: bug or test tooling (order-dependent)
- Reproduces: 3 of 3 full-suite runs (`--test-threads=1`); 0 of 4 when the
  test runs alone in the same image; 0 of 3 on the host (Intel UHD 620)
- Steps:
  1. `bash scripts/linux/test.sh`
- Expected: exit 0.
- Actual: `src/render/renderer/tests.rs:191`: `(3, 0) [8, 12, 17, 80] is
  not premultiplied`; later runs `[7, 11, 15, 98]` and `[8, 12, 17, 90]`
  (want about 10/15/20 at alpha 80). The suite is 875 passed, 1 failed, so
  `run.sh` stops before its smoke steps. Run by hand in the same image, the
  remaining steps pass: `linux_xdg_open_dispatch_smoke`, `x11_smoke.py`
  (4/4), `wayland_smoke.py` (3/3), `wayland_sway_smoke.py` (3/3).
- Notes: the corner pixel's alpha changes between runs, so the frame
  depends on state left by earlier tests (shared device or pipelines) or
  on uninitialised memory. Test added in `42320d7`.

### ISSUE-021 — a window title over about 4 KB kills Litty on Wayland

- Cases: I-14 (W) · Platform: W, GNOME 50 · Build: `fc3d1d4`
- Severity: S1 · Type: bug
- Reproduces: 3 of 3 (title of 4,100, 5,000 and 100,000 bytes); 0 of 4 at
  1,000 and 4,000 bytes; 0 of 5 on X11 (same lengths)
- Steps:
  1. `LITTY_BACKEND=wayland litty`, then in the shell:
     `python3 -c "print('\x1b]2;' + 'A'*4100 + '\x07', end='')"`
- Expected: the title is truncated or ignored; the terminal keeps running.
- Actual: stderr `wl_display#1: error 1: message length 4116 exceeds 4096`,
  `litty: lost the connection to the Wayland compositor: Io error: Invalid
  argument (os error 22)`; Litty exits 1 with every window and shell.
- Notes: any program can print such a title, so output from an untrusted
  source can close all of the user's sessions. The Wayland
  `xdg_toplevel.set_title` message is not length-limited before sending.

### ISSUE-022 — default background is `#000000`, docs say `#121316`

- Cases: H-13 · Platform: X and W · Build: `fc3d1d4`
- Severity: S4 · Type: docs or spec question
- Reproduces: 2 of 2 (one per backend), empty config
- Steps:
  1. Empty config; in the shell query `printf '\e]11;?\a'`.
- Expected (plan, `docs/guide/configuration.md:34` and `:80`): `#121316`.
- Actual: `\e]11;rgb:0000/0000/0000\a`; foreground `rgb:e6e6/e6e6/e6e6`
  matches. `src/config.rs:182` sets `background: 0x0000_0000`;
  `#121316` is palette colour 0 (`src/config.rs:79`) and the commented
  example in the config template (`src/config.rs:975`).
- Notes: decide which one is intended; the screen colour itself was not
  checked by eye.

### ISSUE-023 — a UTF-8 BOM breaks the first config line

- Cases: H-16 · Platform: X and W · Build: `fc3d1d4`
- Severity: S3 · Type: bug
- Reproduces: 2 of 2 (one per backend)
- Steps:
  1. Config written as `EF BB BF` + `background\t=\t"#405060"\r\n` +
     `foreground = "#a0b0c0"\r\n`.
- Expected: parsed (BOM ignored).
- Actual: `line 2: ﻿background: unknown option` (the BOM is part of the
  key; line 2 because the harness prepends `auto-update = off`, so in a
  user's file it is line 1); background stays default. The second line
  with CRLF, spaces and quotes applies (`rgb:a0a0/b0b0/c0c0`). Without the
  BOM, `background\t=\t"#405060"` with CRLF applies (`rgb:4040/5050/6060`).
- Notes: Windows editors (Notepad) save UTF-8 with a BOM by default.


### ISSUE-024 — dragging a tab out does not reliably tear it off on X11

- Cases: B-08 · Platform: X, Ubuntu 26.04.1, private headless GNOME Shell
  50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: 0 of 3 straight-down drags; 6 of 9 drags with sideways
  motion; 1 of 2 drops below the window
- Steps:
  1. Window with 2 tabs; drag the 2nd tab straight down by 300 px inside
     the window, release.
  2. Repeat with sideways motion, and with a drop below the window.
- Expected: A new window holds the dragged tab.
- Actual: Straight-down drags never tear out; other drags work about two
  times in three.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-025 — dropping a tab on another window’s tab strip makes a new window

- Cases: B-09 · Platform: X, Ubuntu 26.04.1, private headless GNOME Shell
  50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: 1 of 1
- Steps:
  1. Window A and window B with 2 tabs each.
  2. Drag a tab of B onto A’s tab strip and release.
- Expected: A has 3 tabs, B has 1.
- Actual: A still has 2 tabs; the tab is torn out into a 3rd window.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-026 — GNOME default shortcuts take Ctrl+Alt+arrows and Alt+`

- Cases: C-10, C-01 · Platform: X and W, Ubuntu 26.04.1, private headless
  GNOME Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: spec question
- Reproduces: every attempt
- Steps:
  1. GNOME 50.1 with default bindings, 2 panes.
  2. Press Ctrl+Alt+Right/Left/Up/Down, Ctrl+Alt+Shift+Right, Alt+`.
- Expected: Pane navigation and resize act in Litty.
- Actual: Ctrl+Alt+Right switches the workspace (focus lost); the other
  Ctrl+Alt arrows and Ctrl+Alt+Shift+Right reach no Litty action; Alt+` is
  taken. Super+arrows, Super+[ ] and Super+Ctrl+arrows reach Litty.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: Linux defaults clash with the GNOME desktop; the docs could name
  a working alternative.

### ISSUE-027 — `keybind = ctrl++=…` parses but never matches

- Cases: C-14 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: 1 of 1 per backend
- Steps:
  1. Config `keybind = ctrl++=text:S2`.
  2. Press Ctrl+Shift+= (Ctrl++).
- Expected: Sends `S2`, or the config reports an error.
- Actual: No config error; the default increase_font_size runs (cols 119 →
  105) and nothing is sent.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-028 — F1–F12 send no bytes on X11

- Cases: C-21 · Platform: X, Ubuntu 26.04.1, private headless GNOME Shell
  50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S2 · Type: bug
- Reproduces: every key and modifier in the run
- Steps:
  1. `cat -v` in Litty with the X11 backend.
  2. Press F1–F12, alone and with Shift/Alt/Ctrl.
- Expected: xterm sequences such as `^[OP`, `^[[15~`.
- Actual: Nothing arrives. The same injected keys reach a GTK window on
  X11 (F5, F12) and Litty on Wayland.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: Physical keyboard not checked; injection goes through Mutter
  RemoteDesktop.

### ISSUE-029 — modified F1 uses rxvt-style sequences

- Cases: C-21 · Platform: W, Ubuntu 26.04.1, private headless GNOME Shell
  50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: spec question
- Reproduces: every attempt
- Steps:
  1. `cat -v`, press Shift+F1, Alt+F1, Ctrl+F1.
- Expected: xterm `CSI 1;2P` / `CSI 1;3P` / `CSI 1;5P`.
- Actual: `CSI 11;2~`, `CSI 11;3~`, `CSI 11;5~`.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: A `src/keymap.rs` test expects `\x1b[1;2P` on a different path.

### ISSUE-030 — Copy leaves CLIPBOARD unchanged on X11

- Cases: E-02, E-07, E-10, C-01 · Platform: X, Ubuntu 26.04.1, private
  headless GNOME Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at
  `95c2158`)
- Severity: S2 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Select text; press Ctrl+Shift+C (also Ctrl+Insert, and Select All
     then Copy).
  2. Read CLIPBOARD with `wl-paste`.
- Expected: CLIPBOARD holds the selection.
- Actual: CLIPBOARD is unchanged; no stderr. Copy-on-select does set
  CLIPBOARD and PRIMARY; Wayland copies fine.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-031 — Shift+Click does not extend a selection, and Shift snaps to the bottom, on X11

- Cases: E-06, E-20, K-03 · Platform: X, Ubuntu 26.04.1, private headless
  GNOME Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Select a word; Shift+Click further on.
  2. Scroll back; press Shift alone.
  3. Press Ctrl+Shift+Up repeatedly.
- Expected: Selection extends; Shift alone keeps the scroll position;
  Ctrl+Shift+Up walks back through prompts.
- Actual: Selection does not extend; Shift alone jumps to the bottom, so
  Ctrl+Shift+Up never gets past the last prompt. Wayland is correct.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-032 — `window-width` / `window-height` give 2 rows fewer on Linux

- Cases: G-22 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: 1 of 1 per backend
- Steps:
  1. Config `window-width = 100`, `window-height = 30`.
  2. Query the size with `stty size`.
- Expected: 30 rows × 100 columns.
- Actual: 28 × 100.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: `src/app/window.rs:71` sizes the window as rows × cell + padding,
  but Linux draws its own title bar inside that area.

### ISSUE-033 — Devanagari and some combining marks are not shaped

- Cases: G-12 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Print `कि`, `नमस्ते`, `x̧̌`.
- Expected: Shaped text: i-matra before its base, virama and e-matra
  drawn, both marks on x.
- Actual: i-matra drawn after its base; virama and e-matra missing; the
  cedilla missing. Latin and Vietnamese stacked diacritics are fine.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-034 — kitty graphics delete with an unknown selector is silent

- Cases: J-07 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: spec question
- Reproduces: 1 of 1 per backend
- Steps:
  1. Send `ESC _G a=d,d=Q ESC \`.
- Expected: An error reply (kitty replies EINVAL).
- Actual: No reply; ignored.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-035 — `manual-test.sh img shm` uses a name Litty refuses

- Cases: J-01 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: test tooling
- Reproduces: every attempt
- Steps:
  1. `scripts/manual-test.sh`, group 24, shm item.
- Expected: The image shows.
- Actual: Nothing shows: the script names its segment `/lpl<pid>`, and
  Litty accepts only `/kitty-`, `/litty-`, `/ghostty-`, `/shm-`
  (`src/terminal/graphics/kitty.rs:407`).
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: Either the script or the allowlist is wrong; kitty accepts any
  name. Group 24 "under" also expects red cells to cover a z = −1 image,
  but by the kitty spec only z < −1073741824 goes under non-default
  backgrounds, which Litty does.

### ISSUE-036 — Unicode-placeholder images show seams between rows on Wayland

- Cases: J-01, P6-05 · Platform: W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Place a 10×4-cell image with Unicode placeholders.
- Expected: One seamless image.
- Actual: 1 px black seams between cell rows. The same binary on X11 draws
  it seamless.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-037 — bash title shows only the cwd, not the running command

- Cases: K-06, P6-05 · Platform: X and W, Ubuntu 26.04.1, private headless
  GNOME Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Default `title` feature; run `sleep 5` in zsh and in bash.
- Expected: Title is the directory, then the running command, in every
  shell.
- Actual: zsh shows the command; bash (`src/shell/litty.bash`) only ever
  reports the cwd.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-038 — the history popup drops Space

- Cases: L-02 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Ctrl+R; type `zeta word`.
- Expected: The query holds a space; multi-word AND and `term !neg` work.
- Actual: The space is dropped; multi-word and negation queries never
  match. `zeta word` only finds `zeta multi word` because `zetaword`
  matches fuzzily.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: `handle_history_search_key` (`src/app/search_ui.rs:14`) has no
  `NamedKey::Space` arm; the find box has one.

### ISSUE-039 — the find box does not refresh while output streams

- Cases: M-08 · Platform: W (X not isolated, see ISSUE-043), Ubuntu
  26.04.1, private headless GNOME Shell 50.1 · Build: `fc3d1d4` (`src/`
  unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: 1 of 1
- Steps:
  1. Ctrl+Shift+F; type `stream-2500$`.
  2. Start output that prints that line.
- Expected: The match count updates.
- Actual: Still `no matches` 15 s later; closing and reopening finds 1/1.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: `refresh_search` runs only on key input (`src/app/search_ui.rs`).

### ISSUE-040 — Ctrl+Click on a soft-wrapped path opens a prefix

- Cases: N-03, P6-05 · Platform: X and W, Ubuntu 26.04.1, private headless
  GNOME Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: 1 of 1 per backend
- Steps:
  1. Print a 160-character absolute file path that wraps.
  2. Ctrl+Click it.
- Expected: The file opens.
- Actual: Its parent prefix (`…/homes`) opens, cut at the wrap.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-041 — plain-text links recognise only http(s)

- Cases: N-02 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Print `mailto:someone@example.com` and `ftp://ftp.example.com/file`.
  2. Ctrl+Click each.
- Expected: They open.
- Actual: Not clickable. OSC 8 accepts these schemes
  (`src/terminal/grid/links.rs:469`); plain-text detection (`links.rs:55`)
  knows only http(s).
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-042 — the scanlines example shader makes the window translucent

- Cases: P-02 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. `custom-shader = examples/shaders/scanlines.glsl`,
     `background-opacity` 1.
- Expected: Opaque window.
- Actual: The desktop shows through: the shader multiplies alpha by s < 1.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: Either the example should keep alpha, or the post-process pass
  should force opaque output for an opaque window.

### ISSUE-043 — X11 input starves while a pane floods output

- Cases: V-04, M-08 · Platform: X, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S2 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Split into 2 or 4 panes; run `yes stream-line` in one.
  2. Press Ctrl+C there; click another pane and type.
- Expected: Responsive: Ctrl+C lands at once, other panes take input.
- Actual: Ctrl+C takes 20–60+ s; typing into other panes never arrives
  while the flood runs. Wayland: about 2 s. During the flood the
  `pty-reader` thread uses a full core, the main thread about 3 %,
  Xwayland is idle.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: The main thread is waiting, not busy. Grid-lock or event-loop
  wakeup starvation on the X11 path is a guess, not verified.

### ISSUE-044 — `yo` retries a refused connection for 4 minutes

- Cases: W-04 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: 1 of 1 per backend
- Steps:
  1. Point the AI endpoint at `127.0.0.1:9` (nothing listening).
  2. Run `yo hello`.
- Expected: Fails fast with a clear error.
- Actual: Retries 10 times with backoff, 242 s before the error; Enter
  typed meanwhile waits the whole time.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: A refused connection is not transient.

### ISSUE-045 — AI transport error shows Rust Debug formatting

- Cases: W-04 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. As ISSUE-044.
- Expected: A readable error.
- Actual: `AI transport failed (curl exit Some(7))`.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-046 — SIGTERM / SIGHUP leave the reuse-registry entry behind

- Cases: A-10, A-14 · Platform: X and W, Ubuntu 26.04.1, private headless
  GNOME Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: 20 of 20
- Steps:
  1. Start Litty; send SIGTERM (or SIGHUP).
  2. List `$XDG_RUNTIME_DIR/litty-a1`.
- Expected: Entry removed on exit.
- Actual: Default signal action (exit −15/−1); the entry stays. Shells are
  cleaned up; later launches treat the entries as stale (Q-05), so the
  impact is clutter.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-047 — Linux never withdraws a notification when the window regains focus

- Cases: T-16 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Window in the background; OSC 9 notification arrives.
  2. Focus the window again.
- Expected: The notification is withdrawn, as on macOS.
- Actual: No `CloseNotification` call; the banner stays in the GNOME list.
  Clicking the banner works and selects the tab that rang.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: on quit (O-09) GNOME Shell closes the notification itself
  (`NotificationClosed`, reason 2) as Litty exits; Litty still sends no
  `CloseNotification`.

### ISSUE-048 — a confirm dialog freezes every window

- Cases: T-08 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: every attempt
- Steps:
  1. Stream output in one window; trigger a confirm (close with a running
     process).
- Expected: Other windows keep drawing and taking input.
- Actual: All windows stop redrawing and handling input until the dialog
  closes; output is read meanwhile and shows afterwards.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: Confirm dialogs run `zenity … .status()` on the main thread
  (`src/app/tabs.rs`).

### ISSUE-049 — with an IME active, find box and history popup input goes to the shell

- Cases: D-05 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S3 · Type: bug
- Reproduces: every attempt
- Steps:
  1. ibus with Unikey (Telex) active.
  2. Ctrl+Shift+F (or Ctrl+R) and type.
- Expected: The query takes the text.
- Actual: Preedit and committed text go to the shell prompt behind the
  box; the query stays empty. With Unikey on even ASCII letters arrive as
  commits, so the boxes cannot be typed into.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-050 — keys stop reaching Litty after the XIM server is killed (unconfirmed)

- Cases: D-06 · Platform: X, Ubuntu 26.04.1, private headless GNOME Shell
  50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S2 · Type: bug
- Reproduces: 1 of 1, not re-run
- Steps:
  1. Litty on X11 with `ibus-daemon --xim`.
  2. Kill ibus; type.
- Expected: Plain text input continues.
- Actual: Typed text did not reach Litty within 5 s. Wayland recovers
  (plain text, composes again after restart).
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
- Notes: Not re-run: killing ibus made the headless GNOME Shell rewrite
  the input-source order in the shared user dconf.

### ISSUE-051 — Ctrl+, opens the config with xdg-open, not `$EDITOR`

- Cases: C-01 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: docs
- Reproduces: every attempt
- Steps:
  1. Press Ctrl+,.
- Expected: `docs/guide/shortcuts.md`: opens `~/.litty/config` in
  `$EDITOR`.
- Actual: Runs `xdg-open` (`open_config_in_editor` in `src/config.rs`), so
  the desktop text handler opens it and `$EDITOR` is ignored.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo

### ISSUE-052 — `+update` from a cached download still says "Downloading"

- Cases: R-11 · Platform: X and W, Ubuntu 26.04.1, private headless GNOME
  Shell 50.1 · Build: `fc3d1d4` (`src/` unchanged at `95c2158`)
- Severity: S4 · Type: bug
- Reproduces: 1 of 1
- Steps:
  1. `auto-update = download`; launch; wait for "Litty Update Ready".
  2. Run `litty +update`.
- Expected: Installs from the cache and says so.
- Actual: Prints "Downloading the linux/x86_64 build..." although the exec
  trace shows only the releases API call; installs v0.14.1 correctly.
- Evidence: headless harness run (input via Mutter RemoteDesktop); logs
  and screenshots kept outside the repo
