# Full test run — fix backlog (2026-11-26)

Source: [issue note](../validation/2026-11-23-full-test-issues.md) of the
[2026-11-23 full test run](2026-11-23-full-test-plan.md). Issue IDs, steps
and evidence live in the issue note; this file only groups and orders the
open ones so they can be ticked off.

## How to use

- Work groups top to bottom, items top to bottom: both are sorted easiest
  first.
- Tick `[x]` when the fix is merged **and** verified (CLAUDE.md
  verification steps; e2e re-run where the issue came from a manual case).
  Also add a `Status:` line to the issue in the issue note.
- One branch per group (`fix/<group-topic>`), one commit per issue.
- Items marked **needs decision** wait for the matching entry in
  [Decisions](#decisions) first.
- Items marked **unconfirmed** come from source reading: reproduce first;
  if it does not reproduce, close it in the issue note instead of fixing.

Effort: **L** low (one function or file, obvious fix) · **M** medium
(several files or a design choice inside one module) · **H** high
(cross-module, platform protocol work, or no root cause yet).
Severity S1–S4 as defined in the issue note.

## Progress

| # | Group | Effort | Items | Done |
| --- | --- | --- | --- | --- |
| 1 | Docs | L | 7 (+3 after decisions) | 0 |
| 2 | Test tooling | L–M | 8 | 0 |
| 3 | Lifecycle, config, CLI | L–M | 6 | 0 |
| 4 | Update and AI | L–M | 4 | 0 |
| 5 | Search, links, text, render | L–H | 10 | 0 |
| 6 | Linux input and IME | M–H | 7 | 0 |
| 7 | Windows, tabs, dialogs | M–H | 6 | 0 |
| 8 | Performance | M–H | 5 | 0 |

## Decisions

Answer these before the items that depend on them.

- [x] **D1** — ISSUE-004: should `--version` / `--help` print and exit
  instead of opening a window? (blocks ISSUE-004)
  Answer: print and exit 0.
- [x] **D2** — ISSUE-006 / ISSUE-051: should `open_config` use `$EDITOR`
  (in a terminal tab) or keep `open -t` / `xdg-open` and fix the docs?
  Answer: keep `open -t` / `xdg-open`, docs fixed.
- [x] **D3** — ISSUE-022: is the default background `#000000` (fix docs)
  or `#121316` (fix code)?
  Answer: `#000000`, docs fixed.
- [ ] **D4** — ISSUE-026: change the Linux pane shortcuts that clash with
  GNOME, or only document the clash?
- [ ] **D5** — ISSUE-029: modified F1 as xterm `CSI 1;nP` or keep rxvt
  `CSI 11;n~`?
- [ ] **D6** — ISSUE-034: reply `EINVAL` to an unknown kitty delete
  selector, like kitty?
- [ ] **D7** — ISSUE-042: fix the scanlines example to keep alpha, or force
  an opaque result in the post-process pass?
- [ ] **D8** — ISSUE-003 / ISSUE-015: should macOS drive ProMotion at
  120 Hz, and which `+bench-perf` / `+bench-stress` limits apply to Linux?

## 1. Docs — `docs/<topic>`

- [ ] **ISSUE-009** (S4, L) — install docs name the v0.8.0 packages
  (`README.md:42`, `docs/guide/installation.md`).
- [ ] **ISSUE-005** (S4, L) — default shortcuts missing from
  `shortcuts.md`.
- [ ] **ISSUE-008** (S4, L) — actions missing from
  `keybinding-actions.md`.
- [ ] **ISSUE-007** (S4, L) — undocumented environment variables and config
  alias.
- [ ] **ISSUE-022** (S4, L) — default background docs vs code. Needs
  decision D3.
- [ ] **ISSUE-006 + ISSUE-051** (S4, L if docs only, M if code) —
  `open_config` and `$EDITOR`. Needs decision D2.
- [ ] **ISSUE-026** (S3, L if docs only) — GNOME shortcut clash. Needs
  decision D4.
- [ ] **Follow-up** (S4, L) — the install commands' globs
  (`./litty_*_amd64.deb`, `./litty-*.x86_64.rpm` in `README.md:42`,
  `docs/guide/installation.md`) match several files when older downloads
  are in the directory; say to use the downloaded file's name. Found in
  the `fix/docs-guide-sync` review.
- [ ] **Follow-up** (S4, L) — `docs/development/building.md:104-123`
  packaging examples still name `0.8.0`. Found with ISSUE-009.
- [ ] **Follow-up** (S4, L) — `docs/guide/updates-and-cli.md`: a
  `GITHUB_TOKEN` that is not valid UTF-8 falls through to `GH_TOKEN`
  (`src/update/client.rs:167`, `env::var` returns `Err`); the docs do
  not say so. Found in the `fix/docs-guide-sync` review.

## 2. Test tooling — `fix/test-tooling`

Done first among the code groups: a failing `cargo test` on the host blocks
the verification of every later group.

- [ ] **ISSUE-056** (S3, L) — two X11 tests fail with an ibus XIM; tests
  should clear `XMODIFIERS` or open no input context.
- [ ] **ISSUE-002** (S4, L) — `poc_cwe22_macos_library_is_blocked` must
  create `~/Library/Caches` itself.
- [ ] **ISSUE-012** (S4, L) — PTY test depends on the user's `$SHELL` and
  `HOME`; pin a shell in the test.
- [ ] **ISSUE-013** (S4, L) — `linux_xdg_open_dispatch_smoke` opens real
  apps on a desktop; gate it to the container.
- [ ] **ISSUE-035** (S3, L) — `manual-test.sh img shm` segment name is
  refused by Litty's allowlist; align script or allowlist.
- [ ] **ISSUE-011** (S4, L) — update plan steps need a build with a pinned
  key (plan/docs change; refusing is correct).
- [ ] **ISSUE-019 / REV-10** (S4, M) — `scripts/manual-test.sh` assumes
  macOS (`pbpaste`, …).
- [ ] **ISSUE-020** (S3, M) — `rounded_corners_fade_a_premultiplied_frame`
  is order-dependent in the container's full suite.

Deferred (not in this backlog until a real user report):
ISSUE-014, host heap corruption in `cargo test`.

## 3. Lifecycle, config, CLI — `fix/lifecycle-config`

- [ ] **ISSUE-023** (S3, L) — strip a UTF-8 BOM before parsing the config.
- [ ] **ISSUE-055** (S3, L, unconfirmed) — cap OSC 0/2 titles in the grid
  (`src/terminal/grid/perform.rs:392`).
- [ ] **ISSUE-017** (S3, L–M) — missing `SHELL` exits silently; fall back
  to the passwd shell or `/bin/sh` and report errors.
- [ ] **ISSUE-004 / REV-08** (S4, M) — `--version` / `--help`. Needs
  decision D1.
- [ ] **ISSUE-046** (S4, M) — SIGTERM / SIGHUP leave the reuse-registry
  entry; handle the signals and clean up on exit.
- [ ] **ISSUE-047** (S4, M) — Linux never withdraws a notification on
  focus; send `CloseNotification` (both platforms keep the same API).

## 4. Update and AI — `fix/update-ai-errors`

- [ ] **ISSUE-045** (S4, L) — AI transport error shows Rust `Debug`
  formatting (`curl exit Some(7)`).
- [ ] **ISSUE-052** (S4, L) — `+update` from a cached download still says
  "Downloading".
- [ ] **ISSUE-044** (S3, L–M) — `yo` retries a refused connection for
  4 minutes; do not retry non-transient curl errors.
- [ ] **ISSUE-018** (S4, M) — `+update` on a package install downloads,
  then fails; detect a package-managed binary before downloading.

## 5. Search, links, text, render — `fix/search-links-render`

- [ ] **ISSUE-038** (S3, L) — history popup drops Space
  (`handle_history_search_key`, `src/app/search_ui.rs:14`).
- [ ] **ISSUE-034** (S4, L) — kitty delete with an unknown selector.
  Needs decision D6.
- [ ] **ISSUE-042** (S4, L) — scanlines example shader makes the window
  translucent. Needs decision D7.
- [ ] **ISSUE-041** (S4, L–M) — plain-text links recognise only http(s);
  match the schemes OSC 8 accepts.
- [ ] **ISSUE-037** (S4, L–M) — bash title shows only the cwd
  (`src/shell/litty.bash`); add the running command like zsh.
- [ ] **ISSUE-039** (S3, M) — find box does not refresh while output
  streams; refresh on grid change, not only on keys.
- [ ] **ISSUE-040** (S4, M) — Ctrl+Click on a soft-wrapped path opens a
  prefix; join wrapped rows when extracting the path.
- [ ] **ISSUE-032** (S3, M) — `window-width` / `window-height` give 2 rows
  fewer on Linux (`src/app/window.rs:71` sizing vs decorations/padding).
- [ ] **ISSUE-036** (S4, M) — Unicode-placeholder images show 1 px seams
  between rows on Wayland (fractional scale).
- [ ] **ISSUE-033** (S4, H) — Devanagari and some combining marks are not
  shaped (complex script shaping).

## 6. Linux input and IME — `fix/linux-input`

- [ ] **ISSUE-029** (S4, L) — modified F1 encoding. Needs decision D5.
- [ ] **ISSUE-027** (S4, L–M) — `keybind = ctrl++=…` parses but never
  matches; match it or report a config error.
- [ ] **ISSUE-031** (S3, M, rest) — X11 Shift+Click does not extend a
  selection; Ctrl+Shift+Up not re-tested (Shift snap already fixed).
- [ ] **ISSUE-054** (S3, M, unconfirmed) — AltGr (maybe Caps Lock, Hyper)
  clears the selection and scrolls to the bottom; same cause as ISSUE-030.
- [ ] **ISSUE-053** (S3, M, unconfirmed) — Wayland without a keymap:
  F-keys and modifiers unnamed; name them like the X11 fix.
- [ ] **ISSUE-049** (S3, M–H) — with an IME active, find box and history
  popup input goes to the shell; route preedit/commit to the open popup.
- [ ] **ISSUE-050** (S2, H, unconfirmed) — keys stop after the XIM server
  is killed; reproduce needs a user decision (it rewrites dconf input
  sources).

## 7. Windows, tabs, dialogs — `fix/linux-windows-dialogs`

- [ ] **REV-09** (S4, L, unconfirmed) — multi-line paste dialog overstates
  the line count (one line plus newline).
- [ ] **REV-02** (S3, M) — Linux without zenity: multi-line paste is always
  cancelled; need a fallback.
- [ ] **ISSUE-025** (S3, M) — dropping a tab on another window's tab strip
  makes a new window instead of moving it there.
- [ ] **ISSUE-024** (S3, M–H) — tab tear-off unreliable on X11
  (straight-down drags never tear out).
- [ ] **ISSUE-048** (S3, H) — a confirm dialog freezes every window
  (`zenity … .status()` on the main thread); make dialogs async.
- [ ] **REV-01** (S3, H) — file drag-and-drop only on Wayland; X11 needs
  XDND.

## 8. Performance — `perf/<topic>`

- [ ] **ISSUE-010** (S4, M) — idle CPU time grows with cursor blink off
  (macOS; check focus state first).
- [ ] **ISSUE-001** (S3, M) — benchmark worst frame stall sometimes > 3 ms
  (re-run on an idle machine before treating as a bug).
- [ ] **ISSUE-003** (S3, M–H) — macOS reports no refresh rate, scroll at
  60 fps. Needs decision D8.
- [ ] **ISSUE-016** (S3, H) — native Wayland scroll misses every fifth
  frame on 60 Hz (frame pacing).
- [ ] **ISSUE-015** (S3, H) — Linux misses several `+bench-perf` /
  `+bench-stress` limits. Needs decision D8 for the limits.

## Already fixed

- [x] **ISSUE-021** (S1) — long title kills Litty on Wayland (`75fc9b0`).
- [x] **ISSUE-028** (S2) — F1–F12 send no bytes on X11 (`7983d3e`).
- [x] **ISSUE-030** (S2) — Copy leaves CLIPBOARD unchanged on X11
  (`7983d3e`).
- [x] **ISSUE-043** (S2) — X11 input starves while a pane floods output
  (`49324e7`).
