# Double-click Selection Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: executing-plans-strict.
**Goal:** Exclude square brackets and quotes from double-clicked tokens and traverse soft wraps.
**Architecture:** Reuse absolute selection coordinates and retained grid rows; no temporary flattened buffer.
**Tech Stack:** Rust, vte, existing Grid tests.
**Branch:** fix/double-click-selection

## Confirmed Requirements
- User examples: `[fix/macos-delegate-notifications]` selects its text without brackets.
- The quoted `/Users/linh.nguyen/Desktop/personal/litty-terminal/.claude/worktrees/check-and-resolve-16cdb6` selects without quotes, both on one row and across automatic wraps.
## Global Constraints
- Preserve 12-byte Cell, platform parity, short grid locks, no I/O during selection.
- No public API, config, dependency or keybinding changes. Preserve hard-newline boundaries and existing blank-cell and repeated-click behavior.
- Verify cargo check; cargo check --target x86_64-unknown-linux-gnu; cargo test outside sandbox; cargo clippy --all-targets -- -D warnings; git diff --check.
- Read full diff including untracked files, review correctness, concurrency, performance, error paths, security, naming, assumptions, tests and design. Independent review via code-review-strict.
- Commit only when asked. No push or release.
## Technical Defaults
- Use absolute coordinates because wrapped tokens can begin outside the viewport; use existing line_wrapped metadata to distinguish real newlines.
- Scope delimiters to square brackets and single/double quotes from the report; preserve other punctuation.
## Risks & Rollback
- Wrong history coordinates or hard-newline crossing: deterministic coverage of scrolled viewports and newline boundaries; revert this isolated diff. No persisted data/API/format changes. Platform-neutral grid implementation.

## Task 1 — Token boundaries and wrapped selection
**Files:** src/terminal/grid/selection.rs:284–327; src/terminal/grid/tests.rs:1481; README.md:27; src/app/window_event.rs:458 (review round 1).
**Interfaces:** Keep select_block(&mut self, row: usize, col: usize) -> String and is_block_selected(&self, row: usize, col: usize) -> bool. Replace private block_range with block_selection(&self, row: usize, col: usize) -> Option<Selection> using absolute coordinates. Only callers are select_block and is_block_selected.
- [ ] Add the following regression tests:
```rust
#[test]
fn double_click_excludes_brackets_and_quotes() {
    for (text, expected) in [
        ("[fix/macos-delegate-notifications]", "fix/macos-delegate-notifications"),
        ("'/Users/linh.nguyen/Desktop/personal/litty-terminal/.claude/worktrees/check-and-resolve-16cdb6'",
         "/Users/linh.nguyen/Desktop/personal/litty-terminal/.claude/worktrees/check-and-resolve-16cdb6"),
        ("\"./src/main.rs\"", "./src/main.rs"),
    ] {
        let mut g = Grid::new(120, 3);
        feed(&mut g, text);
        for col in 1..text.len() - 1 {
            assert_eq!(g.select_block(0, col), expected, "{text}, column {col}");
            assert!(g.is_block_selected(0, col));
        }
        assert!(!g.is_selected(0, 0));
        assert!(!g.is_selected(0, text.len() - 1));
    }
}

#[test]
fn double_click_selects_entire_soft_wrapped_path() {
    let path = "/Users/linh.nguyen/Desktop/personal/litty-terminal/.claude/worktrees/check-and-resolve-16cdb6";
    let text = format!("'{path}'");
    for width in [20, 50, 120] {
        let mut g = Grid::new(width, 8);
        feed(&mut g, &text);
        for offset in 1..text.len() - 1 {
            let (row, col) = (offset / width, offset % width);
            assert_eq!(g.select_block(row, col), path, "width {width}, offset {offset}");
            assert!(g.is_block_selected(row, col));
            assert_eq!(g.selected_text().as_deref(), Some(path));
        }
        assert!(!g.is_selected(0, 0));
        let last = text.len() - 1;
        assert!(!g.is_selected(last / width, last % width));
    }
}

```
- [ ] Run `cargo test double_click_` outside sandbox: expect quoted/bracketed content and wrapped-path tests to fail because delimiters are included and selection stops at the physical row.
- [ ] Replace the private block-range implementation and its two callers with:
```rust
    /// Absolute bounds of the token at `(row, col)`, across soft wraps only.
    fn block_selection(&self, row: usize, col: usize) -> Option<Selection> {
        if row >= self.rows + usize::from(self.scroll_fraction > 0.0) || col >= self.cols {
            return None;
        }
        let filled = |pos: (usize, usize)| {
            let Some(cells) = self.line_cells(pos.0) else { return false };
            let cp = cells[pos.1].codepoint;
            let wide_tail = pos.1 > 0 && cp == 0 && is_wide_cp(cells[pos.1 - 1].codepoint);
            wide_tail || (!Self::is_blank(cp) && !matches!(cp, 0x27 | 0x22 | 0x5b | 0x5d))
        };
        let pos = (self.abs_line(row), col);
        if !filled(pos) {
            return None;
        }
        let mut start = pos;
        loop {
            let previous = if start.1 > 0 {
                (start.0, start.1 - 1)
            } else if start.0 > 0 && self.line_wrapped(start.0 - 1) {
                (start.0 - 1, self.cols - 1)
            } else {
                break;
            };
            if !filled(previous) { break; }
            start = previous;
        }
        let mut end = pos;
        loop {
            let next = if end.1 + 1 < self.cols {
                (end.0, end.1 + 1)
            } else if self.line_wrapped(end.0) {
                (end.0 + 1, 0)
            } else {
                break;
            };
            if !filled(next) { break; }
            end = next;
        }
        Some(Selection { start, end })
    }

    /// Double-click selects a token without surrounding brackets or quotes,
    /// including continuations on soft-wrapped rows. Blank cells and delimiters
    /// retain the smart-line fallback.
    pub fn select_block(&mut self, row: usize, col: usize) -> String {
        let Some(selection) = self.block_selection(row, col) else {
            return self.select_smart_line(row, col);
        };
        if self.selection != Some(selection) {
            self.selection = Some(selection);
            self.all_dirty = true;
        }
        self.selected_text().unwrap_or_default()
    }

    /// Whether the selection is exactly the token at `(row, col)`, so a further
    /// double-click there widens it to the line.
    pub fn is_block_selected(&self, row: usize, col: usize) -> bool {
        let (Some(sel), Some(block)) = (self.selection, self.block_selection(row, col)) else {
            return false;
        };
        sel.ordered() == block.ordered()
    }

```
- [ ] Add boundary tests for hard newlines, scrollback, fractional viewport, wide cells, and invalid coordinates. Preserve existing block/line tests.
- [ ] Add README bullet explaining quote/bracket exclusion and automatic-wrap traversal.
- [ ] Run `cargo test double_click_`: expect all matching tests PASS; inspect selected coordinates and selected_text for the original examples.
- [ ] Final verification: run all required commands above; review entire diff and untracked files; apply code-review-strict, address findings and repeat gates after any edit. Exercise GUI if available; otherwise report parser-to-selection tests as evidence and desktop verification as unverified.
**Ready to commit:** selection.rs, tests.rs, window_event.rs, README.md, this plan — `fix(grid): select quoted tokens across soft wraps` (no commit authorized).

## Review
One task; no persisted format/API/invariant changes, so independent plan dispatch is not required. Self-review: each user example is an explicit failing test; absolute bounds match storage and callers; only private interface changes; rollback is local.

## Execution notes
- Reproduction on macOS, commit 7794644: `cargo test double_click_` failed both original-example tests (1 passed, 2 failed).
- Additional failing boundary test found NUL padding before a wrapped wide glyph: select_block returned 中x instead of ab中x. Extend the local selection implementation to recognize that padding and omit it during text extraction. Explicit spaces remain boundaries and are retained when copied. This is necessary for the planned wide-cell coverage; public signatures remain unchanged.
- User chose Ghostty's full default `selection-word-chars` set (space, tab, ' " │ ` | : ; , ( ) [ ] { } < > $), verified against Ghostty's src/config/Config.zig. Consequence: `https://x` selects `//x`, `file.rs:42` splits at `:`, as in Ghostty.
- Review round 1: a second double-click on a soft-wrapped token widened to one physical row and shrank the selection. Added `Grid::widen_block`, used by the double-click handler: tokens spanning soft wraps stay selected; single-row tokens still widen to the smart line.
- Round 1 adds crate-internal `Grid::widen_block(&mut self, row, col) -> String`; no external API, config or format change.
