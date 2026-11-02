# Split `main.rs` God Object — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Break the 4604-line `src/main.rs` (one 50+ field `State`, ~90 free functions over `&mut State`, a 501-line `handle_window_event`) into focused modules under `src/app/` and `src/platform/`, with zero behavior change.

**Architecture:** Phase 1 of a 4-phase structural refactor (phases 2–4 are outlined at the end and get their own plans). We first extract the leaf code with no `State` dependency (`platform/`, pure `paste` and `cli` modules), then move the whole app core into `src/app/mod.rs` in one step, then peel cohesive groups out of it one commit at a time, then split the giant event handlers into per-event methods, and finally group `State` fields into sub-structs. Every commit compiles and passes the full test suite on both targets.

**Tech Stack:** Rust (edition 2024), winit, wgpu, parking_lot; cargo targets `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`.

## Global Constraints

- Pure move / rename refactor: no behavior, timing, or allocation change. If a task tempts you to "fix" something, write it in `ROADMAP.md` instead.
- Hot paths (PTY parsing, scrolling, render loop) must not gain heap allocations, `dyn` dispatch, or extra locking.
- Never hold the grid `Mutex` across blocking I/O — do not reorder any lock / I/O sequence while moving code.
- Every platform-specific item keeps its `#[cfg(...)]` attributes verbatim, including `#[cfg_attr(..., allow(dead_code))]`.
- Moved items keep their doc comments (`///`) and attributes (`#[...]`) that sit directly above them.
- Visibility: items moved into `src/app/<x>.rs` become `pub(super)`; `State` fields become `pub(super)`. Items used outside `app` (only `show_notification`, `spawn_detached`) become `pub(crate)` in `platform`.
- Verification before every commit (from `CLAUDE.md`):
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside sandbox; PTY tests need `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit format: `refactor(<scope>): <summary>`, lowercase, imperative, ≤ 72 chars. Stage files explicitly; never `git add -A`.
- Test count must stay equal to the Task 0 baseline after every task (tests move, never disappear), except where a task adds new tests.

## Target File Structure

```
src/main.rs                 mod decls, allocator, main(): parse CLI → dispatch → app::run()   (~120 lines)
src/cli.rs                  pure: argv → Command enum (+ unit tests)
src/paste.rs                pure: shell_quote, paste_payload, history_insert_payload (+ tests)
src/platform/mod.rs         pub(crate) mod clipboard; pub(crate) mod process;
src/platform/process.rs     spawn_detached, show_notification, open_target, linux_open_command,
                            safe_path_for_linux_open, system_bin, play_sound (+ tests)
src/platform/clipboard.rs   copy_to_clipboard, paste_from_clipboard, read_clipboard_* (+ tests)
src/app/mod.rs              App, ApplicationHandler impl, App window bookkeeping, run()
src/app/state.rs            State (+ PointerState, ImeState, WindowRequests, InspectorState)
src/app/window.rs           create_window, spawn_pane, render_active, sync_size, zoom, reload_config,
                            appearance, cursor blink, tab titles
src/app/window_event.rs     App::handle_window_event + one method per WindowEvent arm
src/app/user_event.rs       one method per AppEvent arm
src/app/terminal_io.rs      write_pty, write_pane_input, handle_terminal_requests, pointer_icon
src/app/ai_bridge.rs        ai_display, handle_ai_requests
src/app/pointer.rs          scrollbar drag, mouse reporting, hovered links, alt-screen wheel
src/app/ime.rs              handle_ime
src/app/tabs.rs             tab / split lifecycle and navigation
src/app/keyboard.rs         handle_key, key release, Kitty / legacy encoding, describe_key
src/app/inspector.rs        inspector_lines
src/app/search_ui.rs        find box + history search key handling
src/app/actions.rs          run_action, run_tab_action
src/app/latency_benchmarks.rs   #[cfg(test)] benchmark moved from main.rs
```

Names avoid clashing with existing crate modules (`crate::mouse`, `crate::search`), hence `pointer.rs` and `search_ui.rs`.

## How to Move an Item (used by Tasks 1–3 and 5–11)

Line numbers below are from commit `28c60f3` and **shift after every move**. Always re-locate an item before cutting it:

```bash
grep -nE '^(pub(\(crate\))? )?(fn|static|const|struct|impl)[ <]' src/main.rs
```

An item spans from its first preceding `///` / `#[` line through its closing `}` at column 0 (or `;` for `static`/`const`). Cut it with your editor (not `sed -i`, which differs between BSD and GNU). After moving a group:

1. Prepend the new file with a `//!` module doc line and, temporarily, a copy of the `use` block from the top of `src/main.rs` plus `use crate::{AppEvent, Waker, Pane, Tab, HoveredPaneLink, next_id};`.
2. Add `pub(super)` (or `pub(crate)`) to each moved item, then fix call sites with `use super::<file>::{...};` imports.
3. Run `cargo clippy --all-targets -- -D warnings` and delete every `use` it reports as unused. Repeat until clean.

**Test placement rule:** `mod paste_tests` in `src/main.rs` mixes tests for clipboard, process and paste helpers. Each test moves to the `#[cfg(test)] mod tests` of the module that owns the function(s) it calls (look at its `super::xxx` paths and `use super::{...}` lines). Keep each test's `#[cfg]`, `#[ignore]` and doc comments. List them first so none is lost:

```bash
awk '/^mod paste_tests/,/^}/' src/main.rs | grep -nE 'fn [a-z_0-9]+\(|super::'
```

---

### Task 0: Prepare branch and record baseline

**Files:** none modified.

- [ ] **Step 1: Make sure the working tree is clean**

Other in-flight work (at the time of writing: `feat/ai-live-validation`, with uncommitted edits to `src/main.rs` and `scripts/linux/run.sh`) must be committed and merged to `main` first. This refactor moves nearly every line of `src/main.rs`, so any concurrent edit to that file becomes a painful conflict; pause other `main.rs` work until Task 11 lands.

Run: `git status --short`
Expected: no output.

- [ ] **Step 2: Create the branch from the up-to-date base**

```bash
git switch main
git switch -c refactor/split-main
```

- [ ] **Step 3: Record the baseline**

```bash
cargo test 2>&1 | grep -E '^test result' > "$TMPDIR/baseline-tests.txt"
cat "$TMPDIR/baseline-tests.txt"
wc -l src/main.rs
```

Expected: every line shows `0 failed`; note the total `passed` count (call it `N`). `src/main.rs` is 4604 lines.

- [ ] **Step 4: Confirm a clean clippy baseline on both targets**

```bash
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu
```

Expected: both succeed. If the baseline is not clean, stop and fix that in a separate commit first; otherwise later tasks cannot tell their own warnings apart.

---

### Task 0.5: Build the safety net and record baseline evidence

**Why:** `cargo test` alone cannot prove Tasks 5–12 preserve behavior. Of the 13 tests in `src/main.rs`, about 10 cover leaf helpers (clipboard, paste, `xdg-open`, `spawn_detached`); nothing exercises `State`, `handle_window_event`, `user_event`, `about_to_wait`, or keyboard / pointer / IME / tab / search dispatch (about 3,900 lines). Compile success proves types, not behavior. This task assembles four independent kinds of evidence from what the repo already has plus two small scripts, and records their baseline on `main` before any code moves.

**What the existing tooling covers (read before relying on it):**

| Tool | Exercises | Does not cover |
|---|---|---|
| `cargo test` (309 tests, 5 ignored) + `tests/yo_shells.rs` | Pure modules (`grid`, `keymap`, `history`, `config`, `split`, `search`), leaf helpers, `yo` interception in 4 real shells | `State` and the event loop |
| `litty +bench-ram` | The real `State` through `user_event`: `OpenWindow`, `OpenTabInLastWindow`, `CloseLastWindow`, `CloseExtraTabs`; RAM budget | Input handling. Budgets are absolute (85 / 95 / 145 MB, leak <= 10 MB), not before/after, and the leak check is known flaky (`ROADMAP.md`, 2026-11-01) |
| `litty +bench-perf` | PTY round trip, file ingest, Kitty / Sixel ingest, 20k-line render stress, frame stalls | Input handling |
| `litty +bench-features` | fzf search, regex find, split engine, paste security — all on pure modules | `State` wiring of those features |
| `scripts/manual-test.sh` (30 groups) | Real-app behavior; some items auto-checked via terminal replies and key presses | **IME and Cmd+F search have no group** (Step 4 adds them) |
| `src/test_alloc.rs` | Per-thread allocation counter for tests | Not used on `State` paths |

**Files:**
- Create: `scripts/refactor-census.sh`, `scripts/refactor-moved.sh` (both removed in Task 13)

- [ ] **Step 1: Create the token census script**

`scripts/refactor-census.sh` counts tokens that a pure move must leave unchanged: lock acquisitions, `unsafe`, `#[cfg]`, thread spawns, blocking I/O, allocation sites, `#[test]`, and every `WindowEvent::` / `AppEvent::` / `BenchCommand::` / `Action::` / `NamedKey::` path (so a dropped `match` arm shows up by name).

```bash
#!/usr/bin/env bash
# Token census of src/**/*.rs. A pure move keeps every count identical.
# Usage: scripts/refactor-census.sh > census.txt
set -eo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
files=$(find src -name '*.rs' | sort)
count() { # label, ERE
    printf '%-18s %s\n' "$1" "$(cat $files | grep -oE "$2" | wc -l | tr -d ' ')"
}
count lock '\.lock\(\)'
count unsafe '\bunsafe\b'
count cfg '#\[cfg(_attr)?\('
count thread_spawn 'thread::(spawn|Builder)|\.spawn\('
count blocking_io 'std::fs::|File::(open|create)|\.write_all\(|\.read_to_|Command::new|\.output\(\)|\.wait\(\)'
count alloc 'Vec::new|vec!\[|Vec::with_capacity|Box::new|String::new|String::from|format!\(|\.to_string\(\)|\.to_owned\(\)|\.to_vec\(\)|\.clone\(\)|\.collect\(\)'
count test_attr '#\[test\]'
cat $files | grep -oE '\b(WindowEvent|AppEvent|BenchCommand|ElementState|MouseButton|MouseScrollDelta|Ime|NamedKey|Action)::[A-Za-z0-9_]+' | sort | uniq -c | awk '{printf "arm %-40s %s\n", $2, $1}'
```

Make it executable: `chmod +x scripts/refactor-census.sh`.

- [ ] **Step 2: Create the move-residual script**

`scripts/refactor-moved.sh` diffs a commit and cancels out removed and added lines after trimming indentation and visibility keywords (`pub`, `pub(crate)`, `pub(super)`), ignoring `use` / `mod` / comment lines. A pure move prints nothing; anything printed is a real change that must be explained in the commit body.

```bash
#!/usr/bin/env bash
# Residual of a refactor commit: lines removed and added that do NOT cancel out
# after trimming indentation and visibility keywords. A pure move prints nothing.
# Usage: scripts/refactor-moved.sh [<base>] [<head>]   (default: HEAD~1 HEAD)
set -eo pipefail
base=${1:-HEAD~1}; head=${2:-HEAD}
norm() {
    git diff -U0 --no-color "$base" "$head" -- src |
        grep -E '^[-+]' | grep -vE '^(\+\+\+|---)' |
        sed -E 's/^([-+])[[:space:]]+/\1/; s/^([-+])(pub\((crate|super|in [a-z_:]+)\)|pub) /\1/' |
        grep -vE '^[-+](use |mod |//)' | grep -vE '^[-+]\}?$'
}
{ norm | grep '^-' | sed 's/^-//' | sort | sed 's/^/R /'
  norm | grep '^+' | sed 's/^+//' | sort | sed 's/^/A /'; } |
    awk '{k=substr($0,3); if ($1=="R") r[k]++; else a[k]++}
         END {for (k in r) if (r[k]>a[k]) printf "only-removed x%d: %s\n", r[k]-a[k], k
              for (k in a) if (a[k]>r[k]) printf "only-added   x%d: %s\n", a[k]-r[k], k}' | sort
```

Make it executable: `chmod +x scripts/refactor-moved.sh`. Sanity check that it detects real changes: `scripts/refactor-moved.sh 865f4a0~1 865f4a0` must list the added `ai.busy` / `ai.submitted` lines of that fix.

- [ ] **Step 3: Record the automated baseline on `main`**

Run on a quiet machine (no build or test running) with a GUI session, because `+bench-ram` opens real windows. `scripts/release-test-report.md` is git-ignored; `scripts/manual-test-results.md` is tracked, so the manual runs below redirect their log with `LITTY_TEST_LOG`.

```bash
mkdir -p "$TMPDIR/baseline"
scripts/refactor-census.sh > "$TMPDIR/baseline/census.txt"
wc -l src/main.rs | tee "$TMPDIR/baseline/main-lines.txt"
cargo build --release
for i in 1 2 3; do
  scripts/pre-release-check.sh 2>&1 | tee "$TMPDIR/baseline/pre-release-$i.txt"
  cp /tmp/litty_ram_bench.json "$TMPDIR/baseline/ram-$i.json"
done
```

Expected: `cargo test` inside the script passes every run. The RAM leak check is known flaky, so keep all three runs and treat the **median** footprint per phase (1w/1t, 1w/3t, 2w/6t, after cleanup) as the baseline; a single failing leak check at baseline is recorded, not fixed here. If any other check fails at baseline, stop: the net has no stable reference.

- [ ] **Step 4: Record the manual baseline on `main`**

Run these groups inside a Litty window built from `main`, logging outside the repo:

```bash
export LITTY_TEST_LOG="$TMPDIR/baseline/manual.md"
bash scripts/manual-test.sh 2 3 4 6 7 8 11 12 15 16 17 18 19 29 30
```

Coverage map (which Task each group guards):

| Group | Guards |
|---|---|
| 2, 16, 27 (Linux only), 29 | `app::keyboard` (Task 10): modifyOtherKeys, Caps Lock, Kitty keys in the real app |
| 4, 7, 11, 30 | `app::pointer` (Task 8): pixel mouse, drag and drop, scrollbar drag, OSC 8 hover |
| 3, 6, 12 | `app::terminal_io` (Task 7): notifications, progress, bell, shell integration |
| 8, 18 | `app::window`, `app::tabs` (Tasks 6, 9): multiple windows, native tabs |
| 15, 17 | `app::actions`, `app::inspector` (Task 10): menu bar, terminal inspector |
| 19 (macOS) / 28 (Linux) | Quick Terminal toggle (`app::window`) |

Then add the items no group covers, as a short checklist saved to `$TMPDIR/baseline/manual-extra.md` with a pass / fail per line:
1. Cmd+F opens search, typing highlights matches, Enter / Shift+Enter move between them, Esc closes it (`app::search_ui`).
2. Vietnamese Telex typing (`vieejt` → `việt`) shows preedit text and commits correctly (`app::ime`). Repeat with a second IME if available.
3. Ctrl+R opens fuzzy history and the vim redo in alt screen still works (`app::keyboard`).
4. Cmd+C, Cmd+V (Ctrl+Shift+C / V on Linux); paste of text containing `\x1b[201~` is stripped.
5. Cmd+T, Cmd+W, Cmd+1..9, Cmd+D / Cmd+Shift+D splits, pane focus and zoom.
6. `yo hello` renders the AI status line and the prompt returns (`app::ai_bridge`; `bash scripts/test-ai-live.sh` only if the user has a key configured).
7. Config reload applies a changed `theme`.
8. Idle check: all windows closed, `footprint` < 50 MB; one window idle, `top` shows 0.0% CPU.

- [ ] **Step 5: Commit the two scripts**

```bash
git add scripts/refactor-census.sh scripts/refactor-moved.sh
git commit -m "chore(refactor): add token census and move-residual scripts"
```

- [ ] **Step 6: Define the checkpoints (used by later tasks)**

| Checkpoint | After | Required evidence |
|---|---|---|
| **A** | Task 4 | Full verification suite; test count == baseline + new `cli` tests; `scripts/refactor-moved.sh` per commit shows only explained lines; re-record `census.txt` as `census-A.txt` (Task 4 adds new code, so later comparisons use this one) |
| **B** | Task 10 | Everything in A, plus: `scripts/refactor-census.sh` diff against `census-A.txt` is **empty**; for every Task 5–10 commit `scripts/refactor-moved.sh` is **empty** (or each residual line is justified in the commit body); `pre-release-check.sh` 3 runs, median footprints within 5% of baseline and no new failing check; Step 4 manual groups and extra checklist identical to baseline |
| **C** | Task 12 | Same as B, with the census diff again empty or explained line by line |

**Gate:** Tasks 11 and 12 are the only tasks that are not pure moves, and no automated test reaches the code they change. Do not start Task 11 until Checkpoint B passes. If any residual cannot be explained, stop and fix or revert the offending commit; do not carry on. If the user prefers lower risk, Tasks 11 and 12 can be dropped from this plan after Checkpoint B and done later as a separate plan with their own characterization tests.

---

### Task 1: Extract `platform::process`

**Files:**
- Create: `src/platform/mod.rs`, `src/platform/process.rs`
- Modify: `src/main.rs` (remove items; add `mod platform;`), `src/config.rs:920-922`, `src/update.rs:1040-1046`

**Interfaces:**
- Produces: `crate::platform::process::{spawn_detached, show_notification, open_target, system_bin, play_sound}` — signatures unchanged:
  - `pub(crate) fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<()>`
  - `pub(crate) fn show_notification(title: &str, body: &str)`
  - `pub(crate) fn open_target(target: &str, is_path: bool, cwd: Option<&std::path::Path>)`
  - `pub(crate) fn system_bin(name: &'static str) -> &'static str`
  - `pub(crate) fn play_sound(path: &std::path::Path, volume: f32)`

- [ ] **Step 1: Create the module root**

`src/platform/mod.rs`:

```rust
//! OS integration that does not depend on window or terminal state:
//! detached child processes, notifications, URL / file opening, clipboard.

pub(crate) mod process;
```

Add `mod platform;` to the `mod` list in `src/main.rs` (alphabetical, after `mod mouse;`).

- [ ] **Step 2: Move the items**

Move from `src/main.rs` to `src/platform/process.rs`, in this order:
`system_bin` (≈1724), `play_sound` (≈1739), `show_notification` (≈1780), `IN_FLIGHT_DETACHED` + `MAX_CONCURRENT_DETACHED` (≈1995-1996), `spawn_detached` (≈2000), `safe_path_for_linux_open` (≈2033, keep both `#[cfg]` lines), `linux_open_command` (≈2055, keep `#[cfg_attr]`), `open_target` (≈2068).

Header for `src/platform/process.rs`:

```rust
//! Detached helper processes: `open` / `xdg-open`, notifications, sounds.
//!
//! URLs and paths are always passed as a single argument, never through a shell.
```

- [ ] **Step 3: Move the matching tests**

Following the test placement rule, move every test in `mod paste_tests` that calls `spawn_detached`, `IN_FLIGHT_DETACHED`, `safe_path_for_linux_open`, `linux_open_command` or `open_target` (at the time of writing: `linux_open_has_one_supported_target_argument`, `linux_xdg_open_dispatch_smoke`, and the tests using `spawn_detached` / `safe_path_for_linux_open`) into a new block at the end of `src/platform/process.rs`:

```rust
#[cfg(test)]
mod tests {
    // moved test fns, unchanged; `super::open_target` etc. still resolve
}
```

`linux_xdg_open_dispatch_smoke` is `#[ignore]`d and run by `scripts/linux/run.sh`; check that script's `cargo test` filter still matches the new path (`platform::process::tests::linux_xdg_open_dispatch_smoke`) and update it if it uses the old `paste_tests::` path.

- [ ] **Step 4: Update callers**

```bash
grep -rn 'crate::spawn_detached\|crate::show_notification' src
```

Replace `crate::spawn_detached` → `crate::platform::process::spawn_detached` in `src/config.rs` and `crate::show_notification` → `crate::platform::process::show_notification` in `src/update.rs`. In `src/main.rs` add:

```rust
use crate::platform::process::{open_target, play_sound, show_notification, spawn_detached, system_bin};
```

then let clippy prune the unused names.

- [ ] **Step 5: Verify**

Run the five Global Constraints commands.
Expected: all pass; `cargo test` total passed == `N`.

- [ ] **Step 6: Commit**

```bash
git add src/platform/mod.rs src/platform/process.rs src/main.rs src/config.rs src/update.rs
git commit -m "refactor(platform): move process helpers out of main.rs"
```

---

### Task 2: Extract `platform::clipboard`

**Files:**
- Create: `src/platform/clipboard.rs`
- Modify: `src/platform/mod.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `crate::platform::process::system_bin` (if the moved code calls it).
- Produces:
  - `pub(crate) fn copy_to_clipboard(text: &str)`
  - `pub(crate) fn paste_from_clipboard() -> Option<String>`

- [ ] **Step 1: Register the module**

In `src/platform/mod.rs` add `pub(crate) mod clipboard;` above `pub(crate) mod process;`.

- [ ] **Step 2: Move the items**

Move to `src/platform/clipboard.rs`: `clipboard_command` (≈3646), `IN_FLIGHT_COPIES` + `MAX_CONCURRENT_COPIES` (≈3654-3655), `copy_to_clipboard` (≈3657), `IN_FLIGHT_PASTES` + `MAX_CONCURRENT_PASTES` (≈3777-3778), `paste_from_clipboard` (≈3780, keep its per-OS `let cmds` cfg branches), `MAX_CLIPBOARD_BYTES` (≈3799), `read_clipboard_child` (≈3803), `read_clipboard_with` (≈3854). Do **not** move `shell_quote`, `paste_payload`, `history_insert_payload` (Task 3).

Header:

```rust
//! System clipboard through `pbcopy` / `pbpaste` or `wl-copy` / `xclip` / `xsel`,
//! bounded in concurrency, time, and size.
```

Only `copy_to_clipboard` and `paste_from_clipboard` become `pub(crate)`; everything else stays private.

- [ ] **Step 3: Move the matching test**

Move every remaining `paste_tests` test that calls `read_clipboard_child`, `read_clipboard_with`, `copy_to_clipboard` or `paste_from_clipboard` (at least `clipboard_helpers_drain_large_output_and_reject_over_limit_atomically`; also check the `#[cfg(target_os = "macos")]` test near the end of the module) into `#[cfg(test)] mod tests { use super::*; ... }` at the end of `src/platform/clipboard.rs`. `super::read_clipboard_child` inside them still resolves.

- [ ] **Step 4: Fix imports in `src/main.rs`**

```rust
use crate::platform::clipboard::{copy_to_clipboard, paste_from_clipboard};
```

- [ ] **Step 5: Verify** — five commands; test total == `N`.

- [ ] **Step 6: Commit**

```bash
git add src/platform/mod.rs src/platform/clipboard.rs src/main.rs
git commit -m "refactor(platform): move clipboard access out of main.rs"
```

---

### Task 3: Extract pure `paste` module

**Files:**
- Create: `src/paste.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces (signatures unchanged, now `pub(crate)`):
  - `fn shell_quote(s: &str) -> String`
  - `fn paste_payload(text: &str, bracketed: bool) -> Vec<u8>`
  - `fn history_insert_payload(cmd: &str, run: bool, bracketed: bool) -> Vec<u8>`

- [ ] **Step 1: Move the functions**

Move `shell_quote` (≈3721), `paste_payload` (≈3729), `history_insert_payload` (≈3753) to `src/paste.rs` with header:

```rust
//! Pure byte payloads for pasting into the PTY.
//!
//! `paste_payload` strips `ESC` so pasted text cannot terminate bracketed
//! paste (`\x1b[201~`) early.
```

Add `mod paste;` to `src/main.rs` and `use crate::paste::{history_insert_payload, paste_payload, shell_quote};`.

- [ ] **Step 2: Move the remaining `paste_tests`**

What is left of `mod paste_tests` in `src/main.rs` should now only test these three functions (re-run the listing command; if anything else is left, move it per the placement rule first). Move the whole module body to the end of `src/paste.rs` as:

```rust
#[cfg(test)]
mod tests {
    use super::{paste_payload, shell_quote};
    // remaining test fns, unchanged
}
```

Delete the now-empty `mod paste_tests` from `src/main.rs`.

- [ ] **Step 3: Verify** — five commands; test total == `N`.

- [ ] **Step 4: Commit**

```bash
git add src/paste.rs src/main.rs
git commit -m "refactor(paste): move paste payload helpers into pure module"
```

---

### Task 4: Extract pure `cli` argument parsing (TDD)

**Files:**
- Create: `src/cli.rs`
- Modify: `src/main.rs` (`fn main`, ≈3891-3970)

**Interfaces:**
- Produces:

```rust
pub(crate) enum Command {
    ListThemes,
    ToggleQuickTerminal,
    CheckUpdate,
    Update { force: bool },
    BenchPerf,
    BenchStress { lines: usize },
    BenchFeatures,
    Run { bench_ram: bool, bench_all: bool, scroll_fps: bool },
}
pub(crate) fn parse(args: &[String]) -> Command
```

`args` is the full argv including `args[0]`, exactly like `std::env::args()`, so every existing `nth(1)` / `any(...)` check keeps its meaning (note: `--force` is matched anywhere in argv, as today).

- [ ] **Step 1: Write the failing tests**

Create `src/cli.rs`:

```rust
//! Command-line dispatch for `litty +<command>` / `--<command>`.

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    ListThemes,
    ToggleQuickTerminal,
    CheckUpdate,
    Update { force: bool },
    BenchPerf,
    BenchStress { lines: usize },
    BenchFeatures,
    Run { bench_ram: bool, bench_all: bool, scroll_fps: bool },
}

pub(crate) fn parse(_args: &[String]) -> Command {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(a: &[&str]) -> Vec<String> {
        std::iter::once("litty").chain(a.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn no_args_runs_terminal() {
        assert_eq!(parse(&argv(&[])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: false });
    }

    #[test]
    fn plus_and_dash_spellings_are_equivalent() {
        for (plus, dash, want) in [
            ("+list-themes", "--list-themes", Command::ListThemes),
            ("+check-update", "--check-update", Command::CheckUpdate),
            ("+bench-perf", "--bench-perf", Command::BenchPerf),
            ("+bench-features", "--bench-features", Command::BenchFeatures),
        ] {
            assert_eq!(parse(&argv(&[plus])), want);
            assert_eq!(parse(&argv(&[dash])), want);
        }
    }

    #[test]
    fn toggle_quick_terminal_has_only_plus_spelling() {
        assert_eq!(parse(&argv(&["+toggle-quick-terminal"])), Command::ToggleQuickTerminal);
        assert_eq!(parse(&argv(&["--toggle-quick-terminal"])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: false });
    }

    #[test]
    fn update_force_flag_anywhere() {
        assert_eq!(parse(&argv(&["+update"])), Command::Update { force: false });
        assert_eq!(parse(&argv(&["--update", "-f"])), Command::Update { force: true });
        assert_eq!(parse(&argv(&["+update", "--force"])), Command::Update { force: true });
    }

    #[test]
    fn bench_stress_line_count_defaults_and_parses() {
        assert_eq!(parse(&argv(&["+bench-stress"])), Command::BenchStress { lines: 20_000 });
        assert_eq!(parse(&argv(&["--bench-stress", "500"])), Command::BenchStress { lines: 500 });
        assert_eq!(parse(&argv(&["--bench-stress", "abc"])), Command::BenchStress { lines: 20_000 });
    }

    #[test]
    fn bench_all_implies_bench_ram() {
        assert_eq!(parse(&argv(&["+bench-all"])), Command::Run { bench_ram: true, bench_all: true, scroll_fps: false });
        assert_eq!(parse(&argv(&["--bench-ram"])), Command::Run { bench_ram: true, bench_all: false, scroll_fps: false });
        assert_eq!(parse(&argv(&["+bench-scroll-fps"])), Command::Run { bench_ram: false, bench_all: false, scroll_fps: true });
    }
}
```

Add `mod cli;` to `src/main.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test cli::tests`
Expected: 6 tests FAIL with `not implemented`.

- [ ] **Step 3: Implement `parse`**

Replace the stub:

```rust
pub(crate) fn parse(args: &[String]) -> Command {
    let first = args.get(1).map(String::as_str).unwrap_or("");
    let is = |names: &[&str]| names.contains(&first);
    if is(&["+list-themes", "--list-themes"]) {
        Command::ListThemes
    } else if is(&["+toggle-quick-terminal"]) {
        Command::ToggleQuickTerminal
    } else if is(&["+check-update", "--check-update"]) {
        Command::CheckUpdate
    } else if is(&["+update", "--update"]) {
        Command::Update { force: args.iter().any(|a| a == "--force" || a == "-f") }
    } else if is(&["+bench-perf", "--bench-perf"]) {
        Command::BenchPerf
    } else if is(&["+bench-stress", "--bench-stress"]) {
        Command::BenchStress { lines: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20_000) }
    } else if is(&["+bench-features", "--bench-features"]) {
        Command::BenchFeatures
    } else {
        let bench_all = is(&["+bench-all", "--bench-all"]);
        Command::Run {
            bench_ram: bench_all || is(&["+bench-ram", "--bench-ram"]),
            bench_all,
            scroll_fps: is(&["+bench-scroll-fps", "--bench-scroll-fps"]),
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test cli::tests`
Expected: 6 passed.

- [ ] **Step 5: Use it in `main`**

Rewrite `fn main` in `src/main.rs` (all bodies are copied verbatim from the current `if` branches):

```rust
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match cli::parse(&args) {
        cli::Command::ListThemes => {
            for name in config::bundled_theme_names() {
                println!("{name}");
            }
        }
        cli::Command::ToggleQuickTerminal => {
            if cfg!(target_os = "macos") {
                eprintln!("litty: on macOS, use `keybind = global:<trigger>=toggle_quick_terminal`");
                std::process::exit(1);
            }
            if let Err(e) = hotkey::send_toggle() {
                eprintln!("litty: no running Litty to toggle ({e})");
                std::process::exit(1);
            }
        }
        cli::Command::CheckUpdate => update::cli_check_update(),
        cli::Command::Update { force } => update::cli_update(force),
        cli::Command::BenchPerf => {
            let lat = bench::run_latency_benchmark();
            let stress = bench::run_render_stress_benchmark(20_000);
            let feat = bench::run_critical_features_benchmark();
            if !lat.passed || !stress.passed || !feat.passed {
                std::process::exit(1);
            }
        }
        cli::Command::BenchStress { lines } => {
            if !bench::run_render_stress_benchmark(lines).passed {
                std::process::exit(1);
            }
        }
        cli::Command::BenchFeatures => {
            if !bench::run_critical_features_benchmark().passed {
                std::process::exit(1);
            }
        }
        cli::Command::Run { bench_ram, bench_all, scroll_fps } => {
            if bench_all {
                let lat = bench::run_latency_benchmark();
                let stress = bench::run_render_stress_benchmark(20_000);
                let feat = bench::run_critical_features_benchmark();
                if !lat.passed || !stress.passed || !feat.passed {
                    eprintln!("litty: performance/features benchmark failed");
                    std::process::exit(1);
                }
            }
            let event_loop = EventLoop::<AppEvent>::with_user_event().build().expect("event loop");
            event_loop.set_control_flow(ControlFlow::Wait);
            let mut app = App {
                waker: Waker { proxy: event_loop.create_proxy(), queued: Arc::new(AtomicBool::new(false)) },
                windows: Vec::new(),
                bench_ram_mode: bench_ram,
                scroll_fps_run: scroll_fps.then(frame_metrics::ScrollRun::new),
            };
            event_loop.run_app(&mut app).expect("run app");
        }
    }
}
```

- [ ] **Step 6: Smoke test the CLI**

```bash
cargo run -- +list-themes | head -3
cargo run -- --bench-stress 200; echo "exit=$?"
```

Expected: theme names print; stress bench prints its report and `exit=0`.

- [ ] **Step 7: Verify** — five commands; test total == `N + 6`. From now on call this `N1`.

- [ ] **Step 8: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "refactor(cli): parse command-line dispatch in a pure module"
```

---

### Task 5: Move the app core into `src/app/mod.rs`

**Files:**
- Create: `src/app/mod.rs`, `src/app/latency_benchmarks.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `pub(crate) fn app::run(bench_ram_mode: bool, scroll_fps: bool)`, called from `main`'s `Command::Run` arm.

This is one large cut-and-paste; everything still lives in one file afterwards, so it is safe. Later tasks split it.

- [ ] **Step 1: Move everything except the crate root into `src/app/mod.rs`**

Keep in `src/main.rs` only: the `//!` doc, `mod` declarations, `pub use event::...`, `pub use tab::...`, the mimalloc `GLOBAL`, `mod test_alloc`, and `fn main`. Move **everything else** (the `use` block, `State`, `App`, all `impl`s, all remaining free functions and statics) into `src/app/mod.rs`, in original order, under:

```rust
//! Window and event-loop coordination: one `State` per window, action dispatch,
//! mouse / keyboard / IME input.
```

Add `mod app;` to `src/main.rs`.

- [ ] **Step 2: Add the `run` entry point**

At the end of `src/app/mod.rs` (before tests):

```rust
pub(crate) fn run(bench_ram_mode: bool, scroll_fps: bool) {
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        waker: Waker { proxy: event_loop.create_proxy(), queued: Arc::new(AtomicBool::new(false)) },
        windows: Vec::new(),
        bench_ram_mode,
        scroll_fps_run: scroll_fps.then(frame_metrics::ScrollRun::new),
    };
    event_loop.run_app(&mut app).expect("run app");
}
```

In `src/main.rs`, replace the last five statements of the `Command::Run` arm (event loop creation through `run_app`) with `app::run(bench_ram, scroll_fps);`.

- [ ] **Step 3: Move `latency_benchmarks`**

Cut `#[cfg(test)] mod latency_benchmarks { ... }` out of `src/app/mod.rs`; put its body into `src/app/latency_benchmarks.rs` (without the outer `mod` braces) and declare at the end of `src/app/mod.rs`:

```rust
#[cfg(test)]
mod latency_benchmarks;
```

Its `use super::*;` still resolves to `app`.

- [ ] **Step 4: Fix paths**

Inside `src/app/mod.rs`, sibling crate modules are referenced as `crate::x` or via `use`. Add near the top:

```rust
use crate::{ai, config, frame_metrics, keymap, macos, mouse, renderer, scroll, split, update};
use crate::{next_id, AppEvent, HoveredPaneLink, Pane, Tab, Waker};
```

Then `cargo check` and add any other `crate::` module it names as unresolved; prune unused imports in both files with clippy.

- [ ] **Step 5: Verify**

Five commands; test total == `N1`. Then:

```bash
wc -l src/main.rs
```

Expected: under ~130 lines.

- [ ] **Step 6: Commit**

```bash
git add src/app/mod.rs src/app/latency_benchmarks.rs src/main.rs
git commit -m "refactor(app): move window and event loop code into app module"
```

---

### Task 6: Peel out `app::state` and `app::window`

**Files:**
- Create: `src/app/state.rs`, `src/app/window.rs`
- Modify: `src/app/mod.rs`

**Interfaces:**
- Produces: `pub(super) struct State` with all fields `pub(super)`; `pub(super) fn` for every moved function, signatures unchanged.

- [ ] **Step 1: `state.rs`**

Move `struct State`, `impl State { invalidate_pane_state }`, `impl Drop for State`. Prefix the struct and every field with `pub(super)`, and `invalidate_pane_state` with `pub(super)`. Header: `//! Per-window state.` In `mod.rs`: `mod state; use state::State;`.

- [ ] **Step 2: `window.rs`**

Move: `place_quick_terminal`, `font_features`, `create_window`, `spawn_pane`, `tab_title`, `refresh_tab_titles`, `cursor_blinks`, `cursor_is_visible_now`, `reload_config`, `render_active`, `apply_system_appearance`, `sync_size`, `zoom`. Header: `//! Window creation, sizing, config reload and per-frame rendering.` In `mod.rs`: `mod window;` plus `use window::{...};` for the names `mod.rs` still calls. Also add `use super::window::render_active;` in `latency_benchmarks.rs` if the compiler asks.

- [ ] **Step 3: Verify** — five commands; test total == `N1`.

- [ ] **Step 4: Commit**

```bash
git add src/app/state.rs src/app/window.rs src/app/mod.rs src/app/latency_benchmarks.rs
git commit -m "refactor(app): split window state and lifecycle into modules"
```

---

### Task 7: Peel out `app::terminal_io` and `app::ai_bridge`

**Files:**
- Create: `src/app/terminal_io.rs`, `src/app/ai_bridge.rs`
- Modify: `src/app/mod.rs` (and siblings' `use` lines)

- [ ] **Step 1: `terminal_io.rs`** — move `write_pty`, `write_pane_input`, `handle_terminal_requests`, `pointer_icon`. Header: `//! PTY writes and draining terminal-originated requests (title, cwd, bell, OSC 52, notifications).`

- [ ] **Step 2: `ai_bridge.rs`** — move `ai_display`, `handle_ai_requests`. Header: `//! Bridges grid AI requests to the async `ai` module and back.`

- [ ] **Step 3: Verify** — five commands; test total == `N1`. Also confirm no grid lock is now held across a call that moved files:

```bash
grep -n 'lock()' src/app/terminal_io.rs src/app/ai_bridge.rs
```

Expected: same lock / write ordering as before (diff the function bodies with `git diff -M --color-moved=zebra HEAD~1 -- src/app` — moved lines show as moved, not changed).

- [ ] **Step 4: Commit**

```bash
git add src/app/terminal_io.rs src/app/ai_bridge.rs src/app/mod.rs src/app/window.rs
git commit -m "refactor(app): move pty io and ai bridge into modules"
```

(Stage only the files that `git status` shows as changed.)

---

### Task 8: Peel out `app::pointer` and `app::ime`

**Files:**
- Create: `src/app/pointer.rs`, `src/app/ime.rs`
- Modify: `src/app/mod.rs`

- [ ] **Step 1: `pointer.rs`** — move `scrollbar_at_mouse`, `drag_scrollbar`, `pane_cell_at_mouse`, `report_mouse`, `alt_screen_scroll`, `matches_cached_hover_candidate`, `update_hovered_link`. Header: `//! Mouse pointer: scrollbar drag, application mouse reporting, link hover.` Move the test `test_cached_hover_candidate_invalidated_when_row_overwritten` from `latency_benchmarks.rs` into `#[cfg(test)] mod tests` in `pointer.rs`.

- [ ] **Step 2: `ime.rs`** — move `handle_ime`. Header: `//! Input-method composition (preedit, commit, candidate window placement).`

- [ ] **Step 3: Verify** — five commands; test total == `N1`.

- [ ] **Step 4: Commit**

```bash
git add src/app/pointer.rs src/app/ime.rs src/app/mod.rs src/app/latency_benchmarks.rs
git commit -m "refactor(app): move pointer and ime handling into modules"
```

---

### Task 9: Peel out `app::tabs`

**Files:**
- Create: `src/app/tabs.rs`
- Modify: `src/app/mod.rs`

- [ ] **Step 1: Move** `open_fresh_tab`, `duplicate_active_tab`, `confirm_close`, `close_last_window`, `hide_or_exit_last_window`, `select_tab`, `close_active_tab`, `close_tab_at`, `split_active_pane`, `close_active_pane_or_tab`, `navigate_split_directional`, `cycle_split`, `toggle_split_zoom`, `equalize_splits`, `resize_active_split`. Header: `//! Tab and split-pane lifecycle and navigation for one window.` Move `test_tab_active_pane_empty_panes_does_not_panic` from `latency_benchmarks.rs` into `#[cfg(test)] mod tests` in `tabs.rs`.

- [ ] **Step 2: Verify** — five commands; test total == `N1`.

- [ ] **Step 3: Commit**

```bash
git add src/app/tabs.rs src/app/mod.rs src/app/latency_benchmarks.rs
git commit -m "refactor(app): move tab and split management into module"
```

---

### Task 10: Peel out `app::keyboard`, `app::inspector`, `app::search_ui`, `app::actions`

**Files:**
- Create: `src/app/keyboard.rs`, `src/app/inspector.rs`, `src/app/search_ui.rs`, `src/app/actions.rs`
- Modify: `src/app/mod.rs`

- [ ] **Step 1: `keyboard.rs`** — move `handle_key`, `encode_terminal_key`, `describe_key`, `consume_key`, `handle_key_release`, `kitty_key`. Header: `//! Key events: shortcuts first, then Kitty / legacy encoding to the PTY. Pure key → bytes lives in `crate::keymap`.`

- [ ] **Step 2: `inspector.rs`** — move `inspector_lines`. Header: `//! Terminal inspector overlay text.`

- [ ] **Step 3: `search_ui.rs`** — move `handle_history_search_key`, `handle_search_key`, `refresh_search`, `search_step`, `close_search`. Header: `//! Find box and history search popup key handling.`

- [ ] **Step 4: `actions.rs`** — move `run_action`, `run_tab_action`. Header: `//! Dispatches `keymap::Action` (keybindings and menu items) to window operations.`

- [ ] **Step 5: Verify** — five commands; test total == `N1`.

```bash
grep -cE '^(pub\(super\) )?fn ' src/app/mod.rs
```

Expected: `0` free functions left in `mod.rs` besides `run` (only `App` methods and trait impl remain).

- [ ] **Step 6: Commit**

```bash
git add src/app/keyboard.rs src/app/inspector.rs src/app/search_ui.rs src/app/actions.rs src/app/mod.rs
git commit -m "refactor(app): move keyboard, search and action dispatch into modules"
```

---

### Task 11: Split the giant event handlers into per-event methods

**Files:**
- Create: `src/app/window_event.rs`, `src/app/user_event.rs`
- Modify: `src/app/mod.rs`

**Interfaces:**
- Produces (all `impl App`, `pub(super)` unless noted):
  - `fn handle_window_event(&mut self, id: WindowId, event: WindowEvent)` — a thin `match` that calls:
  - `fn on_cursor_moved(&mut self, id: WindowId, position: PhysicalPosition<f64>)`
  - `fn on_mouse_input(&mut self, id: WindowId, btn_state: ElementState, button: MouseButton)`
  - `fn on_mouse_wheel(&mut self, id: WindowId, delta: MouseScrollDelta, phase: TouchPhase)`
  - `fn on_focused(&mut self, id: WindowId, focused: bool)`
  - `fn on_redraw_requested(&mut self, id: WindowId)`
  - `fn on_child_exit(&mut self, event_loop: &ActiveEventLoop, exited_id: u64)`
  - `fn on_reopen(&mut self, event_loop: &ActiveEventLoop)`
  - `fn on_update_check_result(&mut self, result: Result<update::ReleaseInfo, String>)`
  - `fn on_bench_action(&mut self, event_loop: &ActiveEventLoop, cmd: crate::bench::BenchCommand)`

- [ ] **Step 1: Move `handle_window_event`**

Move the second `impl App { fn handle_window_event ... }` block (≈501 lines) verbatim into `src/app/window_event.rs` as `impl App { pub(super) fn handle_window_event(...) }`. Verify and commit separately so the next diff is reviewable:

```bash
git add src/app/window_event.rs src/app/mod.rs
git commit -m "refactor(app): move window event handler into module"
```

- [ ] **Step 2: Extract one arm at a time**

Start with the largest: the `WindowEvent::MouseInput` arm (≈225 lines). `handle_window_event` begins with:

```rust
let waker = self.waker.clone();
let Some(s) = self.window_mut(id) else { return };
match event { ... }
```

and nothing runs after the `match`, so an early `return` inside an arm behaves the same inside an extracted method. Each new method repeats the prologue (drop the `waker` line if the arm body does not use `waker`):

```rust
fn on_mouse_input(&mut self, id: WindowId, btn_state: ElementState, button: MouseButton) {
    let waker = self.waker.clone();
    let Some(s) = self.window_mut(id) else { return };
    // original arm body, unchanged
}
```

`handle_window_event` keeps its prologue only if some inline arm still uses `s` / `waker`; the extracted arms are dispatched before the lookup, e.g. by matching them first. Simplest borrowck-friendly form — put an early dispatch above the prologue, one line per extracted arm, and leave the existing `match` below untouched for the inline arms:

```rust
let event = match event {
    WindowEvent::MouseInput { state, button, .. } => return self.on_mouse_input(id, state, button),
    WindowEvent::CursorMoved { position, .. } => return self.on_cursor_moved(id, position),
    other => other,
};
```

Replace the arm with `WindowEvent::MouseInput { state: btn_state, button, .. } => self.on_mouse_input(id, btn_state, button),`. Then do `CursorMoved`, `MouseWheel`, `Focused`, `RedrawRequested` the same way. Leave arms under ~15 lines inline.

- [ ] **Step 3: Same for `user_event`**

Create `src/app/user_event.rs` with `impl App { ... }` and extract the `AppEvent::ChildExit`, `Reopen`, `UpdateCheckResult`, `BenchAction` arms of `ApplicationHandler::user_event` into the methods listed above. `user_event` itself stays in `mod.rs` as a thin `match`.

- [ ] **Step 4: Verify** — five commands; test total == `N1`.

```bash
awk '/fn handle_window_event/,/^    }$/' src/app/window_event.rs | wc -l
```

Expected: under ~80 lines.

- [ ] **Step 5: Manual smoke test** (event routing changed shape, so exercise it)

`cargo run --release`, then: click, double-click, triple-click select; Shift+click extend; drag the scrollbar; wheel scroll in shell and in `vim` (alt screen); open `htop` and click (mouse reporting); focus another app and back (secure input / cursor); hover and Cmd+click a URL; resize the window; quit and reopen from Dock (macOS).

- [ ] **Step 6: Commit**

```bash
git add src/app/window_event.rs src/app/user_event.rs src/app/mod.rs
git commit -m "refactor(app): split window and user event handlers per event"
```

---

### Task 12: Group `State` fields into sub-structs

**Files:**
- Modify: `src/app/state.rs`, `src/app/window.rs` (`create_window` initializer), every `src/app/*.rs` that touches the moved fields.

**Interfaces:**
- Produces in `src/app/state.rs`:

```rust
/// Mouse pointer, click sequence and hover tracking.
pub(super) struct PointerState {
    pub(super) mouse_pos: (f64, f64),
    pub(super) selecting: bool,
    pub(super) drag_moved: bool,
    pub(super) last_click_time: Option<std::time::Instant>,
    pub(super) last_click_cell: Option<(usize, usize)>,
    pub(super) active_click_pane_id: Option<u64>,
    pub(super) click_count: u8,
    pub(super) click_on_selected_block: bool,
    pub(super) extend_drag: bool,
    pub(super) scrollbar_drag: Option<u64>,
    pub(super) wheel_scroll: scroll::Accumulator,
    pub(super) pointer_icon: CursorIcon,
    pub(super) hovered_link: Option<HoveredPaneLink>,
    pub(super) mouse_hidden: bool,
    pub(super) mouse_button_down: Option<mouse::Button>,
    pub(super) last_mouse_report: Option<(u64, usize, usize)>,
}

/// Input-method composition.
pub(super) struct ImeState {
    pub(super) preedit: String,
    pub(super) cursor_cell: Option<(usize, usize)>,
}

/// Requests a window raises during an event; `App` acts on them afterwards.
pub(super) struct WindowRequests {
    pub(super) close: bool,
    pub(super) new_window: bool,
    pub(super) native_tab: bool,
    pub(super) quick_terminal: bool,
    pub(super) quit: bool,
    pub(super) reload_config: bool,
}

/// Terminal inspector overlay.
pub(super) struct InspectorState {
    pub(super) shown: bool,
    pub(super) last_key: String,
}
```

and in `State`, replacing the individual fields: `pub(super) pointer: PointerState, pub(super) ime: ImeState, pub(super) requests: WindowRequests, pub(super) inspector: InspectorState,`. Keep the existing `///` doc comments on each moved field.

Do one sub-struct per commit, in this order (smallest blast radius first): `InspectorState`, `ImeState`, `WindowRequests`, `PointerState`.

- [ ] **Step 1: `InspectorState`**

Add the struct, replace `inspector` / `inspector_key` fields with `inspector: InspectorState`, change the initializer in `create_window` from `inspector: false, inspector_key: String::new(),` (use whatever the current values are) to `inspector: InspectorState { shown: <old inspector value>, last_key: <old inspector_key value> },`. Rename accesses:

```bash
perl -pi -e 's/\b(s|self|st|state|w)\.inspector_key\b/$1.inspector.last_key/g; s/\b(s|self|st|state|w)\.inspector\b(?!\.)/$1.inspector.shown/g' src/app/*.rs
cargo check
```

Fix any remaining access the compiler reports by hand (other binding names). Verify (five commands; total == `N1`), then commit `refactor(app): group inspector fields in state`.

- [ ] **Step 2: `ImeState`**

```bash
perl -pi -e 's/\b(s|self|st|state|w)\.ime_preedit\b/$1.ime.preedit/g; s/\b(s|self|st|state|w)\.ime_cursor_cell\b/$1.ime.cursor_cell/g' src/app/*.rs
```

Update the initializer, verify, commit `refactor(app): group ime fields in state`.

- [ ] **Step 3: `WindowRequests`**

```bash
perl -pi -e 's/\b(s|self|st|state|w)\.(close|new_window|native_tab|quick_terminal|quit|reload_config)_requested\b/$1.requests.$2/g' src/app/*.rs
```

Update the initializer, verify, commit `refactor(app): group pending window requests in state`.

- [ ] **Step 4: `PointerState`**

```bash
perl -pi -e 's/\b(s|self|st|state|w)\.(mouse_pos|selecting|drag_moved|last_click_time|last_click_cell|active_click_pane_id|click_count|click_on_selected_block|extend_drag|scrollbar_drag|wheel_scroll|pointer_icon|hovered_link|mouse_hidden|mouse_button_down|last_mouse_report)\b/$1.pointer.$2/g' src/app/*.rs
```

`pointer_icon` is also a free function name in `terminal_io.rs`; the regex only matches field access (`x.pointer_icon`), but check `git diff src/app/terminal_io.rs` to be sure a call like `pointer_icon(name)` was not touched. Update `State::invalidate_pane_state` accordingly. Update the initializer, verify, commit `refactor(app): group pointer fields in state`.

- [ ] **Step 5: Borrow-split opportunity check (no code change required)**

```bash
grep -c 'pub(super)' src/app/state.rs
```

Record the number of top-level `State` fields (expected ≈ 25, down from 50+). Do not change function signatures to take `&mut PointerState` in this plan — that is a follow-up once the structure has settled.

---

### Task 13: Docs and final verification

**Files:**
- Modify: `CLAUDE.md` (Architecture & Modules), `ROADMAP.md`

- [ ] **Step 1: Update `CLAUDE.md` Architecture & Modules**

Replace the `src/main.rs` bullet with:

```markdown
- `src/main.rs`: crate root and CLI dispatch (`src/cli.rs`); starts `app::run`.
- `src/app/`: winit loop and per-window `State`; input (`keyboard`, `pointer`, `ime`), `tabs`, `actions`, `window_event` / `user_event` handlers.
- `src/platform/`: clipboard and detached processes (`open` / `xdg-open`, notifications, sounds).
```

- [ ] **Step 2: Update `ROADMAP.md`**

Add under the code-health section: "Phase 1 done: `main.rs` split into `app/` + `platform/` (plan: `plans/2026-11-01-split-main-rs.md`). Next: phase 2 `grid/mod.rs`."

- [ ] **Step 3: Full verification and size check**

```bash
cargo check && cargo check --target x86_64-unknown-linux-gnu && cargo test && cargo clippy --all-targets -- -D warnings && git diff --check
wc -l src/main.rs src/app/*.rs src/platform/*.rs src/cli.rs src/paste.rs | sort -rn | head
```

Expected: all green; no file in `src/app/` above ~700 lines; total test count == `N1`.

- [ ] **Step 4: Release-mode smoke test**

Re-run the Task 0.5 Checkpoint C evidence (census diff, `pre-release-check.sh` x3, manual groups and extra checklist) against the Task 0.5 baseline, then remove the helpers in their own commit: `git rm scripts/refactor-census.sh scripts/refactor-moved.sh && git commit -m "chore(refactor): remove refactor helper scripts"`. Repeat Task 11 Step 5 plus: Cmd+C / Cmd+V (and Ctrl+Shift+C / V on Linux), bracketed paste of text containing `\x1b[201~`, Cmd+T / Cmd+W, Cmd+D splits and navigation, Cmd+F search, Ctrl+R history, Telex IME typing, config reload, Quick Terminal toggle. Check idle RAM (< 50 MB) and idle CPU (0%) in Activity Monitor / `top`.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md ROADMAP.md
git commit -m "docs(app): describe app and platform module layout"
```

---

## Phases 2–4 (outline)

Detailed plans, which supersede this outline where they differ: `2026-11-01-split-grid.md` (phase 2), `2026-11-01-split-renderer-atlas.md` (phase 3), `2026-11-01-layered-layout.md` (phase 4).

### Phase 2 — `src/grid/mod.rs` (5499 lines, `Grid` with 88 fields)

**Gate:** run `cargo run --release -- +bench-perf` and `cargo test --release` latency benchmarks on `main` and save the numbers; after each task, numbers must stay within noise (±3%). `Cell` stays 12 bytes.

Target layout (each a separate commit, data first, behavior-free):
1. `grid/tests.rs` — move the ~2200-line test module out (`#[cfg(test)] mod tests;`). Biggest win for readability, zero risk.
2. `grid/colors.rs` — `DEFAULT_*_RGB`, `PALETTE_BASE`, `COLOR_SCHEME_DARK`, `set_*`, `sgr`, `fg_color`, `bg_color`, `osc4`, `osc104`, `osc_dynamic_color`, `recolor_pairs`.
3. `grid/clusters.rs` — `struct ClusterTable` owning `clusters`, `cluster_ids`, `cluster_buf`, `cluster_evictions`, `cluster_marks`, `cluster_remap`; methods `intern_cluster`, `cluster`, `gc_clusters`, `cluster_count`.
4. `grid/storage.rs` — `struct Ring { cells, top, cap, scrollback, history, wrapped }` with `row_start`, `row_mut`, `grow_capacity`, `copy_row`, `clear_rows`, `scroll_up_in`, `scroll_down_in`. All accessors `#[inline]`; benchmark gate is strict here.
5. `grid/images.rs` — `defer_images`, `image_job`, `image_epoch`, `graphics`, `queue_image`, `complete_image`, `apply_kitty`, `place_*`, `placeholder_*`, `visible_images`.
6. `grid/control.rs` — OSC / DCS / APC buffers (`osc_*`, `dcs*`, `apc_*`) and `reply_color`, `reply_termcap`, `reply_decrqss`, `sgr_report`, `apc_dispatch`.
7. `grid/prompt.rs` (exists, 13 lines) — `prompts`, `prompt_input_mark`, `record_prompt_mark`, `jump_to_prompt`, `select_last_output`, `full_prompt_input`, `current_prompt_input`, `calculate_cursor_movement_to`, `calculate_delete_*`, `guess_command_start`.
8. `grid/links.rs` — `is_path_char`, `find_hyperlink_*`, `set_link`, `cell_link`, `osc8`, extras (`extras*`, `intern_extra`, `collect_extras`, `trim_extras`).
9. `AiTrack` struct for `ai_available`, `ai_direct`, `ai_armed`, `ai_track`, `ai_command`, `ai_request` (grid keeps the data, `ai` module owns the meaning).
10. `TerminalModes` struct for the ~20 mode flags (`app_cursor`, `bracketed_paste`, `mouse_*`, `focus_events`, `autowrap`, `insert_mode`, `kitty_keyboard`, …).

Expected end state: `grid/mod.rs` ≈ 1000 lines, `Grid` ≈ 25 top-level fields.

### Phase 3 — `src/renderer.rs` (2270 lines) and `src/atlas.rs` (2904 lines)

**Gate:** frame-time benchmark (`+bench-scroll-fps`) and RAM check before and after; visual diff via screenshots of a fixed scene (tab bar, split, search box, history popup, inspector, images).

- `renderer/mod.rs` (device, pipelines, bind groups, upload, `Renderer` API) and `renderer/ui/{tab_bar,search_box,history_popup,inspector,scrollbar}.rs` for the `build_*` instance builders.
- Split `render_panes` (607 lines) into `collect_pane_instances` (per pane) + `collect_overlays` + `submit`.
- `atlas/mod.rs` (cache, packing), `atlas/shaper.rs` (rustybuzz), `atlas/coretext.rs` (`#[cfg(target_os = "macos")]`, dedupe the two copies of `CGPoint` / `CGSize` / `CGRect`), `atlas/linux.rs` (counterpart), `atlas/tests.rs`.

### Phase 4 — Layered directory layout and platform symmetry

- `src/terminal/` ← `grid/`, `pty.rs`, `search.rs`, `history.rs`, `bidi.rs`, `shell_integration.rs`, `graphics.rs`.
- `src/render/` ← `renderer/`, `atlas/`, `fontdb.rs`, `postfx.rs`, `image_layer.rs`, `*.wgsl`.
- `src/platform/{macos,linux}/` ← `macos.rs` (split by window / hotkey / menu / secure input), `hotkey.rs`, `x11colors.rs`, with a matching no-op or Linux implementation for each macOS item.
- Move `security_poc.rs` and `bench.rs` test-only parts under `#[cfg(test)]` files beside the code they test.
- Update `CLAUDE.md` architecture section and module paths in docs.

Phase 4 is pure `git mv` + path fixes; do it last so phases 2–3 diffs stay readable in `git log --follow`.
