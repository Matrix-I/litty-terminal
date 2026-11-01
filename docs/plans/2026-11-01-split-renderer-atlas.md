# Split `renderer.rs` and `atlas.rs` — Implementation Plan (Phase 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Break the 2270-line `src/renderer.rs` (a 505-line `render_panes`, five overlay builders that take `&mut Renderer`) and the 2904-line `src/atlas.rs` (glyph cache, ligature shaper, ~770 lines of procedural sprites, CoreText FFI with duplicated declarations, font discovery, tests) into `src/renderer/` and `src/atlas/` module directories, with zero behavior, timing or allocation change.

**Architecture:** Phase 3 of the 4-phase structural refactor outlined at the end of `docs/plans/2026-11-01-split-main-rs.md`. Atlas first (pure leaf moves, then the cfg-sensitive platform split and FFI dedupe), then renderer: directory + tests, wgpu object creation, overlay widgets as free functions over a small borrowed `Cells` context (no `&mut Renderer`), the per-pane row builder with narrowed borrows, and finally `render_panes` split into named frame phases. Every commit compiles and passes clippy on macOS and `x86_64-unknown-linux-gnu`, and passes the full test suite.

**Tech Stack:** Rust edition 2024, wgpu 24 (Metal on macOS, Vulkan elsewhere), fontdue 0.9, rustybuzz 0.20, memmap2; CoreText / CoreGraphics / CoreFoundation FFI on macOS; cargo targets `aarch64-apple-darwin` and `x86_64-unknown-linux-gnu`.

## Prerequisites

- **Phase 1 is merged** (`docs/plans/2026-11-01-split-main-rs.md`). After it, the external callers of these two files live in `src/app/*.rs` instead of `src/main.rs`; this plan finds them with `grep`, never by path.
- **Phase 2 (`grid/`) is independent.** The renderer only uses grid items re-exported from `crate::grid` (`Grid`, `Hyperlink`, `SearchHit`, `char_width`, `BG_*`, `FG_*`, `UNDERLINE_SINGLE`, `PH_FLAG`, `CLUSTER_FLAG`, `is_wide_cp`) and the `Grid` fields `scroll_fraction`, `dirty`, `all_dirty`, `viewport_dirty`, `cols`, `rows`, `cursor_shape`, `progress`. If Phase 2 lands while this branch is open, rebase; conflicts are limited to those `use` lines and field accesses.
- Line numbers below are from commit `d1d0c92` (`main` at the time of writing). Phase 1 does not edit `renderer.rs` / `atlas.rs`, but every move **must re-locate items with grep** (see "How to Move an Item"); numbers shift after every cut.

## Global Constraints

- Pure move / rename refactor: no behavior, timing, allocation, lock-scope or draw-order change. If a task tempts you to "fix" something, write it in `ROADMAP.md` (Task 19 lists the known ones) instead.
- Render loop (`render_panes` and everything it calls) must not gain heap allocations, `dyn` dispatch, `Box`, extra locking, or extra `queue.write_buffer` / draw calls. Pre-existing allocations in overlays stay exactly as they are (they are listed in Task 19, not fixed here).
- The cell pipeline keeps **one instanced cell draw per layer** (`draw(0..4, 0..instance_count)`, two only when an image has `z < 0`). Do not reorder pass commands.
- Lock scope: each pane's grid is locked exactly where it is today (once in the fast-path check for non-active panes, once for the active pane's state, once per pane in the build loop, once per pane in the eviction rebuild). Never split one lock into two or hold one across a phase boundary.
- Atlas slot request order must not change (tab bar → panes → eviction rebuild → link preview → preedit → find box → history popup → inspector), because slot allocation order decides LRU eviction.
- Every platform-specific item keeps its `#[cfg(...)]` / `#[cfg_attr(...)]` verbatim; every macOS-only file has a non-macOS counterpart so both targets compile (`CLAUDE.md`).
- Moved items keep the `///` doc comments and `#[...]` attributes directly above them.
- `const _: () = assert!(size_of::<Instance>() == 12)` and `size_of::<Uniforms>() == 128` stay next to their structs.
- Visibility rules (Rust privacy is module-tree based: a child module sees its ancestors' private items and fields, a parent does not see its children's private items):
  - Types stay private in `renderer/mod.rs` / `atlas/mod.rs`; submodules use them through `super::`.
  - Functions in a direct child that the parent or a sibling calls become `pub(super)`.
  - Items in `renderer/ui/*.rs` (grandchildren) become `pub(in crate::renderer)` so `renderer` and all its descendants can use them (Phase 4 will rewrite this path when `renderer/` moves).
  - The crate-facing API (`crate::renderer::*`, `crate::atlas::*` paths used outside the module) does not change: items that move get a `pub use` / `pub(crate) use` re-export in `mod.rs`.
- Verification before every commit — the five `CLAUDE.md` commands plus a Linux clippy pass (the atlas/renderer split is cfg-heavy, and `cargo check --target ...` does not compile Linux tests). The sixth command was confirmed to work on `d1d0c92` (≈30 s, clean):
  1. `cargo check`
  2. `cargo check --target x86_64-unknown-linux-gnu`
  3. `cargo test` (outside the sandbox; PTY tests need `openpty`)
  4. `cargo clippy --all-targets -- -D warnings`
  5. `cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings`
  6. `git diff --check`

  Below, "six commands" means this list.
- Test count must equal the Task 0 baseline `M` after every task, except where a task adds tests (it says so and gives the new total).
- Commit format: `refactor(<scope>): <summary>`, scope `atlas` or `render`, lowercase, imperative, ≤ 72 chars. Stage files explicitly; never `git add -A`. One task = one commit.

## Target File Structure

```
src/atlas/mod.rs          MetricAdjust, set_font_tweaks, slot-key constants (BLANK_SLOT, WIDE_*, STYLE_*,
                          CODEPOINT_MASK, CLUSTER_KEY, GLYPH_PIECE, SHAPED_BLANK), GlyphStyle, FontSet,
                          Atlas struct + cache / LRU / slot API (new, grow_if_needed, glyph_slots, slot,
                          cluster_slots, shape_row, ...), is_emoji, is_emoji_text, blit_fit;
                          re-exports load_font, parse_font_feature, set_font_features      (~730 lines)
src/atlas/raster.rs       impl Atlas: rasterize*, apply_style, upload*; rasterize_with_font, stem_boost (~390)
src/atlas/sprites.rs      procedural Powerline / box drawing / blocks / braille / nerd-icon fallback  (~775)
src/atlas/shaper.rs       GlyphPiece, Shaper (rustybuzz ligatures), FONT_FEATURES, parse/set_font_features (~145)
src/atlas/fonts.rs        FONT_CACHE, load_font, shaper_mmap_for, parse_font, symbol / Nerd Font discovery (~235)
src/atlas/coretext.rs     #[cfg(target_os = "macos")]: CF/CG/CT FFI declared once, CFRange/CGPoint/CGSize/CGRect
                          declared once, cached_ct_base_font, macos_coretext_rasterize, coretext_line,
                          rasterize_color, rasterize_text_cluster                               (~400)
src/atlas/linux.rs        #[cfg(not(target_os = "macos"))]: macos_coretext_rasterize / rasterize_text_cluster
                          stubs (no system fallback), rasterize_color via Noto Color Emoji CBDT PNG  (~50)
src/atlas/tests.rs        #[cfg(test)] tests moved from atlas.rs                                    (~180)

src/renderer/mod.rs       RenderPane, Instance, Uniforms, Viewport, UNFOCUSED, COLOR_GLYPH, Renderer struct,
                          Renderer::new, frame capture, TabBarAction, public geometry / setter API,
                          history_popup_geometry wrapper; re-exports SCROLLBAR_WIDTH, scrollbar_thumb (~640)
src/renderer/gpu.rs       global wgpu Instance, shared adapter/device/queue, cached cell pipeline,
                          create_cells_pipeline, make_bind_group, make_instance_buf, make_viewport_buffer (~220)
src/renderer/frame.rs     render_panes orchestrator + phases: FrameLayout, ActivePane, FrameCtx,
                          OverlayInputs, collect_pane_instances, rebuild_after_eviction, collect_overlays,
                          upload_instances, frame_uniforms, submit                               (~500)
src/renderer/pane_row.rs  PaneRowCtx, RowScratch, build_pane_row, Renderer::row_target             (~190)
src/renderer/ui/mod.rs    Cells (borrowed atlas + queue + instances + cols), DirtyRows, Renderer::cells (~70)
src/renderer/ui/tab_bar.rs        build, tab_label (+ test)                                         (~110)
src/renderer/ui/search_box.rs     geometry, build (+ test)                                          (~150)
src/renderer/ui/inspector.rs      build                                                             (~80)
src/renderer/ui/history_popup.rs  geometry, build (+ test)                                          (~350)
src/renderer/ui/link_preview.rs   build (hovered URL on the last row)                               (~50)
src/renderer/ui/preedit.rs        build (IME composition at the cursor)                             (~45)
src/renderer/ui/scrollbar.rs      SCROLLBAR_OVERLAY, SCROLLBAR_WIDTH, scrollbar_thumb, instance (+ tests) (~60)
src/renderer/tests.rs     precise_scroll_gpu_pixels, test_shader_wgsl_syntax, test_tab_bar_action_variants (~140)
```

`src/shader.wgsl` does not move (Phase 4 moves all `*.wgsl`); the include paths become `"../shader.wgsl"`.

### Why the builders can borrow narrowly

Every overlay builder (`build_tab_bar_row`, `build_search_box`, `build_inspector`, `build_history_popup`, and the inline link-preview and preedit blocks) touches only `self.instances`, `self.atlas`, `self.queue` and the window column count — `build_inspector` and `build_search_box` already split-borrow exactly those three fields. `build_history_popup` additionally calls `self.history_popup_geometry`, which only needs `grid_size()`; the caller computes the geometry first and passes it in. So they become free functions over:

```rust
pub(in crate::renderer) struct Cells<'a> {
    pub(in crate::renderer) atlas: &'a mut Atlas,
    pub(in crate::renderer) queue: &'a wgpu::Queue,
    pub(in crate::renderer) instances: &'a mut [Instance],
    pub(in crate::renderer) cols: usize,
}
```

built from disjoint field borrows (`Renderer::cells`) — a struct of references on the stack, no allocation.

`build_pane_row` (hot path) additionally needs `selection_colors` (Copy) and the four reusable scratch `Vec`s (`shaped`, `bidi_chars`, `bidi_levels`, `bidi_order`), which it currently `mem::take`s out of `self` and puts back. Grouping them into a `RowScratch` field lets it borrow `&mut RowScratch` next to `Cells` (`Renderer::row_target` returns both), and its 12 parameters collapse into a `Copy` `PaneRowCtx` built once per pane. No allocation is added; the `mem::take` swaps disappear.

The frame phases of `render_panes` (viewport list, image placements, buffers, uniforms, surface, postfx, frame capture) use most of `Renderer`, so they stay `impl Renderer` methods in `renderer/frame.rs` — narrowing them would only move the 15-field borrow into a struct.

## How to Move an Item (used by every task)

Line numbers in this plan are from `d1d0c92` and **shift after every cut**. Always re-locate first. Create two helper scripts once (Task 0 Step 4) in `$TMPDIR/phase3/` (not in the repo):

`$TMPDIR/phase3/span.sh` — prints `FIRST LAST` of an item whose signature matches a regex; `FIRST` includes the `///` and `#[...]` lines directly above; `LAST` is the first following line that is exactly `INDENT}` or `INDENT}}`:

```sh
#!/bin/sh
set -eu
f=$1; pat=$2; ind=${3:-}
sig=$(grep -nE "$pat" "$f" | head -1 | cut -d: -f1)
[ -n "$sig" ] || { echo "no match: $pat" >&2; exit 1; }
first=$sig
while [ "$first" -gt 1 ]; do
  prev=$(sed -n "$((first-1))p" "$f" | sed 's/^ *//')
  case "$prev" in '///'*|'#['*) first=$((first-1));; *) break;; esac
done
last=$(awk -v s="$sig" -v ind="$ind" 'NR>=s && ($0 == ind "}" || $0 == ind "}}") {print NR; exit}' "$f")
echo "$first $last"
```

`$TMPDIR/phase3/cut.sh` — appends lines `FIRST..LAST` of `FILE` to `DEST` and deletes them from `FILE` (no `sed -i`, portable between BSD and GNU):

```sh
#!/bin/sh
set -eu
f=$1; a=$2; b=$3; dest=$4
sed -n "${a},${b}p" "$f" >> "$dest"
sed "${a},${b}d" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
```

Procedure for each item:

1. `set -- $("$TMPDIR/phase3/span.sh" src/atlas/mod.rs '^fn is_builtin_sprite')` (top-level) or with a third argument `'    '` for a method inside an `impl`.
2. Print the boundaries and eyeball them: `sed -n "$1p;$2p" src/atlas/mod.rs`.
3. `"$TMPDIR/phase3/cut.sh" src/atlas/mod.rs "$1" "$2" src/atlas/sprites.rs`, then `echo >> src/atlas/sprites.rs` to keep a blank line between items.
4. `static` / `const` items end with `;` — locate them with `grep -nE '^(pub )?(static|const) NAME'` and cut the exact lines (a multi-line `static`, e.g. `SHARED_PIPELINE`, ends at the first line ending in `;`).
5. After all items of a task are moved: add the file's `//!` header and `use` lines, add `mod x;` to the parent, fix visibility as the task says, then run `cargo clippy --all-targets -- -D warnings` and the Linux clippy, deleting every `use` they report as unused, until clean.
6. Review the move as a move: `git diff -M --color-moved=zebra --color-moved-ws=allow-indentation-change HEAD -- src/atlas src/renderer`. Moved lines must show as moved; the only changed lines are headers, `use` lines, visibility keywords and the substitutions the task lists.

Find external callers (run before and after each task; the list must not change except where a task says so):

```bash
grep -rnE '\b(crate::)?(renderer|atlas)::' src --include='*.rs' | grep -vE '^src/(renderer|atlas)/'
```

Expected callers after Phase 1 (paths may differ slightly):

| Caller | Uses |
| --- | --- |
| `src/config.rs` | `atlas::MetricAdjust`, `atlas::parse_font_feature` |
| `src/app/window.rs` | `atlas::{parse_font_feature, set_font_tweaks, set_font_features, load_font, Atlas::cell_size_pt}` |
| `src/app/state.rs`, `src/app/*` | `renderer::{Renderer, RenderPane, TabBarAction}` and `Renderer` methods |
| `src/app/pointer.rs` | `renderer::SCROLLBAR_WIDTH`, `renderer::scrollbar_thumb` |
| `src/app/latency_benchmarks.rs` | `crate::atlas::macos_coretext_rasterize` (inside `#[cfg(target_os = "macos")]`, test-only) |
| `src/tab.rs` | `crate::renderer::RenderPane` |

`src/bench.rs`, `src/image_layer.rs`, `src/postfx.rs`, `src/graphics.rs`, `src/frame_metrics.rs` do not reference either module (only string literals in `bench.rs`); the renderer calls into them, which this plan does not change.

---

### Task 0: Prepare branch and record baselines

**Files:** none modified in the repo; artifacts go to `$TMPDIR/phase3/`.

- [ ] **Step 1: Clean tree on up-to-date `main`, Phase 1 merged**

```bash
git status --short
git switch main
git log --oneline -20 | grep -c 'refactor(app)'
```

Expected: no `git status` output; the count is ≥ 1 (Phase 1 commits present). Pause other edits to `src/renderer.rs` / `src/atlas.rs` until Task 18 lands.

- [ ] **Step 2: Create the branch**

```bash
git switch -c refactor/split-renderer-atlas
mkdir -p "$TMPDIR/phase3/shots"
```

- [ ] **Step 3: Record test and lint baseline**

```bash
cargo test 2>&1 | grep -E '^test result' > "$TMPDIR/phase3/baseline-tests.txt"
awk '{p += $4} END {print "passed:", p}' "$TMPDIR/phase3/baseline-tests.txt"
cargo clippy --all-targets -- -D warnings
cargo clippy --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
wc -l src/renderer.rs src/atlas.rs
```

Expected: every `test result` line has `0 failed`; note the passed total as `M`. Both clippy runs clean. 2270 and 2904 lines. If any baseline is red, stop and fix it in a separate commit first.

- [ ] **Step 4: Write the helper scripts**

Write `span.sh` and `cut.sh` from "How to Move an Item" into `$TMPDIR/phase3/` and `chmod +x "$TMPDIR"/phase3/*.sh`. Then write `$TMPDIR/phase3/compare_scroll.py`:

```python
"""Compare +bench-scroll-fps runs: median per scenario, fail beyond +-3%."""
import glob, json, statistics, sys

def load(prefix):
    runs = {}
    for path in sorted(glob.glob(prefix + "-*.jsonl")):
        for line in open(path):
            if '"scenario"' in line:
                r = json.loads(line)
                runs.setdefault(r["scenario"], []).append(r)
    return runs

before, after = load(sys.argv[1]), load(sys.argv[2])
ok = True
for sc in before:
    if not all(r["valid"] for r in after.get(sc, [])) or sc not in after:
        print(f"{sc}: missing or invalid after-run (window occluded or samples dropped)")
        ok = False
        continue
    for k in ("submission_fps", "interval_p95_ms", "render_p95_ms"):
        b = statistics.median(r[k] for r in before[sc])
        a = statistics.median(r[k] for r in after[sc])
        d = (a - b) / b * 100 if b else 0.0
        flag = "OK" if abs(d) <= 3.0 else "FAIL"
        ok &= flag == "OK"
        print(f"{sc:18} {k:16} before={b:8.3f} after={a:8.3f} {d:+6.2f}% {flag}")
    cpu_b = statistics.median(r["render_p95_ms"] - r["acquire_p95_ms"] for r in before[sc])
    cpu_a = statistics.median(r["render_p95_ms"] - r["acquire_p95_ms"] for r in after[sc])
    print(f"{sc:18} render-acquire   before={cpu_b:8.3f} after={cpu_a:8.3f} (info only)")
sys.exit(0 if ok else 1)
```

- [ ] **Step 5: Performance baseline**

What each entry point measures (verified in the code):
- `+bench-scroll-fps` (`src/frame_metrics.rs`, driven from `App::about_to_wait`): real swapchain submissions of the full `render_panes` path while scrolling 5,000 lines by 2 px / 12 px per tick; JSON lines with `submission_fps`, `interval_p95_ms`, `render_p95_ms`, `acquire_p95_ms`, `valid`. FIFO-paced, so it catches only regressions large enough to miss vsync — the primary guard for the hot path is the "moved, not changed" diff review.
- `benchmark_before_vs_after` (`src/app/latency_benchmarks.rs`, test): section **[8] Wide CJK CoreText Rasterization** calls the real `atlas::macos_coretext_rasterize` (macOS only). The other sections do not touch atlas/renderer code.
- `+bench-ram`: RSS / footprint JSON in `/tmp/litty_ram_bench.json` (renderer owns the instance buffers and atlas textures).
- `+bench-stress` / `+bench-perf`: grid parsing and PTY only — they do not call the renderer or atlas. Run `+bench-stress` as a **noise control**: if it moves by more than 3% between before and after, the machine is noisy and all numbers must be re-taken.

Close other heavy apps, plug in power, keep the benchmark window fully visible (occlusion invalidates a run), then:

```bash
cargo build --profile fast-release
for i in 1 2 3; do ./target/fast-release/litty +bench-scroll-fps > "$TMPDIR/phase3/scroll-before-$i.jsonl"; done
grep -c '"valid":true' "$TMPDIR"/phase3/scroll-before-*.jsonl
for i in 1 2 3; do ./target/fast-release/litty +bench-stress 20000 | grep -E 'Throughput|Worst' ; done | tee "$TMPDIR/phase3/stress-before.txt"
./target/fast-release/litty +bench-ram && cp /tmp/litty_ram_bench.json "$TMPDIR/phase3/ram-before.json"
for i in 1 2 3; do cargo test --release benchmark_before_vs_after -- --nocapture --test-threads=1 2>&1 | grep -A2 '^\[8\]'; done | tee "$TMPDIR/phase3/coretext-before.txt"
LITTY_SCROLL_TEST_IMAGE="$TMPDIR/phase3/gpu-before.png" cargo test precise_scroll_gpu_pixels -- --ignored --nocapture
```

Expected: each scroll file has 3 `"valid":true` lines (one per scenario, 9 total); stress prints throughput ≥ 20,000 lines/s; RAM JSON has `"passed":true`; three `[8]` blocks; the GPU pixel test passes and writes `gpu-before.png`.

- [ ] **Step 6: Visual baseline scene**

Make a deterministic home for screenshots (fixed config, empty shell rc, seeded history). Fonts are found through `$HOME`, so link the real font directory:

```bash
SHOT_HOME="$TMPDIR/phase3/home"
mkdir -p "$SHOT_HOME/.litty" "$SHOT_HOME/Library" "$SHOT_HOME/.local/share"
ln -sfn "$HOME/Library/Fonts" "$SHOT_HOME/Library/Fonts"          # macOS
ln -sfn "$HOME/.local/share/fonts" "$SHOT_HOME/.local/share/fonts" # Linux
cat > "$SHOT_HOME/.litty/config" <<'EOF'
font-size = 14
window-width = 110
window-height = 34
cursor-style-blink = false
EOF
printf '%s\n' 'git commit -m "phase 3"' 'cargo test --release' 'docker compose up -d' \
  'nvim src/renderer/mod.rs' 'printf "Tiếng Việt\n"' 'ssh prod-db-01' > "$SHOT_HOME/.zsh_history"
```

If a ligature font is installed (e.g. JetBrains Mono / any Nerd Font variant of it), add `font-family = <its exact family name>` to that config and write the name into `$TMPDIR/phase3/font.txt`; otherwise skip the ligature check in the checklist.

Write the scene script `$TMPDIR/phase3/scene.zsh` (run it with zsh, whose `printf` understands `\u`):

```zsh
#!/bin/zsh
# Phase 3 fixed scene. Run from the repo root inside Litty.
img=$(printf %s "$PWD/resources/icon.png" | base64 | tr -d '\n')
printf '\e[2J\e[H'
printf 'Ligatures : -> => != === <= >= :: |> <!-- ~~> www\n'
printf 'Styles    : \e[1mbold\e[0m \e[3mitalic\e[0m \e[1;3mbold-italic\e[0m \e[4munder\e[0m \e[4:3mcurly\e[0m \e[58:2::255:0:0m\e[4mred-ul\e[0m \e[9mstrike\e[0m\n'
printf 'Colors    : \e[31mred \e[32mgreen \e[34mblue\e[0m \e[48;2;40;80;160m truecolor bg \e[0m\n'
printf 'Box/blocks: ┌─┬─┐ ╭──╮ ║═╬ ▀▄█▌▐░▒▓ ◢◣◤◥ ⣿⠿⠛\n'
printf 'Powerline :           Nerd:  \n'
printf 'Emoji     : 😀 👍🏽 👨‍👩‍👧 🇻🇳 ⌚ ☕ ❤️ 1️⃣\n'
printf 'CJK       : 中文字符 日本語のテキスト 한국어\n'
printf 'Vietnamese: Tiếng Việt ắ ằ ẳ ẵ ặ ấ ầ ẩ ẫ ậ ố ồ ổ ỗ ộ ớ ờ ở ỡ ợ ứ ừ ử ữ ự đ Đ | NFD: Việt\n'
printf 'RTL       : مرحبا بالعالم   שלום\n'
printf 'Indic     : कि नमस्ते ক্ষ\n'
printf 'Link      : \e]8;;https://example.com/litty/phase-3/a/very/long/path/to/check/truncation\e\\hyperlink\e]8;;\e\\  https://github.com/\n'
printf 'Image     :\n'
printf '\e_Ga=T,f=100,t=f,c=12,r=6;%s\e\\' "$img"
printf '\n\n\n\n\n\n'
printf 'Under-text image (z<0): '
printf '\e_Ga=T,f=100,t=f,c=12,r=3,z=-1;%s\e\\' "$img"
printf 'TEXT OVER IMAGE\n\n\n'
printf '\e]9;4;1;60\e\\'
```

Launch and capture (macOS: `screencapture -o -w` lets you click the Litty window; Linux: `grim -g "$(slurp)"` or `import`):

```bash
HOME="$SHOT_HOME" ./target/fast-release/litty
# inside Litty:  cd <repo root> && zsh "$TMPDIR/phase3/scene.zsh"
```

Take one screenshot per checklist item below, named `before-NN-<item>.png`, into `$TMPDIR/phase3/shots/`. The same checklist is repeated after Tasks 7, 16 and 18 with `after-...` names.

**Visual checklist** (macOS key / Linux key):

1. Scene as rendered: ligatures (if font set), bold/italic/curly/colored underline/strike, truecolor, box drawing and blocks seamless, Powerline arrows touching neighbours, Nerd icons, color emoji incl. ZWJ family / skin tone / flag / keycap, CJK double width, Vietnamese precomposed and NFD diacritics stacked correctly, Arabic joined right-to-left, Hebrew, Devanagari cluster.
2. Images: icon above, z<0 image under "TEXT OVER IMAGE" (text drawn on top); scroll up 3 lines with the wheel — images move with the text and are clipped at the pane edge; progress bar (OSC 9;4 at 60%) on the active pane.
3. Tab bar: `Cmd+T` / `Ctrl+Shift+T` twice → 3 tabs; ✕ at the left of each tab, `⌘1..⌘3` / `^1..^3` badges at the right, active tab brighter; `cd` into a long path so a title is truncated with `…`; click a ✕, a tab, and the area right of the tabs (new tab).
4. Splits: `Cmd+D` / `Ctrl+Shift+D` then `Cmd+Shift+D` / `Ctrl+Shift+E`; dividers drawn, active divider highlighted, unfocused pane dimmed; scroll one pane — its scrollbar thumb appears at its right edge and the other pane stays still.
5. Find box: `Cmd+F` / `Ctrl+Shift+F`, type `Tiếng` → box top-right below the tab bar, counter `1/1`, matches amber with the current one orange; type a 60-character query → its tail shown with leading `…`; `Ctrl+R` toggles the `.*` badge; query `(` with regex on → red `invalid regex`; `Esc`.
6. History popup: `Ctrl+R` → title, `[Relevance]`/`[Recent]` badge, counter, seeded commands, typed `git` highlights matches in gold, ➜ on the selection, preview section, footer hints; shrink the window to < 70 and < 54 columns → shorter hint sets; `Esc`.
7. Inspector: `Opt+Cmd+I` / `Ctrl+Shift+I` → panel bottom-right with cyan title, label / value colors; press a few keys (last-key line updates); toggle off.
8. Link hover: hover `hyperlink` → underline + URL on the last row, truncated with `...`; hover `https://github.com/`.
9. IME: switch to the macOS Vietnamese (Telex) input source (Linux: ibus/fcitx Unikey) and type `tieengs vieetj` → underlined preedit at the cursor, cursor at its end, committed text correct.
10. Atlas pressure: `python3 -c 'print("".join(chr(c) for c in range(0x4E00, 0x5800)))'` then scroll up and down — no garbage or blank glyphs (exercises atlas growth and the eviction rebuild), then `clear`.
11. Zoom and reload: `Cmd+=` ×3, `Cmd+-` ×3, `Cmd+0` (`Ctrl` on Linux) — glyphs re-rasterize crisply; edit `font-size = 16` in `$SHOT_HOME/.litty/config` and reload (`Cmd+Shift+,` / `Ctrl+Shift+,`).
12. Cursor and bell: `printf '\e[6 q'` (bar), `printf '\e[4 q'` (underline), `printf '\e[2 q'` (block); `printf '\a'` with `bell-features = border` in the config shows the border.

Comparison: open `before-NN` and `after-NN` side by side (Quick Look / an image viewer) and flip between them. If ImageMagick is installed, `magick compare -metric AE before-NN.png after-NN.png "$TMPDIR/phase3/shots/diff-NN.png"` should print `0` for static items (1, 3, 4, 5, 6, 7); items with live input (2, 8–12) are checked by eye.

---

### Task 1: Turn `atlas.rs` into `atlas/mod.rs` + `atlas/tests.rs`

**Files:**
- Rename: `src/atlas.rs` → `src/atlas/mod.rs`
- Create: `src/atlas/tests.rs`

**Interfaces:** none change; `mod atlas;` in `src/main.rs` resolves to the directory automatically.

- [ ] **Step 1: Move the file with history**

```bash
mkdir -p src/atlas
git mv src/atlas.rs src/atlas/mod.rs
```

- [ ] **Step 2: Move the test module (the last item in the file) to `tests.rs`**

```bash
f=src/atlas/mod.rs
start=$(grep -n '^#\[cfg(test)\]$' "$f" | tail -1 | cut -d: -f1)
sed -n "$((start+1))p" "$f"            # must print: mod tests {
tail -1 "$f"                            # must print: }
sed -n "$((start+2)),\$p" "$f" | sed '$d' | sed 's/^    //' > src/atlas/tests.rs
head -n "$((start-1))" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
printf '#[cfg(test)]\nmod tests;\n' >> "$f"
```

`src/atlas/tests.rs` starts with `use super::*;` and needs nothing else at this point (it is still a child of `atlas`, so private fields such as `atlas.n_slots`, `atlas.map`, `atlas.shaper` and private fns such as `alloc_slot` stay reachable).

- [ ] **Step 3: Verify** — six commands; test total == `M`.

```bash
cargo test atlas::tests 2>&1 | grep -E '^test result'
```

Expected: same number of `atlas::tests::` tests as before the move (11 on macOS, 7 elsewhere).

- [ ] **Step 4: Commit**

```bash
git add src/atlas/mod.rs src/atlas/tests.rs
git commit -m "refactor(atlas): turn atlas into a module directory"
```

---

### Task 2: Extract `atlas/sprites.rs`

**Files:**
- Create: `src/atlas/sprites.rs`
- Modify: `src/atlas/mod.rs`, `src/atlas/tests.rs`

**Interfaces:**
- Produces (`pub(super)`, signatures unchanged): `fn is_builtin_sprite(cp: u32) -> bool`, `fn draw_builtin_sprite(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool`, `fn draw_nerd_icon_fallback(buf: &mut [u8], cw: u32, ch: u32, cp: u32) -> bool`. Everything else in the file stays private.

- [ ] **Step 1: Cut the whole sprite section (one contiguous block)**

It runs from the `// ====` banner above `// Built-in Procedural Sprite Renderer` (≈1185) to the blank line before the `// ====` banner above `// macOS CoreText` (≈1953), and contains, in order: `is_builtin_sprite`, `draw_builtin_sprite`, `rasterize_supersampled`, `dist_to_segment`, `draw_powerline`, `fill_rect`, `draw_block_element`, `draw_braille`, `draw_box_drawing`, `draw_nerd_icon_fallback`.

```bash
f=src/atlas/mod.rs
a=$(( $(grep -n '^// Built-in Procedural Sprite Renderer' "$f" | cut -d: -f1) - 1 ))
b=$(( $(grep -n '^// macOS CoreText + CoreGraphics System Fallback Rasterizer' "$f" | cut -d: -f1) - 2 ))
sed -n "${a}p;${b}p" "$f"     # expect the '// ====' banner, then an empty line
printf '%s\n' '//! Procedural glyphs drawn to exact cell size with 4×4 supersampling, like' \
  "//! Ghostty's \`font/sprite\`: Powerline (U+E0A0..=U+E0D7), Box Drawing" \
  '//! (U+2500..=U+257F), Block Elements (U+2580..=U+259F), geometric triangles' \
  '//! (U+25E2..=U+25E5), Braille (U+2800..=U+28FF), and a few Nerd Font icons.' '' > src/atlas/sprites.rs
"$TMPDIR/phase3/cut.sh" "$f" "$a" "$b" src/atlas/sprites.rs
```

Delete the three banner lines (`// ===`, `// Built-in ...`, `// ===`) now at the top of the moved code in `sprites.rs`.

- [ ] **Step 2: Wire it up**

In `src/atlas/mod.rs`, below the `use` block: `mod sprites;` and `use sprites::{draw_builtin_sprite, draw_nerd_icon_fallback, is_builtin_sprite};`. Add `pub(super)` to those three functions in `sprites.rs`. In `src/atlas/tests.rs` add `use super::sprites::draw_builtin_sprite;` below `use super::*;` — explicit, because Task 5 removes the `mod.rs` import the glob would otherwise pick up (an explicit import next to a glob is not a warning: `redundant_imports` is allow-by-default).

- [ ] **Step 3: Verify** — six commands; test total == `M`.

- [ ] **Step 4: Commit**

```bash
git add src/atlas/mod.rs src/atlas/sprites.rs src/atlas/tests.rs
git commit -m "refactor(atlas): move procedural sprites into submodule"
```

---

### Task 3: Extract `atlas/fonts.rs`

**Files:**
- Create: `src/atlas/fonts.rs`
- Modify: `src/atlas/mod.rs`, `src/atlas/tests.rs`

**Interfaces:**
- Produces: `pub fn load_font(families: &[String]) -> Arc<FontSet>` (re-exported as `crate::atlas::load_font`), `pub(super) static FONT_CACHE`, `pub(super) fn shaper_mmap_for(path: &str, index: u32) -> Option<(Arc<memmap2::Mmap>, u32)>`, `pub(super) fn parse_font(path: &str, collection_index: u32, substitutions: bool) -> Option<fontdue::Font>`. Private: `find_first_symbol_font`, `try_parse_font`, `try_parse_font_index`, `discover_nerd_fonts`.
- `FontSet` **stays in `mod.rs`**: `tests.rs` constructs it with its private fields (`shaper_mmap`, `shaper_index`), which only `atlas` and its descendants can see; `fonts.rs`, also a child, can construct it too.

- [ ] **Step 1: Cut the font discovery section**

From the `// ====` banner above `// Font Discovery (Primary + Nerd Font / Symbol Fallbacks)` (≈2498) to the line before `#[cfg(test)]` at the end of the file:

```bash
f=src/atlas/mod.rs
a=$(( $(grep -n '^// Font Discovery' "$f" | cut -d: -f1) - 1 ))
b=$(( $(grep -n '^#\[cfg(test)\]$' "$f" | tail -1 | cut -d: -f1) - 1 ))
sed -n "${a}p;${b}p" "$f"
printf '%s\n' '//! Primary font resolution (`font-family`, `LITTY_FONT`, platform defaults,' \
  '//! installed Nerd Fonts) and symbol-font fallbacks, cached per family list.' '' \
  'use std::path::Path;' 'use std::sync::Arc;' '' 'use parking_lot::Mutex;' '' 'use super::FontSet;' '' > src/atlas/fonts.rs
"$TMPDIR/phase3/cut.sh" "$f" "$a" "$b" src/atlas/fonts.rs
```

Delete the moved banner lines. The two `#[cfg(target_os = ...)] primary_candidates.extend(...)` statements inside `load_font` move verbatim — they already have both platform arms.

- [ ] **Step 2: Wire it up**

In `src/atlas/mod.rs`:

```rust
mod fonts;
pub use fonts::load_font;
use fonts::FONT_CACHE;
```

Apply the visibilities above. `set_font_tweaks` and `set_font_features` (still in `mod.rs`) use `FONT_CACHE`; `rasterize_color` (non-macOS, still in `mod.rs`) uses `fonts::shaper_mmap_for` — add `use fonts::shaper_mmap_for;` gated exactly like its caller: `#[cfg(not(target_os = "macos"))]`. In `src/atlas/tests.rs` add `use super::fonts::{parse_font, shaper_mmap_for};`.

- [ ] **Step 3: Verify** — six commands; test total == `M`. The external caller grep still shows `atlas::load_font` resolving (it compiles).

- [ ] **Step 4: Commit**

```bash
git add src/atlas/mod.rs src/atlas/fonts.rs src/atlas/tests.rs
git commit -m "refactor(atlas): move font discovery into submodule"
```

---

### Task 4: Extract `atlas/shaper.rs`

**Files:**
- Create: `src/atlas/shaper.rs`
- Modify: `src/atlas/mod.rs`, `src/atlas/tests.rs`

**Interfaces:**
- Produces: `pub(super) struct GlyphPiece` (all five fields `pub(super)`), `pub(super) struct Shaper` (fields `pieces`, `piece_ids` `pub(super)`; `features`, `buf`, `cell_units` private), `pub(super) fn Shaper::new(features: Vec<rustybuzz::Feature>) -> Self`, `pub fn Shaper::shape_row(&mut self, fonts: &FontSet, px: f32, cell_w: u32, row: &[crate::grid::Cell], out: &mut Vec<u32>) -> (bool, bool)` (unchanged), `pub(super) static FONT_FEATURES`, `pub fn parse_font_feature(v: &str) -> Result<rustybuzz::Feature, String>`, `pub fn set_font_features(features: &[rustybuzz::Feature])`.
- `GLYPH_PIECE` and `SHAPED_BLANK` stay in `mod.rs` with the other slot-key constants; `shaper.rs` uses `super::{FontSet, GLYPH_PIECE, SHAPED_BLANK}`.
- Field visibility is required by `Atlas::rasterize_piece` / `Atlas::shape_row` (`self.shaper.pieces`) and by `tests.rs` (`atlas.shaper.piece_ids`, `on.pieces`, `GlyphPiece { gid, span, origin, bold, italic }`).

- [ ] **Step 1: Cut the block**

From `/// A glyph by index, drawn across` (≈111) to the blank line before `pub struct Atlas {` (≈244): `GlyphPiece`, `Shaper`, `impl Shaper`, `FONT_FEATURES`, `parse_font_feature`, `set_font_features`.

```bash
f=src/atlas/mod.rs
a=$(grep -n '^/// A glyph by index, drawn across' "$f" | cut -d: -f1)
b=$(( $(grep -n '^pub struct Atlas {' "$f" | cut -d: -f1) - 1 ))
sed -n "${a}p;${b}p" "$f"
printf '%s\n' '//! Ligature and `font-feature` shaping with rustybuzz: runs of printable ASCII' \
  '//! that contain punctuation become interned multi-cell glyph pieces.' '' \
  'use std::collections::HashMap;' '' 'use parking_lot::Mutex;' '' \
  'use super::fonts::FONT_CACHE;' 'use super::{FontSet, GLYPH_PIECE, SHAPED_BLANK};' '' > src/atlas/shaper.rs
"$TMPDIR/phase3/cut.sh" "$f" "$a" "$b" src/atlas/shaper.rs
```

- [ ] **Step 2: Wire it up**

In `src/atlas/mod.rs`:

```rust
mod shaper;
pub use shaper::{parse_font_feature, set_font_features};
use shaper::{Shaper, FONT_FEATURES};
```

Apply the visibilities above. In `src/atlas/tests.rs` add `use super::shaper::{GlyphPiece, Shaper};`.

- [ ] **Step 3: Verify** — six commands; test total == `M`. `LITTY_LIGATURE_FONT=<path to a ligature .ttf> cargo test ligatures_shape_into_glyph_pieces` passes if such a font is available (the test returns early otherwise).

- [ ] **Step 4: Commit**

```bash
git add src/atlas/mod.rs src/atlas/shaper.rs src/atlas/tests.rs
git commit -m "refactor(atlas): move ligature shaper into submodule"
```

---

### Task 5: Extract `atlas/raster.rs`

**Files:**
- Create: `src/atlas/raster.rs`
- Modify: `src/atlas/mod.rs`

**Interfaces:**
- Produces a second `impl Atlas` block. Methods called from `mod.rs` become `pub(super)`: `fn rasterize(&mut self, key: u32) -> bool`, `fn rasterize_wide(&self, base_key: u32, wide_r8: &mut [u8], wide_rgba: &mut [u8]) -> bool`, `fn upload(&self, queue: &wgpu::Queue, slot: u16)`, `fn upload_color(&self, queue: &wgpu::Queue, slot: u16)`. Private: `rasterize_piece`, `rasterize_cluster_into`, `apply_style`, `rasterize_into`, free fns `rasterize_with_font`, `stem_boost`.
- Consumes from `super`: `THICKEN`, `WIDE_LEFT`, `WIDE_RIGHT`, `STYLE_BOLD`, `STYLE_ITALIC`, `CODEPOINT_MASK`, `CLUSTER_KEY`, `GLYPH_PIECE`, `SHAPED_BLANK`, `GlyphStyle`, `is_emoji`, `is_emoji_text`, `rasterize_color`, `rasterize_text_cluster`, `macos_coretext_rasterize`, and `super::sprites::{draw_builtin_sprite, draw_nerd_icon_fallback}`.

- [ ] **Step 1: Cut the raster methods out of `impl Atlas`**

From `    /// Rasterize a glyph piece (see [`GLYPH_PIECE`]) into `buf`.` (≈802) through the last method `upload` — i.e. up to the line before the `}` that closes `impl Atlas` (≈1118). The closing `}` stays in `mod.rs`.

```bash
f=src/atlas/mod.rs
a=$(grep -n '^    /// Rasterize a glyph piece' "$f" | cut -d: -f1)
close=$(awk -v s="$a" 'NR>s && /^}$/ {print NR; exit}' "$f")
b=$((close - 1))
sed -n "${a}p;${b}p;${close}p" "$f"   # doc line, '    }' of upload, '}' of the impl
printf '%s\n' '//! Glyph rasterization into atlas slots: primary font + fallbacks, synthetic' \
  '//! bold / italic, shaped ligature pieces, clusters, wide and color glyphs, and' \
  '//! the texture uploads. Sprites, CoreText and bitmap emoji are called from here.' '' \
  'use super::*;' '' 'impl Atlas {' > src/atlas/raster.rs
"$TMPDIR/phase3/cut.sh" "$f" "$a" "$b" src/atlas/raster.rs
printf '}\n\n' >> src/atlas/raster.rs
```

(`use super::*;` is a temporary convenience; Step 3 replaces it with explicit imports.)

- [ ] **Step 2: Cut `rasterize_with_font` and `stem_boost`**

```bash
set -- $("$TMPDIR/phase3/span.sh" src/atlas/mod.rs '^fn rasterize_with_font')
"$TMPDIR/phase3/cut.sh" src/atlas/mod.rs "$1" "$2" src/atlas/raster.rs; echo >> src/atlas/raster.rs
set -- $("$TMPDIR/phase3/span.sh" src/atlas/mod.rs '^fn stem_boost')
"$TMPDIR/phase3/cut.sh" src/atlas/mod.rs "$1" "$2" src/atlas/raster.rs
```

(`span.sh` includes `#[allow(clippy::too_many_arguments)]` and `#[inline]` above them.)

- [ ] **Step 3: Wire it up**

Add `mod raster;` in `src/atlas/mod.rs`; add `pub(super)` to the four methods listed above. Replace `use super::*;` in `raster.rs` with explicit imports of what the compiler reports as needed, e.g.:

```rust
use std::sync::atomic::Ordering;

use super::sprites::{draw_builtin_sprite, draw_nerd_icon_fallback};
use super::{
    is_emoji, is_emoji_text, macos_coretext_rasterize, rasterize_color, rasterize_text_cluster, Atlas, GlyphStyle,
    CLUSTER_KEY, CODEPOINT_MASK, GLYPH_PIECE, SHAPED_BLANK, STYLE_BOLD, STYLE_ITALIC, THICKEN, WIDE_LEFT, WIDE_RIGHT,
};
```

Remove from `mod.rs` the `use sprites::{draw_builtin_sprite, draw_nerd_icon_fallback}` that clippy now reports unused (keep `is_builtin_sprite`, used by `glyph_slots`); `tests.rs` already imports `draw_builtin_sprite` explicitly (Task 2).

- [ ] **Step 4: Verify** — six commands; test total == `M`.

```bash
wc -l src/atlas/*.rs
```

Expected: `mod.rs` ≈ 1060 (it still holds the platform rasterizers until Task 6), `raster.rs` ≈ 390.

- [ ] **Step 5: Commit**

```bash
git add src/atlas/mod.rs src/atlas/raster.rs
git commit -m "refactor(atlas): move glyph rasterization into submodule"
```

---

### Task 6: Split the platform rasterizers into `atlas/coretext.rs` and `atlas/linux.rs`

**Files:**
- Create: `src/atlas/coretext.rs`, `src/atlas/linux.rs`
- Modify: `src/atlas/mod.rs`, `src/atlas/raster.rs`, `src/atlas/tests.rs`

**What the code actually has (re-verify with `grep -n 'cfg' src/atlas/mod.rs`):**

| Item | cfg | Goes to |
| --- | --- | --- |
| `macos_coretext_rasterize` stub (returns `false`), ≈1960 | `not(target_os = "macos")` | `linux.rs` |
| `cached_ct_base_font`, ≈1965 | `target_os = "macos"` | `coretext.rs` |
| `pub(crate) fn macos_coretext_rasterize`, ≈2012 | `target_os = "macos"` | `coretext.rs` |
| `is_emoji`, `is_emoji_text`, ≈2242 / 2248 | none | stay in `mod.rs` |
| `rasterize_color` (CoreText line), ≈2256 | `target_os = "macos"` | `coretext.rs` |
| `rasterize_text_cluster` (CoreText line), ≈2264 | `target_os = "macos"` | `coretext.rs` |
| `rasterize_text_cluster` stub, ≈2269 | `not(target_os = "macos")` | `linux.rs` |
| `coretext_line`, ≈2277 | `target_os = "macos"` | `coretext.rs` |
| `rasterize_color` (Noto Color Emoji CBDT PNG), ≈2439 | `not(target_os = "macos")` | `linux.rs` |
| `blit_fit`, ≈2467 | `cfg_attr(target_os = "macos", allow(dead_code))` | stays in `mod.rs` (the cross-platform test `blit_fit_scales_premultiplies_and_centers` uses it on macOS) |
| `load_font` platform default paths | `#[cfg]` statements inside one fn | already in `fonts.rs`, unchanged |

The non-macOS path uses **no fontconfig and no FreeType**: family lookup is the crate's own `crate::fontdb` (reads OpenType `name` tables), rasterization is `fontdue` (in `raster.rs`, shared), color emoji is `rustybuzz` + `ttf_parser` CBDT/sbix PNG strikes from "Noto Color Emoji" decoded with `crate::graphics::decode_png`, and there is no system glyph fallback (the stubs return `false`). The file is named `linux.rs` per the outline but is gated `not(target_os = "macos")`, matching the existing gates.

**Interfaces** (identical in both files, `pub(super)` unless noted):
- `fn macos_coretext_rasterize(buf: &mut [u8], cw: u32, ch_px: u32, baseline: i32, px: f32, ch: char) -> bool` — `pub(crate)` in `coretext.rs` (the CoreText latency benchmark calls it), `pub(super)` in `linux.rs`.
- `fn rasterize_color(buf: &mut [u8], w: u32, h: u32, px: f32, text: &str) -> bool`
- `fn rasterize_text_cluster(buf: &mut [u8], w: u32, h: u32, baseline: i32, px: f32, text: &str) -> bool`

- [ ] **Step 1: Create both files with headers, then move items one by one**

`src/atlas/coretext.rs`:

```rust
//! macOS system rasterizers through CoreText: single-glyph fallback for
//! codepoints the configured fonts lack, color emoji, and grapheme clusters
//! the primary font cannot shape. `linux.rs` is the non-macOS counterpart.

use parking_lot::Mutex;
```

`src/atlas/linux.rs`:

```rust
//! Non-macOS counterparts of `coretext.rs`. There is no system glyph fallback
//! (no fontconfig / FreeType); color emoji come from bitmap strikes of
//! Noto Color Emoji found through `crate::fontdb`.

use std::sync::Arc;

use super::blit_fit;
use super::fonts::shaper_mmap_for;
```

For each row of the table, run `span.sh` with the fn name **and** check that the printed first line is the expected `#[cfg(...)]` (two items share the names `rasterize_color` / `rasterize_text_cluster`, so match on the cfg line instead):

```bash
f=src/atlas/mod.rs
grep -nE '^#\[cfg\((not\()?target_os = "macos"\)?\)\]$' "$f" -A1 | grep -E 'fn '
```

This lists every gated fn with its line. Two names exist twice, so use these exact `span.sh` regexes (they differ in the `_` prefix of the unused parameters):

| Item | `span.sh` regex | Destination |
| --- | --- | --- |
| stub `macos_coretext_rasterize` | `'^fn macos_coretext_rasterize\(_buf'` | `linux.rs` |
| `cached_ct_base_font` | `'^unsafe fn cached_ct_base_font'` | `coretext.rs` |
| CoreText `macos_coretext_rasterize` | `'^pub\(crate\) fn macos_coretext_rasterize'` | `coretext.rs` |
| CoreText `rasterize_color` | `'^fn rasterize_color\(buf: &mut \[u8\], w: u32, h: u32, px'` | `coretext.rs` |
| CoreText `rasterize_text_cluster` | `'^fn rasterize_text_cluster\(buf'` | `coretext.rs` |
| stub `rasterize_text_cluster` | `'^fn rasterize_text_cluster\(_buf'` | `linux.rs` |
| `coretext_line` | `'^fn coretext_line'` | `coretext.rs` |
| bitmap `rasterize_color` | `'^fn rasterize_color\(buf: &mut \[u8\], w: u32, h: u32, _px'` | `linux.rs` |

Cut each (doc lines + cfg line + body) into the file from the table, re-running `span.sh` before every cut. Remove the `// ====` banner pairs for "macOS CoreText ..." from `mod.rs` and keep the "Color emoji" banner above `is_emoji`.

Then delete the now-redundant `#[cfg(...)]` line on every item inside `coretext.rs` and `linux.rs` — the module declaration carries the gate. Keep `#[cfg_attr]` on `blit_fit` in `mod.rs` as is.

- [ ] **Step 2: Declare the modules with one alias**

In `src/atlas/mod.rs`:

```rust
#[cfg(target_os = "macos")]
mod coretext;
#[cfg(not(target_os = "macos"))]
mod linux;

/// The platform's system rasterizers (CoreText on macOS, bitmap emoji only elsewhere).
#[cfg(target_os = "macos")]
use coretext as system;
#[cfg(not(target_os = "macos"))]
use linux as system;

/// Called by the CoreText benchmark in `app::latency_benchmarks`.
#[cfg(all(test, target_os = "macos"))]
pub(crate) use coretext::macos_coretext_rasterize;
```

In `src/atlas/raster.rs` replace the `macos_coretext_rasterize, rasterize_color, rasterize_text_cluster` names in the `use super::{...}` with a separate `use super::system::{macos_coretext_rasterize, rasterize_color, rasterize_text_cluster};`.

In `src/atlas/tests.rs` add:

```rust
#[cfg(target_os = "macos")]
use super::coretext::{rasterize_color, rasterize_text_cluster};
```

(`macos_coretext_rasterize` reaches the tests through the `pub(crate) use` above and `use super::*`.)

- [ ] **Step 3: Check that both targets see the same API**

```bash
grep -nE '^pub(\(crate\)|\(super\)) fn' src/atlas/coretext.rs src/atlas/linux.rs
```

Expected: `macos_coretext_rasterize`, `rasterize_color`, `rasterize_text_cluster` in both files with identical parameter types.

- [ ] **Step 4: Verify** — six commands (the Linux clippy compiles `linux.rs` and the non-macOS tests); test total == `M`. Then:

```bash
cargo test --release benchmark_before_vs_after -- --nocapture --test-threads=1 2>&1 | grep -A2 '^\[8\]'
```

Expected: section [8] still runs (proves the `crate::atlas::macos_coretext_rasterize` path resolves).

- [ ] **Step 5: Commit**

```bash
git add src/atlas/mod.rs src/atlas/coretext.rs src/atlas/linux.rs src/atlas/raster.rs src/atlas/tests.rs
git commit -m "refactor(atlas): split coretext and non-macos rasterizers"
```

---

### Task 7: Declare CoreText FFI types and functions once

**Files:**
- Modify: `src/atlas/coretext.rs`

**What is duplicated (re-verify):** the `#[repr(C)]` structs `CGPoint`, `CGSize`, `CGRect` are declared as fn-local items twice — inside `macos_coretext_rasterize` (with `CFRange`, ≈2024–2050 in the original file) and inside `coretext_line` (≈2280–2298). The extern declarations are also repeated per function: `CFRelease` ×3, `CFStringCreateWithCharacters` ×3, `CTFontCreateWithName` ×2, `CGBitmapContextCreate` ×2, `CGColorSpaceCreateDeviceGray` ×2, `CGColorSpaceRelease` ×2, `CGContextRelease` ×2, `CGContextSetGrayFillColor` ×2. List them:

```bash
awk '/#\[link\(name = "/{fw=$0; sub(/.*name = "/,"",fw); sub(/".*/,"",fw); inb=1} inb && /^ *}$/{inb=0} inb && /^ +(fn|static) [A-Za-z]/{n=$0; sub(/^ +(fn|static) /,"",n); sub(/[(:].*/,"",n); print fw, n}' src/atlas/coretext.rs | sort | uniq -c
```

**Interfaces:** none change (all items are private to `coretext.rs`).

- [ ] **Step 1: Hoist the types to module level**

Directly below the `use` lines of `coretext.rs`:

```rust
use std::ffi::c_void;
use std::ptr;

#[repr(C)]
#[derive(Copy, Clone)]
struct CFRange {
    location: isize,
    length: isize,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
```

Delete both fn-local copies, and the fn-local `use std::ffi::c_void;` / `use std::ptr;` lines in `cached_ct_base_font`, `macos_coretext_rasterize` and `coretext_line`.

- [ ] **Step 2: Hoist the extern blocks**

Create one module-level block per framework holding the union of the declarations, each copied verbatim from its first occurrence (`#[link(name = "CoreFoundation", kind = "framework")] unsafe extern "C" { ... }`, then `CoreGraphics`, then `CoreText`), including the `static` items (`kCFTypeDictionaryKeyCallBacks`, `kCFTypeDictionaryValueCallBacks`, `kCFBooleanTrue`, `kCTFontAttributeName`, `kCTForegroundColorFromContextAttributeName`). Delete the fn-local extern blocks. Keep the fn-local constants `PREMULTIPLIED_LAST` and `GLYPH_PATH_BOUNDS` in `coretext_line` and the fn-local `static BASE_FONT_CACHE` in `cached_ct_base_font` where they are.

Before deleting a duplicate, compare its parameter and return types with the kept one; if any differ, stop and keep both (rustc's `clashing_extern_declarations` lint would also flag a mismatch as a warning, which `-D warnings` turns into an error).

- [ ] **Step 3: Verify** — six commands; test total == `M`. Re-run the counting command from above.

Expected: every name appears exactly once. macOS-only tests `color_emoji_rasterizes_in_color`, `text_clusters_render_through_coretext`, `macos_coretext_fallback_renders_missing_menlo_glyphs` pass.

- [ ] **Step 4: Atlas gate** (atlas is complete; run the after-measurements once here)

```bash
cargo build --profile fast-release
for i in 1 2 3; do cargo test --release benchmark_before_vs_after -- --nocapture --test-threads=1 2>&1 | grep -A2 '^\[8\]'; done | tee "$TMPDIR/phase3/coretext-atlas.txt"
LITTY_SCROLL_TEST_IMAGE="$TMPDIR/phase3/gpu-atlas.png" cargo test precise_scroll_gpu_pixels -- --ignored --nocapture
```

Expected: the median "Sau" time of [8] is within ±3% of `coretext-before.txt` (if not, re-run both with nothing else running). Repeat visual checklist items 1, 2, 9, 10, 11 as `after7-NN-*.png` and compare.

- [ ] **Step 5: Commit**

```bash
git add src/atlas/coretext.rs
git commit -m "refactor(atlas): declare coretext ffi types and functions once"
```

---

### Task 8: Turn `renderer.rs` into `renderer/mod.rs` + `renderer/tests.rs`

**Files:**
- Rename: `src/renderer.rs` → `src/renderer/mod.rs`
- Create: `src/renderer/tests.rs`

**Interfaces:** none change.

- [ ] **Step 1: Move the file and the tests**

```bash
mkdir -p src/renderer
git mv src/renderer.rs src/renderer/mod.rs
f=src/renderer/mod.rs
start=$(grep -n '^#\[cfg(test)\]$' "$f" | tail -1 | cut -d: -f1)
sed -n "$((start+1))p" "$f"; tail -1 "$f"     # expect: mod tests {   and   }
sed -n "$((start+2)),\$p" "$f" | sed '$d' | sed 's/^    //' > src/renderer/tests.rs
head -n "$((start-1))" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
printf '#[cfg(test)]\nmod tests;\n' >> "$f"
```

- [ ] **Step 2: Fix the shader include paths** (they resolve relative to the source file)

In `src/renderer/mod.rs`: `wgpu::include_wgsl!("shader.wgsl")` → `wgpu::include_wgsl!("../shader.wgsl")`.
In `src/renderer/tests.rs`: `include_str!("shader.wgsl")` → `include_str!("../shader.wgsl")`.

```bash
grep -n 'shader.wgsl' src/renderer/*.rs
```

Expected: both lines show `../shader.wgsl`.

- [ ] **Step 3: Verify** — six commands; test total == `M`. Then:

```bash
LITTY_SCROLL_TEST_IMAGE="$TMPDIR/phase3/gpu-t8.png" cargo test precise_scroll_gpu_pixels -- --ignored --nocapture
cmp "$TMPDIR/phase3/gpu-before.png" "$TMPDIR/phase3/gpu-t8.png" && echo identical
```

Expected: test passes, `identical`.

- [ ] **Step 4: Commit**

```bash
git add src/renderer/mod.rs src/renderer/tests.rs
git commit -m "refactor(render): turn renderer into a module directory"
```

---

### Task 9: Extract wgpu object creation into `renderer/gpu.rs`

**Files:**
- Create: `src/renderer/gpu.rs`
- Modify: `src/renderer/mod.rs`, `src/renderer/tests.rs`

Deviation from the outline: keeping device / pipeline / bind-group creation in `mod.rs` would leave it at ~810 lines; `Renderer::new` stays in `mod.rs` and calls into `gpu.rs`.

**Interfaces** (all `pub(super)`):
- `fn global_instance() -> &'static wgpu::Instance` (moved; private `WGPU_INSTANCE` moves with it)
- `fn shared_device(surface: &wgpu::Surface<'static>) -> (wgpu::Adapter, Arc<wgpu::Device>, Arc<wgpu::Queue>)` (new wrapper around the existing block; `SharedGpu` and `SHARED_GPU` move and stay private to `gpu.rs`)
- `fn shared_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline)` (new wrapper; `SHARED_PIPELINE` moves, private)
- `fn create_cells_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline)` (moved; used by `shared_pipeline` and `tests.rs`)
- `fn make_bind_group(device, bgl, uniform_buf, atlas, sampler, viewport_buf) -> wgpu::BindGroup`, `fn make_instance_buf(device, cells) -> wgpu::Buffer`, `fn make_viewport_buffer(device, panes) -> wgpu::Buffer` (moved, signatures unchanged)

- [ ] **Step 1: Move the items**

Header:

```rust
//! wgpu objects shared by every window: the instance, one adapter / device /
//! queue, the cached cell pipeline per surface format, and buffer / bind-group
//! constructors.

use std::sync::Arc;

use super::{Instance, Viewport};
use crate::atlas::Atlas;
```

Move, re-locating each with `span.sh` / `grep`: `make_viewport_buffer` (≈123), `static WGPU_INSTANCE` (≈202, one line), `global_instance` (≈204), `struct SharedGpu` (≈219), `static SHARED_GPU` (≈225, one line), `static SHARED_PIPELINE` (≈226–227, two lines), `create_cells_pipeline` (≈229), `make_bind_group` (≈2083 originally; now near the end of `mod.rs`), `make_instance_buf`. The `#[cfg]` pair choosing `METAL` / `VULKAN` inside `global_instance` moves verbatim.

`create_cells_pipeline` contains `wgpu::include_wgsl!("../shader.wgsl")`; `gpu.rs` is in the same directory as `mod.rs`, so the path stays `../shader.wgsl`.

- [ ] **Step 2: Add the two wrappers**

Cut the bodies of the two blocks in `Renderer::new` verbatim into:

```rust
/// The process-wide adapter, device and queue; created on the first window.
pub(super) fn shared_device(surface: &wgpu::Surface<'static>) -> (wgpu::Adapter, Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance = global_instance();
    let mut guard = SHARED_GPU.lock();
    // ...the existing `if let Some(gpu) = guard.as_ref() { ... } else { ... }` expression,
    // with `compatible_surface: Some(&surface)` changed to `compatible_surface: Some(surface)`
}

/// The cell pipeline for `format`, reused while every window has the same format.
pub(super) fn shared_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let mut guard = SHARED_PIPELINE.lock();
    // ...the existing `if let Some((cached_fmt, ref cached_bgl, ref cached_pipe)) = *guard { ... } else { ... }`
    // expression, with `create_cells_pipeline(&device, format)` changed to `create_cells_pipeline(device, format)`
}
```

The lock is held for exactly the same span as before (the whole block). In `Renderer::new` the two blocks become:

```rust
let instance = gpu::global_instance();
let surface = instance.create_surface(window.clone()).expect("create surface");
let (adapter, device, queue) = gpu::shared_device(&surface);
// ...unchanged surface configuration and Atlas::new...
let (bgl, pipeline) = gpu::shared_pipeline(&device, format);
```

- [ ] **Step 3: Wire it up**

`mod.rs`: `mod gpu;` and `use gpu::{make_bind_group, make_instance_buf, make_viewport_buffer};`. `tests.rs`: add `use super::gpu::create_cells_pipeline;` (the test body's inner `use super::*;` no longer brings it).

- [ ] **Step 4: Verify** — six commands; test total == `M`; GPU pixel image identical:

```bash
LITTY_SCROLL_TEST_IMAGE="$TMPDIR/phase3/gpu-t9.png" cargo test precise_scroll_gpu_pixels -- --ignored --nocapture
cmp "$TMPDIR/phase3/gpu-before.png" "$TMPDIR/phase3/gpu-t9.png" && echo identical
cargo run --release   # open two windows (Cmd+N / Ctrl+Shift+N): both render (shared device and pipeline reused)
```

- [ ] **Step 5: Commit**

```bash
git add src/renderer/mod.rs src/renderer/gpu.rs src/renderer/tests.rs
git commit -m "refactor(render): move wgpu object creation into gpu module"
```

---

### Task 10: Add the `ui` cell context and move the tab bar

**Files:**
- Create: `src/renderer/ui/mod.rs`, `src/renderer/ui/tab_bar.rs`
- Modify: `src/renderer/mod.rs`, `src/renderer/tests.rs`

**Interfaces:**
- Produces in `src/renderer/ui/mod.rs`:

```rust
//! Overlay widgets drawn as cell instances on top of the panes. Each builder
//! borrows only the atlas, the queue and the instance slice (`Cells`), never
//! the whole `Renderer`.

use super::{Instance, Renderer};
use crate::atlas::Atlas;

pub(super) mod tab_bar;

/// The window's cell instances and the glyph atlas they index.
pub(in crate::renderer) struct Cells<'a> {
    pub(in crate::renderer) atlas: &'a mut Atlas,
    pub(in crate::renderer) queue: &'a wgpu::Queue,
    pub(in crate::renderer) instances: &'a mut [Instance],
    /// Window width in cells: the row stride of `instances`.
    pub(in crate::renderer) cols: usize,
}

/// Window rows whose instances changed this frame; `lo..=hi` is uploaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::renderer) struct DirtyRows {
    pub(in crate::renderer) lo: usize,
    pub(in crate::renderer) hi: usize,
}

impl DirtyRows {
    /// Nothing dirty (`lo > hi`).
    pub(in crate::renderer) const NONE: Self = Self { lo: usize::MAX, hi: 0 };

    /// Widen the range to include rows `first..=last`.
    #[inline]
    pub(in crate::renderer) fn mark(&mut self, first: usize, last: usize) {
        self.lo = self.lo.min(first);
        self.hi = self.hi.max(last);
    }
}

impl Renderer {
    /// Disjoint borrows of the fields every overlay writes through.
    pub(in crate::renderer) fn cells(&mut self, cols: usize) -> Cells<'_> {
        Cells { atlas: &mut self.atlas, queue: &self.queue, instances: &mut self.instances, cols }
    }
}

#[cfg(test)]
mod tests {
    use super::DirtyRows;

    #[test]
    fn dirty_rows_mark_widens_range() {
        let mut d = DirtyRows::NONE;
        assert!(d.lo > d.hi, "starts empty");
        d.mark(5, 7);
        d.mark(2, 3);
        assert_eq!(d, DirtyRows { lo: 2, hi: 7 });
    }
}
```

- Produces in `src/renderer/ui/tab_bar.rs`: `pub(in crate::renderer) fn build(cells: &mut Cells, titles: &[String], active_idx: usize)` and private `fn tab_label(title: &str) -> &str`.

- [ ] **Step 1: Switch `render_panes` from `lo` / `hi` to `DirtyRows`** (mechanical, same values)

In `render_panes` (`src/renderer/mod.rs`), with `use ui::DirtyRows;`:
- `let (mut lo, mut hi) = (usize::MAX, 0);` → `let mut dirty = DirtyRows::NONE;`
- tab bar: `lo = 0;` / `hi = 0;` → `dirty.lo = 0;` / `dirty.hi = 0;`
- dirty pane rows: `lo = lo.min(dst_row); hi = hi.max(dst_row);` → `dirty.mark(dst_row, dst_row);`
- eviction: `lo = 0; hi = total_rows - 1;` → `dirty.lo = 0; dirty.hi = total_rows - 1;`
- link preview: `lo = lo.min(preview_row); hi = hi.max(preview_row);` → `dirty.mark(preview_row, preview_row);`
- preedit: `lo = lo.min(cy); hi = hi.max(cy);` → `dirty.mark(cy, cy);`
- builder calls: `&mut lo, &mut hi` → `&mut dirty.lo, &mut dirty.hi` (their signatures change in Tasks 11–13)
- upload: `lo <= hi`, `lo * cols`, `(hi + 1) * cols` → `dirty.lo <= dirty.hi`, `dirty.lo * cols`, `(dirty.hi + 1) * cols`

```bash
grep -nE '\b(lo|hi)\b' src/renderer/mod.rs | grep -v 'dirty\.'
```

Expected: only the builder bodies' own `lo` / `hi` parameters remain.

- [ ] **Step 2: Move the tab bar**

Move `build_tab_bar_row` (method, `span.sh ... '^    fn build_tab_bar_row' '    '`) and `tab_label` (top-level) into `src/renderer/ui/tab_bar.rs`:

```rust
//! Tab strip on the top row: close button, centered title, shortcut badge.

use super::super::Instance;
use super::Cells;
```

Turn the method into a free function: signature `pub(in crate::renderer) fn build(cells: &mut Cells, titles: &[String], active_idx: usize)`, add `let cols = cells.cols;` as its first line, dedent by 4 spaces, then:

```bash
perl -pi -e 's/&self\.queue/cells.queue/g; s/self\.atlas\./cells.atlas./g; s/self\.instances/cells.instances/g' src/renderer/ui/tab_bar.rs
grep -n 'self' src/renderer/ui/tab_bar.rs
```

Expected: no `self` left. The `#[cfg(target_os = "macos")] let mod_ch = '⌘';` / `#[cfg(not(...))] let mod_ch = '^';` pair moves verbatim.

Move `test_tab_label_strips_user_host_prefix` from `src/renderer/tests.rs` into `#[cfg(test)] mod tests { use super::tab_label; ... }` at the end of `tab_bar.rs`, and drop `use super::tab_label;` from `src/renderer/tests.rs`.

- [ ] **Step 3: Call it**

In `src/renderer/mod.rs` add `mod ui;` and replace both `self.build_tab_bar_row(cols, titles, active_idx);` calls (normal path and eviction loop) with:

```rust
ui::tab_bar::build(&mut self.cells(cols), titles, active_idx);
```

- [ ] **Step 4: Verify** — six commands; test total == `M + 1` (new `dirty_rows_mark_widens_range`). Quick look: `cargo run --release`, open 3 tabs (checklist item 3).

- [ ] **Step 5: Commit**

```bash
git add src/renderer/mod.rs src/renderer/tests.rs src/renderer/ui/mod.rs src/renderer/ui/tab_bar.rs
git commit -m "refactor(render): add ui cell context and move tab bar into it"
```

---

### Task 11: Move the find box to `ui/search_box.rs`

**Files:**
- Create: `src/renderer/ui/search_box.rs`
- Modify: `src/renderer/ui/mod.rs`, `src/renderer/mod.rs`

**Interfaces:**
- `pub(in crate::renderer) fn geometry(cols: usize, top_rows: usize, total_rows: usize) -> Option<(usize, usize, usize)>` (was `Renderer::search_box_geometry`, already `self`-free)
- `pub(in crate::renderer) fn build(cells: &mut Cells, top_rows: usize, total_rows: usize, search: &SearchBar, dirty: &mut DirtyRows)`

- [ ] **Step 1: Write the failing test first** (characterizes the existing geometry)

Create `src/renderer/ui/search_box.rs` with the header and test only, and add `pub(super) mod search_box;` to `ui/mod.rs`:

```rust
//! Find box (`Cmd+F` / `Ctrl+Shift+F`) in the top-right corner, below the tab bar.

#[cfg(test)]
mod tests {
    use super::geometry;

    #[test]
    fn geometry_needs_room_for_the_box() {
        assert_eq!(geometry(80, 1, 30), Some((33, 1, 46)));
        assert_eq!(geometry(25, 1, 30), None, "narrower than 24 columns");
        assert_eq!(geometry(80, 1, 4), None, "box needs 3 rows below the tab bar");
    }
}
```

Run: `cargo test geometry_needs_room_for_the_box`
Expected: compile error `cannot find function geometry`.

- [ ] **Step 2: Move the code**

Move `search_box_geometry` (≈1547) and `build_search_box` (≈1622) above the test module. Rename them `geometry` and `build`; dedent; in `build`: new signature as above, first lines

```rust
let cols = cells.cols;
let Some((x0, y0, w)) = geometry(cols, top_rows, total_rows) else {
    return;
};
```

and replace

```rust
let queue = &self.queue;
let atlas = &mut self.atlas;
let instances = &mut self.instances;
```

with

```rust
let queue = cells.queue;
let atlas = &mut *cells.atlas;
let instances = &mut *cells.instances;
```

and the last two lines `*lo = (*lo).min(y0); *hi = (*hi).max(y0 + 2);` with `dirty.mark(y0, y0 + 2);`. Imports:

```rust
use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::atlas::GlyphStyle;
use crate::grid::char_width;
use crate::search::SearchBar;
```

The `format!` / `to_string()` / `Vec<char>` in `build` stay (pre-existing, Task 19 records them).

- [ ] **Step 3: Call it** — in `render_panes`:

```rust
if search.is_open {
    ui::search_box::build(&mut self.cells(cols), top_rows, total_rows, search, &mut dirty);
}
```

- [ ] **Step 4: Verify** — six commands; test total == `M + 2`. Checklist item 5 by eye.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/mod.rs src/renderer/ui/mod.rs src/renderer/ui/search_box.rs
git commit -m "refactor(render): move find box into ui module"
```

---

### Task 12: Move the inspector panel to `ui/inspector.rs`

**Files:**
- Create: `src/renderer/ui/inspector.rs`
- Modify: `src/renderer/ui/mod.rs`, `src/renderer/mod.rs`

**Interfaces:** `pub(in crate::renderer) fn build(cells: &mut Cells, top_rows: usize, total_rows: usize, lines: &[String], dirty: &mut DirtyRows)`

- [ ] **Step 1: Move `build_inspector`** (≈1553) into the new file:

```rust
//! Terminal inspector panel (`Opt+Cmd+I` / `Ctrl+Shift+I`) in the bottom-right corner.

use super::super::Instance;
use super::{Cells, DirtyRows};
use crate::atlas::GlyphStyle;
use crate::grid::char_width;
```

Same conversion as Task 11 Step 2: rename to `build`, `let cols = cells.cols;` first, the three `let queue / atlas / instances` lines → `cells.queue` / `&mut *cells.atlas` / `&mut *cells.instances`, and `*lo = (*lo).min(y0); *hi = (*hi).max(y0 + h - 1);` → `dirty.mark(y0, y0 + h - 1);`. Add `pub(super) mod inspector;` to `ui/mod.rs`.

- [ ] **Step 2: Call it** — in `render_panes`:

```rust
if let Some(lines) = inspector {
    ui::inspector::build(&mut self.cells(cols), top_rows, total_rows, lines, &mut dirty);
}
```

- [ ] **Step 3: Verify** — six commands; test total == `M + 2`. Checklist item 7.

- [ ] **Step 4: Commit**

```bash
git add src/renderer/mod.rs src/renderer/ui/mod.rs src/renderer/ui/inspector.rs
git commit -m "refactor(render): move inspector panel into ui module"
```

---

### Task 13: Move the history popup to `ui/history_popup.rs`

**Files:**
- Create: `src/renderer/ui/history_popup.rs`
- Modify: `src/renderer/ui/mod.rs`, `src/renderer/mod.rs`

**Interfaces:**
- `pub(in crate::renderer) fn geometry(cols: usize, total_rows: usize) -> Option<(usize, usize, usize, usize, usize)>` — pure; body of `Renderer::history_popup_geometry` after its `is_open` check and `grid_size()` call.
- `pub(in crate::renderer) fn build(cells: &mut Cells, geometry: (usize, usize, usize, usize, usize), total_rows: usize, search: &HistorySearch, dirty: &mut DirtyRows)`
- `pub fn Renderer::history_popup_geometry(&self, search: &HistorySearch) -> Option<(usize, usize, usize, usize, usize)>` **stays in `mod.rs`** (called from app pointer / keyboard code) as a thin wrapper:

```rust
/// Position and size of the history search popup `(x0, y0, width, height, visible_rows)`.
pub fn history_popup_geometry(&self, search: &HistorySearch) -> Option<(usize, usize, usize, usize, usize)> {
    if !search.is_open {
        return None;
    }
    let (cols, total_rows) = self.grid_size();
    ui::history_popup::geometry(cols, total_rows)
}
```

Subtlety preserved on purpose: the geometry uses `grid_size()` rows (window rows), while the builder clips against `render_panes`' `total_rows` (`grid_size_for_tabs(..) + top_rows`, one less than `grid_size().1` when the tab bar shows). The caller still computes the geometry through `self.history_popup_geometry`, exactly as `build_history_popup` did.

- [ ] **Step 1: Failing test first** — create the file with header and:

```rust
//! History search popup (`Ctrl+R`): query, fzf-style matches, preview, key hints.

#[cfg(test)]
mod tests {
    use super::geometry;

    #[test]
    fn geometry_centers_and_sizes_popup() {
        assert_eq!(geometry(100, 40), Some((7, 8, 85, 16, 9)));
        assert_eq!(geometry(60, 12), Some((4, 1, 51, 10, 6)), "short window: no preview, nudged off row 0");
        assert_eq!(geometry(29, 40), None, "needs 30 columns");
        assert_eq!(geometry(100, 5), None, "needs 6 rows");
    }
}
```

Add `pub(super) mod history_popup;` to `ui/mod.rs`. Run `cargo test geometry_centers_and_sizes_popup` → expected compile error `cannot find function geometry`.

- [ ] **Step 2: Move the code**

Move the body of `history_popup_geometry` from `if cols < 30 || total_rows < 6 {` to `Some((x0, y0, popup_w, popup_h, visible_rows))` into `geometry`, and replace the method in `mod.rs` with the wrapper above. Move `build_history_popup` (≈1767) as `build`:

- signature as above; first lines `let cols = cells.cols;` and `let (x0, y0, popup_w, popup_h, visible_rows) = geometry;` replacing the `let Some(...) = self.history_popup_geometry(search) else { return; };`
- `*lo = (*lo).min(y0); *hi = (*hi).max((y0 + popup_h - 1).min(total_rows.saturating_sub(1)));` → `dirty.mark(y0, (y0 + popup_h - 1).min(total_rows.saturating_sub(1)));`
- `let queue = &self.queue;` → `let queue = cells.queue;`
- then `perl -pi -e 's/self\.atlas\./cells.atlas./g; s/self\.instances/cells.instances/g' src/renderer/ui/history_popup.rs`; `grep -n self src/renderer/ui/history_popup.rs` → nothing.

Imports: `use super::super::Instance; use super::{Cells, DirtyRows}; use crate::history::HistorySearch;`. The `format!` / `Vec<char>` / `preview_lines: Vec<String>` stay (Task 19).

- [ ] **Step 3: Call it** — in `render_panes` replace the `build_history_popup` call:

```rust
if let Some(search) = history_search
    && search.is_open
    && let Some(geometry) = self.history_popup_geometry(search)
{
    ui::history_popup::build(&mut self.cells(cols), geometry, total_rows, search, &mut dirty);
}
```

- [ ] **Step 4: Verify** — six commands; test total == `M + 3`. Checklist item 6 incl. narrow-window hint sets; click a result row with the mouse (app code uses `history_popup_geometry`).

- [ ] **Step 5: Commit**

```bash
git add src/renderer/mod.rs src/renderer/ui/mod.rs src/renderer/ui/history_popup.rs
git commit -m "refactor(render): move history popup into ui module"
```

---

### Task 14: Move scrollbar, link preview and preedit into `ui/`

Contradiction with the outline: there is no scrollbar *builder*. The scrollbar is `scrollbar_thumb` (geometry shared with mouse dragging in the app) plus a 5-line instance write inside the per-pane loop; link preview and IME preedit are inline blocks in `render_panes` that the outline did not mention.

**Files:**
- Create: `src/renderer/ui/scrollbar.rs`, `src/renderer/ui/link_preview.rs`, `src/renderer/ui/preedit.rs`
- Modify: `src/renderer/ui/mod.rs`, `src/renderer/mod.rs`, `src/renderer/tests.rs`

**Interfaces:**
- `scrollbar.rs`: `pub(in crate::renderer) const SCROLLBAR_OVERLAY: u32`, `pub const SCROLLBAR_WIDTH: f64`, `pub fn scrollbar_thumb(track_height: f64, rows: usize, history: usize, offset: usize) -> (f64, f64)` (both re-exported: `pub use ui::scrollbar::{scrollbar_thumb, SCROLLBAR_WIDTH};` in `renderer/mod.rs`, so `renderer::SCROLLBAR_WIDTH` / `renderer::scrollbar_thumb` keep working for the app), and new `#[inline] pub(in crate::renderer) fn instance(right: f64, y: f64, height: f64, offset: usize) -> Instance`.
- `link_preview.rs`: `pub(in crate::renderer) fn build(cells: &mut Cells, total_rows: usize, link: &Hyperlink, dirty: &mut DirtyRows)`
- `preedit.rs`: `pub(in crate::renderer) fn build(cells: &mut Cells, preedit: &str, cursor: (usize, usize), dirty: &mut DirtyRows) -> (usize, usize)` — returns the cursor moved to the end of the composition (the moved body never reads `total_rows`; the bounds check stays in the caller's guard).

- [ ] **Step 1: Failing test for the scrollbar instance**

Create `scrollbar.rs` with a header and tests; add `pub(super) mod scrollbar;` to `ui/mod.rs`:

```rust
//! Per-pane scrollbar: thumb geometry (shared with mouse dragging) and its
//! dedicated instance, appended after the cells and drawn by the shader.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_packs_position_and_hides_thumb_at_bottom() {
        let i = instance(100.0, 20.0, 40.0, 3);
        assert_eq!(f32::from_bits(i.glyph), 90.0, "x = right edge - width");
        assert_eq!(f32::from_bits(i.fg), 20.0);
        assert_eq!(i.bg, SCROLLBAR_OVERLAY | (40 << 8));
        assert_eq!(instance(100.0, 20.0, 40.0, 0).bg, SCROLLBAR_OVERLAY, "no thumb when not scrolled back");
    }
}
```

Run `cargo test instance_packs_position` → compile error (no `instance`).

- [ ] **Step 2: Fill `scrollbar.rs`**

Move from `mod.rs` the doc comment + `const SCROLLBAR_OVERLAY` (≈28–29), `pub const SCROLLBAR_WIDTH` (≈30) and `scrollbar_thumb` (≈32–40) above the tests; make `SCROLLBAR_OVERLAY` `pub(in crate::renderer)`. Add:

```rust
use super::super::Instance;

/// The pane's scrollbar instance: x / y as f32 bits in `glyph` / `fg`, the
/// thumb height above the 0x30 tag in `bg` (0 = hidden at the bottom).
#[inline]
pub(in crate::renderer) fn instance(right: f64, y: f64, height: f64, offset: usize) -> Instance {
    Instance {
        glyph: ((right - SCROLLBAR_WIDTH) as f32).to_bits(),
        fg: (y as f32).to_bits(),
        bg: SCROLLBAR_OVERLAY | (if offset > 0 { (height as u32).min(0xFF_FFFF) } else { 0 } << 8),
    }
}
```

In `render_panes` the per-pane write becomes `self.instances[scrollbar_start + pane_index] = ui::scrollbar::instance(right, y, height, offset);`. Move `scrollbar_thumb_reaches_track_ends_and_keeps_caps_visible` from `renderer/tests.rs` into this file's tests. In `renderer/tests.rs` add `use super::ui::scrollbar::SCROLLBAR_OVERLAY;` (used by `precise_scroll_gpu_pixels`).

- [ ] **Step 3: `link_preview.rs`**

Header: `//! URL of the hovered link on the window's last row, blended with the default background.` Move the body of the `if let Some((_, link)) = hovered_link && total_rows > 0 && cols > 0 { ... }` block (≈1092–1128) into `build`, with `let cols = cells.cols;` first, `self.atlas.slot(&self.queue,` → `cells.atlas.slot(cells.queue,`, `self.instances` → `cells.instances`, and `lo = lo.min(preview_row); hi = hi.max(preview_row);` (already `dirty.mark(preview_row, preview_row);` since Task 10). Imports: `use super::super::Instance; use super::{Cells, DirtyRows}; use crate::grid::Hyperlink;`. The `const PREVIEW_BG` / `PREVIEW_FG` move inside. Caller keeps the guard:

```rust
if let Some((_, link)) = hovered_link
    && total_rows > 0 && cols > 0 {
        ui::link_preview::build(&mut self.cells(cols), total_rows, link, &mut dirty);
    }
```

- [ ] **Step 4: `preedit.rs`**

Header: `//! IME composition (Telex, Pinyin, ...) drawn underlined at the cursor.` Move the body of the `if let (false, Some((cx, cy))) = (...) && cols > 0 && cy < total_rows && cx < cols { ... }` block (≈1133–1158) into `build`, starting with `let cols = cells.cols; let (cx, cy) = cursor;`, same `self.*` → `cells.*` substitutions (`self.atlas.glyph_slots(&self.queue,` → `cells.atlas.glyph_slots(cells.queue,`), `lo`/`hi` → `dirty.mark(cy, cy)`, and returning `(col.min(cols.saturating_sub(1)), cy)` instead of assigning `cursor = Some(...)`. Imports: `use super::super::Instance; use super::{Cells, DirtyRows}; use crate::atlas::GlyphStyle; use crate::grid::{char_width, BG_UNDERLINE};`. Caller:

```rust
let mut cursor = active_cur;
if let (false, Some((cx, cy))) = (preedit.is_empty(), active_cur)
    && cols > 0 && cy < total_rows && cx < cols {
        cursor = Some(ui::preedit::build(&mut self.cells(cols), preedit, (cx, cy), &mut dirty));
    }
```

- [ ] **Step 5: Verify** — six commands; test total == `M + 4`. External caller grep: `renderer::SCROLLBAR_WIDTH` and `renderer::scrollbar_thumb` still resolve. GPU pixel image identical to `gpu-before.png` (`cmp`, as in Task 8). Checklist items 4 (scrollbar), 8, 9.

- [ ] **Step 6: Commit**

```bash
git add src/renderer/mod.rs src/renderer/tests.rs src/renderer/ui/mod.rs src/renderer/ui/scrollbar.rs src/renderer/ui/link_preview.rs src/renderer/ui/preedit.rs
git commit -m "refactor(render): move scrollbar, link preview and preedit to ui"
```

---

### Task 15: Move `build_pane_row` into `renderer/pane_row.rs` (verbatim)

Hot path; split in two commits so this one is a pure move that `--color-moved` shows as moved.

**Files:**
- Create: `src/renderer/pane_row.rs`
- Modify: `src/renderer/mod.rs`

**Interfaces:** `impl Renderer { #[allow(clippy::too_many_arguments)] pub(super) fn build_pane_row(&mut self, g: &Grid, src_row: usize, instance_start: Option<usize>, dst_row: usize, dst_col: usize, p_cols: usize, window_cols: usize, rect: &PaneRect, is_active: bool, active_rect: Option<&PaneRect>, hovered_link: Option<&Hyperlink>, pane_defaults: Option<(u32, u32)>) }` — unchanged except visibility.

- [ ] **Step 1: Move**

```bash
set -- $("$TMPDIR/phase3/span.sh" src/renderer/mod.rs '^    fn build_pane_row' '    ')
sed -n "$1p;$2p" src/renderer/mod.rs
printf '%s\n' '//! One pane row → cell instances: glyph slots (shaped, bidi, clusters, wide),' \
  '//! selection / find highlights, decorations, split divider flags.' '' \
  'use super::{Instance, Renderer, COLOR_GLYPH, UNFOCUSED};' \
  'use crate::atlas::GlyphStyle;' \
  'use crate::grid::{char_width, Grid, Hyperlink, SearchHit, BG_EXTRA, BG_SET, BG_STRIKE, BG_UNDERLINE_MASK, FG_BOLD, FG_EXTRA, FG_ITALIC, FG_SET, UNDERLINE_SINGLE};' \
  'use crate::split::PaneRect;' '' 'impl Renderer {' > src/renderer/pane_row.rs
"$TMPDIR/phase3/cut.sh" src/renderer/mod.rs "$1" "$2" src/renderer/pane_row.rs
printf '}\n' >> src/renderer/pane_row.rs
```

Add `pub(super)` to the method, `mod pane_row;` to `mod.rs`, prune imports in both files with clippy.

- [ ] **Step 2: Verify** — six commands; test total == `M + 4`. Then prove the body is byte-identical:

```bash
git show HEAD:src/renderer/mod.rs > "$TMPDIR/phase3/old-mod.rs"
set -- $("$TMPDIR/phase3/span.sh" "$TMPDIR/phase3/old-mod.rs" '^    fn build_pane_row' '    ')
sed -n "$1,$2p" "$TMPDIR/phase3/old-mod.rs" > "$TMPDIR/phase3/old-row.rs"
set -- $("$TMPDIR/phase3/span.sh" src/renderer/pane_row.rs 'fn build_pane_row' '    ')
sed -n "$1,$2p" src/renderer/pane_row.rs > "$TMPDIR/phase3/new-row.rs"
diff "$TMPDIR/phase3/old-row.rs" "$TMPDIR/phase3/new-row.rs"
```

Expected: exactly one changed line, `fn build_pane_row(` → `pub(super) fn build_pane_row(`.

- [ ] **Step 3: Commit**

```bash
git add src/renderer/mod.rs src/renderer/pane_row.rs
git commit -m "refactor(render): move pane row builder into module"
```

---

### Task 16: Narrow `build_pane_row` to the row inputs it reads

**Files:**
- Modify: `src/renderer/pane_row.rs`, `src/renderer/mod.rs`

**Interfaces:**
- Produces in `pane_row.rs`:

```rust
/// Inputs shared by every row of one pane in one frame.
#[derive(Clone, Copy)]
pub(super) struct PaneRowCtx<'a> {
    pub(super) rect: &'a PaneRect,
    pub(super) is_active: bool,
    pub(super) active_rect: Option<&'a PaneRect>,
    pub(super) hovered_link: Option<&'a Hyperlink>,
    /// This pane's OSC 10 / 11 defaults when they differ from the active pane's.
    pub(super) pane_defaults: Option<(u32, u32)>,
    /// Columns drawn: `rect.cols.min(grid.cols)`.
    pub(super) p_cols: usize,
    /// `selection-background` / `selection-foreground` instance colors.
    pub(super) selection_colors: (u32, u32),
}

/// Per-row buffers reused across rows and frames (never shrunk).
#[derive(Default)]
pub(super) struct RowScratch {
    /// Shaped slot keys of the row being built.
    pub(super) shaped: Vec<u32>,
    /// Bidi scratch: characters, levels and visual order.
    pub(super) bidi_chars: Vec<char>,
    pub(super) bidi_levels: Vec<u8>,
    pub(super) bidi_order: Vec<u16>,
}

impl Renderer {
    /// The cell instances and the row scratch, borrowed together.
    pub(super) fn row_target(&mut self, cols: usize) -> (Cells<'_>, &mut RowScratch) {
        (Cells { atlas: &mut self.atlas, queue: &self.queue, instances: &mut self.instances, cols }, &mut self.row_scratch)
    }
}

pub(super) fn build_pane_row(
    cells: &mut Cells,
    scratch: &mut RowScratch,
    g: &Grid,
    pane: &PaneRowCtx,
    src_row: usize,
    instance_start: Option<usize>,
    dst_row: usize,
)
```

- `Renderer` loses fields `shaped`, `bidi_chars`, `bidi_levels`, `bidi_order` and gains `row_scratch: RowScratch` (initialized `RowScratch::default()` in `new`, keeping the two doc comments on the new struct's fields).
- Dropped parameters and where their values come from: `dst_col` was always `pane.rect.col` → `pane.rect.col`; `window_cols` was always the frame's `cols` → `cells.cols`.

- [ ] **Step 1: Rewrite the head and tail of the function**

Turn the method into the free function above (dedent, remove `impl Renderer` wrapper around it, keep it the only item besides the new types / `row_target`). At the top of the body:

```rust
let (sel_bg, sel_fg) = pane.selection_colors;
// MATCH_* / CURRENT_* consts unchanged
let rect = pane.rect;
let row = g.display_row(src_row);
let hits = g.search_hits(src_row);
let start = instance_start.unwrap_or(dst_row * cells.cols + rect.col);
let is_link_row = pane.hovered_link.is_some_and(|l| l.row == src_row);
let is_last_row = src_row + 1 == rect.rows;
// is_border_right_active / is_border_bottom_active: `is_active` → `pane.is_active`,
// `active_rect` → `pane.active_rect`, otherwise unchanged
```

Replace the `mem::take` block and the `out` line:

```rust
// before
let (mut chars, mut levels, mut order) =
    (std::mem::take(&mut self.bidi_chars), std::mem::take(&mut self.bidi_levels), std::mem::take(&mut self.bidi_order));
let bidi = g.bidi_order(src_row, &mut chars, &mut levels, &mut order);
let mut shaped = std::mem::take(&mut self.shaped);
let any_shaped = !bidi && self.atlas.shape_row(row, &mut shaped);
let out = &mut self.instances[start..start + p_cols];

// after
let RowScratch { shaped, bidi_chars: chars, bidi_levels: levels, bidi_order: order } = scratch;
let bidi = g.bidi_order(src_row, chars, levels, order);
let any_shaped = !bidi && cells.atlas.shape_row(row, shaped);
let out = &mut cells.instances[start..start + pane.p_cols];
```

Delete the two restore lines at the end (`self.shaped = shaped;` and `(self.bidi_chars, self.bidi_levels, self.bidi_order) = (chars, levels, order);`). In the loop:

```bash
perl -pi -e 's/self\.atlas\.(\w+)\(&self\.queue,/cells.atlas.$1(cells.queue,/g; s/self\.atlas\.is_color/cells.atlas.is_color/g' src/renderer/pane_row.rs
perl -pi -e 's/\bhovered_link\.is_some_and\(\|l\| col/pane.hovered_link.is_some_and(|l| col/; s/= pane_defaults \{/= pane.pane_defaults {/; s/if !is_active \{/if !pane.is_active {/' src/renderer/pane_row.rs
grep -nE '\bself\b|\bp_cols\b|\bwindow_cols\b|\bdst_col\b' src/renderer/pane_row.rs
```

Expected: only `row_target`'s `self` lines and the `pane.p_cols` use remain. Everything else in the loop is byte-identical. The pre-existing `joined: String` for Arabic clusters stays (Task 19).

- [ ] **Step 2: Update the four call sites in `render_panes`**

Both the per-pane build loop and the eviction rebuild loop compute `pane_link`, `pane_defaults`, `p_cols`, `p_rows` per pane. Right after those, add:

```rust
let row_ctx = PaneRowCtx {
    rect: &pane.rect,
    is_active: pane.is_active,
    active_rect: active_rect.as_ref(),
    hovered_link: pane_link,
    pane_defaults,
    p_cols,
    selection_colors: self.selection_colors,
};
```

In the build loop, after the image placements (`self.placed.push` loop) — so no other `self` borrow is live — take `let (mut cells, scratch) = self.row_target(cols);` and replace each `self.build_pane_row(&g, X, START, DST, pane.rect.col, p_cols, cols, &pane.rect, pane.is_active, active_rect.as_ref(), pane_link, pane_defaults)` with `build_pane_row(&mut cells, scratch, &g, &row_ctx, X, START, DST)`. Nothing else in those loops touches `self`, so the borrow is valid until the end of the pane iteration. In the eviction loop take `row_target` after `let extra = self.viewports[pane_index].extra_start as usize;`.

`use pane_row::{build_pane_row, PaneRowCtx, RowScratch};` in `mod.rs`; `use super::ui::Cells;` in `pane_row.rs`.

- [ ] **Step 3: Verify** — six commands; test total == `M + 4`. GPU pixel image identical (`cmp`, as in Task 8).

- [ ] **Step 4: Hot-path gate** (first after-measurement of the renderer)

```bash
cargo build --profile fast-release
for i in 1 2 3; do ./target/fast-release/litty +bench-scroll-fps > "$TMPDIR/phase3/scroll-t16-$i.jsonl"; done
python3 "$TMPDIR/phase3/compare_scroll.py" "$TMPDIR/phase3/scroll-before" "$TMPDIR/phase3/scroll-t16"
```

Expected: every line `OK`, exit 0. On `FAIL`: re-run `+bench-stress` as noise control; if the control is stable and the scroll numbers still fail, diff `pane_row.rs` against the Task 15 commit for any change beyond the listed substitutions before touching anything else. Visual checklist items 1, 4, 5 (find highlights), 8, 10 → `after16-NN-*.png`.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/mod.rs src/renderer/pane_row.rs
git commit -m "refactor(render): borrow only row inputs in pane row builder"
```

---

### Task 17: Move `render_panes` into `renderer/frame.rs` (verbatim)

**Files:**
- Create: `src/renderer/frame.rs`
- Modify: `src/renderer/mod.rs`

**Interfaces:** `pub fn Renderer::render_panes(&mut self, panes: &[RenderPane], tab_bar: Option<(&[String], usize)>, hovered_link: Option<(u64, &Hyperlink)>, cursor_blink_on: bool, history_search: Option<&HistorySearch>, search: &SearchBar, preedit: &str, inspector: Option<&[String]>)` — unchanged (a `pub` method in a child module's `impl Renderer` is as visible as before).

- [ ] **Step 1: Move**

```bash
set -- $("$TMPDIR/phase3/span.sh" src/renderer/mod.rs '^    pub fn render_panes' '    ')
sed -n "$1p;$2p" src/renderer/mod.rs     # '#[allow(clippy::too_many_arguments)]' and '    }'
printf '%s\n' '//! One frame: decide whether anything changed, rebuild dirty instance rows,' \
  '//! draw overlays, upload, and submit the cell / image / custom-shader passes.' '' \
  'impl Renderer {' > "$TMPDIR/phase3/frame-body.rs"
"$TMPDIR/phase3/cut.sh" src/renderer/mod.rs "$1" "$2" "$TMPDIR/phase3/frame-body.rs"
printf '}\n' >> "$TMPDIR/phase3/frame-body.rs"
mv "$TMPDIR/phase3/frame-body.rs" src/renderer/frame.rs
```

Then insert the imports below the `//!` lines (prune with clippy):

```rust
use super::gpu::{make_instance_buf, make_viewport_buffer};
use super::pane_row::{build_pane_row, PaneRowCtx};
use super::ui::{self, DirtyRows};
use super::{scrollbar_thumb, Instance, RenderPane, Renderer, Uniforms, Viewport};
use crate::grid::Hyperlink;
use crate::history::HistorySearch;
use crate::search::SearchBar;
```

Add `mod frame;` to `mod.rs`; drop the imports `mod.rs` no longer needs.

- [ ] **Step 2: Verify** — six commands; test total == `M + 4`. Byte-identical body check, as in Task 15 Step 2:

```bash
git show HEAD:src/renderer/mod.rs > "$TMPDIR/phase3/old-mod.rs"
set -- $("$TMPDIR/phase3/span.sh" "$TMPDIR/phase3/old-mod.rs" '^    pub fn render_panes' '    ')
sed -n "$1,$2p" "$TMPDIR/phase3/old-mod.rs" > "$TMPDIR/phase3/old-frame.rs"
set -- $("$TMPDIR/phase3/span.sh" src/renderer/frame.rs '^    pub fn render_panes' '    ')
sed -n "$1,$2p" src/renderer/frame.rs > "$TMPDIR/phase3/new-frame.rs"
diff "$TMPDIR/phase3/old-frame.rs" "$TMPDIR/phase3/new-frame.rs" && echo identical
```

Expected: `identical`.

- [ ] **Step 3: Commit**

```bash
git add src/renderer/mod.rs src/renderer/frame.rs
git commit -m "refactor(render): move render_panes into frame module"
```

---

### Task 18: Split `render_panes` into named frame phases

**Files:**
- Modify: `src/renderer/frame.rs`

**Interfaces** (all private to `frame.rs` except `render_panes`):

```rust
/// Instance-buffer layout of one frame.
#[derive(Clone, Copy)]
struct FrameLayout {
    cols: usize,
    /// 1 when the tab bar takes the first row.
    top_rows: usize,
    total_rows: usize,
    /// `cols * total_rows`: first instance after the window grid (extra bottom rows follow).
    grid_cells: usize,
    /// First scrollbar instance, after every pane's extra bottom row.
    scrollbar_start: usize,
    instance_count: usize,
    pane_count: usize,
}

/// What the active pane contributes to this frame, read under one short lock.
struct ActivePane {
    cursor: Option<(usize, usize)>,
    dirty: bool,
    defaults: (u32, u32),
    cursor_color: u32,
    progress: Option<(u8, u8)>,
    cursor_style: u32,
}

/// Per-frame inputs shared by the instance-building phases.
#[derive(Clone, Copy)]
struct FrameCtx<'a> {
    layout: FrameLayout,
    show_tabs: bool,
    tab_bar: Option<(&'a [String], usize)>,
    hovered_link: Option<(u64, &'a Hyperlink)>,
    /// Active pane's default fg / bg (OSC 10 / 11), sent to the shader.
    defaults: (u32, u32),
    active_rect: Option<PaneRect>,
}

/// Overlay state passed through from the caller.
struct OverlayInputs<'a> {
    preedit: &'a str,
    search: &'a SearchBar,
    history_search: Option<&'a HistorySearch>,
    inspector: Option<&'a [String]>,
}

fn snap_fractional_scroll(panes: &[RenderPane])

impl Renderer {
    pub fn render_panes(/* unchanged */)
    fn note_overlay_inputs(&mut self, inspector_open: bool, preedit: &str, hovered_link: Option<(u64, &Hyperlink)>)
    fn frame_layout(&self, panes: &[RenderPane], show_tabs: bool) -> FrameLayout
    fn read_active_pane(&self, pane: &RenderPane, top_rows: usize) -> ActivePane
    fn note_active_changes(&mut self, active: &ActivePane, active_id: u64)
    fn nothing_changed(&self, panes: &[RenderPane], active_id: u64, active: &ActivePane, layout: &FrameLayout, cursor_visible: bool) -> bool
    fn acquire_frame(&mut self) -> Option<wgpu::SurfaceTexture>
    fn ensure_buffers(&mut self, instance_count: usize, pane_count: usize)
    fn collect_pane_instances(&mut self, panes: &[RenderPane], ctx: &FrameCtx, full: bool, dirty: &mut DirtyRows)
    fn rebuild_after_eviction(&mut self, panes: &[RenderPane], ctx: &FrameCtx, dirty: &mut DirtyRows)
    fn collect_overlays(&mut self, ctx: &FrameCtx, cursor: Option<(usize, usize)>, overlays: &OverlayInputs, dirty: &mut DirtyRows) -> Option<(usize, usize)>
    fn upload_instances(&self, layout: &FrameLayout, dirty: DirtyRows)
    fn frame_uniforms(&self, ctx: &FrameCtx, active: &ActivePane, active_pane: &RenderPane, cursor: Option<(usize, usize)>, cursor_visible: bool) -> Uniforms
    fn submit(&mut self, frame: wgpu::SurfaceTexture, uniforms: Uniforms, clear_bg: u32, instance_count: usize)
}
```

`PaneRect` is `Copy` (the current code already copies it with `.map(|p| p.rect)`).

- [ ] **Step 1: Write the orchestrator**

Replace the body of `render_panes` with the following; every phase body is the original code block named in Step 2, so side effects keep their order (begin_frame → tab bar → panes → eviction → `force_full = false` / `last_cursor` → overlays → upload → `last_cursor_visible` → uniforms → viewports upload → image prepare → passes → submit → present → capture):

```rust
    #[allow(clippy::too_many_arguments)]
    pub fn render_panes(
        &mut self,
        panes: &[RenderPane],
        tab_bar: Option<(&[String], usize)>,
        hovered_link: Option<(u64, &Hyperlink)>,
        cursor_blink_on: bool,
        history_search: Option<&HistorySearch>,
        search: &SearchBar,
        preedit: &str,
        inspector: Option<&[String]>,
    ) {
        let capture_start = self.frame_capture.as_ref().map(|_| std::time::Instant::now());
        if panes.is_empty() {
            return;
        }
        self.note_overlay_inputs(inspector.is_some(), preedit, hovered_link);
        // Overlays are redrawn every frame on top of the cells.
        let is_search_open = history_search.is_some_and(|h| h.is_open) || search.is_open;
        // Cell-based overlays stay fixed while open; snap once when opening them.
        if is_search_open || inspector.is_some() {
            snap_fractional_scroll(panes);
        }
        let layout = self.frame_layout(panes, tab_bar.is_some());
        let active_pane = panes.iter().find(|p| p.is_active).unwrap_or(&panes[0]);
        let active = self.read_active_pane(active_pane, layout.top_rows);
        self.note_active_changes(&active, active_pane.id);
        let effective_visible = active.cursor.is_some() && cursor_blink_on;

        // Fast check before acquiring a Metal/Vulkan swapchain drawable
        if !self.force_full
            && !is_search_open
            && !self.animate
            && self.nothing_changed(panes, active_pane.id, &active, &layout, effective_visible)
        {
            return;
        }

        let acquire_start = capture_start.map(|_| std::time::Instant::now());
        let Some(frame) = self.acquire_frame() else { return };
        let acquire_ms = acquire_start.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);

        self.ensure_buffers(layout.instance_count, panes.len());
        self.viewports.clear();
        self.atlas.begin_frame();
        let full = self.force_full;
        let mut dirty = DirtyRows::NONE;
        let ctx = FrameCtx {
            layout,
            show_tabs: tab_bar.is_some(),
            tab_bar,
            hovered_link,
            defaults: active.defaults,
            active_rect: panes.iter().find(|p| p.is_active).map(|p| p.rect),
        };

        if let Some((titles, active_idx)) = tab_bar
            && full {
                ui::tab_bar::build(&mut self.cells(layout.cols), titles, active_idx);
                dirty.lo = 0;
                dirty.hi = 0;
            }
        self.collect_pane_instances(panes, &ctx, full, &mut dirty);
        if self.atlas.take_evicted() {
            self.rebuild_after_eviction(panes, &ctx, &mut dirty);
        }

        self.force_full = false;
        self.last_cursor = active.cursor;

        let overlays = OverlayInputs { preedit, search, history_search, inspector };
        let cursor = self.collect_overlays(&ctx, active.cursor, &overlays, &mut dirty);
        self.upload_instances(&layout, dirty);
        let effective_cursor_visible = if is_search_open { false } else { effective_visible };
        self.last_cursor_visible = effective_cursor_visible;
        let uniforms = self.frame_uniforms(&ctx, &active, active_pane, cursor, effective_cursor_visible);
        self.submit(frame, uniforms, active.defaults.1, layout.instance_count);
        if let (Some(capture), Some(start)) = (&mut self.frame_capture, capture_start) {
            let now = std::time::Instant::now();
            capture.record(now, now.duration_since(start).as_secs_f64() * 1000.0, acquire_ms);
        }
    }
```

- [ ] **Step 2: Fill each phase from the original block** (references are to the original `render_panes`, ≈805–1309 in `d1d0c92`; take the code from `git show HEAD:src/renderer/frame.rs`)

| Phase | Original block | Substitutions |
| --- | --- | --- |
| `note_overlay_inputs` | from `// The inspector's content changes` to the end of the `last_hovered_link` `if`, minus the `let top_rows` line | `inspector.is_some()` → `inspector_open` |
| `snap_fractional_scroll` (free fn) | the `for pane in panes { ... }` loop inside `if is_search_open \|\| inspector.is_some()` | none |
| `frame_layout` | `let (cols, term_rows) = ...` through `let instance_count = ...` plus `top_rows` | builds `FrameLayout { cols, top_rows: usize::from(show_tabs), total_rows, grid_cells: n, scrollbar_start, instance_count, pane_count: panes.len() }` |
| `read_active_pane` | the `{ let g = active_pane.grid.lock(); ... }` block | `active_pane` → `pane`; returns `ActivePane { cursor: cur, dirty, defaults: (g.fg_color(), g.bg_color()), cursor_color, progress: g.progress, cursor_style }` |
| `note_active_changes` | the three `if progress != ...`, `if defaults != ...`, `if active_pane.id != ...` blocks | `progress` → `active.progress`, `defaults` → `active.defaults`, `active_pane.id` → `active_id` |
| `nothing_changed` | the `let any_dirty = ...` expression and the `if !any_dirty && ...` condition | `active_dirty` → `active.dirty`, `active_pane.id` → `active_id`, `instance_count` → `layout.instance_count`, `active_cur` → `active.cursor`; returns the condition |
| `acquire_frame` | the `match self.surface.get_current_texture()` | `Ok(f) => Some(f)`; both error arms keep their side effects and end with `None` |
| `ensure_buffers` | `if self.instances.len() != instance_count { ... }` and `if self.viewport_buf.size() < ... { ... }` | `panes.len()` → `pane_count` |
| `collect_pane_instances` | `self.placed.clear();`, `let (cw, ch) = ...`, then the whole `for (pane_index, pane) in panes.iter().enumerate() { ... }` build loop; starts with `let mut extra_start = ctx.layout.grid_cells;` | `top_rows` / `total_rows` / `cols` / `scrollbar_start` → `ctx.layout.*`; `tab_bar.is_some()` → `ctx.show_tabs`; `defaults` → `ctx.defaults`; `hovered_link` → `ctx.hovered_link`; `active_rect.as_ref()` → `ctx.active_rect.as_ref()`; `lo`/`hi` already `dirty` |
| `rebuild_after_eviction` | the body of `if self.atlas.take_evicted() { ... }` (the `for _ in 0..3` loop and the final `if total_rows > 0 { dirty.lo = 0; dirty.hi = total_rows - 1; }`) | same as above; `tab_bar` → `ctx.tab_bar` |
| `collect_overlays` | from `if let Some((_, link)) = hovered_link` through the inspector call; returns `cursor` | `hovered_link` → `ctx.hovered_link`; `active_cur` → the `cursor` parameter; `preedit` / `search` / `history_search` / `inspector` → `overlays.*`; `cols` / `total_rows` / `top_rows` → `ctx.layout.*` |
| `upload_instances` | the `if total_rows > 0 && cols > 0 && dirty.lo <= dirty.hi { ... }` block and the following `write_buffer` of `self.instances[n..]` | `n` → `layout.grid_cells`, `cols` / `total_rows` → `layout.*` |
| `frame_uniforms` | `let (cx, cy) = cursor.unwrap_or((0, 0));`, the `tab_info` match, `let (underline_y, strike_y) = ...`, and the `Uniforms { ... }` literal | `tab_bar` → `ctx.tab_bar`; `defaults` → `ctx.defaults`; `cursor_color` / `progress` / `active_cursor_style` → `active.*`; `effective_cursor_visible` → `cursor_visible`; `top_rows` / `cols` → `ctx.layout.*`; `panes.len()` → `ctx.layout.pane_count`; `active_pane.rect` unchanged (it is the fallback pane when none is active, unlike `ctx.active_rect`) |
| `submit` | from `self.queue.write_buffer(&self.viewport_buf, ...)` through `frame.present();` | `u` → `uniforms`; `defaults.1` → `clear_bg`; `instance_count` parameter |

Lock check — each `.lock()` must appear in exactly one phase and in the same loop as before:

```bash
grep -n 'lock()' src/renderer/frame.rs
```

Expected: 5 lines — `snap_fractional_scroll`, `read_active_pane`, `nothing_changed`, `collect_pane_instances`, `rebuild_after_eviction`.

Allocation / dispatch check — the diff must not add any of these to `frame.rs`:

```bash
git diff HEAD -- src/renderer/frame.rs | grep -E '^\+' | grep -nE 'Vec::|vec!|String|format!|to_string|collect|Box|dyn |clone\(\)|write_buffer|draw\('
```

Expected: no output, except lines that also appear as `-` (moved), i.e. none of them new: cross-check each hit with `git diff -M --color-moved=zebra HEAD -- src/renderer/frame.rs`.

- [ ] **Step 3: Verify** — six commands; test total == `M + 4`.

```bash
awk '/pub fn render_panes/,/^    }$/' src/renderer/frame.rs | wc -l
grep -cE '^    (pub )?fn ' src/renderer/frame.rs
LITTY_SCROLL_TEST_IMAGE="$TMPDIR/phase3/gpu-t18.png" cargo test precise_scroll_gpu_pixels -- --ignored --nocapture
cmp "$TMPDIR/phase3/gpu-before.png" "$TMPDIR/phase3/gpu-t18.png" && echo identical
```

Expected: `render_panes` ≈ 80 lines (was 505); 14 methods; `identical`.

- [ ] **Step 4: Final performance and visual gate**

```bash
cargo build --profile fast-release
for i in 1 2 3; do ./target/fast-release/litty +bench-scroll-fps > "$TMPDIR/phase3/scroll-after-$i.jsonl"; done
python3 "$TMPDIR/phase3/compare_scroll.py" "$TMPDIR/phase3/scroll-before" "$TMPDIR/phase3/scroll-after"
for i in 1 2 3; do ./target/fast-release/litty +bench-stress 20000 | grep -E 'Throughput|Worst'; done | tee "$TMPDIR/phase3/stress-after.txt"
./target/fast-release/litty +bench-ram && cp /tmp/litty_ram_bench.json "$TMPDIR/phase3/ram-after.json"
cat "$TMPDIR/phase3/ram-before.json" "$TMPDIR/phase3/ram-after.json"
for i in 1 2 3; do cargo test --release benchmark_before_vs_after -- --nocapture --test-threads=1 2>&1 | grep -A2 '^\[8\]'; done | tee "$TMPDIR/phase3/coretext-after.txt"
```

Expected:
- `compare_scroll.py` exits 0 (all three metrics within ±3% per scenario, all runs valid).
- Stress throughput (control) within ±3% of `stress-before.txt`; otherwise the machine was noisy — re-take before and after.
- RAM: `window_1_tab_1_fp_mb` and `window_2_tab_6_fp_mb` within ±3% (or ±1.5 MB, whichever is larger) of `ram-before.json`; `"passed":true`.
- CoreText [8] "Sau" median within ±3% of `coretext-before.txt`.

Run the full visual checklist (items 1–12) as `after-NN-*.png` and compare each with `before-NN-*.png`. Also run once with `background-opacity = 0.85` and once with a `custom-shader` from your config if you use one (post-process path in `submit`). Record numbers and the checklist result in the commit body.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/frame.rs
git commit -m "refactor(render): split render_panes into frame phases" -m "Scroll FPS, RAM and CoreText timings within 3% of baseline; GPU
pixel test image identical; visual checklist 1-12 matches."
```

---

### Task 19: Docs and follow-ups

**Files:**
- Modify: `CLAUDE.md` (Architecture & Modules), `ROADMAP.md`

- [ ] **Step 1: `CLAUDE.md`**

Replace the line

```markdown
- `src/renderer.rs` & `src/atlas.rs`: wgpu pipelines, single instanced draw call (`shader.wgsl`), glyph cache, font fallback.
```

with

```markdown
- `src/renderer/`: `Renderer` API (`mod.rs`), wgpu setup (`gpu.rs`), per-frame phases (`frame.rs`), pane rows (`pane_row.rs`), overlay widgets over a borrowed `Cells` context (`ui/`); one instanced cell draw per layer (`shader.wgsl`).
- `src/atlas/`: glyph cache and slot LRU (`mod.rs`), rasterization (`raster.rs`), procedural sprites, ligature shaper, font discovery, CoreText (`coretext.rs`) / non-macOS (`linux.rs`) system rasterizers.
```

- [ ] **Step 2: `ROADMAP.md`**

Next to the Phase 1 note (code-health section) add:

```markdown
- Phase 3 done: `renderer.rs` → `renderer/` (gpu, frame phases, pane_row, ui widgets) and `atlas.rs` → `atlas/` (raster, sprites, shaper, fonts, coretext / linux) (plan: `docs/plans/2026-11-01-split-renderer-atlas.md`). Next: phase 4 layered directories.
- [ ] Render loop: remove per-frame heap allocations found during phase 3 — tab bar `Vec<char>` per title (`ui/tab_bar.rs`), find box status `format!` / `to_string` and query `Vec<char>` (`ui/search_box.rs`), history popup `format!` badges, per-item `Vec<char>` and `preview_lines: Vec<String>` (`ui/history_popup.rs`), inspector title `format!` (`ui/inspector.rs`), hovered-URL `format!` / `clone` (`ui/link_preview.rs`), Arabic joined-cluster `String` (`pane_row.rs`).
- [ ] Share the tab span math (`c0`, `c1`, close / badge reserve) between `Renderer::hit_test_tab_bar` and `ui::tab_bar::build`.
- [ ] Phase 4: one CoreFoundation / CoreGraphics FFI module shared by `atlas/coretext.rs`, `macos.rs` (`NSRect` / `NSPoint` are layout-identical to `CGRect` / `CGPoint`) and the CoreText benchmark's own extern block in `app/latency_benchmarks.rs`.
```

- [ ] **Step 3: Final size check**

```bash
wc -l src/renderer/*.rs src/renderer/ui/*.rs src/atlas/*.rs | sort -rn
```

Expected (±15%): `atlas/sprites.rs` ~775, `atlas/mod.rs` ~730, `renderer/mod.rs` ~640, `renderer/frame.rs` ~500, `atlas/coretext.rs` ~400, `atlas/raster.rs` ~390, `renderer/ui/history_popup.rs` ~350, `atlas/fonts.rs` ~235, `renderer/gpu.rs` ~220, `renderer/pane_row.rs` ~190, `atlas/tests.rs` ~180, `ui/search_box.rs` ~150, `atlas/shaper.rs` ~145, `renderer/tests.rs` ~140, `ui/tab_bar.rs` ~110, `ui/inspector.rs` ~80, `ui/mod.rs` ~70, `ui/scrollbar.rs` ~60, `atlas/linux.rs` ~50, `ui/link_preview.rs` ~50, `ui/preedit.rs` ~45. No file above ~800 lines.

- [ ] **Step 4: Verify** — six commands; test total == `M + 4`.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md ROADMAP.md
git commit -m "docs(render): describe renderer and atlas module layout"
```

---

## Notes for Phase 4 (from reading `src/macos.rs`; not part of this plan)

- `macos.rs` is already split into `mod imp` (macOS) and `mod stub` (no-op twins) re-exported with `pub use`; `atlas/` now follows the same "same API, two files" shape (`coretext.rs` / `linux.rs`), so Phase 4 can place both under `src/platform/{macos,linux}/`.
- Overlap with the atlas FFI: `macos.rs` declares `NSRect { x, y, w, h }` and `NSPoint { x, y }` (same `repr(C)` layout as `CGRect` / `CGPoint`) and links `CoreGraphics` for `CGEventSourceFlagsState`; `atlas/coretext.rs` links `CoreFoundation`, `CoreGraphics`, `CoreText`; `app/latency_benchmarks.rs` declares its own `CFStringCreateWithCharacters` / `CFRelease` / `CTFontCreateWithName`. A shared `platform/macos/cf.rs` (geometry types, `CFRelease`, UTF-16 `CFString` helper) would remove the remaining duplicates.
- `pub(in crate::renderer)` in `renderer/ui/*.rs` must be rewritten to the new path when `renderer/` moves under `src/render/`.
