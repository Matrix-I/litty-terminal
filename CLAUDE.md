# Litty Terminal — Rules

Cross-platform, zero-latency GPU terminal emulator (macOS Metal / Linux Vulkan, Wayland, X11).

## Core Invariants & Safety
- **Performance**: < 50MB idle RAM (macOS `footprint`, app idle with all windows closed; open windows add GPU swapchain surfaces and are budgeted by `+bench-ram`), 0% idle CPU (`ControlFlow::Wait`), zero heap allocation on hot paths (PTY parsing, scrolling, render loop).
- **Grid Cell**: `Cell` struct must stay exactly 12 bytes (`static_assert`). Pack new state into existing bits.
- **Concurrency & Locking**: Minimize grid `Mutex` hold time. Never perform blocking I/O (PTY writes, subprocesses, file ops) while holding it.
- **Process & Security**:
  - Only async-signal-safe calls after `fork()`; prepare all allocations beforehand.
  - Strip `ESC` (`\x1b[201~`) from pasted text to prevent bracketed paste escapes.
  - User/terminal URLs to `open` / `xdg-open` must pass as a single argument (never via shell).
  - Shell history writes: mutate raw bytes, write temp file, atomic `rename`.

## Keybinding & Platform Parity
- **Modifier Rules**: macOS uses `Cmd`. Linux terminal actions (Copy, Paste, Tab, Quit) **MUST** use `Ctrl+Shift` (or `Super`) to avoid clashing with shell/readline control characters (`Ctrl+C/V/W/N/T`). Navigation/UI can use `Ctrl` or `Alt`.
- **Primary Bindings Matrix (macOS vs Linux)**:
  - Copy / Paste: `Cmd+C` / `Cmd+V` ↔ `Ctrl+Shift+C` / `Ctrl+Shift+V`
  - Tabs (New / Close): `Cmd+T` / `Cmd+W` ↔ `Ctrl+Shift+T` / `Ctrl+Shift+W`
  - Window (New / Quit): `Cmd+N` / `Cmd+Q` ↔ `Ctrl+Shift+N` / `Ctrl+Shift+Q`
  - Splits (V / H): `Cmd+D` / `Cmd+Shift+D` ↔ `Ctrl+Shift+D` / `Ctrl+Shift+E` (or `Super+Shift+D`)
  - Clear Scrollback: `Cmd+K` ↔ `Ctrl+Shift+K`
  - Quick Zoom: `Cmd + / - / 0` ↔ `Ctrl + / - / 0`
  - Tabs 1..9: `Cmd+1..9` ↔ `Ctrl+1..9` / `Alt+1..9`
  - Jump Word: `Opt+Left/Right` ↔ `Ctrl+Left/Right` (avoid macOS Mission Control collision)
  - Search / History: `Cmd+F` ↔ `Ctrl+Shift+F`; `Ctrl+R` fuzzy search (preserve vim redo in alt screen)
  - UI hints: macOS renders `⌘`, Linux renders `^` or `Alt+`.

## Architecture & Modules
Layers (enforced by `src/layering_tests.rs`): `terminal` → nothing; `render` → `terminal`; `platform` → nothing; `app` and crate-root modules → anything. Exceptions are listed in `TOLERATED` with their reason.
- `src/main.rs`: crate root and CLI dispatch (`src/cli.rs`); starts `app::run`.
- `src/app/`: winit loop and per-window `State`; input (`keyboard`, `pointer`, `ime`), `tabs`, `actions`, `window_event` / `user_event` handlers.
- `src/paste.rs`: pure paste / shell-quote payloads.
- `src/keymap.rs`: Pure key-to-byte / shortcut resolution for `Platform::MacOs` & `Platform::Linux` + Kitty protocol.
- `src/split.rs`: Pane tree layout management.
- `src/terminal/`: `grid/` (12-byte `Cell`, scrollback ring, `vte::Perform`; responses (DSR, DA) go to `Grid::response`), `pty.rs` (`openpty` + `fork`/`execve`, reader thread pushing bytes to grid), search, shell history, bidi, graphics, shell integration.
- `src/render/`: `renderer/` and `atlas/` (wgpu pipelines, one instanced cell draw per layer (`shaders/shader.wgsl`), glyph cache, CoreText / non-macOS rasterizers), fontdb, postfx, image layer, frame metrics.
- `src/platform/`: one API (`platform::*`, listed by name in `platform/mod.rs`) implemented by `macos/` and `linux/` with identical file and function names; plus clipboard, detached processes (`open` / `xdg-open`, notifications, sounds) and the control socket.

## Workflow & PR Checklist
### Implement
- **Scope**: Change only what the task needs. Note unrelated bugs or refactors as follow-ups (or in `ROADMAP.md`) instead of fixing them in passing.
- **Branching**: Non-trivial work (new feature, multi-file refactor, anything touching invariants above) goes on a branch `feat/<topic>`, `fix/<topic>`, `refactor/<topic>`. Small docs/typo fixes may go straight to `main`.
- **Tests**: Bug fixes start with a failing regression test that reproduces the bug. New logic in pure modules (`keymap`, `grid`, `split`, `search`, `history`, `config`) gets unit tests in the same file (`grid` keeps its tests in `src/terminal/grid/tests.rs`).
- **Platform code**: OS integration goes in `src/platform/{macos,linux}/` with the same `pub fn` in both (an explicit no-op is fine) and is exported by name from `src/platform/mod.rs`. Elsewhere, gate with `#[cfg(target_os = ...)]`; every platform-specific path needs a counterpart (or an explicit no-op) on the other platform so both targets compile.
- **Docs**: Update `ROADMAP.md` when a roadmap item is completed or descoped; update `README.md` when user-visible behavior, config keys, or keybindings change.
- **Done means verified**: Never report a task done without running the verification steps below and stating the result. If a step could not run (sandbox, missing toolchain), say so explicitly.

### Verification (run before every commit that touches `src/`)
1. All keybinding additions must update `src/keymap.rs` for both platforms with dedicated unit tests.
2. Verification steps:
   - `cargo check`
   - `cargo check --target x86_64-unknown-linux-gnu`
   - `cargo test` (run outside sandbox; PTY tests require `openpty`)
   - `cargo clippy --all-targets -- -D warnings` (zero warnings tolerated)
   - `git diff --check` (no trailing whitespace / conflict markers)

### Commit
- **Format**: Conventional Commits — `type(scope): summary` in imperative mood, lowercase, no trailing period, ≤ 72 chars. Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `build`. Scope is the module or area (`grid`, `keymap`, `render`, `pty`, `linux`, `macos`, `update`, `release`).
- **Body**: Explain *why* and any trade-off; reference roadmap items or CWE IDs where relevant. Wrap at 72 chars.
- **Granularity**: One logical change per commit; each commit must build and pass tests on its own. Don't mix formatting/refactors with behavior changes.
- **Never commit**: `target/`, `dist/` build outputs, benchmark scratch logs, secrets or signing keys, or unrelated working-tree changes (stage files explicitly, no `git add -A`).
- **Permission**: Commit only when asked. Never push, force-push, rewrite published history, tag, or create releases without explicit approval.

### Release
Releases are published via GitHub Releases. Follow `docs/release.md` (read it only when preparing a release).
