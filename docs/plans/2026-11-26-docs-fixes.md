# Docs fixes from the 2026-11-23 run — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: subagent-driven-strict or executing-plans-strict. Tasks are `### Task N: <name>`; steps use `- [ ]`.

**Goal:** Close the docs and test-tooling issues of the full test run
([issue note](../validation/2026-11-23-full-test-issues.md)): ISSUE-005,
006, 007, 008, 009, 012, 013, 019, 022, 035, 051 (and their REV-03..07,
REV-10 twins).
**Architecture:** no design change; docs follow the code, tests stop
depending on the developer's shell and desktop, the manual test script
gains Linux branches.
**Tech Stack:** Rust (tests only), Bash, Markdown.
**Branch:** `fix/docs-guide-sync` from `main` at `4c264c0`, one commit
per task (backlog group 1, `docs/plans/2026-11-26-full-test-fix-backlog.md`).
Companion plan for backlog group 2: `docs/plans/2026-11-26-test-tooling-fixes.md`;
its Scope Map lines marked "test-tooling plan" are executed there.

## Confirmed Requirements

1. Scope is docs + tooling only: 005–009, 012, 013, 019, 035, 051, plus
   022 (user, 2026-11-26).
2. ISSUE-022: the code default `#000000` is right; fix the docs (user).
3. ISSUE-006 / 051: keep `xdg-open` / `open -t`; fix the docs, no
   `$EDITOR` support (user).
4. ISSUE-004: `--version` / `--help` should print and exit 0 (user). Not in
   this batch's scope (it is runtime code); recorded as a follow-up.

## Decision Log

- 022, 006/051, 004 decided by the user in chat on 2026-11-26 → not
  posted: no ticket system; recorded in the issue note by Task 7.

## Scope Map

Invariant: every claim in the user guide matches `src/`, and every test
or manual-test step gives the same result whatever the developer's shell,
dotfiles, desktop or OS.

- `docs/guide/shortcuts.md` missing bindings → in (Task 1)
- `docs/guide/configuration.md:8`, `docs/guide/shortcuts.md:28` `$EDITOR`
  claims → in (Task 2)
- `docs/guide/configuration.md` Environment table, `background-blur` row;
  `docs/guide/updates-and-cli.md` token vars → in (Task 3)
- `docs/guide/keybinding-actions.md` missing actions → in (Task 4)
- `README.md:42`, `docs/guide/installation.md:17-42` package names → in (Task 5)
- `docs/guide/configuration.md:34` default and the config template comment
  `src/config.rs:974-975` → in (Task 6). `configuration.md:80` is an
  example config, not a default → out (unchanged, it is a valid example)
- `src/terminal/pty.rs` test `shell_exit_is_reported_and_reaped` → in
  (test-tooling plan Task 1); the other four `Pty::spawn_in` tests (`pty.rs:1183, 1194,
  1263, 1306`) → checked in test-tooling plan Task 1 under the same fresh `HOME`; changed
  only if they fail there
- `src/platform/process.rs` `linux_xdg_open_dispatch_smoke` → in (test-tooling plan Task 2)
- `docs/development/building.md:104-123` `0.8.0` packaging examples → out
  (developer doc, not named by ISSUE-009; follow-up)
- `scripts/manual-test.sh` macOS-only steps → in (test-tooling plan Task 3)
- `scripts/manual-test.sh:123` shm name → in (test-tooling plan Task 4); Litty's prefix
  allowlist `src/terminal/graphics/kitty.rs:407` → out (security
  allowlist kept; the script was wrong)
- ISSUE-004 CLI flags → out (follow-up: runtime change, next batch)
- ISSUE-035 note on group 24 "under" (z = −1 over red cells) → in (test-tooling plan Task
  4: fix the verdict text to the kitty rule Litty follows)

## Impact Map

- `shell_exit_is_reported_and_reaped` spawns `/bin/sh` instead of
  `$SHELL` → only this test; production `spawn_in` unchanged.
- Config template comment text (`src/config.rs:975`) → template written by
  `open_config` for new users; tests that assert template text, if any,
  updated in Task 6 (grep `121316` in `src/`).
- No other shared symbol changes.

## Existing Data & States

- Users with an existing `~/.litty/config` keep their file; the template
  change affects only newly created ones → Task 6 drift test pins the
  template comment to the code default.

## Global Constraints

- Rules: `CLAUDE.md` (project) and `~/.claude/CLAUDE.md` (global). No
  edits outside the files each task names.
- Docs keys and bindings are copied from `src/keymap.rs` / `src/config.rs`
  as they are on the branch, never from memory.
- Commits: only when the user asks. Each task ends with a "Ready to
  commit" line; the commit format is `type(scope): summary`, no
  `Co-authored-by`.
- Verification for any task touching `src/` (Task 6 here; Tasks 1, 2 of the test-tooling plan): `cargo check`,
  `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`,
  `cargo clippy --all-targets -- -D warnings`, `git diff --check`.
- Scripts: `bash -n scripts/manual-test.sh` (no shellcheck installed).

## Technical Defaults

- One branch per backlog group, one commit per issue (backlog rule; user
  chose the split on 2026-11-26).
- Install docs use a version-free name (`./litty_*_amd64.deb`,
  `./litty-*.x86_64.rpm`) and the latest-release link, so they do not go
  stale again.
- ISSUE-012: the test spawns `/bin/sh` through `Pty::spawn_program` — it
  checks exit reporting and reaping, not the user's shell.
- ISSUE-013: isolate `xdg-open` from the desktop by pointing
  `DBUS_SESSION_BUS_ADDRESS` at an unreachable socket and removing the
  desktop variables, so `xdg-open` falls back to `$BROWSER`.
- ISSUE-019: a `MOD` / `clip_read` / bell-sound helper chosen by `$OS`
  in `scripts/manual-test.sh`.
- ISSUE-035: rename the segment to `litty-<pid>`.

## Risks & Rollback

- Wrong key named in a docs table → every key is checked against
  `src/keymap.rs` in its task; revert the commit.
- Test-tooling Task 2 isolation does not stop `xdg-open` from using the desktop → the
  task reports it instead of weakening the test; no guard that silently
  passes.
- Manual-test script prompts wrong on one OS → `bash -n` plus a diff review
  of every changed prompt against `src/keymap.rs`; revert the commit.

### Task 1: List missing default shortcuts (ISSUE-005)

Type: docs · Files: `docs/guide/shortcuts.md`; read `src/keymap.rs:130-215`.

- [x] Confirm in `src/keymap.rs` the bindings: `Ctrl+PageUp` / `Ctrl+PageDown` (previous / next tab, `keymap.rs:209-210`, bound on **both** platforms: no `!mac` guard), Linux `Ctrl+Alt+D` (split down), macOS `Cmd+Ctrl+=` (equalize splits). Note the exact lines.
- [x] Add each to the matching row of `docs/guide/shortcuts.md` (tab cycling in both the macOS and Linux columns, split down, equalize at line 13) as an alternative binding in the existing table style.
- [x] RED: n/a — no executable behavior, checked by a diff review of each added key against the `src/keymap.rs` lines noted above.
- [x] Ready to commit: `docs/guide/shortcuts.md` — "docs(keymap): list tab, split and equalize shortcuts"

### Task 2: Describe how the config opens (ISSUE-006, ISSUE-051)

Type: docs · Files: `docs/guide/configuration.md:8`, `docs/guide/shortcuts.md:28`; read `src/config.rs` `open_config_in_editor` (around line 935).

- [x] Read `open_config_in_editor` and confirm the exact commands (`/usr/bin/open -t` on macOS, `xdg-open` on Linux).
- [x] Replace "your default `$EDITOR`" / "in `$EDITOR`" with: macOS opens the file in the default text editor (`open -t`); Linux opens it with the desktop's handler for text files (`xdg-open`). Say that `$EDITOR` is not used.
- [x] RED: n/a — no executable behavior, checked by `grep -n 'EDITOR' docs/guide/*.md README.md` returning no claim that `$EDITOR` is used.
- [x] Ready to commit: `docs/guide/configuration.md docs/guide/shortcuts.md` — "docs(config): say the config opens with open -t or xdg-open"

### Task 3: Document environment variables and the blur alias (ISSUE-007)

Type: docs · Files: `docs/guide/configuration.md` (Environment table at line 132, `background-blur` row at line 20), `docs/guide/updates-and-cli.md`; read `src/render/atlas/fonts.rs:39`, `src/config.rs:175` and `:363`, `src/update/client.rs:160-175`.

- [x] Read each source line and note what the variable does and its accepted values (`LITTY_FONT` font family; `LITTY_FONT_SIZE` size; how each relates to the config keys).
- [x] Add `LITTY_FONT` and `LITTY_FONT_SIZE` rows to the Environment table.
- [x] Add "alias: `background-blur-radius`" to the `background-blur` row.
- [x] Add a short section to `docs/guide/updates-and-cli.md`: `GITHUB_TOKEN` (else `GH_TOKEN`) is sent to the GitHub API by the update client, what for (rate limit) as the code shows, and that it is optional.
- [x] RED: n/a — no executable behavior, checked by a diff review against the source lines read above.
- [x] Ready to commit: `docs/guide/configuration.md docs/guide/updates-and-cli.md` — "docs(config): document font and token environment variables"

### Task 4: List the missing keybinding actions (ISSUE-008)

Type: docs · Files: `docs/guide/keybinding-actions.md:13-40`; read `src/keymap.rs:384-388`.

- [x] Add rows for `increase_font_size`, `decrease_font_size`, `reset_font_size`, `scroll_page_up`, `scroll_page_down` in the table's style, next to related actions.
- [x] Check that every other action name in `src/keymap.rs`'s action parser appears in the guide; add any other missing one and list it in the task report.
- [x] RED: n/a — no executable behavior, checked by comparing the parser's string arms with the guide's action names (script or grep, output quoted in the report).
- [x] Ready to commit: `docs/guide/keybinding-actions.md` — "docs(keymap): list font size and page scroll actions"

### Task 5: Use version-free package names (ISSUE-009)

Type: docs · Files: `README.md:42`, `docs/guide/installation.md:17-42`; read `.github/workflows/release.yml` around line 215.

- [x] Confirm the release asset names (`litty_${VERSION}_amd64.deb`, `litty-${VERSION}-1.x86_64.rpm`).
- [x] Point both docs at the latest release (`/releases/latest`) and use `./litty_*_amd64.deb` and `./litty-*.x86_64.rpm` in the commands and asset names.
- [x] RED: n/a — no executable behavior, checked by `grep -n '0\.8\.0' README.md docs/guide/installation.md` returning nothing.
- [x] Ready to commit: `README.md docs/guide/installation.md` — "docs(release): name packages without a fixed version"

### Task 6: State the real default background (ISSUE-022)

Type: docs · Files: `docs/guide/configuration.md:34`, `src/config.rs:974-975` (template comment); read `src/config.rs:182`.

- [x] Change the default in `configuration.md:34` to `#000000`.
- [x] Change the template comment `# background = #121316` to `# background = #000000` so the commented default matches the code.
- [x] `grep -rn '121316' src/` and update any test that asserts the template's background line; palette colour 0 (`src/config.rs:79`, `src/terminal/grid/colors.rs:51`) stays.
- [x] RED: add a unit test next to `template_is_valid_config` (`src/config.rs:1343`), `template_documents_default_background`: find the template line starting `# background = #` (the crash-course line `# background =` at `config.rs:964` has no value and must not match), strip `# `, parse it with `Config::parse`, and assert its background equals `Config::default().background`. Run `cargo test template_documents_default_background` before changing the comment; it fails (`0x1213_1600` vs `0x0000_0000`, colours are packed `0xRRGGBB00`). Quote it.
- [x] GREEN: after the comment change the test passes.
- [x] Run the `src/` verification list from Global Constraints; quote results.
- [x] Ready to commit: `docs/guide/configuration.md src/config.rs` — "docs(config): state the default background as #000000"

### Task 7: Verification & Docs

Type: docs · Files: `docs/validation/2026-11-23-full-test-issues.md`.

- [x] Run the full `src/` verification list from Global Constraints and `git diff --check`; quote commands and results.
- [x] Add `- Status: fixed on fix/docs-guide-sync (<commit>)` — deviated: written as `(uncommitted)`; replace with the hash at commit time. with the evidence to ISSUE-005, 006, 007, 008, 009, 022, 051, and mark REV-03..07 accordingly.
- [x] In `docs/plans/2026-11-26-full-test-fix-backlog.md` tick group 1 items ISSUE-009, 005, 008, 007, 022, 006+051 and decisions D1, D2, D3 (record each answer: D1 print and exit 0; D2 keep open -t / xdg-open, docs fixed; D3 `#000000`, docs fixed), and update the Progress row. — deviated: only D1–D3 ticked; the items stay unticked until merge (backlog rule, line 12), see checkpoint 2.
- [x] Record under ISSUE-004 the user's decision (print and exit 0) as the agreed fix, still open.
- [x] Final gate from `~/.claude/CLAUDE.md` (full diff read, self-review); the orchestrator's whole-branch `code-review-strict` review covers step 4. CI: the Task 6 test runs in `cargo test`.
- [x] Ready to commit: `docs/validation/2026-11-23-full-test-issues.md docs/plans/2026-11-26-docs-fixes.md docs/plans/2026-11-26-full-test-fix-backlog.md` — "docs(validation): record docs and tooling fixes"
