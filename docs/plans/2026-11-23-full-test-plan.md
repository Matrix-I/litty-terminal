# Full feature test plan — macOS and Linux (2026-11-23)

Status: automated cases run on macOS (column M, `178357e`) and Linux X11
and Wayland (columns X and W, `fc3d1d4`) on 2026-11-23; Linux manual
cases run on 2026-11-24 in a private headless GNOME session; macOS manual
cases not run (`S manual`). Results:
[macOS](../validation/2026-11-23-macos-run-results.md),
[Linux](../validation/2026-11-23-linux-run-results.md). Written from a
review of the source at `9fd8ab4` (v0.14.1 plus release documentation),
`docs/guide/`, `scripts/` and the open roadmap items.

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
  `LITTY_BACKEND=wayland`. The **W column** (Wayland) is unrelated to
  functional **section W** (AI assistant).
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
| E-M2 | Older macOS (e.g. 14) on Apple Silicon (M1 or later) | optional | Older-OS activation limits. Intel Macs are not supported: macOS releases ship `aarch64` only (`src/update/install.rs`) |
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
| P0-01 | Record the environment data from section 1 at the top of the issue note | Recorded | P | P | P (same machine) |
| P0-02 | Back up `~/.litty`, `~/.cache/litty`, `~/.config/ghostty/themes`, the shell history files; move `~/.litty/config` aside | Litty starts with no config | P (isolated `HOME` per run instead of backups) | P (isolated `HOME` per run instead of backups) | P (isolated `HOME` per run) |
| P0-03 | `cargo build --release`; macOS: `./bundle_macos.sh --no-install`; Linux: `bash bundle_linux.sh` | All succeed; record binary sizes | P (raw 2,338,384 B; app 2,341,328 B) | P (raw 4,795,768 B; `.deb` built; `.rpm` skipped: no `rpmbuild`) | P (same build as X) |
| P0-04 | Create fixtures (appendix A) in `~/litty-test` | Present | S manual (fixtures for P4) | P (harness creates per-case fixtures in each private HOME instead of ~/litty-test) | P (same) |
| P0-05 | Define the helpers `q` and `k` (appendix B) in each shell under test | `q '\e[5n'` prints `^[[0n` | S manual (helpers for P4) | P (harness uses its own key logger and DSR helpers in place of `q`/`k`) | P (same) |
| P0-06 | Set `auto-update = off` until section R | No background check during the run | P | P | P |

## 4. P1 — Build and static gates

Run on M and on a Linux machine (X column). These are the `CLAUDE.md`
verification steps plus release-only checks.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P1-01 | `cargo check` | No errors | P | P | — |
| P1-02 | `cargo check --target x86_64-unknown-linux-gnu` | No errors | P | P | — |
| P1-03 | `cargo clippy --all-targets -- -D warnings` | Zero warnings | P (0 warnings) | P (0 warnings) | — |
| P1-04 | `cargo test` in a normal desktop session (needs `openpty`) | 0 failed; record passed/ignored counts (v0.14.1: macOS 722 unit + 1 + 14, Linux 876 unit + 1 + 14) | P (722 + 1 + 14 passed, 12 ignored) | P (876 + 1 + 14 passed, 17 ignored; fresh `HOME`: ISSUE-012) | — |
| P1-05 | `cargo test --release` | 0 failed | P | P (876 + 1 + 14) | — |
| P1-06 | macOS: `cargo test --test native_activation` | Pass | P | — | — |
| P1-07 | `git diff --check` on a clean tree | No output | P | P | — |
| P1-08 | `cargo test layering` (`src/layering_tests.rs`) | Pass; `TOLERATED` entries still have a reason | P (`TOLERATED` empty) | P (6 tests; `TOLERATED` empty) | — |
| P1-09 | Confirm `src/terminal/grid/cell.rs` keeps `const _: () = assert!(size_of::<Cell>() == 12)`; `cargo test precise_scroll_allocates_nothing_and_preserves_cell_layout` | Assert present (P1-01 compiles it); test passes | P | P | — |
| P1-10 | `ls -l target/release/litty` (and the app binary) | < 5,000,000 bytes | P (2.34 MB) | P (4,795,768 B; release asset 4,819,704 B) | — |
| P1-11 | Linux: `ldd target/release/litty` | Record the list; libX11, libxkbcommon, libwayland and libvulkan are opened with `dlopen` at run time, so they should not be listed | — | P (only `libgcc_s`, `libm`, `libc`, `ld-linux`) | — |
| P1-12 | `cargo build` with the CI flags `RUSTFLAGS=-D warnings` | Builds | P | P | — |

## 5. P2 — Automated suites on real hardware

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P2-01 | `cargo test -- --ignored precise_scroll_gpu_pixels --nocapture` | Pass with pixel checks, no validation errors | P | P | — |
| P2-02 | `cargo test -- --ignored warm_overlay_builders_do_not_allocate` | Pass | P | P | — |
| P2-03 | Linux: `cargo test -- --ignored an_uploaded_image_is_drawn_in_the_same_frame a_given_up_frame_still_runs_its_uploads_and_signals_its_fence` with the Khronos validation layer enabled (`VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation`) | Pass, zero VUID messages | — | B (2/2 pass; Khronos validation layer not installed, VUIDs not checked) | B (offscreen test, same run as X) |
| P2-04 | Linux: `cargo test -- --ignored glyph_upload_cost --nocapture` | Record numbers (reference: 1024 glyphs ≈ 8 ms on Intel UHD 620) | — | P (1 glyph 0.26 ms, 64 1.21 ms, 1024 8.26 ms) | — |
| P2-05 | Linux: `cargo test -- --ignored linux_xdg_open_dispatch_smoke --nocapture` | Pass | — | F ISSUE-013 | — |
| P2-06 | Linux X11: `cargo test -- --ignored set_fullscreen_is_reported_back_by_the_window_manager` | Pass | — | P (GNOME 50 XWayland) | — |
| P2-07 | Remaining ignored probes: `grid_throughput_probe`, `resize_probe`, `benchmark_*`, `perf_legit_images`, `theme_lookup_timing` with `--release --ignored --nocapture` | Run to completion; record numbers | P (9 probes) | P (9 probes) | — |
| P2-08 | Linux: `bash scripts/linux/test.sh` (container: check, clippy, tests under Xvfb, shell smoke, X11, Weston, sway smoke) | Exit 0 | — | F ISSUE-020 (3/3 full runs; smoke steps pass when run by hand) | — |
| P2-09 | Linux GNOME: `python3 scripts/linux/wayland_text_input_smoke.py --telex target/debug/litty` | `tiếng việt` reaches the PTY | — | — | P (Telex `tiếng việt`, text-input-v3) |
| P2-10 | Linux GNOME/KDE: `scripts/linux/wayland_data_smoke.py` and `wayland_activation_smoke.py` | Pass | — | — | P (data 4/4, activation 4/4) |
| P2-11 | macOS: `python3 scripts/macos/test-reuse.py` | Pass | P | — | — |
| P2-12 | macOS: `bash scripts/macos/test-activation-probe.sh` and `bash scripts/macos/test-shutdown-race.sh` | Pass; negative-control round leaves notifications | B (shutdown race P; activation probe only builds, needs a manual launch) | — | — |
| P2-13 | Shell smoke: `cargo test --test yo_shells` with zsh, bash 3.2/5, fish, nushell installed | Pass for every installed shell | P (zsh, bash 3.2, nu; fish/elvish not installed) | P (zsh 5.9, bash 5.3; fish, nu, elvish not installed) | — |

## 6. P3 — Performance and invariants

Close all other heavy apps. Repeat each measurement 3 times; record every
value, not only the best.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P3-01 | `bash scripts/pre-release-check.sh` | Exit 0; keep `scripts/release-test-report.md` (do not commit it) | P (ISSUE-002 under isolated `HOME`) | F ISSUE-014, ISSUE-015 | — |
| P3-02 | `litty +bench-ram` | 1w/1t ≤ 85 MB, 1w/3t ≤ 95 MB, 2w/6t ≤ 145 MB, cleanup delta ≤ 10 MB (macOS `footprint`; Linux RSS) | P (3/3; 30–48 MB) | P (3/3; 74.9–76.3 MB) | P (3/3; 74.2–75.5 MB) |
| P3-03 | `litty +bench-perf` (latency, 20k stress, features) | Exit 0: DSR mean ≤ 0.05 ms, P99 ≤ 0.4 ms; spawn ≤ 6 ms; APC ≤ 3 ms; Sixel ≤ 5 ms; worst stall ≤ 3 ms; FZF 10k ≤ 2 ms; regex 5k ≤ 0.5 ms | F ISSUE-001 | F ISSUE-015 | — |
| P3-04 | `litty +bench-stress 100000` | Exit 0; record lines/s and worst stall | F ISSUE-001 | F ISSUE-015 | — |
| P3-05 | `litty +bench-dsr 5000` | Exit 0 | P (mean 0.021 ms, P99 0.062 ms) | P (3/3; mean 0.020–0.049 ms, P99 0.044–0.309 ms) | — |
| P3-05a | `litty +bench-features` on its own | Exit 0 (critical-features benchmark, same as the features part of P3-03) | P (3/3) | F ISSUE-015 (1 of 3) | — |
| P3-06 | `litty +bench-input-latency` with the window visible and focused | Record p50/p95 (reference macOS 6.6–7.1 ms p50 under load) | B (window reported covered/hidden when launched from automation) | P (p50 9.4 ms, p95 13.7 ms) | P (p50 10.0 ms, p95 29.0 ms) |
| P3-07 | `litty +bench-first-frame 10` | Record p50 (reference macOS 213–219 ms) | P (p50 161–186 ms) | P (p50 113–224 ms) | P (p50 67–186 ms) |
| P3-08 | `litty +bench-pty-throughput` | Exit 0; record MiB/s per stream kind | P (ascii 115.8 MB/s) | P (ascii 46.1 MB/s) | P (ascii 20.3–26.5 MB/s) |
| P3-09 | `litty +bench-scroll-fps` | Completes; record frame rate on a 120 Hz display (macOS ProMotion) | F ISSUE-003 | P (60 fps on a 59.94 Hz panel; no 120 Hz display) | F ISSUE-016 |
| P3-10 | `litty +bench-all` | Runs latency/stress/features, then opens the app | S (parts covered by P3-03..05a; opens the app) | S (parts covered by P3-03..05a; opens the app) | S (as X) |
| P3-11 | Idle CPU, 1 window at an idle prompt: `ps -o time= -p <pid>` at t=0 and t=60 s | CPU time unchanged except cursor blink (record ticks); `cursor-style-blink = false` → unchanged | F ISSUE-010 | P (blink on +5 ticks/60 s; off 0 ticks) | P (blink on +5 ticks/60 s; off 0 ticks) |
| P3-12 | macOS idle RAM, all windows closed: packaged app, isolated `HOME`, `SHELL=/usr/bin/true`; `footprint <pid>` 60 s after launch | < 50 MB (`CLAUDE.md`); reference 21 MB | P (19 MB) | — | — |
| P3-13 | macOS idle CPU with all windows closed, 60 s | 0.0% in `ps`; record CPU time delta | P (0.0%; +0.01 s CPU in 60 s) | — | — |
| P3-14 | Linux idle memory, 1 window: RSS and PSS (`smem` or `/proc/<pid>/smaps_rollup`) | Record; reference ~67 MB RSS / ~21 MB PSS | — | P (RSS 76.9 MB, PSS 28.8 MB) | P (RSS 76.1 MB, PSS 29.0 MB) |
| P3-15 | Occlusion: `yes` in a window, then minimize; then cover with another window; then other workspace/Space | CPU drops while hidden (X11 minimized reference: 34–35 → 3 ticks/5 s); output is correct when shown again | S manual | P (output every 10 ms: 18 → 1 ticks/5 s minimized, 16 → 1 covered by a full-screen window; output correct when shown again. `yes` flat out stays ~90% CPU hidden or not: PTY parsing dominates) | P (output every 10 ms: 3 ticks/5 s shown, 3 minimized, 3 covered, so no measurable render cost to save; output correct when shown again) |
| P3-16 | 6 windows × 4 tabs × 2 splits, all idle | No CPU growth; RAM recorded; no frame drops when switching | S manual | P (6 windows × 4 tabs × 2 splits = 48 shells; RSS 74.2 → 84.6 MB; idle 0–2 ticks/5 s; 40 split/tab switches + echo in 13.6 s) | P (48 shells; RSS 73.5 → 79.9 MB; idle 0–2 ticks/5 s; 40 switches + echo in 11.9 s) |
| P3-17 | Close everything back to 1 window/1 tab after P3-16, wait 30 s | Memory returns near the 1w/1t level (Linux: `malloc_trim` path) | S manual | P (back to 1 window/1 tab, after 30 s RSS 76.5 MB vs 74.2 MB at start) | P (after 30 s RSS 71.6 MB vs 73.5 MB at start) |
| P3-18 | `cat` of a 200 MB file (`big.txt`), then `TermMod+K` | UI stays responsive (input echoes, window resizes); memory bounded by `scrollback-limit` | S manual | P (200 MB `cat` in 10.8 s, peak RSS 84.0 MB, Ctrl+Shift+K → 74.5 MB, prompt fine) | P (200 MB `cat` in 7.4 s, peak RSS 83.2 MB, Ctrl+Shift+K → 73.7 MB, prompt fine) |
| P3-19 | Hot-path allocation: `cargo test alloc` (counting-allocator tests such as `steady_state_print_and_scroll_allocate_nothing`, `wheel_events_do_not_allocate`) | Pass (zero allocation on PTY parse, scroll, render loop) | P | P (16 tests) | — |

## 7. P4 — Functional areas

Unless a case says otherwise, start from an empty config and one window.

### A. Launch, lifecycle and CLI

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| A-01 | Launch from Finder/Dock (M), the app menu (X/W), and from a shell | One window, shell prompt, tab bar; record the starting directory (code default: `$HOME` when none is given) | S manual | P (from a shell with cwd /tmp: one window, prompt, shell starts in $HOME; app-menu launch not run: the installed `litty.desktop` runs /usr/bin/litty, not the build under test) | P (same) |
| A-02 | Shell environment: `env \| grep -E 'TERM\|COLORTERM\|LITTY\|LANG'` | `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=litty`, `TERM_PROGRAM_VERSION=0.14.1`, `LITTY_SHELL_INTEGRATION=1`, `LITTY_AI_TRIGGER=yo`; `LANG=en_US.UTF-8` only if neither `LANG` nor `LC_ALL` was set | S manual | P (all listed vars; `LANG=en_US.UTF-8` inherited) | P (all listed vars; `LANG=en_US.UTF-8` inherited) |
| A-03 | `SHELL=/bin/zsh`, `/bin/bash`, fish, nu, elvish, `/bin/sh`, a non-existent path | Each starts; a bad `SHELL` gives a clear error or fallback, never a hang or crash | S manual | F ISSUE-017 (`/bin/sh`, bash start) | F ISSUE-017 (`/bin/sh`, bash start) |
| A-04 | Type `exit` in the only tab | M: window closes, app stays in the Dock at 0% CPU; X/W: process exits | S manual | P (`exit` in the only tab: process exits 0, no leftover bash) | P (same) |
| A-05 | M: after A-04, click the Dock icon; `open -a Litty`; Shell ▸ New Window | Each opens one fresh window | S manual | — | — |
| A-06 | `TermMod+Q` at an idle prompt, then with `sleep 100` running | Idle: quits; running: "Quit Litty?" dialog (see section D-confirm) | S manual | P (idle: Ctrl+Shift+Q exits 0 with `confirm-close-surface = true`; with `sleep 100`: zenity "Quit Litty?" / "A program is still running…"; Esc keeps it; Quit exits 0, `sleep` ended) | P (same) |
| A-07 | `litty +list-themes \| wc -l` | 861 | P (861 themes) | P (861 themes) | P (CLI, as X) |
| A-08 | `litty +toggle-quick-terminal` | stderr `litty: the Quick Terminal was removed in 0.10.0`, exit 1 | P | P | P (CLI, as X) |
| A-09 | `litty --version`, `litty --help`, `litty -e ls`, `litty foo` | Record what happens (code: any unknown argument launches the terminal) | F ISSUE-004 | F ISSUE-004 | F ISSUE-004 |
| A-10 | `kill -TERM`, `kill -HUP` the Litty process with 2 windows | Shells are hung up; no orphaned shell processes (`ps -ef`), no crash report | S manual | P (2 windows + `sleep 300`: SIGTERM and SIGHUP end Litty (default signal action, exit −15/−1); no bash or sleep left. The reuse registry entry stays behind, see ISSUE-046) | P (same) |
| A-11 | `kill -9` the shell of one pane | That pane shows the exit / closes per design; other panes unaffected | S manual | P (`kill -9` the shell of the right pane: that pane closes, the left pane fills the window and still runs commands) | P (same) |
| A-12 | Shell that exits immediately (`SHELL=/usr/bin/true`) | M: window closes, app keeps running; X/W: process exits; no busy loop | S manual | P (exits in 0.56 s, 0.12 s CPU) | P (exits in 0.36 s, 0.08 s CPU) |
| A-13 | Launch with `HOME` unset and with a read-only `HOME` | Starts; config/cache failures reported on stderr, no crash | S manual | P (read-only: stderr reports cache and shell-integration failures) | P (as X) |
| A-14 | Launch 20 instances in a loop, close them all | All start and exit cleanly; no leftover registry entries in `$XDG_RUNTIME_DIR/litty-a1` (Linux) | S manual | P (20 instances; registry empty after exit) | P (20 instances, 20 registry entries while running; `exit` in each → exit 0, registry empty) |

### B. Windows, tabs and splits

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| B-01 | `cd /tmp`, `TermMod+N` | New window, shell in `/tmp` | S manual | P | P |
| B-02 | `cd /tmp`, `TermMod+T` | New tab after the current one, in `/tmp` | S manual | P | P |
| B-03 | Tab inherits a directory with spaces/Vietnamese name (`~/litty-test/thư mục 1`) | New tab opens there | S manual | P | P |
| B-04 | 10 tabs; `UIMod+1..8` (Linux also `Ctrl+1..9`, `Alt+1..9`), `+9` | Jumps to tab N; 9 = last; badges `^1..^9` match | S manual | P | P |
| B-05 | Cycle tabs: `TermMod+Shift+[`/`]` (Linux `Ctrl+Shift+[`/`]`), `Ctrl+Tab`/`Ctrl+Shift+Tab`, Linux `Ctrl+PageUp/PageDown` | Previous/next, wraps around | S manual | P | P |
| B-06 | Click a tab; click its `✕` | Selects; closes that tab only | S manual | P | P |
| B-07 | Drag a tab left/right in the strip | Reorders; titles and badges follow | S manual | P | P |
| B-08 | Drag a tab out of the strip and release | Tab becomes a new window that followed the pointer | S manual | F ISSUE-024 ISSUE-ISSUE-024 (tear-out fails on some drags: straight-down drags never, others ~2 of 3) | P (1 drag) |
| B-09 | Drag a tab over another window's tab bar | Merges there at the drop slot; source window closes if empty | S manual | F ISSUE-025 ISSUE-ISSUE-025 (drop on other window's strip makes a new window) | B (harness has no window geometry on Wayland) |
| B-10 | Split right `Cmd+D` / `Ctrl+Shift+D`; split down `Cmd+Shift+D` / `Ctrl+Shift+E` (also `Super+Shift+D`, `Ctrl+Alt+D`) | Panes with hairline divider; new pane in the same directory | S manual | P | P |
| B-11 | Navigate panes: `Cmd+Arrows`, `Cmd+Option+Arrows` / `Ctrl+Alt+Arrows`, `Super+Arrows` | Spatial focus; at the left/right edge it moves to the neighbour tab | S manual | P (Super+Arrows; Ctrl+Alt+Arrows taken by GNOME, C-10) | P (as X) |
| B-12 | Cycle panes: `Cmd+[`/`]` / `Ctrl+Alt+[`/`]`, `Super+[`/`]` | Previous/next pane in the tab | S manual | P | P |
| B-13 | Resize: `Cmd+Ctrl+Arrows` / `Super+Ctrl+Arrows`, `Ctrl+Alt+Shift+Arrows` | Active pane grows/shrinks 5% per press; shells get SIGWINCH (`tput cols` updates) | S manual | P (Super+Ctrl+Right +5%; Ctrl+Alt+Shift+Arrows do nothing on GNOME) | P (as X) |
| B-14 | Equalize: `Cmd+Option+=` / `Ctrl+Alt+=` | All ratios equal | S manual | P | P |
| B-15 | Zoom: `Cmd+Shift+Enter` / `Ctrl+Shift+Enter` twice | Pane fills the tab, then restores | S manual | P | P |
| B-16 | Drag a divider with the mouse | Resizes; shells get the new size | S manual | P | P |
| B-17 | Close pane `TermMod+W` in a 3-pane tab, then the last pane | Pane closes, focus moves to a neighbour; last pane closes the tab; last tab closes the window | S manual | P | P |
| B-18 | Unfocused panes fade (`unfocused-split-opacity`, `-fill`); a single pane never fades | As configured (manual-test group 9) | S manual | P (unfocused panes dimmed; single pane not) | P (`unfocused-split-opacity = 0.4`, fill `#ff0000`: unfocused pane tinted red and dimmed, focused pane and single pane not) |
| B-19 | Window resize by edges/corners, maximize, full screen (M: `Cmd+Ctrl+F`; X/W: double-click title band, maximize button) | Grid reflows, no garbage, prompt kept | S manual | P (double-click title band and maximize button) | P (as X) |
| B-20 | Minimize `TermMod+M` (Linux also `Super+M`) and restore | Restores with correct content | S manual | P (restored with Alt+Tab) | P (as X) |
| B-21 | Move a window between monitors of different scale | Glyphs re-rasterize sharp at the new scale; cell grid recomputed | S manual | B (single virtual monitor) | B (single virtual monitor) |
| B-22 | 50 tabs in one window | Tab strip stays usable; no crash | S manual | P | P |
| B-23 | Each window keeps its own tabs/splits; find, reload, theme switch work in every window | manual-test group 8 | S manual | P (new window from a /tmp shell opens in /tmp; tab + split stay in window 2; find in window 2 marks 10/10; config reload and portal light/dark switch recolor both windows, checked on a strip of window 1 left visible by the cascade) | P (same) |

### C. Keyboard

Use `k` (appendix B) to see the bytes a key sends.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| C-01 | Every default shortcut of `docs/guide/shortcuts.md` on its platform | Each runs its action once | S manual | F ISSUE-051, ISSUE-026, ISSUE-030 (Ctrl+Shift+E split down, Ctrl+Shift+Enter zoom/unzoom, Ctrl+Alt+[ cycle split, Super+Up/Down navigate, Super+Ctrl+Up resize, Ctrl+Alt+= and Super+Alt+= equalize, Ctrl/Alt+1 and +9, Ctrl+Shift+[ ], Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+= / Ctrl+- / Ctrl+0, Shift+Insert paste, Ctrl+Shift+O, Ctrl+, (opens via xdg-open, see ISSUE-051), Shift+PageUp/PageDown, Ctrl+Shift+M all act once; the rest of the table is covered in E/F/K/L/M/N/P. Not usable here: Ctrl+Alt+arrows and Ctrl+Alt+Shift+arrows are taken by GNOME (ISSUE-026); Super+[ / ] are eaten by the Unikey IME (Telex maps [ ] to ơ ư), not judged; Ctrl+Insert copy leaves CLIPBOARD unchanged, as Ctrl+Shift+C does, ISSUE-030) | P partial (Ctrl+Shift+E split down, Ctrl+Shift+Enter zoom/unzoom, Ctrl+Alt+[ cycle split, Super+Up/Down navigate, Super+Ctrl+Up resize, Ctrl+Alt+= and Super+Alt+= equalize, Ctrl/Alt+1 and +9, Ctrl+Shift+[ ], Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+= / Ctrl+- / Ctrl+0, Shift+Insert paste, Ctrl+Shift+O, Ctrl+, (opens via xdg-open, see ISSUE-051), Shift+PageUp/PageDown, Ctrl+Shift+M all act once; the rest of the table is covered in E/F/K/L/M/N/P. Not usable here: Ctrl+Alt+arrows and Ctrl+Alt+Shift+arrows are taken by GNOME (ISSUE-026); Super+[ / ] are eaten by the Unikey IME (Telex maps [ ] to ơ ư), not judged; Ctrl+Insert copies) |
| C-02 | Linux: bare `Ctrl+C/V/W/N/T/A/F/Q/M/O/K` with `k` | Bytes `03 16 17 0e 14 01 06 11 0d 0f 0b`; no Litty action | — | P | P |
| C-03 | macOS: bare `Ctrl+<letter>` (same list) with `k` | Same bytes; no Litty action | S manual | — | — |
| C-04 | `Ctrl+R` at a prompt; `Ctrl+R` inside vim | Prompt: history popup; vim: redo (not intercepted) | S manual | B (prompt popup P; vim not installed) | B (as X) |
| C-05 | Word jump: `Option+Left/Right` (M), `Ctrl+Left/Right` and `Alt+Left/Right` (X/W) | `ESC b` / `ESC f`; Mission Control Spaces not triggered on M | S manual | P | P |
| C-06 | `Ctrl+U` at a prompt with text after the cursor | Deletes from prompt to cursor only; prompt and text after cursor stay | S manual | P | P |
| C-07 | `Ctrl+U` in vim | Half-page scroll up (vim gets `^U`) | S manual | P (0x15 reaches the app; vim not installed) | P (as X) |
| C-08 | `Ctrl+K` at a prompt | Deletes to end of line | S manual | P | P |
| C-09 | `Shift+PageUp/PageDown` at a prompt; in `less` | Scrolls Litty's scrollback; reaches top/bottom | S manual | P (prompt; less not tried) | P (as X) |
| C-10 | Desktop interception: on GNOME/KDE/sway press every `Super+…` and `Ctrl+Alt+Arrow` alternative | Record which ones the desktop takes (e.g. GNOME `Super+V`, `Super+A`, `Super+Arrows`); log each lost default as an issue | — | F ISSUE-026 ISSUE-ISSUE-026 | F ISSUE-026 ISSUE-ISSUE-026 |
| C-11 | User keybinds: `keybind = cmd+t=new_split:right` (Linux `ctrl+shift+t=…`), `keybind = cmd+d=unbind`, `keybind = ctrl+shift+h=text:hello\n`, then `keybind = clear` | Rebinds, unbinds (key reaches shell), sends text; `clear` restores defaults (manual-test group 5) | S manual | P | P |
| C-12 | Every action name in `docs/guide/keybinding-actions.md`, plus `increase_font_size`, `decrease_font_size`, `reset_font_size`, `scroll_page_up`, `scroll_page_down`, `csi:`, `esc:`, `ignore`, `last_tab`, `goto_tab:N`, `resize_split:dir`, `goto_split:next/previous` | Each binds and runs | S manual | P (all documented actions parse; 20 actions run-checked) | P (as X) |
| C-13 | Invalid keybinds: unknown action, `toggle_quick_terminal`, bad trigger (`cmd+`), `goto_tab:0`, `goto_tab:10` | One clear error per line on stderr; other lines still apply | S manual | P | P |
| C-14 | Trigger spellings: `super+comma`, `ctrl++`, `alt+grave_accent`, `opt+left`, `command+bracket_left` | Parsed and matched | S manual | F ISSUE-027 ISSUE-ISSUE-027 (ctrl++ never matches; super+comma, opt+left, command+bracket_left OK; Alt+` taken by GNOME) | F ISSUE-027 ISSUE-ISSUE-027 |
| C-15 | Kitty protocol: `bash scripts/manual-test.sh 27` and `16`/`26`; `kitten show-key -m kitty` | `CSI u` encodings; `Ctrl+I` ≠ `Tab`; release/repeat events with flag 2; Caps/Num Lock in report-all mode | S manual | P | P |
| C-16 | Kitty flag stacks: `q '\e[>1u\e[?u'`; enter alt screen, `q '\e[?u'` | `^[[?1u` on main; alt screen has its own stack (`^[[?0u`) | S manual | P | P |
| C-17 | Neovim / Helix / fish 4 / Kakoune: map `<C-i>`, `<C-S-x>`, `<D-s>` (Cmd/Super) | Distinguished; `<D-…>` reaches nvim only when it enables Kitty flags | S manual | B (nvim, helix, fish 4, kakoune not installed) | B (as X) |
| C-18 | modifyOtherKeys: `bash scripts/manual-test.sh 2`; `q '\e[>4;2m\e[?4m'` | `^[[>4;2m`; `Ctrl+Enter` → `^[[27;5;13~`, `Ctrl+1` → `^[[27;5;49~` | S manual | P | P |
| C-19 | Unbound `Cmd+J` (M) / `Super+J` (X/W) without Kitty flags | Nothing sent to the shell | S manual | P | P |
| C-20 | Application cursor mode: `printf '\e[?1h'`; arrows with `k` | `ESC O A..D`; `\e[?1l` back to `ESC [ A` | S manual | P | P |
| C-21 | Function keys F1–F12, Home/End/Insert/Delete/PageUp/PageDown with all modifier combos (`k`) | xterm encodings (`ESC O P`, `ESC [ 15~`, `ESC [ 1;5H` …) | S manual | F ISSUE-028 ISSUE-ISSUE-028 (F1-F12 send nothing on X11; other keys OK) | F ISSUE-029 ISSUE-ISSUE-029 (modified F1-F4 send CSI 11..14;m~ not CSI 1;mP) |
| C-22 | Numeric keypad (NumLock on/off), keypad Enter | Digits / navigation as expected; keypad Enter sends `\r` | S manual | P (KP_5 with NumLock off sends nothing) | P (as X) |
| C-23 | `Alt+letter` (M: Option produces characters on many layouts) | Record; Linux `Alt+b` → `ESC b` | S manual | P | P |
| C-24 | Non-US layouts: German (dead keys `´ e` → `é`), French AZERTY (`Ctrl+Shift+…` shortcuts by key position or symbol?), Russian (`Ctrl+С` with Cyrillic) | Text correct; shortcuts still work on non-Latin layouts (Kitty alternate keys per `keymap.rs` tests) | S manual | B (keyboard layouts not switchable in harness) | B (as X) |
| C-25 | Linux dead keys/Compose with a physical keyboard: `dead_acute e`, `Compose o c` | `é`, `©` (X11 XIM built-in method; Wayland xkb compose) | — | B (as C-24) | B (as C-24) |
| C-26 | Key repeat: hold a letter, hold Backspace, hold an arrow in vim | Smooth repeat, stops on release, no stuck keys after focus loss | S manual | P | P |
| C-27 | `mouse-hide-while-typing = true` | Pointer hides on keypress, returns on mouse move | S manual | P | P |
| C-28 | Caps Lock / Num Lock state with Kitty report-all mode | Lock bits correct (manual-test 16, 26) | S manual | P (Caps Lock bit in report-all) | P (as X) |

### D. Input methods (IME)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| D-01 | Vietnamese Telex: type `tieengs vieetj` at a prompt | `tiếng việt`; preedit shown at the cursor, underlined | S manual | P (ibus Unikey, Telex: `tieengs vieetj ` → `tiếng việt ` at the prompt and in the shell) | P (same, via text-input-v3) |
| D-02 | VNI and Unikey modes | Correct text | S manual | P (Unikey VNI: `tie61ng vie65t ` → `tiếng việt `; Unikey only, no separate VNI engine installed) | P (same) |
| D-03 | Japanese/Chinese IME: compose, open candidate list, pick | Candidate window at the cursor cell; commit inserts text | S manual | B (no Japanese/Chinese IME engine installed) | B (same) |
| D-04 | IME at the right edge, in a split, after a font-size change | Candidate window follows the cursor | S manual | P (preedit underlined at the cursor near the right edge, in a right split and after 4× zoom-in; commits `việt` each time) | P (same) |
| D-05 | IME inside vim insert mode and in the find box / history popup | Text reaches the right target | S manual | F ISSUE-049 (vi insert mode: `tiếng việt` written. Find box and history popup: IME preedit and commits go to the shell prompt behind them, the box query stays empty; see ISSUE-049) | F ISSUE-049 (same) |
| D-06 | Linux X11: start Litty before `ibus-daemon`; kill and restart `ibus-x11` | Picks up late start and restart in every window | — | F ISSUE-050 (Litty started with no ibus: plain text; ibus started later: `việt` composes. After killing ibus, typed text never reached Litty within 5 s; not re-run, see ISSUE-050) | — |
| D-07 | Linux Wayland: text-input-v3 on GNOME, KDE, sway | Works on GNOME/KDE; sway result recorded (open v0.15.0 item) | — | — | P partial (GNOME text-input-v3 works, D-01..D-05; KDE and sway not available) |
| D-08 | Shortcut while preedit is active (`TermMod+T`) | No lost or duplicated text | S manual | P (Ctrl+Shift+T with preedit `viêt` active: the new tab opens clean; the preedit is committed into the first tab line) | P (same) |

### E. Mouse, selection and clipboard

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| E-01 | Drag-select across lines, release | Selected; copied when `copy-on-select = true` (default) | S manual | P (selection on PRIMARY and CLIPBOARD) | P (as X) |
| E-02 | `copy-on-select = false`; `copy-on-select = clipboard` | False: drag does not touch clipboard, `TermMod+C` copies; record the difference between `true` and `clipboard` on Linux (PRIMARY) | S manual | F ISSUE-030 ISSUE-ISSUE-030 (false: drag sets PRIMARY only, Ctrl+Shift+C does not copy; clipboard and true both set CLIPBOARD+PRIMARY) | P (false: drag sets PRIMARY only, Ctrl+Shift+C copies; clipboard and true behave the same: CLIPBOARD+PRIMARY) |
| E-03 | Double-click a word containing `'"│\`\|:;,()[]{}<>$` boundaries; on a token wrapped over lines | Ghostty word boundaries; whole wrapped token selected | S manual | P (12 boundary characters; 156-char wrapped token whole) | P (as X) |
| E-04 | Double-click an already selected single-row token | Widens to the line | S manual | P | P |
| E-05 | Triple-click | Selects the line (record behaviour on wrapped lines) | S manual | P (wrapped line: triple-click selects the clicked row only) | P (as X) |
| E-06 | `Shift+Click` and `Shift+Drag` to extend | Nearest end moves; copied | S manual | F ISSUE-031 ISSUE-ISSUE-031 (Shift+Click does not extend) | P |
| E-07 | `TermMod+A` then `TermMod+C` | All scrollback + screen copied | S manual | F ISSUE-030 ISSUE-ISSUE-030 (Select All + Copy leaves the clipboard unchanged) | P |
| E-08 | Copy CJK, emoji ZWJ, combining marks, RTL text | Clipboard has the logical text (manual-test 23, 25) | S manual | P (CJK, ZWJ emoji, accents, Hebrew round trip) | P (as X) |
| E-09 | Copy from a line with trailing spaces and from soft-wrapped lines | No trailing padding; wrapped lines joined without a newline | S manual | P | P |
| E-10 | Paste `TermMod+V`, Linux `Shift+Insert`, copy with Linux `Ctrl+Insert` | Pasted; bracketed in a shell with 2004 | S manual | F ISSUE-030 ISSUE-ISSUE-030 (Ctrl+Shift+V and Shift+Insert paste bracketed; Ctrl+Insert does not copy) | P (Ctrl+Shift+V, Shift+Insert bracketed; Ctrl+Insert copies) |
| E-11 | Linux PRIMARY: select in Litty, middle-click in another app; select elsewhere, middle-click in Litty | Both directions work | — | P | P |
| E-12 | Middle-click in vim with `set mouse=a`; in htop | Record whether it is reported to the app or pastes (xterm convention: reported) | S manual | P (middle click reported to the app, SGR button 1; vim not installed, key logger used) | P (as X) |
| E-13 | Selection in a mouse-reporting app (`vim mouse=a`, tmux, htop): plain drag vs `Shift+Drag` | Plain goes to the app; `Shift+Drag` selects locally | S manual | P (plain drag reported; Shift+Drag selects locally; key logger instead of vim/tmux) | P (as X) |
| E-14 | Mouse reporting: `printf '\e[?1000h\e[?1006h'` then click (`k`); `1002` drag; `1003` motion; `1016` pixels; wheel | SGR reports with correct buttons/coords/modifiers | S manual | P (1000/1006 clicks, 1002 drag 32, 1003 motion 35, 1016, wheel 64/65) | P (as X) |
| E-15 | Alternate scroll (1007) in `less`/`man` | Wheel scrolls the pager | S manual | P (1007: wheel sends 3x ESC [ A in alt screen) | P (as X) |
| E-16 | Click-to-move cursor on the command line: plain, soft-wrapped, multi-line (shell integration), CJK/emoji/`❯` prompt | Cursor lands on the clicked character | S manual | P (plain single line; wrapped/multi-line/CJK prompt not tried) | P (as X) |
| E-17 | Click outside the current command line, in vim, while scrolled back | No cursor movement keystrokes sent | S manual | P (click on output row: no cursor keys) | P (as X) |
| E-18 | Scroll: trackpad (precision) and wheel (discrete); `mouse-scroll-multiplier = 2`, `precision:0.5,discrete:1` | Speed matches config | S manual | P (wheel: 3 lines/notch, 6 with multiplier 2; trackpad not available) | P (as X) |
| E-19 | Scrollbar: appears while scrolled back, drag thumb top/bottom (manual-test 7, 11) | Thumb follows; bottom is live | S manual | B (scrollbar appears while scrolled back; thumb drag not tried) | B (as X) |
| E-20 | Scroll back, then type a key; press a lone modifier | Real key snaps to the prompt; lone modifier keeps position and selection | S manual | F ISSUE-031 ISSUE-ISSUE-031 (real key snaps OK; lone Shift also snaps to bottom) | P |
| E-21 | OSC 22 pointer shapes `pointer`, `text`, default (manual-test 3) | Cursor icon changes | S manual | P (pointer hand / I-beam / default) | P (cursor-shape 4, 9, 1) |
| E-22 | OSC 52: `printf '\e]52;c;%s\a' "$(printf 'hi' \| base64)"`; query `\e]52;c;?\a` | Clipboard = `hi`; query gets no reply (read not supported) | S manual | P | P |
| E-23 | `clipboard-write = false` / `deny`, then OSC 52 | Clipboard unchanged | S manual | P (false and deny keep clipboard) | P (as X) |
| E-24 | Clipboard with 10 MB text paste; paste while output is streaming | Pasted completely; UI responsive | S manual | P (10,010,000 chars pasted complete; paste during streaming not tried) | P (as X) |
| E-25 | Linux Wayland clipboard with GNOME (no `wl-copy` window flash), KDE, sway | Round trips both ways | — | — | P (GNOME round trips via native clipboard; KDE, sway not available) |
| E-26 | File drag-and-drop from Finder (M), Nautilus/Dolphin (X), Nautilus/Dolphin/sway (W): names with spaces, quotes, Vietnamese, several files | Shell-quoted paths separated by spaces are pasted | S manual | B (no file manager in the private session) | B (repo drop smoke cannot place the pointer in the headless session) |

### F. Scrollback and output

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| F-01 | `seq 20000`, scroll to top | Default 10,000,000-byte limit keeps about 10,400 lines at 80 columns; record the first line | S manual | P (first line 12968 at 119 cols = 10 MB / 12 B per cell; plan's ~10,400 assumes 80 cols) | P (same: 12968, 7033 lines) |
| F-02 | `scrollback-limit = 96000`, new tab, `seq 1000` | History starts around line 900 (manual-test 9) | S manual | P (first line 903) | P (first line 903) |
| F-03 | `TermMod+K` (`clear_screen`) | Scrollback cleared, prompt redrawn at the top | S manual | P (scrollback gone, prompt at top) | P |
| F-04 | `clear`, `printf '\e[3J'`, `reset`, `printf '\ec'` (RIS) | Screen/scrollback cleared per sequence; RIS resets modes and colors | S manual | P (RIS resets DECCKM; \e[3J clears scrollback) | P |
| F-05 | Resize narrower/wider with long wrapped lines and with a running `top` | Wrapped lines reflow; alt-screen apps redraw; no lost lines | S manual | P (maximize 119->159->119 cols, wrapped line reflows; top redraws) | P (same) |
| F-06 | Very long line without newline (200k chars) | Wraps; no hang | S manual | P (200k-char line, 4.7 s, prompt back) | P (4.6 s) |
| F-07 | Binary garbage `head -c 1M /dev/urandom` | No crash/hang; `reset` recovers | S manual | P (1 MiB urandom + reset, 9.9 s, shell responsive) | P (5.4 s) |
| F-08 | Synchronized output: `printf '\e[?2026h'`; draw; `\e[?2026l`; also forget the `l` | Frame appears atomically; a missing end is released by timeout | S manual | P (unterminated 2026 released, text shown) | P |
| F-09 | Alt screen: vim, less, htop, tmux; exit each | Main screen and scrollback restored; cursor restored (1049) | S manual | P (less, htop restore main screen; vim/tmux not installed) | P (same) |
| F-10 | `yes` for 60 s; `TermMod+K` while running | UI stays responsive; memory bounded | S manual | P (60 s yes + clear: RSS 78->87 MB, Ctrl+C ok) | P (73->83 MB) |

### G. Rendering, fonts and Unicode

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| G-01 | Default font, `font-family = <name>`, a file path, a fallback chain (3 lines), a missing name | Fonts used in order; missing name logs `font-family … not found; using fallback` | S manual | P (Liberation Mono used; missing name -> `font-family NoSuchFontXYZ not found; using fallback`) | P (same) |
| G-02 | `LITTY_FONT=<file.ttf>` and `LITTY_FONT_SIZE=20` env vars | Used when no config overrides | S manual | P (`LITTY_FONT=<file>` + `LITTY_FONT_SIZE=20` → 79×23 grid; config overrides it) | P (same) |
| G-03 | Zoom in/out/reset `UIMod+=`/`-`/`0` (with and without Shift) | ±1 pt per press, clamped 6–72; reset to `font-size` | S manual | P (Ctrl+= / Ctrl+Shift+= / Ctrl+- / Ctrl+0: 119->105->95->105->119 cols; clamps at 238 and 22 cols) | P (same) |
| G-04 | `font-size = 5`, `= 80`, `= abc` | Clamped / error line; Litty still starts | S manual | P (5 → 6 pt, 80 → 72 pt by cell size; `abc` → one error line) | P (5 → 6 pt, 80 → 72 pt by cell size; `abc` → one error line) |
| G-05 | Bold, italic, bold-italic, dim, inverse, hidden, strikethrough (`SGR 1 3 2 7 8 9`) | Rendered correctly | S manual | P (bold, italic, bold-italic, dim, inverse, hidden, strike) | P |
| G-06 | Underline styles `4:1..4:5`, `21`, colored `58:2::r:g:b`, reset `59` (manual-test 10) | Single/double/curly/dotted/dashed with their own color | S manual | P (single/double/curly/dotted/dashed, 21, red 58 color, 59 reset) | P |
| G-07 | 16/256/truecolor test (`printf '\e[38;2;255;100;0m'`, 256-color chart script) | Exact colors | S manual | P (16, 256, truecolor) | P |
| G-08 | Box drawing and block elements (`tree`, `htop`, Powerline/Nerd Font glyphs) | Lines connect between cells at all `adjust-cell-height` values | S manual | P (box drawing connects at default, +30%, -10% cell height; tree not installed) | P |
| G-09 | Ligatures (JetBrains Mono / Fira Code); `font-feature = -calt`, `liga=0`, `ss01` (manual-test 21) | On by default; features apply | S manual | B (JetBrains Mono / Fira Code not installed) | B |
| G-10 | CJK double width, half-width katakana, fullwidth forms | 2 cells; following text aligned | S manual | P (CJK, half-width katakana, fullwidth 2 cells, text aligned) | P |
| G-11 | Emoji: ZWJ families, flags, skin tones, VS16, keycaps; `\e[?2027l` legacy (manual-test 23) | One glyph in 2 cells with mode 2027; separate with legacy; `grapheme-width-method = legacy` same as `l` | S manual | P (ZWJ family, flag, skin tone, VS16, keycap as single glyphs) | P |
| G-12 | Combining marks `q̃ x̧̌ कि नमस्ते`, Vietnamese stacked diacritics `ỗ ữ` | Marks on their base | S manual | F ISSUE-033 (Latin/Vietnamese stacked marks OK; Devanagari unshaped: ि after its base, ् and े missing; cedilla on x̧̌ missing) | F ISSUE-033 (same) |
| G-13 | RTL Hebrew/Arabic (manual-test 25) | Visual order, joined Arabic on the primary screen | S manual | P (Hebrew/Arabic visual order, Arabic joined) | P |
| G-14 | `adjust-cell-width/height` with `10%`, `-1`, `2`, and extremes `5000`, `-5000` | Applied; extremes clamp to 1–2048 px without crash | S manual | P (10% -> 105 cols, -1 -> 136, height 2 -> 29 rows; 5000 -> 1 col, -5000 -> 590 rows, no crash) | P (same) |
| G-15 | `font-thicken = true` | Bolder text | S manual | P (thicker strokes) | P |
| G-16 | `minimum-contrast = 21`, `= 3`, `= 0.5`, `= 30` | Black/white; readable; out-of-range rejected or clamped (record) | S manual | P (21 and 3 lift dark-on-dark; 0.5 clamps to 1 = unchanged; 30 clamps to 21; no error line) | P (same) |
| G-17 | Cursor styles `block`, `bar`, `underline`, `block_hollow`; DECSCUSR `\e[0..6 q`; `cursor-style-blink = true/false/empty`; unfocused window | Shape and blink as set; hollow when unfocused | S manual | P (block/bar/underline/hollow with shell-integration no-cursor; DECSCUSR 0-6 shapes correct; note: default shell integration forces bar at prompt) | P (same) |
| G-18 | `cursor-color`, `selection-background/foreground` | Applied (manual-test 5) | S manual | P (green cursor, magenta selection with yellow text) | P |
| G-19 | `background-opacity = 0.5` with default cells and colored cells; X11 compositor on/off | Translucent, premultiplied; colored cells keep opacity (v0.14.1 fixes) | S manual | P (window translucent, red cell keeps opacity) | P |
| G-20 | M: `background-blur = true`, `20`, `0`; `window-colorspace = display-p3` | Vibrancy behind translucent window; P3 colors on a P3 display | S manual | — | — |
| G-21 | Window padding `window-padding-x = 24`, `= 10,30`; `window-padding-y = 2,6`; `window-padding-balance = true` | Margins as set (x left/right, y top/bottom); grid centred when balanced | S manual | P (padding 24 -> 114 cols; 10,30 -> 115; balance centres grid) | P (same) |
| G-22 | `window-width = 100` + `window-height = 30`; only one of them; `window-width = 5` | Initial grid 100×30; one alone → error "must both be set"; below minimum rejected | S manual | F ISSUE-032 (100x30 config gives 100x28: own title bar inside the computed size); one key alone -> error line; width 5 -> 11 cols (min 10 + clamp) | F ISSUE-032 (same) |
| G-23 | Glyph atlas pressure: print 20,000 distinct CJK characters, then zoom several times | No missing glyphs or corruption; memory recorded | S manual | P (20,000 CJK chars + 6 zoom steps: RSS 77->79 MB, glyphs intact) | P (78->79 MB) |
| G-24 | 120 Hz display (M ProMotion) scroll smoothness | No stutter (record with `+bench-scroll-fps`) | S manual | B (no 120 Hz display; macOS-only ProMotion) | B |

### H. Colors, themes and config

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| H-01 | `UIMod+,` with no config; again with `~/.litty` missing and read-only | Creates the template (no options set) and opens it. Code uses `open -t` (macOS) / `xdg-open` (Linux), while the docs say `$EDITOR`: record which editor opens. Errors go to stderr | S manual | P (Ctrl+, with no config writes the commented template (0 active lines) and opens it via xdg-open, not $EDITOR; missing ~/.litty recreated; read-only dir -> `litty: cannot create …`) | P (same) |
| H-02 | Edit config, `UIMod+Shift+,` | Live reload in every window without losing shells | S manual | P (Ctrl+Shift+, reload: background #202020 -> #203060, shell kept) | P (same) |
| H-03 | Edit config, switch to another app and back | Auto-reload on focus when the file changed | S manual | P (config edited while another window had focus; on refocus background #602030) | P (same) |
| H-04 | Unknown key, line without `=`, bad values for every key type | One error per line (`line N: key: …`); valid lines still apply; Litty starts | S manual | P (6 error lines; valid line applied) | P (6 error lines; valid line applied) |
| H-05 | `ai-api-key secret` (no `=`) | Error line shows `ai-api-key <redacted>`, never the secret | S manual | P (`ai-api-key <redacted>`) | P (`ai-api-key <redacted>`) |
| H-06 | Accepted Ghostty keys `link-url`, `adjust-cursor-thickness`, `window-inherit-working-directory`, `alpha-blending` | Accepted silently | S manual | P (no output) | P (no output) |
| H-06a | Removed key `quick-terminal-autohide = false` | One error line `the Quick Terminal was removed in 0.10.0; this line is ignored`; other lines still apply | S manual | P | P |
| H-07 | Empty value `font-size =` for each key | Resets that key to its default | S manual | P (background and font size back to default) | P (background and font size back to default) |
| H-08 | (manual-test 13) `theme = Dracula`, `theme = catppuccin-mocha` (alias), `theme = light:Horizon Bright,dark:Dracula`, unknown theme | Applied before other keys; own colors win; unknown → error | S manual | P (Dracula, alias, light/dark in dark mode, unknown → error, own color wins; precedence of later lines not checked visually) | P (Dracula, alias, light/dark in dark mode, unknown → error, own color wins; precedence of later lines not checked visually) |
| H-09 | Switch OS appearance light↔dark with a `light:…,dark:…` theme | Theme follows live; mode 2031 pane receives `CSI ?997;1n`/`;2n` | S manual | B (switching light/dark needs the shared org.gnome.desktop.interface color-scheme dconf key; not changed) | B |
| H-10 | Custom theme files in `~/.litty/themes/` and `~/.config/ghostty/themes/` with a bundled name | Custom file overrides the bundled one | S manual | P (~/.litty/themes/Dracula and ~/.config/ghostty/themes/Dracula override bundled Dracula) | P (same) |
| H-11 | `palette = 1=#ff0000` … `15`, `palette = 16=…`, `palette = x` | 0–15 applied; others rejected | S manual | P (1 and 15 applied; 16 and `x` rejected) | P (1 and 15 applied; 16 and `x` rejected) |
| H-12 | `background`/`foreground` as `#rrggbb`, `rrggbb`, X11 names (`midnight blue`) | Accepted where documented | S manual | P (all three forms) | P (all three forms) |
| H-13 | Default background/foreground with no config | `#121316` / `#e6e6e6` (documented) | S manual | F ISSUE-022 | F ISSUE-022 |
| H-14 | `title = Work`; OSC 2 from a program | Title stays `Work` | S manual | P (`_NET_WM_NAME` stays `Work`) | P (Wayland set_title stays `Work` after OSC 2) |
| H-15 | OSC 4/10/11/12 set and query, OSC 104/110/111/112 reset, per-pane colors (manual-test 1, 20) | Replies `rgb:rrrr/gggg/bbbb`; reset to config; other split unaffected | S manual | P (OSC 4/10/11/12 set, query, reset; per-pane part manual) | P (OSC 4/10/11/12 set, query, reset; per-pane part manual) |
| H-16 | Config file with CRLF line endings, UTF-8 BOM, tabs around `=`, quoted values | Parsed (record any failure) | S manual | F ISSUE-023 (BOM; CRLF, tabs and quotes OK) | F ISSUE-023 (BOM; CRLF, tabs and quotes OK) |
| H-17 | Config is a directory / unreadable (chmod 000) | `cannot read config …` on stderr, defaults used | S manual | P (`cannot read config …: Is a directory` / `Permission denied`) | P (`cannot read config …: Is a directory` / `Permission denied`) |

### I. Escape-sequence conformance

Use `q` for replies. Reference: `src/terminal/grid/perform.rs`.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| I-01 | `q '\e[c'`, `q '\e[>c'`, `q '\e[=c'` | `^[[?62;4;22c`, `^[[>0;0;0c`, `^[P!\|00000000^[\` | S manual | P | P |
| I-02 | `q '\e[5n'`, `q '\e[6n'`, `q '\e[?6n'` | `^[[0n`, `^[[row;colR`, `^[[?row;colR` | S manual | P | P |
| I-03 | `q '\e[?996n'` in dark and light OS mode | `^[[?997;1n` / `^[[?997;2n` | S manual | P (private portal switched to prefer-light: `CSI ?996n` → `CSI ?997;2n`; prefer-dark → `;1n`; see T-03) | P (same) |
| I-04 | DECRQM for modes 1 6 7 25 1000 1002 1003 1004 1006 1007 1016 1049 2004 2026 2027 2031 and an unknown mode | `^[[?N;1$y` set / `;2$y` reset; unknown `;0$y` | S manual | P | P |
| I-05 | XTWINOPS `14t`, `16t`, `18t` (manual-test 4) | Pixel size, cell size, rows/cols matching `tput` | S manual | P (matches `stty`) | P (matches `stty`) |
| I-06 | XTGETTCAP `TN`, `Co`, `RGB`, `Smulx`, `Setulc`, `Ss`, `Se`, `Ms`, `Sync`, unknown | Values from `control.rs`; unknown → `^[P0+r…` | S manual | P (matches `control.rs`) | P |
| I-07 | DECRQSS `m`, `r`, ` q`, `"q`, unknown | `^[P1$r…^[\`; unknown `^[P0$r^[\` | S manual | P | P |
| I-08 | Cursor movement CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP, tabs HTS/TBC/CHT/CBT, IL/DL/ICH/DCH/ECH, ED 0/1/2/3, EL 0/1/2, DECSTBM, DECOM, IRM (4), DECAWM (7), IND/NEL/RI, DECSC/DECRC, SCOSC/SCORC, DEC line drawing `ESC ( 0` | `vttest` menus 1–2, 8 pass; record each failing screen | S manual | P (29 rows of CUP/CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/HVP, HTS/TBC/HT, IL/DL/ICH/DCH/ECH, ED 0, EL 0/1/2, DECSTBM+IND, IRM, DECAWM, DECSC/DECRC, SCOSC/SCORC, DEC line drawing; screen pixel-identical to Wayland) | P (all 29 rows match expected text) |
| I-09 | `vttest` full run (menus 1–11) | Record per-screen result; failures that real apps hit become issues | S manual | B (vttest not installed) | B |
| I-10 | Focus events `\e[?1004h`, switch focus | `^[[I` / `^[[O` | S manual | P (CSI O on focus loss, CSI I on focus gain) | P |
| I-11 | Bracketed paste `\e[?2004h`, paste | `^[[200~…^[[201~` | S manual | P (bracketed paste markers seen by reader in E-10/E-24) | P |
| I-12 | OSC 0/2 titles with `;` in them, OSC 7 cwd, OSC 133 marks | Title keeps `;`; new tab uses OSC 7 dir; marks used by prompt jump | S manual | P (OSC 2 `a;b c`, OSC 0 `zero;x` kept whole; OSC 7 /usr/share -> new tab opens there; shell-integration = none so the prompt does not overwrite them) | P (same) |
| I-13 | OSC 9;4 progress states 0–4 (manual-test 3, 11) | Title prefix and top-edge bar as documented | S manual | P (states 1/2/3/4 draw blue/red/full/yellow bar + title prefix, 0 clears) | P (same) |
| I-14 | Malformed/oversized sequences: OSC 100 KB, OSC 2 MB (over the 1 MiB + 64 limit), unterminated OSC then normal output, DCS without ST, CSI with 100 params, nested ESC | No hang, no crash; oversized OSC discarded; later output renders | S manual | P | F ISSUE-021 |
| I-15 | 8-bit C1 controls (`\x9b`) and UTF-8 C1 | Handled per vte; no injection | S manual | P (8-bit and UTF-8 C1 ignored) | B (probe stopped at ISSUE-021) |
| I-16 | `infocmp xterm-256color` features used by tmux/nvim (`tmux` with `TERM=xterm-256color`, `set -g extended-keys on`) | tmux works inside Litty incl. truecolor, undercurl, focus events | S manual | B (tmux, nvim not installed) | B |

### J. Inline images

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| J-01 | manual-test groups 22 and 24 | All verdicts pass | S manual | F ISSUE-035 (groups 22/24 via the script's own img helper: png, cells 20x10, sixel, z=-1 under text, deep z under red cells, placeholder, animation red/blue/green and 0% CPU scrolled away OK; `img shm` shows nothing: name /lpl<pid> is rejected by Litty's shm prefix allowlist) | F ISSUE-035, ISSUE-036 (same) + ISSUE-036 (placeholder rows separated by 1 px black seams; X11 seamless) |
| J-02 | `kitten icat` PNG, GIF (animated), large JPEG; `timg`; `chafa -f kitty`; yazi previews | Displayed, animation runs, ~0% CPU when scrolled away | S manual | B (kitten, timg, chafa, yazi not installed; animation covered by J-01) | B |
| J-03 | Sixel: `img2sixel`, `chafa -f sixel` | Displayed; prompt continues below | S manual | P (generated sixel shown, prompt continues below; img2sixel/chafa not installed) | P |
| J-04 | Transfer media `t=d`, `t=f`, `t=t`, `t=s` (shm) | All work; `t=t` file in a temp dir is deleted after reading | S manual | P (t=d, t=f, t=t (temp file deleted), t=s (shm unlinked) all OK and displayed) | P (same) |
| J-05 | `t=f` with a path outside temp dirs, a symlink to `~/.ssh/id_*`, `..` traversal, a missing file | Allowed paths per design; every failure replies the same `EPERM:access denied` | S manual | P (temp PNG OK; outside temp, symlink, `..`, missing, `/etc/passwd` all `EPERM:access denied`) | P (temp PNG OK; outside temp, symlink, `..`, missing, `/etc/passwd` all `EPERM:access denied`) |
| J-06 | Images and resize, scroll, `clear`, alt screen, split panes | Move with text, clipped at pane edge, removed by clear | S manual | P (image scrolls with text, comes back in scrollback, clear removes it, alt screen round trip keeps it; splits not exercised) | P (pixel-identical) |
| J-07 | Delete commands `a=d` with each selector; unsupported selector | Correct placements removed; unsupported → error, nothing changed | S manual | F ISSUE-034 (d=i,i=22 removes only that image, d=A removes all; unsupported d=Q changes nothing but sends no error reply) | F ISSUE-034 (same) |
| J-08 | Unicode placeholders inside tmux | Image shown via placeholders | S manual | B (tmux not installed) | B |
| J-09 | 100 images; a 50 MB image; an APC/Sixel string over 64 MiB (`MAX_IMAGE_STRING`) | Memory bounded; quota/eviction; oversized string rejected; no crash | S manual | P (100 images: RSS 75->75 MB; 50 MB raw image accepted, +47 MB; 65 MiB APC and 65 MiB sixel rejected without reply, RSS 134 MB, no crash) | P (74->121->132 MB) |

### K. Shell integration

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| K-01 | zsh, bash 3.2 (M), bash 5, fish, nushell: no dotfile edits | OSC 133/7 emitted; user rc files still load (`ZDOTDIR` restored for zsh) | S manual | P (zsh 5.9 and bash 5.3: OSC 133 A–D and OSC 7; user rc loaded, bash via `.profile`); fish, nu not installed | P (zsh 5.9 and bash 5.3: OSC 133 A–D and OSC 7; user rc loaded, bash via `.profile`); fish, nu not installed |
| K-02 | elvish with `use litty` in `rc.elv` | Works; without the line nothing breaks | S manual | B (elvish not installed) | B |
| K-03 | Prompt jump `Cmd+Shift+↑/↓` / `Ctrl+Shift+↑/↓` (manual-test 6) | Scrolls to previous/next prompt | S manual | F ISSUE-031 (first Ctrl+Shift+Up lands on the last prompt; a second press lands on the same prompt and Ctrl+Shift+Down goes to the bottom: pressing Shift snaps the view to the bottom first) | P (Up, Up, Down -> MARK-C, MARK-B, MARK-C prompts) |
| K-04 | Select last output `Cmd+Shift+O` / `Ctrl+Shift+O` | Selects and copies the last command's output; empty command → previous output (record) | S manual | P (Ctrl+Shift+O copies `out-1\nout-2`; after an empty command it copies the same previous output) | P (same) |
| K-05 | Marks survive resize (manual-test 12) | Jumps still land on prompts | S manual | P (after maximize, Ctrl+Shift+Up lands on the last prompt) | P (same) |
| K-06 | `shell-integration-features = cursor,title`, `no-cursor`, `sudo`, `false` | Bar cursor at prompt, block in vim; title dir then command; `sudo` feature behaviour recorded | S manual | F ISSUE-037 (default: bar cursor, title = cwd; zsh shows the running command, bash keeps showing the cwd; no-title/false -> `Litty`; sudo not exercised; vim not installed) | F ISSUE-037 (same) |
| K-07 | `shell-integration = none`, and forcing `zsh` while running bash | Off: no OSC 133; forced mismatch does not break the shell | S manual | P (shell-integration = none: title stays `Litty`; forcing zsh for bash: shell works) | P (same) |
| K-08 | `TermMod+T` after `cd` in a nested shell / ssh session | New tab uses the OSC 7 / foreground process directory; remote dir not used locally (record) | S manual | P (recorded: nested `bash --norc` + cd /usr/lib -> new tab opens in the outer shell's last OSC 7 dir, not the foreground process dir; ssh not exercised) | P (same) |
| K-09 | User with existing `ZDOTDIR`, `oh-my-zsh`, `starship`, `powerlevel10k`, `bash-preexec`, `atuin` | Prompts and plugins keep working | S manual | B (oh-my-zsh, starship, powerlevel10k, bash-preexec, atuin not installed) | B |
| K-10 | `~/.cache/litty/shell-integration/<version>/` permissions | Owned by the user, not world-writable | S manual | P (user-owned, mode 775, not world-writable) | P (user-owned, mode 775, not world-writable) |

### L. History search (built-in)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| L-01 | `Ctrl+R` (M also `Cmd+R`; Linux also `Super+R`, `Ctrl+Shift+R`) | Popup with zsh/bash/fish history (`$HISTFILE` first, then by `$SHELL`) | S manual | P (Ctrl+R, Super+R, Ctrl+Shift+R open the popup with ~/.bash_history, newest first) | P (same) |
| L-02 | Syntax: multi-word AND, `'exact`, `^prefix`, `suffix$`, `!neg`, fuzzy, smart case | fzf `--scheme=history` results with match highlight | S manual | F ISSUE-038 (single terms work: fuzzy `bttw`, `'alpha-one`, `^git`, `tmp$`, smart case `gamma`/`GAMMA`, lone `!alpha`; Space is not accepted, so `echo !beta` becomes `echo!beta` and multi-word AND/negation after a term never match) | F ISSUE-038 (same) |
| L-03 | Keys: Up/Down, `Ctrl+N/P/J/K`, PageUp/Down, Home/End, Backspace, `Ctrl+U/W`, `Ctrl+R`/`Alt+S` sort toggle, `Esc`/`Ctrl+C`/`Ctrl+G` | Each works | S manual | P (Down, Ctrl+N/J/K, End, Home, Backspace, Ctrl+U, Ctrl+R sort Relevance<->Recent; Esc, Ctrl+C, Ctrl+G close without running) | P (same) |
| L-04 | `Enter` runs; `Tab` inserts for editing; double-click an item; mouse click | As documented | S manual | P (Enter runs; Tab inserts without running, Return then runs it; mouse not exercised) | P (same) |
| L-05 | Commands run seconds ago appear (in-session recording) | Yes | S manual | P (command typed this session found) | P |
| L-06 | Delete an entry `Shift+Delete` / `Ctrl+D` | Removed from the history file via temp file + atomic rename; file mode and other entries unchanged; zsh extended format and fish YAML preserved | S manual | P (Shift+Delete and Ctrl+D remove the entry from the file; mode 0600 kept; other entries kept; zsh/fish formats not exercised) | P (same) |
| L-07 | Delete while the shell appends to the same file; delete several quickly | No lost or corrupted entries | S manual | P (3 deletes while a background loop appended 300 lines: all 300 present, deletions applied) | P (same) |
| L-08 | 100k-entry history, multi-line entries, invalid UTF-8 bytes | Opens instantly; entries shown; no crash | S manual | P (100,002-entry file opens at once, capped at 10,000 newest by MAX_HISTORY_ITEMS; multi-line shown as \n, invalid UTF-8 as U+FFFD) | P (same) |
| L-09 | nushell / elvish history | Record whether their history files are read (code reads only zsh/bash/fish files) | S manual | B (nushell, elvish not installed) | B |
| L-10 | Inserted command contains ESC or `\x1b[201~` | Stripped from the inserted text | S manual | P (entry with ESC[201~ inserted with ESC stripped) | P |

### M. Find in terminal

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| M-01 | `Cmd+F` / `Ctrl+Shift+F` (`Super+F`) | Box top-right; all matches highlighted in scrollback | S manual | P (Ctrl+Shift+F and Super+F open the box top-right; matches highlighted) | P (pixel-identical to X11) |
| M-02 | Regex syntax: alternation, lazy `*?`, `(?i)`, `(?s)`, `\w \s \b` with Unicode, `\d`, `^`/`$` per line | Per `features.md` | S manual | P (cat\|dog 3, lazy <.*?> 2, (?i)mixed 1, \w+_\w+ 1 with Unicode, \d{3} 163, ^START, END$ per line) | P (same) |
| M-03 | Smart case and accent folding: `tiếng` finds `TIẾNG` | Found | S manual | P (pasted query `tiếng` and `TIẾNG` find `TIẾNG`; `café` finds `CAFÉ`; harness cannot type non-Latin-1 keysyms, so queries were pasted) | P (same) |
| M-04 | `Ctrl+R` literal toggle; invalid regex `(` | Literal mode; invalid shown without crash | S manual | P (Ctrl+R literal: `(` 1 match, `a.b` no match; regex `(` shows `invalid regex`, no crash) | P (same) |
| M-05 | `Enter`/`↑`/`Cmd+G` older, `Shift+Enter`/`↓`/`Cmd+Shift+G` newer (Linux `Super+G`); `Ctrl+P/N` | Steps through matches, scrolls view | S manual | P (Enter/Up/Super+G/Ctrl+P older, Shift+Enter/Down/Super+Shift+G/Ctrl+N newer, wrap 1<->3; view scrolls to ^1$ at the top of scrollback) | P (same) |
| M-06 | `Ctrl+U/W`, `Ctrl/Alt/Cmd+Backspace` clear; `Esc`, `Ctrl+C/G` close | Query cleared on close (v0.14.0 fix) | S manual | P (Ctrl+W, Ctrl+Backspace, Alt+Backspace clear; Esc, Ctrl+C, Ctrl+G close; query empty on reopen) | P (same) |
| M-07 | Paste into the box (`TermMod+V`), `UIMod+1` while open | Paste goes to the box; shortcuts still work; nothing reaches the shell | S manual | P (Ctrl+Shift+V pastes into the box (1/1), Ctrl+1 handled, nothing reached the shell) | P (same) |
| M-08 | Search 10 MB scrollback while output streams | Responsive; matches update | S manual | B (not isolated: X11 input and redraw lag during the flood, ISSUE-043; final count 1/1) | F ISSUE-039 (box keeps `no matches` while `stream-2500` is printed; closing and reopening finds 1/1) |

### N. Links and opening files

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| N-01 | `LinkMod`+hover over `https://example.com` | Underline and preview pill | S manual | P (Ctrl+hover underlines the URL; pill bottom-left shows the target) | P (same) |
| N-02 | `LinkMod+Click` URL, `mailto:`, `ftp://` | Opens in the default handler | S manual | F ISSUE-041 (Ctrl+Click https opens it via xdg-open; plain-text `mailto:` and `ftp://` are not detected, nothing opens; they work only as OSC 8) | F ISSUE-041 (same) |
| N-03 | `LinkMod+Click` `~/…`, `./…`, absolute paths: folder, `.txt`, `.rs`, `.pdf`, `.png` | Opens with the default app | S manual | F ISSUE-040 (~/w/dir1, ./a.txt, ./c.pdf, ./d.png open with the default handler; an absolute path that soft-wraps opens only its first row, here the parent `…/homes`) | F ISSUE-040 (same) |
| N-04 | `LinkMod+Click` a `.sh`, `.command`, `.app`, `.terminal`, `.webloc`, `.desktop`, `.AppImage`, a file without extension | Only revealed in its folder, never executed | S manual | P (.sh, .desktop, .AppImage, .command, no-extension file: containing folder opened, nothing executed) | P (same) |
| N-05 | OSC 8 (manual-test 28): `ls --hyperlink=auto`, `rg --hyperlink-format`, `gcc` diagnostics, `delta` | Hover shows the real target; click opens | S manual | P (OSC 8 file:// to local file opens it, https opens; hover pill shows the real target; ls/rg/gcc/delta hyperlinks not exercised) | P (same) |
| N-06 | OSC 8 with `file://otherhost/…`, `javascript:`, `file:` to a script | Ignored / revealed only | S manual | P (file://otherhost and javascript: ignored; file:// to a script reveals its folder only) | P (same) |
| N-07 | Path starting with `-` (`./-rf`), names with spaces, quotes, `$()` and `;` | Passed as one argument, nothing executed by a shell | S manual | P (./-rf revealed as one argument; names with spaces, quotes, $() and ; are not detected as links, so nothing opens and nothing runs, no PWNED file) | P (same) |
| N-08 | Click 50 links rapidly | Bounded helper processes, no zombies (`ps`) | S manual | P (50 rapid Ctrl+Clicks -> 50 opens, no zombie children) | P (same) |

### O. Notifications, bell, title and progress

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| O-01 | OSC 9 and OSC 777 while Litty is in the background (manual-test 3) | Notification with title/body | S manual | P (OSC 9 -> Notify title `Litty` body `osc9-body`; OSC 777 -> title/body; delivered via notify-send fallback in the headless session; at most one per second by design, so a second one 0.5 s later is dropped; also notifies while focused) | P (same) |
| O-02 | M packaged app: first notification permission prompt; Allow | Delivered under Litty identity and icon | S manual | — | — |
| O-03 | M: Don't Allow; relocated/re-signed bundle; raw binary | Helper fallback shows it without click routing | S manual | — | — |
| O-04 | Click a notification from a pane in a split, after moving its tab to another window | Correct window/tab/pane selected | S manual | P partial (short runtime dir, native bus path: OSC 9 from the lower pane of a split in a background tab; clicking the GNOME banner focused the window and typing landed in that pane; moving the tab to another window first was not done, tab tear-out is unreliable, ISSUE-024) | P partial (same) |
| O-05 | Click after the pane closed (app running); after full quit (M cold relaunch) | Ignored; M opens one fresh window | S manual | — | — |
| O-06 | 3 processes; notification from the oldest; click | Routed to its origin process | S manual | P (3 processes on one runtime dir, newest focused; notification from the oldest; banner click → only the oldest is focused) | P (same) |
| O-07 | Linux without a session bus / without a notification server | `notify-send` fallback, no click routing | — | P (session bus pointed at a missing socket: no crash; notify-send fallback child not observed) | P (same) |
| O-08 | Body/title with 10 KB text, leading `-`, quotes, newlines | Truncated safely (title 128, body 512 chars); no option injection | S manual | P (10 KB title/body: title 127, body 510 chars; leading `-`/`--` stripped; quotes kept; newline removed) | P (same) |
| O-09 | Clean quit withdraws notifications (`Cmd+Q`, Dock Quit) | Withdrawn | S manual | P (Ctrl+Shift+Q with a delivered notification: Litty sends no `CloseNotification`, but GNOME Shell closes it (`NotificationClosed`, reason 2) as Litty exits; Dock quit is macOS only) | P (same) |
| O-10 | Bell: `bell-features` `system`, `attention`, `title`, `audio` (+ path/volume), `border`, `no-*` (manual-test 7, 11) | Each effect; 🔔 on background tab cleared on visit; Dock bounce / urgency hint | S manual | P (title: background tab shows 🔔, cleared on visit; border flashes; attention: _NET_WM_STATE_DEMANDS_ATTENTION set; audio: no error, paplay spawn not observed (PULSE_SERVER pointed at a dead socket); system bell not tried: it would sound on the real speakers) | P (title, border same; attention via xdg_activation_v1 activate; audio as X11) |
| O-10a | `bell-features = audio` with `bell-audio-volume = 0`, `0.2`, `1`, `2`, `abc` | Silent / quiet / full; `2` clamps to `1.0`; `abc` → one error line | S manual | P (0, 0.2, 1, 2 accepted silently (2 clamps per config.rs); `abc` → one error line `bell-audio-volume: invalid number`; audio output pointed at a missing PulseAudio server, so loudness not judged) | P (same) |
| O-11 | `bell-audio-path` missing file; Linux without `paplay` | No crash; error recorded | S manual | P (missing path -> `litty: cannot play …: No such file or directory`, no crash; paplay present, so the no-paplay case not exercised) | P (same) |
| O-12 | Window title from OSC 2, cwd, running command; non-ASCII title on X11 (`xprop _NET_WM_NAME WM_NAME`) | Correct; `WM_CLASS = litty, litty` | S manual | P (OSC 2 with `;`, non-ASCII `COMPOUND_TEXT`, `WM_CLASS = litty, litty`; cwd/command titles in K-01) | P (xdg title correct, app_id `litty`) |

### P. Terminal inspector and custom shaders

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P-01 | Inspector `Cmd+Option+I` / `Ctrl+Shift+I` (`Super+Alt+I`) (manual-test 17) | Sections, live cell, last key bytes, modes | S manual | P (Ctrl+Shift+I and Super+Alt+I toggle the inspector: grid, cursor, screen modes, mouse, keyboard, colors, process, title, cell, last key `x`) | P (same) |
| P-02 | `custom-shader = examples/shaders/invert.glsl`, then `scanlines.glsl`, both chained (manual-test 19) | Inverted, upright, mouse aligned; chain order respected | S manual | F ISSUE-042 (invert: inverted and upright; chain invert->scanlines applied in order; scanlines makes the whole window translucent, desktop wallpaper shows through, because the example scales alpha too) | F ISSUE-042 (same) |
| P-03 | `custom-shader-animation = true/false/always`; unfocused window | Animates only per policy | S manual | P (true: animates focused, stops unfocused; always: animates unfocused too; false: static; note: with cursor blink on, every blink redraw advances iTime, so `false` still changes twice a second) | P (same) |
| P-04 | Shader with a syntax error; missing file | Reported once on stderr with path (Linux also line); skipped; Litty starts | S manual | P (syntax error -> one stderr line with path and `line 2`; missing file -> one line with path; Litty starts) | P (same) |
| P-05 | Shader + 2 windows + resize + occlusion | No validation errors (Linux layer on); CPU stops when hidden | S manual | P (2 windows + maximize/restore with an animated shader and WGPU_VALIDATION=1: no validation errors; CPU after minimizing not isolated (minimize not confirmed)) | P (same) |

### Q. Process reuse (`+reuse`)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| Q-01 | `cd /tmp && litty +reuse` with Litty running | New window in the newest eligible process, shell in `/tmp` | S manual | P (`cd /tmp && litty +reuse` -> rc 0, new window in the running process, shell in /tmp) | P (same) |
| Q-02 | `litty --reuse` with no Litty running | Starts a new process at that directory | S manual | P (empty registry: `+reuse` starts a new process in /tmp, which registers itself) | P (same) |
| Q-03 | Running process is a benchmark / `+update` | Not chosen | S manual | P (`+bench-ram` running: the registry holds only the normal window's entry; `+reuse` from `/tmp` rc 0 into that window; `+update` never starts the activation service) | P (same) |
| Q-04 | Wayland: launcher activation token forwarded | New window focused (compositor policy permitting) | — | — | P (window opened by +reuse gets keyboard focus) |
| Q-05 | Kill the target process between lookup and request; stale registry entries | Error exit, no duplicate window; stale entries ignored/cleaned | S manual | P (stale registry entry with a dead socket ignored, reuse succeeds; entry left in place) | P (same) |
| Q-06 | Registry dir permissions (`$XDG_RUNTIME_DIR/litty-a1`, `/tmp/litty-a1-<uid>`); another user's process | `0700`, owned by the user; other UID never joined | S manual | B (`0700`, owned by the user; no second user for the other-UID part) | P (same) |
| Q-07 | Directory with spaces/Vietnamese; deleted directory | Correct dir; deleted → clear error | S manual | P (dir with spaces: new shell there (Vietnamese letters dropped by the harness keyboard, not tested); deleted dir -> rc 1 `litty: reuse failed: No such file or directory`, no window) | P (same) |

### R. Update (real GitHub releases)

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| R-01 | `litty +check-update` on v0.14.1 | "up to date" (latest is v0.14.1) | P | P | — |
| R-02 | Build tag `v0.14.0` (`git worktree add ../litty-0.14.0 v0.14.0`, then `LITTY_MINISIGN_PUBKEY="$(tail -n 1 resources/minisign.pub)" cargo build --release`; a build without the pinned key refuses to update, by design; ISSUE-011), run `+check-update` | Reports v0.14.1 available | P | P | — |
| R-03 | v0.14.0 standalone binary in a user-writable path: `+update` | Downloads asset for this CPU, verifies minisign + SHA-256 + `# litty-release: v0.14.1`, replaces the binary; new binary contains 0.14.1 | P (with pinned key, ISSUE-011) | P (pinned key, ISSUE-011; result byte-identical to the asset) | — |
| R-04 | `+update --force` / `-f` on v0.14.1 | Reinstalls the current version | P (`--force` and `-f`; pinned key, ISSUE-011) | P (`--force` and `-f`) | — |
| R-05 | M: v0.14.0 `Litty.app`: Litty ▸ Check for Updates… → Update Now | Bundle staged, signed, swapped atomically; app relaunches as 0.14.1 | S manual | — | — |
| R-06 | M: same with `/Applications` read-only or on a filesystem without atomic swap | Error; installed bundle intact | S manual | — | — |
| R-07 | Linux `.deb`/`.rpm` install (`/usr/bin/litty`, root-owned): `+update` as a user | Refused with a clear message; nothing changed | — | F ISSUE-018 | — |
| R-08 | Two `+update` at once | Second waits or fails on the lock in `~/.cache/litty/updates` | S manual | P (CLI, no display; two `+update` on a v0.14.0 copy: the second fails with "another Litty update is already in progress", the first installs v0.14.1; one run, backend-independent) | — |
| R-09 | Network off / DNS failure / slow link (throttle) | Timeouts, clear error, no partial binary | S manual | P partial (network namespace with no network: clear curl error "Could not resolve host", rc 1, binary unchanged; slow link not tested) | — |
| R-10 | `auto-update = check` with `~/.cache/litty/last_update_check` removed; then relaunch within 24 h | One background check notifies; second launch does not check | S manual | P (run with `LITTY_BACKEND=wayland`; the check is backend-independent. Stamp removed: one background check, "Litty Update Available — v0.14.1 is available"; relaunch with the stamp copied in: stamp unchanged, no Notify) | — |
| R-11 | `auto-update = download` | Downloads and verifies; next Check/`+update` installs without re-download | S manual | P (run with `LITTY_BACKEND=wayland`; backend-independent. `auto-update = download`: cache holds the v0.14.1 asset, SHA256SUMS and minisig, "Litty Update Ready" notification; `+update` then calls only the releases API, no asset re-download, installs v0.14.1 and clears the cache; it still prints "Downloading…", ISSUE-052. Offline `+update` fails at the version check, by design) | — |
| R-12 | `auto-update-channel = tip` | Considers prereleases, picks the highest version | S manual | P partial (no prereleases exist on GitHub: tip and stable both pick v0.14.1; prerelease ordering not exercised) | — |
| R-13 | `GITHUB_TOKEN` set / rate-limited API | Token used for API; falls back to the `releases/latest` redirect | S manual | P partial (bogus `GITHUB_TOKEN`: check still finds v0.14.1 via the fallback; rate-limited API not tested) | — |
| R-14 | `check_for_updates` keybind and menu item with no window open (M) | Works without a window | S manual | — | — |

### S. macOS-specific

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| S-01 | Menu bar: every item of Litty, Shell, Edit, View, Window (manual-test 15) | Each works; Window menu lists windows | S manual | — | — |
| S-02 | Secure Keyboard Entry via menu and `toggle_secure_input` keybind | ✓ in menu, 🔒 in title, survives app switch, released when the window closes | S manual | — | — |
| S-03 | `macos-native-tabs = true` (manual-test 18) | Native tabs, `Cmd+T` in cwd, switching and close work | S manual | — | — |
| S-04 | Hide/Hide Others/Show All, Spaces, Stage Manager, full-screen Space | Correct redraw, occlusion stops rendering | S manual | — | — |
| S-05 | Dock bounce on bell; Dock menu Quit with a running process | Bounce; quit confirmation | S manual | — | — |
| S-06 | `confirm-close-surface` `true/running`, `false/never`, `always`; red close button, `Cmd+W`, `Cmd+Q` (manual-test 14) | Dialog wording and buttons; Cancel keeps, Close closes | S manual | — | — |
| S-07 | Gatekeeper: download the DMG, `xattr -dr com.apple.quarantine`, launch | Launches; app signature verifies (`codesign -dv`) | S manual | — | — |
| S-08 | Option key in non-US layouts (e.g. `Option+L` → `@` on German) | Character typed, not ESC prefix | S manual | — | — |
| S-09 | Sleep/wake, display disconnect/reconnect, lid close with external display | Windows redraw; no crash | S manual | — | — |
| S-10 | VoiceOver basic | Record behaviour (no claim of support) | S manual | — | — |

### T. Linux-specific

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| T-01 | Client title bar: drag to move, double-click to maximize, minimize/maximize/close buttons, resize edges and corners | All work | — | P (double-click title and the maximize button maximize 119×32 → 159×42 and restore; left and top edges resize; minimize hides the window; close exits 0. Drag-to-move: Litty sends `move` (W log: `xdg_toplevel.move` with the press serial) but the window stays put, as does a GTK control window, so the harness pointer cannot do move grabs; not verified) | P (same) |
| T-02 | sway/tiling: title band, rounded corners and edges hidden while tiled, back when floating | As documented | — | — | B (sway not installed) |
| T-03 | Portal light/dark (`gsettings set org.gnome.desktop.interface color-scheme prefer-light`) | Live switch; mode 2031 report | — | P (private portal on a separate dconf db: starts light with `theme = light:…,dark:…`; prefer-dark / prefer-light / default switch the background live; mode 2031 reports `CSI ?997;1n` / `;2n` each time) | P (same) |
| T-04 | No session bus / no portal | Dark default; no D-Bus autolaunch | — | P (`DBUS_SESSION_BUS_ADDRESS` pointing nowhere: dark theme, DSR 996 → `CSI ?997;1n`, no dbus-launch/dbus-daemon child, no stderr) | P (same) |
| T-05 | Monitor info: `current_monitor` refresh rate (RandR) on 60/144 Hz | Correct rate (inspector or frame pacing) | — | B (one 60 Hz virtual monitor only) | B (same) |
| T-06 | `confirm-close-surface` with zenity installed: Cancel and Close | Cancel keeps, Close closes | — | P (`confirm-close-surface = true`, pane running `sleep 100`: zenity "Close this terminal?"; Esc keeps the pane; Close ends `sleep` and the pane) | P (same) |
| T-07 | Uninstall zenity: close a pane running `sleep 100`; multi-line paste into `cat` (no bracketed paste) | Record both (code: close proceeds; paste is cancelled) | — | P (zenity made non-executable in a private mount namespace: closing a pane with `sleep 100` proceeds; multi-line paste into `cat` is cancelled, nothing sent. Not literally uninstalled: the file exists but cannot run) | P (same) |
| T-08 | Blocking dialogs: while a zenity dialog is open, another window streams output | Record whether all windows freeze until the dialog closes | — | P recorded (while a zenity dialog is open every pane and window stops redrawing; the streaming pane resumes after the dialog closes; see ISSUE-048) | P recorded (same) |
| T-09 | `LITTY_BACKEND=wayland` without a compositor (`WAYLAND_DISPLAY` unset) | Falls back to X11 | — | P (X11 window after the fallback) | P (stderr names the fallback) |
| T-10 | Wayland fractional scale 100/125/150/200%, live change, move between outputs | Sharp text, correct buffer size | — | — | B (headless virtual monitor; no second output or fractional scale to switch) |
| T-11 | Wayland: key repeat rate from compositor, `caps:escape` remap, horizontal scroll | Repeat follows settings; remap applies; horizontal scroll recorded (unsupported) | — | — | P partial (2 s hold → 52 chars = 500 ms delay + 30 ms interval from GNOME settings; compositor sends repeat_info(0,0) and Litty does not double-repeat; horizontal scroll sends no report (unsupported, recorded); `caps:escape` not tried: it would change the shared xkb-options setting) |
| T-12 | Vulkan: Intel, AMD, NVIDIA proprietary; `MESA_VK_DEVICE_SELECT`; software only (lavapipe) | Picks a hardware device first; lavapipe works slowly; no VUID errors | — | B (one GPU; no lavapipe rendering check) | B (same) |
| T-13 | Vulkan failure: no ICD installed | Clear `litty: … failed (VkResult n)` message, no segfault | — | P (`the Vulkan loader offers no window-system surface extension`) | P (as X) |
| T-14 | X11 without a compositor (no ARGB visual) and `background-opacity < 1` | Opaque fallback, no crash | — | B (Mutter always composites; no compositor-less X server available) | — |
| T-15 | Non-EWMH window manager (twm) title | `WM_NAME` readable (`STRING`/`COMPOUND_TEXT`) | — | B (twm not installed) | — |
| T-16 | GNOME notification removal on focus; KDE/sway notification click | Removed on focus; click raises/selects (record per desktop) | — | F ISSUE-047 (GNOME, native bus path: OSC 9 → Notify with an "Open" action; clicking the banner focuses Litty and selects the tab that rang; refocusing the window does not remove the notification (no CloseNotification), see ISSUE-047) | F ISSUE-047 (same) |

### U. Security

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| U-01 | Copy text containing `\x1b[201~rm -rf ~\n` (use a file and `pbcopy`/`xclip`), paste in a bracketed shell | ESC and C1 CSI stripped; nothing runs; LF/CRLF → CR | S manual | P (clipboard `safe\e[201~touch …\nline2\r\nC1\x9b201~…`: ESC and C1 CSI stripped, LF and CRLF arrive as CR inside one bracketed paste; nothing ran at the bash prompt) | P (clipboard `safe\e[201~touch …\nline2\r\nC1\x9b201~…`: ESC and C1 CSI stripped, LF and CRLF arrive as CR inside one bracketed paste; nothing ran at the bash prompt) |
| U-02 | Multi-line paste into `cat` (no bracketed paste) | "Confirm Multi-Line Paste" with preview; Cancel sends nothing | S manual | P (`Confirm Multi-Line Paste` zenity dialog shown for paste into `cat`; Esc cancels and `cat` received nothing) | P (`Confirm Multi-Line Paste` zenity dialog shown for paste into `cat`; Esc cancels and `cat` received nothing) |
| U-03 | OSC 52 read attempts, title reporting (`\e[21t`), OSC 4/10/11 query flood | No clipboard read; title not reported; replies bounded | S manual | P (OSC 52 read and `21t` unanswered; 2000 OSC 11 queries → 2000 replies, still responsive) | P (OSC 52 read and `21t` unanswered; 2000 OSC 11 queries → 2000 replies, still responsive) |
| U-04 | Notification text with shell metacharacters | Passed as separate arguments | S manual | P (OSC 777 title `-u critical`, body with `$(…)` and backticks reach Notify as literal strings via notify-send fallback; no command ran; title shown as `u critical`, so a leading `-` is dropped) | P (OSC 777 title `-u critical`, body with `$(…)` and backticks reach Notify as literal strings via notify-send fallback; no command ran; title shown as `u critical`, so a leading `-` is dropped) |
| U-05 | Kitty `t=f`/`t=t` probes of private files (J-05) | Same `EPERM` for all failures (CWE-200) | S manual | P (see J-05: every failed `t=f`/`t=t` probe returns the same `EPERM:access denied`) | P (see J-05: every failed `t=f`/`t=t` probe returns the same `EPERM:access denied`) |
| U-06 | History rewrite (L-06) under `strace -f -e rename,openat` / `fs_usage` | Temp file + atomic `rename`; never truncates in place | S manual | P (strace: Ctrl+D in history popup writes `..bash_history.litty-<pid>-….tmp` with O_CREAT\|O_EXCL 0600, then `rename` over `.bash_history`; no truncate/O_TRUNC on the original) | P (strace: Ctrl+D in history popup writes `..bash_history.litty-<pid>-….tmp` with O_CREAT\|O_EXCL 0600, then `rename` over `.bash_history`; no truncate/O_TRUNC on the original) |
| U-07 | `fork()` safety: open 50 tabs concurrently while output streams (`strace -f` on Linux) | No deadlock after fork; child only `execve`s | S manual | P (50 Ctrl+Shift+T while `yes` streams, under `strace -f`: 51 shells; children call only rt_sigaction/rt_sigprocmask/setsid/TIOCSCTTY/dup2/close/close_range/chdir before `execve`; no brk/mmap/futex/openat; app stayed responsive) | P (50 Ctrl+Shift+T while `yes` streams, under `strace -f`: 51 shells; children call only rt_sigaction/rt_sigprocmask/setsid/TIOCSCTTY/dup2/close/close_range/chdir before `execve`; no brk/mmap/futex/openat; app stayed responsive) |
| U-08 | Config/theme/shader paths with `~`, `..`, symlinks; cache dirs pre-created by another user in `/tmp` | Uses user-owned cache dir; never a shared `/tmp` path | S manual | P (strace: caches under `~/.cache/litty`, no `/tmp`, `/var/tmp` or `/dev/shm` path opened; `custom-shader = ~/../link.glsl` (symlink) loads; dir pre-created by another user not exercised: no second account) | P (strace: caches under `~/.cache/litty`, no `/tmp`, `/var/tmp` or `/dev/shm` path opened; `custom-shader = ~/../link.glsl` (symlink) loads; dir pre-created by another user not exercised: no second account) |
| U-09 | Update tampering: covered by unit tests in `src/update/verify.rs` (no URL override exists) | P1-04 pass counts them | P (P1-04) | P (P1-04) | P (P1-04) |
| U-10 | `cargo test security_poc` | Pass | P (ISSUE-002 under isolated `HOME`) | P (15 passed, 1 ignored) | — |

### V. Robustness and stress

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| V-01 | Resize storm: drag a corner continuously for 30 s while `yes` runs | No crash, no stuck frame, final size correct; shell gets the final SIGWINCH | S manual | P (30 s corner-drag storm while a writer floods: 369 SIGWINCH, final TIOCGWINSZ 101×27 = stty size, matches the 817×505 window; alive, RSS 123 MB) | P (30 s corner-drag storm while a writer floods: 414 SIGWINCH, final TIOCGWINSZ 100×26 = stty size; alive, RSS 93 MB) |
| V-02 | 100 rapid `TermMod+T` / `TermMod+W` | No crash, no leaked shells (`ps`) | S manual | P (100 open/close pairs at 30 ms → 1 shell; +50 → 51; −50 → 1; no zombie or leaked bash) | P (same) |
| V-03 | Rapid config reloads (script touching the file 100×) | No crash; final config applied | S manual | P (config rewritten 100× with Ctrl+Shift+, after each: alive, final `background = #3060a0` applied; no stderr. Reload happens on the key or on focus-in, not on file change alone) | P (same) |
| V-04 | `cat /dev/urandom \| head -c 50M` in 4 panes | Responsive; memory bounded | S manual | F ISSUE-043 (4 panes: typing into other panes while pane 0 streams is not delivered; Ctrl+C during `yes` in one pane takes 20–60+ s; see ISSUE-043) | P (4 panes × 50 MB urandom finished in 6.0 s, peak RSS 86 MB, DSR answered at once) |
| V-05 | Run 8 hours idle with 3 windows (overnight) | No memory growth, 0% CPU apart from blink | S manual | B (8-hour idle run not done in this session) | B (same) |
| V-06 | Low memory (Linux cgroup `systemd-run --user -p MemoryMax=200M litty`) | Degrades or exits cleanly; no corrupted files | — | P (`MemoryMax=200M`: 50 MB random output, RSS 80 MB, clean exit) | P (`systemd-run --user --scope -p MemoryMax=200M`: 50 MB urandom, RSS 81 MB, shell usable, clean stop) |
| V-07 | GPU reset / switching GPUs (Linux PRIME, eGPU) | Recovers or reports an error | — | B (headless GNOME has no GPU switch or reset to trigger) | B (same) |
| V-09 | Slow compositor / GPU stall: run under heavy GPU load (e.g. a GPU stress tool) while typing and scrolling (0.15.0 open item) | Input still echoes; frames resume when load ends; no hang or validation error | S manual | B (no GPU stress tool installed; not installing one) | B (same) |
| V-10 | Input-to-photon latency with a high-speed camera (≥ 240 fps) filming key press and screen (0.15.0 open item) | Record p50/p95 per platform next to P3-06 | S manual | B (needs a high-speed camera) | B (same) |
| V-08 | Disk full while writing history delete / update download | Error, original files intact | S manual | B (needs a small full filesystem; mounting one needs root) | B (same) |

### W. AI assistant (offline only)

Live providers are out of scope.

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| W-01 | No `ai-provider` configured, run `yo hello` | Message "Set ai-provider and ai-api-key before using yo."; shell not broken | S manual | P (`yo hello` → "Set ai-provider and ai-api-key before using yo."; next command runs normally) | P (`yo hello` → "Set ai-provider and ai-api-key before using yo."; next command runs normally) |
| W-02 | `yo show documentation`, `yo reset`, `yo show last response` | Local commands work without network | S manual | P (`yo show documentation` prints the docs, `yo reset` → "Session reset.", `yo show last response` → "No response received yet." with no network) | P (`yo show documentation` prints the docs, `yo reset` → "Session reset.", `yo show last response` → "No response received yet." with no network) |
| W-03 | `ai-trigger = ask`, `= off`, `= rm -rf`, empty | Custom word works in new tabs; `off` makes `yo` an ordinary command; invalid rejected; empty restores `yo` | S manual | P (`ai-trigger = ask`: `ask hello` handled in a new tab; `off`: `yo` → "command not found"; `rm -rf`: one config error, default `yo` kept; empty: `yo` works) | P (`ai-trigger = ask`: `ask hello` handled in a new tab; `off`: `yo` → "command not found"; `rm -rf`: one config error, default `yo` kept; empty: `yo` works) |
| W-04 | Typing/paste/Enter during "thinking" with an unreachable `ai-base-url` (`http://127.0.0.1:9/v1`) | Queued; `Ctrl+C` cancels; error shown; prompt usable | S manual | P (Ollama at `127.0.0.1:9`: Enter typed during thinking is queued and runs after the error "AI transport failed (curl exit Some(7))"; prompt usable. Against a server that never answers: "Thinking...", Ctrl+C → "Cancelled", prompt usable. Refused connection is retried 10× for 242 s before the error; see ISSUE-044, ISSUE-045) | P (Ollama at `127.0.0.1:9`: Enter typed during thinking is queued and runs after the error "AI transport failed (curl exit Some(7))"; prompt usable. Against a server that never answers: "Thinking...", Ctrl+C → "Cancelled", prompt usable. Refused connection is retried 10× for 242 s before the error; see ISSUE-044, ISSUE-045) |
| W-05 | Config errors: `ai-provider = unknown`, `ai-thinking = extreme`, `ai-web-search = maybe`, `ai-context-window = 100` | One error each | S manual | P (one stderr line each: `unsupported AI provider: unknown`, `invalid ai-thinking`, `ai-web-search must be true or false`, `ai-context-window must be at least 4096`) | P (one stderr line each: `unsupported AI provider: unknown`, `invalid ai-thinking`, `ai-web-search must be true or false`, `ai-context-window must be at least 4096`) |
| W-06 | Redaction: unit tests (`cargo test redact`) | Pass | P | P (10 tests) | — |

## 8. P5 — Packaging and installation

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P5-01 | `bash scripts/linux/test-bundle-linux.sh` | Pass | — | P | — |
| P5-02 | Debian: `bash scripts/linux/test-package-deb.sh`; `sudo apt install ./litty_<ver>_amd64.deb` | Installs `/usr/bin/litty`, desktop entry, icon; menu launch works; `apt remove` cleans up | — | B (`test-package-deb.sh` passes; `apt install` needs root, not run) | B (same) |
| P5-03 | Fedora: `bash scripts/linux/test-package-rpm.sh`; `sudo dnf install ./litty-<ver>-1.x86_64.rpm` | Same as P5-02 | — | B (no Fedora machine; `rpmbuild` not installed) | B (same) |
| P5-04 | `desktop-file-validate resources/linux/litty.desktop`; dock pinning; `StartupWMClass` grouping | Valid; windows group under one icon | — | P (`desktop-file-validate` OK; dock pinning manual) | P (same .desktop as X: `desktop-file-validate` OK; dock pinning not tested in headless shell) |
| P5-05 | macOS: `./bundle_macos.sh` (install), DMG layout, `Info.plist` versions, `codesign -dv --verbose` | Versions 0.14.1; bundle id `com.litty.terminal` | P | — | — |
| P5-06 | Release asset verification: download v0.14.1 assets, `sha256sum -c`, `minisign -Vm SHA256SUMS.txt -p resources/minisign.pub` | All hashes and the signature verify; one `# litty-release: v0.14.1` line | P (6/6 hashes, signature OK) | P (6/6 hashes, signature OK) | — |
| P5-07 | Raw binary on a clean machine (glibc 2.34 Debian 12; current macOS) | Runs; Linux needs only documented libs | S manual (clean machine) | B (no clean Debian 12 machine) | B (same) |

## 9. P6 — Documentation consistency

| ID | Steps | Expected | M | X | W |
| --- | --- | --- | --- | --- | --- |
| P6-01 | Every shortcut in `shortcuts.md` matches behaviour from C-01; every default in `keymap.rs` is documented | No differences (list each as an issue) | F ISSUE-005 | F ISSUE-005 | F ISSUE-005 (docs, as X) |
| P6-02 | Every key in `configuration.md` matches `src/config.rs` (names, defaults, ranges); undocumented keys/env vars listed | No differences | F ISSUE-006, ISSUE-007 | F ISSUE-006, ISSUE-007 | F ISSUE-006, ISSUE-007 (docs, as X) |
| P6-03 | Every action in `keybinding-actions.md` matches `parse_binding` | No differences | F ISSUE-008 | F ISSUE-008 | F ISSUE-008 (docs, as X) |
| P6-04 | README Quick Start, `installation.md` package names and versions | Point at current release assets | F ISSUE-009 | F ISSUE-009 | F ISSUE-009 (docs, as X) |
| P6-05 | `features.md` claims vs results of this run | Each claim backed by a passing case | B (needs the P4 manual run) | F ISSUE-037, ISSUE-040 (contradicted on Linux: the optional window title shows only the cwd in bash; Ctrl+Click on a wrapped path opens a prefix. Not verifiable here: fish, nushell and elvish integration; Neovim, Helix, fish 4 and Kakoune keyboard claims; CJK IME; Sixel and `kitten icat`/`timg`/`chafa`/yazi. The other claims are backed by passing cases) | F ISSUE-036, ISSUE-037, ISSUE-040 (as X, plus seams in Unicode-placeholder images) |
| P6-06 | `scripts/manual-test.sh` on Linux (it uses `pbpaste`, Cmd wording, macOS sound paths) | Runs, or Linux substitutions are documented | — | F ISSUE-019 | F ISSUE-019 (as X) |

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
