# Full feature test plan — macOS and Linux (2026-11-23)

Status: planned, not run. Written from a review of the source at `9fd8ab4`
(v0.14.1 plus release documentation), `docs/guide/`, `scripts/` and the open
roadmap items. No case below has been executed yet.

Issues go to the [issue note](../validation/2026-11-23-full-test-issues.md).

## 0. Scope and rules

- **Where**: every phase runs on real machines: a real Mac and real Linux
  desktops with a hardware GPU, a physical keyboard, pointer and display. CI
  and containers do not count as a run of this plan, except where a case
  explicitly runs a container script.
- **What**: every user-visible feature, CLI command, config key, keybinding,
  escape sequence and platform integration, plus the `CLAUDE.md` invariants
  (performance, 12-byte `Cell`, locking, `fork()` safety, paste/URL/history
  security) and packaging, install and update.
- **Excluded** (decided 2026-11-23): `yo` with live AI providers. Offline AI
  checks (section W) still run.
- **Do not fix during the run.** When a result differs from "Expected", log an
  issue in the issue note (template there), put its ID in the result cell
  (`F ISSUE-007`) and continue. A case that cannot run is `B` with the reason.
  Fixes start only after the whole plan has run (section 10).
- **Results**: fill the platform columns. `P` pass, `F` fail + issue ID, `B`
  blocked + reason, `S` skipped + reason, `—` not applicable. Blank = not run.
  Columns: **M** macOS (packaged `Litty.app` unless the case says the raw
  binary), **X** Linux with the default X11 backend, **W** Linux with
  `LITTY_BACKEND=wayland`.
- **Unclear expected result**: log it as an issue of type "spec question"
  instead of guessing; do not mark the case `P`.

### Key notation

| Name | macOS | Linux |
| --- | --- | --- |
| `TermMod` | `Cmd` | `Ctrl+Shift` (or `Super`) |
| `UIMod` | `Cmd` | `Ctrl` (or `Super`) |
| `LinkMod` | `Cmd` | `Ctrl` (or `Super`) |

Where the two platforms differ beyond this table, both are written out.

## 1. Environments

| ID | Machine | Required | Notes |
| --- | --- | --- | --- |
| E-M1 | Apple Silicon, current macOS 27.x | yes | Packaged app from `./bundle_macos.sh --no-install`, plus the raw `target/release/litty` |
| E-M2 | macOS 14 or an Intel Mac | optional | Older-OS activation limits, `x86_64` updater asset |
| E-L1 | Ubuntu 24.04+ GNOME, Wayland session | yes | X11 backend through XWayland and the Wayland backend |
| E-L2 | KDE Plasma 6, X11 and Wayland sessions | yes | Native X11 session for XIM, `VisibilityNotify`, non-GNOME focus policy |
| E-L3 | sway (wlroots) | yes for W | Open v0.15.0 items: IME, clipboard, drops, tiling |
| E-L4 | Fedora (current) | yes for RPM | RPM install and runtime |
| E-HW | Second monitor with a different scale | yes | Mixed-DPI moves on both OSes |

Record per run: `git rev-parse HEAD`, `rustc -V`, OS version, desktop and
compositor version, GPU and driver (`system_profiler SPDisplaysDataType`,
`vulkaninfo --summary`), display scales, keyboard layout, versions of every
shell and tool below.

### Prerequisites

- Shells: zsh, macOS `/bin/bash` 3.2, bash 5, fish 3.x and 4.x, nushell
  (≥ 0.101, current), elvish.
- Programs: vim, nvim (≥ 0.10), tmux, htop, less, man, fzf, `ls --hyperlink`
  (GNU coreutils), `rg`, `delta`, `kitten` (kitty), `timg`, `chafa`,
  `img2sixel` (libsixel), python3.
- Linux: ibus + ibus-unikey, fcitx5 (optional), zenity, `notify-send`
  (libnotify-bin), `paplay`, xdg-utils, xclip, wl-clipboard, xprop, xdotool,
  wtype, Docker or Podman, dpkg-deb, rpm-build, desktop-file-utils.
- macOS: Vietnamese Telex and a CJK input source enabled; Xcode command line
  tools; `footprint` (ships with macOS).
- Rust toolchain; `rustup target add x86_64-unknown-linux-gnu` on the Mac.

## 2. Run order

1. P0 preparation → P1 build gates → P2 automated suites → P3 performance.
2. P4 functional sections A–W on M, then X, then W.
3. P5 packaging, install and update; P6 documentation consistency.
4. Section 10: summary and triage.

Run P3 before P4: functional tests change config and leave windows open.

## 3. P0 — Preparation

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P0-01 | Record the environment data from section 1 at the top of the issue note | Recorded | | | |
| P0-02 | Back up `~/.litty`, `~/.cache/litty`, `~/.config/ghostty/themes`, the shell history files; move `~/.litty/config` aside | Litty starts with no config | | | |
| P0-03 | `cargo build --release`; macOS: `./bundle_macos.sh --no-install`; Linux: `bash bundle_linux.sh` | All succeed; record binary sizes | | | |
| P0-04 | Create fixtures (appendix A) in `~/litty-test` | Present | | | |
| P0-05 | Define the helpers `q` and `k` (appendix B) in each shell under test | `q '\e[5n'` prints `^[[0n` | | | |
| P0-06 | Set `auto-update = off` until section R | No background check during the run | | | |

## 4. P1 — Build and static gates

Run on M and on a Linux machine (X column). These are the `CLAUDE.md`
verification steps plus release-only checks.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P1-01 | `cargo check` | No errors | | | — |
| P1-02 | `cargo check --target x86_64-unknown-linux-gnu` | No errors | | | — |
| P1-03 | `cargo clippy --all-targets -- -D warnings` | Zero warnings | | | — |
| P1-04 | `cargo test` in a normal desktop session (needs `openpty`) | 0 failed; record passed/ignored counts (v0.14.1: macOS 722 unit + 1 + 14, Linux 876 unit + 1 + 14) | | | — |
| P1-05 | `cargo test --release` | 0 failed | | | — |
| P1-06 | macOS: `cargo test --test native_activation` | Pass | | — | — |
| P1-07 | `git diff --check` on a clean tree | No output | | | — |
| P1-08 | `cargo test layering` (`src/layering_tests.rs`) | Pass; `TOLERATED` entries still have a reason | | | — |
| P1-09 | Confirm `src/terminal/grid/cell.rs` keeps `const _: () = assert!(size_of::<Cell>() == 12)`; `cargo test precise_scroll_allocates_nothing_and_preserves_cell_layout` | Assert present (P1-01 compiles it); test passes | | | — |
| P1-10 | `ls -l target/release/litty` (and the app binary) | < 5,000,000 bytes | | | — |
| P1-11 | Linux: `ldd target/release/litty` | Record the list; libX11, libxkbcommon, libwayland and libvulkan are opened with `dlopen` at run time, so they should not be listed | — | | — |
| P1-12 | `cargo build` with the CI flags `RUSTFLAGS=-D warnings` | Builds | | | — |

## 5. P2 — Automated suites on real hardware

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P2-01 | `cargo test -- --ignored precise_scroll_gpu_pixels --nocapture` | Pass with pixel checks, no validation errors | | | — |
| P2-02 | `cargo test -- --ignored warm_overlay_builders_do_not_allocate` | Pass | | | — |
| P2-03 | Linux: `cargo test -- --ignored an_uploaded_image_is_drawn_in_the_same_frame a_given_up_frame_still_runs_its_uploads_and_signals_its_fence` with the Khronos validation layer enabled (`VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`) | Pass, zero VUID messages | — | | |
| P2-04 | Linux: `cargo test -- --ignored glyph_upload_cost --nocapture` | Record numbers (reference: 1024 glyphs ≈ 8 ms on Intel UHD 620) | — | | — |
| P2-05 | Linux: `cargo test -- --ignored linux_xdg_open_dispatch_smoke --nocapture` | Pass | — | | — |
| P2-06 | Linux X11: `cargo test -- --ignored set_fullscreen_is_reported_back_by_the_window_manager` | Pass | — | | — |
| P2-07 | Remaining ignored probes: `grid_throughput_probe`, `resize_probe`, `benchmark_*`, `perf_legit_images`, `theme_lookup_timing` with `--release --ignored --nocapture` | Run to completion; record numbers | | | — |
| P2-08 | Linux: `bash scripts/linux/test.sh` (container: check, clippy, tests under Xvfb, shell smoke, X11, Weston, sway smoke) | Exit 0 | — | | — |
| P2-09 | Linux GNOME: `python3 scripts/linux/wayland_text_input_smoke.py --telex target/debug/litty` | `tiếng việt` reaches the PTY | — | — | |
| P2-10 | Linux GNOME/KDE: `scripts/linux/wayland_data_smoke.py` and `wayland_activation_smoke.py` | Pass | — | — | |
| P2-11 | macOS: `python3 scripts/macos/test-reuse.py` | Pass | | — | — |
| P2-12 | macOS: `bash scripts/macos/test-activation-probe.sh` and `bash scripts/macos/test-shutdown-race.sh` | Pass; negative-control round leaves notifications | | — | — |
| P2-13 | Shell smoke: `cargo test --test yo_shells` with zsh, bash 3.2/5, fish, nushell installed | Pass for every installed shell | | | — |

## 6. P3 — Performance and invariants

Close all other heavy apps. Repeat each measurement 3 times; record every
value, not only the best.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P3-01 | `bash scripts/pre-release-check.sh` | Exit 0; keep `scripts/release-test-report.md` (do not commit it) | | | — |
| P3-02 | `litty +bench-ram` | 1w/1t ≤ 85 MB, 1w/3t ≤ 95 MB, 2w/6t ≤ 145 MB, cleanup delta ≤ 10 MB (macOS `footprint`; Linux RSS) | | | |
| P3-03 | `litty +bench-perf` (latency, 20k stress, features) | Exit 0: DSR mean ≤ 0.05 ms, P99 ≤ 0.4 ms; spawn ≤ 6 ms; APC ≤ 3 ms; Sixel ≤ 5 ms; worst stall ≤ 3 ms; FZF 10k ≤ 2 ms; regex 5k ≤ 0.5 ms | | | — |
| P3-04 | `litty +bench-stress 100000` | Exit 0; record lines/s and worst stall | | | — |
| P3-05 | `litty +bench-dsr 5000` | Exit 0 | | | — |
| P3-06 | `litty +bench-input-latency` with the window visible and focused | Record p50/p95 (reference macOS 6.6–7.1 ms p50 under load) | | | |
| P3-07 | `litty +bench-first-frame 10` | Record p50 (reference macOS 213–219 ms) | | | |
| P3-08 | `litty +bench-pty-throughput` | Exit 0; record MiB/s per stream kind | | | |
| P3-09 | `litty +bench-scroll-fps` | Completes; record frame rate on a 120 Hz display (macOS ProMotion) | | | |
| P3-10 | `litty +bench-all` | Runs latency/stress/features, then opens the app | | | |
| P3-11 | Idle CPU, 1 window at an idle prompt: `ps -o time= -p <pid>` at t=0 and t=60 s | CPU time unchanged except cursor blink (record ticks); `cursor-style-blink = false` → unchanged | | | |
| P3-12 | macOS idle RAM, all windows closed: packaged app, isolated `HOME`, `SHELL=/usr/bin/true`; `footprint <pid>` 60 s after launch | < 50 MB (`CLAUDE.md`); reference 21 MB | | — | — |
| P3-13 | macOS idle CPU with all windows closed, 60 s | 0.0% in `ps`; record CPU time delta | | — | — |
| P3-14 | Linux idle memory, 1 window: RSS and PSS (`smem` or `/proc/<pid>/smaps_rollup`) | Record; reference ~67 MB RSS / ~21 MB PSS | — | | |
| P3-15 | Occlusion: `yes` in a window, then minimize; then cover with another window; then other workspace/Space | CPU drops while hidden (X11 minimized reference: 34–35 → 3 ticks/5 s); output is correct when shown again | | | |
| P3-16 | 6 windows × 4 tabs × 2 splits, all idle | No CPU growth; RAM recorded; no frame drops when switching | | | |
| P3-17 | Close everything back to 1 window/1 tab after P3-16, wait 30 s | Memory returns near the 1w/1t level (Linux: `malloc_trim` path) | | | |
| P3-18 | `cat` of a 200 MB file (`big.txt`), then `TermMod+K` | UI stays responsive (input echoes, window resizes); memory bounded by `scrollback-limit` | | | |
| P3-19 | Hot-path allocation: `cargo test alloc` (counting-allocator tests such as `steady_state_print_and_scroll_allocate_nothing`, `wheel_events_do_not_allocate`) | Pass (zero allocation on PTY parse, scroll, render loop) | | | — |

## 7. P4 — Functional areas

Unless a case says otherwise, start from an empty config and one window.

### A. Launch, lifecycle and CLI

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| A-01 | Launch from Finder/Dock (M), the app menu (X/W), and from a shell | One window, shell prompt, tab bar; record the starting directory (code default: `$HOME` when none is given) | | | |
| A-02 | Shell environment: `env \| grep -E 'TERM\|COLORTERM\|LITTY\|LANG'` | `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=litty`, `TERM_PROGRAM_VERSION=0.14.1`, `LITTY_SHELL_INTEGRATION=1`, `LITTY_AI_TRIGGER=yo`; `LANG=en_US.UTF-8` only if neither `LANG` nor `LC_ALL` was set | | | |
| A-03 | `SHELL=/bin/zsh`, `/bin/bash`, fish, nu, elvish, `/bin/sh`, a non-existent path | Each starts; a bad `SHELL` gives a clear error or fallback, never a hang or crash | | | |
| A-04 | Type `exit` in the only tab | M: window closes, app stays in the Dock at 0% CPU; X/W: process exits | | | |
| A-05 | M: after A-04, click the Dock icon; `open -a Litty`; Shell ▸ New Window | Each opens one fresh window | | — | — |
| A-06 | `TermMod+Q` at an idle prompt, then with `sleep 100` running | Idle: quits; running: "Quit Litty?" dialog (see section D-confirm) | | | |
| A-07 | `litty +list-themes \| wc -l` | 861 | | | |
| A-08 | `litty +toggle-quick-terminal` | stderr `litty: the Quick Terminal was removed in 0.10.0`, exit 1 | | | |
| A-09 | `litty --version`, `litty --help`, `litty -e ls`, `litty foo` | Record what happens (code: any unknown argument launches the terminal) | | | |
| A-10 | `kill -TERM`, `kill -HUP` the Litty process with 2 windows | Shells are hung up; no orphaned shell processes (`ps -ef`), no crash report | | | |
| A-11 | `kill -9` the shell of one pane | That pane shows the exit / closes per design; other panes unaffected | | | |
| A-12 | Shell that exits immediately (`SHELL=/usr/bin/true`) | M: window closes, app keeps running; X/W: process exits; no busy loop | | | |
| A-13 | Launch with `HOME` unset and with a read-only `HOME` | Starts; config/cache failures reported on stderr, no crash | | | |
| A-14 | Launch 20 instances in a loop, close them all | All start and exit cleanly; no leftover registry entries in `$XDG_RUNTIME_DIR/litty-a1` (Linux) | | | |

### B. Windows, tabs and splits

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| B-01 | `cd /tmp`, `TermMod+N` | New window, shell in `/tmp` | | | |
| B-02 | `cd /tmp`, `TermMod+T` | New tab after the current one, in `/tmp` | | | |
| B-03 | Tab inherits a directory with spaces/Vietnamese name (`~/litty-test/thư mục 1`) | New tab opens there | | | |
| B-04 | 10 tabs; `UIMod+1..8` (Linux also `Ctrl+1..9`, `Alt+1..9`), `+9` | Jumps to tab N; 9 = last; badges `^1..^9` match | | | |
| B-05 | Cycle tabs: `TermMod+Shift+[`/`]` (Linux `Ctrl+Shift+[`/`]`), `Ctrl+Tab`/`Ctrl+Shift+Tab`, Linux `Ctrl+PageUp/PageDown` | Previous/next, wraps around | | | |
| B-06 | Click a tab; click its `✕` | Selects; closes that tab only | | | |
| B-07 | Drag a tab left/right in the strip | Reorders; titles and badges follow | | | |
| B-08 | Drag a tab out of the strip and release | Tab becomes a new window that followed the pointer | | | |
| B-09 | Drag a tab over another window's tab bar | Merges there at the drop slot; source window closes if empty | | | |
| B-10 | Split right `Cmd+D` / `Ctrl+Shift+D`; split down `Cmd+Shift+D` / `Ctrl+Shift+E` (also `Super+Shift+D`, `Ctrl+Alt+D`) | Panes with hairline divider; new pane in the same directory | | | |
| B-11 | Navigate panes: `Cmd+Arrows`, `Cmd+Option+Arrows` / `Ctrl+Alt+Arrows`, `Super+Arrows` | Spatial focus; at the left/right edge it moves to the neighbour tab | | | |
| B-12 | Cycle panes: `Cmd+[`/`]` / `Ctrl+Alt+[`/`]`, `Super+[`/`]` | Previous/next pane in the tab | | | |
| B-13 | Resize: `Cmd+Ctrl+Arrows` / `Super+Ctrl+Arrows`, `Ctrl+Alt+Shift+Arrows` | Active pane grows/shrinks 5% per press; shells get SIGWINCH (`tput cols` updates) | | | |
| B-14 | Equalize: `Cmd+Option+=` / `Ctrl+Alt+=` | All ratios equal | | | |
| B-15 | Zoom: `Cmd+Shift+Enter` / `Ctrl+Shift+Enter` twice | Pane fills the tab, then restores | | | |
| B-16 | Drag a divider with the mouse | Resizes; shells get the new size | | | |
| B-17 | Close pane `TermMod+W` in a 3-pane tab, then the last pane | Pane closes, focus moves to a neighbour; last pane closes the tab; last tab closes the window | | | |
| B-18 | Unfocused panes fade (`unfocused-split-opacity`, `-fill`); a single pane never fades | As configured (manual-test group 9) | | | |
| B-19 | Window resize by edges/corners, maximize, full screen (M: `Cmd+Ctrl+F`; X/W: double-click title band, maximize button) | Grid reflows, no garbage, prompt kept | | | |
| B-20 | Minimize `TermMod+M` (Linux also `Super+M`) and restore | Restores with correct content | | | |
| B-21 | Move a window between monitors of different scale | Glyphs re-rasterize sharp at the new scale; cell grid recomputed | | | |
| B-22 | 50 tabs in one window | Tab strip stays usable; no crash | | | |
| B-23 | Each window keeps its own tabs/splits; find, reload, theme switch work in every window | manual-test group 8 | | | |

### C. Keyboard

Use `k` (appendix B) to see the bytes a key sends.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| C-01 | Every default shortcut of `docs/guide/shortcuts.md` on its platform | Each runs its action once | | | |
| C-02 | Linux: bare `Ctrl+C/V/W/N/T/A/F/Q/M/O/K` with `k` | Bytes `03 16 17 0e 14 01 06 11 0d 0f 0b`; no Litty action | — | | |
| C-03 | macOS: bare `Ctrl+<letter>` (same list) with `k` | Same bytes; no Litty action | | — | — |
| C-04 | `Ctrl+R` at a prompt; `Ctrl+R` inside vim | Prompt: history popup; vim: redo (not intercepted) | | | |
| C-05 | Word jump: `Option+Left/Right` (M), `Ctrl+Left/Right` and `Alt+Left/Right` (X/W) | `ESC b` / `ESC f`; Mission Control Spaces not triggered on M | | | |
| C-06 | `Ctrl+U` at a prompt with text after the cursor | Deletes from prompt to cursor only; prompt and text after cursor stay | | | |
| C-07 | `Ctrl+U` in vim | Half-page scroll up (vim gets `^U`) | | | |
| C-08 | `Ctrl+K` at a prompt | Deletes to end of line | | | |
| C-09 | `Shift+PageUp/PageDown` at a prompt; in `less` | Scrolls Litty's scrollback; reaches top/bottom | | | |
| C-10 | Desktop interception: on GNOME/KDE/sway press every `Super+…` and `Ctrl+Alt+Arrow` alternative | Record which ones the desktop takes (e.g. GNOME `Super+V`, `Super+A`, `Super+Arrows`); log each lost default as an issue | — | | |
| C-11 | User keybinds: `keybind = cmd+t=new_split:right` (Linux `ctrl+shift+t=…`), `keybind = cmd+d=unbind`, `keybind = ctrl+shift+h=text:hello\n`, then `keybind = clear` | Rebinds, unbinds (key reaches shell), sends text; `clear` restores defaults (manual-test group 5) | | | |
| C-12 | Every action name in `docs/guide/keybinding-actions.md`, plus `increase_font_size`, `decrease_font_size`, `reset_font_size`, `scroll_page_up`, `scroll_page_down`, `csi:`, `esc:`, `ignore`, `last_tab`, `goto_tab:N`, `resize_split:dir`, `goto_split:next/previous` | Each binds and runs | | | |
| C-13 | Invalid keybinds: unknown action, `toggle_quick_terminal`, bad trigger (`cmd+`), `goto_tab:0`, `goto_tab:10` | One clear error per line on stderr; other lines still apply | | | |
| C-14 | Trigger spellings: `super+comma`, `ctrl++`, `alt+grave_accent`, `opt+left`, `command+bracket_left` | Parsed and matched | | | |
| C-15 | Kitty protocol: `bash scripts/manual-test.sh 27` and `16`/`26`; `kitten show-key -m kitty` | `CSI u` encodings; `Ctrl+I` ≠ `Tab`; release/repeat events with flag 2; Caps/Num Lock in report-all mode | | | |
| C-16 | Kitty flag stacks: `q '\e[>1u\e[?u'`; enter alt screen, `q '\e[?u'` | `^[[?1u` on main; alt screen has its own stack (`^[[?0u`) | | | |
| C-17 | Neovim / Helix / fish 4 / Kakoune: map `<C-i>`, `<C-S-x>`, `<D-s>` (Cmd/Super) | Distinguished; `<D-…>` reaches nvim only when it enables Kitty flags | | | |
| C-18 | modifyOtherKeys: `bash scripts/manual-test.sh 2`; `q '\e[>4;2m\e[?4m'` | `^[[>4;2m`; `Ctrl+Enter` → `^[[27;5;13~`, `Ctrl+1` → `^[[27;5;49~` | | | |
| C-19 | Unbound `Cmd+J` (M) / `Super+J` (X/W) without Kitty flags | Nothing sent to the shell | | | |
| C-20 | Application cursor mode: `printf '\e[?1h'`; arrows with `k` | `ESC O A..D`; `\e[?1l` back to `ESC [ A` | | | |
| C-21 | Function keys F1–F12, Home/End/Insert/Delete/PageUp/PageDown with all modifier combos (`k`) | xterm encodings (`ESC O P`, `ESC [ 15~`, `ESC [ 1;5H` …) | | | |
| C-22 | Numeric keypad (NumLock on/off), keypad Enter | Digits / navigation as expected; keypad Enter sends `\r` | | | |
| C-23 | `Alt+letter` (M: Option produces characters on many layouts) | Record; Linux `Alt+b` → `ESC b` | | | |
| C-24 | Non-US layouts: German (dead keys `´ e` → `é`), French AZERTY (`Ctrl+Shift+…` shortcuts by key position or symbol?), Russian (`Ctrl+С` with Cyrillic) | Text correct; shortcuts still work on non-Latin layouts (Kitty alternate keys per `keymap.rs` tests) | | | |
| C-25 | Linux dead keys/Compose with a physical keyboard: `dead_acute e`, `Compose o c` | `é`, `©` (X11 XIM built-in method; Wayland xkb compose) | — | | |
| C-26 | Key repeat: hold a letter, hold Backspace, hold an arrow in vim | Smooth repeat, stops on release, no stuck keys after focus loss | | | |
| C-27 | `mouse-hide-while-typing = true` | Pointer hides on keypress, returns on mouse move | | | |
| C-28 | Caps Lock / Num Lock state with Kitty report-all mode | Lock bits correct (manual-test 16, 26) | | | |

### D. Input methods (IME)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| D-01 | Vietnamese Telex: type `tieengs vieetj` at a prompt | `tiếng việt`; preedit shown at the cursor, underlined | | | |
| D-02 | VNI and Unikey modes | Correct text | | | |
| D-03 | Japanese/Chinese IME: compose, open candidate list, pick | Candidate window at the cursor cell; commit inserts text | | | |
| D-04 | IME at the right edge, in a split, after a font-size change | Candidate window follows the cursor | | | |
| D-05 | IME inside vim insert mode and in the find box / history popup | Text reaches the right target | | | |
| D-06 | Linux X11: start Litty before `ibus-daemon`; kill and restart `ibus-x11` | Picks up late start and restart in every window | — | | — |
| D-07 | Linux Wayland: text-input-v3 on GNOME, KDE, sway | Works on GNOME/KDE; sway result recorded (open v0.15.0 item) | — | — | |
| D-08 | Shortcut while preedit is active (`TermMod+T`) | No lost or duplicated text | | | |

### E. Mouse, selection and clipboard

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| E-01 | Drag-select across lines, release | Selected; copied when `copy-on-select = true` (default) | | | |
| E-02 | `copy-on-select = false`; `copy-on-select = clipboard` | False: drag does not touch clipboard, `TermMod+C` copies; record the difference between `true` and `clipboard` on Linux (PRIMARY) | | | |
| E-03 | Double-click a word containing `'"│\`\|:;,()[]{}<>$` boundaries; on a token wrapped over lines | Ghostty word boundaries; whole wrapped token selected | | | |
| E-04 | Double-click an already selected single-row token | Widens to the line | | | |
| E-05 | Triple-click | Selects the line (record behaviour on wrapped lines) | | | |
| E-06 | `Shift+Click` and `Shift+Drag` to extend | Nearest end moves; copied | | | |
| E-07 | `TermMod+A` then `TermMod+C` | All scrollback + screen copied | | | |
| E-08 | Copy CJK, emoji ZWJ, combining marks, RTL text | Clipboard has the logical text (manual-test 23, 25) | | | |
| E-09 | Copy from a line with trailing spaces and from soft-wrapped lines | No trailing padding; wrapped lines joined without a newline | | | |
| E-10 | Paste `TermMod+V`, Linux `Shift+Insert`, copy with Linux `Ctrl+Insert` | Pasted; bracketed in a shell with 2004 | | | |
| E-11 | Linux PRIMARY: select in Litty, middle-click in another app; select elsewhere, middle-click in Litty | Both directions work | — | | |
| E-12 | Middle-click in vim with `set mouse=a`; in htop | Record whether it is reported to the app or pastes (xterm convention: reported) | | | |
| E-13 | Selection in a mouse-reporting app (`vim mouse=a`, tmux, htop): plain drag vs `Shift+Drag` | Plain goes to the app; `Shift+Drag` selects locally | | | |
| E-14 | Mouse reporting: `printf '\e[?1000h\e[?1006h'` then click (`k`); `1002` drag; `1003` motion; `1016` pixels; wheel | SGR reports with correct buttons/coords/modifiers | | | |
| E-15 | Alternate scroll (1007) in `less`/`man` | Wheel scrolls the pager | | | |
| E-16 | Click-to-move cursor on the command line: plain, soft-wrapped, multi-line (shell integration), CJK/emoji/`❯` prompt | Cursor lands on the clicked character | | | |
| E-17 | Click outside the current command line, in vim, while scrolled back | No cursor movement keystrokes sent | | | |
| E-18 | Scroll: trackpad (precision) and wheel (discrete); `mouse-scroll-multiplier = 2`, `precision:0.5,discrete:1` | Speed matches config | | | |
| E-19 | Scrollbar: appears while scrolled back, drag thumb top/bottom (manual-test 7, 11) | Thumb follows; bottom is live | | | |
| E-20 | Scroll back, then type a key; press a lone modifier | Real key snaps to the prompt; lone modifier keeps position and selection | | | |
| E-21 | OSC 22 pointer shapes `pointer`, `text`, default (manual-test 3) | Cursor icon changes | | | |
| E-22 | OSC 52: `printf '\e]52;c;%s\a' "$(printf 'hi' \| base64)"`; query `\e]52;c;?\a` | Clipboard = `hi`; query gets no reply (read not supported) | | | |
| E-23 | `clipboard-write = false` / `deny`, then OSC 52 | Clipboard unchanged | | | |
| E-24 | Clipboard with 10 MB text paste; paste while output is streaming | Pasted completely; UI responsive | | | |
| E-25 | Linux Wayland clipboard with GNOME (no `wl-copy` window flash), KDE, sway | Round trips both ways | — | — | |
| E-26 | File drag-and-drop from Finder (M), Nautilus/Dolphin (X), Nautilus/Dolphin/sway (W): names with spaces, quotes, Vietnamese, several files | Shell-quoted paths separated by spaces are pasted | | | |

### F. Scrollback and output

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| F-01 | `seq 20000`, scroll to top | Default 10,000,000-byte limit keeps about 10,400 lines at 80 columns; record the first line | | | |
| F-02 | `scrollback-limit = 96000`, new tab, `seq 1000` | History starts around line 900 (manual-test 9) | | | |
| F-03 | `TermMod+K` (`clear_screen`) | Scrollback cleared, prompt redrawn at the top | | | |
| F-04 | `clear`, `printf '\e[3J'`, `reset`, `printf '\ec'` (RIS) | Screen/scrollback cleared per sequence; RIS resets modes and colors | | | |
| F-05 | Resize narrower/wider with long wrapped lines and with a running `top` | Wrapped lines reflow; alt-screen apps redraw; no lost lines | | | |
| F-06 | Very long line without newline (200k chars) | Wraps; no hang | | | |
| F-07 | Binary garbage `head -c 1M /dev/urandom` | No crash/hang; `reset` recovers | | | |
| F-08 | Synchronized output: `printf '\e[?2026h'`; draw; `\e[?2026l`; also forget the `l` | Frame appears atomically; a missing end is released by timeout | | | |
| F-09 | Alt screen: vim, less, htop, tmux; exit each | Main screen and scrollback restored; cursor restored (1049) | | | |
| F-10 | `yes` for 60 s; `TermMod+K` while running | UI stays responsive; memory bounded | | | |

### G. Rendering, fonts and Unicode

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| G-01 | Default font, `font-family = <name>`, a file path, a fallback chain (3 lines), a missing name | Fonts used in order; missing name logs `font-family … not found; using fallback` | | | |
| G-02 | `LITTY_FONT=<file.ttf>` and `LITTY_FONT_SIZE=20` env vars | Used when no config overrides | | | |
| G-03 | Zoom in/out/reset `UIMod+=`/`-`/`0` (with and without Shift) | ±1 pt per press, clamped 6–72; reset to `font-size` | | | |
| G-04 | `font-size = 5`, `= 80`, `= abc` | Clamped / error line; Litty still starts | | | |
| G-05 | Bold, italic, bold-italic, dim, inverse, hidden, strikethrough (`SGR 1 3 2 7 8 9`) | Rendered correctly | | | |
| G-06 | Underline styles `4:1..4:5`, `21`, colored `58:2::r:g:b`, reset `59` (manual-test 10) | Single/double/curly/dotted/dashed with their own color | | | |
| G-07 | 16/256/truecolor test (`printf '\e[38;2;255;100;0m'`, 256-color chart script) | Exact colors | | | |
| G-08 | Box drawing and block elements (`tree`, `htop`, Powerline/Nerd Font glyphs) | Lines connect between cells at all `adjust-cell-height` values | | | |
| G-09 | Ligatures (JetBrains Mono / Fira Code); `font-feature = -calt`, `liga=0`, `ss01` (manual-test 21) | On by default; features apply | | | |
| G-10 | CJK double width, half-width katakana, fullwidth forms | 2 cells; following text aligned | | | |
| G-11 | Emoji: ZWJ families, flags, skin tones, VS16, keycaps; `\e[?2027l` legacy (manual-test 23) | One glyph in 2 cells with mode 2027; separate with legacy; `grapheme-width-method = legacy` same as `l` | | | |
| G-12 | Combining marks `q̃ x̧̌ कि नमस्ते`, Vietnamese stacked diacritics `ỗ ữ` | Marks on their base | | | |
| G-13 | RTL Hebrew/Arabic (manual-test 25) | Visual order, joined Arabic on the primary screen | | | |
| G-14 | `adjust-cell-width/height` with `10%`, `-1`, `2`, and extremes `5000`, `-5000` | Applied; extremes clamp to 1–2048 px without crash | | | |
| G-15 | `font-thicken = true` | Bolder text | | | |
| G-16 | `minimum-contrast = 21`, `= 3`, `= 0.5`, `= 30` | Black/white; readable; out-of-range rejected or clamped (record) | | | |
| G-17 | Cursor styles `block`, `bar`, `underline`, `block_hollow`; DECSCUSR `\e[0..6 q`; `cursor-style-blink = true/false/empty`; unfocused window | Shape and blink as set; hollow when unfocused | | | |
| G-18 | `cursor-color`, `selection-background/foreground` | Applied (manual-test 5) | | | |
| G-19 | `background-opacity = 0.5` with default cells and colored cells; X11 compositor on/off | Translucent, premultiplied; colored cells keep opacity (v0.14.1 fixes) | | | |
| G-20 | M: `background-blur = true`, `20`, `0`; `window-colorspace = display-p3` | Vibrancy behind translucent window; P3 colors on a P3 display | | — | — |
| G-21 | Window padding `window-padding-x = 24`, `= 10,30`, `window-padding-balance = true` | Margins as set; grid centred when balanced | | | |
| G-22 | `window-width = 100` + `window-height = 30`; only one of them; `window-width = 5` | Initial grid 100×30; one alone → error "must both be set"; below minimum rejected | | | |
| G-23 | Glyph atlas pressure: print 20,000 distinct CJK characters, then zoom several times | No missing glyphs or corruption; memory recorded | | | |
| G-24 | 120 Hz display (M ProMotion) scroll smoothness | No stutter (record with `+bench-scroll-fps`) | | | |

### H. Colors, themes and config

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| H-01 | `UIMod+,` with no config; again with `~/.litty` missing and read-only | Creates the template (no options set) and opens it. Code uses `open -t` (macOS) / `xdg-open` (Linux), while the docs say `$EDITOR`: record which editor opens. Errors go to stderr | | | |
| H-02 | Edit config, `UIMod+Shift+,` | Live reload in every window without losing shells | | | |
| H-03 | Edit config, switch to another app and back | Auto-reload on focus when the file changed | | | |
| H-04 | Unknown key, line without `=`, bad values for every key type | One error per line (`line N: key: …`); valid lines still apply; Litty starts | | | |
| H-05 | `ai-api-key secret` (no `=`) | Error line shows `ai-api-key <redacted>`, never the secret | | | |
| H-06 | Accepted Ghostty keys `link-url`, `adjust-cursor-thickness`, `window-inherit-working-directory`, `alpha-blending` | Accepted silently | | | |
| H-07 | Empty value `font-size =` for each key | Resets that key to its default | | | |
| H-08 | `theme = Dracula`, `theme = catppuccin-mocha` (alias), `theme = light:Horizon Bright,dark:Dracula`, unknown theme | Applied before other keys; own colors win; unknown → error | | | |
| H-09 | Switch OS appearance light↔dark with a `light:…,dark:…` theme | Theme follows live; mode 2031 pane receives `CSI ?997;1n`/`;2n` | | | |
| H-10 | Custom theme files in `~/.litty/themes/` and `~/.config/ghostty/themes/` with a bundled name | Custom file overrides the bundled one | | | |
| H-11 | `palette = 1=#ff0000` … `15`, `palette = 16=…`, `palette = x` | 0–15 applied; others rejected | | | |
| H-12 | `background`/`foreground` as `#rrggbb`, `rrggbb`, X11 names (`midnight blue`) | Accepted where documented | | | |
| H-13 | Default background/foreground with no config | `#121316` / `#e6e6e6` (documented) | | | |
| H-14 | `title = Work`; OSC 2 from a program | Title stays `Work` | | | |
| H-15 | OSC 4/10/11/12 set and query, OSC 104/110/111/112 reset, per-pane colors (manual-test 1, 20) | Replies `rgb:rrrr/gggg/bbbb`; reset to config; other split unaffected | | | |
| H-16 | Config file with CRLF line endings, UTF-8 BOM, tabs around `=`, quoted values | Parsed (record any failure) | | | |
| H-17 | Config is a directory / unreadable (chmod 000) | `cannot read config …` on stderr, defaults used | | | |

### I. Escape-sequence conformance

Use `q` for replies. Reference: `src/terminal/grid/perform.rs`.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| I-01 | `q '\e[c'`, `q '\e[>c'`, `q '\e[=c'` | `^[[?62;4;22c`, `^[[>0;0;0c`, `^[P!\|00000000^[\` | | | |
| I-02 | `q '\e[5n'`, `q '\e[6n'`, `q '\e[?6n'` | `^[[0n`, `^[[row;colR`, `^[[?row;colR` | | | |
| I-03 | `q '\e[?996n'` in dark and light OS mode | `^[[?997;1n` / `^[[?997;2n` | | | |
| I-04 | DECRQM for modes 1 6 7 25 1000 1002 1003 1004 1006 1007 1016 1049 2004 2026 2027 2031 and an unknown mode | `^[[?N;1$y` set / `;2$y` reset; unknown `;0$y` | | | |
| I-05 | XTWINOPS `14t`, `16t`, `18t` | Pixel size, cell size, rows/cols matching `tput` | | | |
| I-06 | XTGETTCAP `TN`, `Co`, `RGB`, `Smulx`, `Setulc`, `Ss`, `Se`, `Ms`, `Sync`, unknown | Values from `control.rs`; unknown → `^[P0+r…` | | | |
| I-07 | DECRQSS `m`, `r`, ` q`, `"q`, unknown | `^[P1$r…^[\`; unknown `^[P0$r^[\` | | | |
| I-08 | Cursor movement CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP, tabs HTS/TBC/CHT/CBT, IL/DL/ICH/DCH/ECH, ED 0/1/2/3, EL 0/1/2, DECSTBM, DECOM, IRM (4), DECAWM (7), IND/NEL/RI, DECSC/DECRC, SCOSC/SCORC, DEC line drawing `ESC ( 0` | `vttest` menus 1–2, 8 pass; record each failing screen | | | |
| I-09 | `vttest` full run (menus 1–11) | Record per-screen result; failures that real apps hit become issues | | | |
| I-10 | Focus events `\e[?1004h`, switch focus | `^[[I` / `^[[O` | | | |
| I-11 | Bracketed paste `\e[?2004h`, paste | `^[[200~…^[[201~` | | | |
| I-12 | OSC 0/2 titles with `;` in them, OSC 7 cwd, OSC 133 marks | Title keeps `;`; new tab uses OSC 7 dir; marks used by prompt jump | | | |
| I-13 | OSC 9;4 progress states 0–4 (manual-test 3, 11) | Title prefix and top-edge bar as documented | | | |
| I-14 | Malformed/oversized sequences: OSC 100 KB, OSC 2 MB (over the 1 MiB + 64 limit), unterminated OSC then normal output, DCS without ST, CSI with 100 params, nested ESC | No hang, no crash; oversized OSC discarded; later output renders | | | |
| I-15 | 8-bit C1 controls (`\x9b`) and UTF-8 C1 | Handled per vte; no injection | | | |
| I-16 | `infocmp xterm-256color` features used by tmux/nvim (`tmux` with `TERM=xterm-256color`, `set -g extended-keys on`) | tmux works inside Litty incl. truecolor, undercurl, focus events | | | |

### J. Inline images

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| J-01 | manual-test groups 22 and 24 | All verdicts pass | | | |
| J-02 | `kitten icat` PNG, GIF (animated), large JPEG; `timg`; `chafa -f kitty`; yazi previews | Displayed, animation runs, ~0% CPU when scrolled away | | | |
| J-03 | Sixel: `img2sixel`, `chafa -f sixel` | Displayed; prompt continues below | | | |
| J-04 | Transfer media `t=d`, `t=f`, `t=t`, `t=s` (shm) | All work; `t=t` file in a temp dir is deleted after reading | | | |
| J-05 | `t=f` with a path outside temp dirs, a symlink to `~/.ssh/id_*`, `..` traversal, a missing file | Allowed paths per design; every failure replies the same `EPERM:access denied` | | | |
| J-06 | Images and resize, scroll, `clear`, alt screen, split panes | Move with text, clipped at pane edge, removed by clear | | | |
| J-07 | Delete commands `a=d` with each selector; unsupported selector | Correct placements removed; unsupported → error, nothing changed | | | |
| J-08 | Unicode placeholders inside tmux | Image shown via placeholders | | | |
| J-09 | 100 images; a 50 MB image; an APC/Sixel string over 64 MiB (`MAX_IMAGE_STRING`) | Memory bounded; quota/eviction; oversized string rejected; no crash | | | |

### K. Shell integration

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| K-01 | zsh, bash 3.2 (M), bash 5, fish, nushell: no dotfile edits | OSC 133/7 emitted; user rc files still load (`ZDOTDIR` restored for zsh) | | | |
| K-02 | elvish with `use litty` in `rc.elv` | Works; without the line nothing breaks | | | |
| K-03 | Prompt jump `Cmd+Shift+↑/↓` / `Ctrl+Shift+↑/↓` | Scrolls to previous/next prompt | | | |
| K-04 | Select last output `Cmd+Shift+O` / `Ctrl+Shift+O` | Selects and copies the last command's output; empty command → previous output (record) | | | |
| K-05 | Marks survive resize (manual-test 12) | Jumps still land on prompts | | | |
| K-06 | `shell-integration-features = cursor,title`, `no-cursor`, `sudo`, `false` | Bar cursor at prompt, block in vim; title dir then command; `sudo` feature behaviour recorded | | | |
| K-07 | `shell-integration = none`, and forcing `zsh` while running bash | Off: no OSC 133; forced mismatch does not break the shell | | | |
| K-08 | `TermMod+T` after `cd` in a nested shell / ssh session | New tab uses the OSC 7 / foreground process directory; remote dir not used locally (record) | | | |
| K-09 | User with existing `ZDOTDIR`, `oh-my-zsh`, `starship`, `powerlevel10k`, `bash-preexec`, `atuin` | Prompts and plugins keep working | | | |
| K-10 | `~/.cache/litty/shell-integration/<version>/` permissions | Owned by the user, not world-writable | | | |

### L. History search (built-in)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| L-01 | `Ctrl+R` (M also `Cmd+R`; Linux also `Super+R`, `Ctrl+Shift+R`) | Popup with zsh/bash/fish history (`$HISTFILE` first, then by `$SHELL`) | | | |
| L-02 | Syntax: multi-word AND, `'exact`, `^prefix`, `suffix$`, `!neg`, fuzzy, smart case | fzf `--scheme=history` results with match highlight | | | |
| L-03 | Keys: Up/Down, `Ctrl+N/P/J/K`, PageUp/Down, Home/End, Backspace, `Ctrl+U/W`, `Ctrl+R`/`Alt+S` sort toggle, `Esc`/`Ctrl+C`/`Ctrl+G` | Each works | | | |
| L-04 | `Enter` runs; `Tab` inserts for editing; double-click an item; mouse click | As documented | | | |
| L-05 | Commands run seconds ago appear (in-session recording) | Yes | | | |
| L-06 | Delete an entry `Shift+Delete` / `Ctrl+D` | Removed from the history file via temp file + atomic rename; file mode and other entries unchanged; zsh extended format and fish YAML preserved | | | |
| L-07 | Delete while the shell appends to the same file; delete several quickly | No lost or corrupted entries | | | |
| L-08 | 100k-entry history, multi-line entries, invalid UTF-8 bytes | Opens instantly; entries shown; no crash | | | |
| L-09 | nushell / elvish history | Record whether their history files are read (code reads only zsh/bash/fish files) | | | |
| L-10 | Inserted command contains ESC or `\x1b[201~` | Stripped from the inserted text | | | |

### M. Find in terminal

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| M-01 | `Cmd+F` / `Ctrl+Shift+F` (`Super+F`) | Box top-right; all matches highlighted in scrollback | | | |
| M-02 | Regex syntax: alternation, lazy `*?`, `(?i)`, `(?s)`, `\w \s \b` with Unicode, `\d`, `^`/`$` per line | Per `features.md` | | | |
| M-03 | Smart case and accent folding: `tiếng` finds `TIẾNG` | Found | | | |
| M-04 | `Ctrl+R` literal toggle; invalid regex `(` | Literal mode; invalid shown without crash | | | |
| M-05 | `Enter`/`↑`/`Cmd+G` older, `Shift+Enter`/`↓`/`Cmd+Shift+G` newer (Linux `Super+G`); `Ctrl+P/N` | Steps through matches, scrolls view | | | |
| M-06 | `Ctrl+U/W`, `Ctrl/Alt/Cmd+Backspace` clear; `Esc`, `Ctrl+C/G` close | Query cleared on close (v0.14.0 fix) | | | |
| M-07 | Paste into the box (`TermMod+V`), `UIMod+1` while open | Paste goes to the box; shortcuts still work; nothing reaches the shell | | | |
| M-08 | Search 10 MB scrollback while output streams | Responsive; matches update | | | |

### N. Links and opening files

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| N-01 | `LinkMod`+hover over `https://example.com` | Underline and preview pill | | | |
| N-02 | `LinkMod+Click` URL, `mailto:`, `ftp://` | Opens in the default handler | | | |
| N-03 | `LinkMod+Click` `~/…`, `./…`, absolute paths: folder, `.txt`, `.rs`, `.pdf`, `.png` | Opens with the default app | | | |
| N-04 | `LinkMod+Click` a `.sh`, `.command`, `.app`, `.terminal`, `.webloc`, `.desktop`, `.AppImage`, a file without extension | Only revealed in its folder, never executed | | | |
| N-05 | OSC 8: `ls --hyperlink=auto`, `rg --hyperlink-format`, `gcc` diagnostics, `delta` | Hover shows the real target; click opens | | | |
| N-06 | OSC 8 with `file://otherhost/…`, `javascript:`, `file:` to a script | Ignored / revealed only | | | |
| N-07 | Path starting with `-` (`./-rf`), names with spaces, quotes, `$()` and `;` | Passed as one argument, nothing executed by a shell | | | |
| N-08 | Click 50 links rapidly | Bounded helper processes, no zombies (`ps`) | | | |

### O. Notifications, bell, title and progress

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| O-01 | OSC 9 and OSC 777 while Litty is in the background (manual-test 3) | Notification with title/body | | | |
| O-02 | M packaged app: first notification permission prompt; Allow | Delivered under Litty identity and icon | | — | — |
| O-03 | M: Don't Allow; relocated/re-signed bundle; raw binary | Helper fallback shows it without click routing | | — | — |
| O-04 | Click a notification from a pane in a split, after moving its tab to another window | Correct window/tab/pane selected | | | |
| O-05 | Click after the pane closed (app running); after full quit (M cold relaunch) | Ignored; M opens one fresh window | | — | — |
| O-06 | 3 processes; notification from the oldest; click | Routed to its origin process | | | |
| O-07 | Linux without a session bus / without a notification server | `notify-send` fallback, no click routing | — | | |
| O-08 | Body/title with 10 KB text, leading `-`, quotes, newlines | Truncated safely (title 128, body 512 chars); no option injection | | | |
| O-09 | Clean quit withdraws notifications (`Cmd+Q`, Dock Quit) | Withdrawn | | | |
| O-10 | Bell: `bell-features` `system`, `attention`, `title`, `audio` (+ path/volume), `border`, `no-*` (manual-test 7, 11) | Each effect; 🔔 on background tab cleared on visit; Dock bounce / urgency hint | | | |
| O-11 | `bell-audio-path` missing file; Linux without `paplay` | No crash; error recorded | | | |
| O-12 | Window title from OSC 2, cwd, running command; non-ASCII title on X11 (`xprop _NET_WM_NAME WM_NAME`) | Correct; `WM_CLASS = litty, litty` | | | |

### P. Terminal inspector and custom shaders

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P-01 | Inspector `Cmd+Option+I` / `Ctrl+Shift+I` (`Super+Alt+I`) (manual-test 17) | Sections, live cell, last key bytes, modes | | | |
| P-02 | `custom-shader = examples/shaders/invert.glsl`, then `scanlines.glsl`, both chained (manual-test 19) | Inverted, upright, mouse aligned; chain order respected | | | |
| P-03 | `custom-shader-animation = true/false/always`; unfocused window | Animates only per policy | | | |
| P-04 | Shader with a syntax error; missing file | Reported once on stderr with path (Linux also line); skipped; Litty starts | | | |
| P-05 | Shader + 2 windows + resize + occlusion | No validation errors (Linux layer on); CPU stops when hidden | | | |

### Q. Process reuse (`+reuse`)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| Q-01 | `cd /tmp && litty +reuse` with Litty running | New window in the newest eligible process, shell in `/tmp` | | | |
| Q-02 | `litty --reuse` with no Litty running | Starts a new process at that directory | | | |
| Q-03 | Running process is a benchmark / `+update` | Not chosen | | | |
| Q-04 | Wayland: launcher activation token forwarded | New window focused (compositor policy permitting) | — | — | |
| Q-05 | Kill the target process between lookup and request; stale registry entries | Error exit, no duplicate window; stale entries ignored/cleaned | | | |
| Q-06 | Registry dir permissions (`$XDG_RUNTIME_DIR/litty-a1`, `/tmp/litty-a1-<uid>`); another user's process | `0700`, owned by the user; other UID never joined | | | |
| Q-07 | Directory with spaces/Vietnamese; deleted directory | Correct dir; deleted → clear error | | | |

### R. Update (real GitHub releases)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| R-01 | `litty +check-update` on v0.14.1 | "up to date" (latest is v0.14.1) | | | — |
| R-02 | Build tag `v0.14.0` (`git worktree add ../litty-0.14.0 v0.14.0`, release build), run `+check-update` | Reports v0.14.1 available | | | — |
| R-03 | v0.14.0 standalone binary in a user-writable path: `+update` | Downloads asset for this CPU, verifies minisign + SHA-256 + `# litty-release: v0.14.1`, replaces the binary; new binary contains 0.14.1 | | | — |
| R-04 | `+update --force` / `-f` on v0.14.1 | Reinstalls the current version | | | — |
| R-05 | M: v0.14.0 `Litty.app`: Litty ▸ Check for Updates… → Update Now | Bundle staged, signed, swapped atomically; app relaunches as 0.14.1 | | — | — |
| R-06 | M: same with `/Applications` read-only or on a filesystem without atomic swap | Error; installed bundle intact | | — | — |
| R-07 | Linux `.deb`/`.rpm` install (`/usr/bin/litty`, root-owned): `+update` as a user | Refused with a clear message; nothing changed | — | | — |
| R-08 | Two `+update` at once | Second waits or fails on the lock in `~/.cache/litty/updates` | | | — |
| R-09 | Network off / DNS failure / slow link (throttle) | Timeouts, clear error, no partial binary | | | — |
| R-10 | `auto-update = check` with `~/.cache/litty/last_update_check` removed; then relaunch within 24 h | One background check notifies; second launch does not check | | | — |
| R-11 | `auto-update = download` | Downloads and verifies; next Check/`+update` installs without re-download | | | — |
| R-12 | `auto-update-channel = tip` | Considers prereleases, picks the highest version | | | — |
| R-13 | `GITHUB_TOKEN` set / rate-limited API | Token used for API; falls back to the `releases/latest` redirect | | | — |
| R-14 | `check_for_updates` keybind and menu item with no window open (M) | Works without a window | | — | — |

### S. macOS-specific

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| S-01 | Menu bar: every item of Litty, Shell, Edit, View, Window (manual-test 15) | Each works; Window menu lists windows | | — | — |
| S-02 | Secure Keyboard Entry via menu and `toggle_secure_input` keybind | ✓ in menu, 🔒 in title, survives app switch, released when the window closes | | — | — |
| S-03 | `macos-native-tabs = true` (manual-test 18) | Native tabs, `Cmd+T` in cwd, switching and close work | | — | — |
| S-04 | Hide/Hide Others/Show All, Spaces, Stage Manager, full-screen Space | Correct redraw, occlusion stops rendering | | — | — |
| S-05 | Dock bounce on bell; Dock menu Quit with a running process | Bounce; quit confirmation | | — | — |
| S-06 | `confirm-close-surface` `true/running`, `false/never`, `always`; red close button, `Cmd+W`, `Cmd+Q` (manual-test 14) | Dialog wording and buttons; Cancel keeps, Close closes | | — | — |
| S-07 | Gatekeeper: download the DMG, `xattr -dr com.apple.quarantine`, launch | Launches; app signature verifies (`codesign -dv`) | | — | — |
| S-08 | Option key in non-US layouts (e.g. `Option+L` → `@` on German) | Character typed, not ESC prefix | | — | — |
| S-09 | Sleep/wake, display disconnect/reconnect, lid close with external display | Windows redraw; no crash | | — | — |
| S-10 | VoiceOver basic | Record behaviour (no claim of support) | | — | — |

### T. Linux-specific

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| T-01 | Client title bar: drag to move, double-click to maximize, minimize/maximize/close buttons, resize edges and corners | All work | — | | |
| T-02 | sway/tiling: title band, rounded corners and edges hidden while tiled, back when floating | As documented | — | — | |
| T-03 | Portal light/dark (`gsettings set org.gnome.desktop.interface color-scheme prefer-light`) | Live switch; mode 2031 report | — | | |
| T-04 | No session bus / no portal | Dark default; no D-Bus autolaunch | — | | |
| T-05 | Monitor info: `current_monitor` refresh rate (RandR) on 60/144 Hz | Correct rate (inspector or frame pacing) | — | | |
| T-06 | `confirm-close-surface` with zenity installed: Cancel and Close | Cancel keeps, Close closes | — | | |
| T-07 | Uninstall zenity: close a pane running `sleep 100`; multi-line paste into `cat` (no bracketed paste) | Record both (code: close proceeds; paste is cancelled) | — | | |
| T-08 | Blocking dialogs: while a zenity dialog is open, another window streams output | Record whether all windows freeze until the dialog closes | — | | |
| T-09 | `LITTY_BACKEND=wayland` without a compositor (`WAYLAND_DISPLAY` unset) | Falls back to X11 | — | | |
| T-10 | Wayland fractional scale 100/125/150/200%, live change, move between outputs | Sharp text, correct buffer size | — | — | |
| T-11 | Wayland: key repeat rate from compositor, `caps:escape` remap, horizontal scroll | Repeat follows settings; remap applies; horizontal scroll recorded (unsupported) | — | — | |
| T-12 | Vulkan: Intel, AMD, NVIDIA proprietary; `MESA_VK_DEVICE_SELECT`; software only (lavapipe) | Picks a hardware device first; lavapipe works slowly; no VUID errors | — | | |
| T-13 | Vulkan failure: no ICD installed | Clear `litty: … failed (VkResult n)` message, no segfault | — | | |
| T-14 | X11 without a compositor (no ARGB visual) and `background-opacity < 1` | Opaque fallback, no crash | — | | — |
| T-15 | Non-EWMH window manager (twm) title | `WM_NAME` readable (`STRING`/`COMPOUND_TEXT`) | — | | — |
| T-16 | GNOME notification removal on focus; KDE/sway notification click | Removed on focus; click raises/selects (record per desktop) | — | | |

### U. Security

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| U-01 | Copy text containing `\x1b[201~rm -rf ~\n` (use a file and `pbcopy`/`xclip`), paste in a bracketed shell | ESC and C1 CSI stripped; nothing runs; LF/CRLF → CR | | | |
| U-02 | Multi-line paste into `cat` (no bracketed paste) | "Confirm Multi-Line Paste" with preview; Cancel sends nothing | | | |
| U-03 | OSC 52 read attempts, title reporting (`\e[21t`), OSC 4/10/11 query flood | No clipboard read; title not reported; replies bounded | | | |
| U-04 | Notification text with shell metacharacters | Passed as separate arguments | | | |
| U-05 | Kitty `t=f`/`t=t` probes of private files (J-05) | Same `EPERM` for all failures (CWE-200) | | | |
| U-06 | History rewrite (L-06) under `strace -f -e rename,openat` / `fs_usage` | Temp file + atomic `rename`; never truncates in place | | | |
| U-07 | `fork()` safety: open 50 tabs concurrently while output streams (`strace -f` on Linux) | No deadlock after fork; child only `execve`s | | | |
| U-08 | Config/theme/shader paths with `~`, `..`, symlinks; cache dirs pre-created by another user in `/tmp` | Uses user-owned cache dir; never a shared `/tmp` path | | | |
| U-09 | Update tampering: covered by unit tests in `src/update/verify.rs` (no URL override exists) | P1-04 pass counts them | | | |
| U-10 | `cargo test security_poc` | Pass | | | — |

### V. Robustness and stress

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| V-01 | Resize storm: drag a corner continuously for 30 s while `yes` runs | No crash, no stuck frame, final size correct; shell gets the final SIGWINCH | | | |
| V-02 | 100 rapid `TermMod+T` / `TermMod+W` | No crash, no leaked shells (`ps`) | | | |
| V-03 | Rapid config reloads (script touching the file 100×) | No crash; final config applied | | | |
| V-04 | `cat /dev/urandom \| head -c 50M` in 4 panes | Responsive; memory bounded | | | |
| V-05 | Run 8 hours idle with 3 windows (overnight) | No memory growth, 0% CPU apart from blink | | | |
| V-06 | Low memory (Linux cgroup `systemd-run --user -p MemoryMax=200M litty`) | Degrades or exits cleanly; no corrupted files | — | | |
| V-07 | GPU reset / switching GPUs (M dual-GPU, Linux PRIME) | Recovers or reports an error | | | |
| V-08 | Disk full while writing history delete / update download | Error, original files intact | | | |

### W. AI assistant (offline only)

Live providers are out of scope.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| W-01 | No `ai-provider` configured, run `yo hello` | Message "Set ai-provider and ai-api-key before using yo."; shell not broken | | | |
| W-02 | `yo show documentation`, `yo reset`, `yo show last response` | Local commands work without network | | | |
| W-03 | `ai-trigger = ask`, `= off`, `= rm -rf`, empty | Custom word works in new tabs; `off` makes `yo` an ordinary command; invalid rejected; empty restores `yo` | | | |
| W-04 | Typing/paste/Enter during "thinking" with an unreachable `ai-base-url` (`http://127.0.0.1:9/v1`) | Queued; `Ctrl+C` cancels; error shown; prompt usable | | | |
| W-05 | Config errors: `ai-provider = unknown`, `ai-thinking = extreme`, `ai-web-search = maybe`, `ai-context-window = 100` | One error each | | | |
| W-06 | Redaction: unit tests (`cargo test redact`) | Pass | | | — |

## 8. P5 — Packaging and installation

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P5-01 | `bash scripts/linux/test-bundle-linux.sh` | Pass | — | | — |
| P5-02 | Debian: `bash scripts/linux/test-package-deb.sh`; `sudo apt install ./litty_<ver>_amd64.deb` | Installs `/usr/bin/litty`, desktop entry, icon; menu launch works; `apt remove` cleans up | — | | |
| P5-03 | Fedora: `bash scripts/linux/test-package-rpm.sh`; `sudo dnf install ./litty-<ver>-1.x86_64.rpm` | Same as P5-02 | — | | |
| P5-04 | `desktop-file-validate resources/linux/litty.desktop`; dock pinning; `StartupWMClass` grouping | Valid; windows group under one icon | — | | |
| P5-05 | macOS: `./bundle_macos.sh` (install), DMG layout, `Info.plist` versions, `codesign -dv --verbose` | Versions 0.14.1; bundle id `com.litty.terminal` | | — | — |
| P5-06 | Release asset verification: download v0.14.1 assets, `sha256sum -c`, `minisign -Vm SHA256SUMS.txt -p resources/minisign.pub` | All hashes and the signature verify; one `# litty-release: v0.14.1` line | | | — |
| P5-07 | Raw binary on a clean machine (glibc 2.34 Debian 12; current macOS) | Runs; Linux needs only documented libs | | | |

## 9. P6 — Documentation consistency

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P6-01 | Every shortcut in `shortcuts.md` matches behaviour from C-01; every default in `keymap.rs` is documented | No differences (list each as an issue) | | | |
| P6-02 | Every key in `configuration.md` matches `src/config.rs` (names, defaults, ranges); undocumented keys/env vars listed | No differences | | | |
| P6-03 | Every action in `keybinding-actions.md` matches `parse_binding` | No differences | | | |
| P6-04 | README Quick Start, `installation.md` package names and versions | Point at current release assets | | | |
| P6-05 | `features.md` claims vs results of this run | Each claim backed by a passing case | | | |
| P6-06 | `scripts/manual-test.sh` on Linux (it uses `pbpaste`, Cmd wording, macOS sound paths) | Runs, or Linux substitutions are documented | — | | |

## 10. Exit criteria and triage

1. Every case has a result; every `F` has an issue; every `B`/`S` has a
   reason.
2. At the top of the issue note, record per phase: run, passed, failed,
   blocked, skipped.
3. Triage after the full run: severity (S1 crash/data loss/security, S2
   feature broken, S3 partial or workaround, S4 cosmetic/docs), platform,
   suspected area. Group duplicates.
4. Fix in order S1 → S4, each on its own `fix/<topic>` branch, starting
   with a failing regression test (`CLAUDE.md`), then rerun the cases of
   that issue plus P1 on both platforms.
5. Update `ROADMAP.md` / `docs/plans/0.15.0.md` with issues that are not
   fixed, and keep this plan's result columns as the evidence record.

## Appendix A — fixtures

```sh
mkdir -p ~/litty-test && cd ~/litty-test
mkdir -p 'thư mục 1' "it's here" 'a b'
printf 'x\n' > 'tệp có dấu.txt'; printf 'x\n' > "quote'd \"name\".txt"; printf 'x\n' > ./-rf
seq 5000000 > big.txt                       # ~38 MB; cat it 5× for P3-18
python3 -c "print('x'*200000, end='')" > longline.txt
head -c 1048576 /dev/urandom > random.bin
printf '#!/bin/sh\necho ran > /tmp/litty-ran\n' > run-me.sh; chmod +x run-me.sh
printf 'safe\n\033[201~echo INJECTED > /tmp/litty-injected\n' > paste-attack.txt
```

## Appendix B — helpers

```sh
# q '<sequence>': print the terminal's reply in cat -v form.
q() {
  local old; old=$(stty -g)
  stty raw -echo min 0 time 5
  printf '%b' "$1" > /dev/tty
  dd bs=4096 count=1 2>/dev/null < /dev/tty | cat -v
  stty "$old"; echo
}

# k: press one key (or chord); print the bytes Litty sent.
k() {
  local old; old=$(stty -g)
  stty raw -echo min 1
  dd bs=64 count=1 2>/dev/null < /dev/tty | od -An -tx1
  stty "$old"
}
```

fish and nushell have no `stty -g` in the same form; run the helpers in
`bash` inside the pane under test.
