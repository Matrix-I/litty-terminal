# Full test run — issue note (2026-11-23)

Plan: [full feature test plan](../plans/2026-11-23-full-test-plan.md).
Status: the run has not started. Nothing below is fixed.

Issues are only logged here during the run. Fixing starts after the whole
plan has run (plan section 10).

## Run summary

Fill in at the start (environment) and at the end (counts).

| Item | macOS | Linux X11 | Linux Wayland |
| --- | --- | --- | --- |
| Commit | | | |
| OS / desktop / compositor | | | |
| GPU / driver | | | |
| Display scales | | | |
| Date run | | | |

| Phase | Run | Pass | Fail | Blocked | Skipped |
| --- | --- | --- | --- | --- | --- |
| P0 preparation | | | | | |
| P1 build gates | | | | | |
| P2 automated suites | | | | | |
| P3 performance | | | | | |
| P4 functional A–W | | | | | |
| P5 packaging/install | | | | | |
| P6 documentation | | | | | |

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

None yet.
