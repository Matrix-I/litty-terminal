# Layered Directory Layout and Platform Symmetry (Phase 4) — Implementation Plan

Current Linux status (2026-11-04): the user confirms all features have been tested
on Linux and run stably. Earlier pending Linux feature checks and Linux-specific
maintenance proposals below are historical, superseded by the
[closed Linux validation status](../validation/linux-status-2026-11-04.md).
Original command results and measurement limits are preserved; this update does
not supply new benchmark numbers or command logs.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Regroup the flat `src/` tree into three layers — `src/terminal/` (emulation core), `src/render/` (GPU, glyphs, fonts) and `src/platform/{macos,linux}/` (OS integration with one symmetric API) — using `git mv` plus path fixes, with zero behavior change, so that the dependency rules *terminal → nothing, render → terminal, platform → nothing, app → everything* hold and are enforced by a test.

**Architecture:** Phase 4 of the 4-phase structural refactor outlined in `plans/2026-11-01-split-main-rs.md`. It runs after phase 1 (`src/app/*`, `src/platform/{mod,process,clipboard}.rs`, `src/cli.rs`, `src/paste.rs`), phase 2 (`src/grid/` split into submodules) and phase 3 (`src/renderer/{mod.rs,ui/*}`, `src/atlas/{mod,shaper,coretext,linux,tests}.rs`). Every directory move is its own commit made with `git mv` and only path edits, so `git log --follow` crosses it. The two edits that are not pure moves — cutting the terminal layer's two upward dependencies and splitting `macos.rs` by concern with Linux twins — are separate commits, each guarded by a layering test added in the same commit.

**Tech Stack:** Rust 2024 edition, winit 0.30, wgpu 24, parking_lot, x11rb (Linux); cargo targets `aarch64-apple-darwin` (host) and `x86_64-unknown-linux-gnu`.

## Global Constraints

- Pure move / rename refactor: no behavior, timing, ordering, or allocation change. If a task tempts you to "fix" something, add it to `ROADMAP.md` instead (Task 8 lists the known follow-ups).
- Hot paths (PTY parsing, scrolling, render loop) must not gain heap allocations, `dyn` dispatch, or extra locking. Moving a module never changes inlining across modules inside one crate, so no benchmark gate is needed; Task 8 still runs a release smoke test.
- Never hold the grid `Mutex` across blocking I/O — do not reorder any lock / I/O sequence while moving code.
- Every platform-specific item keeps its `#[cfg(...)]` / `#[cfg_attr(...)]` attributes verbatim unless a task explicitly says to delete one (Task 5 deletes call-site gates whose Linux twin is a no-op).
- Moved items keep their `///` doc comments and `#[...]` attributes.
- Visibility: new layer roots declare children `pub(crate) mod x;`. Items inside moved files keep their current visibility. Items split out of `platform/macos/mod.rs` into sibling files become `pub(super)` only when a sibling needs them — never `pub(crate)`.
- Re-list before every move. File names produced by phases 2–3 may differ slightly from this plan (for example `grid/osc.rs` instead of `grid/control.rs`, or `shader.wgsl` already moved into `src/renderer/`). Every task starts with an `ls` / `git ls-files` / `grep` step; trust its output over this document and adapt the `git mv` lines, not the intent.
- Rename detection: never mix a move and a content split in one commit. After staging, every moved file must show as `rename ... (NN%)` with NN ≥ 90 in `git diff --cached -M --summary`.
- Verification before every commit (`CLAUDE.md`, plus Linux Clippy because this phase rearranges `cfg`-gated code that macOS Clippy never compiles):
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings`
  - `cargo test` (outside the sandbox; PTY tests need `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit format: `type(scope): summary`, lowercase, imperative, ≤ 72 chars. Stage files explicitly (`git add <paths>`); never `git add -A`.
- Test count must equal the Task 0 baseline `N` plus the tests this plan adds (stated per task).

## Decisions

| Item | Decision | Why |
| --- | --- | --- |
| `grid/`, `pty.rs`, `search.rs`, `history.rs`, `bidi.rs`, `graphics.rs`, `shell_integration.rs` | → `src/terminal/` | All are VT / shell-session state. `search` and `history` are consumed by the renderer, so placing them in `terminal` keeps `render → terminal` legal; placing them in `app` would create a `render → app` edge. |
| `x11colors.rs` | → `src/terminal/x11colors.rs`, **not** `platform/linux` | It is the X11 `rgb.txt` color-*name* table used by OSC 4/10/11/12 parsing (`grid`) and by `config` on **both** platforms. It contains no X11 calls. |
| `frame_metrics.rs` | → `src/render/frame_metrics.rs` | `Renderer` owns `frame_metrics::Capture` (swapchain timing); leaving it at the root would be a `render → app` edge. `app` still uses `ScrollRun` (app → render is legal). |
| `renderer/`, `atlas/`, `fontdb.rs`, `postfx.rs`, `image_layer.rs`, `*.wgsl` | → `src/render/`, shaders in `src/render/shaders/` | One place for every GPU / font file; shaders are found by `include_wgsl!`. |
| `macos.rs` | → `src/platform/macos/*.rs` split by concern; its `stub` → `src/platform/linux/*.rs` with identical file and function names | Symmetric API; parity becomes a build error instead of a convention. |
| `hotkey.rs` | → `src/platform/control_socket.rs` (Unix control socket, both platforms) + `src/platform/linux/hotkey.rs` (X11 key grab) | The socket half is shared Unix code; only the grab is Linux-specific. |
| `security_poc.rs` | **stays** at `src/security_poc.rs` (`#[cfg(test)]`); Task 7 makes its source scan cover the whole tree | It is a CWE regression catalogue that deliberately spans every layer (`graphics`, `update`, `bench`, `paste`, `clipboard`). Inside any one layer it would itself violate the layer rules; scattering it would lose the catalogue that `ROADMAP.md` audit entries refer to. `cargo test poc_` keeps selecting the whole suite. |
| `bench.rs` | **stays** at `src/bench.rs` (`pub mod bench`) | Not test-only: it is runtime code behind the `+bench-*` CLI commands and `AppEvent::BenchAction`, and depends on `terminal` + `app`-tier modules, i.e. it is app tier. |
| `test_alloc.rs` | **stays** at `src/test_alloc.rs`, declared `#[cfg(test)] mod test_alloc;` in `main.rs` | It defines the crate's single test `#[global_allocator]`; tests in `terminal`, `render` and `app` call `crate::test_alloc::allocations`. The layering test whitelists it as test infrastructure. |
| `src/shell/` (integration scripts) | **stays**; `terminal/shell_integration.rs` uses `include_str!("../shell/...")` | `scripts/test-yo-shells.py` and `tests/yo_shells.rs` (branch `refactor/yo-shells-test-rs`) read `src/shell/*` via the repo root; not moving them keeps both working untouched. |
| `config`, `keymap`, `split`, `tab`, `event`, `ai/`, `update`, `mouse`, `scroll`, `cli`, `paste`, `bench` | **stay** at the crate root as the app tier | Out of scope for phase 4. `scripts/bench-scroll.rs` includes `../src/scroll.rs` by `#[path]`, and tests resolve the real files `./src/tab.rs` and `src/main.rs`, so these must not move. |
| `keymap::mac_virtual_keycode` / `keymap::x11_keysym` | **stay** in `keymap` | Pure tables unit-tested on both hosts; moving them under `cfg` would drop half their test coverage on each platform. |
| lib target (`src/lib.rs`) | **not now**; optional Task 9 with criteria | See Task 9 for the trade-offs. |

## Target File Structure

```
src/main.rs                     crate root: mod decls, allocators, CLI dispatch (phase 1)
src/cli.rs  src/paste.rs        (phase 1, unchanged)
src/app/…                       (phase 1, unchanged; imports updated)
src/ai/ src/config.rs src/event.rs src/keymap.rs src/mouse.rs src/scroll.rs
src/split.rs src/tab.rs src/update.rs src/bench.rs        app-tier modules (unchanged location)
src/security_poc.rs             #[cfg(test)] cross-layer CWE regression suite
src/test_alloc.rs               #[cfg(test)] counting global allocator
src/layering_tests.rs           #[cfg(test)] layer-rule guard (new, Task 2)
src/shell/                      shell integration scripts (unchanged)

src/terminal/mod.rs             layer root (new)
src/terminal/grid/…             ← src/grid/ (phase-2 submodules unchanged inside)
src/terminal/pty.rs             ← src/pty.rs
src/terminal/search.rs          ← src/search.rs
src/terminal/history.rs         ← src/history.rs
src/terminal/bidi.rs            ← src/bidi.rs
src/terminal/graphics.rs        ← src/graphics.rs
src/terminal/shell_integration.rs  ← src/shell_integration.rs (+ ShellIntegration, ShellFeatures from config)
src/terminal/x11colors.rs       ← src/x11colors.rs
src/terminal/text.rs            strip_controls (moved from ai::markdown::clean, Task 2)

src/render/mod.rs               layer root (new)
src/render/renderer/…           ← src/renderer/ (phase-3 mod.rs + ui/*)
src/render/atlas/…              ← src/atlas/ (phase-3 mod, shaper, coretext, linux, tests)
src/render/fontdb.rs            ← src/fontdb.rs
src/render/postfx.rs            ← src/postfx.rs
src/render/image_layer.rs       ← src/image_layer.rs
src/render/frame_metrics.rs     ← src/frame_metrics.rs
src/render/shaders/shader.wgsl  ← src/shader.wgsl (or wherever phase 3 left it)
src/render/shaders/image.wgsl   ← src/image.wgsl

src/platform/mod.rs             the cross-platform API: explicit `pub(crate) use imp::{…}` list
src/platform/process.rs  src/platform/clipboard.rs   (phase 1, unchanged)
src/platform/control_socket.rs  ← src/hotkey.rs (socket half; both platforms)
src/platform/macos/mod.rs       ← src/macos.rs; then split into:
src/platform/macos/objc.rs      Id, NSRect, NSPoint, objc FFI, msg_send!, sel/class/ns_string helpers (macOS-internal)
src/platform/macos/app.rs       release_free_memory, register_reopen_handler, system_dark, hide_app, set_dock_icon, activate_app
src/platform/macos/dialog.rs    confirm, alert_choice, alert, show_progress_dialog, close_progress_dialog
src/platform/macos/hotkey.rs    set_global_hotkey (Carbon), listen_for_toggle (no-op)
src/platform/macos/input.rs     set_secure_input, beep, caps_lock
src/platform/macos/menu.rs      install_menu, set_secure_input_menu_state
src/platform/macos/native_clipboard.rs  clipboard_string, clipboard_write_string
src/platform/macos/tabs.rs      add_tabbed_window, set_tabbing_allowed, select_native_tab
src/platform/macos/window.rs    configure_layer_drawables, apply_window_config, cleanup_window,
                                set_window_represented_filename (+ zoom / vibrancy / color space internals, tests)
src/platform/linux/{mod,app,dialog,hotkey,input,menu,native_clipboard,tabs,window}.rs
                                same files, same pub fn names; ← macos.rs `stub` + X11 grab from hotkey.rs
```

`objc.rs` is the only file without a Linux namesake; it holds no public API.

## Module Dependency Scan

Run at `d1d0c92` (before phases 1–3) with:

```bash
for f in $(git ls-files 'src/*.rs'); do
  printf '%s: %s\n' "$f" "$(grep -oE 'crate::[A-Za-z_][A-Za-z0-9_]*' "$f" | sort -u | sed 's/crate:://' | tr '\n' ' ')"
done
```

Layer assignment used below: **terminal** = grid, pty, search, history, bidi, graphics, shell_integration, x11colors; **render** = renderer, atlas, fontdb, postfx, image_layer (+ frame_metrics after Task 6); **platform** = macos, hotkey (+ phase-1 process, clipboard); **app** = everything else at the root. Allowed: terminal → terminal; render → render, terminal; platform → platform; app → anything; any layer → `test_alloc` from `#[cfg(test)]` code.

| # | From (layer) | To (layer) | What is used | Verdict | Phase 4 handling |
| --- | --- | --- | --- | --- | --- |
| V1 | `grid` (terminal) | `ai` (app) | `crate::ai::markdown::clean` in `Grid::extract_scrollback_lines` | violation | **Fixed in Task 2**: body moves to `terminal::text::strip_controls`; `ai::markdown` re-exports it as `clean`. |
| V2 | `pty` (terminal) | `config` (app) | `config::ShellIntegration`, `config::ShellFeatures` | violation | **Fixed in Task 2**: both plain enums/structs move into `terminal::shell_integration`; `config` re-exports them. |
| V3 | `shell_integration` (terminal) | `config` (app) | `config::ShellIntegration` | violation | **Fixed in Task 2** (same move). |
| V4 | `renderer` (render) | `frame_metrics` (root) | `frame_metrics::Capture` | violation | **Fixed in Task 6**: `frame_metrics.rs` moves into `render`. |
| V5 | `renderer` (render) | `split` (app) | `split::PaneRect` | violation | **Tolerated** (listed in `TOLERATED`): `split.rs` has zero crate dependencies, so no cycle. Follow-up: move `PaneRect` into `render`. |
| V6 | `renderer` (render) | `macos` (platform) | `configure_layer_drawables(&window)` inside `Renderer::new`, between `surface.configure` and `Atlas::new` | violation | **Tolerated**: moving the call to `app` would reorder GPU initialization. Path becomes `crate::platform::configure_layer_drawables` (Task 5). Follow-up: expose a post-surface hook. |
| V7 | `macos` (platform) | `config` (app) | `Config`, `ColorSpace` in `apply_macos_window_config` / `apply_color_space` | violation | **Tolerated** (signature change, not a move). Follow-up: a platform-owned `WindowLook` struct built by `app`. |
| V8 | `macos`, `hotkey` (platform) | `keymap` (app) | `Trigger`, `Action`, `mac_virtual_keycode`, `x11_keysym` | violation | **Tolerated**. Follow-up: platform takes a resolved key code + modifier mask. |
| V9 | `macos`, `hotkey` (platform) | `AppEvent` (app, `event.rs`) | `EventLoopProxy<AppEvent>` in reopen / menu / hotkey / socket handlers | violation | **Tolerated**. Follow-up: take a `Send` callback instead of a proxy. |
| V10 | `grid` tests, `image_layer` tests, `frame_metrics` tests (terminal / render) | `test_alloc` (root) | `allocations(..)` | test infra | Allowed by `TEST_INFRA`. |
| — | `grid` ↔ `search`, `grid` ↔ `bidi`, `grid` → `graphics`, `grid` → `x11colors`, `pty` → `grid`/`shell_integration` | terminal → terminal | — | ok | Intra-layer (cycles inside one layer are fine). |
| — | `atlas` → `grid` (`Cell`, `FG_BOLD`, `FG_ITALIC`), `atlas`/`image_layer` → `graphics`, `renderer` → `grid`/`history`/`search`/`bidi` | render → terminal | — | ok | — |
| — | `atlas` → `fontdb`, `renderer` → `atlas`/`postfx`/`image_layer` | render → render | — | ok | — |
| — | `config` → `atlas` (`MetricAdjust`, `parse_font_feature`), `config` → `grid`, `x11colors`, `update` → `show_notification`, `config` → `spawn_detached`, `ai` → `grid`/`tab`/`event`, `tab` → `renderer`/`pty`/`grid`/`split`, `bench` → `grid`/`history`/`pty`/`split`/`paste` | app → * | — | ok | — |

Module-level cycle that remains after phase 4 and is accepted: `render → platform (V6) → config (V7) → render (config → atlas::MetricAdjust)`. Removing V6 or V7 (both follow-ups) breaks it. Phases 2–3 may add edges not in this table; Task 0 re-runs the scan and the layering test reports any new one by file.

## Platform-Specific Code Inventory (cfg-gated items and their counterparts)

Locations are as of `d1d0c92`; `main.rs` lines live in `src/app/*.rs` after phase 1.

| Location | macOS | Linux counterpart | Status / Phase 4 action |
| --- | --- | --- | --- |
| `macos.rs` `imp` (26 pub fns) | real | `stub` (26 same-named fns, module-wide `#[allow(dead_code)]`) | Complete at the name level. Task 5 turns it into `platform::{macos,linux}` with an explicit re-export list. |
| `hotkey::set_global_hotkey` | no-op (`#[cfg(target_os = "macos")]`) | X11 grab | Duplicate API: `reload_config` calls **both** `macos::set_global_hotkey` and `hotkey::set_global_hotkey`. Task 5 collapses to one `platform::set_global_hotkey`. |
| `hotkey::listen` (control socket) | **missing** — `#[cfg_attr(target_os = "macos", allow(dead_code))]`, only called from a `#[cfg(not(target_os = "macos"))]` block | real | Task 5 adds `platform::listen_for_toggle`: macOS explicit no-op, Linux calls `control_socket::listen`. |
| `hotkey::socket_path` | `$TMPDIR/litty.sock` | `~/.cache/litty/litty.sock` | Pair exists; stays in `control_socket.rs`. |
| startup block (`main.rs:518-528`) | reopen handler, menu, Carbon hotkey | socket listen, X11 hotkey | Task 5 replaces both `cfg` blocks with four unconditional `platform::` calls (same order per platform). |
| `#[cfg(target_os = "macos")]` on calls to `release_free_memory`, `activate_app`, `set_secure_input`, `apply_macos_window_config`, `set_dock_icon`, `clipboard_write_string`, `clipboard_string`, `configure_layer_drawables` | real | no-op twin already exists | Redundant gates. Task 5 deletes them (Linux behavior identical: the twin is a no-op / returns `false` / `None`). |
| `refresh_tab_titles`: `set_window_represented_filename` block (`main.rs:1817`) | real | **implicit** (block skipped) | Keep the gate: the block also runs `p.cwd()` (a `/proc` readlink on Linux) every title refresh. Re-export gets `allow(unused_imports)` on Linux; Linux twin gets `allow(dead_code)`. |
| `create_window`: `WindowAttributesExtMacOS` (`main.rs:358`) | tabbing id + hidden | **implicit** (attrs unchanged) | Kept (winit macOS-only trait). Follow-up: `platform::prepare_window_attributes`. |
| `confirm_close` (`main.rs:2510-2517`) | `macos::confirm` | inline `zenity` returning **true** when zenity is missing | Divergent duplicate of `linux::confirm` (which returns **false** when zenity is missing). Kept as is (behavior); follow-up. |
| `copy_to_clipboard` / `paste_from_clipboard` command tables | `pbcopy` / `pbpaste` | `wl-copy`/`xclip`/`xsel`, `wl-paste`/… | Pair exists (phase 1 `platform/clipboard.rs`). |
| `open_target`, release-URL open, `show_notification`, `play_sound` | `open`, `osascript`, `afplay` | `xdg-open`, `notify-send`, `paplay`… | Pairs exist (phase 1 `platform/process.rs`). |
| `linux_open_command`, `safe_path_for_linux_open` | — | real, `#[cfg_attr(not(target_os = "linux"), allow(dead_code))]` | Linux-only helpers compiled on both for tests; fine. |
| `cmd_pressed` (`main.rs:1040`, `2333`) | `super_key()` | `control_key() \|\| super_key()` | Pair exists. |
| `main.rs:39` `#[global_allocator] mimalloc` | mimalloc | **system malloc** | Asymmetric by design (target-specific dependency). `release_free_memory` calls `mi_collect` only on macOS. Documented, not changed. |
| `grid` `macos_wcwidth`, `is_spacing_mark_range` (`grid/mod.rs:3567-3600`) | defers to libc `wcwidth_l` for spacing marks | **implicit** (unicode-width only) | No counterpart. Linux shells use glibc `wcwidth`, which may also disagree with unicode-width. Follow-up (ROADMAP). Moves with `grid` unchanged. |
| `pty` post-fork fd closing (`pty.rs:247-315`) | `/dev/fd` list, loop fallback | glibc `close_range` + loop fallback; BSD `closefrom`; generic loop for all others (incl. musl) | Complete. |
| `pty::pid_cwd` | `proc_pidinfo` | `/proc/<pid>/cwd` | Pair exists. |
| `graphics::is_temp_dir` | adds `/private/tmp`, `/private/var/tmp` | — | Data difference only. |
| `fontdb` alias step 3 (SF Mono), font dirs | macOS aliases + dirs | Linux dirs; **no alias step** | Implicit no-op is correct (no SF Mono on Linux). |
| `atlas` CoreText (`macos_coretext_rasterize`, `rasterize_color`, `rasterize_text_cluster`; private `cached_ct_base_font`, `coretext_line`) | CoreText | fontdue / Noto Color Emoji twins for the three entry points; `blit_fit` dead on macOS | Phase 3 splits into `atlas/coretext.rs` / `atlas/linux.rs`; Task 6 moves them unchanged. Check the two files export the same entry points (Task 6 Step 1). |
| `renderer` backends (`renderer.rs:206`), hint glyph `⌘`/`^` (`renderer.rs:1506`) | Metal, `⌘` | Vulkan, `^` | Pairs exist. |
| `update` app-bundle install, `codesign` (`update.rs:629-965`) | `.zip` bundle swap + ad-hoc sign | binary replace (`let _ = asset_name`) | Pair exists. |
| `config::open_config_file` (`config.rs:919`) | `open -t` | `xdg-open` | Pair exists. |
| `bench` RAM footprint (`bench.rs:102`) | `footprint` | RSS | Pair exists. |
| Linux twins that are behavior gaps (explicit no-ops) | — | `beep`, `show_progress_dialog`, `caps_lock` (= `false`, so Kitty never reports Caps Lock), `system_dark` (= `true`, no appearance detection) | Explicit, compile on both; listed as follow-ups. |
| macOS-only tests: `security_poc::poc_cwe22_macos_library_is_blocked`, `fontdb` ×2, `atlas` ×4, `grid` ×1, `direct_macos_clipboard_read_write`, `macos::tests` ×2 | — | none | Platform-specific behavior; no counterpart needed. |

## How to Move a Module (used by Tasks 1, 3, 6)

1. Re-list the sources and the `mod` lines:

   ```bash
   git ls-files src | sort
   grep -nE '^\s*(pub(\([a-z]+\))? )?mod [a-z_0-9]+;' src/main.rs src/platform/mod.rs
   ```

2. `git mv` each file / directory (commands in each task). `git mv a b c dir/` moves several files into an existing directory.
3. Declare the moved modules in the layer root (`pub(crate) mod x;`, alphabetical) and delete their `mod x;` lines from `src/main.rs`.
4. Rewrite absolute paths with the task's `perl -pi -e` line (portable between BSD and GNU, unlike `sed -i`).
5. Fix relative includes: `grep -rnE 'include_(str|bytes)!|include_wgsl!' <moved paths>`.
6. `cargo check 2>&1 | grep -E '^error' | sort | uniq -c`. The perl rewrite misses grouped imports such as `use crate::{config, grid, renderer};` (phase 1 `src/app/mod.rs` has one); split them by hand into `use crate::{config, …};` + `use crate::terminal::grid;`. Repeat until clean.
7. `cargo clippy --all-targets -- -D warnings`, delete every `use` it reports unused.
8. Stage and check rename detection:

   ```bash
   git add <new layer dir> src/main.rs
   git add $(git diff --name-only -- src)
   git status --short            # nothing outside src/ (and docs, if the task says so)
   git diff --cached -M --summary | grep rename
   ```

   Expected: one `rename` line per moved file, each ≥ 90%.

---

### Task 0: Preconditions and baseline

**Files:** none modified.

- [ ] **Step 1: Phases 1–3 are merged and the tree is clean**

```bash
git switch main && git pull --ff-only
git status --short
test -d src/app && test -f src/platform/mod.rs && test -f src/cli.rs && test -d src/grid \
  && test -f src/renderer/mod.rs && test -f src/atlas/mod.rs && echo phases-1-3-present
```

Expected: `git status` prints nothing; last line `phases-1-3-present`. If any path is missing, stop: phase 4 must not start before phases 1–3 land.

Other branches that touch moved files must be merged first or paused until Task 6 lands: at the time of writing `feat/ai-live-validation` (edits `src/main.rs`, `src/shell/litty.nu`, `scripts/linux/run.sh`) and `refactor/yo-shells-test-rs` (adds `tests/yo_shells.rs`, which only reads `src/shell/*` and is unaffected). `src/macos.rs` and `src/hotkey.rs` are split (not just renamed) in Tasks 4–5: concurrent edits to them will not merge automatically.

- [ ] **Step 2: Create the branch**

```bash
git switch -c refactor/layered-layout
```

- [ ] **Step 3: Record the baseline**

```bash
cargo test 2>&1 | grep -E '^test result' > "$TMPDIR/p4-baseline-tests.txt"
cat "$TMPDIR/p4-baseline-tests.txt"
git ls-files src | sort > "$TMPDIR/p4-files-before.txt"
```

Expected: every line `0 failed`. Sum the `passed` counts: call it `N`.

- [ ] **Step 4: Clean lint baseline on both targets**

```bash
cargo clippy --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu
cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
```

Expected: all succeed. The Linux Clippy run needs `rustup target add x86_64-unknown-linux-gnu` (already required by `CLAUDE.md`'s cross `cargo check`). If the baseline has warnings, fix them in a separate commit first.

- [ ] **Step 5: Re-run the dependency scan and diff it against the table above**

```bash
for f in $(git ls-files 'src/*.rs'); do
  printf '%s: %s\n' "$f" "$(grep -oE 'crate::[A-Za-z_][A-Za-z0-9_]*' "$f" | sort -u | sed 's/crate:://' | tr '\n' ' ')"
done > "$TMPDIR/p4-deps-before.txt"
grep -rn 'crate::{' src > "$TMPDIR/p4-grouped-imports.txt"
grep -E '^src/(grid|pty|search|history|bidi|graphics|shell_integration|x11colors|renderer|atlas|fontdb|postfx|image_layer|frame_metrics|macos|hotkey|platform)' "$TMPDIR/p4-deps-before.txt"
```

Expected: the only out-of-layer names are those in V1–V10. Write down any new one (phases 2–3 may have added some); the layering test in Tasks 2, 5 and 6 will flag it again and you decide there: fix it if it is a pure move, otherwise add it to `TOLERATED` with a reason and a `ROADMAP.md` follow-up.

---

### Task 1: Move the emulation core into `src/terminal/`

**Files:**
- Create: `src/terminal/mod.rs`
- Move: `src/grid/` → `src/terminal/grid/`; `src/{pty,search,history,bidi,graphics,shell_integration,x11colors}.rs` → `src/terminal/`
- Modify: `src/main.rs`; every file naming those modules (compiler-driven; expected `src/app/*.rs`, `src/config.rs`, `src/tab.rs`, `src/bench.rs`, `src/ai/mod.rs`, `src/security_poc.rs`, `src/renderer/**`, `src/atlas/**`, `src/image_layer.rs`, and the moved files themselves)

**Interfaces:**
- Produces: `crate::terminal::{grid, pty, search, history, bidi, graphics, shell_integration, x11colors}` — contents unchanged.

- [ ] **Step 1: Re-list**

```bash
ls src/grid
ls src/pty.rs src/search.rs src/history.rs src/bidi.rs src/graphics.rs src/shell_integration.rs src/x11colors.rs
```

Expected: the phase-2 grid submodules, and all seven files. If phase 2 added another root file that only the grid uses (check with `grep -rln 'crate::<name>' src`), move it here too.

- [ ] **Step 2: Move**

```bash
mkdir -p src/terminal
git mv src/grid src/terminal/grid
git mv src/pty.rs src/search.rs src/history.rs src/bidi.rs src/graphics.rs \
       src/shell_integration.rs src/x11colors.rs src/terminal/
```

- [ ] **Step 3: Layer root**

`src/terminal/mod.rs`:

```rust
//! Terminal emulation core: the VT parser and cell grid, the PTY, scrollback
//! search, shell history, bidi, inline graphics and shell integration.
//!
//! Layer rule (enforced by `crate::layering_tests`): nothing in here uses a
//! crate module outside `crate::terminal`.

pub(crate) mod bidi;
pub(crate) mod graphics;
pub(crate) mod grid;
pub(crate) mod history;
pub(crate) mod pty;
pub(crate) mod search;
pub(crate) mod shell_integration;
pub(crate) mod x11colors;
```

In `src/main.rs` delete `mod bidi;`, `mod graphics;`, `mod grid;`, `mod history;`, `mod pty;`, `mod search;`, `mod shell_integration;`, `mod x11colors;` and add `mod terminal;` (alphabetical, before `mod update;`).

- [ ] **Step 4: Rewrite paths**

```bash
perl -pi -e 's/\bcrate::(grid|pty|search|history|bidi|shell_integration|graphics|x11colors)\b/crate::terminal::$1/g' \
  $(git ls-files 'src/*.rs')
```

`\b` keeps `crate::search_ui` / `crate::history_insert_payload` untouched (`_` is a word character). Doc links such as ``[`crate::bidi::visual_order`]`` are rewritten too, which is wanted.

- [ ] **Step 5: Fix the embedded shell scripts' relative path**

`shell_integration.rs` is now one directory deeper than `src/shell/`:

```bash
perl -pi -e 's/include_str!\("shell\//include_str!("..\/shell\//' src/terminal/shell_integration.rs
grep -n 'include_str!' src/terminal/shell_integration.rs
```

Expected: six lines, each `include_str!("../shell/…")`. Its module doc ("the scripts under `src/shell/`") stays correct.

- [ ] **Step 6: Compile, fix grouped imports, prune** — follow "How to Move a Module" steps 6–7. Typical fix in `src/app/mod.rs`:

```rust
use crate::terminal::{grid, history, pty, search};
```

- [ ] **Step 7: Verify** — the six Global Constraints commands. Expected: all pass; test total == `N`.

```bash
git add src/terminal src/main.rs
git add $(git diff --name-only -- src)
git diff --cached -M --summary | grep -c rename
```

Expected: count == number of files under `src/terminal/` minus 1 (`mod.rs` is new), every line ≥ 90%.

- [ ] **Step 8: Commit**

```bash
git commit -m "refactor(terminal): move emulation core under src/terminal"
git log --follow --oneline -- src/terminal/pty.rs | sed -n 2p
```

Expected: the second line is a commit older than this one (history crosses the rename).

---

### Task 2: Guard the terminal layer and cut its two upward dependencies

**Files:**
- Create: `src/layering_tests.rs`, `src/terminal/text.rs`
- Modify: `src/main.rs`, `src/terminal/mod.rs`, `src/ai/markdown.rs`, the grid file holding `extract_scrollback_lines`, `src/config.rs`, `src/terminal/shell_integration.rs`, `src/terminal/pty.rs`

**Interfaces:**
- Produces: `pub fn crate::terminal::text::strip_controls(text: &str) -> String` (body of today's `ai::markdown::clean`, unchanged); `crate::terminal::shell_integration::{ShellIntegration, ShellFeatures}` (moved from `config`, which re-exports them so `crate::config::ShellIntegration` keeps compiling).
- Tests added: 2 (`crate_heads_reads_paths_and_nested_groups`, `terminal_layer_is_self_contained`).

- [ ] **Step 1: Write the layering test**

`src/layering_tests.rs`:

```rust
//! Layer rules for the source tree (plans/2026-11-01-layered-layout.md):
//!
//! - `terminal` uses no crate module outside itself;
//! - `render` uses only itself and `terminal`;
//! - `platform` uses no crate module outside itself.
//!
//! `app` and the crate-root modules may use anything. Known exceptions live in
//! `TOLERATED`, each with the reason it is still there.

use std::path::Path;

/// Crate-root modules any layer may use from `#[cfg(test)]` code.
const TEST_INFRA: &[&str] = &["test_alloc"];

/// `(layer, used crate module, why it is still allowed)`.
const TOLERATED: &[(&str, &str, &str)] = &[];

/// First path segment of every `crate::` path in `src`, including each
/// branch of a `use crate::{a, b::{c, d}}` group.
fn crate_heads(src: &str) -> Vec<String> {
    let mut heads = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find("crate::") {
        rest = &rest[i + "crate::".len()..];
        let Some(group) = rest.strip_prefix('{') else {
            let head: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            if !head.is_empty() {
                heads.push(head);
            }
            continue;
        };
        let (mut depth, mut want, mut cur) = (0usize, true, String::new());
        for c in group.chars() {
            if c.is_alphanumeric() || c == '_' {
                if want {
                    cur.push(c);
                }
                continue;
            }
            if want && !cur.is_empty() {
                heads.push(std::mem::take(&mut cur));
                want = false;
            }
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                ',' if depth == 0 => want = true,
                _ => {}
            }
        }
    }
    heads
}

/// Every `.rs` file under `src/<layer>/`, recursively, as `(path, contents)`.
fn layer_files(layer: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(layer)];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read source");
                out.push((path.display().to_string(), text));
            }
        }
    }
    assert!(!out.is_empty(), "no sources under src/{layer}");
    out
}

/// `path: crate::x` for every crate module a layer uses outside `allowed`.
fn violations(layer: &str, allowed: &[&str]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, text) in layer_files(layer) {
        for head in crate_heads(&text) {
            let ok = head == layer
                || allowed.contains(&head.as_str())
                || TEST_INFRA.contains(&head.as_str())
                || TOLERATED.iter().any(|(l, m, _)| *l == layer && *m == head);
            if !ok {
                bad.push(format!("{path}: crate::{head}"));
            }
        }
    }
    bad.sort();
    bad.dedup();
    bad
}

#[test]
fn crate_heads_reads_paths_and_nested_groups() {
    let src = "use crate::{terminal::{grid, pty}, render, AppEvent};\n/// [`crate::bidi::x`]\nlet c = crate::config::X;\nmacro_rules! m { () => { $crate::platform::beep() } }";
    assert_eq!(crate_heads(src), ["terminal", "render", "AppEvent", "bidi", "config", "platform"]);
}

#[test]
fn terminal_layer_is_self_contained() {
    let bad = violations("terminal", &[]);
    assert!(bad.is_empty(), "terminal must not depend on other layers:\n{}", bad.join("\n"));
}
```

In `src/main.rs`, next to `#[cfg(test)] mod security_poc;`, add:

```rust
#[cfg(test)]
mod layering_tests;
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test layering_tests`
Expected: `crate_heads_reads_paths_and_nested_groups ... ok`; `terminal_layer_is_self_contained ... FAILED` listing exactly (paths absolute; grid file name per phase 2):

```
…/src/terminal/grid/<file>.rs: crate::ai
…/src/terminal/pty.rs: crate::config
…/src/terminal/shell_integration.rs: crate::config
```

Any other line is an edge phases 2–3 introduced: fix it in this task if it is a pure move of a type/function into `terminal`, otherwise stop and ask.

- [ ] **Step 3: Move `clean` into the terminal layer (V1)**

Create `src/terminal/text.rs`, moving the body of `pub fn clean` from `src/ai/markdown.rs` verbatim:

```rust
//! Plain-text helpers shared by the terminal core and its consumers.

/// Drop control characters except `\n` and `\t`, so text taken out of the
/// grid (or produced by a model) never carries escape sequences.
pub fn strip_controls(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
```

Register it in `src/terminal/mod.rs` (`pub(crate) mod text;`, after `shell_integration`). In `src/ai/markdown.rs` replace the whole `pub fn clean(...) { ... }` with:

```rust
pub use crate::terminal::text::strip_controls as clean;
```

so `ai::markdown::clean(..)` callers in `src/app/` and `render()`'s internal `clean(text)` keep compiling. In the grid:

```bash
grep -rn 'ai::markdown::clean' src/terminal
```

and replace that call with `crate::terminal::text::strip_controls(&out)`.

- [ ] **Step 4: Move the shell-integration kinds into the terminal layer (V2, V3)**

Cut from `src/config.rs` (with their `#[derive(...)]` lines) and paste into `src/terminal/shell_integration.rs` directly below its `use` lines, unchanged:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellIntegration {
    Detect,
    None,
    Zsh,
    Bash,
    Fish,
    Nushell,
    Elvish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellFeatures {
    pub cursor: bool,
    pub sudo: bool,
    pub title: bool,
}
```

Delete `use crate::config::ShellIntegration;` from `shell_integration.rs`. In `src/config.rs`, where the two types were, add:

```rust
pub use crate::terminal::shell_integration::{ShellFeatures, ShellIntegration};
```

In `src/terminal/pty.rs`:

```bash
perl -pi -e 's/\bcrate::config::(ShellIntegration|ShellFeatures)\b/crate::terminal::shell_integration::$1/g' src/terminal/pty.rs
```

- [ ] **Step 5: Run the test again**

Run: `cargo test layering_tests`
Expected: 2 passed.

- [ ] **Step 6: Verify** — six commands; test total == `N + 2`. Call it `N2`.

- [ ] **Step 7: Commit**

```bash
git add src/layering_tests.rs src/terminal/text.rs src/terminal/mod.rs src/main.rs src/ai/markdown.rs \
        src/config.rs src/terminal/shell_integration.rs src/terminal/pty.rs
git add $(git diff --name-only -- src/terminal/grid)
git commit -m "refactor(terminal): cut terminal dependencies on config and ai"
```

Body (wrap at 72): explain the layer rule, that both moves are verbatim, and that `config` / `ai::markdown` re-export the moved items so app-tier callers are unchanged.

---

### Task 3: Move `macos.rs` and `hotkey.rs` under `src/platform/`

**Files:**
- Move: `src/macos.rs` → `src/platform/macos/mod.rs`; `src/hotkey.rs` → `src/platform/control_socket.rs`
- Modify: `src/platform/mod.rs`, `src/main.rs`, callers in `src/app/*.rs`, `src/renderer/**` (one call), `src/platform/clipboard.rs` (if it calls `macos::clipboard_*`)

**Interfaces:**
- Produces: `crate::platform::macos::*` (same 26 functions, still with the internal `imp` / `stub` split) and `crate::platform::control_socket::{socket_path, send_toggle, listen, set_global_hotkey}` — unchanged signatures.

- [ ] **Step 1: Re-list**

```bash
git ls-files src | grep -E 'macos|hotkey'
grep -rn 'macos::\|hotkey::' src --include='*.rs' | grep -v '^src/macos.rs\|^src/hotkey.rs' | grep -v 'winit::platform::macos' | wc -l
```

Expected: `src/macos.rs`, `src/hotkey.rs` (plus `src/atlas/coretext.rs`, which is not moved). Note the caller count.

- [ ] **Step 2: Move**

```bash
mkdir -p src/platform/macos
git mv src/macos.rs src/platform/macos/mod.rs
git mv src/hotkey.rs src/platform/control_socket.rs
```

- [ ] **Step 3: Declare**

`src/platform/mod.rs` becomes (keep the phase-1 doc comment):

```rust
pub(crate) mod clipboard;
pub(crate) mod control_socket;
pub(crate) mod macos;
pub(crate) mod process;
```

`macos` is **not** `cfg`-gated yet: the file still carries its own non-macOS `stub`. Delete `mod hotkey;` and `mod macos;` from `src/main.rs`.

- [ ] **Step 4: Fix the Dock icon include**

`src/platform/macos/mod.rs` is two directories deeper than `src/macos.rs`:

```bash
perl -pi -e 's{include_bytes!\("\.\./resources/icon\.png"\)}{include_bytes!("../../../resources/icon.png")}' src/platform/macos/mod.rs
grep -n 'include_bytes' src/platform/macos/mod.rs
```

Expected: `include_bytes!("../../../resources/icon.png")`.

- [ ] **Step 5: Rewrite paths**

```bash
perl -pi -e 's/\bcrate::macos\b/crate::platform::macos/g; s/\bcrate::hotkey\b/crate::platform::control_socket/g' \
  $(git ls-files 'src/*.rs')
```

Then bare `hotkey::…` call sites (resolved through the crate root or a `use crate::{…}` group) — at the time of writing `hotkey::listen`, `hotkey::set_global_hotkey` (×2) and `hotkey::send_toggle`:

```bash
grep -rn '\bhotkey::' src | grep -v '^src/platform/'
```

Replace each `hotkey::` with `control_socket::` and import `use crate::platform::control_socket;` in that file. For `macos::…` call sites, change the import (`use crate::{…, macos, …}` or `use crate::macos;`) to `use crate::platform::macos;`; the `macos::name(..)` calls stay as they are in this task. Do **not** touch `winit::platform::macos::…` paths.

- [ ] **Step 6: Compile, prune, verify** — "How to Move a Module" steps 6–8; six commands; test total == `N2`.

Expected rename summary:

```
rename src/{macos.rs => platform/macos/mod.rs} (99%)
rename src/{hotkey.rs => platform/control_socket.rs} (9x%)
```

- [ ] **Step 7: Commit**

```bash
git add src/platform src/main.rs
git add $(git diff --name-only -- src)
git commit -m "refactor(platform): move macos and hotkey modules under src/platform"
```

---

### Task 4: Split `platform/macos/mod.rs` by concern (macOS-only change)

**Files:**
- Create: `src/platform/macos/{objc,app,dialog,hotkey,input,menu,native_clipboard,tabs,window}.rs`
- Modify: `src/platform/macos/mod.rs`

**Interfaces:**
- Produces: the same 26 `pub fn`s, now re-exported from `platform::macos` by `pub use self::{app::*, …};`. No caller changes.
- Internal: `objc.rs` items are `pub(super)` (`Id`, `NSRect`, `NSPoint`, the objc FFI functions including `objc_msgSend`, `sel`, `class`, `send_id`, `responds_to`, `ns_string`, `ns_string_to_string`, `shared_application`, `for_each_in_array`, `ns_window_of`, and the `msg_send!` macro via `pub(super) use msg_send;`).

Line numbers below are from `src/macos.rs` at `d1d0c92`; phase 1–3 do not edit this file, but re-locate anyway:

```bash
grep -nE '^    (pub )?(unsafe )?(extern "C" )?(fn|static|const|struct|enum|type|macro_rules!|mod) |^    #\[link|^    unsafe extern' src/platform/macos/mod.rs
```

- [ ] **Step 1: Cut the `imp` body into files**

Everything inside `mod imp { … }` goes to exactly one file below; the `stub` module and the two `pub use` lines stay in `mod.rs` for now. De-indent moved code by 4 spaces (`perl -pi -e 's/^    //' <new file>` is safe here: the file has no multi-line string literals). Line 1298 (inside `test_saved_frames_cycle`) is whitespace-only: make it empty, or `git diff --check` fails on the moved lines.

| File | Items (≈ line in `src/macos.rs`) | Header (`//!`) |
| --- | --- | --- |
| `objc.rs` | `type Id` (21), `NSRect` (24), `NSPoint` (33), `#[link(name = "objc")]` block (39-50), `sel` (174), `class` (179), `macro_rules! msg_send` (186-198), `send_id` (200), `responds_to` (205), `ns_string` (209), `ns_string_to_string` (215), `shared_application` (254), `for_each_in_array` (259), `ns_window_of` (269) | `Objective-C runtime access: ids, selectors, message sends.` |
| `app.rs` | `malloc_zone_pressure_relief` block (52-54), `release_free_memory` (279), `REOPEN_PROXY` (637), `send_reopen` (640), `register_reopen_handler` (647), `observe_system_appearance` (683), `system_dark` (726), `hide_app` (934), `set_dock_icon` (998, contains the `include_bytes!`), `activate_app` (1033) | `App lifecycle: Dock reopen, activation, appearance, memory release.` |
| `hotkey.rs` | `EventTypeSpec` (56), `EventHotKeyId` (62), `type EventHandler` (68), a Carbon block with `GetApplicationEventTarget`, `InstallEventHandler`, `RegisterEventHotKey`, `UnregisterEventHotKey`, `HOTKEY` (95), `HOTKEY_HANDLER` (96), `set_global_hotkey` (100) | `Quick Terminal global hotkey through Carbon EventHotKey.` |
| `input.rs` | a second `#[link(name = "Carbon", kind = "framework")] unsafe extern "C"` block with only `EnableSecureEventInput` / `DisableSecureEventInput`, the `NSBeep` block (138-141), the `CGEventSourceFlagsState` block (143-146), `SECURE_INPUT_ACTIVE` (148), `set_secure_input` (152), `beep` (168), `caps_lock` (984) | `Keyboard: Secure Keyboard Entry, Caps Lock state, system beep.` |
| `native_clipboard.rs` | `clipboard_string` (224), `clipboard_write_string` (243) | `NSPasteboard string access (the subprocess fallback is platform::clipboard).` |
| `window.rs` | `use crate::config::{ColorSpace, Config};`, `configure_layer_drawables` (291), `apply_macos_window_config` (315), `cleanup_window` (366), `HOOK_INSTALLED` / `ORIGINAL_SEND_EVENT` / `ORIGINAL_ZOOM` / `SAVED_FRAMES` (373-377), `default_unzoomed_frame` (379), `toggle_window_zoom` (387), `is_in_window_header` (427), `custom_zoom` (452), `custom_send_event` (458), `install_header_double_click_hook` (496), `apply_color_space` (543), `apply_layer_colorspace` (569), `setup_visual_effect_view` (581), `remove_visual_effect_view` (621), `set_window_represented_filename` (1268), `#[cfg(test)] mod tests` (1278-1312) | `Window look and behavior: vibrancy, blur, color space, title-bar zoom, Metal drawables.` |
| `tabs.rs` | `add_tabbed_window` (944), `set_tabbing_allowed` (955), `select_native_tab` (965) | `Native window tabs.` |
| `menu.rs` | `use crate::keymap::Action;` (739), `enum Item` (745), `CMD` / `OPT` / `CTRL` (751-753), `MENU_ACTIONS` (756), `SECURE_ITEM` (777), `menu_tag` (779), `install_menu` (786), `set_secure_input_menu_state` (990) | `Application menu bar.` |
| `dialog.rs` | `confirm` (1044), `alert_choice` (1070), `alert` (1096), `ACTIVE_PROGRESS_WINDOW` / `PROGRESS_DELEGATE` (1100-1101), `progress_window_will_close` (1103), `progress_delegate` (1112), `show_progress_dialog` (1137), `close_progress_dialog` (1255) | `Modal alerts and the update progress window.` |

In `objc.rs`, make each item `pub(super)` (inside the `extern` block too: `pub(super) fn objc_msgSend();`) and add directly after the macro definition:

```rust
pub(super) use msg_send;
```

`msg_send!` names `Id`, `objc_msgSend` and `sel` unhygienically (they resolve at the call site), so every file that uses the macro starts with `use super::objc::*;`, which brings all four names into scope. Add that glob only to files that use something from `objc` (Clippy reports an unused glob in `input.rs`).

Each file also needs the `std` imports the moved code uses (`use std::ffi::{c_void, CStr};`, `use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};`, `use winit::window::Window;`, `use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};` — copy from the old `imp` header, then let Clippy prune).

Cross-file private uses the compiler will report (make exactly these `pub(super)`): `REOPEN_PROXY` in `app.rs` (used by `hotkey.rs`'s `on_hotkey` and by `menu.rs`'s `on_menu`). If it reports another, make that item `pub(super)` too and note it in the commit body.

- [ ] **Step 2: Rewrite `mod.rs`**

`src/platform/macos/mod.rs` (the `stub` module below stays byte-for-byte as it is, including its `#[cfg(not(target_os = "macos"))]` and `#[allow(dead_code)]`):

```rust
//! macOS-specific window vibrancy, blur, color space, Dock and app lifecycle
//! integration. Talks to the Objective-C runtime directly (no third-party crates).
//! Every function has a no-op twin on other platforms so callers need no `cfg`.

#[cfg(target_os = "macos")]
mod app;
#[cfg(target_os = "macos")]
mod dialog;
#[cfg(target_os = "macos")]
mod hotkey;
#[cfg(target_os = "macos")]
mod input;
#[cfg(target_os = "macos")]
mod menu;
#[cfg(target_os = "macos")]
mod native_clipboard;
#[cfg(target_os = "macos")]
mod objc;
#[cfg(target_os = "macos")]
mod tabs;
#[cfg(target_os = "macos")]
mod window;

#[cfg(target_os = "macos")]
pub use self::{app::*, dialog::*, hotkey::*, input::*, menu::*, native_clipboard::*, tabs::*, window::*};

#[cfg(not(target_os = "macos"))]
pub use stub::*;

// `mod stub { ... }` unchanged below.
```

- [ ] **Step 3: Verify** — six commands; test total == `N2`. `cargo test platform::macos` must still run `test_default_unzoomed_frame` and `test_saved_frames_cycle` (now `platform::macos::window::tests::…`).

```bash
grep -c '^pub fn' src/platform/macos/*.rs | awk -F: '{s+=$2} END {print s}'
```

Expected: `26`.

- [x] **Step 4: Manual smoke test (macOS, release build)** _(2026-11-12: checked by the user on macOS, all items pass.)_

`cargo run --release`, then: menu bar items (including Secure Keyboard Entry check mark), double-click the title bar (zoom / restore), change `background-opacity` / blur in the config and reload, Quick Terminal global hotkey, native tab switching, quit with a running program (confirm dialog), Cmd+Q, reopen from the Dock, toggle system dark/light mode, Cmd+C / Cmd+V.

- [ ] **Step 5: Commit**

```bash
git add src/platform/macos
git commit -m "refactor(macos): split platform module by concern"
```

Body: `git log --follow` stays on `mod.rs`; use `git blame -C -C -M <file>` to trace a split file's lines back to `src/macos.rs`.

---

### Task 5: Linux counterparts and one symmetric `platform::` API

**Files:**
- Create: `src/platform/linux/{mod,app,dialog,hotkey,input,menu,native_clipboard,tabs,window}.rs`
- Modify: `src/platform/mod.rs`, `src/platform/macos/mod.rs` (drop `stub` and per-line `cfg`s), `src/platform/macos/hotkey.rs` (add `listen_for_toggle`), `src/platform/macos/window.rs` (rename `apply_macos_window_config`), `src/platform/control_socket.rs` (remove the X11 grab and the macOS `set_global_hotkey` no-op), `src/platform/clipboard.rs`, `src/layering_tests.rs`, `src/main.rs` (CLI `send_toggle`), `src/app/*.rs`, `src/renderer/**`

**Interfaces:**
- Produces (re-exported by name from `crate::platform`; `macos` and `linux` define the same set):

| Function | macOS | Linux |
| --- | --- | --- |
| `release_free_memory()` | `mi_collect` + `malloc_zone_pressure_relief` | no-op |
| `configure_layer_drawables(window: &Window)` | 2 Metal drawables | no-op |
| `apply_window_config(window: &Window, cfg: &Config)` (renamed from `apply_macos_window_config`) | blur, vibrancy, color space, title-bar hook | no-op |
| `cleanup_window(window: &Window)` | forget saved frame / effect view | no-op |
| `register_reopen_handler(proxy: EventLoopProxy<AppEvent>)` | Dock reopen + appearance observer | no-op |
| `install_menu()` | NSMenu | no-op |
| `listen_for_toggle(proxy: EventLoopProxy<AppEvent>)` (new) | no-op | `control_socket::listen(proxy)` |
| `set_global_hotkey(trigger: Option<&Trigger>)` | Carbon | X11 grab |
| `send_toggle() -> io::Result<()>` | shared `control_socket::send_toggle` | same |
| `hide_app()`, `set_dock_icon()`, `activate_app()` | AppKit | no-op |
| `add_tabbed_window(parent, child)`, `set_tabbing_allowed(window, allowed)` | native tabs | no-op |
| `select_native_tab(window, step: isize, index: Option<usize>) -> bool` | native tabs | `false` |
| `caps_lock() -> bool` | `CGEventSourceFlagsState` | `false` |
| `system_dark() -> bool` | NSAppearance | `true` |
| `set_secure_input(on: bool)`, `set_secure_input_menu_state(on: bool)` | Carbon / menu | no-op |
| `beep()` | `NSBeep` | no-op |
| `show_progress_dialog(title, text)`, `close_progress_dialog()` | NSWindow | no-op |
| `alert_choice(title, text, buttons: &[&str]) -> usize`, `confirm(title, text, button) -> bool`, `alert(title, text)` | NSAlert | `zenity` |
| `clipboard_string() -> Option<String>`, `clipboard_write_string(s: &str) -> bool` | NSPasteboard | `None` / `false` |
| `set_window_represented_filename(window, path: &str)` | title-bar proxy icon | no-op |

- Tests added: 1 (`platform_layer_is_self_contained`).

- [ ] **Step 1: Create the Linux files from `stub` and the X11 grab**

Copy each `stub` function **verbatim** (same body, same `_`-prefixed parameters, full paths such as `winit::window::Window`) into the file of the same concern as its macOS twin:

| File | Functions from `stub` |
| --- | --- |
| `linux/app.rs` | `release_free_memory`, `register_reopen_handler`, `hide_app`, `system_dark`, `set_dock_icon`, `activate_app` |
| `linux/dialog.rs` | `show_progress_dialog`, `close_progress_dialog`, `alert_choice`, `confirm`, `alert` |
| `linux/input.rs` | `caps_lock`, `set_secure_input`, `beep` |
| `linux/menu.rs` | `install_menu`, `set_secure_input_menu_state` |
| `linux/native_clipboard.rs` | `clipboard_string`, `clipboard_write_string` |
| `linux/tabs.rs` | `add_tabbed_window`, `set_tabbing_allowed`, `select_native_tab` |
| `linux/window.rs` | `configure_layer_drawables`, `apply_macos_window_config` (renamed `apply_window_config`), `cleanup_window`, `set_window_represented_filename` |
| `linux/hotkey.rs` | — (see below) |

`stub` had a module-wide `#[allow(dead_code)]`; do **not** copy it. The only Linux twin still unused after Step 4 is `set_window_represented_filename`; give it a per-item attribute so any other twin that becomes unused is reported:

```rust
/// macOS shows the cwd as the title-bar proxy icon; Linux has no equivalent.
/// Called only from macOS-gated code in `app`; kept for API parity.
#[allow(dead_code)]
pub fn set_window_represented_filename(_window: &winit::window::Window, _path: &str) {}
```

`src/platform/linux/hotkey.rs`: move from `src/platform/control_socket.rs` the `#[cfg(not(target_os = "macos"))] pub fn set_global_hotkey` (with its doc comment) and the whole `#[cfg(not(target_os = "macos"))] mod x11 { … }`, drop those two `cfg` lines (the module is now Linux-only as a whole), and add:

```rust
//! Quick Terminal global hotkey on Linux.
//!
//! - X11: a passive key grab on the root window, served by a thread that
//!   waits for the key press.
//! - Wayland has no global hotkey API for clients, so the control socket
//!   (`platform::control_socket`) lets a desktop-environment shortcut run
//!   `litty +toggle-quick-terminal`. The socket works under X11 too.

use winit::event_loop::EventLoopProxy;

use crate::keymap::Trigger;
use crate::AppEvent;

/// Serve the control socket; it also provides the event proxy that X11
/// hotkey presses use, so call it before [`set_global_hotkey`].
pub fn listen_for_toggle(proxy: EventLoopProxy<AppEvent>) {
    crate::platform::control_socket::listen(proxy);
}
```

Inside `mod x11`, replace `super::toggle();` with `crate::platform::control_socket::toggle();`. In `control_socket.rs`, change `fn toggle()` to `pub(super) fn toggle()` (visible to `platform` and its descendants), delete the `#[cfg(target_os = "macos")] pub fn set_global_hotkey(_trigger: Option<&Trigger>) {}` twin, update the module doc (first two lines) to:

```rust
//! Control socket for `litty +toggle-quick-terminal` (Unix, both platforms).
//! The per-platform global hotkey lives in `platform::{macos,linux}::hotkey`.
```

and let Clippy prune `Trigger` from its imports. Keep `#[cfg_attr(target_os = "macos", allow(dead_code))]` on `listen` (macOS still never calls it).

`src/platform/linux/mod.rs`:

```rust
//! Linux (X11 / Wayland) side of the platform API. Every public function has a
//! namesake in `platform::macos`; most are explicit no-ops because the feature
//! is macOS-only (Dock, native tabs, menu bar, Secure Keyboard Entry).

mod app;
mod dialog;
mod hotkey;
mod input;
mod menu;
mod native_clipboard;
mod tabs;
mod window;

pub use self::{app::*, dialog::*, hotkey::*, input::*, menu::*, native_clipboard::*, tabs::*, window::*};
```

- [ ] **Step 2: macOS side**

In `src/platform/macos/mod.rs`: delete the `stub` module and the `#[cfg(not(target_os = "macos"))] pub use stub::*;` line; delete every per-line `#[cfg(target_os = "macos")]` (the whole module is gated next); change the third doc line to `//! The Linux twins of every public function live in \`platform::linux\`.` In `src/platform/macos/window.rs` rename `apply_macos_window_config` → `apply_window_config`. In `src/platform/macos/hotkey.rs` add:

```rust
/// The control socket is Linux-only: on macOS the Carbon hotkey and the Dock
/// cover the Quick Terminal, and `litty +toggle-quick-terminal` exits early.
pub fn listen_for_toggle(_proxy: winit::event_loop::EventLoopProxy<crate::AppEvent>) {}
```

- [ ] **Step 3: The API in `src/platform/mod.rs`**

```rust
pub(crate) mod clipboard;
pub(crate) mod control_socket;
pub(crate) mod process;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(target_os = "macos"))]
mod linux;
#[cfg(not(target_os = "macos"))]
use linux as imp;

// The cross-platform API, listed by name (never `imp::*`): a function missing
// on one platform fails that platform's build instead of silently vanishing.
pub(crate) use imp::{
    activate_app, add_tabbed_window, alert, alert_choice, apply_window_config, beep, caps_lock,
    cleanup_window, clipboard_string, clipboard_write_string, close_progress_dialog,
    configure_layer_drawables, confirm, hide_app, install_menu, listen_for_toggle,
    register_reopen_handler, release_free_memory, select_native_tab, set_dock_icon,
    set_global_hotkey, set_secure_input, set_secure_input_menu_state, set_tabbing_allowed,
    show_progress_dialog, system_dark,
};
// Only called from macOS-gated code (see the cfg inventory in
// plans/2026-11-01-layered-layout.md); the Linux twin keeps the API symmetric.
#[cfg_attr(not(target_os = "macos"), allow(unused_imports))]
pub(crate) use imp::set_window_represented_filename;
pub(crate) use control_socket::send_toggle;
```

- [ ] **Step 4: Point every caller at `platform::`**

```bash
files=$(git ls-files 'src/*.rs' | grep -vE '^src/platform/(macos|linux)/')
perl -pi -e 's/\bcrate::platform::macos::(\w+)/crate::platform::$1/g;
             s/\bsuper::macos::(\w+)/super::$1/g;
             s/(?<![:\w])macos::(\w+)/platform::$1/g;
             s/\bapply_macos_window_config\b/apply_window_config/g' $files
grep -rn 'winit::platform::platform' src && echo "BAD: winit path rewritten"
```

Expected: no `BAD` line (the look-behind keeps `winit::platform::macos::…` intact). Replace now-invalid imports `use crate::platform::macos;` with `use crate::platform;` (compiler-driven).

Then the four control-socket / hotkey call sites, by hand:

1. App startup (phase 1 put it in `App::resumed` / `new_events` in `src/app/mod.rs`; find it with `grep -rn 'register_reopen_handler' src/app`). Replace both `cfg` blocks:

   ```rust
   #[cfg(target_os = "macos")]
   {
       macos::register_reopen_handler(self.waker.proxy.clone());
       macos::install_menu();
       macos::set_global_hotkey(self.windows[0].config.quick_terminal_hotkey.as_ref());
   }
   #[cfg(not(target_os = "macos"))]
   {
       control_socket::listen(self.waker.proxy.clone());
       control_socket::set_global_hotkey(self.windows[0].config.quick_terminal_hotkey.as_ref());
   }
   ```

   (after Step 4's perl the first block reads `platform::…`) with:

   ```rust
   platform::register_reopen_handler(self.waker.proxy.clone());
   platform::install_menu();
   platform::listen_for_toggle(self.waker.proxy.clone());
   platform::set_global_hotkey(self.windows[0].config.quick_terminal_hotkey.as_ref());
   ```

   Same real calls in the same order on each platform (macOS: reopen, menu, hotkey; Linux: socket, hotkey).

2. `reload_config` (`grep -rn 'quick_terminal_hotkey != old' src/app`): the two lines calling `macos::set_global_hotkey` and `hotkey::set_global_hotkey` (one was always a no-op) become one:

   ```rust
   platform::set_global_hotkey(new.quick_terminal_hotkey.as_ref());
   ```

3. CLI in `src/main.rs`: `control_socket::send_toggle()` → `platform::send_toggle()`.

- [ ] **Step 5: Delete redundant call-site gates**

```bash
grep -rn -A1 'cfg(target_os = "macos")' src/app src/renderer src/platform/clipboard.rs | grep -B1 'platform::'
```

Delete `#[cfg(target_os = "macos")]` when the statement or block it gates contains **only** `platform::` calls whose Linux twin is a no-op (or returns `false` / `None`, which the surrounding code already handles). At the time of writing that is: every `release_free_memory()` (11 sites), every `activate_app()` (4), `set_secure_input(false)` in `Drop for State` and `hide_or_exit_last_window`, the `{ apply_window_config(&window, &cfg); set_dock_icon(); }` block in `create_window`, `apply_window_config` in the appearance and config-reload paths, the native-pasteboard-first blocks in `copy_to_clipboard` / `paste_from_clipboard`, and `configure_layer_drawables` in `Renderer::new`. For a gated `{ … }` block, keep the braces' content and drop the braces.

**Keep** these gates (not pure API calls):
- `WindowAttributesExtMacOS` in `create_window` (winit macOS-only trait);
- the `set_window_represented_filename` block in `refresh_tab_titles` (also runs `p.cwd()`);
- `confirm_close`'s `return platform::confirm(…)` and its Linux `zenity` fallback (different fallback result);
- the `cmd_pressed` pairs, the `cfg!(target_os = "macos")` expression in `hide_or_exit_last_window`, and every `#[cfg]` on tests.

- [ ] **Step 6: Guard the platform layer**

Append to `src/layering_tests.rs`:

```rust
#[test]
fn platform_layer_is_self_contained() {
    let bad = violations("platform", &[]);
    assert!(bad.is_empty(), "platform must not depend on other layers:\n{}", bad.join("\n"));
}
```

Run: `cargo test layering_tests`
Expected: `platform_layer_is_self_contained ... FAILED` listing only `crate::config`, `crate::keymap` and `crate::AppEvent` lines. Then fill `TOLERATED`:

```rust
const TOLERATED: &[(&str, &str, &str)] = &[
    ("platform", "config", "apply_window_config reads Config / ColorSpace; follow-up: a platform-owned WindowLook"),
    ("platform", "keymap", "hotkeys take keymap::Trigger, the menu dispatches keymap::Action; follow-up: pass key code + mods"),
    ("platform", "AppEvent", "reopen, menu, hotkey and socket handlers post AppEvent; follow-up: take a Send callback"),
];
```

Run again: 3 passed (`crate_heads_…`, `terminal_…`, `platform_…`). Any other reported edge (for example from phase-1 `process.rs`): fix if it is a pure move, otherwise stop and ask.

- [ ] **Step 7: Parity check**

```bash
diff <(ls src/platform/macos) <(ls src/platform/linux)
diff <(grep -rhoE '^pub fn [a-z_]+' src/platform/macos | sort) <(grep -rhoE '^pub fn [a-z_]+' src/platform/linux | sort)
```

Expected: first diff prints only `< objc.rs`; second prints nothing (27 names on each side).

- [ ] **Step 8: Verify** — six commands (the Linux Clippy run matters most here); test total == `N2 + 1`. Call it `N3`.

```bash
grep -rn 'cfg(target_os = "macos")' src/app | wc -l
```

Record the count in the commit body (it should drop by ≈ 20 from Task 4's tree).

- [ ] **Step 9: Linux runtime check**

```bash
bash scripts/linux/test.sh
```

Expected: native Linux `cargo check`, Clippy, `cargo test` pass and `PASS X11 Quick Terminal global hotkey, geometry, socket, session reuse, autohide`. It needs Docker or Podman; if neither is available, say so explicitly in the task report and in the PR description.

Repeat the Task 4 macOS smoke test, with extra attention to the Quick Terminal hotkey (startup and after editing `keybind = global:…` in the config) and the Dock reopen.

- [ ] **Step 10: Commit**

```bash
git add src/platform src/layering_tests.rs src/main.rs
git add $(git diff --name-only -- src)
git commit -m "refactor(platform): add linux twins and one symmetric platform api"
```

Body: the explicit re-export list makes parity a build error; `reload_config` no longer calls two hotkey setters; call-site `cfg`s removed only where the Linux twin is a no-op; tolerated `platform → config / keymap / AppEvent` edges.

---

### Task 6: Move GPU, glyph and font code into `src/render/`

**Files:**
- Create: `src/render/mod.rs`
- Move: `src/renderer/` → `src/render/renderer/`; `src/atlas/` → `src/render/atlas/`; `src/{fontdb,postfx,image_layer,frame_metrics}.rs` → `src/render/`; `*.wgsl` → `src/render/shaders/`
- Modify: `src/main.rs`, `src/layering_tests.rs`, callers (`src/app/*.rs`, `src/config.rs`, `src/tab.rs`, `src/bench.rs`, `src/event.rs` if it names the renderer)

**Interfaces:**
- Produces: `crate::render::{renderer, atlas, fontdb, postfx, image_layer, frame_metrics}` — contents unchanged.
- Tests added: 1 (`render_layer_uses_only_terminal`).

- [ ] **Step 1: Re-list**

```bash
ls src/renderer src/atlas
git ls-files '*.wgsl'
grep -rnE 'include_wgsl!|include_str!\("[^"]*\.wgsl' src
diff <(grep -hoE '^\s*pub(\(crate\))? (unsafe )?fn [a-z_]+' src/atlas/coretext.rs | sed 's/^ *//' | sort) \
     <(grep -hoE '^\s*pub(\(crate\))? (unsafe )?fn [a-z_]+' src/atlas/linux.rs | sed 's/^ *//' | sort)
```

Expected: phase-3 files; the two shaders (wherever phase 3 left them); their includes; and an empty last diff (phase 3's CoreText / Linux rasterizer files expose the same entry points). If the last diff is not empty, list the missing twins in the commit body and in `ROADMAP.md` (Task 8) — do not add code here.

- [ ] **Step 2: Move**

```bash
mkdir -p src/render/shaders
git mv src/renderer src/render/renderer
git mv src/atlas src/render/atlas
git mv src/fontdb.rs src/postfx.rs src/image_layer.rs src/frame_metrics.rs src/render/
git mv src/shader.wgsl src/image.wgsl src/render/shaders/
```

(Adjust the last line to the paths Step 1 printed.)

- [ ] **Step 3: Layer root**

`src/render/mod.rs`:

```rust
//! GPU rendering: wgpu pipelines and the single instanced draw call, the glyph
//! atlas and shaper, font discovery, post effects, inline images and frame
//! timing.
//!
//! Layer rule (enforced by `crate::layering_tests`): uses only `crate::render`
//! and `crate::terminal`, plus the tolerated edges listed there.

pub(crate) mod atlas;
pub(crate) mod fontdb;
pub(crate) mod frame_metrics;
pub(crate) mod image_layer;
pub(crate) mod postfx;
pub(crate) mod renderer;
```

In `src/main.rs` delete `mod atlas;`, `mod fontdb;`, `mod frame_metrics;`, `mod image_layer;`, `mod postfx;`, `mod renderer;`; add `mod render;` (alphabetical).

- [ ] **Step 4: Rewrite paths**

```bash
perl -pi -e 's/\bcrate::(renderer|atlas|fontdb|postfx|image_layer|frame_metrics)\b/crate::render::$1/g' \
  $(git ls-files 'src/*.rs')
```

- [ ] **Step 5: Shader includes**

Every file under `src/render/renderer/` includes `../shaders/shader.wgsl`; `src/render/image_layer.rs` includes `shaders/image.wgsl`:

```bash
perl -pi -e 's{(include_wgsl!|include_str!)\("(?:\.\./)*shader\.wgsl"\)}{$1("../shaders/shader.wgsl")}g' \
  $(git ls-files 'src/render/renderer/*.rs')
perl -pi -e 's{include_wgsl!\("image\.wgsl"\)}{include_wgsl!("shaders/image.wgsl")}' src/render/image_layer.rs
grep -rnE 'include_wgsl!|include_str!\("[^"]*\.wgsl' src
```

Expected: every include now names `shaders/…` relative to its file. A wrong path is a compile error; `test_shader_wgsl_syntax` additionally parses `shader.wgsl`.

- [ ] **Step 6: Compile, prune** — "How to Move a Module" steps 6–7.

- [ ] **Step 7: Guard the render layer**

Append to `src/layering_tests.rs`:

```rust
#[test]
fn render_layer_uses_only_terminal() {
    let bad = violations("render", &["terminal"]);
    assert!(bad.is_empty(), "render may only depend on terminal:\n{}", bad.join("\n"));
}
```

Run: `cargo test layering_tests`
Expected: FAILED listing only `crate::split` (renderer, `PaneRect`) and `crate::platform` (`configure_layer_drawables`). Add to `TOLERATED`:

```rust
    ("render", "split", "Renderer takes split::PaneRect; split.rs has no crate deps. Follow-up: move PaneRect into render"),
    ("render", "platform", "Renderer::new configures Metal drawables between surface setup and atlas creation"),
```

Run again: 4 passed. Any other edge from phase 3 (for example `crate::config` or `crate::keymap` in `renderer/ui`): fix if it is a pure move, otherwise stop and ask.

- [ ] **Step 8: Verify** — six commands; test total == `N3 + 1`. Call it `N4`. Rename summary: one `rename` per moved file, ≥ 90%.

- [ ] **Step 9: Commit**

```bash
git add src/render src/main.rs src/layering_tests.rs
git add $(git diff --name-only -- src)
git commit -m "refactor(render): move gpu, glyph and font code under src/render"
```

---

### Task 7: Make the security PoC source scan cover the whole tree

**Files:**
- Modify: `src/security_poc.rs` (`poc_dead_windows_shell_code_removed`)

`poc_dead_windows_shell_code_removed` reads `include_str!("main.rs")`. Since phase 1 the URL / file opener lives in `src/platform/process.rs` and `src/app/`, so the test still compiles but checks almost nothing. `include_str!("update.rs")` in `poc_cwe321_…` stays correct (`update.rs` did not move).

- [ ] **Step 1: Show the gap**

```bash
grep -n 'include_str!' src/security_poc.rs
wc -l src/main.rs
```

Expected: `include_str!("main.rs")` and `main.rs` around 120 lines (phase 1).

- [ ] **Step 2: Replace the test body**

```rust
#[test]
fn poc_dead_windows_shell_code_removed() {
    // Scan every source file: the opener code left main.rs in phase 1.
    let needle = concat!("\"/c\", ", "\"start\"");
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    let mut scanned = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).expect("read source");
                assert!(!src.contains(needle), "`cmd /c start` code is still present in {}", path.display());
                scanned += 1;
            }
        }
    }
    assert!(scanned > 50, "source scan found only {scanned} files");
}
```

The `concat!` keeps the needle out of this file's own text, as before.

- [ ] **Step 3: Verify** — six commands; test total == `N4` (one test changed, none added).

```bash
cargo test poc_dead_windows_shell_code_removed
```

Expected: 1 passed.

- [ ] **Step 4: Commit**

```bash
git add src/security_poc.rs
git commit -m "test(security): scan the whole source tree for removed shell code"
```

---

### Task 8: Docs, follow-ups and final verification

**Files:**
- Modify: `CLAUDE.md` (`AGENTS.md` and `GEMINI.md` are symlinks to it), `ROADMAP.md`

No other file hard-codes a moved path: `Cargo.toml` (`src/main.rs`), `scripts/bench-scroll.rs` (`../src/scroll.rs`), `scripts/test-yo-shells.py` and `tests/yo_shells.rs` (`src/shell/*`), `scripts/pre-release-check.sh` (`src/bench.rs`), `docs/release.md` (`src/update.rs`) all name files that stay. `README.md` has no source paths and no user-visible change. There is no `.github/` and no `build.rs`. `ROADMAP.md`'s `src/grid.rs`, `src/atlas.rs`, `src/macos.rs`, … mentions are in completed (`[x]`) audit entries: historical, leave them. `plans/yosh_implementation_plan.md` and `plans/2026-11-01-split-main-rs.md` are historical plans: leave them.

- [ ] **Step 1: Re-check for stale paths**

```bash
grep -rnE 'src/(grid|renderer|atlas|macos|hotkey|x11colors|pty|search|history|bidi|graphics|shell_integration|fontdb|postfx|image_layer|frame_metrics)\b|src/[a-z_]+\.wgsl' \
  --exclude-dir=.git --exclude-dir=target . | grep -vE '^\./(ROADMAP\.md|docs/)'
```

Expected: only `CLAUDE.md` lines (fixed next). Anything else (a script added since this plan was written): update it in this commit.

- [ ] **Step 2: `CLAUDE.md` → Architecture & Modules**

Replace the whole bullet list under `## Architecture & Modules` with:

```markdown
Layers (enforced by `src/layering_tests.rs`): `terminal` → nothing; `render` → `terminal`; `platform` → nothing; `app` and crate-root modules → anything. Exceptions are listed in `TOLERATED` with their reason.
- `src/main.rs`: crate root and CLI dispatch (`src/cli.rs`); starts `app::run`.
- `src/app/`: winit loop and per-window `State`; input (`keyboard`, `pointer`, `ime`), `tabs`, `actions`, `window_event` / `user_event` handlers.
- `src/keymap.rs`: Pure key-to-byte / shortcut resolution for `Platform::MacOs` & `Platform::Linux` + Kitty protocol.
- `src/split.rs`: Pane tree layout management.
- `src/terminal/`: `grid/` (12-byte `Cell`, ring buffer, `vte::Perform`; responses (DSR, DA) go to `Grid::response`), `pty.rs` (`openpty` + `fork`/`execve`, reader thread pushing bytes to grid), search, shell history, bidi, graphics, shell integration.
- `src/render/`: `renderer/` & `atlas/` (wgpu pipelines, single instanced draw call (`shaders/shader.wgsl`), glyph cache, font fallback), fontdb, postfx, image layer, frame metrics.
- `src/platform/`: one API (`platform::*`, listed by name in `platform/mod.rs`) implemented by `macos/` and `linux/` with identical file and function names; plus clipboard, detached processes, control socket.
```

Under `### Implement`, replace the **Platform code** bullet with:

```markdown
- **Platform code**: OS integration goes in `src/platform/{macos,linux}/` with the same `pub fn` in both (an explicit no-op is fine) and is exported by name from `src/platform/mod.rs`. Elsewhere, gate with `#[cfg(target_os = ...)]`; every platform-specific path needs a counterpart (or an explicit no-op) on the other platform so both targets compile.
```

- [ ] **Step 3: `ROADMAP.md`**

Add a section at the end:

```markdown
### Code Health — Layered Layout (Phase 4, 2026-11-01)

- [x] Phase 4 done: `src/terminal/`, `src/render/`, `src/platform/{macos,linux}/` with one symmetric platform API and a layering test (plan: `plans/2026-11-01-layered-layout.md`).
- [ ] Remove tolerated `render → split`: move `PaneRect` into `render`.
- [ ] Remove tolerated `render → platform`: let `Renderer::new` expose a post-surface hook for Metal drawable setup.
- [ ] Remove tolerated `platform → config`: pass a platform-owned `WindowLook` to `apply_window_config`.
- [ ] Remove tolerated `platform → keymap` / `AppEvent`: platform takes a resolved key code + modifier mask and a `Send` callback instead of `EventLoopProxy<AppEvent>`.
- [ ] Linux `confirm_close` fallback (`zenity` missing → close) differs from `platform::confirm` (missing → cancel); unify after deciding the intended behavior.
- [ ] `platform::prepare_window_attributes` for the `WindowAttributesExtMacOS` block in `app::window::create_window`.
- [ ] Linux behavior gaps behind explicit no-ops: `caps_lock` (Kitty never reports Caps Lock), `system_dark` (no appearance detection; e.g. xdg-desktop-portal `color-scheme`), `beep`, update progress dialog.
- [ ] Linux has no `wcwidth` override for spacing marks (`grid` uses `macos_wcwidth` only on macOS); check glibc `wcwidth` vs unicode-width for Indic scripts.
- [ ] Optional: lib target (see Task 9 of the phase 4 plan). Descoped from v0.13.0 on 2026-11-13: no consumer yet.
```

If Task 6 Step 1 found CoreText entry points without a Linux twin, add one line per missing function here.

- [ ] **Step 4: Full verification and layout check**

```bash
cargo check && cargo check --target x86_64-unknown-linux-gnu \
  && cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings \
  && cargo test && cargo clippy --all-targets -- -D warnings && git diff --check
ls src
git ls-files src | sort > "$TMPDIR/p4-files-after.txt"
diff "$TMPDIR/p4-files-before.txt" "$TMPDIR/p4-files-after.txt" | grep -c '^>'
```

Expected: all green; total test count == `N4` (== `N + 4`); `ls src` shows `ai app bench.rs cli.rs config.rs event.rs keymap.rs layering_tests.rs main.rs mouse.rs paste.rs platform render scroll.rs security_poc.rs shell split.rs tab.rs terminal test_alloc.rs update.rs`.

- [ ] **Step 5: Release smoke test**

`cargo build --release && ./target/release/litty`, then the Task 4 Step 4 list plus: `vim` (alt screen), wheel scroll, Cmd+F search, Ctrl+R history popup, an inline image (`kitten icat` or a Sixel file), split panes, `+bench-stress 200` (exits 0), `+list-themes`. Check idle RAM (< 50 MB) and idle CPU (0%) in Activity Monitor. On Linux, `bash scripts/linux/test.sh` (or state that it could not run).

- [ ] **Step 6: Commit**

```bash
git add CLAUDE.md ROADMAP.md
git commit -m "docs(arch): describe layered layout and platform api"
```

---

### Task 9 (optional, separate decision): introduce a lib target

**Recommendation: do not do this in phase 4.** Revisit when a concrete consumer exists (for example criterion benches in `benches/`, or an integration test that needs `Grid`).

Today nothing needs it: there is no `tests/` or `benches/` on `main`; `tests/yo_shells.rs` (branch `refactor/yo-shells-test-rs`) drives real shells and only reads `src/shell/*` through `CARGO_MANIFEST_DIR`; `scripts/bench-scroll.rs` reaches `src/scroll.rs` via `#[path]`, which works because `scroll.rs` has no crate dependencies outside `#[cfg(test)]`.

| For | Against |
| --- | --- |
| `tests/` and `benches/` can use internals (`Grid`, `keymap`, `Accumulator`) instead of `#[path]` hacks or a real PTY | Anything an integration test touches must become `pub`, and every type in those signatures too: large visibility churn |
| `src/main.rs` shrinks to `fn main() { litty::cli_main() }` | `dead_code` stops reporting unused `pub` items in a lib, weakening the zero-warning hygiene this repo relies on |
| Doc tests become possible | `#[cfg(test)]` items (`test_alloc`, test helpers) are invisible to `tests/`; unit tests must stay in the lib; `#[global_allocator]` placement must be re-checked (mimalloc stays in the bin, `test_alloc` in the lib's `cfg(test)`) |
| — | `cargo doc` warns about the lib/bin output-name collision (`litty`) unless the bin sets `doc = false` |

If chosen later, make it one commit `build(crate): add lib target behind thin binary`:

- [ ] Create `src/lib.rs` holding every `mod` declaration now in `src/main.rs` (same visibility; `pub mod bench` stays `pub`), the `#[cfg(test)] mod test_alloc;`, `#[cfg(test)] mod security_poc;`, `#[cfg(test)] mod layering_tests;`, `pub use event::{AppEvent, Waker};`, `pub use tab::{…};`, and `pub fn cli_main()` containing today's `fn main` body.
- [ ] Reduce `src/main.rs` to the mimalloc `#[global_allocator]` (`#[cfg(all(target_os = "macos", not(test)))]`) and `fn main() { litty::cli_main() }`.
- [ ] `Cargo.toml`: add `[lib] name = "litty" path = "src/lib.rs"` and `doc = false` under `[[bin]]`.
- [ ] Export only what a real consumer needs through `#[doc(hidden)] pub mod testing { pub use crate::…; }`; keep everything else `pub(crate)` so `dead_code` keeps working.
- [ ] Verify: six commands; `cargo test` total unchanged (all unit tests now run under the lib target); `cargo run -- +list-themes`.

---

## Risks

- **Phase 2–3 drift.** File names and edges may differ from this plan. Mitigation: every task re-lists first; the layering test reports unexpected edges by file; the decision rule (pure move → fix, otherwise stop and ask) is stated in Tasks 2, 5 and 6.
- **Concurrent branches.** Renames merge well when the other side edits a file whose similarity stays high; the `macos.rs` / `hotkey.rs` split (Tasks 4–5) does not. Merge or pause `feat/ai-live-validation` and any branch touching `src/macos.rs`, `src/hotkey.rs` or `src/main.rs` first (Task 0 Step 1).
- **Linux-only warnings.** macOS Clippy never compiles `platform/linux/` or `#[cfg(not(target_os = "macos"))]` code; Task 5 changes exactly that. Mitigation: `cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings` is part of every task's verification, and `bash scripts/linux/test.sh` runs the real Linux suite plus the X11 Quick Terminal smoke test.
- **Startup order of platform calls** (Task 5). The unified sequence calls no-ops in between, but the real calls keep their per-platform order; the Quick Terminal hotkey still depends on the proxy being registered first (`register_reopen_handler` on macOS, `listen_for_toggle` on Linux), which the order preserves. Verified by the macOS smoke test and the X11 smoke script.
- **`msg_send!` hygiene** (Task 4). The macro resolves `Id`, `objc_msgSend` and `sel` at the call site; every file using it must `use super::objc::*;`. A missing import is a compile error, not a runtime bug.
- **Split Carbon `extern` block.** Both halves need `#[link(name = "Carbon", kind = "framework")]`; duplicated link attributes are harmless.
- **Silent relative-path breakage.** `include_bytes!` / `include_wgsl!` / `include_str!` paths fail loudly at compile time, but `include_str!("main.rs")` in `security_poc` kept compiling while checking the wrong file — fixed in Task 7. Tests that resolve real files (`./Cargo.toml`, `./src/tab.rs`, `src/main.rs`) keep passing because none of those files move.
- **Rename detection.** Moves and splits are separate commits; path rewrites are a few lines per file, so similarity stays ≥ 90%. Verify with `git diff --cached -M --summary` before each commit. For split files use `git blame -C -C -M`.
- **Layering test brittleness.** It reads sources at test time through `CARGO_MANIFEST_DIR` (fine for `cargo test` in a checkout) and matches the text `crate::`, so a doc comment naming another layer counts as a dependency. That is intended: such doc links would break if the target moved.
