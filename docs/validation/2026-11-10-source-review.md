# Source review — 2026-11-10

Reviewed at commit `442534c` (v0.11.0). No source was changed by this review.

## Scope

Read: `terminal/pty.rs`, `update/*`, `terminal/grid/{mod,perform,control,storage,links,prompt}.rs`,
`terminal/graphics/kitty.rs` (file and shared-memory reads), `paste.rs`,
`platform/{process,clipboard}.rs`, `app/terminal_io.rs`, and the paste and click
paths in `app/actions.rs` and `app/window_event.rs`.

Not reviewed: `render/`, `platform/{macos,linux}/`, most of `app/`, `config.rs`,
`ai/`, `keymap.rs`, `split.rs`.

Baseline: `cargo test` passed (541 + 1 + 14, 12 ignored) on Linux x86_64.
`cargo clippy` and the cross-target `cargo check` were not run.

## Method

- Manual reading of the files above.
- Temporary grid fuzz test (removed afterwards): random escape sequences, OSC,
  DCS, Kitty APC, resizes and UI reads (selection, search text, links, prompt
  helpers) in a debug build, 9,000 seeds.
- Temporary decoder fuzz (removed afterwards): PNG, zlib, sixel, JSON, config
  and bidi, 20,000 iterations. No panics.
- Search regex engine checked against common catastrophic-backtracking
  patterns. All returned in under 1 ms.

## Confirmed bugs

Each was reproduced with a throwaway test.

### 1. Click-to-move-cursor can crash the app

- Where: `src/terminal/grid/prompt.rs:315`, `calculate_cursor_movement_to`.
- Cause: `display_row(r)[begin..end]` is sliced with `begin > end` when the
  OSC 133;B input mark is past the last non-blank cell of the first input row
  and the cursor is on a later row.
- Reproducer: `Grid::new(20, 5)`, feed `"$ \x1b]133;B\x07\n"`, call
  `calculate_cursor_movement_to(0, 0)`. Panics with "slice index starts at 2
  but ends at 1".
- Reach: a plain single click (`src/app/window_event.rs:516`).
- Impact: release builds use `panic = "abort"`, so all tabs and windows die.
- Fuzz: 21 of 3,000 seeds.

### 2. `ESC c` (RIS) blanks scrollback but keeps its length

- Where: `src/terminal/grid/mod.rs:938`, `full_reset`.
- Cause: it calls `release_scrollback_pages()` and clears the cluster table
  without resetting `history`.
- Observed on Linux: after 3,000 lines and `ESC c`, `history` stayed 2,977 and
  2,969 of those lines were blank.
- macOS: the same path uses `MADV_FREE`, so content is likely indeterminate.
  Not tested.
- Reference: `CSI 3 J` zeroes `history` before releasing pages.

### 3. Window titles are cut at the first `;`

- Where: `src/terminal/grid/perform.rs`, OSC 0 / 2.
- Cause: only `params[1]` is used; vte splits the payload on `;`.
- Reproducer: `OSC 0;make; make test BEL` gives the title `make`.
- Reference: OSC 9, 777 and 8 rejoin the parameters (`lossy_join`).

### 4. Cursor escapes the scroll region; CPR reports a garbage row

- Where: `src/terminal/grid/perform.rs:283` (`CSI 6 n`), and `CSI A/B/E/F`.
- Cause: cursor up/down ignore the margins, so in origin mode the cursor can
  sit above `scroll_top`; the report then computes `cur.row - scroll_top`.
- Reproducer: `CSI 5;10r`, `CSI ?6h`, `CSI 3A`, `CSI 6n`. The cursor is on
  row 1 with the top margin on row 4.
- Impact: panic in debug builds. In release the subtraction wraps and the
  reported row is near 1.8e19 (release behaviour inferred, not run).

## Lower severity (from reading, not reproduced)

- **OSC 8 `file://` links** (`grid/links.rs:383`, `platform/process.rs`): the
  host is ignored and the path is opened with the default handler. Only
  executables, `.app` and `.command` are downgraded to "reveal". `.webloc`,
  `.inetloc`, `.terminal`, `.jar` and `.pkg` still open, which may get around
  the scheme allowlist.
- **Paste sends `\n` unchanged** (`paste.rs`): most terminals convert LF to
  CR. Raw-mode programs without bracketed paste receive Ctrl+J, and CRLF text
  sends both.
- **Update signature does not bind the version** (`update/`): the signature
  covers `SHA256SUMS.txt` only, so an older signed release could be served
  under a newer tag. Needs control of the GitHub responses.
- **macOS bundle update is not atomic** (`update/install.rs:294`): `ditto`
  merges over the live bundle, so stale files remain and a failure midway
  leaves a mixed bundle.
- **Failed `spawn_reader` blocks up to 2 s** (`terminal/pty.rs:477`): it calls
  `reap_child` while the master is still open, then SIGKILLs the shell.
- **Clipboard copies dropped silently** when two are already in flight
  (`platform/clipboard.rs`).
- **Kitty `t=f` reveals file existence** outside the deny-list: the reply is
  `OK` versus `EPERM`.
- **Misplaced doc comment** (`paste.rs`): the text for `paste_payload` is
  attached to `shell_quote`.

## Checked and found sound

- Grid: no other panics in 9,000 fuzz seeds.
- PNG, zlib, sixel, JSON, config and bidi parsers: no panics in 20,000
  iterations of light fuzzing.
- Search regex: no catastrophic backtracking on the patterns tried.
- Bracketed paste: ESC and C1 CSI are stripped; the terminator cannot be
  injected.
- OSC 52: write-only, size-bounded, cancelled atomically, and gated on the
  focused pane and `clipboard-write`.
- Child spawn: only async-signal-safe calls after `fork()`; inherited
  descriptors are closed.

## Suggested order

1. Bug 1 (crash from a click).
2. Bug 2 (scrollback loss on reset).
3. Bugs 3 and 4.
4. The OSC 8 `file://` handling.
