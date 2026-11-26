# Test Tooling Fixes (Backlog Group 2) Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: subagent-driven-strict or executing-plans-strict. Tasks are `### Task N: <name>`; steps use `- [ ]`.

**Goal:** make the test suites and manual-test tooling independent of the
host's input method, shell, `HOME` and desktop session (backlog group 2 of
[full-test-fix-backlog.md](2026-11-26-full-test-fix-backlog.md); issue
details in [the issue note](../validation/2026-11-23-full-test-issues.md)).
**Architecture:** test-only changes plus `scripts/manual-test.sh`,
`scripts/linux/run.sh` and the full test plan; no production behavior change.
**Tech Stack:** Rust 2024 (cargo), bash, Python 3 (inside the script).
**Branch:** `fix/test-tooling` from `main`, one commit per issue.

## Confirmed Requirements

- The user asked to do every backlog item that needs no decision
  (2026-11-26). The backlog fixes the approach per item: ISSUE-056 "clear
  `XMODIFIERS` or open no input context", ISSUE-002 "create
  `~/Library/Caches` itself", ISSUE-012 "pin a shell in the test",
  ISSUE-013 "gate it to the container", ISSUE-011 "plan/docs change;
  refusing is correct".
- ISSUE-020 is not in this plan: it has no root cause yet, so it goes to
  `bugfix-strict` first (planning-strict: no plan on a bug without a root
  cause).

## Decision Log

none: no business decision. The backlog left ISSUE-035 open ("align script or allowlist"); this plan picks the script as a technical default (see Technical Defaults) because the allowlist is a security control. Not posted: no ticket system; the PR description states it.

## Scope Map

Invariant: a test or manual-test step gives the same result whatever the
host's input method, login shell, dotfiles, `HOME` and desktop session.

| Path | Status |
| --- | --- |
| `x11::tests::keycode_zero_presses_are_not_keys`, `a_window_after_a_closed_one_still_looks_up_text` (the only tests that inject key events, `assert_keycode_zero_is_not_a_key`, `x11.rs:1862-1900`) | in (Task 1) |
| `x11::tests::new_window_has_an_input_context`, `a_reconnect_outside_filtering_selects_the_new_methods_events` | out: they hold under any method (assert a context exists / reselect); unchanged |
| `security_poc::poc_cwe22_macos_library_is_blocked` (`src/security_poc.rs:119-127`) | in (Task 2) |
| `security_poc::poc_cwe200_no_file_existence_oracle` | out: probes `$HOME/.ssh` and a missing path; both fail identically whether or not they exist, so a fresh `HOME` does not change it |
| PTY tests whose result depends on the shell reading input: `shell_exit_is_reported_and_reaped` (`pty.rs:1246`), `output_after_enter_is_not_held_back_waiting_for_the_prompt` (`pty.rs:1262`) | in (Task 3) |
| PTY tests that only spawn and drop (`dropping_pty_before_spawn_reader_reaps_the_shell`, `dropping_pty_never_blocks_and_reaps_the_shell`, `shell_spawn_closes_inherited_fds`), `tab::tests::render_buffers_allocate_only_when_growing` | out: their assertions hold before the shell reads anything; they keep `spawn_in` so the shell-spawn path stays covered |
| `process::tests::linux_xdg_open_dispatch_smoke` + `scripts/linux/run.sh:18` | in (Task 4) |
| `scripts/manual-test.sh` group 24 shm segment name (`:123`) | in (Task 5) |
| `scripts/manual-test.sh` macOS-only steps: clipboard (`:340`), bell sound (`:256`), `Cmd` wording (about 25 prompts) | in (Task 6) |
| `manual-test.sh` macOS-only groups without an `$OS` guard: g15 "macOS menu bar" (`:281`), g18 "Native macOS tabs" (`:300`) | in (Task 6: skip on Linux like g26) |
| `manual-test.sh` `ls -G /` in g17 (`:296`) and the shader group (`:313`) (GNU ls: no group, no colour) | in (Task 6) |
| `manual-test.sh` `:228` "closing last window keeps Dock icon" (macOS lifecycle; on Linux the last window's close quits Litty, `src/app/mod.rs:291`) | in (Task 6) |
| `manual-test.sh` "red button" (`:227`, `:277`) | in (Task 6: "the window's close button") |
| `manual-test.sh` macOS wording: Dock bounce (`:178`, `:221`), macOS Appearance (`:190`), Finder (`:216`) | in (Task 6: Linux wording) |
| `docs/plans/2026-11-23-full-test-plan.md` R-02 build step | in (Task 7) |
| ISSUE-020 renderer test order dependence | out (follow-up: `bugfix-strict` root cause first) |
| ISSUE-014 host heap corruption | out (backlog defers it until a real user report) |

## Impact Map

| Changed symbol | Caller / consumer | Before → after | Status |
| --- | --- | --- | --- |
| new `#[cfg(test)] XimContext::builtin_only` (`xim.rs`) | Task 1 test helper only | — → opens `@im=none`, never watches for a server | intended (Task 1 tests) |
| `XimContext::new` | `X11Window::create` (`x11.rs:792`) | unchanged | unchanged: not edited |
| `scripts/linux/run.sh` env for the smoke test | local container run (`scripts/linux/test.sh`) | runs smoke → runs smoke (gate var set) | intended (Task 4 run) |

## Existing Data & States

none: no persisted data or user state; `~/Library/Caches` created by
Task 2 is a standard macOS directory that already exists for real users.

## Global Constraints

- Project rules: `CLAUDE.md` (verification list, commit format, scope).
  Global rules: `~/.claude/CLAUDE.md`.
- Test-only Rust code stays under `#[cfg(test)]`; no production behavior
  changes. Never change the shm name allowlist (`kitty.rs:407`, a security
  control).
- Never run `linux_xdg_open_dispatch_smoke` with the gate set on the real
  desktop (it opens real apps); GUI/e2e work only in `hs-gnome` or the
  container (`scripts/linux/test.sh`).
- Verification per task: `cargo test <name>`; `cargo clippy --all-targets -- -D warnings`.
  Shell scripts: `bash -n <script>`.
- Commits: only when the user asks. Each task ends at a `Ready to commit`
  checkpoint; one commit per issue, `fix(test)`/`test(<scope>)`/`docs(validation)` per Conventional Commits.

## Technical Defaults

- ISSUE-056: open Xlib's built-in method (`@im=none`) in a test-only
  constructor instead of mutating `XMODIFIERS` — `set_var` races other
  test threads (unsafe in Rust 2024), and the second test is about the
  built-in method anyway.
- ISSUE-012: pin `/bin/sh` through the existing `Pty::spawn_program`
  (same fork path) instead of adding a shell parameter to `spawn_in`.
- ISSUE-013: gate with env var `LITTY_XDG_OPEN_SMOKE=1`, set by
  `scripts/linux/run.sh`; without it the test prints a skip line and
  returns (the pattern `hidden_test_window` uses).
- ISSUE-035: rename the script's segment to `/litty-<pid>`; the allowlist
  stays (security control; kitty's own clients use `kitty-` names).
- ISSUE-019: on Linux, read the clipboard with `wl-paste --no-newline`
  under Wayland, else `xclip -o -selection clipboard`; bell sound
  `/usr/share/sounds/freedesktop/stereo/bell.oga` (played by `paplay`);
  print one note at start mapping `Cmd` to the Linux keys (Ctrl+Shift for
  terminal actions, Option = Alt, see `docs/guide/shortcuts.md`) instead of rewriting
  about 25 prompts; macOS-only groups skip on Linux; Dock/Finder/Appearance
  wording picked per OS (attention request / file manager / desktop
  light-dark style).

## Risks & Rollback

- Built-in-only context hides a real XIM bug from the two tests → the
  ibus path stays covered by `new_window_has_an_input_context` and the
  e2e IME cases; revert the commit to restore.
- Gate var forgotten in another runner → the smoke silently skips; the
  skip line is printed with `--nocapture`, and `run.sh` is the only runner
  (`grep -rn linux_xdg_open_dispatch_smoke scripts`).
- All changes are test/script/docs; revert per commit.

### Task 1: Run the X11 key-injection tests on Xlib's built-in method (ISSUE-056)

Type: feature (test behavior) · Test level: unit (needs `DISPLAY`) ·
Files: `src/platform/linux/native/xim.rs` (next to `XimContext::new`,
~`:441`), `src/platform/linux/native/x11.rs` (tests module, `:1370-1392`,
`:1862-1877`).
Interfaces: Produces `#[cfg(test)] pub unsafe fn builtin_only(display: *mut Display, window: c_ulong) -> XimContext`.

- [x] RED: on this host (`XMODIFIERS=@im=ibus`, ibus running) run
  `cargo test x11::tests::keycode_zero_presses_are_not_keys` and
  `cargo test x11::tests::a_window_after_a_closed_one_still_looks_up_text`;
  expect each to fail with `Ime(Preedit("a"))`. Quote the output. If ibus is
  not running (`pgrep -x ibus-daemon`), do not start one in the user's
  session: report RED as not reproduced and stop the task.
- [x] In `xim.rs` add, under `#[cfg(test)]`, `builtin_only`: build the
  same `Self` as `new`, then under `XLIB_IM.lock()` and `init_locale(lib)`
  call `ctx.open(lib, &watch, c"@im=none")` only; never call
  `start_waiting` (so no server can reconnect it). Doc comment: tests that
  inject keys must not depend on the host's input method; add a `# Safety`
  section matching `new` (`display` and `window` stay live until drop).
- [x] In `x11.rs` tests add `fn hidden_test_window_builtin_im(title: &'static str) -> Option<X11Window>`:
  call `hidden_test_window(title)?`, then
  `let mut xim = unsafe { XimContext::builtin_only(win.0.display, win.0.window) };`,
  `unsafe { (win.0.lib.XSelectInput)(win.0.display, win.0.window, BASE_EVENT_MASK | xim.filter_events()) };`,
  `*win.0.xim.lock() = Some(xim);` (drops the host-method context).
- [x] Switch both tests to `hidden_test_window_builtin_im`, including the
  **first** window (`first`) in `a_window_after_a_closed_one_still_looks_up_text`:
  its premise is that the first window's built-in compose tree is freed and
  the second's lands on reused heap. Update both doc comments to say they
  pin Xlib's built-in method.
- [x] GREEN: rerun the RED command with `XMODIFIERS=@im=ibus`: both pass.
  Also `XMODIFIERS= cargo test x11::tests`: all pass.
- [x] `cargo clippy --all-targets -- -D warnings`: no warnings.
- [x] Ready to commit: `src/platform/linux/native/xim.rs src/platform/linux/native/x11.rs` — "test(linux): pin xlib's built-in im in x11 key tests"

### Task 2: Create `~/Library/Caches` in the macOS t=f test (ISSUE-002)

Type: feature (test behavior) · Test level: unit (macOS only) ·
Files: `src/security_poc.rs:119-127`.

- [x] RED: n/a on this Linux host (`#[cfg(target_os = "macos")]`); the
  reported failure is `HOME=$(mktemp -d) cargo test security_poc` on macOS
  (`security_poc.rs:124`, NotFound). Report that RED/GREEN were not run here.
- [x] Before `std::fs::write`, add `std::fs::create_dir_all(&dir).unwrap();`
  with a comment that a fresh `HOME` (new CI user) has no `Library/Caches`.
- [x] `cargo check --tests` and `cargo clippy --all-targets -- -D warnings`
  on Linux (the test is cfg'd out; this checks nothing else changed). If
  `rustup target list --installed` shows an Apple target, also run
  `cargo check --tests --target <it>`; if it fails in a build script for
  lack of the Apple SDK, quote the first error and report it as not run.
  Note for the report: the CI macOS job only proves it compiles (the
  runner's `HOME` already has `Library/Caches`); the fix is verified only by
  `HOME=$(mktemp -d) cargo test security_poc` on a Mac.
- [x] Ready to commit: `src/security_poc.rs` — "test(security): create ~/Library/Caches in the t=f poc"

### Task 3: Pin `/bin/sh` in PTY tests that type into the shell (ISSUE-012)

Type: feature (test behavior) · Test level: integration (real PTY) ·
Files: `src/terminal/pty.rs` tests (`:1246-1259`, `:1261-1300`).
Interfaces: Consumes `Pty::spawn_program(program: &str, args: &[&str], env: &[(&str, &str)], (cols, rows): (u16, u16), (cell_w, cell_h): (u32, u32)) -> io::Result<Pty>` (`pty.rs:328`).

- [x] RED: `RUSTUP_HOME=~/.rustup CARGO_HOME=~/.cargo HOME=$(mktemp -d) SHELL=/usr/bin/zsh cargo test shell_exit_is_reported_and_reaped`
  (rustup's `cargo` proxy needs `RUSTUP_HOME`/`CARGO_HOME` once `HOME` changes)
  (zsh is installed here: `command -v zsh`); expect `on_exit fires: Timeout`
  at `pty.rs:1255`. Quote it. Also run the same for
  `output_after_enter_is_not_held_back_waiting_for_the_prompt` and quote
  the result (expected to fail too with "prompt never arrived": the
  newuser menu eats the `p` of `printf`).
- [x] Add in the tests module
  `fn spawn_test_shell() -> Pty { Pty::spawn_program("/bin/sh", &["sh"], &[], (80, 24), (8, 16)).expect("spawn /bin/sh") }`
  (check first: `[ -z "$ENV" ] && echo unset`) with a doc comment: non-login POSIX sh reads only `$ENV` (unset in the
  test environment), so the result does not depend on `$SHELL` or `HOME`
  (ISSUE-012).
- [x] Use it in `shell_exit_is_reported_and_reaped` and
  `output_after_enter_is_not_held_back_waiting_for_the_prompt` in place of
  `Pty::spawn_in(80, 24, 8, 16, None, None).expect("spawn pty")`.
- [x] GREEN: rerun both RED commands: pass. Then `cargo test pty::`: all pass.
- [x] `cargo clippy --all-targets -- -D warnings`: no warnings.
- [x] Ready to commit: `src/terminal/pty.rs` — "test(pty): pin /bin/sh in tests that type into the shell"

### Task 4: Gate the xdg-open smoke test to the container (ISSUE-013)

Type: feature (test behavior) · Test level: integration (container) ·
Files: `src/platform/process.rs:239-280`, `scripts/linux/run.sh:18`.

- [x] RED: n/a — reproduced in the issue note (3 of 3 on GNOME). Not rerun:
  the URL goes through `gio open` to the default browser, whose
  single-instance socket lives in the shared `HOME`, so even `hs-gnome`
  could open a tab on the real desktop.
- [x] At the top of `linux_xdg_open_dispatch_smoke`, before any file is
  written: if `std::env::var_os("LITTY_XDG_OPEN_SMOKE").as_deref() != Some("1".as_ref())`,
  `eprintln!("LITTY_XDG_OPEN_SMOKE unset; skipping (it opens real apps on a desktop session)");`
  and return. Update the `#[ignore]` reason and doc comment to name the variable.
- [x] In `scripts/linux/run.sh:18` prefix the command with `LITTY_XDG_OPEN_SMOKE=1`.
- [x] Check the skip on the host: `cargo test --locked linux_xdg_open_dispatch_smoke -- --ignored --nocapture`
  (gate unset): passes and prints the skip line, opens nothing.
- [x] Check the run in the container: `bash scripts/linux/test.sh`; the
  smoke step passes. ISSUE-020 may stop `run.sh` before that step; if so,
  run the smoke step alone in the image the way the issue note did and
  quote it, and report that the full `run.sh` did not reach it.
- [x] `bash -n scripts/linux/run.sh`; `cargo clippy --all-targets -- -D warnings`.
- [x] Ready to commit: `src/platform/process.rs scripts/linux/run.sh` — "test(linux): run the xdg-open smoke only when asked"

### Task 5: Name the manual-test shm segment `litty-` (ISSUE-035)

Type: feature (script behavior) · Test level: e2e (manual script) ·
Files: `scripts/manual-test.sh:120-127`.

- [x] RED: `grep -n 'name="lpl' scripts/manual-test.sh` shows the name
  the allowlist (`kitty.rs:407-410`) refuses.
- [x] Change `name="lpl%d"` to `name="litty-%d"`; Python's
  `SharedMemory` adds the leading `/` on POSIX, so the segment is
  `/litty-<pid>`. Add a short comment pointing at the allowlist.
- [x] `bash -n scripts/manual-test.sh`.
- [x] E2E in `hs-gnome` (never the real desktop): build `cargo build --release`,
  start Litty there with a private `XDG_RUNTIME_DIR`, run
  `bash scripts/manual-test.sh 24` in it, take a screenshot at the shm item
  with `hs-gnome run python3 -m drive shot`; expect the image.
  If `hs-gnome` is unavailable, report the e2e as not run.
- [x] Ready to commit: `scripts/manual-test.sh` — "fix(test): name the manual-test shm segment litty-"

### Task 6: Make `manual-test.sh` usable on Linux (ISSUE-019, REV-10)

Type: feature (script behavior) · Test level: e2e (manual script) ·
Files: `scripts/manual-test.sh` (helpers `:20-60`, `:256`, `:340`).

- [x] RED: `grep -n 'pbpaste\|/System/Library' scripts/manual-test.sh` lists `:256` and `:340`.
- [x] Add helper `clip()`: on `$OS = Darwin` → `pbpaste`; else if
  `$WAYLAND_DISPLAY` set → `wl-paste --no-newline`; else
  `xclip -o -selection clipboard`. Use `"$(clip)"` at `:340`.
- [x] Add `BELL_SOUND`: Darwin → `/System/Library/Sounds/Glass.aiff`,
  else `/usr/share/sounds/freedesktop/stereo/bell.oga`; use it at `:256`
  and say "quiet bell sound" in the verdict.
- [x] Guard g15 (`:281`) and g18 (`:300`) with
  `[ "$OS" = Darwin ] || { note "macOS only"; log skip "all" "not on macOS"; return; }`
  as their first line (mirrors g26).
- [x] `:296` and `:313`: use `$LS_COLOR` (`ls -G` on Darwin, `ls --color=always` elsewhere) in place of `ls -G`.
- [x] `:228`: on Darwin keep the verdict; on Linux use verdict "closing the last window quits Litty".
- [x] `:227`, `:277`: "red button" → "the window's close button" (both OSes).
- [x] Per-OS wording: `:178` and `:221` Dock bounce → on Linux "window
  requests attention (taskbar/dock highlight)" / "no attention request";
  `:190` "macOS Appearance" → on Linux "desktop light/dark style";
  `:216` "Finder file" → on Linux "file from the file manager". Use one
  variable per phrase set after `OS=`.
- [x] After `OS=$(uname -s)`, on Linux print one `note`: prompts say
  `Cmd`; on Linux use Ctrl+Shift for terminal actions (copy, paste, tab,
  split, close), Option = Alt, and see `docs/guide/shortcuts.md` for the rest.
- [x] `bash -n scripts/manual-test.sh`; `grep -n 'pbpaste\|Glass\|Finder\|Dock\|ls -G' scripts/manual-test.sh`
  shows them only inside Darwin branches.
- [x] E2E in `hs-gnome`: run group 23 (`bash scripts/manual-test.sh 23`)
  inside Litty on Wayland (private `XDG_RUNTIME_DIR` for Litty, input via
  `hs-gnome run python3 -m drive …`), select and copy as prompted; the
  clipboard item auto-passes. The bell item cannot be heard here (`paplay`
  not on PATH): report it as not verified. Report if not runnable.
- [x] Ready to commit: `scripts/manual-test.sh` — "fix(test): read clipboard and bell sound per os in manual tests"

### Task 7: Say to build the update cases with the pinned key (ISSUE-011)

Type: docs · Files: `docs/plans/2026-11-23-full-test-plan.md:501-503`.

- [x] RED: n/a — no executable behavior, checked by a diff review: R-02's
  step names `LITTY_MINISIGN_PUBKEY="$(tail -n 1 resources/minisign.pub)" cargo build --release`.
- [x] In R-02, replace "release build" with that command and add "(a build
  without the pinned key refuses to update, by design; ISSUE-011)".
- [x] Ready to commit: `docs/plans/2026-11-23-full-test-plan.md` — "docs(validation): build update cases with the pinned key"

### Task 8: Verification & Docs

- [x] Final gate (global `CLAUDE.md` 1–5): read `git diff main --stat` and
  the full diff incl. untracked files; no debug output, TODOs, secrets.
- [x] Run and quote: `cargo check`; `cargo check --target x86_64-unknown-linux-gnu`;
  `cargo test` (outside sandbox, with the host's `XMODIFIERS=@im=ibus`);
  `cargo clippy --all-targets -- -D warnings`; `git diff --check`.
  Known: ISSUE-020 can fail only in the container; ISSUE-014 may crash the
  host suite rarely — rerun once and report, never hide.
- [x] `RUSTUP_HOME=~/.rustup CARGO_HOME=~/.cargo HOME=$(mktemp -d) SHELL=/usr/bin/zsh cargo test pty::` passes (ISSUE-012 e2e).
- [x] `bash scripts/linux/test.sh` result quoted (Task 4).
- [x] Issue note: add `- Status: fixed on fix/test-tooling (\`<hash>\`), <evidence>`
  under ISSUE-056, 002 (macOS run pending), 012, 013, 035, 019 (and REV-10 row), 011;
  while nothing is committed write `fixed on fix/test-tooling (uncommitted)`
  and replace it with the hash at commit time.
- [x] Backlog: tick the group 2 items whose fix is verified; ISSUE-002 stays
  open until `HOME=$(mktemp -d) cargo test security_poc` passes on a Mac.
  Progress table Done count.
- [x] README/ROADMAP: no change (no user-visible behavior, config key or keybinding).
- [x] `code-review-strict` on the branch; address or report findings.
- [x] Report: commands and results, what was not run (macOS for ISSUE-002;
  any `hs-gnome` e2e skipped; bell sound), and that CI (`ci.yml`) runs only
  `cargo test`, never `run.sh`, so ISSUE-013 is verified only by the local
  `scripts/linux/test.sh` run.
- [x] Ready to commit: `docs/validation/2026-11-23-full-test-issues.md docs/plans/2026-11-26-full-test-fix-backlog.md docs/plans/2026-11-26-test-tooling-fixes.md` — "docs(validation): record test tooling fix results"
